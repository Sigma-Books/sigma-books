//! Cas d'isolation des routes du module `pieces` (PR 12 ; ADR 0015).
//!
//! Le harnais ne substitue que `{cabinet}` et `{dossier}` : sur les routes d'une pièce
//! (`{piece}` laissé tel quel), une requête admise jusqu'au cas d'usage reçoit 400 (identifiant
//! illisible). L'isolation **par pièce** (404 identique pour la pièce d'un autre dossier ou
//! d'un autre cabinet) est vérifiée avec de vraies pièces dans `tests/pieces.rs`.

use api::serveur::routes::{Methode, Route};
use axum::http::StatusCode;

use super::{Attendu, CasModule, Monde, Resultat, attendu};

pub(super) const CAS: CasModule = CasModule {
    attentes,
    hors_cabinet: CasModule::AUCUN.hors_cabinet,
};

fn attentes(route: &Route, m: &Monde) -> Option<Vec<Attendu>> {
    use Resultat::{Cree, Statut};
    let (a, b) = (&m.a, &m.b);
    let ok = || Statut(StatusCode::OK);
    let interdit = || Statut(StatusCode::FORBIDDEN);
    let absent = || Statut(StatusCode::NOT_FOUND);
    let illisible = || Statut(StatusCode::BAD_REQUEST);
    Some(match route.chemin {
        // Le même fichier : créé dans le premier dossier de A, signalé en doublon au second
        // dépôt dans ce dossier (200), créé sans avertissement dans un autre dossier ou un autre
        // cabinet (F3.5).
        "/api/cabinets/{cabinet}/dossiers/{dossier}/pieces" if route.methode == Methode::Post => {
            vec![
                attendu(a.administrateur, a.cabinet, a.dossier, Cree),
                attendu(m.valideur, a.cabinet, a.dossier, ok()),
                attendu(m.auditeur, a.cabinet, a.dossier, interdit()),
                attendu(a.collaborateur, a.cabinet, a.autre_dossier, absent()),
                attendu(a.administrateur, a.cabinet, a.autre_dossier, Cree),
                attendu(m.multi, b.cabinet, b.dossier, Cree),
                attendu(b.administrateur, b.cabinet, b.dossier, ok()),
            ]
        }
        "/api/cabinets/{cabinet}/dossiers/{dossier}/pieces/internes" => vec![
            attendu(a.administrateur, a.cabinet, a.dossier, Cree),
            attendu(a.collaborateur, a.cabinet, a.dossier, Cree),
            attendu(m.auditeur, a.cabinet, a.dossier, interdit()),
            attendu(a.collaborateur, a.cabinet, a.autre_dossier, absent()),
            attendu(m.multi, b.cabinet, b.dossier, Cree),
        ],
        "/api/cabinets/{cabinet}/dossiers/{dossier}/pieces" => vec![
            attendu(a.administrateur, a.cabinet, a.autre_dossier, ok()),
            attendu(m.auditeur, a.cabinet, a.dossier, ok()),
            attendu(a.collaborateur, a.cabinet, a.autre_dossier, absent()),
            attendu(m.multi, b.cabinet, b.dossier, ok()),
            attendu(m.multi, b.cabinet, b.autre_dossier, absent()),
        ],
        "/api/cabinets/{cabinet}/dossiers/{dossier}/pieces/{piece}/contenu" => vec![
            attendu(a.administrateur, a.cabinet, a.dossier, illisible()),
            attendu(m.auditeur, a.cabinet, a.dossier, illisible()),
            attendu(a.collaborateur, a.cabinet, a.autre_dossier, absent()),
        ],
        "/api/cabinets/{cabinet}/dossiers/{dossier}/pieces/{piece}/ecart" => vec![
            attendu(a.administrateur, a.cabinet, a.dossier, illisible()),
            attendu(m.auditeur, a.cabinet, a.dossier, interdit()),
            attendu(a.collaborateur, a.cabinet, a.autre_dossier, absent()),
        ],
        _ => return None,
    })
}
