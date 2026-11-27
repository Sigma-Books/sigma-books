//! Cas d'isolation des routes du module `parametres_fiscaux` (ADR 0030) : lecture des versions du
//! dossier (tout rôle qui voit le dossier) et création d'une version propre (administrateur du
//! cabinet seul). Dossier non affecté ou d'un autre cabinet : ressource absente (404 identique).
//! Le corps de test de la création ne change rien : refusé (422) pour qui a le droit, sans rien
//! créer. Résultats détaillés : `tests/parametres_dossier.rs`.

use api::serveur::routes::{CasUsage, Methode, Route};
use axum::http::StatusCode;

use super::{Attendu, CasModule, Monde, Resultat, attendu};

pub(super) const CAS: CasModule = CasModule {
    attentes,
    hors_cabinet: CasModule::AUCUN.hors_cabinet,
};

fn attentes(route: &Route, m: &Monde) -> Option<Vec<Attendu>> {
    use Resultat::Statut;
    let (a, b) = (&m.a, &m.b);
    let interdit = || Statut(StatusCode::FORBIDDEN);
    let absent = || Statut(StatusCode::NOT_FOUND);
    let ok = || Statut(StatusCode::OK);
    let refuse = || Statut(StatusCode::UNPROCESSABLE_ENTITY);
    if !matches!(route.cas_usage, CasUsage::ParametresFiscaux(_)) {
        return None;
    }
    Some(if route.methode == Methode::Get {
        vec![
            attendu(a.administrateur, a.cabinet, a.dossier, ok()),
            attendu(a.administrateur, a.cabinet, a.autre_dossier, ok()),
            attendu(m.valideur, a.cabinet, a.dossier, ok()),
            attendu(m.auditeur, a.cabinet, a.dossier, ok()),
            attendu(a.collaborateur, a.cabinet, a.dossier, ok()),
            attendu(a.collaborateur, a.cabinet, a.autre_dossier, absent()),
            attendu(m.valideur, a.cabinet, a.autre_dossier, absent()),
            attendu(m.multi, a.cabinet, a.autre_dossier, ok()),
            // Membre des deux cabinets : il lit les versions du dossier qui lui est affecté dans
            // le second, jamais celles d'un dossier non affecté.
            attendu(m.multi, b.cabinet, b.dossier, ok()),
            attendu(m.multi, b.cabinet, b.autre_dossier, absent()),
        ]
    } else {
        vec![
            attendu(a.administrateur, a.cabinet, a.dossier, refuse()),
            attendu(a.administrateur, a.cabinet, a.autre_dossier, refuse()),
            attendu(m.valideur, a.cabinet, a.dossier, interdit()),
            attendu(m.auditeur, a.cabinet, a.dossier, interdit()),
            attendu(a.collaborateur, a.cabinet, a.dossier, interdit()),
            attendu(a.collaborateur, a.cabinet, a.autre_dossier, absent()),
            attendu(m.multi, a.cabinet, a.autre_dossier, refuse()),
            attendu(m.multi, b.cabinet, b.dossier, interdit()),
            attendu(m.multi, b.cabinet, b.autre_dossier, absent()),
        ]
    })
}
