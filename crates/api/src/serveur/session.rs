//! Sessions (ADR 0012 ; L2-17 ; F1.2, F1.4, F2.11 ; ASVS 4.0.3 V3) : jeton, cookie, anti-CSRF
//! lié à la session, horloge, et implémentation PostgreSQL de [`Authentification`], la seule du
//! binaire.
//!
//! # Configuration
//! Lue dans l'environnement par [`super::config::Configuration`] ([`configurer`]).
//!
//! | Variable | Obligatoire | Contenu |
//! |---|---|---|
//! | `SIGMA_CONSERVATION_SESSIONS_JOURS` | non | Jours de conservation d'une session close avant sa purge (1 à 3650 ; 30 par défaut, à confirmer par le mainteneur, loi 2008-12) |
//!
//! # Jeton
//! 256 bits tirés par le système (`getrandom`), transmis en base64url sans remplissage
//! (43 caractères) dans le cookie [`NOM_COOKIE`]. Seule son empreinte SHA-256 est stockée
//! (NIST SP 800-63B §5.1.2.2 et ASVS V2.6.2 admettent un hachage non salé au-delà de 112 bits
//! d'aléa). Le jeton n'a ni `Display` ni `Debug` lisible : il ne peut pas atteindre un journal.
//!
//! # Cookie
//! `__Host-sigma_session=<jeton>; Max-Age=<reste>; Path=/; Secure; HttpOnly; SameSite=Strict`,
//! sans `Domain` (préfixe `__Host-`). `Max-Age` vaut le reste de la durée maximale de la session
//! (12 heures pour une session complète) : le navigateur l'oublie au plus tard à son échéance.
//!
//! # Anti-CSRF lié à la session
//! `base64url(SHA-256("sigma-books csrf v1\0" ‖ jeton))`, où `jeton` est la suite des 32 octets.
//! Dérivé, jamais stocké ; il change avec le jeton (rotation) et ne permet de remonter ni au
//! jeton ni à son empreinte stockée. L'interface le reçoit par `GET /api/session` (ou la
//! rotation), le garde en mémoire et le renvoie dans l'en-tête [`EN_TETE_CSRF`] de toute
//! mutation ; il est comparé **en temps constant** (`ctutils`). Les contrôles de
//! [`super::csrf`] (`Sec-Fetch-Site`, `Origin`, présence de l'en-tête) restent en place avant
//! l'authentification.
//!
//! # Authentification d'une requête
//! [`AuthentificationSession`] lit le cookie, hache le jeton et lit la session dans une courte
//! transaction propre (`sigma.jeton_session`) : **chaque requête relit la session**, d'où la
//! révocation immédiate. Refus (`None`, donc 401) : cookie absent, malformé ou en double, jeton
//! inconnu (faux ou remplacé par une rotation), session révoquée, inactive depuis 30 minutes,
//! ouverte depuis plus de 12 heures (15 minutes pour une session restreinte ou d'enrôlement).
//! La nature de la session est rendue : le pipeline refuse une session autre que complète hors
//! des routes de la session elle-même. `derniere_activite` n'est réécrite qu'**après** une
//! requête admise (nature, anti-CSRF, décision : [`Authentification::noter_activite`]), et
//! seulement si elle date de plus de 60 secondes : une requête refusée ne prolonge pas la
//! session.

use std::fmt;
use std::sync::Arc;

use axum::http::header::InvalidHeaderValue;
use axum::http::request::Parts;
use axum::http::{HeaderValue, header};
use base64ct::{Base64UrlUnpadded, Encoding};
use ctutils::CtEq;
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use time::{Duration, OffsetDateTime};
use zeroize::Zeroize;

use super::authentification::{
    Authentification, ErreurAuthentification, Identite, NotationActivite, ResultatAuthentification,
};
use super::config::{Environnement, ErreurConfiguration};
use super::csrf::EN_TETE_CSRF;
use crate::base::session::{self, DureesSession, EmpreinteJeton, NouvelleSession, Session};
use crate::base::{ErreurBase, IdSession, ouvrir_transaction_utilisateur};

/// Nom du cookie de session (préfixe `__Host-` : `Secure`, `Path=/`, sans `Domain`).
pub const NOM_COOKIE: &str = "__Host-sigma_session";

/// Préfixe de dérivation du jeton anti-CSRF (version 1).
const PREFIXE_CSRF: &[u8] = b"sigma-books csrf v1\0";

/// Longueur du jeton en octets (256 bits).
const OCTETS_JETON: usize = 32;

/// Longueur du jeton encodé (base64url sans remplissage).
const LONGUEUR_TEXTE: usize = 43;

/// Source de l'instant courant. Le binaire lit l'horloge du système ([`HorlogeSysteme`]) ; les
/// tests en injectent une autre (expiration par inactivité, au-delà de 12 heures).
pub trait Horloge: Send + Sync + 'static {
    /// L'instant courant.
    fn maintenant(&self) -> OffsetDateTime;
}

/// Horloge du système.
#[derive(Debug, Clone, Copy, Default)]
pub struct HorlogeSysteme;

impl Horloge for HorlogeSysteme {
    fn maintenant(&self) -> OffsetDateTime {
        OffsetDateTime::now_utc()
    }
}

/// Échec du tirage d'un jeton (source d'aléa du système indisponible).
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("aléa du système indisponible")]
pub struct ErreurAlea;

/// Jeton de session : 32 octets secrets. Ni `Display`, ni `Debug` lisible, ni `Clone` ;
/// effacé de la mémoire à sa libération (`zeroize`).
pub struct JetonSession([u8; OCTETS_JETON]);

impl Drop for JetonSession {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

impl fmt::Debug for JetonSession {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("JetonSession(<masqué>)")
    }
}

impl JetonSession {
    /// Tire un nouveau jeton (aléa du système).
    ///
    /// # Errors
    /// [`ErreurAlea`] si le système ne fournit pas d'aléa.
    pub fn tirer() -> Result<Self, ErreurAlea> {
        let mut octets = [0u8; OCTETS_JETON];
        getrandom::fill(&mut octets).map_err(|_| ErreurAlea)?;
        Ok(Self(octets))
    }

    /// Lit un jeton transmis : exactement 43 caractères base64url, 32 octets. `None` sinon.
    #[must_use]
    pub fn depuis_texte(texte: &str) -> Option<Self> {
        if texte.len() != LONGUEUR_TEXTE {
            return None;
        }
        let mut octets = [0u8; OCTETS_JETON];
        let lus = Base64UrlUnpadded::decode(texte, &mut octets).ok()?;
        (lus.len() == OCTETS_JETON).then_some(Self(octets))
    }

    /// Forme transmise dans le cookie. À ne passer qu'à [`cookie`].
    fn texte(&self) -> String {
        Base64UrlUnpadded::encode_string(&self.0)
    }

    /// Empreinte SHA-256, seule forme stockée en base.
    #[must_use]
    pub fn empreinte(&self) -> EmpreinteJeton {
        EmpreinteJeton::new(Sha256::digest(self.0).into())
    }

    /// Jeton anti-CSRF lié à cette session.
    #[must_use]
    pub fn csrf(&self) -> String {
        let mut h = Sha256::new();
        h.update(PREFIXE_CSRF);
        h.update(self.0);
        Base64UrlUnpadded::encode_string(&h.finalize())
    }

    /// Vrai si `presente` est le jeton anti-CSRF de cette session (comparaison en temps
    /// constant ; seule la longueur, publique, peut abréger).
    #[must_use]
    pub fn csrf_conforme(&self, presente: &[u8]) -> bool {
        self.csrf().as_bytes().ct_eq(presente).to_bool()
    }
}

/// En-tête `Set-Cookie` qui pose le jeton, valable `max_age` (secondes entières, au moins 0).
///
/// # Errors
/// Jamais en pratique (le texte est en ASCII).
pub fn cookie(jeton: &JetonSession, max_age: Duration) -> Result<HeaderValue, InvalidHeaderValue> {
    let secondes = max_age.whole_seconds().max(0);
    HeaderValue::try_from(format!(
        "{NOM_COOKIE}={}; Max-Age={secondes}; Path=/; Secure; HttpOnly; SameSite=Strict",
        jeton.texte()
    ))
}

/// En-tête `Set-Cookie` qui efface le cookie (déconnexion).
#[must_use]
pub fn cookie_efface() -> HeaderValue {
    HeaderValue::from_static(
        "__Host-sigma_session=; Max-Age=0; Path=/; Secure; HttpOnly; SameSite=Strict",
    )
}

/// Le jeton du cookie [`NOM_COOKIE`] de la requête. `None` s'il est absent, malformé, ou
/// présent plusieurs fois (ambiguïté : refus).
#[must_use]
pub fn jeton_de_la_requete(requete: &Parts) -> Option<JetonSession> {
    let mut trouve = None;
    for valeur in requete.headers.get_all(header::COOKIE) {
        let texte = valeur.to_str().ok()?;
        for paire in texte.split(';') {
            let Some((nom, valeur)) = paire.trim().split_once('=') else {
                continue;
            };
            if nom == NOM_COOKIE {
                if trouve.is_some() {
                    return None;
                }
                trouve = Some(valeur);
            }
        }
    }
    trouve.and_then(JetonSession::depuis_texte)
}

/// Une session ouverte : identifiant public, jeton (à poser par [`SessionOuverte::cookie`]) et
/// jeton anti-CSRF.
#[derive(Debug)]
pub struct SessionOuverte {
    /// Identifiant public.
    pub id: IdSession,
    /// Jeton, secret.
    pub jeton: JetonSession,
    /// Échéance absolue (ouverture plus durée maximale de sa nature).
    pub fin_absolue: OffsetDateTime,
}

impl SessionOuverte {
    /// `Set-Cookie` de la session, valable jusqu'à sa fin absolue vue de `maintenant`.
    ///
    /// # Errors
    /// Voir [`cookie`].
    pub fn cookie(&self, maintenant: OffsetDateTime) -> Result<HeaderValue, InvalidHeaderValue> {
        cookie(&self.jeton, self.fin_absolue - maintenant)
    }
}

/// Échec de l'ouverture d'une session.
#[derive(Debug, thiserror::Error)]
pub enum ErreurOuverture {
    /// Aléa indisponible.
    #[error(transparent)]
    Alea(#[from] ErreurAlea),
    /// Base de données.
    #[error(transparent)]
    Base(#[from] ErreurBase),
}

/// Ouvre une session pour `nouvelle.utilisateur`, au contexte de cet utilisateur, et purge ses
/// sessions closes depuis plus de [`DureesSession::conservation`] (aucune purge ne traverse
/// plusieurs utilisateurs).
///
/// Fonction interne : en PR 8a, seuls les tests l'appellent ; la connexion par secours (PR 8b)
/// et par passkey (PR 8c) l'appelleront après la vérification du facteur.
///
/// # Errors
/// [`ErreurOuverture`].
pub async fn ouvrir_session(
    pool: &PgPool,
    nouvelle: &NouvelleSession,
    durees: &DureesSession,
) -> Result<SessionOuverte, ErreurOuverture> {
    let jeton = JetonSession::tirer()?;
    let id = IdSession::nouveau();
    let mut tx = ouvrir_transaction_utilisateur(pool, nouvelle.utilisateur).await?;
    session::creer(&mut tx, id, nouvelle, &jeton.empreinte(), durees).await?;
    tx.confirmer().await?;
    // Point d'insertion de l'événement d'audit « session ouverte » (hors cabinet) : en attente
    // de la décision du mainteneur sur la traçabilité de l'authentification (ADR 0012, 0014).
    Ok(SessionOuverte {
        id,
        jeton,
        fin_absolue: nouvelle.instant + durees.duree_max_de(nouvelle.nature),
    })
}

/// Authentification par le cookie de session, lue dans PostgreSQL : seule implémentation de
/// [`Authentification`] du binaire.
pub struct AuthentificationSession {
    pool: PgPool,
    horloge: Arc<dyn Horloge>,
    durees: DureesSession,
}

impl AuthentificationSession {
    /// Authentification sur le pool du rôle applicatif, avec l'horloge et les durées données
    /// (bornées par celles de l'ADR 0012 : [`DureesSession::raccourcies`]).
    #[must_use]
    pub fn new(pool: PgPool, horloge: Arc<dyn Horloge>, durees: DureesSession) -> Self {
        Self {
            pool,
            horloge,
            durees: durees.raccourcies(),
        }
    }

    async fn session_valide(
        &self,
        requete: &Parts,
    ) -> Result<Option<(JetonSession, Session)>, ErreurBase> {
        let Some(jeton) = jeton_de_la_requete(requete) else {
            return Ok(None);
        };
        let empreinte = jeton.empreinte();
        let Some(session) = session::lire_par_empreinte(&self.pool, &empreinte).await? else {
            return Ok(None);
        };
        let maintenant = self.horloge.maintenant();
        if !session.est_valide(maintenant, &self.durees) {
            return Ok(None);
        }
        Ok(Some((jeton, session)))
    }
}

impl Authentification for AuthentificationSession {
    fn authentifier<'a>(&'a self, requete: &'a Parts) -> ResultatAuthentification<'a> {
        Box::pin(async move {
            let Some((jeton, session)) = self
                .session_valide(requete)
                .await
                .map_err(|_| ErreurAuthentification)?
            else {
                return Ok(None);
            };
            let csrf_lie = requete
                .headers
                .get(EN_TETE_CSRF)
                .is_some_and(|v| jeton.csrf_conforme(v.as_bytes()));
            Ok(Some(Identite {
                utilisateur: session.utilisateur,
                session: session.id,
                derniere_verification: session.derniere_verification,
                nature: session.nature,
                csrf_lie,
                derniere_activite: session.derniere_activite,
            }))
        })
    }

    /// Note l'activité d'une requête **admise**, au plus une fois par
    /// [`DureesSession::pas_activite`]. Un échec n'est pas une erreur de la requête, déjà
    /// traitée : la session expirerait seulement plus tôt (sens sûr).
    fn noter_activite<'a>(&'a self, identite: &'a Identite) -> NotationActivite<'a> {
        Box::pin(async move {
            let maintenant = self.horloge.maintenant();
            let avant = maintenant - self.durees.pas_activite;
            if identite.derniere_activite >= avant {
                return;
            }
            let Ok(mut tx) = ouvrir_transaction_utilisateur(&self.pool, identite.utilisateur).await
            else {
                return;
            };
            if session::noter_activite(&mut tx, identite.session, maintenant, avant)
                .await
                .is_ok()
            {
                let _ = tx.confirmer().await;
            }
        })
    }
}

/// Lit les variables du module et les applique aux durées des sessions.
///
/// # Errors
/// [`ErreurConfiguration::Invalide`] si une valeur est hors bornes ou illisible.
pub fn configurer(
    env: &Environnement<'_>,
    durees: &mut DureesSession,
) -> Result<(), ErreurConfiguration> {
    if let Some(jours) = env.facultative("SIGMA_CONSERVATION_SESSIONS_JOURS") {
        let jours: i64 = jours
            .trim()
            .parse()
            .ok()
            .filter(|j| (1..=3650).contains(j))
            .ok_or(ErreurConfiguration::Invalide(
                "SIGMA_CONSERVATION_SESSIONS_JOURS",
            ))?;
        durees.conservation = Duration::days(jours);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use axum::http::Request;

    use super::*;

    fn parts(cookies: &[&str]) -> Parts {
        let mut requete = Request::get("/api/session");
        for c in cookies {
            requete = requete.header(header::COOKIE, *c);
        }
        requete.body(()).unwrap().into_parts().0
    }

    #[test]
    fn jeton_aller_retour_et_formes_refusees() {
        let jeton = JetonSession::tirer().unwrap();
        let texte = jeton.texte();
        assert_eq!(texte.len(), LONGUEUR_TEXTE);
        let relu = JetonSession::depuis_texte(&texte).unwrap();
        assert_eq!(relu.0, jeton.0);
        assert_ne!(JetonSession::tirer().unwrap().0, jeton.0);
        for invalide in [
            "",
            &texte[1..],
            &format!("{texte}A"),
            &format!("{}+", &texte[1..]),
            &format!("{}=", &texte[1..]),
        ] {
            assert!(JetonSession::depuis_texte(invalide).is_none(), "{invalide}");
        }
        assert!(!format!("{jeton:?}").contains(&texte));
    }

    #[test]
    fn csrf_derive_du_jeton_et_compare_strictement() {
        let a = JetonSession([1; 32]);
        let b = JetonSession([2; 32]);
        // Vecteur fixe : base64url(SHA-256("sigma-books csrf v1\0" ‖ 0x01 × 32)).
        let mut h = Sha256::new();
        h.update(b"sigma-books csrf v1\0");
        h.update([1u8; 32]);
        assert_eq!(a.csrf(), Base64UrlUnpadded::encode_string(&h.finalize()));
        assert_eq!(a.csrf().len(), 43);
        assert!(a.csrf_conforme(a.csrf().as_bytes()));
        assert!(!a.csrf_conforme(b.csrf().as_bytes()));
        assert!(!a.csrf_conforme(b""));
        assert!(!a.csrf_conforme(&a.csrf().as_bytes()[1..]));
        // Ni le jeton ni son empreinte.
        assert!(!a.csrf_conforme(a.texte().as_bytes()));
        assert_ne!(
            a.csrf(),
            Base64UrlUnpadded::encode_string(&Sha256::digest(a.0))
        );
    }

    #[test]
    fn attributs_du_cookie() {
        let jeton = JetonSession([3; 32]);
        let valeur = cookie(&jeton, Duration::hours(12)).unwrap();
        let texte = valeur.to_str().unwrap();
        assert_eq!(
            texte,
            format!(
                "__Host-sigma_session={}; Max-Age=43200; Path=/; Secure; HttpOnly; SameSite=Strict",
                jeton.texte()
            )
        );
        assert!(!texte.to_ascii_lowercase().contains("domain"));
        assert!(
            cookie(&jeton, Duration::seconds(-5))
                .unwrap()
                .to_str()
                .unwrap()
                .contains("Max-Age=0;")
        );
        assert!(
            cookie_efface()
                .to_str()
                .unwrap()
                .starts_with("__Host-sigma_session=; Max-Age=0;")
        );
    }

    #[test]
    fn lecture_du_cookie() {
        let jeton = JetonSession([4; 32]);
        let t = jeton.texte();
        let lu = |cookies: &[&str]| jeton_de_la_requete(&parts(cookies)).map(|j| j.0);
        assert_eq!(lu(&[&format!("{NOM_COOKIE}={t}")]), Some([4; 32]));
        assert_eq!(
            lu(&[&format!("autre=1; {NOM_COOKIE}={t}; x=y")]),
            Some([4; 32])
        );
        assert_eq!(lu(&[]), None);
        assert_eq!(lu(&[&format!("sigma_session={t}")]), None);
        assert_eq!(lu(&[&format!("{NOM_COOKIE}=court")]), None);
        // En double, dans un en-tête ou dans deux : refus.
        assert_eq!(lu(&[&format!("{NOM_COOKIE}={t}; {NOM_COOKIE}={t}")]), None);
        assert_eq!(
            lu(&[&format!("{NOM_COOKIE}={t}"), &format!("{NOM_COOKIE}={t}")]),
            None
        );
    }
}
