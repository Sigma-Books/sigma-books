//! Passkeys, WebAuthn (ADR 0006, ADR 0012, ADR 0018 ; L2 PR 8c) : le protocole, sans base ni HTTP.
//!
//! La vérification est celle de `webauthn-rs` (audité par SUSE) : défi, origine, identifiant de
//! la partie de confiance, signature, compteur, vérification de l'utilisateur. Rien de
//! cryptographique n'est écrit ici. Ce module fixe les **choix de Sigma Books** :
//!
//! - la partie de confiance (`rp_id`) est l'hôte de l'origine de l'instance (`SIGMA_ORIGINE`),
//!   l'origine admise est cette seule origine ;
//! - **vérification de l'utilisateur exigée** (code, biométrie) à l'enrôlement et à chaque
//!   authentification : une passkey est un facteur fort à elle seule (ADR 0012) ;
//! - pas d'attestation (aucun modèle d'authentificateur n'est exigé) ni de passkey découvrable :
//!   on part du courriel, puis `allowCredentials` ;
//! - au plus [`PASSKEYS_MAX`] passkeys par utilisateur, [`VALIDITE_DEFI`] pour terminer une
//!   cérémonie.
//!
//! # Jeton de cérémonie
//! L'état d'une cérémonie vit en base, côté serveur ; le navigateur n'en détient que le
//! **jeton**, dans le cookie `__Host-sigma_ceremonie` (`HttpOnly`, `Secure`, `SameSite=Strict`).
//! La base n'en garde que l'empreinte SHA-256 ([`JetonDefi::empreinte`]).
//!
//! # Anti-énumération
//! Pour un courriel inconnu ou sans passkey, [`defi_factice`] rend un défi de même forme qu'un
//! vrai, avec de faux identifiants de credential dérivés du courriel par la clé de l'instance :
//! stables d'un appel à l'autre (un vrai compte rend toujours les mêmes), indiscernables sans
//! la clé. Leur nombre (1 à 10, plus souvent peu) et leurs longueurs (16 à 128 octets) sont eux
//! aussi dérivés du courriel, dans les distributions de vrais identifiants ; un compte plafonné
//! reçoit ses vraies options (sans état gardé), non des faux.

use base64ct::{Base64UrlUnpadded, Encoding};
use sha2::{Digest, Sha256};
use uuid::Uuid;
use webauthn_rs::prelude::{
    AuthenticationResult, CreationChallengeResponse, Passkey, PasskeyAuthentication,
    PasskeyRegistration, PublicKeyCredential, RegisterPublicKeyCredential,
    RequestChallengeResponse, Url, Webauthn, WebauthnBuilder, WebauthnError,
};
use webauthn_rs_proto::{
    AllowCredentials, PublicKeyCredentialRequestOptions, UserVerificationPolicy,
};
use zeroize::Zeroize;

use super::cle::CleInstance;
use super::tirer;

/// Nombre maximal de passkeys d'un utilisateur.
pub const PASSKEYS_MAX: i64 = 10;
/// Durée de validité d'une cérémonie (un défi non utilisé échoit).
pub const VALIDITE_DEFI: time::Duration = time::Duration::minutes(5);
/// Défis de connexion conservés par utilisateur (les plus récents) : borne le stockage, sans
/// qu'un tiers qui connaît un courriel puisse invalider sans fin la cérémonie de son titulaire.
pub const DEFIS_CONNEXION_MAX: i64 = 4;
/// Nom du cookie qui porte le jeton de cérémonie.
pub const NOM_COOKIE_CEREMONIE: &str = "__Host-sigma_ceremonie";

/// Longueurs des faux identifiants de credential, parmi celles des authentificateurs courants.
const LONGUEURS_FACTICES: [usize; 6] = [16, 32, 32, 48, 64, 128];

const OCTETS_JETON: usize = 32;
/// 32 octets en base64url sans remplissage.
const LONGUEUR_TEXTE: usize = 43;

/// Échec de la configuration ou du protocole WebAuthn. Aucune cause n'est détaillée : elle ne
/// doit pas guider un attaquant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ErreurPasskey {
    /// Origine de l'instance inutilisable comme partie de confiance (par exemple une adresse IP).
    #[error("origine inutilisable comme partie de confiance WebAuthn")]
    Origine,
    /// Cérémonie refusée par la vérification WebAuthn (défi, origine, signature, compteur,
    /// vérification de l'utilisateur…).
    #[error("cérémonie WebAuthn refusée")]
    Refusee,
    /// Compteur de signatures qui n'avance pas : l'authentificateur a peut-être été cloné
    /// (WebAuthn §6.1.1). Refusé comme tout échec, mais tracé à part.
    #[error("compteur de signatures régressif : clonage possible")]
    Clonage,
    /// Aléa du système indisponible.
    #[error("aléa du système indisponible")]
    Alea,
}

/// La partie de confiance : `webauthn-rs` configuré pour l'origine de l'instance.
#[derive(Debug, Clone)]
pub struct Passkeys {
    webauthn: Webauthn,
    rp_id: String,
}

impl Passkeys {
    /// Configure la partie de confiance pour l'origine de l'instance (`https://hote[:port]`) :
    /// `rp_id` est son hôte, seule origine admise.
    ///
    /// # Errors
    /// [`ErreurPasskey::Origine`] si l'origine n'est pas une URL à hôte nommé.
    pub fn depuis_origine(origine: &str) -> Result<Self, ErreurPasskey> {
        let url = Url::parse(origine).map_err(|_| ErreurPasskey::Origine)?;
        let hote = match url.host() {
            Some(url::Host::Domain(domaine)) => domaine.to_owned(),
            _ => return Err(ErreurPasskey::Origine),
        };
        let webauthn = WebauthnBuilder::new(&hote, &url)
            .map_err(|_| ErreurPasskey::Origine)?
            .rp_name("Sigma Books")
            .build()
            .map_err(|_| ErreurPasskey::Origine)?;
        Ok(Self {
            webauthn,
            rp_id: hote,
        })
    }

    /// Début d'un enrôlement. `existantes` : les passkeys déjà enregistrées de l'utilisateur,
    /// exclues pour qu'un même authentificateur ne s'enregistre pas deux fois.
    ///
    /// # Errors
    /// [`ErreurPasskey::Refusee`].
    pub fn debut_enrolement(
        &self,
        utilisateur: Uuid,
        courriel: &str,
        existantes: &[Passkey],
    ) -> Result<(CreationChallengeResponse, PasskeyRegistration), ErreurPasskey> {
        let exclues = (!existantes.is_empty())
            .then(|| existantes.iter().map(|p| p.cred_id().clone()).collect());
        self.webauthn
            .start_passkey_registration(utilisateur, courriel, courriel, exclues)
            .map_err(|_| ErreurPasskey::Refusee)
    }

    /// Fin d'un enrôlement : la passkey à enregistrer, si la réponse est valide.
    ///
    /// # Errors
    /// [`ErreurPasskey::Refusee`].
    pub fn fin_enrolement(
        &self,
        reponse: &RegisterPublicKeyCredential,
        etat: &PasskeyRegistration,
    ) -> Result<Passkey, ErreurPasskey> {
        self.webauthn
            .finish_passkey_registration(reponse, etat)
            .map_err(|_| ErreurPasskey::Refusee)
    }

    /// Début d'une authentification parmi `passkeys` (au moins une).
    ///
    /// # Errors
    /// [`ErreurPasskey::Refusee`], notamment sans passkey.
    pub fn debut_authentification(
        &self,
        passkeys: &[Passkey],
    ) -> Result<(RequestChallengeResponse, PasskeyAuthentication), ErreurPasskey> {
        self.webauthn
            .start_passkey_authentication(passkeys)
            .map_err(|_| ErreurPasskey::Refusee)
    }

    /// Fin d'une authentification : résultat vérifié (signature, défi, origine, vérification de
    /// l'utilisateur, compteur qui n'a pas régressé).
    ///
    /// # Errors
    /// [`ErreurPasskey::Clonage`] si le compteur régresse ; [`ErreurPasskey::Refusee`] sinon.
    pub fn fin_authentification(
        &self,
        reponse: &PublicKeyCredential,
        etat: &PasskeyAuthentication,
    ) -> Result<AuthenticationResult, ErreurPasskey> {
        let resultat = self
            .webauthn
            .finish_passkey_authentication(reponse, etat)
            .map_err(|e| match e {
                WebauthnError::CredentialPossibleCompromise => ErreurPasskey::Clonage,
                _ => ErreurPasskey::Refusee,
            })?;
        // Défense en profondeur : la vérification de l'utilisateur est exigée par le défi ;
        // on la contrôle encore sur le résultat, quoi que fasse l'authentificateur.
        if resultat.user_verified() {
            Ok(resultat)
        } else {
            Err(ErreurPasskey::Refusee)
        }
    }

    /// Faux défi d'authentification pour un courriel inconnu ou sans passkey : même forme qu'un
    /// vrai (voir la documentation du module). Rien n'en est stocké : aucune réponse ne le
    /// terminera.
    ///
    /// # Errors
    /// [`ErreurPasskey::Alea`].
    pub fn defi_factice(
        &self,
        cle: &CleInstance,
        courriel: &str,
    ) -> Result<RequestChallengeResponse, ErreurPasskey> {
        let mut defi = [0u8; 32];
        tirer(&mut defi).map_err(|_| ErreurPasskey::Alea)?;
        // Nombre et longueurs tirés, par courriel, dans les distributions de vrais identifiants :
        // un à dix (plus souvent peu), de 16 à 128 octets (longueurs courantes des
        // authentificateurs). Stables d'un appel à l'autre.
        let graine = cle.octets_factices(courriel, 0, 2);
        let zeros = u16::from_be_bytes([graine[0], graine[1]]).leading_zeros();
        let nombre = 1 + usize::try_from(zeros.min(9)).unwrap_or(0);
        let identifiants = (1..=nombre)
            .map(|rang| {
                let rang = u8::try_from(rang).unwrap_or(u8::MAX);
                let choix = cle.octets_factices(courriel, rang, 1)[0];
                let longueur = LONGUEURS_FACTICES[usize::from(choix) % LONGUEURS_FACTICES.len()];
                AllowCredentials {
                    type_: "public-key".to_owned(),
                    id: cle.octets_factices(courriel, rang, longueur),
                    transports: None,
                }
            })
            .collect();
        let reponse = RequestChallengeResponse {
            public_key: PublicKeyCredentialRequestOptions {
                challenge: defi.to_vec(),
                timeout: Some(300_000),
                rp_id: self.rp_id.clone(),
                allow_credentials: identifiants,
                user_verification: UserVerificationPolicy::Required,
                hints: None,
                extensions: None,
            },
            mediation: None,
        };
        defi.zeroize();
        Ok(reponse)
    }
}

/// Jeton d'une cérémonie : 32 octets secrets, effacés à la libération. Ni `Display`, ni `Debug`
/// lisible, ni `Clone`.
pub struct JetonDefi([u8; OCTETS_JETON]);

impl Drop for JetonDefi {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

impl std::fmt::Debug for JetonDefi {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("JetonDefi(<masqué>)")
    }
}

impl JetonDefi {
    /// Tire un jeton (aléa du système).
    ///
    /// # Errors
    /// [`ErreurPasskey::Alea`].
    pub fn tirer() -> Result<Self, ErreurPasskey> {
        let mut octets = [0u8; OCTETS_JETON];
        tirer(&mut octets).map_err(|_| ErreurPasskey::Alea)?;
        Ok(Self(octets))
    }

    /// Lit un jeton transmis : exactement 43 caractères base64url. `None` sinon.
    #[must_use]
    pub fn depuis_texte(texte: &str) -> Option<Self> {
        if texte.len() != LONGUEUR_TEXTE {
            return None;
        }
        let mut octets = [0u8; OCTETS_JETON];
        let lus = Base64UrlUnpadded::decode(texte, &mut octets).ok()?;
        (lus.len() == OCTETS_JETON).then_some(Self(octets))
    }

    /// Empreinte SHA-256, seule forme stockée en base.
    #[must_use]
    pub fn empreinte(&self) -> [u8; 32] {
        Sha256::digest(self.0).into()
    }

    /// `Set-Cookie` qui pose le jeton pour [`VALIDITE_DEFI`].
    #[must_use]
    pub fn cookie(&self) -> String {
        format!(
            "{NOM_COOKIE_CEREMONIE}={}; Max-Age={}; Path=/; Secure; HttpOnly; SameSite=Strict",
            Base64UrlUnpadded::encode_string(&self.0),
            VALIDITE_DEFI.whole_seconds()
        )
    }
}

/// `Set-Cookie` qui efface le cookie de cérémonie (cérémonie terminée ou refusée).
#[must_use]
pub fn cookie_ceremonie_efface() -> String {
    format!("{NOM_COOKIE_CEREMONIE}=; Max-Age=0; Path=/; Secure; HttpOnly; SameSite=Strict")
}

/// Jeton de cérémonie du cookie de la requête. `None` s'il est absent, malformé, ou présent
/// plusieurs fois (ambiguïté : refus).
#[must_use]
pub fn jeton_de_la_requete(en_tetes: &axum::http::HeaderMap) -> Option<JetonDefi> {
    let mut trouve = None;
    for valeur in en_tetes.get_all(axum::http::header::COOKIE) {
        for paire in valeur.to_str().ok()?.split(';') {
            let Some((nom, valeur)) = paire.trim().split_once('=') else {
                continue;
            };
            if nom == NOM_COOKIE_CEREMONIE {
                if trouve.is_some() {
                    return None;
                }
                trouve = Some(valeur);
            }
        }
    }
    JetonDefi::depuis_texte(trouve?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn passkeys() -> Passkeys {
        Passkeys::depuis_origine("https://compta.exemple.invalid").expect("origine")
    }

    #[test]
    fn la_partie_de_confiance_est_l_hote_de_l_origine() {
        assert_eq!(passkeys().rp_id, "compta.exemple.invalid");
    }

    #[test]
    fn une_adresse_ip_n_est_pas_une_partie_de_confiance() {
        for origine in [
            "https://127.0.0.1:8443",
            "https://[::1]",
            "pas une url",
            "https:",
        ] {
            assert_eq!(
                Passkeys::depuis_origine(origine).err(),
                Some(ErreurPasskey::Origine),
                "{origine}"
            );
        }
    }

    #[test]
    fn le_faux_defi_est_stable_pour_un_courriel_et_distinct_d_un_autre() {
        let cle = CleInstance::depuis_octets([7; 32]);
        let p = passkeys();
        let a1 = p
            .defi_factice(&cle, "A@exemple.invalid")
            .expect("faux défi");
        let a2 = p
            .defi_factice(&cle, "a@exemple.invalid")
            .expect("faux défi");
        let b = p
            .defi_factice(&cle, "b@exemple.invalid")
            .expect("faux défi");
        let ids = |r: &RequestChallengeResponse| {
            r.public_key
                .allow_credentials
                .iter()
                .map(|c| c.id.clone())
                .collect::<Vec<_>>()
        };
        // Mêmes identifiants (casse du courriel comprise), défi différent à chaque appel.
        assert_eq!(ids(&a1), ids(&a2));
        assert_ne!(a1.public_key.challenge, a2.public_key.challenge);
        assert_ne!(ids(&a1), ids(&b));
        assert!((1..=10).contains(&ids(&a1).len()));
        assert_eq!(a1.public_key.rp_id, "compta.exemple.invalid");
        assert_eq!(
            a1.public_key.user_verification,
            UserVerificationPolicy::Required
        );
    }

    #[test]
    fn le_jeton_de_ceremonie_fait_l_aller_retour() {
        let jeton = JetonDefi::tirer().expect("aléa");
        let cookie = jeton.cookie();
        assert!(cookie.starts_with("__Host-sigma_ceremonie="));
        for attribut in [
            "Secure",
            "HttpOnly",
            "SameSite=Strict",
            "Path=/",
            "Max-Age=300",
        ] {
            assert!(cookie.contains(attribut), "{attribut}");
        }
        assert!(!cookie.contains("Domain"));
        let valeur = cookie
            .split(';')
            .next()
            .and_then(|p| p.split_once('='))
            .map(|(_, v)| v.to_owned())
            .expect("valeur");
        let mut en_tetes = axum::http::HeaderMap::new();
        en_tetes.insert(
            axum::http::header::COOKIE,
            axum::http::HeaderValue::try_from(format!("autre=1; {NOM_COOKIE_CEREMONIE}={valeur}"))
                .expect("en-tête"),
        );
        let relu = jeton_de_la_requete(&en_tetes).expect("jeton relu");
        assert_eq!(relu.empreinte(), jeton.empreinte());
        assert_eq!(format!("{jeton:?}"), "JetonDefi(<masqué>)");
    }

    #[test]
    fn un_jeton_absent_malforme_ou_double_est_refuse() {
        let mut en_tetes = axum::http::HeaderMap::new();
        assert!(jeton_de_la_requete(&en_tetes).is_none());
        for mauvais in ["", "court", &"é".repeat(43), &"A".repeat(44)] {
            en_tetes.insert(
                axum::http::header::COOKIE,
                axum::http::HeaderValue::try_from(format!("{NOM_COOKIE_CEREMONIE}={mauvais}"))
                    .expect("en-tête"),
            );
            assert!(jeton_de_la_requete(&en_tetes).is_none(), "{mauvais}");
        }
        let jeton = JetonDefi::tirer().expect("aléa");
        let valeur = jeton
            .cookie()
            .split(';')
            .next()
            .map(str::to_owned)
            .expect("paire");
        en_tetes.insert(
            axum::http::header::COOKIE,
            axum::http::HeaderValue::try_from(format!("{valeur}; {valeur}")).expect("en-tête"),
        );
        assert!(jeton_de_la_requete(&en_tetes).is_none());
    }

    #[test]
    fn deux_jetons_different_et_leur_empreinte_aussi() {
        let a = JetonDefi::tirer().expect("aléa");
        let b = JetonDefi::tirer().expect("aléa");
        assert_ne!(a.empreinte(), b.empreinte());
    }
}
