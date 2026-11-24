//! Routes de l'intégrité d'un dossier (PR 15b de `docs/lots/L2.md`, L2-31 ; spec §6.4, §20, §21) :
//! état, jetons, vérification depuis la base, témoin et export d'intégrité. Les cas d'usage vivent
//! dans [`crate::serveur::integrite`].
//!
//! Consultation : administrateur, valideur, auditeur affectés au dossier (`VerifierIntegrite`).
//! Export : mêmes rôles, session fraîche (`ExporterIntegrite`, ré-authentification) ; il est
//! inscrit à l'audit du dossier et alerte l'administrateur.

use authz::Action;

use super::{ActionRoute, CasUsage, Methode, Module, Portee, Requete, Resultat, Route};
use crate::base::TransactionTenant;
use crate::serveur::erreur::ErreurApi;
use crate::serveur::integrite;
use crate::serveur::pipeline::Garde;

/// Cas d'usage du module.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cas {
    /// État de la chaîne et des jetons.
    EtatIntegrite,
    /// Liste des jetons d'horodatage.
    ListerJetons,
    /// Vérification de la chaîne depuis la base (#188).
    VerifierChaine,
    /// Témoin externe courant, en pièce jointe.
    Temoin,
    /// Export d'intégrité (ré-authentification, audit, alerte).
    ExporterIntegrite,
}

/// Tranche du module.
pub const MODULE: Module = Module {
    nom: "integrite",
    routes: ROUTES,
    champs_journal: &[],
};

const fn lecture(chemin: &'static str, cas: Cas) -> Route {
    Route {
        methode: Methode::Get,
        chemin,
        action: ActionRoute::Cabinet(Action::VerifierIntegrite),
        portee: Portee::Dossier,
        cas_usage: CasUsage::Integrite(cas),
    }
}

/// Routes du module.
pub const ROUTES: &[Route] = &[
    lecture(
        "/api/cabinets/{cabinet}/dossiers/{dossier}/integrite",
        Cas::EtatIntegrite,
    ),
    lecture(
        "/api/cabinets/{cabinet}/dossiers/{dossier}/integrite/jetons",
        Cas::ListerJetons,
    ),
    lecture(
        "/api/cabinets/{cabinet}/dossiers/{dossier}/integrite/verification",
        Cas::VerifierChaine,
    ),
    lecture(
        "/api/cabinets/{cabinet}/dossiers/{dossier}/integrite/temoin",
        Cas::Temoin,
    ),
    Route {
        methode: Methode::Post,
        chemin: "/api/cabinets/{cabinet}/dossiers/{dossier}/integrite/export",
        action: ActionRoute::Cabinet(Action::ExporterIntegrite),
        portee: Portee::Dossier,
        cas_usage: CasUsage::Integrite(Cas::ExporterIntegrite),
    },
];

/// Exécute un cas d'usage du module dans la transaction du pipeline.
pub(crate) async fn executer(
    cas: Cas,
    tx: &mut TransactionTenant,
    garde: &Garde,
    requete: &Requete<'_>,
) -> Result<Resultat, ErreurApi> {
    match cas {
        Cas::EtatIntegrite => Ok(Resultat::lecture(integrite::etat(tx, requete).await?)),
        Cas::ListerJetons => Ok(Resultat::lecture(
            integrite::lister_jetons(tx, requete).await?,
        )),
        Cas::VerifierChaine => Ok(Resultat::lecture(integrite::verifier(tx, requete).await?)),
        Cas::Temoin => Ok(Resultat::lecture(integrite::temoin(tx, requete).await?)),
        Cas::ExporterIntegrite => {
            let (reponse, trace) = integrite::exporter(tx, garde, requete).await?;
            Ok(Resultat {
                reponse,
                traces: vec![trace],
            })
        }
    }
}
