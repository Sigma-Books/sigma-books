//! Cas d'isolation des routes du module `echeancier` (PR 8 de `docs/lots/L3.md`, L3-17 ;
//! ADR 0033).
//!
//! Rôles : lecture des conditions `ConsulterTiers` et des échéances `ConsulterEditionsTiers`
//! (administrateur, valideur, collaborateur, auditeur) ; conditions `ParametrerConditionsReglement`
//! (administrateur, valideur) ; assistant `SaisirBrouillard` et révision `ReviserEcheance`
//! (administrateur, valideur, collaborateur). Chaque dossier du monde a sa condition fixe
//! (`{condition}`) et son écriture validée fixe (`{ecriture}`, sans ligne de tiers). Une
//! condition ou une écriture d'un autre dossier placée dans le chemin d'un dossier de A est
//! absente (404 identique) ; les contrôles génériques du harnais ajoutent l'autre cabinet, le
//! dossier non affecté et l'absence de session.
//!
//! - L'assistant est appelé sans paramètre : qui peut l'utiliser reçoit 400 **après** avoir
//!   trouvé la condition de son dossier (celle d'autrui : 404).
//! - La révision vise la ligne 1 de l'écriture fixe, qui n'a pas de tiers : 422
//!   `echeance_sans_tiers` pour qui peut réviser (l'écriture d'autrui : 404). Les révisions
//!   réussies sont dans `tests/echeancier.rs` et dans le test d'audit de chaque mutation.
//! - La création réussit une fois par dossier ; la seconde, même code : 409. La modification
//!   renomme la condition fixe : le second appel sur le même dossier ne change rien (409).

use api::base::IdDossier;
use api::serveur::routes::echeancier::Cas;
use api::serveur::routes::{CasUsage, Route};
use axum::http::StatusCode;
use uuid::Uuid;

use super::{Attendu, CasModule, Monde, Resultat, attendu, attendu_croise};

pub(super) const CAS: CasModule = CasModule {
    attentes,
    hors_cabinet: CasModule::AUCUN.hors_cabinet,
};

fn sous_ressource(dossier: IdDossier, nom: &str) -> Uuid {
    super::SOUS_RESSOURCES
        .lock()
        .unwrap()
        .iter()
        .find_map(|((d, n), id)| (*d == dossier.uuid() && *n == nom).then_some(*id))
        .unwrap()
}

/// Liste des conditions de `dossier` : sa condition fixe, aucune de celles des autres dossiers.
fn liste(m: &Monde, dossier: IdDossier) -> Resultat {
    Resultat::ListeContient {
        cle: "conditions",
        present: sous_ressource(dossier, "condition"),
        absents: m
            .fixtures
            .keys()
            .filter(|d| **d != dossier)
            .map(|d| sous_ressource(*d, "condition"))
            .collect(),
    }
}

#[allow(clippy::too_many_lines)]
fn attentes(route: &Route, m: &Monde) -> Option<Vec<Attendu>> {
    use Resultat::{Cree, Ressource, Statut};
    let (a, b) = (&m.a, &m.b);
    let interdit = || Statut(StatusCode::FORBIDDEN);
    let absent = || Statut(StatusCode::NOT_FOUND);
    let conflit = || Statut(StatusCode::CONFLICT);
    let CasUsage::Echeancier(cas) = route.cas_usage else {
        return None;
    };
    // Une condition ou une écriture d'un autre dossier dans le chemin : absente, même pour qui
    // peut agir.
    let croises = |mut cas: Vec<Attendu>| {
        cas.extend([
            attendu_croise(a.administrateur, a.cabinet, a.dossier, b.dossier, absent()),
            attendu_croise(m.valideur, a.cabinet, a.dossier, a.autre_dossier, absent()),
            attendu_croise(m.multi, b.cabinet, b.dossier, a.dossier, absent()),
        ]);
        cas
    };
    let condition = |d: IdDossier| Ressource(sous_ressource(d, "condition"));
    let ecriture = |d: IdDossier| Ressource(sous_ressource(d, "ecriture"));
    Some(match cas {
        Cas::ListerConditions => vec![
            attendu(a.administrateur, a.cabinet, a.dossier, liste(m, a.dossier)),
            attendu(
                a.administrateur,
                a.cabinet,
                a.autre_dossier,
                liste(m, a.autre_dossier),
            ),
            attendu(a.collaborateur, a.cabinet, a.dossier, liste(m, a.dossier)),
            attendu(m.auditeur, a.cabinet, a.dossier, liste(m, a.dossier)),
            attendu(a.collaborateur, a.cabinet, a.autre_dossier, absent()),
            attendu(m.multi, b.cabinet, b.dossier, liste(m, b.dossier)),
            attendu(m.multi, b.cabinet, b.autre_dossier, absent()),
        ],
        Cas::LireCondition => croises(vec![
            attendu(a.administrateur, a.cabinet, a.dossier, condition(a.dossier)),
            attendu(a.collaborateur, a.cabinet, a.dossier, condition(a.dossier)),
            attendu(m.auditeur, a.cabinet, a.dossier, condition(a.dossier)),
            attendu(a.collaborateur, a.cabinet, a.autre_dossier, absent()),
            attendu(m.multi, b.cabinet, b.dossier, condition(b.dossier)),
        ]),
        Cas::CreerCondition => vec![
            attendu(a.administrateur, a.cabinet, a.dossier, Cree),
            // Même code dans le même dossier : unicité (409).
            attendu(m.valideur, a.cabinet, a.dossier, conflit()),
            attendu(m.auditeur, a.cabinet, a.dossier, interdit()),
            attendu(a.collaborateur, a.cabinet, a.dossier, interdit()),
            attendu(a.collaborateur, a.cabinet, a.autre_dossier, absent()),
            attendu(m.multi, a.cabinet, a.autre_dossier, Cree),
            // Collaborateur dans B : le paramétrage lui est refusé.
            attendu(m.multi, b.cabinet, b.dossier, interdit()),
            attendu(m.multi, b.cabinet, b.autre_dossier, absent()),
        ],
        // Le membre des deux cabinets n'est que collaborateur dans B (403 avant toute lecture) :
        // le cas croisé du paramétrage passe par A, où il est administrateur.
        Cas::ModifierCondition => vec![
            attendu_croise(a.administrateur, a.cabinet, a.dossier, b.dossier, absent()),
            attendu_croise(m.valideur, a.cabinet, a.dossier, a.autre_dossier, absent()),
            attendu_croise(m.multi, a.cabinet, a.autre_dossier, a.dossier, absent()),
            attendu(m.auditeur, a.cabinet, a.dossier, interdit()),
            attendu(a.collaborateur, a.cabinet, a.dossier, interdit()),
            attendu(a.administrateur, a.cabinet, a.dossier, condition(a.dossier)),
            attendu(m.valideur, a.cabinet, a.dossier, conflit()),
            attendu(a.collaborateur, a.cabinet, a.autre_dossier, absent()),
            attendu(
                m.multi,
                a.cabinet,
                a.autre_dossier,
                condition(a.autre_dossier),
            ),
            attendu(m.multi, b.cabinet, b.dossier, interdit()),
        ],
        // Sans paramètre : 400 une fois la condition du dossier trouvée.
        Cas::CalculerEcheances => croises(vec![
            attendu(m.auditeur, a.cabinet, a.dossier, interdit()),
            attendu(
                a.collaborateur,
                a.cabinet,
                a.dossier,
                Statut(StatusCode::BAD_REQUEST),
            ),
            attendu(
                a.administrateur,
                a.cabinet,
                a.autre_dossier,
                Statut(StatusCode::BAD_REQUEST),
            ),
            attendu(a.collaborateur, a.cabinet, a.autre_dossier, absent()),
            attendu(
                m.multi,
                b.cabinet,
                b.dossier,
                Statut(StatusCode::BAD_REQUEST),
            ),
        ]),
        Cas::LireEcheances => croises(vec![
            attendu(a.administrateur, a.cabinet, a.dossier, ecriture(a.dossier)),
            attendu(a.collaborateur, a.cabinet, a.dossier, ecriture(a.dossier)),
            attendu(m.auditeur, a.cabinet, a.dossier, ecriture(a.dossier)),
            attendu(a.collaborateur, a.cabinet, a.autre_dossier, absent()),
            attendu(m.multi, b.cabinet, b.dossier, ecriture(b.dossier)),
        ]),
        // L'écriture fixe n'a pas de ligne de tiers : 422 pour qui peut réviser.
        Cas::ReviserEcheance => croises(vec![
            attendu(m.auditeur, a.cabinet, a.dossier, interdit()),
            attendu(
                a.collaborateur,
                a.cabinet,
                a.dossier,
                Statut(StatusCode::UNPROCESSABLE_ENTITY),
            ),
            attendu(
                m.valideur,
                a.cabinet,
                a.dossier,
                Statut(StatusCode::UNPROCESSABLE_ENTITY),
            ),
            attendu(a.collaborateur, a.cabinet, a.autre_dossier, absent()),
            attendu(
                m.multi,
                b.cabinet,
                b.dossier,
                Statut(StatusCode::UNPROCESSABLE_ENTITY),
            ),
        ]),
    })
}
