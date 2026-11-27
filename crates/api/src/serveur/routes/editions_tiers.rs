//! Éditions de tiers : grand livre et balance auxiliaires, balance âgée, relevé de compte (L3-20 ; ADR 0033).
//!
//! **Squelette** posé par la PR 6 de `docs/lots/L3.md` (L3-14) pour la PR 10 : tranche vide,
//! aucune route, aucun accès à la base. La PR 10 ajoute ici ses routes, ses cas d'usage
//! ([`Cas`]) et leur exécution ([`executer`]), sans toucher aux registres communs
//! (`routes::MODULES`, `CasUsage`, `routes::executer`), déjà branchés sur ce module.
//!
//! Actions Cedar prévues : `ConsulterEditionsTiers` (`roles/tiers.cedar`) ; export : `Exporter`. Migrations : aucune (lecture seule).

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
    nom: "editions_tiers",
    routes: ROUTES,
    champs_journal: &[],
};

/// Routes du module.
pub const ROUTES: &[Route] = &[];

/// Exécute un cas d'usage du module dans la transaction du pipeline. Aucun cas n'existe : la
/// fonction ne peut pas être appelée ; sa signature est celle des autres modules, pour que la PR
/// 10 n'en remplace que le corps.
#[allow(clippy::unused_async)] // Signature commune des modules : le corps deviendra asynchrone.
pub(crate) async fn executer(
    cas: Cas,
    _tx: &mut TransactionTenant,
    _garde: &Garde,
    _requete: &Requete<'_>,
) -> Result<Resultat, ErreurApi> {
    match cas {}
}
