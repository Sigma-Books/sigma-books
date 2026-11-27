//! Cas d'isolation des routes du module `tiers` (PR 7a de `docs/lots/L3.md`, L3-15 ; ADR 0031).
//!
//! Chaque dossier du monde a son tiers fixe (`Monde::fixtures`). Une lecture rend celui de son
//! dossier et aucun d'un autre ; une mutation réussit **une fois par dossier** (premier et second
//! dossier de A, premier de B), dans l'ordre de la table : renommage, archivage, désarchivage,
//! coordonnée bancaire. Les rôles suivent `roles/tiers.cedar` : lecture des fiches par les quatre
//! rôles du cabinet, coordonnées bancaires **sans l'auditeur** (VP17), création par
//! l'administrateur et le valideur, tenue de la fiche par les rôles de saisie. Les cas génériques
//! du harnais ajoutent, pour chaque route, le tiers d'un autre dossier placé dans le chemin d'un
//! dossier de A (404 identique), l'autre cabinet, le dossier non affecté et l'absence de session.

use api::base::IdDossier;
use api::serveur::routes::tiers::Cas;
use api::serveur::routes::{CasUsage, Route};
use axum::http::StatusCode;

use super::{Attendu, CasModule, Monde, Resultat, attendu, attendu_croise};

pub(super) const CAS: CasModule = CasModule {
    attentes,
    hors_cabinet: CasModule::AUCUN.hors_cabinet,
};

/// Liste qui contient le tiers fixe de `dossier` et aucun de ceux des autres dossiers.
fn liste(m: &Monde, dossier: IdDossier) -> Resultat {
    Resultat::ListeContient {
        cle: "tiers",
        present: m.fixtures[&dossier].tiers,
        absents: m
            .fixtures
            .iter()
            .filter(|(d, _)| **d != dossier)
            .map(|(_, f)| f.tiers)
            .collect(),
    }
}

fn attentes(route: &Route, m: &Monde) -> Option<Vec<Attendu>> {
    use Resultat::{Cree, Ressource, Statut};
    let (a, b) = (&m.a, &m.b);
    let interdit = || Statut(StatusCode::FORBIDDEN);
    let absent = || Statut(StatusCode::NOT_FOUND);
    let CasUsage::Tiers(cas) = route.cas_usage else {
        return None;
    };
    let tiers = |d: IdDossier| Ressource(m.fixtures[&d].tiers);
    // Un tiers d'un autre dossier dans le chemin : absent, même pour qui peut agir.
    let croises = |mut cas: Vec<Attendu>| {
        cas.extend([
            attendu_croise(a.administrateur, a.cabinet, a.dossier, b.dossier, absent()),
            attendu_croise(m.valideur, a.cabinet, a.dossier, a.autre_dossier, absent()),
            attendu_croise(m.multi, b.cabinet, b.dossier, a.dossier, absent()),
        ]);
        cas
    };
    // Tenue de la fiche (`ModifierTiers`, `ArchiverTiers`) : rôles de saisie ; une réussite par
    // dossier ; l'auditeur refusé ; le dossier non affecté absent.
    let saisie = || {
        croises(vec![
            attendu(m.auditeur, a.cabinet, a.dossier, interdit()),
            attendu(a.collaborateur, a.cabinet, a.autre_dossier, absent()),
            attendu(m.valideur, a.cabinet, a.dossier, tiers(a.dossier)),
            attendu(
                a.administrateur,
                a.cabinet,
                a.autre_dossier,
                tiers(a.autre_dossier),
            ),
            attendu(m.multi, b.cabinet, b.dossier, tiers(b.dossier)),
        ])
    };
    Some(match cas {
        Cas::Lister => vec![
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
        Cas::Lire => croises(vec![
            attendu(a.administrateur, a.cabinet, a.dossier, tiers(a.dossier)),
            attendu(a.collaborateur, a.cabinet, a.dossier, tiers(a.dossier)),
            attendu(m.auditeur, a.cabinet, a.dossier, tiers(a.dossier)),
            attendu(a.collaborateur, a.cabinet, a.autre_dossier, absent()),
            attendu(m.multi, b.cabinet, b.dossier, tiers(b.dossier)),
        ]),
        // Coordonnées bancaires : l'auditeur est refusé (minimisation, VP17).
        Cas::LireComptesBancaires => croises(vec![
            attendu(m.auditeur, a.cabinet, a.dossier, interdit()),
            attendu(a.collaborateur, a.cabinet, a.dossier, tiers(a.dossier)),
            attendu(m.valideur, a.cabinet, a.dossier, tiers(a.dossier)),
            attendu(a.collaborateur, a.cabinet, a.autre_dossier, absent()),
            attendu(m.multi, b.cabinet, b.dossier, tiers(b.dossier)),
        ]),
        // Création : administrateur et valideur.
        Cas::Creer => vec![
            attendu(a.collaborateur, a.cabinet, a.dossier, interdit()),
            attendu(m.auditeur, a.cabinet, a.dossier, interdit()),
            attendu(a.collaborateur, a.cabinet, a.autre_dossier, absent()),
            attendu(m.multi, b.cabinet, b.dossier, interdit()),
            attendu(a.administrateur, a.cabinet, a.autre_dossier, Cree),
            attendu(m.valideur, a.cabinet, a.dossier, Cree),
        ],
        Cas::Modifier | Cas::Archiver | Cas::Desarchiver | Cas::ModifierComptesBancaires => {
            saisie()
        }
    })
}
