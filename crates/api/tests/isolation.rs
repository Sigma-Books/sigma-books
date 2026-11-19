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
//!
//! Chaque route est appelée avec sa méthode : une mutation (`POST`) porte les en-têtes
//! anti-CSRF de la même origine et le corps de `corps_de` ; un cas « 200 » d'une mutation crée
//! réellement la ressource et l'inscrit au journal d'audit.
//!
//! Les routes **hors cabinet** (session courante, ses sessions, ses cabinets : ADR 0012) n'ont
//! pas de `{cabinet}` à substituer : leurs cas ([`cas_hors_cabinet`]) s'exécutent avec des
//! sessions réelles (cookie, jeton anti-CSRF lié) de deux utilisateurs de cabinets différents
//! et du membre des deux cabinets ; aucun ne voit ni n'agit sur ce qui est à un autre.
#![cfg(feature = "tests-postgres")]
// Fichier de test : les `expect`/`unwrap` y sont voulus.
#![allow(clippy::expect_used, clippy::unwrap_used)]

mod commun;

use std::sync::Arc;

use api::base::session::NatureSession;
use api::base::{IdCabinet, IdDossier, IdUtilisateur};
use api::serveur::routes::{CasUsage, Methode, ROUTES, Route};
use api::serveur::session::SessionOuverte;
use api::serveur::{Application, Reglages, routeur};
use authz::Autorisation;
use axum::Router;
use axum::http::StatusCode;
use commun::serveur::{
    AuthentificationTest, HorlogeTest, ORIGINE, Reponse, ToujoursAutorise, appeler_route,
    appeler_session, application_base, application_sessions, capture_globale, cookie_de,
    evenements_de, id_requete, lire, ouvrir, verifier_liste_blanche,
};
use commun::{
    CabinetTest, affecter, ajouter_affecte, ajouter_membre, creer_cabinet, creer_utilisateur, env,
};
use sqlx::PgPool;
use uuid::Uuid;

struct Monde {
    a: CabinetTest,
    b: CabinetTest,
    multi: IdUtilisateur,
    /// Valideur et auditeur de A, affectés au premier dossier de A.
    valideur: IdUtilisateur,
    auditeur: IdUtilisateur,
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
    let valideur = ajouter_affecte(admin, &a, "valideur").await;
    let auditeur = ajouter_affecte(admin, &a, "auditeur").await;
    Monde {
        a,
        b,
        multi,
        valideur,
        auditeur,
    }
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

/// Appelle une route avec sa méthode (et, pour une mutation, les en-têtes anti-CSRF).
async fn appel(
    routeur: &Router,
    route: &Route,
    chemin: &str,
    utilisateur: Option<IdUtilisateur>,
) -> Reponse {
    appeler_route(routeur, route, chemin, utilisateur).await
}

/// Résultat attendu d'un appel dans le cabinet du chemin.
enum Resultat {
    /// 200, ressource d'identifiant donné.
    Ressource(Uuid),
    /// 200, liste exacte de ces dossiers.
    Liste(Vec<IdDossier>),
    /// 201, ressource créée avec un nouvel identifiant.
    Cree,
    /// 200, page de consultation de l'audit.
    PageAudit,
    /// 200, vérification d'une chaîne d'audit intègre.
    Verification,
    /// 200, export d'audit.
    ExportAudit,
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
        // Audit du cabinet : administrateur seul (ADR 0016), 403 pour les autres membres.
        "/api/cabinets/{cabinet}/audit"
        | "/api/cabinets/{cabinet}/audit/verification"
        | "/api/cabinets/{cabinet}/audit/export" => {
            let ok = || resultat_audit(route);
            vec![
                attendu(a.administrateur, a.cabinet, a.dossier, ok()),
                attendu(m.valideur, a.cabinet, a.dossier, interdit()),
                attendu(m.auditeur, a.cabinet, a.dossier, interdit()),
                attendu(a.collaborateur, a.cabinet, a.dossier, interdit()),
                attendu(m.multi, a.cabinet, a.dossier, ok()),
                attendu(m.multi, b.cabinet, b.dossier, interdit()),
            ]
        }
        // Audit d'un dossier : administrateur, valideur, auditeur affectés ; collaborateur 403 ;
        // dossier non affecté 404.
        "/api/cabinets/{cabinet}/dossiers/{dossier}/audit"
        | "/api/cabinets/{cabinet}/dossiers/{dossier}/audit/verification"
        | "/api/cabinets/{cabinet}/dossiers/{dossier}/audit/export" => {
            let ok = || resultat_audit(route);
            vec![
                attendu(a.administrateur, a.cabinet, a.dossier, ok()),
                attendu(a.administrateur, a.cabinet, a.autre_dossier, ok()),
                attendu(m.valideur, a.cabinet, a.dossier, ok()),
                attendu(m.auditeur, a.cabinet, a.dossier, ok()),
                attendu(a.collaborateur, a.cabinet, a.dossier, interdit()),
                attendu(m.valideur, a.cabinet, a.autre_dossier, absent()),
                attendu(m.auditeur, a.cabinet, a.autre_dossier, absent()),
                attendu(m.multi, a.cabinet, a.autre_dossier, ok()),
                attendu(m.multi, b.cabinet, b.dossier, interdit()),
            ]
        }
        _ => return None,
    })
}

/// Résultat d'un appel autorisé d'une route d'audit.
fn resultat_audit(route: &Route) -> Resultat {
    match route.cas_usage {
        CasUsage::ConsulterAudit => Resultat::PageAudit,
        CasUsage::VerifierAudit => Resultat::Verification,
        _ => Resultat::ExportAudit,
    }
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
    let reponse = appel(routeur, route, &chemin, Some(attendu.utilisateur)).await;
    let contexte = format!("{} {} {chemin}", route.methode.nom(), route.chemin);
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
        Resultat::Cree => {
            assert_eq!(
                reponse.statut,
                StatusCode::CREATED,
                "{contexte} : {}",
                reponse.texte()
            );
            let id = reponse.json()["id"].as_str().unwrap().to_owned();
            assert!(Uuid::try_parse(&id).is_ok(), "{contexte}");
        }
        Resultat::PageAudit => {
            assert_eq!(
                reponse.statut,
                StatusCode::OK,
                "{contexte} : {}",
                reponse.texte()
            );
            assert!(reponse.json()["entrees"].is_array(), "{contexte}");
        }
        Resultat::Verification => {
            assert_eq!(
                reponse.statut,
                StatusCode::OK,
                "{contexte} : {}",
                reponse.texte()
            );
            assert_eq!(
                reponse.json()["integre"],
                true,
                "{contexte} : {}",
                reponse.texte()
            );
        }
        Resultat::ExportAudit => {
            assert_eq!(
                reponse.statut,
                StatusCode::OK,
                "{contexte} : {}",
                reponse.texte()
            );
            assert!(
                reponse.texte().starts_with("sigma-books export-audit v1\n"),
                "{contexte}"
            );
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
    let reference = appel(
        routeur,
        route,
        &chemin(route, Uuid::new_v4(), Uuid::new_v4()),
        Some(a.administrateur),
    )
    .await;
    assert_eq!(reference.statut, StatusCode::NOT_FOUND, "{}", route.chemin);

    // Ressource de l'autre cabinet : 404 identique, quel que soit le rôle dans le sien.
    for utilisateur in [a.administrateur, a.collaborateur] {
        for dossier in [b.dossier, b.autre_dossier] {
            let reponse = appel(
                routeur,
                route,
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
            let reponse = appel(routeur, route, &chemin_a, Some(utilisateur)).await;
            verifier_absent(
                &reponse,
                &reference,
                &format!("{} : dossier d'autrui", route.chemin),
            );
            reponses.push(reponse);
        }
    }
    // Sans authentification.
    let reponse = appel(
        routeur,
        route,
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
    let routeur = routeur(&application_base(&env, Reglages::new(ORIGINE.to_owned())));

    let sans_cas: Vec<&str> = ROUTES
        .iter()
        .filter(|r| {
            if r.portee.du_cabinet() {
                attentes(r, &m).is_none()
            } else {
                cas_hors_cabinet(r).is_none()
            }
        })
        .map(|r| r.chemin)
        .collect();
    assert!(
        sans_cas.is_empty(),
        "routes sans cas d'isolation (ajoutez-les à `attentes` ou à `cas_hors_cabinet`) : \
         {sans_cas:?}"
    );

    for route in ROUTES.iter().filter(|r| r.portee.du_cabinet()) {
        let reponses = verifier_generique(&routeur, route, &m).await;
        let reference = reponses.last().unwrap();
        for attendu in attentes(route, &m).unwrap() {
            verifier_attendu(&routeur, route, &attendu, reference).await;
        }
    }

    // Routes hors cabinet : sessions réelles, décision Cedar réelle.
    let horloge = HorlogeTest::default();
    let routeur = api::serveur::routeur(&application_sessions(
        &env,
        &horloge,
        Arc::new(Autorisation::charger().unwrap()),
    ));
    for route in ROUTES.iter().filter(|r| !r.portee.du_cabinet()) {
        let cas = cas_hors_cabinet(route).unwrap();
        verifier_hors_cabinet(&env, &horloge, &routeur, cas, &m).await;
    }
}

/// Cas d'isolation d'une route hors cabinet. `None` : route sans cas, le test échoue.
#[derive(Debug, Clone, Copy)]
enum HorsCabinet {
    EtatSession,
    Deconnexion,
    Rotation,
    ListeSessions,
    Revocation,
    Cabinets,
}

fn cas_hors_cabinet(route: &Route) -> Option<HorsCabinet> {
    Some(match (route.methode, route.chemin) {
        (Methode::Get, "/api/session") => HorsCabinet::EtatSession,
        (Methode::Post, "/api/session/deconnexion") => HorsCabinet::Deconnexion,
        (Methode::Post, "/api/session/rotation") => HorsCabinet::Rotation,
        (Methode::Get, "/api/moi/sessions") => HorsCabinet::ListeSessions,
        (Methode::Delete, "/api/moi/sessions/{session}") => HorsCabinet::Revocation,
        (Methode::Get, "/api/moi/cabinets") => HorsCabinet::Cabinets,
        _ => return None,
    })
}

/// `GET` au nom d'une session réelle.
async fn lire_s(routeur: &Router, chemin: &str, s: &SessionOuverte) -> Reponse {
    appeler_session(routeur, "GET", chemin, Some(&cookie_de(s)), None).await
}

/// Mutation au nom d'une session réelle, avec un jeton anti-CSRF donné.
async fn muter_s(
    routeur: &Router,
    methode: &str,
    chemin: &str,
    s: &SessionOuverte,
    csrf: &str,
) -> Reponse {
    appeler_session(routeur, methode, chemin, Some(&cookie_de(s)), Some(csrf)).await
}

/// L'identifiant de la session courante, vu par `GET /api/session` (`None` : 401).
async fn session_vue(routeur: &Router, s: &SessionOuverte) -> Option<String> {
    let r = lire_s(routeur, "/api/session", s).await;
    match r.statut {
        StatusCode::OK => Some(r.json()["id"].as_str().unwrap().to_owned()),
        StatusCode::UNAUTHORIZED => None,
        autre => panic!("statut inattendu {autre} : {}", r.texte()),
    }
}

/// Identifiants des éléments d'une liste JSON, triés.
fn ids_json(reponse: &Reponse) -> Vec<String> {
    assert_eq!(reponse.statut, StatusCode::OK, "{}", reponse.texte());
    ids(reponse)
}

/// Contexte d'un cas hors cabinet : sessions complètes de l'administrateur de A, de celui de B
/// et du membre des deux cabinets.
struct Ctx<'a> {
    env: &'a commun::Env,
    horloge: &'a HorlogeTest,
    routeur: &'a Router,
    m: &'a Monde,
    sa: SessionOuverte,
    sb: SessionOuverte,
    s_multi: SessionOuverte,
}

async fn verifier_hors_cabinet(
    env: &commun::Env,
    horloge: &HorlogeTest,
    routeur: &Router,
    cas: HorsCabinet,
    m: &Monde,
) {
    let complete = NatureSession::Complete;
    let c = Ctx {
        env,
        horloge,
        routeur,
        m,
        sa: ouvrir(env, horloge, m.a.administrateur, complete).await,
        sb: ouvrir(env, horloge, m.b.administrateur, complete).await,
        s_multi: ouvrir(env, horloge, m.multi, complete).await,
    };
    match cas {
        HorsCabinet::EtatSession => cas_etat_session(&c).await,
        HorsCabinet::Deconnexion => cas_deconnexion(&c).await,
        HorsCabinet::Rotation => cas_rotation(&c).await,
        HorsCabinet::ListeSessions => cas_liste_sessions(&c).await,
        HorsCabinet::Revocation => cas_revocation(&c).await,
        HorsCabinet::Cabinets => cas_cabinets(&c).await,
    }
}

async fn cas_etat_session(c: &Ctx<'_>) {
    let Ctx {
        routeur, sa, sb, ..
    } = c;
    assert_eq!(session_vue(routeur, sa).await, Some(sa.id.to_string()));
    assert_eq!(session_vue(routeur, sb).await, Some(sb.id.to_string()));
    let r = lire_s(routeur, "/api/session", sa).await;
    assert_eq!(r.json()["csrf"], sa.jeton.csrf());
    let r = appeler_session(routeur, "GET", "/api/session", None, None).await;
    assert_eq!(r.statut, StatusCode::UNAUTHORIZED);
}

async fn cas_deconnexion(c: &Ctx<'_>) {
    let Ctx {
        routeur,
        sa,
        sb,
        s_multi,
        ..
    } = c;
    let chemin = "/api/session/deconnexion";
    // Jeton anti-CSRF d'une autre session : 403, rien n'est révoqué.
    let r = muter_s(routeur, "POST", chemin, sa, &sb.jeton.csrf()).await;
    assert_eq!(r.statut, StatusCode::FORBIDDEN, "{}", r.texte());
    assert!(session_vue(routeur, sa).await.is_some());
    let r = muter_s(routeur, "POST", chemin, sa, &sa.jeton.csrf()).await;
    assert_eq!(r.statut, StatusCode::NO_CONTENT, "{}", r.texte());
    assert!(
        r.entetes["set-cookie"]
            .to_str()
            .unwrap()
            .starts_with("__Host-sigma_session=; Max-Age=0;")
    );
    assert_eq!(session_vue(routeur, sa).await, None);
    // Les sessions des autres restent ouvertes.
    assert!(session_vue(routeur, sb).await.is_some());
    assert!(session_vue(routeur, s_multi).await.is_some());
    let r = appeler_session(routeur, "POST", chemin, None, Some("x")).await;
    assert_eq!(r.statut, StatusCode::UNAUTHORIZED);
}

async fn cas_rotation(c: &Ctx<'_>) {
    let Ctx {
        routeur,
        sa,
        sb,
        s_multi,
        ..
    } = c;
    let chemin = "/api/session/rotation";
    let r = muter_s(routeur, "POST", chemin, sa, &sb.jeton.csrf()).await;
    assert_eq!(r.statut, StatusCode::FORBIDDEN, "{}", r.texte());
    let r = muter_s(routeur, "POST", chemin, sa, &sa.jeton.csrf()).await;
    assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
    let nouveau = r.entetes["set-cookie"]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_owned();
    // L'ancien jeton : 401 ; le nouveau désigne la même session.
    assert_eq!(session_vue(routeur, sa).await, None);
    let r = appeler_session(routeur, "GET", "/api/session", Some(&nouveau), None).await;
    assert_eq!(r.json()["id"], sa.id.to_string());
    // Les sessions des autres sont intactes.
    assert_eq!(session_vue(routeur, sb).await, Some(sb.id.to_string()));
    assert!(session_vue(routeur, s_multi).await.is_some());
}

async fn cas_liste_sessions(c: &Ctx<'_>) {
    let Ctx {
        env,
        horloge,
        routeur,
        m,
        sa,
        sb,
        s_multi,
        ..
    } = c;
    let a = &m.a;
    let complete = NatureSession::Complete;
    let sa2 = ouvrir(env, horloge, a.administrateur, complete).await;
    let r = lire_s(routeur, "/api/moi/sessions", sa).await;
    let vues = ids_json(&r);
    for s in [sa, &sa2] {
        assert!(vues.contains(&s.id.to_string()), "{}", r.texte());
    }
    for autre in [&sb, &s_multi] {
        assert!(!vues.contains(&autre.id.to_string()), "{}", r.texte());
    }
    // Ni jeton, ni empreinte, ni jeton anti-CSRF.
    for s in [sa, &sa2] {
        let cookie = cookie_de(s);
        let jeton = cookie.split_once('=').unwrap().1;
        assert!(!r.texte().contains(jeton));
        assert!(!r.texte().contains(&s.jeton.csrf()));
    }
    let cles: Vec<String> = r.json()[0].as_object().unwrap().keys().cloned().collect();
    for cle in &cles {
        assert!(
            [
                "id",
                "nature",
                "methode",
                "cree_le",
                "derniere_activite",
                "echeance",
                "appareil",
                "adresse",
                "courante"
            ]
            .contains(&cle.as_str()),
            "champ inattendu {cle}"
        );
    }
    // B (qui a d'autres sessions ouvertes par les cas précédents) : aucune de A.
    let vues_b = ids_json(&lire_s(routeur, "/api/moi/sessions", sb).await);
    assert!(vues_b.contains(&sb.id.to_string()));
    for autre in [sa, &sa2, s_multi] {
        assert!(!vues_b.contains(&autre.id.to_string()));
    }
}

async fn cas_revocation(c: &Ctx<'_>) {
    let Ctx {
        env,
        horloge,
        routeur,
        m,
        sa,
        sb,
        s_multi,
        ..
    } = c;
    let a = &m.a;
    let complete = NatureSession::Complete;
    let sa2 = ouvrir(env, horloge, a.administrateur, complete).await;
    let csrf = sa.jeton.csrf();
    let reference = muter_s(
        routeur,
        "DELETE",
        &format!("/api/moi/sessions/{}", Uuid::new_v4()),
        sa,
        &csrf,
    )
    .await;
    assert_eq!(reference.statut, StatusCode::NOT_FOUND);
    // La session de B, ou celle du membre des deux cabinets : absente pour A.
    for autre in [&sb, &s_multi] {
        let r = muter_s(
            routeur,
            "DELETE",
            &format!("/api/moi/sessions/{}", autre.id),
            sa,
            &csrf,
        )
        .await;
        verifier_absent(&r, &reference, "session d'autrui");
        assert!(session_vue(routeur, autre).await.is_some());
    }
    // Sa propre autre session : révoquée aussitôt ; une seconde fois, absente.
    let chemin = format!("/api/moi/sessions/{}", sa2.id);
    let r = muter_s(routeur, "DELETE", &chemin, sa, &csrf).await;
    assert_eq!(r.statut, StatusCode::NO_CONTENT, "{}", r.texte());
    assert_eq!(session_vue(routeur, &sa2).await, None);
    let r = muter_s(routeur, "DELETE", &chemin, sa, &csrf).await;
    verifier_absent(&r, &reference, "session déjà révoquée");
    assert!(session_vue(routeur, sa).await.is_some());
}

async fn cas_cabinets(c: &Ctx<'_>) {
    let Ctx {
        env,
        horloge,
        routeur,
        m,
        sa,
        sb,
        s_multi,
        ..
    } = c;
    let (a, b) = (&m.a, &m.b);
    let complete = NatureSession::Complete;
    let s_collab = ouvrir(env, horloge, a.collaborateur, complete).await;
    for (s, attendus) in [
        (sa, vec![a.cabinet]),
        (sb, vec![b.cabinet]),
        (&s_collab, vec![a.cabinet]),
        (s_multi, vec![a.cabinet, b.cabinet]),
    ] {
        let r = lire_s(routeur, "/api/moi/cabinets", s).await;
        let mut attendus: Vec<String> = attendus.iter().map(ToString::to_string).collect();
        attendus.sort();
        assert_eq!(ids_json(&r), attendus, "{}", r.texte());
    }
    let r = lire_s(routeur, "/api/moi/cabinets", s_multi).await;
    let json = r.json();
    let roles: Vec<&str> = json
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["role"].as_str().unwrap())
        .collect();
    assert_eq!(roles, vec!["administrateur", "collaborateur"]);
}

#[tokio::test]
async fn la_requete_et_la_rls_tiennent_quand_cedar_se_trompe() {
    let env = env().await;
    let m = monde(&env.admin).await;
    let (a, b) = (&m.a, &m.b);
    // Décision Cedar forcée à « autorisé » pour toute action.
    let routeur = routeur(&Application::new(
        env.application.clone(),
        env.lecture_audit.clone(),
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
    for route in ROUTES.iter().filter(|r| r.portee.du_cabinet()) {
        let reponse = appel(
            &routeur,
            route,
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

    creation_refusee_quand_cedar_se_trompe(&env, &routeur, &m).await;
    audit_refuse_quand_cedar_se_trompe(&routeur, &m, &reference).await;
    sessions_bornees_quand_cedar_se_trompe(&env, &m).await;
    cabinets_bornes_quand_cedar_se_trompe(&env, &m).await;
}

/// Cedar forcé : la RLS et le filtre de la requête bornent toujours les routes de session à
/// l'utilisateur. A ne révoque pas la session de B (404, rien n'est révoqué) et ne la voit pas.
async fn sessions_bornees_quand_cedar_se_trompe(env: &commun::Env, m: &Monde) {
    let (a, b) = (&m.a, &m.b);
    let horloge = HorlogeTest::default();
    let routeur = api::serveur::routeur(&application_sessions(
        env,
        &horloge,
        Arc::new(ToujoursAutorise),
    ));
    let sa = ouvrir(env, &horloge, a.administrateur, NatureSession::Complete).await;
    let sb = ouvrir(env, &horloge, b.administrateur, NatureSession::Complete).await;
    let csrf = sa.jeton.csrf();
    let reference = muter_s(
        &routeur,
        "DELETE",
        &format!("/api/moi/sessions/{}", Uuid::new_v4()),
        &sa,
        &csrf,
    )
    .await;
    let r = muter_s(
        &routeur,
        "DELETE",
        &format!("/api/moi/sessions/{}", sb.id),
        &sa,
        &csrf,
    )
    .await;
    verifier_absent(&r, &reference, "session de B, Cedar forcé");
    let revoquee: Option<time::OffsetDateTime> =
        sqlx::query_scalar("SELECT revoquee_le FROM session WHERE id = $1")
            .bind(sb.id.uuid())
            .fetch_one(&env.admin)
            .await
            .unwrap();
    assert_eq!(revoquee, None);
    assert_eq!(session_vue(&routeur, &sb).await, Some(sb.id.to_string()));
    let vues = ids_json(&lire_s(&routeur, "/api/moi/sessions", &sa).await);
    assert!(vues.contains(&sa.id.to_string()));
    assert!(!vues.contains(&sb.id.to_string()));
}

/// Cedar forcé : la liste des cabinets d'un utilisateur reste bornée à ses appartenances
/// actives (RLS sans cabinet en contexte, filtre de la requête).
async fn cabinets_bornes_quand_cedar_se_trompe(env: &commun::Env, m: &Monde) {
    let (a, b) = (&m.a, &m.b);
    let horloge = HorlogeTest::default();
    let routeur = routeur(&application_sessions(
        env,
        &horloge,
        Arc::new(ToujoursAutorise),
    ));
    let sa = ouvrir(env, &horloge, a.collaborateur, NatureSession::Complete).await;
    assert_eq!(
        ids_json(&lire_s(&routeur, "/api/moi/cabinets", &sa).await),
        vec![a.cabinet.to_string()]
    );
    // Une appartenance suspendue reste exclue par la requête.
    sqlx::query(
        "UPDATE appartenance SET actif = false WHERE cabinet_id = $1 AND utilisateur_id = $2",
    )
    .bind(b.cabinet.uuid())
    .bind(m.multi.uuid())
    .execute(&env.admin)
    .await
    .unwrap();
    let s_multi = ouvrir(env, &horloge, m.multi, NatureSession::Complete).await;
    assert_eq!(
        ids_json(&lire_s(&routeur, "/api/moi/cabinets", &s_multi).await),
        vec![a.cabinet.to_string()]
    );
}

/// Cedar forcé : la création d'un dossier reste refusée au collaborateur et à l'auditeur.
async fn creation_refusee_quand_cedar_se_trompe(env: &commun::Env, routeur: &Router, m: &Monde) {
    let a = &m.a;
    // Création d'un dossier : la requête exige le rôle d'administrateur ou de valideur lu en
    // base. Le collaborateur et l'auditeur, visibles dans leur cabinet, reçoivent 403 ; rien
    // n'est créé ni inscrit à l'audit.
    let creation = ROUTES
        .iter()
        .find(|r| r.cas_usage == CasUsage::CreerDossier)
        .unwrap();
    let compter = |sql: &'static str| {
        let admin = env.admin.clone();
        let cabinet = a.cabinet.uuid();
        async move {
            sqlx::query_scalar::<_, i64>(sql)
                .bind(cabinet)
                .fetch_one(&admin)
                .await
                .unwrap()
        }
    };
    let etat = || async {
        (
            compter("SELECT count(*) FROM dossier WHERE cabinet_id = $1").await,
            compter("SELECT count(*) FROM acces_dossier WHERE cabinet_id = $1").await,
            compter("SELECT count(*) FROM audit_cabinet WHERE cabinet_id = $1").await,
            compter("SELECT count(*) FROM audit_dossier WHERE cabinet_id = $1").await,
        )
    };
    let avant = etat().await;
    for utilisateur in [a.collaborateur, m.auditeur] {
        let reponse = appel(
            routeur,
            creation,
            &chemin(creation, a.cabinet.uuid(), a.dossier.uuid()),
            Some(utilisateur),
        )
        .await;
        assert_eq!(reponse.statut, StatusCode::FORBIDDEN, "{}", reponse.texte());
    }
    assert_eq!(etat().await, avant);
    // Le valideur, lui, crée le dossier.
    let reponse = appel(
        routeur,
        creation,
        &chemin(creation, a.cabinet.uuid(), a.dossier.uuid()),
        Some(m.valideur),
    )
    .await;
    assert_eq!(reponse.statut, StatusCode::CREATED, "{}", reponse.texte());
}

/// Cedar forcé : les routes d'audit restent fermées hors de la portée permise.
async fn audit_refuse_quand_cedar_se_trompe(routeur: &Router, m: &Monde, reference: &Reponse) {
    let a = &m.a;
    // Routes d'audit : la requête revérifie la portée. Le collaborateur, le valideur et
    // l'auditeur n'obtiennent ni l'audit du cabinet (administrateur lu en base, ADR 0016) ni
    // celui d'un dossier non affecté ; rien n'est inscrit pour un export refusé.
    let routes_audit: Vec<&Route> = ROUTES
        .iter()
        .filter(|r| {
            matches!(
                r.cas_usage,
                CasUsage::ConsulterAudit | CasUsage::VerifierAudit | CasUsage::ExporterAudit
            )
        })
        .collect();
    assert_eq!(routes_audit.len(), 6);
    for route in routes_audit {
        for utilisateur in [a.collaborateur, m.valideur, m.auditeur] {
            let reponse = appel(
                routeur,
                route,
                &chemin(route, a.cabinet.uuid(), a.autre_dossier.uuid()),
                Some(utilisateur),
            )
            .await;
            verifier_absent(
                &reponse,
                reference,
                &format!("{} {} : Cedar forcé", route.methode.nom(), route.chemin),
            );
        }
        // L'administrateur, lui, y accède.
        let reponse = appel(
            routeur,
            route,
            &chemin(route, a.cabinet.uuid(), a.autre_dossier.uuid()),
            Some(a.administrateur),
        )
        .await;
        assert_eq!(reponse.statut, StatusCode::OK, "{}", route.chemin);
    }
}

#[tokio::test]
async fn une_appartenance_suspendue_vaut_non_membre_sur_chaque_route() {
    let env = env().await;
    let m = monde(&env.admin).await;
    let (a, b) = (&m.a, &m.b);
    let routeur = routeur(&application_base(&env, Reglages::new(ORIGINE.to_owned())));
    // Suspension de l'administrateur de A et du membre des deux cabinets dans B seulement.
    for (cabinet, utilisateur) in [(a.cabinet, a.administrateur), (b.cabinet, m.multi)] {
        sqlx::query(
            "UPDATE appartenance SET actif = false WHERE cabinet_id = $1 AND utilisateur_id = $2",
        )
        .bind(cabinet.uuid())
        .bind(utilisateur.uuid())
        .execute(&env.admin)
        .await
        .unwrap();
    }
    for route in ROUTES.iter().filter(|r| r.portee.du_cabinet()) {
        let reference = appel(
            &routeur,
            route,
            &chemin(route, Uuid::new_v4(), Uuid::new_v4()),
            Some(a.collaborateur),
        )
        .await;
        for (utilisateur, cabinet, dossier) in [
            (a.administrateur, a.cabinet, a.dossier),
            (a.administrateur, a.cabinet, a.autre_dossier),
            (m.multi, b.cabinet, b.dossier),
        ] {
            let reponse = appel(
                &routeur,
                route,
                &chemin(route, cabinet.uuid(), dossier.uuid()),
                Some(utilisateur),
            )
            .await;
            verifier_absent(
                &reponse,
                &reference,
                &format!("{} : suspendu", route.chemin),
            );
        }
    }
    // Dans A, où il reste actif, le membre des deux cabinets garde ses droits d'administrateur.
    let reponse = lire(
        &routeur,
        &format!("/api/cabinets/{}", a.cabinet),
        Some(m.multi),
    )
    .await;
    assert_eq!(reponse.statut, StatusCode::OK);
    // Ses cabinets (décision Cedar réelle) : le cabinet de l'appartenance suspendue disparaît.
    let horloge = HorlogeTest::default();
    let routeur = api::serveur::routeur(&application_sessions(
        &env,
        &horloge,
        Arc::new(Autorisation::charger().unwrap()),
    ));
    for (utilisateur, attendus) in [
        (a.administrateur, vec![]),
        (m.multi, vec![a.cabinet.to_string()]),
        (b.administrateur, vec![b.cabinet.to_string()]),
    ] {
        let s = ouvrir(&env, &horloge, utilisateur, NatureSession::Complete).await;
        assert_eq!(
            ids_json(&lire_s(&routeur, "/api/moi/cabinets", &s).await),
            attendus
        );
    }
}

#[tokio::test]
async fn les_journaux_d_un_scenario_complet_restent_en_liste_blanche() {
    let capture = capture_globale();
    let env = env().await;
    let m = monde(&env.admin).await;
    let routeur = routeur(&application_base(&env, Reglages::new(ORIGINE.to_owned())));
    let mut reponses = Vec::new();
    for route in ROUTES.iter().filter(|r| r.portee.du_cabinet()) {
        reponses.extend(verifier_generique(&routeur, route, &m).await);
        for attendu in attentes(route, &m).unwrap() {
            let chemin = chemin(route, attendu.cabinet.uuid(), attendu.dossier.uuid());
            reponses.push(appel(&routeur, route, &chemin, Some(attendu.utilisateur)).await);
        }
    }
    assert!(reponses.iter().any(|r| r.statut == StatusCode::OK));
    assert!(reponses.iter().any(|r| r.statut == StatusCode::CREATED));
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
    interdits.extend([m.multi, m.valideur, m.auditeur].map(|u| u.to_string()));
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
