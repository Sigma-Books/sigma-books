//! Routes du journal d'audit (PR 7 ; ADR 0014 et 0016) : consultation, vérification et export
//! de l'audit du cabinet et d'un dossier. Les cas d'usage vivent dans [`crate::serveur::audit`].

use authz::Action;

use super::{ActionRoute, CasUsage, Methode, Module, Portee, Requete, Resultat, Route};
use crate::base::TransactionTenant;
use crate::serveur::audit;
use crate::serveur::erreur::ErreurApi;
use crate::serveur::pipeline::Garde;

/// Cas d'usage du module.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cas {
    /// Consultation paginée de l'audit de la portée du chemin.
    ConsulterAudit,
    /// Vérification de la chaîne d'audit de la portée du chemin.
    VerifierAudit,
    /// Export de l'audit de la portée du chemin, inscrit lui-même à l'audit.
    ExporterAudit,
}

/// Tranche du module.
pub const MODULE: Module = Module {
    nom: "audit",
    routes: ROUTES,
    champs_journal: &[],
};

/// Routes du module.
pub const ROUTES: &[Route] = &[
    // Audit du cabinet : administrateur seul (ADR 0016) ; l'export exige la ré-authentification.
    Route {
        methode: Methode::Get,
        chemin: "/api/cabinets/{cabinet}/audit",
        action: ActionRoute::Cabinet(Action::ConsulterAuditCabinet),
        portee: Portee::Cabinet,
        cas_usage: CasUsage::Audit(Cas::ConsulterAudit),
    },
    Route {
        methode: Methode::Get,
        chemin: "/api/cabinets/{cabinet}/audit/verification",
        action: ActionRoute::Cabinet(Action::VerifierAuditCabinet),
        portee: Portee::Cabinet,
        cas_usage: CasUsage::Audit(Cas::VerifierAudit),
    },
    Route {
        methode: Methode::Post,
        chemin: "/api/cabinets/{cabinet}/audit/export",
        action: ActionRoute::Cabinet(Action::ExporterAuditCabinet),
        portee: Portee::Cabinet,
        cas_usage: CasUsage::Audit(Cas::ExporterAudit),
    },
    // Audit d'un dossier : administrateur, valideur, auditeur affectés (§20, ADR 0016). La
    // vérification relève de la consultation ; l'export, de l'export complet (ré-authentifié).
    Route {
        methode: Methode::Get,
        chemin: "/api/cabinets/{cabinet}/dossiers/{dossier}/audit",
        action: ActionRoute::Cabinet(Action::ConsulterAudit),
        portee: Portee::Dossier,
        cas_usage: CasUsage::Audit(Cas::ConsulterAudit),
    },
    Route {
        methode: Methode::Get,
        chemin: "/api/cabinets/{cabinet}/dossiers/{dossier}/audit/verification",
        action: ActionRoute::Cabinet(Action::ConsulterAudit),
        portee: Portee::Dossier,
        cas_usage: CasUsage::Audit(Cas::VerifierAudit),
    },
    Route {
        methode: Methode::Post,
        chemin: "/api/cabinets/{cabinet}/dossiers/{dossier}/audit/export",
        action: ActionRoute::Cabinet(Action::ExporterComplet),
        portee: Portee::Dossier,
        cas_usage: CasUsage::Audit(Cas::ExporterAudit),
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
        Cas::ConsulterAudit => Ok(Resultat::lecture(
            audit::consulter(tx, garde, requete).await?,
        )),
        Cas::VerifierAudit => Ok(Resultat::lecture(
            audit::verifier(tx, garde, requete).await?,
        )),
        Cas::ExporterAudit => {
            let (reponse, trace) = audit::exporter(tx, garde, requete).await?;
            Ok(Resultat {
                reponse,
                traces: vec![trace],
            })
        }
    }
}
