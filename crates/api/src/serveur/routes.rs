//! Table déclarative des routes de l'API (L2, « Tests exigés », « chaque route »).
//!
//! Le routeur se construit **uniquement** à partir de [`ROUTES`] ([`super::routeur`]) : une route
//! absente de la table n'existe pas. Le test d'isolation « chaque route »
//! (`crates/api/tests/isolation.rs`) parcourt cette table et échoue si une route n'a pas son cas
//! d'isolation.
//!
//! Routes de la PR 6 (lecture) et de la PR 7 : journal d'audit (consultation, vérification,
//! export, ADR 0014 et 0016) et première mutation, la création d'un dossier, inscrite au
//! journal d'audit dans sa transaction (`crates/api/CLAUDE.md`). PR 8a : la session courante
//! et les ressources propres de l'utilisateur, hors de tout cabinet (ADR 0012, module
//! [`super::moi`]).

use authz::Action;
use axum::Json;
use axum::http::{Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::MethodFilter;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;

use super::audit::{self, Trace};
use super::authentification::NatureSession;
use super::erreur::ErreurApi;
use super::pipeline::{CabinetAutorise, DossierAutorise, Garde};
use crate::base::TransactionTenant;
use crate::base::audit::PorteeAudit;
use crate::base::organisation::{
    affecter_utilisateur_du_contexte, creer_dossier, lire_appartenance, lire_cabinet,
    lire_dossier_visible, lister_dossiers_visibles,
};

/// Méthode HTTP d'une route.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Methode {
    /// Lecture.
    Get,
    /// Mutation : anti-CSRF, et inscription au journal d'audit exigée sur un cabinet.
    Post,
    /// Mutation (suppression ou révocation) : anti-CSRF.
    Delete,
}

impl Methode {
    /// Filtre du routeur.
    #[must_use]
    pub const fn filtre(self) -> MethodFilter {
        match self {
            Self::Get => MethodFilter::GET,
            Self::Post => MethodFilter::POST,
            Self::Delete => MethodFilter::DELETE,
        }
    }

    /// Nom HTTP.
    #[must_use]
    pub const fn nom(self) -> &'static str {
        match self {
            Self::Get => "GET",
            Self::Post => "POST",
            Self::Delete => "DELETE",
        }
    }

    /// Méthode HTTP.
    #[must_use]
    pub fn methode(self) -> Method {
        match self {
            Self::Get => Method::GET,
            Self::Post => Method::POST,
            Self::Delete => Method::DELETE,
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
    /// Ressources propres de l'utilisateur authentifié, hors de tout cabinet (ses sessions, ses
    /// cabinets) : transaction au contexte du seul utilisateur ; session complète exigée.
    Utilisateur,
    /// La session courante elle-même (état, déconnexion) : toute nature de session admise
    /// (une session restreinte doit pouvoir lire son état et se fermer).
    Session,
}

impl Portee {
    /// Vrai pour une portée d'un cabinet (`{cabinet}` dans le chemin) : pipeline complet,
    /// décision Cedar sur la ressource du chemin, audit des mutations.
    #[must_use]
    pub const fn du_cabinet(self) -> bool {
        matches!(
            self,
            Self::Cabinet | Self::DossiersDuCabinet | Self::Dossier
        )
    }

    /// Vrai si une session de cette nature est admise : seule une session complète ouvre les
    /// routes métier et les ressources de l'utilisateur (ADR 0012, F1.12).
    #[must_use]
    pub const fn admet(self, nature: NatureSession) -> bool {
        matches!(self, Self::Session) || matches!(nature, NatureSession::Complete)
    }
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
    /// État de la session courante et son jeton anti-CSRF.
    EtatSession,
    /// Rotation du jeton de la session courante (changement de cabinet actif).
    TournerJeton,
    /// Déconnexion : révocation de la session courante.
    Deconnecter,
    /// Sessions actives de l'utilisateur.
    ListerSessions,
    /// Révocation d'une session de l'utilisateur.
    RevoquerSession,
    /// Cabinets de l'utilisateur, chacun décidé par Cedar (`AccederCabinet`).
    ListerCabinets,
}

/// Une route : méthode, modèle de chemin, action Cedar, portée et cas d'usage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Route {
    /// Méthode HTTP.
    pub methode: Methode,
    /// Modèle de chemin (syntaxe d'`axum`), cabinet et dossier explicites (ADR 0011).
    pub chemin: &'static str,
    /// Action Cedar décidée. `None` : route **hors Cedar**, sur la session courante ou les
    /// sessions de l'utilisateur, qui relèvent de l'utilisateur lui-même et d'aucun cabinet
    /// (doc du crate `authz`, « Hors Cedar ») ; la RLS et le filtre de la requête les bornent
    /// à l'utilisateur authentifié.
    pub action: Option<Action>,
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
        action: Some(Action::GererCabinet),
        portee: Portee::Cabinet,
        cas_usage: CasUsage::LireCabinet,
    },
    // Dossiers visibles : tous pour l'administrateur, les dossiers affectés pour les autres (Q5.1).
    Route {
        methode: Methode::Get,
        chemin: "/api/cabinets/{cabinet}/dossiers",
        action: Some(Action::AccederDossier),
        portee: Portee::DossiersDuCabinet,
        cas_usage: CasUsage::ListerDossiers,
    },
    // §20 « Créer un dossier » : administrateur, valideur.
    Route {
        methode: Methode::Post,
        chemin: "/api/cabinets/{cabinet}/dossiers",
        action: Some(Action::CreerDossier),
        portee: Portee::Cabinet,
        cas_usage: CasUsage::CreerDossier,
    },
    Route {
        methode: Methode::Get,
        chemin: "/api/cabinets/{cabinet}/dossiers/{dossier}",
        action: Some(Action::AccederDossier),
        portee: Portee::Dossier,
        cas_usage: CasUsage::LireDossier,
    },
    // Audit du cabinet : administrateur seul (ADR 0016) ; l'export exige la ré-authentification.
    Route {
        methode: Methode::Get,
        chemin: "/api/cabinets/{cabinet}/audit",
        action: Some(Action::ConsulterAuditCabinet),
        portee: Portee::Cabinet,
        cas_usage: CasUsage::ConsulterAudit,
    },
    Route {
        methode: Methode::Get,
        chemin: "/api/cabinets/{cabinet}/audit/verification",
        action: Some(Action::VerifierAuditCabinet),
        portee: Portee::Cabinet,
        cas_usage: CasUsage::VerifierAudit,
    },
    Route {
        methode: Methode::Post,
        chemin: "/api/cabinets/{cabinet}/audit/export",
        action: Some(Action::ExporterAuditCabinet),
        portee: Portee::Cabinet,
        cas_usage: CasUsage::ExporterAudit,
    },
    // Audit d'un dossier : administrateur, valideur, auditeur affectés (§20, ADR 0016). La
    // vérification relève de la consultation ; l'export, de l'export complet (ré-authentifié).
    Route {
        methode: Methode::Get,
        chemin: "/api/cabinets/{cabinet}/dossiers/{dossier}/audit",
        action: Some(Action::ConsulterAudit),
        portee: Portee::Dossier,
        cas_usage: CasUsage::ConsulterAudit,
    },
    Route {
        methode: Methode::Get,
        chemin: "/api/cabinets/{cabinet}/dossiers/{dossier}/audit/verification",
        action: Some(Action::ConsulterAudit),
        portee: Portee::Dossier,
        cas_usage: CasUsage::VerifierAudit,
    },
    Route {
        methode: Methode::Post,
        chemin: "/api/cabinets/{cabinet}/dossiers/{dossier}/audit/export",
        action: Some(Action::ExporterComplet),
        portee: Portee::Dossier,
        cas_usage: CasUsage::ExporterAudit,
    },
    // Session courante (ADR 0012) : état et jeton anti-CSRF ; déconnexion. Toute nature.
    Route {
        methode: Methode::Get,
        chemin: "/api/session",
        action: None,
        portee: Portee::Session,
        cas_usage: CasUsage::EtatSession,
    },
    Route {
        methode: Methode::Post,
        chemin: "/api/session/deconnexion",
        action: None,
        portee: Portee::Session,
        cas_usage: CasUsage::Deconnecter,
    },
    // Rotation du jeton, appelée par l'interface au changement de cabinet actif (ADR 0012) ;
    // le serveur ne mémorise aucun cabinet (ADR 0011, F1.14).
    Route {
        methode: Methode::Post,
        chemin: "/api/session/rotation",
        action: None,
        portee: Portee::Utilisateur,
        cas_usage: CasUsage::TournerJeton,
    },
    // Ses sessions (spec §21) : liste, révocation.
    Route {
        methode: Methode::Get,
        chemin: "/api/moi/sessions",
        action: None,
        portee: Portee::Utilisateur,
        cas_usage: CasUsage::ListerSessions,
    },
    Route {
        methode: Methode::Delete,
        chemin: "/api/moi/sessions/{session}",
        action: None,
        portee: Portee::Utilisateur,
        cas_usage: CasUsage::RevoquerSession,
    },
    // Ses cabinets, avant le choix du cabinet actif : Cedar décide chaque appartenance.
    Route {
        methode: Methode::Get,
        chemin: "/api/moi/cabinets",
        action: Some(Action::AccederCabinet),
        portee: Portee::Utilisateur,
        cas_usage: CasUsage::ListerCabinets,
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
        // Cas d'usage hors cabinet : module `moi`, jamais avec une garde de cabinet.
        CasUsage::EtatSession
        | CasUsage::TournerJeton
        | CasUsage::Deconnecter
        | CasUsage::ListerSessions
        | CasUsage::RevoquerSession
        | CasUsage::ListerCabinets => Err(ErreurApi::Interne(super::CategorieErreur::Donnee)),
    }
}

/// Défense en profondeur de la création d'un dossier (ADR 0011) : même si Cedar se trompait,
/// la requête exige une appartenance **active**, lue en base, de rôle `administrateur` ou
/// `valideur` (§20 « Créer un dossier »). Appartenance absente ou suspendue : le cabinet est
/// d'autrui (404) ; autre rôle : action refusée sur une ressource visible (403).
async fn exiger_createur(tx: &mut TransactionTenant) -> Result<(), ErreurApi> {
    let appartenance = lire_appartenance(tx)
        .await?
        .filter(|a| a.actif)
        .ok_or(ErreurApi::Introuvable)?;
    let autorises = [authz::Role::Administrateur, authz::Role::Valideur];
    if autorises.iter().any(|r| r.nom() == appartenance.role) {
        Ok(())
    } else {
        Err(ErreurApi::Interdit)
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
    exiger_createur(tx).await?;
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
        // Une mutation est décidée sur une ressource du chemin : sa trace a une portée. Que
        // chaque mutation produise bien des traces sur cette portée est vérifié contre la base
        // (`tests/audit.rs`, `chaque_mutation_produit_des_traces_sur_sa_portee`) ; le pipeline
        // refuse une mutation sans trace (`pipeline::exiger_traces`).
        for route in ROUTES
            .iter()
            .filter(|r| r.methode != Methode::Get && r.portee.du_cabinet())
        {
            assert_ne!(route.portee, Portee::DossiersDuCabinet, "{}", route.chemin);
        }
    }

    #[test]
    fn portees_actions_et_natures_coherentes() {
        for route in ROUTES {
            // Toute route d'un cabinet est décidée par Cedar ; hors cabinet, seule la liste des
            // cabinets l'est (une décision par appartenance).
            let attendue = route.portee.du_cabinet() || route.cas_usage == CasUsage::ListerCabinets;
            assert_eq!(route.action.is_some(), attendue, "{}", route.chemin);
            assert_eq!(
                route.portee.du_cabinet(),
                route.chemin.contains("{cabinet}"),
                "{}",
                route.chemin
            );
        }
        for nature in [NatureSession::Restreinte, NatureSession::Enrolement] {
            for portee in [
                Portee::Cabinet,
                Portee::DossiersDuCabinet,
                Portee::Dossier,
                Portee::Utilisateur,
            ] {
                assert!(!portee.admet(nature));
            }
            assert!(Portee::Session.admet(nature));
        }
    }
}
