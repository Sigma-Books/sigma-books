//! Cas d'isolation des routes du module `integrite` (PR 15b de `docs/lots/L2.md`, L2-31) : état,
//! jetons, vérification, témoin et export d'intégrité d'un dossier. Administrateur, valideur et
//! auditeur affectés au dossier ; collaborateur refusé (403) ; dossier non affecté ou d'un autre
//! cabinet : ressource absente (404 identique). Résultats chiffrés : `tests/integrite.rs`.

use api::serveur::routes::{CasUsage, Route};
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
    // Les écritures semées par le harnais ne forment pas une vraie chaîne : la vérification
    // répond 200 (rapport), sans exiger « intègre » ; ce résultat est testé dans `integrite.rs`.
    let ok = || Statut(StatusCode::OK);
    matches!(route.cas_usage, CasUsage::Integrite(_)).then(|| {
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
            attendu(m.multi, b.cabinet, b.autre_dossier, absent()),
        ]
    })
}
