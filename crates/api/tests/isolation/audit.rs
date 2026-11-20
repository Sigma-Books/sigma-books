//! Cas d'isolation des routes du module `audit` : audit du cabinet et d'un dossier.

use api::serveur::routes::{CasUsage, Route, audit};
use axum::http::StatusCode;

use super::{Attendu, CasModule, Monde, Resultat, attendu};

pub(super) const CAS: CasModule = CasModule {
    attentes,
    hors_cabinet: CasModule::AUCUN.hors_cabinet,
};

/// Cas d'isolation propres à chaque route, dont le membre des deux cabinets, qui n'agit que
/// selon son rôle dans le cabinet du chemin. `None` : route sans cas, le test échoue.
fn attentes(route: &Route, m: &Monde) -> Option<Vec<Attendu>> {
    use Resultat::Statut;
    let (a, b) = (&m.a, &m.b);
    let interdit = || Statut(StatusCode::FORBIDDEN);
    let absent = || Statut(StatusCode::NOT_FOUND);
    Some(match route.chemin {
        // Audit du cabinet : administrateur seul (ADR 0016), 403 pour les autres membres.
        "/api/cabinets/{cabinet}/audit"
        | "/api/cabinets/{cabinet}/audit/verification"
        | "/api/cabinets/{cabinet}/audit/export" => {
            let ok = || resultat_audit(route);
            vec![
                attendu(a.administrateur, a.cabinet, a.dossier, ok()),
                attendu(m.valideur, a.cabinet, a.dossier, interdit()),
                attendu(m.auditeur, a.cabinet, a.dossier, interdit()),
                attendu(a.collaborateur, a.cabinet, a.dossier, interdit()),
                attendu(m.multi, a.cabinet, a.dossier, ok()),
                attendu(m.multi, b.cabinet, b.dossier, interdit()),
            ]
        }
        // Audit d'un dossier : administrateur, valideur, auditeur affectés ; collaborateur 403 ;
        // dossier non affecté 404.
        "/api/cabinets/{cabinet}/dossiers/{dossier}/audit"
        | "/api/cabinets/{cabinet}/dossiers/{dossier}/audit/verification"
        | "/api/cabinets/{cabinet}/dossiers/{dossier}/audit/export" => {
            let ok = || resultat_audit(route);
            vec![
                attendu(a.administrateur, a.cabinet, a.dossier, ok()),
                attendu(a.administrateur, a.cabinet, a.autre_dossier, ok()),
                attendu(m.valideur, a.cabinet, a.dossier, ok()),
                attendu(m.auditeur, a.cabinet, a.dossier, ok()),
                attendu(a.collaborateur, a.cabinet, a.dossier, interdit()),
                attendu(m.valideur, a.cabinet, a.autre_dossier, absent()),
                attendu(m.auditeur, a.cabinet, a.autre_dossier, absent()),
                attendu(m.multi, a.cabinet, a.autre_dossier, ok()),
                attendu(m.multi, b.cabinet, b.dossier, interdit()),
            ]
        }
        _ => return None,
    })
}

/// Résultat d'un appel autorisé d'une route d'audit.
fn resultat_audit(route: &Route) -> Resultat {
    match route.cas_usage {
        CasUsage::Audit(audit::Cas::ConsulterAudit) => Resultat::PageAudit,
        CasUsage::Audit(audit::Cas::VerifierAudit) => Resultat::Verification,
        _ => Resultat::ExportAudit,
    }
}
