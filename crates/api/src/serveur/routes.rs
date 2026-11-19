//! Table déclarative des routes de l'API (L2, « Tests exigés », « chaque route »).
//!
//! Le routeur se construit **uniquement** à partir de [`ROUTES`] ([`super::routeur`]) : une route
//! absente de la table n'existe pas. Le test d'isolation « chaque route »
//! (`crates/api/tests/isolation.rs`) parcourt cette table et échoue si une route n'a pas son cas
//! d'isolation.
//!
//! Routes de la PR 6 (lecture) et de la PR 7 : journal d'audit (consultation, vérification,
//! export, ADR 0014 et 0016) et première mutation, la création d'un dossier, inscrite au
//! journal d'audit dans sa transaction (`crates/api/CLAUDE.md`).

use authz::Action;
use axum::Json;
use axum::http::{Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::MethodFilter;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;

use super::audit::{self, Trace};
use super::erreur::ErreurApi;
use super::pipeline::{CabinetAutorise, DossierAutorise, Garde};
use crate::base::TransactionTenant;
use crate::base::audit::PorteeAudit;
use crate::base::organisation::{
    affecter_utilisateur_du_contexte, creer_dossier, lire_cabinet, lire_dossier_visible,
    lister_dossiers_visibles,
};

/// Méthode HTTP d'une route.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Methode {
    /// Lecture.
    Get,
    /// Mutation : anti-CSRF, et inscription au journal d'audit exigée.
    Post,
}

impl Methode {
    /// Filtre du routeur.
    #[must_use]
    pub const fn filtre(self) -> MethodFilter {
        match self {
            Self::Get => MethodFilter::GET,
            Self::Post => MethodFilter::POST,
        }
    }

    /// Nom HTTP.
    #[must_use]
    pub const fn nom(self) -> &'static str {
        match self {
            Self::Get => "GET",
            Self::Post => "POST",
        }
    }

    /// Méthode HTTP.
    #[must_use]
    pub fn methode(self) -> Method {
        match self {
            Self::Get => Method::GET,
            Self::Post => Method::POST,
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
    /// Créer un dossier : inscrit au cabinet, puis en première entrée du dossier (ADR 0014) ;
    /// le créateur y est affecté.
    CreerDossier,
    /// Consultation paginée de l'audit de la portée du chemin.
    ConsulterAudit,
    /// Vérification de la chaîne d'audit de la portée du chemin.
    VerifierAudit,
    /// Export de l'audit de la portée du chemin, inscrit lui-même à l'audit.
    ExporterAudit,
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
    // §20 « Créer un dossier » : administrateur, valideur.
    Route {
        methode: Methode::Post,
        chemin: "/api/cabinets/{cabinet}/dossiers",
        action: Action::CreerDossier,
        portee: Portee::Cabinet,
        cas_usage: CasUsage::CreerDossier,
    },
    Route {
        methode: Methode::Get,
        chemin: "/api/cabinets/{cabinet}/dossiers/{dossier}",
        action: Action::AccederDossier,
        portee: Portee::Dossier,
        cas_usage: CasUsage::LireDossier,
    },
    // Audit du cabinet : administrateur seul (ADR 0016) ; l'export exige la ré-authentification.
    Route {
        methode: Methode::Get,
        chemin: "/api/cabinets/{cabinet}/audit",
        action: Action::ConsulterAuditCabinet,
        portee: Portee::Cabinet,
        cas_usage: CasUsage::ConsulterAudit,
    },
    Route {
        methode: Methode::Get,
        chemin: "/api/cabinets/{cabinet}/audit/verification",
        action: Action::VerifierAuditCabinet,
        portee: Portee::Cabinet,
        cas_usage: CasUsage::VerifierAudit,
    },
    Route {
        methode: Methode::Post,
        chemin: "/api/cabinets/{cabinet}/audit/export",
        action: Action::ExporterAuditCabinet,
        portee: Portee::Cabinet,
        cas_usage: CasUsage::ExporterAudit,
    },
    // Audit d'un dossier : administrateur, valideur, auditeur affectés (§20, ADR 0016). La
    // vérification relève de la consultation ; l'export, de l'export complet (ré-authentifié).
    Route {
        methode: Methode::Get,
        chemin: "/api/cabinets/{cabinet}/dossiers/{dossier}/audit",
        action: Action::ConsulterAudit,
        portee: Portee::Dossier,
        cas_usage: CasUsage::ConsulterAudit,
    },
    Route {
        methode: Methode::Get,
        chemin: "/api/cabinets/{cabinet}/dossiers/{dossier}/audit/verification",
        action: Action::ConsulterAudit,
        portee: Portee::Dossier,
        cas_usage: CasUsage::VerifierAudit,
    },
    Route {
        methode: Methode::Post,
        chemin: "/api/cabinets/{cabinet}/dossiers/{dossier}/audit/export",
        action: Action::ExporterComplet,
        portee: Portee::Dossier,
        cas_usage: CasUsage::ExporterAudit,
    },
];

/// Ce que le cas d'usage reçoit de la requête, en plus de la transaction et de la garde.
#[derive(Debug, Clone, Copy)]
pub struct Requete<'a> {
    /// Paramètres de l'URL (`apres=…&limite=…`), non décodés.
    pub parametres: Option<&'a str>,
    /// Corps, déjà borné.
    pub corps: &'a [u8],
    /// Pool du rôle de lecture d'audit.
    pub lecture_audit: &'a PgPool,
}

/// Résultat d'un cas d'usage : la réponse, et les traces à inscrire au journal d'audit avant
/// la validation de la transaction.
#[derive(Debug)]
pub struct Resultat {
    /// Réponse au client, envoyée seulement si la transaction est validée.
    pub reponse: Response,
    /// Traces de l'action ; vide pour une lecture.
    pub traces: Vec<Trace>,
}

impl Resultat {
    fn lecture(reponse: Response) -> Self {
        Self {
            reponse,
            traces: Vec::new(),
        }
    }
}

/// Corps de la création d'un dossier.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct NouveauDossier {
    nom: String,
}

/// Longueur maximale du nom d'un dossier (contrainte de la table `dossier`).
const NOM_DOSSIER_MAX: usize = 200;

/// Nom de dossier admis : 1 à 200 caractères, non blanc, sans caractère de contrôle.
fn nom_dossier(corps: &[u8]) -> Result<String, ErreurApi> {
    let Json(NouveauDossier { nom }) =
        Json::<NouveauDossier>::from_bytes(corps).map_err(|_| ErreurApi::RequeteInvalide)?;
    let valide = !nom.trim().is_empty()
        && nom.chars().count() <= NOM_DOSSIER_MAX
        && !nom.chars().any(char::is_control);
    if valide {
        Ok(nom)
    } else {
        Err(ErreurApi::RequeteInvalide)
    }
}

/// Vue d'un cabinet.
#[derive(Debug, Serialize)]
struct VueCabinet {
    id: String,
    nom: String,
    separation_taches: bool,
}

/// Vue d'un dossier : ne se construit que depuis un [`DossierAutorise`], que seule la [`Garde`]
/// fabrique.
#[derive(Debug, Serialize)]
struct VueDossier {
    id: String,
    nom: String,
}

impl From<DossierAutorise> for VueDossier {
    fn from(dossier: DossierAutorise) -> Self {
        Self {
            id: dossier.id().to_string(),
            nom: dossier.nom().to_owned(),
        }
    }
}

impl From<CabinetAutorise> for VueCabinet {
    fn from(autorise: CabinetAutorise) -> Self {
        let cabinet = autorise.cabinet();
        Self {
            id: cabinet.id.to_string(),
            nom: cabinet.nom.clone(),
            separation_taches: cabinet.separation_taches,
        }
    }
}

/// Exécute un cas d'usage dans la transaction du pipeline.
pub(crate) async fn executer(
    cas: CasUsage,
    tx: &mut TransactionTenant,
    garde: &Garde,
    requete: &Requete<'_>,
) -> Result<Resultat, ErreurApi> {
    match cas {
        CasUsage::LireCabinet => {
            let cabinet = lire_cabinet(tx).await?.ok_or(ErreurApi::Introuvable)?;
            let vue = VueCabinet::from(garde.cabinet_du_chemin(cabinet)?);
            Ok(Resultat::lecture(Json(vue).into_response()))
        }
        CasUsage::ListerDossiers => {
            // Filtre de la requête (affectation), puis décision Cedar pour chaque dossier.
            let autorises = garde.filtrer(lister_dossiers_visibles(tx).await?)?;
            let vues: Vec<VueDossier> = autorises.into_iter().map(VueDossier::from).collect();
            Ok(Resultat::lecture(Json(vues).into_response()))
        }
        CasUsage::LireDossier => {
            let id = tx.contexte().id_dossier().ok_or(ErreurApi::Introuvable)?;
            let dossier = lire_dossier_visible(tx, id)
                .await?
                .ok_or(ErreurApi::Introuvable)?;
            let vue = VueDossier::from(garde.dossier_du_chemin(dossier)?);
            Ok(Resultat::lecture(Json(vue).into_response()))
        }
        CasUsage::CreerDossier => creer(tx, garde, requete).await,
        CasUsage::ConsulterAudit => Ok(Resultat::lecture(
            audit::consulter(tx, garde, requete).await?,
        )),
        CasUsage::VerifierAudit => Ok(Resultat::lecture(
            audit::verifier(tx, garde, requete).await?,
        )),
        CasUsage::ExporterAudit => {
            let (reponse, trace) = audit::exporter(tx, garde, requete).await?;
            Ok(Resultat {
                reponse,
                traces: vec![trace],
            })
        }
    }
}

/// Création d'un dossier (action du cabinet) : le dossier, l'affectation de son créateur, puis
/// deux traces : au cabinet, puis en première entrée de la chaîne du dossier (ADR 0014).
async fn creer(
    tx: &mut TransactionTenant,
    garde: &Garde,
    requete: &Requete<'_>,
) -> Result<Resultat, ErreurApi> {
    garde.exiger_cabinet()?;
    let nom = nom_dossier(requete.corps)?;
    let dossier = creer_dossier(tx, &nom).await?;
    affecter_utilisateur_du_contexte(tx, dossier.id).await?;
    let id = dossier.id.uuid();

    let mut au_cabinet = Trace::new(PorteeAudit::Cabinet, "dossier", id);
    au_cabinet.champs.push(("nom", None, Some(nom.clone())));
    au_cabinet
        .champs
        .push(("affectation", None, Some(garde.utilisateur().to_string())));
    let mut au_dossier = Trace::new(PorteeAudit::Dossier(dossier.id), "dossier", id);
    au_dossier.champs.push(("nom", None, Some(nom.clone())));

    let vue = VueDossier {
        id: dossier.id.to_string(),
        nom,
    };
    Ok(Resultat {
        reponse: (StatusCode::CREATED, Json(vue)).into_response(),
        traces: vec![au_cabinet, au_dossier],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noms_de_dossier() {
        assert_eq!(
            nom_dossier(br#"{"nom":"SARL Exemple"}"#),
            Ok("SARL Exemple".to_owned())
        );
        let long = format!(r#"{{"nom":"{}"}}"#, "é".repeat(200));
        assert!(nom_dossier(long.as_bytes()).is_ok());
        for invalide in [
            r#"{"nom":""}"#.to_owned(),
            r#"{"nom":"   "}"#.to_owned(),
            r#"{"nom":"a\u0007b"}"#.to_owned(),
            r#"{"nom":"a","autre":1}"#.to_owned(),
            r#"{"nom":1}"#.to_owned(),
            "pas du json".to_owned(),
            format!(r#"{{"nom":"{}"}}"#, "a".repeat(201)),
        ] {
            assert_eq!(
                nom_dossier(invalide.as_bytes()),
                Err(ErreurApi::RequeteInvalide),
                "{invalide}"
            );
        }
    }

    #[test]
    fn chaque_mutation_porte_sur_une_ressource_unique() {
        // Une mutation est décidée sur une ressource du chemin : sa trace a une portée.
        for route in ROUTES.iter().filter(|r| r.methode == Methode::Post) {
            assert_ne!(route.portee, Portee::DossiersDuCabinet, "{}", route.chemin);
        }
    }
}
