//! Passkeys, WebAuthn (ADR 0006, ADR 0012 ; L2 PR 8c) : connexion, ré-authentification,
//! enrôlement et suppression. Routes : `routes/passkeys.rs` ; protocole :
//! [`crate::facteurs::passkey`] ; base : [`crate::base::passkeys`]. La session, le contexte
//! (`Soi`), l'ouverture de session et l'audit sont ceux de la méthode de secours
//! ([`super::facteurs`]).
//!
//! # Une cérémonie, deux appels
//! 1. **Défi** : le serveur calcule les options WebAuthn, garde l'état de la cérémonie en base
//!    (à usage unique, 5 minutes) et ne remet au navigateur que les options et un jeton, dans le
//!    cookie `__Host-sigma_ceremonie` (seule son empreinte est stockée) ;
//! 2. **Réponse** : le navigateur renvoie la réponse de l'authentificateur ; le serveur retrouve
//!    **et supprime** l'état par l'empreinte du jeton, puis fait vérifier la réponse par
//!    `webauthn-rs`. Une réponse rejouée, un défi échu ou d'une autre cérémonie ne passent pas.
//!
//! # Décisions Cedar (règle absolue 7)
//! Comme pour le secours : `SeConnecter` (principal anonyme) avant toute lecture, puis
//! `OuvrirSession` une fois la passkey vérifiée et consommée ; pour les routes authentifiées, la
//! session est lue en base et Cedar décide l'action (`EnrolerPasskey`, `SupprimerPasskey`,
//! `SeReauthentifier`) sur son contexte (nature, méthode, fraîcheur). La RLS borne tout à
//! l'utilisateur de la session.
//!
//! # Anti-énumération
//! Le défi d'une connexion rend la même forme (`{"defi": …}`, cookie compris) pour un courriel
//! inconnu, un compte sans passkey et un compte plafonné : faux identifiants dérivés du
//! courriel par la clé de l'instance ([`Passkeys::defi_factice`]), rien d'écrit. Durée
//! plancher identique en cas de succès comme d'échec. Une réponse refusée rend toujours
//! 401 `non_authentifie`.
//!
//! # Journal
//! La connexion par passkey, l'enrôlement et la suppression sont inscrits aux événements de
//! l'utilisateur (ADR 0017) et à la chaîne d'audit de chaque cabinet où il a une appartenance
//! active, dans la transaction de l'action, comme pour le secours.

use axum::Json;
use axum::extract::{FromRequestParts, RawPathParams};
use axum::http::request::Parts;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use webauthn_rs::prelude::{
    Passkey, PasskeyAuthentication, PasskeyRegistration, PublicKeyCredential,
    RegisterPublicKeyCredential,
};

use super::Etat;
use super::erreur::{CategorieErreur, ErreurApi, MotifRefus};
use super::facteurs::{
    Client, Soi, cle_instance, corps_json, courriel_limite, interne, ouvrir, plafonne,
    refuser_connexion, terminer_reauthentification,
};
use super::pipeline::id_chemin;
use super::routes::passkeys::Cas;
use super::session::jeton_de_la_requete as jeton_de_session;
use crate::base::evenement::{self, Issue, NatureEvenement};
use crate::base::facteurs::{self as base_facteurs, ouvrir_transaction_connexion};
use crate::base::passkeys::{self as base_passkeys, NatureDefi, NouveauDefi, PasskeyLue};
use crate::base::session::{self, MethodeSession, MotifRevocation, NatureSession};
use crate::facteurs::passkey::{
    DEFIS_CONNEXION_MAX, JetonDefi, PASSKEYS_MAX, Passkeys, VALIDITE_DEFI, cookie_ceremonie_efface,
    jeton_de_la_requete as jeton_de_ceremonie,
};

/// Nombre maximal de caractères du nom d'une passkey (contrainte de la table).
const NOM_MAX: usize = 100;

fn passkeys(etat: &Etat) -> Result<&Passkeys, ErreurApi> {
    etat.facteurs
        .passkeys
        .as_ref()
        .ok_or(ErreurApi::Interne(CategorieErreur::Authentification))
}

/// Ajoute un `Set-Cookie` à la réponse (sans écraser ceux déjà posés).
fn ajouter_cookie(mut reponse: Response, valeur: &str) -> Result<Response, ErreurApi> {
    let valeur = HeaderValue::try_from(valeur).map_err(interne)?;
    reponse.headers_mut().append(header::SET_COOKIE, valeur);
    Ok(reponse)
}

/// Options WebAuthn rendues au navigateur, dans l'enveloppe `{"defi": …}`.
#[derive(Debug, Serialize)]
struct VueDefi<T> {
    defi: T,
}

/// Réponse d'un défi : les options et le cookie du jeton de cérémonie.
fn reponse_defi<T: Serialize>(options: T, jeton: &JetonDefi) -> Result<Response, ErreurApi> {
    ajouter_cookie(
        Json(VueDefi { defi: options }).into_response(),
        &jeton.cookie(),
    )
}

fn etat_json<T: Serialize>(etat: &T) -> Result<String, ErreurApi> {
    serde_json::to_string(etat).map_err(interne)
}

/// Résultat d'une authentification vérifiée, prêt pour la mise à jour conditionnelle.
struct Verifiee {
    id: Uuid,
    etat_lu: String,
    nouvel_etat: String,
}

/// Fait vérifier la réponse de l'authentificateur contre l'état de la cérémonie, puis cherche
/// la passkey signataire parmi celles du compte et calcule son nouvel état (compteur). `None`
/// à la moindre irrégularité : l'appelant refuse sans rien dire de la cause.
fn verifier_authentification(
    passkeys: &Passkeys,
    lues: &[PasskeyLue],
    etat_ceremonie: &str,
    reponse: &PublicKeyCredential,
) -> Option<Verifiee> {
    let etat: PasskeyAuthentication = serde_json::from_str(etat_ceremonie).ok()?;
    let resultat = passkeys.fin_authentification(reponse, &etat).ok()?;
    let lue = lues
        .iter()
        .find(|l| l.passkey.cred_id() == resultat.cred_id())?;
    let mut passkey = lue.passkey.clone();
    passkey.update_credential(&resultat)?;
    Some(Verifiee {
        id: lue.id,
        etat_lu: lue.etat_lu.clone(),
        nouvel_etat: base_passkeys::serialiser(&passkey).ok()?,
    })
}

// --- Connexion -------------------------------------------------------------------------------

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CorpsDefiConnexion {
    courriel: String,
}

/// Défi de connexion par passkey : options réelles pour un compte qui a des passkeys et n'est
/// pas plafonné, faux défi de même forme sinon (anti-énumération).
pub(super) async fn defi_connexion(etat: &Etat, corps: &[u8]) -> Result<Response, ErreurApi> {
    let corps: CorpsDefiConnexion = corps_json(corps, ErreurApi::NonAuthentifie)?;
    let cle = cle_instance(etat)?;
    let passkeys = passkeys(etat)?;
    let courriel = courriel_limite(etat, &cle, &corps.courriel)?;
    let maintenant = etat.horloge.maintenant();

    let mut tx = ouvrir_transaction_connexion(&etat.pool, &courriel).await?;
    let lues = tx.lire_passkeys().await?;
    let utilisateur = lues.first().map(|p| p.utilisateur);
    let plafonne = plafonne(etat, &mut tx, utilisateur, maintenant).await?;
    let jeton = JetonDefi::tirer().map_err(interne)?;
    let options = match utilisateur {
        Some(utilisateur) if !plafonne => {
            let cles: Vec<Passkey> = lues.iter().map(|p| p.passkey.clone()).collect();
            let (options, etat_auth) = passkeys.debut_authentification(&cles).map_err(interne)?;
            tx.creer_defi(
                &NouveauDefi {
                    utilisateur,
                    nature: NatureDefi::Connexion,
                    session: None,
                    jeton_hache: jeton.empreinte(),
                    etat: etat_json(&etat_auth)?,
                    instant: maintenant,
                    expire_le: maintenant + VALIDITE_DEFI,
                },
                DEFIS_CONNEXION_MAX,
            )
            .await?;
            options
        }
        _ => passkeys.defi_factice(&cle, &courriel).map_err(interne)?,
    };
    tx.confirmer().await?;
    reponse_defi(options, &jeton)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CorpsConnexionPasskey {
    courriel: String,
    credential: PublicKeyCredential,
}

/// Connexion par passkey : le défi est consommé, la réponse vérifiée, l'état de la passkey
/// remplacé (compteur), puis la session ouverte, dans une seule transaction.
pub(super) async fn connecter_passkey(
    etat: &Etat,
    client: &Client,
    parts: &Parts,
    corps: &[u8],
) -> Result<Response, ErreurApi> {
    let corps: CorpsConnexionPasskey = corps_json(corps, ErreurApi::NonAuthentifie)?;
    let cle = cle_instance(etat)?;
    let passkeys = passkeys(etat)?;
    let courriel = courriel_limite(etat, &cle, &corps.courriel)?;
    let jeton = jeton_de_ceremonie(&parts.headers).ok_or(ErreurApi::NonAuthentifie)?;
    let maintenant = etat.horloge.maintenant();

    let mut tx = ouvrir_transaction_connexion(&etat.pool, &courriel).await?;
    let lues = tx.lire_passkeys().await?;
    let utilisateur = match lues.first() {
        Some(p) => Some(p.utilisateur),
        None => tx.lire_utilisateur().await?,
    };
    let plafonne = plafonne(etat, &mut tx, utilisateur, maintenant).await?;
    // Le défi est consommé dans tous les cas : une cérémonie ne sert qu'une fois.
    let defi = tx.consommer_defi(&jeton.empreinte(), maintenant).await?;
    // Courriel inconnu : rien n'est inscrit (ADR 0017).
    let Some(utilisateur) = utilisateur else {
        return Err(ErreurApi::NonAuthentifie);
    };
    let preuve = match defi {
        Some(defi) if !plafonne && defi.utilisateur == utilisateur => {
            match verifier_authentification(passkeys, &lues, &defi.etat, &corps.credential) {
                Some(v) => {
                    tx.consommer_passkey(utilisateur, v.id, &v.etat_lu, &v.nouvel_etat, maintenant)
                        .await?
                }
                None => None,
            }
        }
        _ => None,
    };
    let Some(preuve) = preuve else {
        return refuser_connexion(tx, utilisateur, MethodeSession::Passkey, maintenant, client)
            .await;
    };
    let tx = tx.authentifier(preuve).await?;
    let reponse = ouvrir(
        etat,
        tx,
        utilisateur,
        NatureSession::Complete,
        MethodeSession::Passkey,
        maintenant,
        client,
    )
    .await?;
    ajouter_cookie(reponse, &cookie_ceremonie_efface())
}

// --- Routes d'un utilisateur authentifié -----------------------------------------------------

/// Exécute un cas d'usage des passkeys de l'utilisateur authentifié.
pub(super) async fn executer(
    soi: &Soi<'_>,
    cas: Cas,
    parts: &Parts,
    corps: &[u8],
) -> Result<Response, ErreurApi> {
    match cas {
        Cas::DefiReauthentification => defi_reauthentification(soi).await,
        Cas::ReauthentifierPasskey => reauthentifier_passkey(soi, parts, corps).await,
        Cas::DefiEnrolement => defi_enrolement(soi).await,
        Cas::EnrolerPasskey => enroler_passkey(soi, parts, corps).await,
        Cas::SupprimerPasskey => supprimer_passkey(soi, parts).await,
        Cas::DefiConnexion | Cas::ConnexionPasskey => {
            Err(ErreurApi::Interne(CategorieErreur::Donnee))
        }
    }
}

fn nouveau_defi(
    soi: &Soi<'_>,
    nature: NatureDefi,
    session: crate::base::IdSession,
    jeton: &JetonDefi,
    etat: String,
) -> NouveauDefi {
    NouveauDefi {
        utilisateur: soi.utilisateur(),
        nature,
        session: Some(session),
        jeton_hache: jeton.empreinte(),
        etat,
        instant: soi.maintenant,
        expire_le: soi.maintenant + VALIDITE_DEFI,
    }
}

// --- Ré-authentification ---------------------------------------------------------------------

async fn defi_reauthentification(soi: &Soi<'_>) -> Result<Response, ErreurApi> {
    soi.limiter()?;
    let passkeys = passkeys(soi.etat)?;
    let mut tx = soi.transaction().await?;
    let (s, _) = soi.decider(&mut tx).await?;
    let lues = base_passkeys::lister(&mut tx).await?;
    if lues.is_empty() {
        return Err(ErreurApi::Refus(MotifRefus::AucunePasskey));
    }
    let cles: Vec<Passkey> = lues.iter().map(|p| p.passkey.clone()).collect();
    let (options, etat) = passkeys.debut_authentification(&cles).map_err(interne)?;
    let jeton = JetonDefi::tirer().map_err(interne)?;
    base_passkeys::creer_defi(
        &mut tx,
        &nouveau_defi(
            soi,
            NatureDefi::Reauthentification,
            s.id,
            &jeton,
            etat_json(&etat)?,
        ),
    )
    .await?;
    tx.confirmer().await?;
    reponse_defi(options, &jeton)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CorpsReauthentificationPasskey {
    credential: PublicKeyCredential,
}

async fn reauthentifier_passkey(
    soi: &Soi<'_>,
    parts: &Parts,
    corps: &[u8],
) -> Result<Response, ErreurApi> {
    soi.limiter()?;
    let corps: CorpsReauthentificationPasskey = corps_json(corps, ErreurApi::RequeteInvalide)?;
    let passkeys = passkeys(soi.etat)?;
    let present = jeton_de_session(parts).ok_or(ErreurApi::NonAuthentifie)?;
    let refus = ErreurApi::Refus(MotifRefus::FacteursRefuses);
    let jeton = jeton_de_ceremonie(&parts.headers).ok_or(refus)?;

    let mut tx = soi.transaction().await?;
    let (s, _) = soi.decider(&mut tx).await?;
    let lues = base_passkeys::lister(&mut tx).await?;
    // Le défi est consommé dans tous les cas : une cérémonie ne sert qu'une fois.
    let defi = base_passkeys::consommer_defi(
        &mut tx,
        NatureDefi::Reauthentification,
        s.id,
        &jeton.empreinte(),
        soi.maintenant,
    )
    .await?;
    let verifiee =
        defi.and_then(|etat| verifier_authentification(passkeys, &lues, &etat, &corps.credential));
    let admis = match &verifiee {
        Some(v) => {
            base_passkeys::consommer_passkey(
                &mut tx,
                v.id,
                &v.etat_lu,
                &v.nouvel_etat,
                soi.maintenant,
            )
            .await?
        }
        None => false,
    };
    if !admis {
        evenement::inscrire(
            &mut tx,
            &soi.evenement(
                NatureEvenement::Reauthentification,
                Some(MethodeSession::Passkey),
                Issue::Echec,
            ),
        )
        .await?;
        tx.confirmer().await?;
        return Err(ErreurApi::Refus(MotifRefus::FacteursRefuses));
    }
    let reponse =
        terminer_reauthentification(soi, tx, &s, &present, MethodeSession::Passkey).await?;
    ajouter_cookie(reponse, &cookie_ceremonie_efface())
}

// --- Enrôlement ------------------------------------------------------------------------------

async fn defi_enrolement(soi: &Soi<'_>) -> Result<Response, ErreurApi> {
    soi.limiter()?;
    let passkeys = passkeys(soi.etat)?;
    let mut tx = soi.transaction().await?;
    let (s, _) = soi.decider(&mut tx).await?;
    let courriel = base_facteurs::lire_courriel(&mut tx)
        .await?
        .ok_or(ErreurApi::NonAuthentifie)?;
    let lues = base_passkeys::lister(&mut tx).await?;
    if i64::try_from(lues.len()).unwrap_or(i64::MAX) >= PASSKEYS_MAX {
        return Err(ErreurApi::Refus(MotifRefus::PasskeysMax));
    }
    let existantes: Vec<Passkey> = lues.into_iter().map(|p| p.passkey).collect();
    let (options, etat) = passkeys
        .debut_enrolement(soi.utilisateur().uuid(), &courriel, &existantes)
        .map_err(interne)?;
    let jeton = JetonDefi::tirer().map_err(interne)?;
    base_passkeys::creer_defi(
        &mut tx,
        &nouveau_defi(soi, NatureDefi::Enrolement, s.id, &jeton, etat_json(&etat)?),
    )
    .await?;
    tx.confirmer().await?;
    reponse_defi(options, &jeton)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CorpsEnrolementPasskey {
    nom: String,
    credential: RegisterPublicKeyCredential,
}

/// Passkey enregistrée : son identifiant et son nom.
#[derive(Debug, Serialize)]
struct VuePasskeyEnrolee {
    id: String,
    nom: String,
}

/// Nom saisi : sans espaces autour, 1 à 100 caractères, sans caractère de contrôle.
fn nom_saisi(texte: &str) -> Option<String> {
    let texte = texte.trim();
    let longueur = texte.chars().count();
    ((1..=NOM_MAX).contains(&longueur) && !texte.chars().any(char::is_control))
        .then(|| texte.to_owned())
}

async fn enroler_passkey(
    soi: &Soi<'_>,
    parts: &Parts,
    corps: &[u8],
) -> Result<Response, ErreurApi> {
    soi.limiter()?;
    let corps: CorpsEnrolementPasskey = corps_json(corps, ErreurApi::RequeteInvalide)?;
    let refus = ErreurApi::Refus(MotifRefus::PasskeyRefusee);
    let nom = nom_saisi(&corps.nom).ok_or(refus)?;
    let passkeys = passkeys(soi.etat)?;
    let jeton = jeton_de_ceremonie(&parts.headers).ok_or(refus)?;

    let mut tx = soi.transaction().await?;
    // Décision revérifiée sur l'état exact de la session, dans la transaction de l'action.
    let (s, verdict) = soi.decider(&mut tx).await?;
    // Le défi est consommé dans tous les cas : une cérémonie ne sert qu'une fois.
    let defi = base_passkeys::consommer_defi(
        &mut tx,
        NatureDefi::Enrolement,
        s.id,
        &jeton.empreinte(),
        soi.maintenant,
    )
    .await?;
    let passkey = defi
        .and_then(|etat| serde_json::from_str::<PasskeyRegistration>(&etat).ok())
        .and_then(|etat| passkeys.fin_enrolement(&corps.credential, &etat).ok());
    let Some(passkey) = passkey else {
        evenement::inscrire(
            &mut tx,
            &soi.evenement(
                NatureEvenement::EnrolementPasskey,
                Some(MethodeSession::Passkey),
                Issue::Echec,
            ),
        )
        .await?;
        tx.confirmer().await?;
        return Err(refus);
    };
    let existantes = base_passkeys::lister(&mut tx).await?;
    if i64::try_from(existantes.len()).unwrap_or(i64::MAX) >= PASSKEYS_MAX {
        return Err(ErreurApi::Refus(MotifRefus::PasskeysMax));
    }
    let id = base_passkeys::inserer(&mut tx, &passkey, &nom, soi.maintenant)
        .await?
        .ok_or(ErreurApi::Refus(MotifRefus::PasskeyDejaEnregistree))?;
    // Un changement de facteur révoque les autres sessions (ADR 0012).
    session::revoquer_autres(
        &mut tx,
        s.id,
        soi.maintenant,
        MotifRevocation::ChangementFacteur,
    )
    .await?;
    evenement::inscrire(
        &mut tx,
        &soi.evenement(
            NatureEvenement::EnrolementPasskey,
            Some(MethodeSession::Passkey),
            Issue::Succes,
        ),
    )
    .await?;
    soi.auditer_et_confirmer(tx, &verdict, &s, MethodeSession::Passkey)
        .await?;
    let reponse = (
        StatusCode::CREATED,
        Json(VuePasskeyEnrolee {
            id: id.to_string(),
            nom,
        }),
    )
        .into_response();
    ajouter_cookie(reponse, &cookie_ceremonie_efface())
}

// --- Suppression -----------------------------------------------------------------------------

async fn supprimer_passkey(soi: &Soi<'_>, parts: &Parts) -> Result<Response, ErreurApi> {
    let mut parts = parts.clone();
    let parametres = RawPathParams::from_request_parts(&mut parts, &())
        .await
        .map_err(|_| ErreurApi::RequeteInvalide)?;
    let id = id_chemin(&parametres, "passkey")?;
    let cle = cle_instance(soi.etat)?;

    let mut tx = soi.transaction().await?;
    let (s, verdict) = soi.decider(&mut tx).await?;
    let lues = base_passkeys::lister(&mut tx).await?;
    if !lues.iter().any(|p| p.id == id) {
        return Err(ErreurApi::Introuvable);
    }
    // F1.1 : le compte garde au moins un facteur fort, une autre passkey ou un secours
    // utilisable (son secret TOTP lisible par la clé courante de l'instance).
    let secours_utilisable = base_facteurs::lire_secours(&mut tx)
        .await?
        .is_some_and(|sec| sec.totp_cle_id == cle.identifiant());
    if lues.len() <= 1 && !secours_utilisable {
        return Err(ErreurApi::Refus(MotifRefus::DernierFacteur));
    }
    if !base_passkeys::supprimer(&mut tx, id).await? {
        return Err(ErreurApi::Introuvable);
    }
    session::revoquer_autres(
        &mut tx,
        s.id,
        soi.maintenant,
        MotifRevocation::ChangementFacteur,
    )
    .await?;
    evenement::inscrire(
        &mut tx,
        &soi.evenement(
            NatureEvenement::SuppressionPasskey,
            Some(MethodeSession::Passkey),
            Issue::Succes,
        ),
    )
    .await?;
    soi.auditer_et_confirmer(tx, &verdict, &s, MethodeSession::Passkey)
        .await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nom_saisi_borne() {
        assert_eq!(nom_saisi("  Téléphone "), Some("Téléphone".to_owned()));
        assert_eq!(nom_saisi(&"é".repeat(100)), Some("é".repeat(100)));
        for invalide in ["", "   ", "a\nb", "a\u{7}", &"a".repeat(101)] {
            assert_eq!(nom_saisi(invalide), None, "{invalide:?}");
        }
    }
}
