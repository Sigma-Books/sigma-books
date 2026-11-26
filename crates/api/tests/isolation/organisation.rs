//! Cas d'isolation des routes du module `organisation` : cabinet et dossiers.

use api::serveur::routes::{Methode, Route};
use axum::http::StatusCode;

use super::{Attendu, CasModule, Monde, Resultat, attendu};

pub(super) const CAS: CasModule = CasModule {
    attentes,
    hors_cabinet: CasModule::AUCUN.hors_cabinet,
};

/// Cas d'isolation propres à chaque route, dont le membre des deux cabinets, qui n'agit que
/// selon son rôle dans le cabinet du chemin. `None` : route sans cas, le test échoue.
fn attentes(route: &Route, m: &Monde) -> Option<Vec<Attendu>> {
    use Resultat::{Cree, Liste, Ressource, Statut};
    let (a, b) = (&m.a, &m.b);
    let interdit = || Statut(StatusCode::FORBIDDEN);
    let absent = || Statut(StatusCode::NOT_FOUND);
    Some(match route.chemin {
        "/api/cabinets/{cabinet}" => vec![
            attendu(
                a.administrateur,
                a.cabinet,
                a.dossier,
                Ressource(a.cabinet.uuid()),
            ),
            attendu(a.collaborateur, a.cabinet, a.dossier, interdit()),
            attendu(m.multi, a.cabinet, a.dossier, Ressource(a.cabinet.uuid())),
            attendu(m.multi, b.cabinet, b.dossier, interdit()),
        ],
        "/api/cabinets/{cabinet}/dossiers" if route.methode == Methode::Get => vec![
            attendu(
                a.administrateur,
                a.cabinet,
                a.dossier,
                Liste(vec![a.dossier, a.autre_dossier]),
            ),
            attendu(
                a.collaborateur,
                a.cabinet,
                a.dossier,
                Liste(vec![a.dossier]),
            ),
            attendu(
                m.multi,
                a.cabinet,
                a.dossier,
                Liste(vec![a.dossier, a.autre_dossier]),
            ),
            attendu(m.multi, b.cabinet, b.dossier, Liste(vec![b.dossier])),
        ],
        "/api/cabinets/{cabinet}/dossiers/{dossier}" => vec![
            attendu(
                a.administrateur,
                a.cabinet,
                a.autre_dossier,
                Ressource(a.autre_dossier.uuid()),
            ),
            attendu(
                a.collaborateur,
                a.cabinet,
                a.dossier,
                Ressource(a.dossier.uuid()),
            ),
            attendu(a.collaborateur, a.cabinet, a.autre_dossier, absent()),
            attendu(
                m.multi,
                a.cabinet,
                a.autre_dossier,
                Ressource(a.autre_dossier.uuid()),
            ),
            attendu(m.multi, b.cabinet, b.dossier, Ressource(b.dossier.uuid())),
            attendu(m.multi, b.cabinet, b.autre_dossier, absent()),
        ],
        "/api/cabinets/{cabinet}/dossiers" if route.methode == Methode::Post => vec![
            attendu(a.administrateur, a.cabinet, a.dossier, Cree),
            attendu(m.valideur, a.cabinet, a.dossier, Cree),
            attendu(a.collaborateur, a.cabinet, a.dossier, interdit()),
            attendu(m.auditeur, a.cabinet, a.dossier, interdit()),
            attendu(m.multi, a.cabinet, a.dossier, Cree),
            attendu(m.multi, b.cabinet, b.dossier, interdit()),
        ],
        // Pack pays : mêmes droits que la création d'un dossier, jamais le cabinet d'autrui.
        "/api/cabinets/{cabinet}/pays" => vec![
            attendu(
                a.administrateur,
                a.cabinet,
                a.dossier,
                Statut(StatusCode::OK),
            ),
            attendu(m.valideur, a.cabinet, a.dossier, Statut(StatusCode::OK)),
            attendu(a.collaborateur, a.cabinet, a.dossier, interdit()),
            attendu(m.auditeur, a.cabinet, a.dossier, interdit()),
            attendu(m.multi, a.cabinet, a.dossier, Statut(StatusCode::OK)),
            attendu(m.multi, b.cabinet, b.dossier, interdit()),
        ],
        _ => return None,
    })
}
