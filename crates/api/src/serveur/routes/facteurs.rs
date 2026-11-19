//! Routes de la méthode de secours (PR 8b de `docs/lots/L2.md` ; ADR 0012, ADR 0017) :
//! connexion par le secours ou par un code de récupération, ré-authentification, enrôlement du
//! secours, codes de récupération, état de ses facteurs, ses événements d'authentification.
//! Les cas d'usage, hors cabinet, vivent dans `serveur::facteurs`.

use authz::ActionFacteur;

use super::{ActionRoute, CasUsage, Methode, Module, Portee, Route};

/// Cas d'usage du module.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cas {
    /// Connexion par la méthode de secours (courriel, mot de passe, code TOTP).
    ConnexionSecours,
    /// Connexion par un code de récupération : session restreinte (F1.12).
    ConnexionCode,
    /// Ré-authentification par la méthode de secours (session « fraîche »).
    Reauthentifier,
    /// État de ses facteurs.
    ConsulterFacteurs,
    /// Ses événements d'authentification (ADR 0017).
    ConsulterEvenements,
    /// Préparation de l'enrôlement du secours : secret TOTP et jeton d'enrôlement.
    PreparerSecours,
    /// Enrôlement (ou remplacement) du secours, et nouvelle série de codes.
    EnrolerSecours,
    /// Nouvelle série de codes de récupération.
    RegenererCodes,
}

/// Tranche du module.
pub const MODULE: Module = Module {
    nom: "facteurs",
    routes: ROUTES,
    champs_journal: &[],
};

/// Routes du module.
pub const ROUTES: &[Route] = &[
    // Connexion (ADR 0012) : sans session ; Cedar décide `SeConnecter` (principal Anonyme),
    // puis, après la vérification du facteur, `OuvrirSession` (principal Utilisateur).
    Route {
        methode: Methode::Post,
        chemin: "/api/connexion/secours",
        action: ActionRoute::Connexion,
        portee: Portee::Anonyme,
        cas_usage: CasUsage::Facteurs(Cas::ConnexionSecours),
    },
    Route {
        methode: Methode::Post,
        chemin: "/api/connexion/code-recuperation",
        action: ActionRoute::Connexion,
        portee: Portee::Anonyme,
        cas_usage: CasUsage::Facteurs(Cas::ConnexionCode),
    },
    // Ré-authentification de la session courante (complète), avec rotation du jeton.
    Route {
        methode: Methode::Post,
        chemin: "/api/session/reauthentification",
        action: ActionRoute::Facteur(ActionFacteur::SeReauthentifier),
        portee: Portee::Utilisateur,
        cas_usage: CasUsage::Facteurs(Cas::Reauthentifier),
    },
    // Ses facteurs et ses événements (session complète).
    Route {
        methode: Methode::Get,
        chemin: "/api/moi/facteurs",
        action: ActionRoute::Facteur(ActionFacteur::ConsulterSesFacteurs),
        portee: Portee::Utilisateur,
        cas_usage: CasUsage::Facteurs(Cas::ConsulterFacteurs),
    },
    Route {
        methode: Methode::Get,
        chemin: "/api/moi/evenements",
        action: ActionRoute::Facteur(ActionFacteur::ConsulterSesEvenements),
        portee: Portee::Utilisateur,
        cas_usage: CasUsage::Facteurs(Cas::ConsulterEvenements),
    },
    // Enrôlement du secours : depuis une session complète fraîche, ou depuis la session
    // restreinte d'un code de récupération (Cedar décide selon la nature et la méthode).
    Route {
        methode: Methode::Post,
        chemin: "/api/moi/facteurs/secours/preparation",
        action: ActionRoute::Facteur(ActionFacteur::EnrolerSecours),
        portee: Portee::Facteurs,
        cas_usage: CasUsage::Facteurs(Cas::PreparerSecours),
    },
    Route {
        methode: Methode::Post,
        chemin: "/api/moi/facteurs/secours",
        action: ActionRoute::Facteur(ActionFacteur::EnrolerSecours),
        portee: Portee::Facteurs,
        cas_usage: CasUsage::Facteurs(Cas::EnrolerSecours),
    },
    // Nouvelle série de codes (session complète fraîche).
    Route {
        methode: Methode::Post,
        chemin: "/api/moi/facteurs/codes-recuperation",
        action: ActionRoute::Facteur(ActionFacteur::RegenererCodesRecuperation),
        portee: Portee::Utilisateur,
        cas_usage: CasUsage::Facteurs(Cas::RegenererCodes),
    },
];
