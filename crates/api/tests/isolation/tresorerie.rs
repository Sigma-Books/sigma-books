//! Cas d'isolation des routes du module `tresorerie` (PR 11 de `docs/lots/L3.md`, L3-21).
//!
//! Rôles (plan C6, `roles/tresorerie.cedar`) : lecture `ConsulterTresorerie` (administrateur,
//! valideur, collaborateur, auditeur) ; comptes `ParametrerTresorerie` (administrateur,
//! valideur) ; arrêté `ArreterCaisse` (administrateur, valideur, collaborateur). Dossier non
//! affecté ou d'un autre cabinet, compte de trésorerie d'un autre dossier : absent (404
//! identique, contrôles génériques du harnais). Le membre des deux cabinets est administrateur
//! de A et collaborateur de B.
//!
//! La modification renomme la caisse fixe : le second appel sur le même dossier ne change rien
//! (409 `aucun_changement`), ce qui prouve que le premier a bien écrit.

use api::serveur::routes::tresorerie::Cas;
use api::serveur::routes::{CasUsage, Route};
use axum::http::StatusCode;
use uuid::Uuid;

use super::{Attendu, CasModule, Monde, Resultat, attendu};

pub(super) const CAS: CasModule = CasModule {
    attentes,
    hors_cabinet: CasModule::AUCUN.hors_cabinet,
};

fn sous_ressource(dossier: api::base::IdDossier) -> Uuid {
    super::SOUS_RESSOURCES
        .lock()
        .unwrap()
        .get(&(dossier.uuid(), "compte_tresorerie"))
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
    let CasUsage::Tresorerie(cas) = route.cas_usage else {
        return None;
    };
    Some(match cas {
        Cas::Lister | Cas::VirementsInternes | Cas::ListerArretes | Cas::Lire => {
            let lu = |d| {
                if cas == Cas::Lire {
                    Ressource(sous_ressource(d))
                } else {
                    ok()
                }
            };
            vec![
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
            ]
        }
        Cas::Creer => vec![
            attendu(a.administrateur, a.cabinet, a.dossier, Cree),
            attendu(m.valideur, a.cabinet, a.dossier, Cree),
            attendu(m.auditeur, a.cabinet, a.dossier, interdit()),
            attendu(a.collaborateur, a.cabinet, a.dossier, interdit()),
            attendu(a.collaborateur, a.cabinet, a.autre_dossier, absent()),
            attendu(m.multi, a.cabinet, a.autre_dossier, Cree),
            // Collaborateur dans B : le paramétrage lui est refusé.
            attendu(m.multi, b.cabinet, b.dossier, interdit()),
            attendu(m.multi, b.cabinet, b.autre_dossier, absent()),
        ],
        Cas::Modifier => vec![
            attendu(
                a.administrateur,
                a.cabinet,
                a.dossier,
                Ressource(sous_ressource(a.dossier)),
            ),
            attendu(
                m.valideur,
                a.cabinet,
                a.dossier,
                Statut(StatusCode::CONFLICT),
            ),
            attendu(m.auditeur, a.cabinet, a.dossier, interdit()),
            attendu(a.collaborateur, a.cabinet, a.dossier, interdit()),
            attendu(a.collaborateur, a.cabinet, a.autre_dossier, absent()),
            attendu(
                m.multi,
                a.cabinet,
                a.autre_dossier,
                Ressource(sous_ressource(a.autre_dossier)),
            ),
            attendu(m.multi, b.cabinet, b.dossier, interdit()),
            attendu(m.multi, b.cabinet, b.autre_dossier, absent()),
        ],
        Cas::ArreterCaisse => vec![
            attendu(a.administrateur, a.cabinet, a.dossier, Cree),
            attendu(m.valideur, a.cabinet, a.dossier, Cree),
            attendu(a.collaborateur, a.cabinet, a.dossier, Cree),
            attendu(m.auditeur, a.cabinet, a.dossier, interdit()),
            attendu(a.collaborateur, a.cabinet, a.autre_dossier, absent()),
            attendu(m.multi, a.cabinet, a.autre_dossier, Cree),
            // Collaborateur affecté dans B : l'arrêté lui est permis.
            attendu(m.multi, b.cabinet, b.dossier, Cree),
            attendu(m.multi, b.cabinet, b.autre_dossier, absent()),
        ],
    })
}
