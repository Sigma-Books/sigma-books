//! Client d'horodatage RFC 3161 (ADR 0010, L2-30), vu de l'extérieur du crate : journaux en
//! liste blanche, et vérification manuelle contre les autorités réelles (test ignoré).
//!
//! Les cas détaillés contre des autorités simulées, qui rejouent les vecteurs figés avec un nonce
//! et une horloge fixés, sont des tests internes au crate (`src/horodatage/simulation.rs`) : hors
//! tests, seul l'aléa du système fournit un nonce.
// Fichier de test : les `expect`/`unwrap` y sont voulus.
#![allow(clippy::expect_used, clippy::unwrap_used)]

mod commun;

use api::horodatage::{Autorite, ClientHorodatage, Limites, TYPE_REPONSE, tirer_nonce};
use axum::Router;
use axum::http::{StatusCode, header};
use axum::response::IntoResponse;
use commun::serveur::{Evenement, capture_globale, champ, verifier_liste_blanche};
use integrite::horodatage::{Temoin, ancres_par_defaut};
use integrite::{Empreinte, IdDossier};
use tokio::net::TcpListener;

const DIGICERT: &[u8] = include_bytes!("../../integrite/tests/vecteurs/horodatage/digicert.tsr");
/// Tête du témoin des vecteurs.
const TETE: &str = "ea5595d5652ca52169b5d87e9bfbcae14eafc1031c7e2f7985af5252dff7229c";

fn temoin() -> Temoin {
    Temoin::nouveau(
        IdDossier::parse("demo").unwrap(),
        Empreinte::parse(TETE).unwrap(),
        0,
    )
}

fn client(autorites: Vec<Autorite>) -> ClientHorodatage {
    ClientHorodatage::new(autorites, ancres_par_defaut().unwrap(), Limites::default()).unwrap()
}

/// Autorité simulée qui répond toujours `statut`, avec ce corps.
async fn simuler(statut: StatusCode, corps: &'static [u8]) -> Autorite {
    let ecouteur = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let adresse = ecouteur.local_addr().unwrap();
    let app = Router::new().fallback(move || async move {
        (statut, [(header::CONTENT_TYPE, TYPE_REPONSE)], corps).into_response()
    });
    tokio::spawn(async move { axum::serve(ecouteur, app).await.unwrap() });
    Autorite::parse(&format!("http://{adresse}")).unwrap()
}

/// Événements de journal d'une autorité.
fn evenements(autorite: &Autorite) -> Vec<Evenement> {
    let nom = autorite.nom();
    capture_globale()
        .0
        .lock()
        .unwrap()
        .iter()
        .filter(|e| champ(e, "autorite") == Some(nom.as_str()))
        .cloned()
        .collect()
}

#[tokio::test]
async fn journaux_en_liste_blanche() {
    capture_globale();
    let panne = simuler(StatusCode::INTERNAL_SERVER_ERROR, b"").await;
    // Réponse réelle, mais à une autre requête (nonce tiré au hasard) : refusée.
    let rejouee = simuler(StatusCode::OK, DIGICERT).await;
    let erreur = client(vec![panne.clone(), rejouee.clone()])
        .horodater(&temoin(), tirer_nonce().unwrap())
        .await
        .unwrap_err();
    assert!(erreur.to_string().contains("2 essai(s)"));

    let hex = |octets: &[u8]| {
        use std::fmt::Write;
        octets.iter().fold(String::new(), |mut texte, o| {
            write!(texte, "{o:02x}").unwrap();
            texte
        })
    };
    // Ni l'empreinte du témoin (même un fragment), ni la tête, ni le corps reçu, ni le nom du
    // signataire de la réponse (texte contrôlé par l'autorité).
    let interdits = vec![
        hex(&temoin().empreinte())[..16].to_owned(),
        TETE[..16].to_owned(),
        hex(&DIGICERT[..12]),
        "DigiCert".to_owned(),
        "demo".to_owned(),
    ];
    for (autorite, statut, issue) in [
        (panne, "500", "statut_http"),
        (rejouee, "200", "reponse_invalide"),
    ] {
        let evenements = evenements(&autorite);
        assert_eq!(evenements.len(), 1, "{evenements:?}");
        verifier_liste_blanche(&evenements, &interdits);
        assert_eq!(champ(&evenements[0], "statut"), Some(statut));
        assert_eq!(champ(&evenements[0], "issue"), Some(issue));
        assert!(champ(&evenements[0], "duree_ms").is_some());
    }
}

/// Vérification manuelle contre les **vraies** autorités de l'ADR 0010 (accès réseau) :
/// `cargo test -p api --test horodatage -- --ignored vraies_autorites`. Ignoré par défaut, la
/// CI n'ayant pas d'accès réseau ; seule l'empreinte d'un témoin fictif part.
#[tokio::test]
#[ignore = "accès réseau aux autorités réelles : vérification manuelle"]
async fn vraies_autorites() {
    for autorite in Autorite::par_defaut() {
        let obtenu = client(vec![autorite.clone()])
            .horodater(&temoin(), tirer_nonce().unwrap())
            .await
            .unwrap_or_else(|e| panic!("{autorite} : {e:?}"));
        assert_eq!(obtenu.autorite, autorite);
    }
}
