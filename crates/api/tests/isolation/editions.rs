//! Cas d'isolation des routes du module `editions` (PR 14 de `docs/lots/L2.md`, L2-28 et L2-29) :
//! éditions et alertes, toutes en lecture, ouvertes aux quatre rôles du cabinet (§20) sur les
//! dossiers qu'ils voient. Résultats chiffrés, filtres et alertes : `tests/editions.rs`.

use api::serveur::routes::Route;
use axum::http::StatusCode;

use super::{Attendu, CasModule, Monde, Resultat, attendu};

pub(super) const CAS: CasModule = CasModule {
    attentes,
    hors_cabinet: CasModule::AUCUN.hors_cabinet,
};

fn attentes(route: &Route, m: &Monde) -> Option<Vec<Attendu>> {
    use Resultat::Statut;
    let (a, b) = (&m.a, &m.b);
    let ok = || Statut(StatusCode::OK);
    let absent = || Statut(StatusCode::NOT_FOUND);
    let edition = route.chemin.contains("/editions/") || route.chemin.ends_with("/alertes");
    edition.then(|| {
        vec![
            attendu(a.administrateur, a.cabinet, a.autre_dossier, ok()),
            // L'auditeur consulte les éditions.
            attendu(m.auditeur, a.cabinet, a.dossier, ok()),
            attendu(a.collaborateur, a.cabinet, a.autre_dossier, absent()),
            attendu(m.multi, b.cabinet, b.dossier, ok()),
            attendu(m.multi, b.cabinet, b.autre_dossier, absent()),
        ]
    })
}
