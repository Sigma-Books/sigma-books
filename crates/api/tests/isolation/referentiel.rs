//! Cas d'isolation des routes du module `referentiel` (PR 11a et 11b de `docs/lots/L2.md`) :
//! plan, journaux, exercices d'un dossier (les tiers : `tiers.rs`) ; transitions de ses
//! périodes et exercice suivant.
//!
//! Les routes des périodes s'enchaînent dans l'ordre de la table, sur la première période de
//! chaque dossier : verrouillée (administrateur sur le second dossier de A, valideur sur le
//! premier), déverrouillée (administrateur seul : le second dossier), puis clôturée (les deux).
//!
//! Chaque dossier du monde a son référentiel et ses sous-ressources fixes (`Monde::fixtures`) :
//! une lecture rend celles de son dossier et aucune d'un autre ; une mutation réussie porte sur
//! un dossier distinct à chaque fois (une création ou une modification ne se rejoue pas à
//! l'identique). Les cas génériques du harnais ajoutent, pour chaque route à sous-ressource,
//! la sous-ressource d'un autre dossier placée dans le chemin d'un dossier de A (404 identique).

use api::serveur::routes::referentiel::Cas;
use api::serveur::routes::{CasUsage, ROUTES, Route};
use axum::Router;
use axum::http::StatusCode;

use super::{
    Attendu, CasModule, Monde, Resultat, appel, attendu, attendu_croise, chemin, chemin_de,
    verifier_absent,
};
use crate::commun::Env;
use crate::commun::referentiel::Fixtures;
use crate::commun::serveur::{Reponse, corps_de, envoyer};

pub(super) const CAS: CasModule = CasModule {
    attentes,
    hors_cabinet: CasModule::AUCUN.hors_cabinet,
};

/// Liste qui contient la sous-ressource de `dossier` et aucune de celles des autres dossiers.
fn liste(m: &Monde, cle: &'static str, dossier: api::base::IdDossier) -> Resultat {
    let de = |f: &Fixtures| match cle {
        "comptes" => f.compte,
        "tiers" => f.tiers,
        "journaux" => f.journal,
        _ => f.exercice,
    };
    Resultat::ListeContient {
        cle,
        present: de(&m.fixtures[&dossier]),
        absents: m
            .fixtures
            .iter()
            .filter(|(d, _)| **d != dossier)
            .map(|(_, f)| de(f))
            .collect(),
    }
}

fn attentes(route: &Route, m: &Monde) -> Option<Vec<Attendu>> {
    use Resultat::{Cree, Ressource, Statut};
    let (a, b) = (&m.a, &m.b);
    let interdit = || Statut(StatusCode::FORBIDDEN);
    let absent = || Statut(StatusCode::NOT_FOUND);
    let CasUsage::Referentiel(cas) = route.cas_usage else {
        return None;
    };
    let lecture = |cle| {
        vec![
            attendu(
                a.administrateur,
                a.cabinet,
                a.autre_dossier,
                liste(m, cle, a.autre_dossier),
            ),
            attendu(
                a.collaborateur,
                a.cabinet,
                a.dossier,
                liste(m, cle, a.dossier),
            ),
            attendu(m.auditeur, a.cabinet, a.dossier, liste(m, cle, a.dossier)),
            attendu(a.collaborateur, a.cabinet, a.autre_dossier, absent()),
            attendu(m.multi, b.cabinet, b.dossier, liste(m, cle, b.dossier)),
            attendu(m.multi, b.cabinet, b.autre_dossier, absent()),
        ]
    };
    // Une mutation : réussie une fois par dossier de A (administrateur, valideur), refusée aux
    // autres rôles, absente sur un dossier non affecté.
    let mutation = |reussite: &dyn Fn(api::base::IdDossier) -> Resultat| {
        vec![
            attendu(a.collaborateur, a.cabinet, a.dossier, interdit()),
            attendu(m.auditeur, a.cabinet, a.dossier, interdit()),
            attendu(a.collaborateur, a.cabinet, a.autre_dossier, absent()),
            attendu(m.multi, b.cabinet, b.dossier, interdit()),
            attendu(
                a.administrateur,
                a.cabinet,
                a.autre_dossier,
                reussite(a.autre_dossier),
            ),
            attendu(m.valideur, a.cabinet, a.dossier, reussite(a.dossier)),
        ]
    };
    // Une sous-ressource d'un autre dossier dans le chemin : absente, même pour qui peut agir.
    let croises = |mut cas: Vec<Attendu>| {
        cas.extend([
            attendu_croise(a.administrateur, a.cabinet, a.dossier, b.dossier, absent()),
            attendu_croise(m.valideur, a.cabinet, a.dossier, a.autre_dossier, absent()),
            attendu_croise(m.multi, a.cabinet, a.autre_dossier, b.dossier, absent()),
        ]);
        cas
    };
    Some(match cas {
        Cas::ListerComptes => lecture("comptes"),
        Cas::ListerJournaux => lecture("journaux"),
        Cas::ListerExercices => lecture("exercices"),
        Cas::CreerCompte | Cas::CreerJournal => mutation(&|_| Cree),
        Cas::RenommerCompte | Cas::DesactiverCompte | Cas::ReactiverCompte => {
            croises(mutation(&|d| Ressource(m.fixtures[&d].compte)))
        }
        Cas::VerrouillerPeriode | Cas::CloturerPeriode => {
            croises(mutation(&|d| Ressource(m.fixtures[&d].periode)))
        }
        // Administrateur seul : le valideur est refusé, même sur son dossier affecté.
        Cas::DeverrouillerPeriode => vec![
            attendu(a.collaborateur, a.cabinet, a.dossier, interdit()),
            attendu(m.auditeur, a.cabinet, a.dossier, interdit()),
            attendu(m.valideur, a.cabinet, a.dossier, interdit()),
            attendu(a.collaborateur, a.cabinet, a.autre_dossier, absent()),
            attendu(m.multi, b.cabinet, b.dossier, interdit()),
            attendu(
                a.administrateur,
                a.cabinet,
                a.autre_dossier,
                Ressource(m.fixtures[&a.autre_dossier].periode),
            ),
            attendu_croise(a.administrateur, a.cabinet, a.dossier, b.dossier, absent()),
            attendu_croise(
                a.administrateur,
                a.cabinet,
                a.dossier,
                a.autre_dossier,
                absent(),
            ),
            attendu_croise(m.multi, a.cabinet, a.autre_dossier, b.dossier, absent()),
        ],
        Cas::OuvrirExerciceSuivant => mutation(&|_| Cree),
    })
}

async fn compter(env: &Env, table: &str, dossier: api::base::IdDossier) -> i64 {
    // Noms de tables constants, choisis par le test.
    let sql = match table {
        "compte" => "SELECT count(*) FROM compte WHERE dossier_id = $1 AND actif",
        "exercice" => "SELECT count(*) FROM exercice WHERE dossier_id = $1",
        "ouverte" => {
            "SELECT count(*) FROM periode WHERE dossier_id = $1 AND statut = 'ouverte' \
             AND statut_par IS NULL"
        }
        _ => "SELECT count(*) FROM audit_dossier WHERE dossier_id = $1",
    };
    sqlx::query_scalar(sql)
        .bind(dossier.uuid())
        .fetch_one(&env.admin)
        .await
        .unwrap()
}

/// Cedar forcé à « autorisé » : la requête tient seule. Le collaborateur et l'auditeur ne
/// paramètrent rien (rôle relu en base : 403) ; un dossier non affecté reste absent ; une
/// sous-ressource d'un autre dossier, sous le chemin d'un dossier de A, est absente et n'est
/// pas modifiée.
pub(super) async fn quand_cedar_se_trompe(
    env: &Env,
    routeur: &Router,
    m: &Monde,
    reference: &Reponse,
) {
    let (a, b) = (&m.a, &m.b);
    let route = |cas| {
        ROUTES
            .iter()
            .find(|r| r.cas_usage == CasUsage::Referentiel(cas))
            .unwrap()
    };
    let etat = || async {
        (
            compter(env, "compte", a.dossier).await,
            compter(env, "audit", a.dossier).await,
            compter(env, "compte", b.dossier).await,
        )
    };
    let avant = etat().await;
    for cas in [Cas::CreerCompte, Cas::DesactiverCompte, Cas::CreerJournal] {
        let r = route(cas);
        for utilisateur in [a.collaborateur, m.auditeur] {
            let reponse = appel(
                routeur,
                r,
                &chemin(r, a.cabinet.uuid(), a.dossier.uuid()),
                Some(utilisateur),
            )
            .await;
            assert_eq!(reponse.statut, StatusCode::FORBIDDEN, "{}", reponse.texte());
        }
        // Dossier non affecté au collaborateur : absent.
        let reponse = appel(
            routeur,
            r,
            &chemin(r, a.cabinet.uuid(), a.autre_dossier.uuid()),
            Some(a.collaborateur),
        )
        .await;
        verifier_absent(&reponse, reference, "dossier non affecté, Cedar forcé");
    }
    let lecture = route(Cas::ListerComptes);
    let reponse = appel(
        routeur,
        lecture,
        &chemin(lecture, a.cabinet.uuid(), a.autre_dossier.uuid()),
        Some(a.collaborateur),
    )
    .await;
    verifier_absent(
        &reponse,
        reference,
        "lecture d'un dossier non affecté, Cedar forcé",
    );
    // Compte de B sous un dossier de A : absent, et toujours actif.
    let r = route(Cas::DesactiverCompte);
    let reponse = appel(
        routeur,
        r,
        &chemin_de(r, a.cabinet.uuid(), a.dossier.uuid(), b.dossier.uuid()),
        Some(a.administrateur),
    )
    .await;
    verifier_absent(&reponse, reference, "compte de B sous A, Cedar forcé");
    // Dossier de B sous le cabinet A, avec ses propres sous-ressources : absent.
    let reponse = appel(
        routeur,
        r,
        &chemin(r, a.cabinet.uuid(), b.dossier.uuid()),
        Some(a.administrateur),
    )
    .await;
    verifier_absent(&reponse, reference, "dossier de B sous A, Cedar forcé");
    periodes_quand_cedar_se_trompe(env, routeur, m, reference).await;
    assert_eq!(etat().await, avant);
}

/// Cedar forcé, routes des périodes et de l'exercice suivant : le rôle relu en base refuse le
/// collaborateur et l'auditeur (et le valideur pour le déverrouillage) ; une session non fraîche
/// est refusée pour les actions ré-authentifiées ; la période d'un autre dossier est absente.
/// Aucune période ni aucun exercice ne change.
async fn periodes_quand_cedar_se_trompe(
    env: &Env,
    routeur: &Router,
    m: &Monde,
    reference: &Reponse,
) {
    let (a, b) = (&m.a, &m.b);
    let etat = || async {
        (
            compter(env, "ouverte", a.dossier).await,
            compter(env, "exercice", a.dossier).await,
            compter(env, "ouverte", b.dossier).await,
            compter(env, "exercice", b.dossier).await,
        )
    };
    let avant = etat().await;
    let cas_des_periodes = [
        Cas::VerrouillerPeriode,
        Cas::DeverrouillerPeriode,
        Cas::CloturerPeriode,
        Cas::OuvrirExerciceSuivant,
    ];
    for cas in cas_des_periodes {
        let r = ROUTES
            .iter()
            .find(|r| r.cas_usage == CasUsage::Referentiel(cas))
            .unwrap();
        let chemin_a = chemin(r, a.cabinet.uuid(), a.dossier.uuid());
        let mut refuses = vec![a.collaborateur, m.auditeur];
        if cas == Cas::DeverrouillerPeriode {
            refuses.push(m.valideur);
        }
        for utilisateur in refuses {
            let reponse = appel(routeur, r, &chemin_a, Some(utilisateur)).await;
            assert_eq!(reponse.statut, StatusCode::FORBIDDEN, "{}", reponse.texte());
        }
        // Session vérifiée il y a dix minutes : non fraîche.
        if cas != Cas::VerrouillerPeriode {
            let reponse = envoyer(
                routeur,
                &chemin_a,
                Some(a.administrateur),
                corps_de(r),
                Some(600),
            )
            .await;
            assert_eq!(reponse.statut, StatusCode::FORBIDDEN, "{}", reponse.texte());
        }
        if cas != Cas::OuvrirExerciceSuivant {
            // Période de B sous un dossier de A : absente.
            let reponse = appel(
                routeur,
                r,
                &chemin_de(r, a.cabinet.uuid(), a.dossier.uuid(), b.dossier.uuid()),
                Some(a.administrateur),
            )
            .await;
            verifier_absent(&reponse, reference, "période de B sous A, Cedar forcé");
        }
    }
    assert_eq!(etat().await, avant);
}
