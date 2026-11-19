//! Serveur sans base de données : en-têtes, anti-CSRF, erreurs uniformes, limites, fichiers
//! statiques, couche HTTP (journaux : `journaux.rs`). Toutes ces requêtes sont refusées ou
//! servies avant toute lecture en base (le pool ne se connecte jamais).
// Fichier de test : les `expect`/`unwrap` y sont voulus.
#![allow(clippy::expect_used, clippy::unwrap_used)]

mod commun;

use std::fmt::Write as _;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use api::base::IdUtilisateur;
use api::serveur::csrf::{self, EN_TETE_CSRF};
use api::serveur::entetes::{CSP_API, CSP_INTERFACE, HSTS};
use api::serveur::journal::EN_TETE_REQUETE;
use api::serveur::routes::ROUTES;
use api::serveur::{Application, ReglageDebit, Reglages, routeur, servir};
use authz::Autorisation;
use axum::Router;
use axum::body::Body;
use axum::http::{HeaderMap, Method, Request, StatusCode, header};
use axum::middleware::from_fn_with_state;
use axum::routing::post;
use commun::serveur::{
    AuthentificationTest, EN_TETE_UTILISATEUR, ORIGINE, Reponse, appeler, appeler_route,
    application, application_du_binaire, lire, pool_sans_base,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use uuid::Uuid;

fn reglages() -> Reglages {
    Reglages::new(ORIGINE.to_owned())
}

fn routeur_test(reglages: Reglages) -> Router {
    routeur(&application(pool_sans_base(), reglages))
}

fn repertoire_web() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/donnees/web")
}

fn chemin_exemple() -> String {
    format!(
        "/api/cabinets/{}/dossiers/{}",
        Uuid::new_v4(),
        Uuid::new_v4()
    )
}

fn verifier_entetes_communs(entetes: &HeaderMap) {
    for (nom, valeur) in [
        ("strict-transport-security", HSTS),
        ("x-content-type-options", "nosniff"),
        ("referrer-policy", "no-referrer"),
        ("cross-origin-opener-policy", "same-origin"),
        ("cross-origin-resource-policy", "same-origin"),
        ("cross-origin-embedder-policy", "require-corp"),
    ] {
        assert_eq!(
            entetes.get(nom).map(|v| v.to_str().unwrap()),
            Some(valeur),
            "{nom}"
        );
    }
    let id = entetes
        .get(EN_TETE_REQUETE)
        .expect("identifiant de requête");
    assert_eq!(
        Uuid::try_parse(id.to_str().unwrap())
            .unwrap()
            .get_version_num(),
        4
    );
}

fn verifier_api(reponse: &Reponse) {
    verifier_entetes_communs(&reponse.entetes);
    assert_eq!(reponse.entetes[header::CONTENT_SECURITY_POLICY], CSP_API);
    assert_eq!(reponse.entetes[header::CACHE_CONTROL], "no-store");
    assert_eq!(reponse.entetes[header::CONTENT_TYPE], "application/json");
}

fn verifier_erreur(reponse: &Reponse, statut: StatusCode, code: &str) {
    assert_eq!(reponse.statut, statut, "{}", reponse.texte());
    assert_eq!(reponse.json(), serde_json::json!({ "erreur": code }));
    verifier_api(reponse);
}

#[tokio::test]
async fn le_binaire_refuse_toute_requete_non_authentifiee() {
    let routeur = routeur(&application_du_binaire(pool_sans_base()));
    for route in ROUTES {
        let chemin = route
            .chemin
            .replace("{cabinet}", &Uuid::new_v4().to_string())
            .replace("{dossier}", &Uuid::new_v4().to_string());
        // Même un en-tête de l'authentification de test ne vaut rien pour le binaire.
        let reponse = appeler_route(&routeur, route, &chemin, Some(IdUtilisateur::nouveau())).await;
        verifier_erreur(&reponse, StatusCode::UNAUTHORIZED, "non_authentifie");
    }
}

#[tokio::test]
async fn un_uuid_invalide_dans_le_chemin_donne_400_sans_echo() {
    let routeur = routeur_test(reglages());
    let utilisateur = Some(IdUtilisateur::nouveau());
    for chemin in [
        "/api/cabinets/pas-un-uuid".to_owned(),
        "/api/cabinets/%3Cscript%3E/dossiers".to_owned(),
        format!("/api/cabinets/{}/dossiers/12345", Uuid::new_v4()),
    ] {
        let reponse = lire(&routeur, &chemin, utilisateur).await;
        verifier_erreur(&reponse, StatusCode::BAD_REQUEST, "requete_invalide");
        for fragment in ["pas-un-uuid", "script", "12345"] {
            assert!(!reponse.texte().contains(fragment));
        }
    }
    // UUID nul ou maximal : jamais attribués, donc absents (404).
    for uuid in [Uuid::nil(), Uuid::max()] {
        let reponse = lire(&routeur, &format!("/api/cabinets/{uuid}"), utilisateur).await;
        verifier_erreur(&reponse, StatusCode::NOT_FOUND, "introuvable");
    }
}

#[tokio::test]
async fn hors_de_la_table_l_api_repond_404_uniforme_et_405() {
    let routeur = routeur_test(reglages());
    for chemin in ["/api", "/api/", "/api/inconnue", "/api/cabinets"] {
        let reponse = lire(&routeur, chemin, None).await;
        verifier_erreur(&reponse, StatusCode::NOT_FOUND, "introuvable");
    }
    let reponse = appeler(
        &routeur,
        Request::put(chemin_exemple())
            .header(EN_TETE_CSRF, "1")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    verifier_erreur(
        &reponse,
        StatusCode::METHOD_NOT_ALLOWED,
        "methode_non_admise",
    );
}

/// Routeur de test avec une route de mutation (il n'y en a aucune dans la table en PR 6).
fn routeur_mutation() -> Router {
    let etat = application(pool_sans_base(), reglages());
    Router::new()
        .route("/api/essai", post(|| async { "accepté" }))
        .layer(from_fn_with_state(Arc::clone(&etat), csrf::proteger))
}

async fn muter(routeur: &Router, methode: Method, entetes: &[(&str, &str)]) -> StatusCode {
    let mut requete = Request::builder().method(methode).uri("/api/essai");
    for (nom, valeur) in entetes {
        requete = requete.header(*nom, *valeur);
    }
    appeler(routeur, requete.body(Body::empty()).unwrap())
        .await
        .statut
}

#[tokio::test]
async fn l_anti_csrf_filtre_les_mutations_avant_tout() {
    const CSRF: &str = "x-sigma-csrf";
    assert_eq!(EN_TETE_CSRF.as_str(), CSRF);
    let routeur = routeur_mutation();
    let jeton = (CSRF, "1");
    // Admises : en-tête personnalisé, et marques de la même origine si présentes.
    for entetes in [
        vec![jeton],
        vec![jeton, ("sec-fetch-site", "same-origin")],
        vec![jeton, ("origin", ORIGINE)],
        vec![
            jeton,
            ("sec-fetch-site", "same-origin"),
            ("origin", ORIGINE),
        ],
    ] {
        assert_eq!(
            muter(&routeur, Method::POST, &entetes).await,
            StatusCode::OK,
            "{entetes:?}"
        );
    }
    // Refusées.
    for entetes in [
        vec![],
        vec![(CSRF, "")],
        vec![("sec-fetch-site", "same-origin"), ("origin", ORIGINE)],
        vec![jeton, ("sec-fetch-site", "cross-site")],
        vec![jeton, ("sec-fetch-site", "same-site")],
        vec![jeton, ("sec-fetch-site", "none")],
        vec![jeton, ("origin", "https://attaquant.exemple.invalid")],
        vec![jeton, ("origin", "null")],
        vec![jeton, ("origin", "http://sigma.exemple.invalid")],
        vec![
            jeton,
            ("sec-fetch-site", "same-origin"),
            ("origin", "https://sigma.exemple.invalid.attaquant.invalid"),
        ],
    ] {
        assert_eq!(
            muter(&routeur, Method::POST, &entetes).await,
            StatusCode::FORBIDDEN,
            "{entetes:?}"
        );
    }
    // Les lectures ne sont pas concernées (la route n'admet que POST : 405 de l'axum nu).
    assert_eq!(
        muter(&routeur, Method::GET, &[("sec-fetch-site", "cross-site")]).await,
        StatusCode::METHOD_NOT_ALLOWED
    );

    // Sur le routeur réel, une mutation sans marque est refusée avant l'authentification.
    let reel = routeur_test(reglages());
    for methode in [Method::POST, Method::PUT, Method::PATCH, Method::DELETE] {
        let reponse = appeler(
            &reel,
            Request::builder()
                .method(methode)
                .uri(chemin_exemple())
                .header(EN_TETE_UTILISATEUR, Uuid::new_v4().to_string())
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        verifier_erreur(&reponse, StatusCode::FORBIDDEN, "interdit");
    }
}

#[tokio::test]
async fn un_corps_trop_grand_donne_413() {
    let routeur = routeur_test(reglages());
    let taille = reglages().taille_corps_max + 1;
    let reponse = appeler(
        &routeur,
        Request::post(chemin_exemple())
            .header(EN_TETE_CSRF, "1")
            .header(header::CONTENT_LENGTH, taille.to_string())
            .body(Body::from(vec![b'x'; taille]))
            .unwrap(),
    )
    .await;
    verifier_erreur(&reponse, StatusCode::PAYLOAD_TOO_LARGE, "corps_trop_grand");
}

#[tokio::test]
async fn la_limite_de_debit_par_adresse_puis_par_utilisateur() {
    let mut r = reglages();
    r.debit_adresse = ReglageDebit {
        capacite: 3,
        periode: Duration::from_secs(60),
        cles_max: 10,
    };
    let routeur = routeur_test(r);
    for _ in 0..3 {
        assert_eq!(
            lire(&routeur, "/api", None).await.statut,
            StatusCode::NOT_FOUND
        );
    }
    let reponse = lire(&routeur, "/api", None).await;
    verifier_erreur(&reponse, StatusCode::TOO_MANY_REQUESTS, "trop_de_requetes");
    let attente: u64 = reponse.entetes[header::RETRY_AFTER]
        .to_str()
        .unwrap()
        .parse()
        .unwrap();
    assert!((1..=60).contains(&attente));

    let mut r = reglages();
    r.debit_utilisateur = ReglageDebit {
        capacite: 2,
        periode: Duration::from_secs(60),
        cles_max: 10,
    };
    let routeur = routeur_test(r);
    let (un, deux) = (IdUtilisateur::nouveau(), IdUtilisateur::nouveau());
    // Chemin invalide : refusé après l'authentification et le débit, avant la base.
    let chemin = "/api/cabinets/x";
    for _ in 0..2 {
        assert_eq!(
            lire(&routeur, chemin, Some(un)).await.statut,
            StatusCode::BAD_REQUEST
        );
    }
    let reponse = lire(&routeur, chemin, Some(un)).await;
    verifier_erreur(&reponse, StatusCode::TOO_MANY_REQUESTS, "trop_de_requetes");
    assert_eq!(
        lire(&routeur, chemin, Some(deux)).await.statut,
        StatusCode::BAD_REQUEST
    );
}

#[tokio::test]
async fn un_traitement_trop_long_est_abandonne() {
    let mut r = reglages();
    r.delai_requete = Duration::from_millis(50);
    let etat = Application::new(
        pool_sans_base(),
        pool_sans_base(),
        Arc::new(Autorisation::charger().unwrap()),
        Arc::new(AuthentificationTest {
            delai: Some(Duration::from_secs(5)),
            panique: false,
        }),
        r,
    );
    let reponse = lire(&routeur(&etat), &chemin_exemple(), None).await;
    verifier_erreur(&reponse, StatusCode::SERVICE_UNAVAILABLE, "delai_depasse");
}

#[tokio::test]
async fn les_fichiers_statiques_sans_traversee_ni_liste() {
    let mut r = reglages();
    r.repertoire_web = Some(repertoire_web());
    let routeur = routeur_test(r);

    for chemin in ["/", "/index.html", "/app.js", "/sous/fichier.txt"] {
        let reponse = lire(&routeur, chemin, None).await;
        assert_eq!(reponse.statut, StatusCode::OK, "{chemin}");
        verifier_entetes_communs(&reponse.entetes);
        assert_eq!(
            reponse.entetes[header::CONTENT_SECURITY_POLICY],
            CSP_INTERFACE
        );
        assert!(reponse.entetes.get(header::CACHE_CONTROL).is_none());
    }
    assert!(
        lire(&routeur, "/", None)
            .await
            .texte()
            .contains("Sigma Books")
    );

    // Traversée (brute, encodée, double), répertoire sans index, fichier absent : 404 uniforme,
    // sans liste ni redirection.
    for chemin in [
        "/../hors_racine.txt",
        "/sous/../../hors_racine.txt",
        "/%2e%2e/hors_racine.txt",
        "/sous/%2e%2e%2f%2e%2e%2fhors_racine.txt",
        "/%252e%252e/hors_racine.txt",
        "/..%5chors_racine.txt",
        "/sous",
        "/sous/",
        "/absent.js",
    ] {
        let reponse = lire(&routeur, chemin, None).await;
        assert_eq!(reponse.statut, StatusCode::NOT_FOUND, "{chemin}");
        assert_eq!(
            reponse.json(),
            serde_json::json!({ "erreur": "introuvable" }),
            "{chemin}"
        );
        assert!(!reponse.texte().contains("ne doit jamais sortir"));
        assert!(!reponse.texte().contains("fichier.txt"));
        assert!(reponse.entetes.get(header::LOCATION).is_none());
        verifier_entetes_communs(&reponse.entetes);
    }
    // Méthode autre que GET ou HEAD (marques de même origine présentes) : 405.
    let reponse = appeler(
        &routeur,
        Request::post("/index.html")
            .header(EN_TETE_CSRF, "1")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(reponse.statut, StatusCode::METHOD_NOT_ALLOWED);
}

#[tokio::test]
async fn une_panique_donne_un_500_uniforme_sans_son_message() {
    let etat = Application::new(
        pool_sans_base(),
        pool_sans_base(),
        Arc::new(Autorisation::charger().unwrap()),
        Arc::new(AuthentificationTest {
            delai: None,
            panique: true,
        }),
        reglages(),
    );
    let reponse = lire(&routeur(&etat), &chemin_exemple(), None).await;
    verifier_erreur(
        &reponse,
        StatusCode::INTERNAL_SERVER_ERROR,
        "erreur_interne",
    );
    assert!(!reponse.texte().contains("1234567"));
}

/// Racine statique construite à l'exécution (fichier caché, liens symboliques), dans le
/// répertoire temporaire que Cargo réserve aux tests d'intégration.
#[cfg(unix)]
fn racine_avec_liens() -> PathBuf {
    let base =
        PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("statique-{}", Uuid::new_v4()));
    let web = base.join("web");
    std::fs::create_dir_all(web.join("rep")).unwrap();
    std::fs::create_dir_all(web.join(".git")).unwrap();
    std::fs::create_dir_all(base.join("dehors")).unwrap();
    std::fs::write(
        web.join("index.html"),
        "<!doctype html><title>Sigma Books</title>",
    )
    .unwrap();
    std::fs::write(web.join(".env"), "SECRET_DE_TEST=ne-doit-jamais-sortir").unwrap();
    std::fs::write(web.join(".git/config"), "ne-doit-jamais-sortir").unwrap();
    std::fs::write(base.join("hors_racine.txt"), "ne-doit-jamais-sortir").unwrap();
    std::fs::write(base.join("dehors/index.html"), "ne-doit-jamais-sortir").unwrap();
    std::os::unix::fs::symlink(base.join("hors_racine.txt"), web.join("lien.txt")).unwrap();
    std::os::unix::fs::symlink(base.join("dehors"), web.join("lien_rep")).unwrap();
    std::os::unix::fs::symlink(base.join("dehors/index.html"), web.join("rep/index.html")).unwrap();
    std::os::unix::fs::symlink(web.join("index.html"), web.join("interne.html")).unwrap();
    web
}

#[cfg(unix)]
#[tokio::test]
async fn ni_fichier_cache_ni_lien_symbolique_hors_de_la_racine() {
    let mut r = reglages();
    r.repertoire_web = Some(racine_avec_liens());
    let routeur = routeur_test(r);
    for chemin in [
        "/.env",
        "/%2e%65nv",
        "/%2E%65NV",
        "/.git/config",
        "/%2egit/config",
        "/lien.txt",
        "/lien_rep/",
        "/lien_rep/index.html",
        "/rep/",
        "/rep",
    ] {
        let reponse = lire(&routeur, chemin, None).await;
        assert_eq!(reponse.statut, StatusCode::NOT_FOUND, "{chemin}");
        assert_eq!(
            reponse.json(),
            serde_json::json!({ "erreur": "introuvable" }),
            "{chemin}"
        );
        assert!(
            !reponse.texte().contains("ne-doit-jamais-sortir"),
            "{chemin}"
        );
    }
    // Un lien qui reste sous la racine est servi.
    let reponse = lire(&routeur, "/interne.html", None).await;
    assert_eq!(reponse.statut, StatusCode::OK);
    assert!(reponse.texte().contains("Sigma Books"));
}

/// Envoie une requête brute sur une connexion TCP et rend la réponse brute.
async fn brut(adresse: std::net::SocketAddr, requete: &[u8]) -> String {
    let mut flux = TcpStream::connect(adresse).await.unwrap();
    flux.write_all(requete).await.unwrap();
    let mut reponse = Vec::new();
    let _ = tokio::time::timeout(Duration::from_secs(5), flux.read_to_end(&mut reponse)).await;
    String::from_utf8_lossy(&reponse).into_owned()
}

#[tokio::test]
async fn la_couche_http_borne_les_en_tetes() {
    let etat = application_du_binaire(pool_sans_base());
    let ecouteur = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let adresse = ecouteur.local_addr().unwrap();
    let reglages = etat.reglages().clone();
    let (arreter, arret) = tokio::sync::oneshot::channel::<()>();
    let serveur = tokio::spawn(async move {
        servir(ecouteur, routeur(&etat), &reglages, async {
            let _ = arret.await;
        })
        .await
    });

    // Requête ordinaire : servie, avec les en-têtes de sécurité.
    let reponse = brut(
        adresse,
        b"GET /api HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n",
    )
    .await;
    assert!(reponse.starts_with("HTTP/1.1 404"), "{reponse}");
    assert!(
        reponse
            .to_ascii_lowercase()
            .contains("cross-origin-embedder-policy: require-corp")
    );

    // En-têtes trop grands, ou trop nombreux : 431.
    let grand = format!(
        "GET /api HTTP/1.1\r\nHost: x\r\nX-Grand: {}\r\nConnection: close\r\n\r\n",
        "a".repeat(20 * 1024)
    );
    let reponse = brut(adresse, grand.as_bytes()).await;
    assert!(reponse.starts_with("HTTP/1.1 431"), "{reponse}");
    let mut nombreux = String::from("GET /api HTTP/1.1\r\nHost: x\r\n");
    for i in 0..100 {
        writeln!(nombreux, "X-{i}: v\r").unwrap();
    }
    nombreux.push_str("Connection: close\r\n\r\n");
    let reponse = brut(adresse, nombreux.as_bytes()).await;
    assert!(reponse.starts_with("HTTP/1.1 431"), "{reponse}");

    arreter.send(()).unwrap();
    serveur.await.unwrap().unwrap();
}
