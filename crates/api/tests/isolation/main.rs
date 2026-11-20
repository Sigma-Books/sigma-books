//! Harnais d'isolation entre tenants par l'API (L2-14 ; ADR 0011, « Tests », ligne « Chaque
//! route » ; F1.7, F1.8, F1.14, F2.7).
//!
//! Monde : deux cabinets A et B, deux dossiers chacun ; dans chacun un administrateur et un
//! collaborateur affecté au seul premier dossier ; un utilisateur `multi`, administrateur de A
//! et collaborateur de B affecté au premier dossier de B.
//!
//! - **Chaque route** de `routes::ROUTES` doit avoir ses attentes ([`attentes`]), lues dans le
//!   fichier de cas de son module (`tests/isolation/<module>.rs`, un par module de
//!   `routes::MODULES`) : sinon le test échoue. Pour chacune sont vérifiés en outre, de façon
//!   générique : une ressource de l'autre cabinet donne 404 avec le même corps qu'un
//!   identifiant inexistant ; un dossier non affecté donne 404 au collaborateur ; sans
//!   authentification, 401.
//! - **Cedar se trompe** : avec une décision forcée à « autorisé », le filtre de la requête et
//!   la RLS ne livrent toujours rien d'autrui ni d'un dossier non affecté.
//! - **Journaux** du scénario complet : liste blanche, aucun nom ni identifiant.
//!
//! Chaque route est appelée avec sa méthode : une mutation (`POST`) porte les en-têtes
//! anti-CSRF de la même origine et le corps de `corps_de` ; un cas « 200 » d'une mutation crée
//! réellement la ressource et l'inscrit au journal d'audit.
//!
//! Les routes **hors cabinet** (session courante, ses sessions, ses cabinets : ADR 0012) n'ont
//! pas de `{cabinet}` à substituer : leurs scénarios ([`cas_hors_cabinet`]) s'exécutent avec des
//! sessions réelles (cookie, jeton anti-CSRF lié) de deux utilisateurs de cabinets différents
//! et du membre des deux cabinets ; aucun ne voit ni n'agit sur ce qui est à un autre.
#![cfg(feature = "tests-postgres")]
// Fichier de test : les `expect`/`unwrap` y sont voulus.
#![allow(clippy::expect_used, clippy::unwrap_used)]

#[path = "../commun/mod.rs"]
mod commun;

// Un fichier de cas par module de routes (`api::serveur::routes::MODULES`).
mod administration;
mod audit;
mod brouillard;
mod editions;
mod facteurs;
mod integrite;
mod moi;
mod organisation;
mod passkeys;
mod pieces;
mod referentiel;
mod session;

use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use api::base::session::NatureSession;
use api::base::{IdCabinet, IdDossier, IdUtilisateur};
use api::serveur::routes::{self, CasUsage, MODULES, ROUTES, Route};
use api::serveur::session::SessionOuverte;
use api::serveur::{Application, Reglages, routeur};
use authz::Autorisation;
use axum::Router;
use axum::http::StatusCode;
use commun::referentiel::{Fixtures, doter};
use commun::serveur::{
    AuthentificationTest, HorlogeTest, ORIGINE, Reponse, ToujoursAutorise, appeler_route,
    appeler_session, application_base, application_sessions, capture_globale, cookie_de, envoyer,
    evenements_de, id_requete, lire, ouvrir, verifier_liste_blanche,
};
use commun::{
    CabinetTest, affecter, ajouter_affecte, ajouter_membre, creer_cabinet, creer_utilisateur, env,
};
use sqlx::PgPool;
use uuid::Uuid;

/// Réglages des tests : le parcours de toutes les routes dépasse le débit par adresse par
/// défaut (600 requêtes par minute), propre à la production, qui n'est pas ce que l'on teste ici.
fn reglages() -> Reglages {
    let mut r = Reglages::new(ORIGINE.to_owned());
    r.debit_adresse.capacite = 20_000;
    r.debit_utilisateur.capacite = 20_000;
    r
}

struct Monde {
    a: CabinetTest,
    b: CabinetTest,
    multi: IdUtilisateur,
    /// Valideur et auditeur de A, affectés au premier dossier de A.
    valideur: IdUtilisateur,
    auditeur: IdUtilisateur,
    /// Sous-ressources du référentiel de chaque dossier (PR 11).
    fixtures: BTreeMap<IdDossier, Fixtures>,
}

/// Sous-ressources réelles des dossiers de tous les mondes, par (dossier, nom) : `{compte}`,
/// `{tiers}` (PR 11) et `{piece}` (PR 12). Chaque monde a ses propres dossiers : les tests
/// parallèles ne se gênent pas.
///
/// Mécanisme unique des deux voies : un chemin concret prend les sous-ressources d'un dossier
/// « source », par défaut celui du chemin ; absentes, un UUID neuf, donc inexistant.
static SOUS_RESSOURCES: Mutex<BTreeMap<(Uuid, &'static str), Uuid>> = Mutex::new(BTreeMap::new());

/// Inscrit la sous-ressource `{nom}` du dossier `dossier`.
fn inscrire_sous_ressource(dossier: IdDossier, nom: &'static str, id: Uuid) {
    SOUS_RESSOURCES
        .lock()
        .unwrap()
        .insert((dossier.uuid(), nom), id);
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
    // Une pièce réelle (PR 12), puis le référentiel (sous-compte, tiers ; PR 11), dans chaque
    // dossier.
    let mut fixtures = BTreeMap::new();
    for c in [&a, &b] {
        for dossier in [c.dossier, c.autre_dossier] {
            let piece = commun::pieces::doter(admin, c.cabinet, dossier, c.administrateur).await;
            inscrire_sous_ressource(dossier, "piece", piece);
            let f = doter(admin, c.cabinet, dossier, c.administrateur).await;
            inscrire_sous_ressource(dossier, "compte", f.compte);
            inscrire_sous_ressource(dossier, "tiers", f.tiers);
            inscrire_sous_ressource(dossier, "periode", f.periode);
            // Un brouillon réel (PR 13a), sous-ressource `{brouillon}`.
            let b = commun::brouillard::doter(admin, c.cabinet, dossier, c.administrateur).await;
            inscrire_sous_ressource(dossier, "brouillon", b);
            fixtures.insert(dossier, f);
        }
    }
    Monde {
        a,
        b,
        multi,
        valideur,
        auditeur,
        fixtures,
    }
}

/// Chemin concret d'une route ; ses sous-ressources (`{compte}`, `{tiers}`, `{piece}`…) sont
/// celles du dossier du chemin.
fn chemin(route: &Route, cabinet: Uuid, dossier: Uuid) -> String {
    chemin_de(route, cabinet, dossier, dossier)
}

/// Chemin concret d'une route dont les sous-ressources sont celles du dossier `source` (un
/// UUID neuf, donc inexistant, si `source` n'en a pas).
fn chemin_de(route: &Route, cabinet: Uuid, dossier: Uuid, source: Uuid) -> String {
    let inscrites = SOUS_RESSOURCES.lock().unwrap().clone();
    commun::chemin_concret(route.chemin, cabinet, dossier, |nom| {
        inscrites
            .iter()
            .find_map(|((d, n), id)| (*d == source && *n == nom).then_some(*id))
            .unwrap_or_else(Uuid::new_v4)
    })
}

/// Vrai si le chemin désigne une sous-ressource du dossier (`{compte}`, `{piece}`…).
fn a_une_sous_ressource(route: &Route) -> bool {
    route
        .chemin
        .replace("{cabinet}", "")
        .replace("{dossier}", "")
        .contains('{')
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
    /// 200, objet dont la liste `cle` contient l'élément d'identifiant `present`, et aucun de
    /// `absents` (référentiel d'un autre dossier).
    ListeContient {
        cle: &'static str,
        present: Uuid,
        absents: Vec<Uuid>,
    },
    /// Refus (403) ou ressource d'autrui (404).
    Statut(StatusCode),
}

struct Attendu {
    utilisateur: IdUtilisateur,
    cabinet: IdCabinet,
    dossier: IdDossier,
    resultat: Resultat,
    /// Dossier dont les sous-ressources sont placées dans le chemin (par défaut, `dossier`).
    sous_ressources_de: Option<IdDossier>,
}

/// Attente d'un appel dont les sous-ressources du chemin sont celles du dossier `de` : celle
/// d'un autre dossier, placée sous `dossier`, est absente (404 identique).
fn attendu_croise(
    utilisateur: IdUtilisateur,
    cabinet: IdCabinet,
    dossier: IdDossier,
    de: IdDossier,
    resultat: Resultat,
) -> Attendu {
    Attendu {
        sous_ressources_de: Some(de),
        ..attendu(utilisateur, cabinet, dossier, resultat)
    }
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
        sous_ressources_de: None,
    }
}

/// Cas d'isolation d'un module de routes : un fichier par module (`tests/isolation/<module>.rs`),
/// pour que deux voies de travail n'ajoutent pas leurs cas au même endroit.
struct CasModule {
    /// Attentes d'une route d'un cabinet. `None` : route sans cas, le test échoue.
    attentes: fn(&Route, &Monde) -> Option<Vec<Attendu>>,
    /// Scénario d'une route hors cabinet. `None` : route sans cas, le test échoue.
    hors_cabinet: fn(&Route) -> Option<Scenario>,
}

impl CasModule {
    /// Aucun cas : module sans route.
    const AUCUN: Self = Self {
        attentes: |_, _| None,
        hors_cabinet: |_| None,
    };
}

/// Scénario d'isolation d'une route hors cabinet.
type Scenario = for<'c> fn(&'c Ctx<'c>) -> Pin<Box<dyn Future<Output = ()> + 'c>>;

/// Fichier de cas du module de routes `nom` ([`api::serveur::routes::Module::nom`]).
fn cas_du_module(nom: &str) -> Option<&'static CasModule> {
    Some(match nom {
        "organisation" => &organisation::CAS,
        "audit" => &audit::CAS,
        "session" => &session::CAS,
        "moi" => &moi::CAS,
        "facteurs" => &facteurs::CAS,
        "passkeys" => &passkeys::CAS,
        "administration" => &administration::CAS,
        "referentiel" => &referentiel::CAS,
        "brouillard" => &brouillard::CAS,
        "editions" => &editions::CAS,
        "pieces" => &pieces::CAS,
        "integrite" => &integrite::CAS,
        _ => return None,
    })
}

/// Cas d'isolation d'une route d'un cabinet, lus dans le fichier de son module.
fn attentes(route: &Route, m: &Monde) -> Option<Vec<Attendu>> {
    cas_du_module(route.cas_usage.module()).and_then(|cas| (cas.attentes)(route, m))
}

/// Cas d'isolation d'une route hors cabinet, lu dans le fichier de son module.
fn cas_hors_cabinet(route: &Route) -> Option<Scenario> {
    cas_du_module(route.cas_usage.module()).and_then(|cas| (cas.hors_cabinet)(route))
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
    let source = attendu.sous_ressources_de.unwrap_or(attendu.dossier);
    let chemin = chemin_de(
        route,
        attendu.cabinet.uuid(),
        attendu.dossier.uuid(),
        source.uuid(),
    );
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
        Resultat::ListeContient {
            cle,
            present,
            absents,
        } => verifier_liste(&reponse, cle, *present, absents, &contexte),
        Resultat::Statut(statut) => {
            assert_eq!(reponse.statut, *statut, "{contexte} : {}", reponse.texte());
            if *statut == StatusCode::NOT_FOUND {
                assert_eq!(reponse.corps, reference.corps, "{contexte}");
            }
        }
    }
}

/// 200, et la liste `cle` contient `present` et aucun de `absents`.
fn verifier_liste(reponse: &Reponse, cle: &str, present: Uuid, absents: &[Uuid], contexte: &str) {
    assert_eq!(
        reponse.statut,
        StatusCode::OK,
        "{contexte} : {}",
        reponse.texte()
    );
    let ids: Vec<String> = reponse.json()[cle]
        .as_array()
        .expect("liste")
        .iter()
        .map(|e| e["id"].as_str().unwrap().to_owned())
        .collect();
    assert!(ids.contains(&present.to_string()), "{contexte}");
    for absent in absents {
        assert!(!ids.contains(&absent.to_string()), "{contexte}");
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
    if a_une_sous_ressource(route) {
        // Sous-ressource d'un autre dossier (de B, ou non affecté de A), ou inexistante, placée
        // dans le chemin d'un dossier de A visible : 404 identique, rien n'est modifié.
        for source in [
            b.dossier.uuid(),
            b.autre_dossier.uuid(),
            a.autre_dossier.uuid(),
            Uuid::new_v4(),
        ] {
            let reponse = appel(
                routeur,
                route,
                &chemin_de(route, a.cabinet.uuid(), a.dossier.uuid(), source),
                Some(a.administrateur),
            )
            .await;
            verifier_absent(
                &reponse,
                &reference,
                &format!("{} : sous-ressource d'autrui", route.chemin),
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

/// Chaque module de routes, même sans route, a son fichier de cas : une voie n'a pas à modifier
/// [`cas_du_module`] pour y ajouter les siens.
#[test]
fn chaque_module_a_son_fichier_de_cas() {
    let sans_fichier: Vec<&str> = MODULES
        .iter()
        .map(|m| m.nom)
        .filter(|nom| cas_du_module(nom).is_none())
        .collect();
    assert!(
        sans_fichier.is_empty(),
        "modules sans fichier de cas : {sans_fichier:?}"
    );
}

#[tokio::test]
async fn chaque_route_a_son_cas_d_isolation_et_le_respecte() {
    let env = env().await;
    let m = monde(&env.admin).await;
    let routeur = routeur(&application_base(&env, reglages()));

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
        "routes sans cas d'isolation (ajoutez-les au fichier de leur module, \
         `tests/isolation/<module>.rs`) : \
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
    cas: Scenario,
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
    cas(&c).await;
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
        reglages(),
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
    referentiel::quand_cedar_se_trompe(&env, &routeur, &m, &reference).await;
    brouillard::quand_cedar_se_trompe(&routeur, &m, &reference).await;
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
        .find(|r| r.cas_usage == CasUsage::Organisation(routes::organisation::Cas::CreerDossier))
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
                CasUsage::Audit(
                    routes::audit::Cas::ConsulterAudit
                        | routes::audit::Cas::VerifierAudit
                        | routes::audit::Cas::ExporterAudit
                )
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
    let routeur = routeur(&application_base(&env, reglages()));
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
    let routeur = routeur(&application_base(&env, reglages()));
    let mut reponses = Vec::new();
    for route in ROUTES.iter().filter(|r| r.portee.du_cabinet()) {
        reponses.extend(verifier_generique(&routeur, route, &m).await);
        for attendu in attentes(route, &m).unwrap() {
            let source = attendu.sous_ressources_de.unwrap_or(attendu.dossier);
            let chemin = chemin_de(
                route,
                attendu.cabinet.uuid(),
                attendu.dossier.uuid(),
                source.uuid(),
            );
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
