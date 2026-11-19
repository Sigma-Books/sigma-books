//! Aucune route hors de la table `routes::ROUTES` (L2, « Tests exigés », « chaque route »).
//!
//! `axum` n'expose pas la liste des routes d'un `Router` : le test « chaque route »
//! (`isolation.rs`) ne peut énumérer que la table. Ce test garantit donc que **la table est le
//! routeur** :
//! 1. **structure** : dans les sources du crate (hors modules de test), le seul `Router::new`,
//!    les seuls `.route(` et les seuls repli (`fallback`) sont ceux de `serveur::routeur`, qui
//!    monte chaque entrée de la table et deux routes « introuvable » sous `/api` ; aucun
//!    `.nest`, `.merge` ni `.route_service`. Toute route ajoutée ailleurs fait échouer ce test ;
//! 2. **exécution** : chaque entrée de la table est bien montée sur le pipeline (une requête
//!    sans authentification y reçoit 401, que seul le pipeline produit), et une route voisine
//!    hors table reçoit le 404 uniforme.
// Fichier de test : les `expect`/`unwrap` y sont voulus.
#![allow(clippy::expect_used, clippy::unwrap_used)]

mod commun;

use std::path::{Path, PathBuf};

use api::serveur::routes::ROUTES;
use api::serveur::routeur;
use axum::http::StatusCode;
use commun::serveur::{appeler_route, application_du_binaire, lire, pool_sans_base};
use uuid::Uuid;

/// Sources du crate, hors modules de test. Un module de test est toujours en fin de fichier :
/// le lint `clippy::items_after_test_module`, bloquant en CI, l'impose ; rien n'échappe donc
/// au contrôle en se plaçant après lui.
fn sources() -> Vec<(PathBuf, String)> {
    fn parcourir(repertoire: &Path, sortie: &mut Vec<(PathBuf, String)>) {
        for entree in std::fs::read_dir(repertoire).unwrap() {
            let chemin = entree.unwrap().path();
            if chemin.is_dir() {
                parcourir(&chemin, sortie);
            } else if chemin.extension().is_some_and(|e| e == "rs") {
                let texte = std::fs::read_to_string(&chemin).unwrap();
                let texte = match texte.find("#[cfg(test)]\nmod tests") {
                    Some(fin) => texte[..fin].to_owned(),
                    None => texte,
                };
                sortie.push((chemin, texte));
            }
        }
    }
    let mut sortie = Vec::new();
    parcourir(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
        &mut sortie,
    );
    assert!(sortie.len() > 5, "sources introuvables");
    sortie
}

/// Occurrences de `motif`, avec le texte qui suit (espaces compactés), par fichier.
fn occurrences<'a>(sources: &'a [(PathBuf, String)], motif: &str) -> Vec<(&'a Path, String)> {
    let mut trouvees = Vec::new();
    for (chemin, texte) in sources {
        for (i, _) in texte.match_indices(motif) {
            let suite: String = texte[i + motif.len()..]
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ");
            trouvees.push((chemin.as_path(), suite.chars().take(40).collect()));
        }
    }
    trouvees
}

#[test]
fn le_routeur_ne_se_construit_que_depuis_la_table() {
    let sources = sources();
    let routeur_rs = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/serveur/mod.rs");
    let seul_dans_le_routeur = |motif: &str, debuts: &[&str]| {
        let trouvees = occurrences(&sources, motif);
        assert_eq!(
            trouvees.len(),
            debuts.len(),
            "`{motif}` : {trouvees:?} ; toute route passe par `routes::ROUTES`"
        );
        for (chemin, suite) in &trouvees {
            assert_eq!(*chemin, routeur_rs.as_path(), "`{motif}` hors du routeur");
            assert!(
                debuts.iter().any(|d| suite.starts_with(d)),
                "`{motif}{suite}` : route hors de la table"
            );
        }
    };
    seul_dans_le_routeur("Router::new(", &[")"]);
    seul_dans_le_routeur(
        ".route(",
        &[
            "route.chemin,",
            "entetes::PREFIXE_API, any(introuvable)",
            "\"/api/{*reste}\", any(introuvable)",
        ],
    );
    seul_dans_le_routeur(".fallback(", &["introuvable)"]);
    seul_dans_le_routeur(".fallback_service(", &["statique::service("]);
    for interdit in [
        ".nest(",
        ".nest_service(",
        ".merge(",
        ".route_service(",
        "Router::from",
    ] {
        assert_eq!(occurrences(&sources, interdit), Vec::new(), "{interdit}");
    }
    // La boucle monte la table, et elle seule.
    let texte = &sources.iter().find(|(c, _)| *c == routeur_rs).unwrap().1;
    assert!(texte.contains("for route in routes::ROUTES {"));
}

#[test]
fn le_binaire_n_authentifie_que_par_les_sessions_de_postgresql() {
    // Une seule implémentation de l'authentification dans les sources du crate : celle des
    // sessions. Celle des tests vit dans `tests/` et n'est pas compilée dans le binaire.
    let sources = sources();
    let implementations = occurrences(&sources, "impl Authentification for ");
    assert_eq!(implementations.len(), 1, "{implementations:?}");
    assert!(
        implementations[0]
            .1
            .starts_with("AuthentificationSession {"),
        "{implementations:?}"
    );
    assert!(implementations[0].0.ends_with("src/serveur/session.rs"));
    // Le binaire l'assemble, avec l'horloge du système.
    let main = &sources
        .iter()
        .find(|(c, _)| c.ends_with("src/main.rs"))
        .expect("main.rs")
        .1;
    assert!(main.contains("AuthentificationSession::new("));
    assert!(main.contains("Arc::new(HorlogeSysteme)"));
    assert!(!main.contains("avec_horloge"));
}

#[tokio::test]
async fn chaque_entree_de_la_table_est_montee_sur_le_pipeline() {
    let routeur = routeur(&application_du_binaire(pool_sans_base()));
    for route in ROUTES {
        let chemin = route
            .chemin
            .replace("{cabinet}", &Uuid::new_v4().to_string())
            .replace("{dossier}", &Uuid::new_v4().to_string())
            .replace("{session}", &Uuid::new_v4().to_string());
        assert_eq!(
            appeler_route(&routeur, route, &chemin, None).await.statut,
            StatusCode::UNAUTHORIZED,
            "{chemin}"
        );
        // Deux segments de plus, qu'aucun modèle n'accepte : 404 uniforme, jamais le pipeline.
        let voisin = format!("{chemin}/inconnu/inconnu");
        assert_eq!(
            lire(&routeur, &voisin, None).await.statut,
            StatusCode::NOT_FOUND,
            "{voisin}"
        );
    }
}
