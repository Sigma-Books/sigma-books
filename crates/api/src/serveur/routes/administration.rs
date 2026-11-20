//! Routes de l'administration du cabinet : invitations, rôles, accès aux dossiers, sessions des
//! membres (PR 9 de `docs/lots/L2.md`). Tranche vide jusqu'à cette PR.

use super::{Module, Route};

/// Cas d'usage du module : aucun tant que le module n'a pas de route.
#[allow(clippy::empty_enums)] // Énumération vide voulue : aucun cas d'usage ne peut exister.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cas {}

/// Tranche du module.
pub const MODULE: Module = Module {
    nom: "administration",
    routes: ROUTES,
    champs_journal: &[],
};

/// Routes du module.
pub const ROUTES: &[Route] = &[];
