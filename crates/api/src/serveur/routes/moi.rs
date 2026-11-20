//! Routes des ressources propres de l'utilisateur (PR 8a ; ADR 0012, spec §21) : ses sessions,
//! ses cabinets. Les cas d'usage, hors cabinet, vivent dans `serveur::moi`.

use authz::{Action, ActionUtilisateur};

use super::{ActionRoute, CasUsage, Methode, Module, Portee, Route};

/// Cas d'usage du module.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cas {
    /// Sessions actives de l'utilisateur.
    ListerSessions,
    /// Révocation d'une session de l'utilisateur.
    RevoquerSession,
    /// Cabinets de l'utilisateur, chacun décidé par Cedar (`AccederCabinet`).
    ListerCabinets,
}

/// Tranche du module.
pub const MODULE: Module = Module {
    nom: "moi",
    routes: ROUTES,
    champs_journal: &[],
};

/// Routes du module.
pub const ROUTES: &[Route] = &[
    // Ses sessions (spec §21) : liste, révocation.
    Route {
        methode: Methode::Get,
        chemin: "/api/moi/sessions",
        action: ActionRoute::Utilisateur(ActionUtilisateur::ListerSesSessions),
        portee: Portee::Utilisateur,
        cas_usage: CasUsage::Moi(Cas::ListerSessions),
    },
    Route {
        methode: Methode::Delete,
        chemin: "/api/moi/sessions/{session}",
        action: ActionRoute::Utilisateur(ActionUtilisateur::RevoquerSaSession),
        portee: Portee::Utilisateur,
        cas_usage: CasUsage::Moi(Cas::RevoquerSession),
    },
    // Ses cabinets, avant le choix du cabinet actif : Cedar décide chaque appartenance.
    Route {
        methode: Methode::Get,
        chemin: "/api/moi/cabinets",
        action: ActionRoute::Cabinet(Action::AccederCabinet),
        portee: Portee::Utilisateur,
        cas_usage: CasUsage::Moi(Cas::ListerCabinets),
    },
];
