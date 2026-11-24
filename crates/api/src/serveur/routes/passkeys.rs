//! Routes des passkeys, WebAuthn (PR 8c de `docs/lots/L2.md` ; ADR 0006, ADR 0012) : connexion,
//! ré-authentification, enrôlement et suppression. Chaque cérémonie se fait en deux appels :
//! un « défi » (le serveur garde l'état, le navigateur reçoit les options WebAuthn et un cookie
//! `__Host-sigma_ceremonie`), puis la réponse de l'authentificateur. L'état de ses passkeys se
//! lit par `GET /api/moi/facteurs` (module `facteurs`). Les cas d'usage, hors cabinet, vivent
//! dans `serveur::passkeys`.

use authz::ActionFacteur;

use super::{ActionRoute, CasUsage, Methode, Module, Portee, Route};

/// Cas d'usage du module.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cas {
    /// Défi de connexion par passkey (courriel).
    DefiConnexion,
    /// Connexion par passkey : réponse de l'authentificateur.
    ConnexionPasskey,
    /// Défi de ré-authentification par passkey.
    DefiReauthentification,
    /// Ré-authentification par passkey : réponse de l'authentificateur.
    ReauthentifierPasskey,
    /// Défi d'enrôlement d'une passkey.
    DefiEnrolement,
    /// Enrôlement d'une passkey : réponse de l'authentificateur.
    EnrolerPasskey,
    /// Suppression d'une de ses passkeys.
    SupprimerPasskey,
}

/// Tranche du module.
pub const MODULE: Module = Module {
    nom: "passkeys",
    routes: ROUTES,
    champs_journal: &[],
};

/// Routes du module.
pub const ROUTES: &[Route] = &[
    // Connexion (ADR 0012) : sans session ; Cedar décide `SeConnecter` (principal Anonyme), puis,
    // après la vérification de la passkey, `OuvrirSession` (principal Utilisateur).
    Route {
        methode: Methode::Post,
        chemin: "/api/connexion/passkey/defi",
        action: ActionRoute::Connexion,
        portee: Portee::Anonyme,
        cas_usage: CasUsage::Passkeys(Cas::DefiConnexion),
    },
    Route {
        methode: Methode::Post,
        chemin: "/api/connexion/passkey",
        action: ActionRoute::Connexion,
        portee: Portee::Anonyme,
        cas_usage: CasUsage::Passkeys(Cas::ConnexionPasskey),
    },
    // Ré-authentification de la session courante (complète), avec rotation du jeton.
    Route {
        methode: Methode::Post,
        chemin: "/api/session/reauthentification/passkey/defi",
        action: ActionRoute::Facteur(ActionFacteur::SeReauthentifier),
        portee: Portee::Utilisateur,
        cas_usage: CasUsage::Passkeys(Cas::DefiReauthentification),
    },
    Route {
        methode: Methode::Post,
        chemin: "/api/session/reauthentification/passkey",
        action: ActionRoute::Facteur(ActionFacteur::SeReauthentifier),
        portee: Portee::Utilisateur,
        cas_usage: CasUsage::Passkeys(Cas::ReauthentifierPasskey),
    },
    // Enrôlement : depuis une session complète fraîche, ou depuis la session restreinte d'un
    // code de récupération (Cedar décide selon la nature et la méthode).
    Route {
        methode: Methode::Post,
        chemin: "/api/moi/facteurs/passkeys/defi",
        action: ActionRoute::Facteur(ActionFacteur::EnrolerPasskey),
        portee: Portee::Facteurs,
        cas_usage: CasUsage::Passkeys(Cas::DefiEnrolement),
    },
    Route {
        methode: Methode::Post,
        chemin: "/api/moi/facteurs/passkeys",
        action: ActionRoute::Facteur(ActionFacteur::EnrolerPasskey),
        portee: Portee::Facteurs,
        cas_usage: CasUsage::Passkeys(Cas::EnrolerPasskey),
    },
    // Suppression d'une de ses passkeys (session complète fraîche).
    Route {
        methode: Methode::Delete,
        chemin: "/api/moi/facteurs/passkeys/{passkey}",
        action: ActionRoute::Facteur(ActionFacteur::SupprimerPasskey),
        portee: Portee::Utilisateur,
        cas_usage: CasUsage::Passkeys(Cas::SupprimerPasskey),
    },
];
