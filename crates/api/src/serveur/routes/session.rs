//! Routes de la session courante (PR 8a ; ADR 0012) : état et jeton anti-CSRF, déconnexion,
//! rotation du jeton. Toute nature de session est admise pour l'état et la déconnexion. Les cas
//! d'usage, hors cabinet, vivent dans `serveur::moi`.

use authz::ActionUtilisateur;

use super::{ActionRoute, CasUsage, Methode, Module, Portee, Route};

/// Cas d'usage du module.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cas {
    /// État de la session courante et son jeton anti-CSRF.
    EtatSession,
    /// Rotation du jeton de la session courante (changement de cabinet actif).
    TournerJeton,
    /// Déconnexion : révocation de la session courante.
    Deconnecter,
}

/// Tranche du module.
pub const MODULE: Module = Module {
    nom: "session",
    routes: ROUTES,
    champs_journal: &[],
};

/// Routes du module.
pub const ROUTES: &[Route] = &[
    // Session courante (ADR 0012) : état et jeton anti-CSRF ; déconnexion. Toute nature.
    Route {
        methode: Methode::Get,
        chemin: "/api/session",
        action: ActionRoute::Utilisateur(ActionUtilisateur::ConsulterSaSession),
        portee: Portee::Session,
        cas_usage: CasUsage::Session(Cas::EtatSession),
    },
    Route {
        methode: Methode::Post,
        chemin: "/api/session/deconnexion",
        action: ActionRoute::Utilisateur(ActionUtilisateur::RevoquerSaSession),
        portee: Portee::Session,
        cas_usage: CasUsage::Session(Cas::Deconnecter),
    },
    // Rotation du jeton, appelée par l'interface au changement de cabinet actif (ADR 0012) ;
    // le serveur ne mémorise aucun cabinet (ADR 0011, F1.14).
    Route {
        methode: Methode::Post,
        chemin: "/api/session/rotation",
        action: ActionRoute::Utilisateur(ActionUtilisateur::TournerSaSession),
        portee: Portee::Utilisateur,
        cas_usage: CasUsage::Session(Cas::TournerJeton),
    },
];
