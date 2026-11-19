//! Client d'horodatage RFC 3161 (ADR 0010, L2-30) contre des **autorités simulées** : serveurs
//! HTTP locaux qui servent les réponses réelles et figées de `DigiCert` et de `Sectigo`
//! (`crates/integrite/tests/vecteurs/horodatage/`). Aucun accès réseau, sauf le test ignoré
//! `vraies_autorites`.
//!
//! Les réponses figées répondent à une requête figée (témoin `demo`, nonce de
//! `requete-openssl.tsq`) : le client envoie cette requête, octet pour octet, et l'autorité
//! simulée renvoie la réponse enregistrée.
// Fichier de test : les `expect`/`unwrap` y sont voulus.
#![allow(clippy::expect_used, clippy::unwrap_used)]

mod commun;

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use api::horodatage::{
    Autorite, ClientHorodatage, EchecAutorite, ErreurHorodatage, JetonObtenu, Limites,
    TYPE_REPONSE, TYPE_REQUETE, tirer_nonce,
};
use axum::Router;
use axum::body::{Body, to_bytes};
use axum::extract::Request;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use commun::serveur::{Evenement, capture_globale, champ, verifier_liste_blanche};
use integrite::horodatage::{ErreurJeton, Temoin, ancres_par_defaut, requete};
use integrite::{Empreinte, IdDossier};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

const REQUETE_OPENSSL: &[u8] =
    include_bytes!("../../integrite/tests/vecteurs/horodatage/requete-openssl.tsq");
const DIGICERT: &[u8] = include_bytes!("../../integrite/tests/vecteurs/horodatage/digicert.tsr");
const SECTIGO: &[u8] = include_bytes!("../../integrite/tests/vecteurs/horodatage/sectigo.tsr");
/// Nonce de `requete-openssl.tsq` (README des vecteurs).
const NONCE: u64 = 0xB17F_41C7_83A6_60D7;
/// Tête du témoin des vecteurs.
const TETE: &str = "ea5595d5652ca52169b5d87e9bfbcae14eafc1031c7e2f7985af5252dff7229c";

fn temoin(nombre: u64) -> Temoin {
    Temoin::nouveau(
        IdDossier::parse("demo").unwrap(),
        Empreinte::parse(TETE).unwrap(),
        nombre,
    )
}

/// Requête reçue par une autorité simulée.
#[derive(Debug, Clone)]
struct Recue {
    methode: String,
    chemin: String,
    hote: Option<String>,
    type_contenu: Option<String>,
    accepte: Option<String>,
    corps: Vec<u8>,
}

/// Comportement d'une autorité simulée.
#[derive(Debug, Clone)]
enum Comportement {
    /// Statut, type de contenu (aucun si `None`) et corps.
    Repondre(StatusCode, Option<&'static str>, Vec<u8>),
    /// Réponse 200 correcte, mais après ce délai.
    Attendre(Duration, Vec<u8>),
    /// Redirection 302 vers cette URL.
    Rediriger(String),
}

/// Autorité simulée : son URL et les requêtes reçues.
struct Simulee {
    url: String,
    recues: Arc<Mutex<Vec<Recue>>>,
}

impl Simulee {
    fn autorite(&self) -> Autorite {
        Autorite::parse(&self.url).unwrap()
    }

    fn recues(&self) -> Vec<Recue> {
        self.recues.lock().unwrap().clone()
    }
}

fn texte(requete: &Request, nom: header::HeaderName) -> Option<String> {
    requete
        .headers()
        .get(nom)
        .map(|v| v.to_str().unwrap().to_owned())
}

/// Démarre une autorité simulée (axum) sur un port local libre.
async fn simuler(comportement: Comportement) -> Simulee {
    let ecouteur = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let adresse = ecouteur.local_addr().unwrap();
    let recues = Arc::new(Mutex::new(Vec::new()));
    let journal = Arc::clone(&recues);
    let app = Router::new().fallback(move |requete: Request| {
        let comportement = comportement.clone();
        let journal = Arc::clone(&journal);
        async move {
            let recue = Recue {
                methode: requete.method().to_string(),
                chemin: requete.uri().to_string(),
                hote: texte(&requete, header::HOST),
                type_contenu: texte(&requete, header::CONTENT_TYPE),
                accepte: texte(&requete, header::ACCEPT),
                corps: to_bytes(requete.into_body(), 1 << 20)
                    .await
                    .unwrap()
                    .to_vec(),
            };
            journal.lock().unwrap().push(recue);
            reponse(comportement).await
        }
    });
    tokio::spawn(async move { axum::serve(ecouteur, app).await.unwrap() });
    Simulee {
        url: format!("http://{adresse}"),
        recues,
    }
}

async fn reponse(comportement: Comportement) -> Response {
    match comportement {
        Comportement::Repondre(statut, type_contenu, corps) => {
            let mut reponse = (statut, corps).into_response();
            reponse.headers_mut().remove(header::CONTENT_TYPE);
            if let Some(t) = type_contenu {
                reponse
                    .headers_mut()
                    .insert(header::CONTENT_TYPE, t.parse().unwrap());
            }
            reponse
        }
        Comportement::Attendre(delai, corps) => {
            tokio::time::sleep(delai).await;
            ([(header::CONTENT_TYPE, TYPE_REPONSE)], corps).into_response()
        }
        Comportement::Rediriger(cible) => Response::builder()
            .status(StatusCode::FOUND)
            .header(header::LOCATION, cible)
            .body(Body::empty())
            .unwrap(),
    }
}

fn valide(corps: &[u8]) -> Comportement {
    Comportement::Repondre(StatusCode::OK, Some(TYPE_REPONSE), corps.to_vec())
}

/// Adresse locale où personne n'écoute : connexion refusée.
async fn port_ferme() -> Autorite {
    let ecouteur = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let adresse: SocketAddr = ecouteur.local_addr().unwrap();
    drop(ecouteur);
    Autorite::parse(&format!("http://{adresse}")).unwrap()
}

fn client_limite(autorites: Vec<Autorite>, limites: Limites) -> ClientHorodatage {
    ClientHorodatage::new(autorites, ancres_par_defaut().unwrap(), limites).unwrap()
}

fn client(autorites: Vec<Autorite>) -> ClientHorodatage {
    client_limite(autorites, Limites::default())
}

/// Motifs d'échec, dans l'ordre d'essai.
fn echecs(resultat: Result<JetonObtenu, ErreurHorodatage>) -> Vec<(Autorite, EchecAutorite)> {
    match resultat {
        Err(ErreurHorodatage::ToutesEnEchec(echecs)) => echecs,
        autre => panic!("échec attendu : {autre:?}"),
    }
}

/// Seul motif d'échec d'un client à une autorité.
async fn echec_unique(simulee: &Simulee, temoin: &Temoin, nonce: u64) -> EchecAutorite {
    let mut echecs = echecs(
        client(vec![simulee.autorite()])
            .horodater(temoin, nonce)
            .await,
    );
    assert_eq!(echecs.len(), 1);
    echecs.remove(0).1
}

#[tokio::test]
async fn succes_sur_la_premiere_autorite() {
    assert_eq!(requete(&temoin(0), NONCE).unwrap(), REQUETE_OPENSSL);
    let digicert = simuler(valide(DIGICERT)).await;
    let sectigo = simuler(valide(SECTIGO)).await;
    let obtenu = client(vec![digicert.autorite(), sectigo.autorite()])
        .horodater(&temoin(0), NONCE)
        .await
        .unwrap();
    assert_eq!(obtenu.autorite, digicert.autorite());
    assert!(obtenu.verifie.autorite.contains("DigiCert"));
    assert_eq!(obtenu.verifie.date.unix_timestamp(), 1_795_000_288);
    // Le jeton conservé se vérifie seul.
    assert_eq!(
        integrite::horodatage::verifier_jeton(
            &obtenu.jeton,
            &temoin(0),
            &ancres_par_defaut().unwrap()
        )
        .unwrap(),
        obtenu.verifie
    );
    let recues = digicert.recues();
    assert_eq!(recues.len(), 1);
    let recue = &recues[0];
    assert_eq!(recue.methode, "POST");
    assert_eq!(recue.chemin, "/");
    assert_eq!(
        recue.hote.as_deref(),
        Some(digicert.autorite().nom().as_str())
    );
    assert_eq!(recue.type_contenu.as_deref(), Some(TYPE_REQUETE));
    assert_eq!(recue.accepte.as_deref(), Some(TYPE_REPONSE));
    assert_eq!(recue.corps, REQUETE_OPENSSL, "requête octet pour octet");
    assert!(sectigo.recues().is_empty(), "secours non sollicité");
}

#[tokio::test]
async fn panne_de_la_premiere_puis_succes_sur_la_seconde() {
    let ferme = port_ferme().await;
    let sectigo = simuler(valide(SECTIGO)).await;
    let obtenu = client(vec![ferme, sectigo.autorite()])
        .horodater(&temoin(0), NONCE)
        .await
        .unwrap();
    assert_eq!(obtenu.autorite, sectigo.autorite());
    assert!(obtenu.verifie.autorite.contains("Sectigo"));
}

#[tokio::test]
async fn reponses_invalides_refusees() {
    // Signature altérée : dernier octet de la réponse, qui finit par la signature RSA.
    let mut alteree = DIGICERT.to_vec();
    *alteree.last_mut().unwrap() ^= 0x01;
    let simulee = simuler(valide(&alteree)).await;
    assert!(matches!(
        echec_unique(&simulee, &temoin(0), NONCE).await,
        EchecAutorite::ReponseInvalide(ErreurJeton::SignatureInvalide(_))
    ));

    // Mauvais nonce : l'autorité renvoie une réponse à une autre requête.
    let simulee = simuler(valide(DIGICERT)).await;
    assert_eq!(
        echec_unique(&simulee, &temoin(0), NONCE ^ 1).await,
        EchecAutorite::ReponseInvalide(ErreurJeton::NonceDifferent)
    );

    // Mauvaise empreinte : la réponse porte sur un autre témoin.
    assert_eq!(
        echec_unique(&simulee, &temoin(1), NONCE).await,
        EchecAutorite::ReponseInvalide(ErreurJeton::EmpreinteDifferente)
    );

    // Corps illisible.
    let simulee = simuler(valide(b"pas du DER")).await;
    assert!(matches!(
        echec_unique(&simulee, &temoin(0), NONCE).await,
        EchecAutorite::ReponseInvalide(ErreurJeton::Illisible(_))
    ));

    // Une réponse invalide est une panne : l'autorité suivante est essayée.
    let invalide = simuler(valide(&alteree)).await;
    let sectigo = simuler(valide(SECTIGO)).await;
    let obtenu = client(vec![invalide.autorite(), sectigo.autorite()])
        .horodater(&temoin(0), NONCE)
        .await
        .unwrap();
    assert_eq!(obtenu.autorite, sectigo.autorite());
}

#[tokio::test]
async fn mauvais_type_de_contenu_refuse() {
    for type_contenu in [Some("application/octet-stream"), Some("text/html"), None] {
        let simulee = simuler(Comportement::Repondre(
            StatusCode::OK,
            type_contenu,
            DIGICERT.to_vec(),
        ))
        .await;
        assert_eq!(
            echec_unique(&simulee, &temoin(0), NONCE).await,
            EchecAutorite::TypeDeContenu,
            "{type_contenu:?}"
        );
    }
}

#[tokio::test]
async fn statut_500_puis_secours() {
    let panne = simuler(Comportement::Repondre(
        StatusCode::INTERNAL_SERVER_ERROR,
        Some(TYPE_REPONSE),
        DIGICERT.to_vec(),
    ))
    .await;
    assert_eq!(
        echec_unique(&panne, &temoin(0), NONCE).await,
        EchecAutorite::StatutHttp(500)
    );
    let sectigo = simuler(valide(SECTIGO)).await;
    let obtenu = client(vec![panne.autorite(), sectigo.autorite()])
        .horodater(&temoin(0), NONCE)
        .await
        .unwrap();
    assert_eq!(obtenu.autorite, sectigo.autorite());
}

#[tokio::test]
async fn delai_depasse() {
    let lente = simuler(Comportement::Attendre(
        Duration::from_secs(30),
        DIGICERT.to_vec(),
    ))
    .await;
    let limites = Limites {
        total: Duration::from_millis(300),
        ..Limites::default()
    };
    let debut = Instant::now();
    let echecs = echecs(
        client_limite(vec![lente.autorite()], limites)
            .horodater(&temoin(0), NONCE)
            .await,
    );
    assert_eq!(echecs, [(lente.autorite(), EchecAutorite::DelaiTotal)]);
    assert!(debut.elapsed() < Duration::from_secs(5));
}

#[tokio::test]
async fn reponse_trop_grande_refusee() {
    // Taille annoncée (`Content-Length`) au-delà de la limite : refus avant lecture.
    let annoncee = simuler(valide(&vec![0u8; 100 * 1024])).await;
    assert_eq!(
        echec_unique(&annoncee, &temoin(0), NONCE).await,
        EchecAutorite::ReponseTropGrande(Limites::default().taille_reponse)
    );

    // Taille annoncée sous la limite (le vrai jeton), mais limite abaissée sous sa taille.
    let simulee = simuler(valide(DIGICERT)).await;
    let limites = Limites {
        taille_reponse: 1024,
        ..Limites::default()
    };
    assert_eq!(
        echecs(
            client_limite(vec![simulee.autorite()], limites)
                .horodater(&temoin(0), NONCE)
                .await
        )[0]
        .1,
        EchecAutorite::ReponseTropGrande(1024)
    );

    // Corps sans `Content-Length` (par morceaux) : coupé à la limite pendant la lecture.
    let morceaux = morceaux_sans_fin().await;
    assert_eq!(
        echecs(
            client(vec![morceaux.clone()])
                .horodater(&temoin(0), NONCE)
                .await
        ),
        [(
            morceaux,
            EchecAutorite::ReponseTropGrande(Limites::default().taille_reponse)
        )]
    );
}

/// Autorité simulée au niveau TCP : réponse 200 envoyée par morceaux (`chunked`), sans
/// `Content-Length`, de 1 Mio au total.
async fn morceaux_sans_fin() -> Autorite {
    let ecouteur = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let adresse = ecouteur.local_addr().unwrap();
    tokio::spawn(async move {
        let (mut flux, _) = ecouteur.accept().await.unwrap();
        let mut tampon = [0u8; 4096];
        let _ = flux.read(&mut tampon).await;
        let entete = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: {TYPE_REPONSE}\r\ntransfer-encoding: chunked\r\n\r\n"
        );
        if flux.write_all(entete.as_bytes()).await.is_err() {
            return;
        }
        let morceau = [b"2000\r\n".as_slice(), &[0u8; 0x2000], b"\r\n"].concat();
        for _ in 0..128 {
            if flux.write_all(&morceau).await.is_err() {
                return;
            }
        }
        let _ = flux.write_all(b"0\r\n\r\n").await;
    });
    Autorite::parse(&format!("http://{adresse}")).unwrap()
}

#[tokio::test]
async fn redirection_non_suivie() {
    let cible = simuler(valide(DIGICERT)).await;
    let redirige = simuler(Comportement::Rediriger(format!("{}/", cible.url))).await;
    assert_eq!(
        echec_unique(&redirige, &temoin(0), NONCE).await,
        EchecAutorite::StatutHttp(302)
    );
    assert!(cible.recues().is_empty(), "la redirection a été suivie");
}

#[tokio::test]
async fn toutes_les_autorites_en_echec() {
    let ferme = port_ferme().await;
    let panne = simuler(Comportement::Repondre(
        StatusCode::SERVICE_UNAVAILABLE,
        None,
        Vec::new(),
    ))
    .await;
    let resultat = client(vec![ferme.clone(), panne.autorite()])
        .horodater(&temoin(0), NONCE)
        .await;
    let message = resultat.as_ref().unwrap_err().to_string();
    assert!(message.contains("2 essai(s)"), "{message}");
    let echecs = echecs(resultat);
    assert_eq!(echecs.len(), 2);
    assert_eq!(echecs[0].0, ferme);
    assert!(matches!(echecs[0].1, EchecAutorite::Reseau(_)));
    assert_eq!(
        echecs[1],
        (panne.autorite(), EchecAutorite::StatutHttp(503))
    );
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
    let panne = simuler(Comportement::Repondre(
        StatusCode::INTERNAL_SERVER_ERROR,
        None,
        Vec::new(),
    ))
    .await;
    let digicert = simuler(valide(DIGICERT)).await;
    client(vec![panne.autorite(), digicert.autorite()])
        .horodater(&temoin(0), NONCE)
        .await
        .unwrap();

    let hex = |octets: &[u8]| {
        use std::fmt::Write;
        octets.iter().fold(String::new(), |mut texte, o| {
            write!(texte, "{o:02x}").unwrap();
            texte
        })
    };
    // Ni l'empreinte du témoin (même un fragment), ni la tête, ni les corps échangés.
    let interdits = vec![
        hex(&temoin(0).empreinte())[..16].to_owned(),
        TETE[..16].to_owned(),
        hex(&REQUETE_OPENSSL[..12]),
        hex(&DIGICERT[..12]),
        "demo".to_owned(),
    ];
    for (autorite, statut, issue) in [
        (panne.autorite(), "500", "statut_http"),
        (digicert.autorite(), "200", "succes"),
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
            .horodater(&temoin(0), tirer_nonce().unwrap())
            .await
            .unwrap_or_else(|e| panic!("{autorite} : {e:?}"));
        assert_eq!(obtenu.autorite, autorite);
    }
}
