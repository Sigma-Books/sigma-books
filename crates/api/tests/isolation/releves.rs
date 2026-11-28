//! Cas d'isolation des routes du module `releves` (PR 12a de `docs/lots/L3.md`, L3-22).
//!
//! Rôles (plan C6, `roles/tresorerie.cedar`) : lecture `ConsulterTresorerie` (administrateur,
//! valideur, collaborateur, auditeur) ; profils `ParametrerReleves` (administrateur, valideur) ;
//! dépôt `DeposerReleve` et analyse `ImporterReleve` (administrateur, valideur, collaborateur).
//! Chaque dossier du monde a une caisse, un profil et un dépôt réels (`{compte_tresorerie}`,
//! `{profil}`, `{depot}`) : ceux d'un autre dossier, placés sous un dossier de A, sont absents
//! (404 identique à un identifiant inexistant). Le membre des deux cabinets est administrateur de
//! A et collaborateur de B.
//!
//! Le dépôt envoie toujours le même fichier : créé au premier dépôt d'un dossier (201), signalé
//! en doublon ensuite (200, rien de stocké), créé sans avertissement dans un autre dossier (F3.5).

use api::serveur::routes::releves::Cas;
use api::serveur::routes::{CasUsage, Route};
use axum::http::StatusCode;
use uuid::Uuid;

use super::{Attendu, CasModule, Monde, Resultat, attendu, attendu_croise};

pub(super) const CAS: CasModule = CasModule {
    attentes,
    hors_cabinet: CasModule::AUCUN.hors_cabinet,
};

fn sous_ressource(dossier: api::base::IdDossier, nom: &'static str) -> Uuid {
    super::SOUS_RESSOURCES
        .lock()
        .unwrap()
        .get(&(dossier.uuid(), nom))
        .copied()
        .unwrap()
}

#[allow(clippy::too_many_lines)]
fn attentes(route: &Route, m: &Monde) -> Option<Vec<Attendu>> {
    use Resultat::{Cree, Ressource, Statut};
    let (a, b) = (&m.a, &m.b);
    let interdit = || Statut(StatusCode::FORBIDDEN);
    let absent = || Statut(StatusCode::NOT_FOUND);
    let ok = || Statut(StatusCode::OK);
    let CasUsage::Releves(cas) = route.cas_usage else {
        return None;
    };
    Some(match cas {
        Cas::ListerProfils | Cas::ListerDepots | Cas::LireProfil | Cas::LireDepot => {
            let lu = |d| match cas {
                Cas::LireProfil => Ressource(sous_ressource(d, "profil")),
                Cas::LireDepot => Ressource(sous_ressource(d, "depot")),
                _ => ok(),
            };
            let mut v = vec![
                attendu(a.administrateur, a.cabinet, a.dossier, lu(a.dossier)),
                attendu(
                    a.administrateur,
                    a.cabinet,
                    a.autre_dossier,
                    lu(a.autre_dossier),
                ),
                attendu(m.valideur, a.cabinet, a.dossier, lu(a.dossier)),
                attendu(m.auditeur, a.cabinet, a.dossier, lu(a.dossier)),
                attendu(a.collaborateur, a.cabinet, a.dossier, lu(a.dossier)),
                attendu(a.collaborateur, a.cabinet, a.autre_dossier, absent()),
                attendu(m.valideur, a.cabinet, a.autre_dossier, absent()),
                attendu(m.multi, b.cabinet, b.dossier, lu(b.dossier)),
                attendu(m.multi, b.cabinet, b.autre_dossier, absent()),
            ];
            if matches!(cas, Cas::LireProfil | Cas::LireDepot) {
                v.push(attendu_croise(
                    a.administrateur,
                    a.cabinet,
                    a.dossier,
                    a.autre_dossier,
                    absent(),
                ));
            }
            v
        }
        Cas::CreerProfil | Cas::VersionnerProfil => {
            let mut v = vec![
                attendu(a.administrateur, a.cabinet, a.dossier, Cree),
                attendu(m.valideur, a.cabinet, a.dossier, Cree),
                attendu(m.auditeur, a.cabinet, a.dossier, interdit()),
                attendu(a.collaborateur, a.cabinet, a.dossier, interdit()),
                attendu(a.collaborateur, a.cabinet, a.autre_dossier, absent()),
                attendu(m.multi, a.cabinet, a.autre_dossier, Cree),
                // Collaborateur dans B : le paramétrage lui est refusé.
                attendu(m.multi, b.cabinet, b.dossier, interdit()),
                attendu(m.multi, b.cabinet, b.autre_dossier, absent()),
            ];
            if cas == Cas::VersionnerProfil {
                // Profil du second dossier de A, sous le premier : absent.
                v.push(attendu_croise(
                    a.administrateur,
                    a.cabinet,
                    a.dossier,
                    a.autre_dossier,
                    absent(),
                ));
            }
            v
        }
        Cas::Deposer => vec![
            attendu(a.administrateur, a.cabinet, a.dossier, Cree),
            // Même fichier, même dossier : doublon signalé, rien de stocké.
            attendu(m.valideur, a.cabinet, a.dossier, ok()),
            attendu(a.collaborateur, a.cabinet, a.dossier, ok()),
            attendu(m.auditeur, a.cabinet, a.dossier, interdit()),
            attendu(a.collaborateur, a.cabinet, a.autre_dossier, absent()),
            attendu(m.multi, a.cabinet, a.autre_dossier, Cree),
            // Collaborateur affecté dans B : le dépôt lui est permis.
            attendu(m.multi, b.cabinet, b.dossier, Cree),
            attendu(m.multi, b.cabinet, b.autre_dossier, absent()),
            // Caisse du second dossier de A, sous le premier : absente.
            attendu_croise(
                a.administrateur,
                a.cabinet,
                a.dossier,
                a.autre_dossier,
                absent(),
            ),
        ],
        Cas::Analyser => vec![
            attendu(a.administrateur, a.cabinet, a.dossier, ok()),
            attendu(m.valideur, a.cabinet, a.dossier, ok()),
            attendu(a.collaborateur, a.cabinet, a.dossier, ok()),
            attendu(m.auditeur, a.cabinet, a.dossier, interdit()),
            attendu(a.collaborateur, a.cabinet, a.autre_dossier, absent()),
            attendu(m.multi, a.cabinet, a.autre_dossier, ok()),
            attendu(m.multi, b.cabinet, b.dossier, ok()),
            attendu(m.multi, b.cabinet, b.autre_dossier, absent()),
            // Dépôt et profil du second dossier de A, sous le premier : absents.
            attendu_croise(
                a.administrateur,
                a.cabinet,
                a.dossier,
                a.autre_dossier,
                absent(),
            ),
        ],
    })
}
