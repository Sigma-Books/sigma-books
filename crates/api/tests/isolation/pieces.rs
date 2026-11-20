//! Cas d'isolation des routes du module `pieces` (PR 12 ; ADR 0015).
//!
//! Chaque dossier du monde a une pièce réelle, sous-ressource `{piece}` de ses routes : le
//! harnais vérifie en outre, pour chaque route d'une pièce, que la pièce d'un autre dossier
//! (de B, ou non affecté) placée sous un dossier de A donne le même 404 qu'une pièce
//! inexistante. Isolation détaillée, doublons et altérations : `tests/pieces.rs`.

use api::serveur::routes::{Methode, Route};
use axum::http::StatusCode;

use super::{Attendu, CasModule, Monde, Resultat, attendu, attendu_croise};

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
            attendu(a.administrateur, a.cabinet, a.dossier, ok()),
            attendu(m.auditeur, a.cabinet, a.dossier, ok()),
            attendu(a.collaborateur, a.cabinet, a.autre_dossier, absent()),
            attendu(m.multi, b.cabinet, b.dossier, ok()),
            // Pièce du second dossier de A, sous le premier : absente même pour
            // l'administrateur.
            attendu_croise(
                a.administrateur,
                a.cabinet,
                a.dossier,
                a.autre_dossier,
                absent(),
            ),
        ],
        "/api/cabinets/{cabinet}/dossiers/{dossier}/pieces/{piece}/ecart" => vec![
            attendu(m.auditeur, a.cabinet, a.dossier, interdit()),
            attendu(a.collaborateur, a.cabinet, a.autre_dossier, absent()),
            attendu_croise(m.multi, b.cabinet, b.dossier, a.dossier, absent()),
            attendu(a.administrateur, a.cabinet, a.dossier, ok()),
            // Déjà écartée : absente.
            attendu(m.valideur, a.cabinet, a.dossier, absent()),
        ],
        _ => return None,
    })
}
