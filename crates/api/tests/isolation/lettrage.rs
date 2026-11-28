//! Cas d'isolation des routes du module `lettrage` (PR 9a de `docs/lots/L3.md`, L3-18 ; ADR 0032).
//!
//! Rôles (spec §20, `roles/lettrage.cedar` et `roles/tiers.cedar`) : lettrer et dé-lettrer
//! (`Lettrer`, `Delettrer`) : administrateur, valideur, collaborateur ; lecture
//! (`ConsulterEditionsTiers`) : les quatre rôles du cabinet. Le membre des deux cabinets est
//! administrateur de A et collaborateur de B. Les cas génériques du harnais ajoutent, pour chaque
//! route, l'autre cabinet, le dossier non affecté, la sous-ressource (lettrage, tiers, compte)
//! d'un autre dossier placée dans le chemin (404 identique) et l'absence de session.
//!
//! - Le lettrage d'exemple cite deux lignes inexistantes : 422 `lettrage_ligne_inconnue` pour qui
//!   a le droit (le rôle et le dossier sont contrôlés **avant** les lignes), rien n'est écrit.
//! - La rupture porte sur le lettrage fixe de chaque dossier : elle réussit **une fois par
//!   dossier** ; le second appel sur le même dossier est refusé (409 `lettrage_deja_rompu`), ce
//!   qui prouve que le premier a bien écrit.
//! - La lecture d'un tiers rend le tiers fixe de son dossier ; celle du sous-compte fixe, qui n'est
//!   pas lettrable, est refusée (422) pour qui a le droit de lire.

use api::base::IdDossier;
use api::serveur::routes::lettrage::Cas;
use api::serveur::routes::{CasUsage, Route};
use axum::http::StatusCode;

use super::{Attendu, CasModule, Monde, Resultat, attendu, attendu_croise};

pub(super) const CAS: CasModule = CasModule {
    attentes,
    hors_cabinet: CasModule::AUCUN.hors_cabinet,
};

#[allow(clippy::too_many_lines)]
fn attentes(route: &Route, m: &Monde) -> Option<Vec<Attendu>> {
    use Resultat::{Cree, Ressource, Statut};
    let (a, b) = (&m.a, &m.b);
    let interdit = || Statut(StatusCode::FORBIDDEN);
    let absent = || Statut(StatusCode::NOT_FOUND);
    let refuse = || Statut(StatusCode::UNPROCESSABLE_ENTITY);
    let conflit = || Statut(StatusCode::CONFLICT);
    let CasUsage::Lettrage(cas) = route.cas_usage else {
        return None;
    };
    Some(match cas {
        Cas::Lettrer => vec![
            attendu(a.administrateur, a.cabinet, a.dossier, refuse()),
            attendu(a.administrateur, a.cabinet, a.autre_dossier, refuse()),
            attendu(m.valideur, a.cabinet, a.dossier, refuse()),
            attendu(a.collaborateur, a.cabinet, a.dossier, refuse()),
            attendu(m.auditeur, a.cabinet, a.dossier, interdit()),
            attendu(a.collaborateur, a.cabinet, a.autre_dossier, absent()),
            attendu(m.valideur, a.cabinet, a.autre_dossier, absent()),
            attendu(m.multi, b.cabinet, b.dossier, refuse()),
            attendu(m.multi, b.cabinet, b.autre_dossier, absent()),
        ],
        Cas::Rompre => vec![
            // L'auditeur, refusé par Cedar, ne consomme pas la rupture.
            attendu(m.auditeur, a.cabinet, a.dossier, interdit()),
            attendu_croise(a.administrateur, a.cabinet, a.dossier, b.dossier, absent()),
            attendu(a.administrateur, a.cabinet, a.dossier, Cree),
            attendu(m.valideur, a.cabinet, a.dossier, conflit()),
            attendu(a.collaborateur, a.cabinet, a.dossier, conflit()),
            attendu(a.collaborateur, a.cabinet, a.autre_dossier, absent()),
            attendu(m.multi, a.cabinet, a.autre_dossier, Cree),
            // Collaborateur affecté dans B : le dé-lettrage lui est permis.
            attendu(m.multi, b.cabinet, b.dossier, Cree),
            attendu(m.multi, b.cabinet, b.autre_dossier, absent()),
        ],
        Cas::LireTiers => {
            let tiers = |d: IdDossier| Ressource(m.fixtures[&d].tiers);
            vec![
                attendu(a.administrateur, a.cabinet, a.dossier, tiers(a.dossier)),
                attendu(
                    a.administrateur,
                    a.cabinet,
                    a.autre_dossier,
                    tiers(a.autre_dossier),
                ),
                attendu(m.valideur, a.cabinet, a.dossier, tiers(a.dossier)),
                attendu(m.auditeur, a.cabinet, a.dossier, tiers(a.dossier)),
                attendu(a.collaborateur, a.cabinet, a.dossier, tiers(a.dossier)),
                attendu(a.collaborateur, a.cabinet, a.autre_dossier, absent()),
                attendu(m.multi, b.cabinet, b.dossier, tiers(b.dossier)),
                attendu(m.multi, b.cabinet, b.autre_dossier, absent()),
                attendu_croise(m.valideur, a.cabinet, a.dossier, a.autre_dossier, absent()),
            ]
        }
        Cas::LireCompte => vec![
            attendu(a.administrateur, a.cabinet, a.dossier, refuse()),
            attendu(m.valideur, a.cabinet, a.dossier, refuse()),
            attendu(m.auditeur, a.cabinet, a.dossier, refuse()),
            attendu(a.collaborateur, a.cabinet, a.dossier, refuse()),
            attendu(a.collaborateur, a.cabinet, a.autre_dossier, absent()),
            attendu(m.multi, b.cabinet, b.dossier, refuse()),
            attendu(m.multi, b.cabinet, b.autre_dossier, absent()),
            attendu_croise(a.administrateur, a.cabinet, a.dossier, b.dossier, absent()),
        ],
    })
}
