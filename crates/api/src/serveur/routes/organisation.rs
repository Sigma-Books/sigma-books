//! Routes du cabinet et de ses dossiers (PR 6 et 7) : paramètres du cabinet, dossiers visibles,
//! un dossier, et la première mutation, la création d'un dossier, inscrite au journal d'audit
//! dans sa transaction (`crates/api/CLAUDE.md`).

use authz::Action;
use axum::Json;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use serde::Serialize;

use super::{
    ActionRoute, CasUsage, Methode, Module, Portee, Requete, Resultat, Route, referentiel,
};
use crate::base::TransactionTenant;
use crate::base::audit::PorteeAudit;
use crate::base::organisation::{
    affecter_utilisateur_du_contexte, creer_dossier, lire_appartenance, lire_cabinet,
    lire_dossier_visible, lister_dossiers_visibles,
};
use crate::serveur::audit::Trace;
use crate::serveur::erreur::ErreurApi;
use crate::serveur::pipeline::{CabinetAutorise, DossierAutorise, Garde};

/// Cas d'usage du module.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cas {
    /// Paramètres du cabinet.
    LireCabinet,
    /// Dossiers visibles du cabinet.
    ListerDossiers,
    /// Un dossier.
    LireDossier,
    /// Créer un dossier : inscrit au cabinet, puis en première entrée du dossier (ADR 0014) ;
    /// le créateur y est affecté.
    CreerDossier,
}

/// Tranche du module.
pub const MODULE: Module = Module {
    nom: "organisation",
    routes: ROUTES,
    champs_journal: &[],
};

/// Routes du module.
pub const ROUTES: &[Route] = &[
    // §20 « Gérer cabinet » : paramètres du cabinet, administrateur seul (403 pour les autres).
    Route {
        methode: Methode::Get,
        chemin: "/api/cabinets/{cabinet}",
        action: ActionRoute::Cabinet(Action::GererCabinet),
        portee: Portee::Cabinet,
        cas_usage: CasUsage::Organisation(Cas::LireCabinet),
    },
    // Dossiers visibles : tous pour l'administrateur, les dossiers affectés pour les autres (Q5.1).
    Route {
        methode: Methode::Get,
        chemin: "/api/cabinets/{cabinet}/dossiers",
        action: ActionRoute::Cabinet(Action::AccederDossier),
        portee: Portee::DossiersDuCabinet,
        cas_usage: CasUsage::Organisation(Cas::ListerDossiers),
    },
    // §20 « Créer un dossier » : administrateur, valideur.
    Route {
        methode: Methode::Post,
        chemin: "/api/cabinets/{cabinet}/dossiers",
        action: ActionRoute::Cabinet(Action::CreerDossier),
        portee: Portee::Cabinet,
        cas_usage: CasUsage::Organisation(Cas::CreerDossier),
    },
    Route {
        methode: Methode::Get,
        chemin: "/api/cabinets/{cabinet}/dossiers/{dossier}",
        action: ActionRoute::Cabinet(Action::AccederDossier),
        portee: Portee::Dossier,
        cas_usage: CasUsage::Organisation(Cas::LireDossier),
    },
];

/// Longueur maximale du nom d'un dossier (contrainte de la table `dossier`).
const NOM_DOSSIER_MAX: usize = 200;

/// Nom de dossier admis : 1 à 200 caractères, non blanc, sans caractère de contrôle. Le corps
/// de la création (nom, ouverture du premier exercice) est lu par le module `referentiel`.
fn nom_dossier(nom: &str) -> Result<String, ErreurApi> {
    let valide = !nom.trim().is_empty()
        && nom.chars().count() <= NOM_DOSSIER_MAX
        && !nom.chars().any(char::is_control);
    if valide {
        Ok(nom.to_owned())
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

/// Exécute un cas d'usage du module dans la transaction du pipeline.
pub(crate) async fn executer(
    cas: Cas,
    tx: &mut TransactionTenant,
    garde: &Garde,
    requete: &Requete<'_>,
) -> Result<Resultat, ErreurApi> {
    match cas {
        Cas::LireCabinet => {
            let cabinet = lire_cabinet(tx).await?.ok_or(ErreurApi::Introuvable)?;
            let vue = VueCabinet::from(garde.cabinet_du_chemin(cabinet)?);
            Ok(Resultat::lecture(Json(vue).into_response()))
        }
        Cas::ListerDossiers => {
            // Filtre de la requête (affectation), puis décision Cedar pour chaque dossier.
            let autorises = garde.filtrer(lister_dossiers_visibles(tx).await?)?;
            let vues: Vec<VueDossier> = autorises.into_iter().map(VueDossier::from).collect();
            Ok(Resultat::lecture(Json(vues).into_response()))
        }
        Cas::LireDossier => {
            let id = tx.contexte().id_dossier().ok_or(ErreurApi::Introuvable)?;
            let dossier = lire_dossier_visible(tx, id)
                .await?
                .ok_or(ErreurApi::Introuvable)?;
            let vue = VueDossier::from(garde.dossier_du_chemin(dossier)?);
            Ok(Resultat::lecture(Json(vue).into_response()))
        }
        Cas::CreerDossier => creer(tx, garde, requete).await,
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
    // Tout le référentiel du dossier (plan, journaux, premier exercice) est calculé en pur
    // avant la première écriture (module `referentiel`, PR 11).
    let demande = referentiel::lire_creation(requete.corps, referentiel::date_du_jour())?;
    let nom = nom_dossier(demande.nom())?;
    let dossier = creer_dossier(tx, &nom).await?;
    affecter_utilisateur_du_contexte(tx, dossier.id).await?;
    let donnees = referentiel::instancier(tx, dossier.id, &demande).await?;
    let id = dossier.id.uuid();

    let mut au_cabinet = Trace::new(PorteeAudit::Cabinet, "dossier", id);
    au_cabinet.champs.push(("nom", None, Some(nom.clone())));
    au_cabinet
        .champs
        .push(("affectation", None, Some(garde.utilisateur().to_string())));
    let mut au_dossier = Trace::new(PorteeAudit::Dossier(dossier.id), "dossier", id);
    au_dossier.champs.push(("nom", None, Some(nom.clone())));
    au_dossier.donnees = donnees;

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
        assert_eq!(nom_dossier("SARL Exemple"), Ok("SARL Exemple".to_owned()));
        assert!(nom_dossier(&"é".repeat(200)).is_ok());
        for invalide in [
            String::new(),
            "   ".to_owned(),
            "a\u{7}b".to_owned(),
            "a".repeat(201),
        ] {
            assert_eq!(
                nom_dossier(&invalide),
                Err(ErreurApi::RequeteInvalide),
                "{invalide}"
            );
        }
    }
}
