//! Journaux techniques en liste blanche (F2.11, VP3 ; L2, « Tests exigés ») : les journaux d'un
//! scénario ne contiennent que les champs de `journal::CHAMPS`, et ni montant, ni libellé, ni
//! jeton, ni identifiant tiré du chemin. Scénario sans base ici ; scénario complet avec la base
//! dans `isolation.rs` (`tests-postgres`).
// Fichier de test : les `expect`/`unwrap` y sont voulus.
#![allow(clippy::expect_used, clippy::unwrap_used)]

mod commun;

use std::path::PathBuf;

use api::base::IdUtilisateur;
use api::serveur::{Reglages, routeur};
use axum::body::Body;
use axum::http::{Request, header};
use commun::serveur::{
    ORIGINE, appeler, application, capture_globale, champ, evenements_de, id_requete, lire,
    pool_sans_base, verifier_liste_blanche,
};
use uuid::Uuid;

#[tokio::test]
async fn les_journaux_ne_contiennent_que_la_liste_blanche() {
    let capture = capture_globale();
    let mut reglages = Reglages::new(ORIGINE.to_owned());
    reglages.repertoire_web =
        Some(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/donnees/web"));
    let routeur = routeur(&application(pool_sans_base(), reglages));
    let secret = "jeton-secret-de-session";
    let utilisateur = IdUtilisateur::nouveau();
    let cabinet = Uuid::new_v4();

    let reponses = [
        lire(&routeur, &format!("/api/cabinets/{cabinet}"), None).await,
        lire(&routeur, "/api/cabinets/montant-1234567", Some(utilisateur)).await,
        lire(&routeur, "/index.html", None).await,
        lire(&routeur, "/libelle-confidentiel", None).await,
        appeler(
            &routeur,
            Request::post(format!("/api/cabinets/{cabinet}"))
                .header(header::COOKIE, format!("__Host-sigma_session={secret}"))
                .header(header::AUTHORIZATION, secret)
                .body(Body::from(
                    "{\"montant\":\"1234567\",\"libelle\":\"Facture confidentielle\"}",
                ))
                .unwrap(),
        )
        .await,
    ];
    let ids: Vec<String> = reponses.iter().map(id_requete).collect();
    let evenements = evenements_de(capture, &ids);
    assert_eq!(evenements.len(), ids.len(), "une ligne par requête");
    verifier_liste_blanche(
        &evenements,
        &[
            secret.to_owned(),
            "1234567".to_owned(),
            "confidentiel".to_owned(),
            "Facture".to_owned(),
            cabinet.to_string(),
            utilisateur.to_string(),
        ],
    );
    let routes: Vec<&str> = evenements
        .iter()
        .filter_map(|e| champ(e, "route"))
        .collect();
    assert_eq!(
        routes,
        [
            "/api/cabinets/{cabinet}",
            "/api/cabinets/{cabinet}",
            "statique",
            "statique",
            // Refusée par l'anti-CSRF avant le routage.
            "aucune",
        ]
    );
    let statuts: Vec<&str> = evenements
        .iter()
        .filter_map(|e| champ(e, "statut"))
        .collect();
    assert_eq!(statuts, ["401", "400", "200", "404", "403"]);
}
