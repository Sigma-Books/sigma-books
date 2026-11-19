//! Authentification : première étape du pipeline (`crates/api/CLAUDE.md`).
//!
//! L'interface [`Authentification`] rend, à partir des en-têtes de la requête, l'utilisateur
//! authentifié et l'instant de sa dernière vérification de facteur, d'où la fraîcheur de la
//! session ([`authz::Session::depuis`], ADR 0012).
//!
//! Jusqu'aux sessions (PR 8), le binaire n'a qu'une implémentation : [`RefusTotal`], qui refuse
//! tout (401). Les tests fournissent la leur dans `crates/api/tests/` : elle n'est compilée que
//! pour eux et le binaire ne peut pas la sélectionner (aucune configuration ne le permet).

use std::future::Future;
use std::pin::Pin;

use axum::http::request::Parts;
use time::OffsetDateTime;

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
}

/// Échec de l'authentification elle-même (base indisponible…), distinct d'un refus.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("authentification impossible")]
pub struct ErreurAuthentification;

/// Résultat d'une authentification : `Ok(None)` si la requête n'est pas authentifiée.
pub type ResultatAuthentification<'a> =
    Pin<Box<dyn Future<Output = Result<Option<Identite>, ErreurAuthentification>> + Send + 'a>>;

/// Authentifie une requête à partir de ses en-têtes (cookie de session, PR 8).
pub trait Authentification: Send + Sync + 'static {
    /// Rend l'utilisateur authentifié, `None` si la requête ne l'est pas.
    fn authentifier<'a>(&'a self, requete: &'a Parts) -> ResultatAuthentification<'a>;
}

/// Refuse toute requête (401) : authentification du binaire jusqu'à la PR 8 (sessions).
#[derive(Debug, Clone, Copy, Default)]
pub struct RefusTotal;

impl Authentification for RefusTotal {
    fn authentifier<'a>(&'a self, _requete: &'a Parts) -> ResultatAuthentification<'a> {
        Box::pin(async { Ok(None) })
    }
}
