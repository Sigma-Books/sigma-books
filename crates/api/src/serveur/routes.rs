//! Table déclarative des routes de l'API (L2, « Tests exigés », « chaque route »).
//!
//! Le routeur se construit **uniquement** à partir de [`ROUTES`] ([`super::routeur`]) : une route
//! absente de la table n'existe pas. Le test d'isolation « chaque route »
//! (`crates/api/tests/isolation.rs`) parcourt cette table et échoue si une route n'a pas son cas
//! d'isolation.
//!
//! Routes de la PR 6, **en lecture seule** : elles éprouvent le pipeline. Aucune mutation
//! métier avant le journal d'audit (PR 7, `crates/api/CLAUDE.md`).

use authz::Action;
use axum::Json;
use axum::response::{IntoResponse, Response};
use axum::routing::MethodFilter;
use serde::Serialize;

use super::erreur::ErreurApi;
use super::pipeline::Garde;
use crate::base::TransactionTenant;
use crate::base::organisation::{lire_cabinet, lire_dossier_visible, lister_dossiers_visibles};

/// Méthode HTTP d'une route.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Methode {
    /// Lecture.
    Get,
}

impl Methode {
    /// Filtre du routeur.
    #[must_use]
    pub const fn filtre(self) -> MethodFilter {
        match self {
            Self::Get => MethodFilter::GET,
        }
    }

    /// Nom HTTP.
    #[must_use]
    pub const fn nom(self) -> &'static str {
        match self {
            Self::Get => "GET",
        }
    }
}

/// Ressource que désigne le chemin, et donc ce que le pipeline vérifie avant le cas d'usage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Portee {
    /// Le cabinet du chemin (`{cabinet}`) : l'action est décidée sur le cabinet.
    Cabinet,
    /// Les dossiers du cabinet du chemin : chaque dossier est décidé un à un par le cas d'usage.
    DossiersDuCabinet,
    /// Un dossier (`{cabinet}`, `{dossier}`) : visibilité (`AccederDossier`), puis l'action.
    Dossier,
}

/// Cas d'usage d'une route.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CasUsage {
    /// Paramètres du cabinet.
    LireCabinet,
    /// Dossiers visibles du cabinet.
    ListerDossiers,
    /// Un dossier.
    LireDossier,
}

/// Une route : méthode, modèle de chemin, action Cedar, portée et cas d'usage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Route {
    /// Méthode HTTP.
    pub methode: Methode,
    /// Modèle de chemin (syntaxe d'`axum`), cabinet et dossier explicites (ADR 0011).
    pub chemin: &'static str,
    /// Action Cedar décidée.
    pub action: Action,
    /// Ressource du chemin.
    pub portee: Portee,
    /// Cas d'usage exécuté dans la transaction.
    pub cas_usage: CasUsage,
}

/// Toutes les routes de l'API.
pub const ROUTES: &[Route] = &[
    // §20 « Gérer cabinet » : paramètres du cabinet, administrateur seul (403 pour les autres).
    Route {
        methode: Methode::Get,
        chemin: "/api/cabinets/{cabinet}",
        action: Action::GererCabinet,
        portee: Portee::Cabinet,
        cas_usage: CasUsage::LireCabinet,
    },
    // Dossiers visibles : tous pour l'administrateur, les dossiers affectés pour les autres (Q5.1).
    Route {
        methode: Methode::Get,
        chemin: "/api/cabinets/{cabinet}/dossiers",
        action: Action::AccederDossier,
        portee: Portee::DossiersDuCabinet,
        cas_usage: CasUsage::ListerDossiers,
    },
    Route {
        methode: Methode::Get,
        chemin: "/api/cabinets/{cabinet}/dossiers/{dossier}",
        action: Action::AccederDossier,
        portee: Portee::Dossier,
        cas_usage: CasUsage::LireDossier,
    },
];

/// Vue d'un cabinet.
#[derive(Debug, Serialize)]
struct VueCabinet {
    id: String,
    nom: String,
    separation_taches: bool,
}

/// Vue d'un dossier.
#[derive(Debug, Serialize)]
struct VueDossier {
    id: String,
    nom: String,
}

/// Exécute un cas d'usage dans la transaction du pipeline.
pub(crate) async fn executer(
    cas: CasUsage,
    tx: &mut TransactionTenant,
    garde: &Garde,
) -> Result<Response, ErreurApi> {
    match cas {
        CasUsage::LireCabinet => {
            let cabinet = lire_cabinet(tx).await?.ok_or(ErreurApi::Introuvable)?;
            Ok(Json(VueCabinet {
                id: cabinet.id.to_string(),
                nom: cabinet.nom,
                separation_taches: cabinet.separation_taches,
            })
            .into_response())
        }
        CasUsage::ListerDossiers => {
            // Filtre de la requête (affectation), puis décision Cedar pour chaque dossier.
            let mut vues = Vec::new();
            for visible in lister_dossiers_visibles(tx).await? {
                if garde.autorise_dossier(visible.dossier.id, visible.acces.as_ref())? {
                    vues.push(VueDossier {
                        id: visible.dossier.id.to_string(),
                        nom: visible.dossier.nom,
                    });
                }
            }
            Ok(Json(vues).into_response())
        }
        CasUsage::LireDossier => {
            let id = tx.contexte().id_dossier().ok_or(ErreurApi::Introuvable)?;
            let dossier = lire_dossier_visible(tx, id)
                .await?
                .ok_or(ErreurApi::Introuvable)?;
            Ok(Json(VueDossier {
                id: dossier.id.to_string(),
                nom: dossier.nom,
            })
            .into_response())
        }
    }
}
