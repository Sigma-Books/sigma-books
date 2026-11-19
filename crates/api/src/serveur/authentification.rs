//! Authentification : première étape du pipeline (`crates/api/CLAUDE.md`).
//!
//! L'interface [`Authentification`] rend, à partir des en-têtes de la requête, l'utilisateur
//! authentifié, sa session (identifiant public, nature) et l'instant de sa dernière vérification
//! de facteur, d'où la fraîcheur de la session ([`authz::Session::depuis`], ADR 0012), ainsi que
//! le résultat de la comparaison du jeton anti-CSRF lié à la session.
//!
//! Le binaire n'a qu'une implémentation : [`super::session::AuthentificationSession`], lue dans
//! PostgreSQL (cookie de session). Les tests fournissent en outre la leur dans
//! `crates/api/tests/` : elle n'est compilée que pour eux et le binaire ne peut pas la
//! sélectionner (aucune configuration ne le permet).

use std::future::Future;
use std::pin::Pin;

use axum::http::request::Parts;
use time::OffsetDateTime;

pub use crate::base::session::NatureSession;
use crate::base::{IdSession, IdUtilisateur};

/// Un utilisateur authentifié.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Identite {
    /// L'utilisateur, tel que la session l'enregistre en base (jamais un identifiant envoyé par
    /// le client).
    pub utilisateur: IdUtilisateur,
    /// Identifiant **public** de la session (ADR 0012), inscrit au journal d'audit
    /// (ADR 0014, clé `session=`) : jamais son jeton.
    pub session: IdSession,
    /// Instant de la dernière vérification de facteur (passkey, ou mot de passe et TOTP).
    pub derniere_verification: OffsetDateTime,
    /// Nature de la session : seule une session complète ouvre les routes métier.
    pub nature: NatureSession,
    /// Vrai si l'en-tête anti-CSRF de la requête est le jeton lié à **cette** session (comparé
    /// en temps constant). Le pipeline refuse une mutation sans lui (403).
    pub csrf_lie: bool,
}

/// Échec de l'authentification elle-même (base indisponible…), distinct d'un refus.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("authentification impossible")]
pub struct ErreurAuthentification;

/// Résultat d'une authentification : `Ok(None)` si la requête n'est pas authentifiée.
pub type ResultatAuthentification<'a> =
    Pin<Box<dyn Future<Output = Result<Option<Identite>, ErreurAuthentification>> + Send + 'a>>;

/// Authentifie une requête à partir de ses en-têtes (cookie de session).
pub trait Authentification: Send + Sync + 'static {
    /// Rend l'utilisateur authentifié, `None` si la requête ne l'est pas.
    fn authentifier<'a>(&'a self, requete: &'a Parts) -> ResultatAuthentification<'a>;
}
