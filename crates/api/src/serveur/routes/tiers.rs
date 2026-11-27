//! Tiers : fiche, identifiants, contacts, adresses, coordonnées bancaires, archivage (L3-15), fusion par lien et anonymisation (L3-16 ; ADR 0031).
//!
//! **Squelette** posé par la PR 6 de `docs/lots/L3.md` (L3-14) pour la PR 7a et 7b : tranche vide,
//! aucune route, aucun accès à la base. La PR 7a et 7b ajoute ici ses routes, ses cas d'usage
//! ([`Cas`]) et leur exécution ([`executer`]), sans toucher aux registres communs
//! (`routes::MODULES`, `CasUsage`, `routes::executer`), déjà branchés sur ce module.
//!
//! Actions Cedar prévues : `ConsulterTiers`, `ConsulterCoordonneesBancairesTiers`, `CreerTiers`, `ModifierTiers`, `ArchiverTiers`, `FusionnerTiers`, `RompreFusionTiers`, `AnonymiserTiers` (`crates/authz/politiques/roles/tiers.cedar`). Migrations : `0500` à `0509`.

use super::{Module, Requete, Resultat, Route};
use crate::base::TransactionTenant;
use crate::serveur::erreur::ErreurApi;
use crate::serveur::pipeline::Garde;

/// Cas d'usage du module : aucun tant que le module n'a pas de route.
#[allow(clippy::empty_enums)] // Énumération vide voulue : aucun cas d'usage ne peut exister.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cas {}

/// Tranche du module.
pub const MODULE: Module = Module {
    nom: "tiers",
    routes: ROUTES,
    champs_journal: &[],
};

/// Routes du module.
pub const ROUTES: &[Route] = &[];

/// Exécute un cas d'usage du module dans la transaction du pipeline. Aucun cas n'existe : la
/// fonction ne peut pas être appelée ; sa signature est celle des autres modules, pour que la PR
/// 7a et 7b n'en remplace que le corps.
#[allow(clippy::unused_async)] // Signature commune des modules : le corps deviendra asynchrone.
pub(crate) async fn executer(
    cas: Cas,
    _tx: &mut TransactionTenant,
    _garde: &Garde,
    _requete: &Requete<'_>,
) -> Result<Resultat, ErreurApi> {
    match cas {}
}
