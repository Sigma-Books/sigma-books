//! Harnais d'isolation entre tenants par l'API (L2-14 ; ADR 0011, « Tests », ligne « Chaque
//! route » ; F1.7, F1.8, F1.14, F2.7).
//!
//! Monde : deux cabinets A et B, deux dossiers chacun ; dans chacun un administrateur et un
//! collaborateur affecté au seul premier dossier ; un utilisateur `multi`, administrateur de A
//! et collaborateur de B affecté au premier dossier de B.
//!
//! - **Chaque route** de `routes::ROUTES` doit avoir ses attentes dans [`attentes`] : sinon le
//!   test échoue. Pour chacune sont vérifiés en outre, de façon générique : une ressource de
//!   l'autre cabinet donne 404 avec le même corps qu'un identifiant inexistant ; un dossier non
//!   affecté donne 404 au collaborateur ; sans authentification, 401.
//! - **Cedar se trompe** : avec une décision forcée à « autorisé », le filtre de la requête et
//!   la RLS ne livrent toujours rien d'autrui ni d'un dossier non affecté.
//! - **Journaux** du scénario complet : liste blanche, aucun nom ni identifiant.
#![cfg(feature = "tests-postgres")]
// Fichier de test : les `expect`/`unwrap` y sont voulus.
#![allow(clippy::expect_used, clippy::unwrap_used)]

mod commun;

use std::sync::Arc;

use api::base::{IdCabinet, IdDossier, IdUtilisateur};
use api::serveur::routes::{ROUTES, Route};
use api::serveur::{Application, Reglages, routeur};
use axum::Router;
use axum::http::StatusCode;
use commun::serveur::{
    AuthentificationTest, ORIGINE, Reponse, ToujoursAutorise, application, capture_globale,
    evenements_de, id_requete, lire, verifier_liste_blanche,
};
use commun::{CabinetTest, affecter, ajouter_membre, creer_cabinet, creer_utilisateur, env};
use sqlx::PgPool;
use uuid::Uuid;

struct Monde {
    a: CabinetTest,
    b: CabinetTest,
    multi: IdUtilisateur,
}

async fn monde(admin: &PgPool) -> Monde {
    let a = creer_cabinet(admin, "Cabinet A").await;
    let b = creer_cabinet(admin, "Cabinet B").await;
    let multi = IdUtilisateur::nouveau();
    let mut tx = admin.begin().await.unwrap();
    creer_utilisateur(&mut tx, multi, "Membre des deux cabinets").await;
    ajouter_membre(&mut tx, a.cabinet, multi, "administrateur").await;
    ajouter_membre(&mut tx, b.cabinet, multi, "collaborateur").await;
    affecter(&mut tx, b.cabinet, b.dossier, multi).await;
    tx.commit().await.unwrap();
    Monde { a, b, multi }
}

/// Chemin concret d'une route.
fn chemin(route: &Route, cabinet: Uuid, dossier: Uuid) -> String {
    route
        .chemin
        .replace("{cabinet}", &cabinet.to_string())
        .replace("{dossier}", &dossier.to_string())
}

fn a_un_dossier(route: &Route) -> bool {
    route.chemin.contains("{dossier}")
}

/// Résultat attendu d'un appel dans le cabinet du chemin.
enum Resultat {
    /// 200, ressource d'identifiant donné.
    Ressource(Uuid),
    /// 200, liste exacte de ces dossiers.
    Liste(Vec<IdDossier>),
    /// Refus (403) ou ressource d'autrui (404).
    Statut(StatusCode),
}

struct Attendu {
    utilisateur: IdUtilisateur,
    cabinet: IdCabinet,
    dossier: IdDossier,
    resultat: Resultat,
}

fn attendu(
    utilisateur: IdUtilisateur,
    cabinet: IdCabinet,
    dossier: IdDossier,
    resultat: Resultat,
) -> Attendu {
    Attendu {
        utilisateur,
        cabinet,
        dossier,
        resultat,
    }
}

/// Cas d'isolation propres à chaque route, dont le membre des deux cabinets, qui n'agit que
/// selon son rôle dans le cabinet du chemin. `None` : route sans cas, le test échoue.
fn attentes(route: &Route, m: &Monde) -> Option<Vec<Attendu>> {
    use Resultat::{Liste, Ressource, Statut};
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
        "/api/cabinets/{cabinet}/dossiers" => vec![
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
        _ => return None,
    })
}

fn ids(reponse: &Reponse) -> Vec<String> {
    let mut ids: Vec<String> = reponse
        .json()
        .as_array()
        .expect("liste")
        .iter()
        .map(|d| d["id"].as_str().unwrap().to_owned())
        .collect();
    ids.sort();
    ids
}

async fn verifier_attendu(routeur: &Router, route: &Route, attendu: &Attendu, reference: &Reponse) {
    let chemin = chemin(route, attendu.cabinet.uuid(), attendu.dossier.uuid());
    let reponse = lire(routeur, &chemin, Some(attendu.utilisateur)).await;
    let contexte = format!("{} {chemin}", route.chemin);
    match &attendu.resultat {
        Resultat::Ressource(id) => {
            assert_eq!(
                reponse.statut,
                StatusCode::OK,
                "{contexte} : {}",
                reponse.texte()
            );
            assert_eq!(reponse.json()["id"], id.to_string(), "{contexte}");
        }
        Resultat::Liste(dossiers) => {
            assert_eq!(reponse.statut, StatusCode::OK, "{contexte}");
            let mut attendus: Vec<String> = dossiers.iter().map(ToString::to_string).collect();
            attendus.sort();
            assert_eq!(ids(&reponse), attendus, "{contexte}");
        }
        Resultat::Statut(statut) => {
            assert_eq!(reponse.statut, *statut, "{contexte} : {}", reponse.texte());
            if *statut == StatusCode::NOT_FOUND {
                assert_eq!(reponse.corps, reference.corps, "{contexte}");
            }
        }
    }
}

/// Le même 404, octet pour octet, que pour une ressource inexistante.
fn verifier_absent(reponse: &Reponse, reference: &Reponse, contexte: &str) {
    assert_eq!(
        reponse.statut,
        StatusCode::NOT_FOUND,
        "{contexte} : {}",
        reponse.texte()
    );
    assert_eq!(reponse.corps, reference.corps, "{contexte}");
    assert_eq!(
        reponse.entetes.get("content-type"),
        reference.entetes.get("content-type"),
        "{contexte}"
    );
}

/// Contrôles d'isolation communs à toute route.
async fn verifier_generique(routeur: &Router, route: &Route, m: &Monde) -> Vec<Reponse> {
    let (a, b) = (&m.a, &m.b);
    let mut reponses = Vec::new();
    // Référence : cabinet inexistant (et dossier inexistant).
    let reference = lire(
        routeur,
        &chemin(route, Uuid::new_v4(), Uuid::new_v4()),
        Some(a.administrateur),
    )
    .await;
    assert_eq!(reference.statut, StatusCode::NOT_FOUND, "{}", route.chemin);

    // Ressource de l'autre cabinet : 404 identique, quel que soit le rôle dans le sien.
    for utilisateur in [a.administrateur, a.collaborateur] {
        for dossier in [b.dossier, b.autre_dossier] {
            let reponse = lire(
                routeur,
                &chemin(route, b.cabinet.uuid(), dossier.uuid()),
                Some(utilisateur),
            )
            .await;
            verifier_absent(
                &reponse,
                &reference,
                &format!("{} : cabinet B", route.chemin),
            );
            reponses.push(reponse);
        }
    }
    if a_un_dossier(route) {
        // Dossier de B, ou inexistant, cité sous le cabinet A ; dossier non affecté.
        for (utilisateur, dossier) in [
            (a.administrateur, b.dossier.uuid()),
            (a.administrateur, b.autre_dossier.uuid()),
            (a.administrateur, Uuid::new_v4()),
            (a.collaborateur, b.dossier.uuid()),
            (a.collaborateur, a.autre_dossier.uuid()),
            (m.multi, b.autre_dossier.uuid()),
        ] {
            let chemin_a = if utilisateur == m.multi {
                chemin(route, b.cabinet.uuid(), dossier)
            } else {
                chemin(route, a.cabinet.uuid(), dossier)
            };
            let reponse = lire(routeur, &chemin_a, Some(utilisateur)).await;
            verifier_absent(
                &reponse,
                &reference,
                &format!("{} : dossier d'autrui", route.chemin),
            );
            reponses.push(reponse);
        }
    }
    // Sans authentification.
    let reponse = lire(
        routeur,
        &chemin(route, a.cabinet.uuid(), a.dossier.uuid()),
        None,
    )
    .await;
    assert_eq!(reponse.statut, StatusCode::UNAUTHORIZED, "{}", route.chemin);
    reponses.push(reponse);
    reponses.push(reference);
    reponses
}

#[tokio::test]
async fn chaque_route_a_son_cas_d_isolation_et_le_respecte() {
    let env = env().await;
    let m = monde(&env.admin).await;
    let routeur = routeur(&application(
        env.application.clone(),
        Reglages::new(ORIGINE.to_owned()),
    ));

    let sans_cas: Vec<&str> = ROUTES
        .iter()
        .filter(|r| attentes(r, &m).is_none())
        .map(|r| r.chemin)
        .collect();
    assert!(
        sans_cas.is_empty(),
        "routes sans cas d'isolation (ajoutez-les à `attentes`) : {sans_cas:?}"
    );

    for route in ROUTES {
        let reponses = verifier_generique(&routeur, route, &m).await;
        let reference = reponses.last().unwrap();
        for attendu in attentes(route, &m).unwrap() {
            verifier_attendu(&routeur, route, &attendu, reference).await;
        }
    }
}

#[tokio::test]
async fn la_requete_et_la_rls_tiennent_quand_cedar_se_trompe() {
    let env = env().await;
    let m = monde(&env.admin).await;
    let (a, b) = (&m.a, &m.b);
    // Décision Cedar forcée à « autorisé » pour toute action.
    let routeur = routeur(&Application::new(
        env.application.clone(),
        Arc::new(ToujoursAutorise),
        Arc::new(AuthentificationTest::default()),
        Reglages::new(ORIGINE.to_owned()),
    ));
    let reference = lire(
        &routeur,
        &format!("/api/cabinets/{}/dossiers/{}", a.cabinet, Uuid::new_v4()),
        Some(a.administrateur),
    )
    .await;

    // Autre cabinet : rien, quelle que soit la route.
    for route in ROUTES {
        let reponse = lire(
            &routeur,
            &chemin(route, b.cabinet.uuid(), b.dossier.uuid()),
            Some(a.collaborateur),
        )
        .await;
        verifier_absent(&reponse, &reference, route.chemin);
    }
    // Dossier de B sous le cabinet A : la lecture du dossier, au contexte de A, ne le trouve pas.
    let reponse = lire(
        &routeur,
        &format!("/api/cabinets/{}/dossiers/{}", a.cabinet, b.dossier),
        Some(a.administrateur),
    )
    .await;
    verifier_absent(&reponse, &reference, "dossier de B sous A");
    // Dossier non affecté : le filtre de la requête (affectation) le refuse seul.
    let reponse = lire(
        &routeur,
        &format!("/api/cabinets/{}/dossiers/{}", a.cabinet, a.autre_dossier),
        Some(a.collaborateur),
    )
    .await;
    verifier_absent(&reponse, &reference, "dossier non affecté");
    let reponse = lire(
        &routeur,
        &format!("/api/cabinets/{}/dossiers", a.cabinet),
        Some(a.collaborateur),
    )
    .await;
    assert_eq!(ids(&reponse), vec![a.dossier.to_string()]);
    // Le membre des deux cabinets, dans B, ne voit que son dossier affecté.
    let reponse = lire(
        &routeur,
        &format!("/api/cabinets/{}/dossiers", b.cabinet),
        Some(m.multi),
    )
    .await;
    assert_eq!(ids(&reponse), vec![b.dossier.to_string()]);
}

#[tokio::test]
async fn les_journaux_d_un_scenario_complet_restent_en_liste_blanche() {
    let capture = capture_globale();
    let env = env().await;
    let m = monde(&env.admin).await;
    let routeur = routeur(&application(
        env.application.clone(),
        Reglages::new(ORIGINE.to_owned()),
    ));
    let mut reponses = Vec::new();
    for route in ROUTES {
        reponses.extend(verifier_generique(&routeur, route, &m).await);
        for attendu in attentes(route, &m).unwrap() {
            let chemin = chemin(route, attendu.cabinet.uuid(), attendu.dossier.uuid());
            reponses.push(lire(&routeur, &chemin, Some(attendu.utilisateur)).await);
        }
    }
    assert!(reponses.iter().any(|r| r.statut == StatusCode::OK));
    let ids: Vec<String> = reponses.iter().map(id_requete).collect();
    let evenements = evenements_de(capture, &ids);
    assert_eq!(evenements.len(), ids.len(), "une ligne par requête");

    let mut interdits = vec![
        "Cabinet A".to_owned(),
        "Cabinet B".to_owned(),
        "dossier".to_owned(),
    ];
    for c in [&m.a, &m.b] {
        interdits.extend([
            c.cabinet.to_string(),
            c.dossier.to_string(),
            c.autre_dossier.to_string(),
            c.administrateur.to_string(),
            c.collaborateur.to_string(),
        ]);
    }
    interdits.push(m.multi.to_string());
    // « dossier » figure dans les modèles de route : on ne le cherche que hors du champ `route`.
    let hors_route: Vec<_> = evenements
        .iter()
        .map(|e| commun::serveur::Evenement {
            cible: e.cible.clone(),
            champs: e
                .champs
                .iter()
                .filter(|(c, _)| c != "route")
                .cloned()
                .collect(),
        })
        .collect();
    verifier_liste_blanche(&hors_route, &interdits);
    verifier_liste_blanche(&evenements, &interdits[3..]);
}
