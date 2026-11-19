//! Client d'horodatage contre des **autorités simulées** : serveurs HTTP locaux (`hyper`, ou
//! TCP brut pour les réponses malformées) qui servent les réponses réelles et figées de `DigiCert` et
//! de `Sectigo` (`crates/integrite/tests/vecteurs/horodatage/`). Aucun accès réseau.
//!
//! Les réponses figées répondent à une requête figée (témoin `demo`, nonce de
//! `requete-openssl.tsq`) et sont datées du 2026-11-18 : ces tests fixent le nonce
//! ([`Nonce::fixe`]) et l'horloge ([`ClientHorodatage::avec_horloge`]), ce que seuls les tests du
//! crate peuvent faire (`cfg(test)`). Journaux et autorités réelles : `tests/horodatage.rs`.

use std::future::Future;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use std::convert::Infallible;

use axum::http::{Request, Response, StatusCode, header};
use http_body_util::{BodyExt, Full};
use hyper::body::{Bytes, Incoming};
use hyper::server;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use integrite::horodatage::{ErreurJeton, Temoin, ancres_par_defaut, requete};
use integrite::{Empreinte, IdDossier};
use time::OffsetDateTime;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use super::{
    Autorite, ClientHorodatage, EchecAutorite, ErreurHorodatage, JetonObtenu, Limites, Nonce,
    TYPE_REPONSE, TYPE_REQUETE,
};

const REQUETE_OPENSSL: &[u8] =
    include_bytes!("../../../integrite/tests/vecteurs/horodatage/requete-openssl.tsq");
const DIGICERT: &[u8] = include_bytes!("../../../integrite/tests/vecteurs/horodatage/digicert.tsr");
const SECTIGO: &[u8] = include_bytes!("../../../integrite/tests/vecteurs/horodatage/sectigo.tsr");
/// Nonce de `requete-openssl.tsq` (README des vecteurs).
const NONCE_VECTEURS: u64 = 0xB17F_41C7_83A6_60D7;
/// Date des deux réponses figées : 2026-11-18T11:11:28Z.
const DATE_VECTEURS: i64 = 1_795_000_288;
/// Tête du témoin des vecteurs.
const TETE: &str = "ea5595d5652ca52169b5d87e9bfbcae14eafc1031c7e2f7985af5252dff7229c";

fn nonce() -> Nonce {
    Nonce::fixe(NONCE_VECTEURS)
}

fn temoin(nombre: u64) -> Temoin {
    Temoin::nouveau(
        IdDossier::parse("demo").unwrap(),
        Empreinte::parse(TETE).unwrap(),
        nombre,
    )
}

fn date(decalage_s: i64) -> OffsetDateTime {
    OffsetDateTime::from_unix_timestamp(DATE_VECTEURS + decalage_s).unwrap()
}

fn a_la_date_des_vecteurs() -> OffsetDateTime {
    date(0)
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

fn texte(requete: &Request<Incoming>, nom: header::HeaderName) -> Option<String> {
    requete
        .headers()
        .get(nom)
        .map(|v| v.to_str().unwrap().to_owned())
}

/// Démarre une autorité simulée (`hyper`, un service unique sans routage) sur un port local
/// libre.
async fn simuler(comportement: Comportement) -> Simulee {
    let ecouteur = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let adresse = ecouteur.local_addr().unwrap();
    let recues = Arc::new(Mutex::new(Vec::new()));
    let journal = Arc::clone(&recues);
    tokio::spawn(async move {
        loop {
            let (flux, _) = ecouteur.accept().await.unwrap();
            let comportement = comportement.clone();
            let journal = Arc::clone(&journal);
            let service = service_fn(move |requete: Request<Incoming>| {
                let comportement = comportement.clone();
                let journal = Arc::clone(&journal);
                async move {
                    let recue = Recue {
                        methode: requete.method().to_string(),
                        chemin: requete.uri().to_string(),
                        hote: texte(&requete, header::HOST),
                        type_contenu: texte(&requete, header::CONTENT_TYPE),
                        accepte: texte(&requete, header::ACCEPT),
                        corps: requete
                            .into_body()
                            .collect()
                            .await
                            .unwrap()
                            .to_bytes()
                            .to_vec(),
                    };
                    journal.lock().unwrap().push(recue);
                    Ok::<_, Infallible>(reponse(comportement).await)
                }
            });
            tokio::spawn(async move {
                let _ = server::conn::http1::Builder::new()
                    .serve_connection(TokioIo::new(flux), service)
                    .await;
            });
        }
    });
    Simulee {
        url: format!("http://{adresse}"),
        recues,
    }
}

async fn reponse(comportement: Comportement) -> Response<Full<Bytes>> {
    let reponse = Response::builder();
    match comportement {
        Comportement::Repondre(statut, type_contenu, corps) => {
            let mut reponse = reponse.status(statut);
            if let Some(t) = type_contenu {
                reponse = reponse.header(header::CONTENT_TYPE, t);
            }
            reponse.body(Full::from(corps)).unwrap()
        }
        Comportement::Attendre(delai, corps) => {
            tokio::time::sleep(delai).await;
            reponse
                .header(header::CONTENT_TYPE, TYPE_REPONSE)
                .body(Full::from(corps))
                .unwrap()
        }
        Comportement::Rediriger(cible) => reponse
            .status(StatusCode::FOUND)
            .header(header::LOCATION, cible)
            .body(Full::default())
            .unwrap(),
    }
}

/// Autorité simulée au niveau TCP : lit la requête, puis `script` écrit ce qu'il veut.
async fn simuler_tcp<F, Fut>(script: F) -> Autorite
where
    F: FnOnce(TcpStream) -> Fut + Send + 'static,
    Fut: Future<Output = ()> + Send,
{
    let ecouteur = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let adresse = ecouteur.local_addr().unwrap();
    tokio::spawn(async move {
        let (mut flux, _) = ecouteur.accept().await.unwrap();
        let mut tampon = [0u8; 4096];
        let _ = flux.read(&mut tampon).await;
        script(flux).await;
    });
    Autorite::parse(&format!("http://{adresse}")).unwrap()
}

fn entete_200(suite: &str) -> Vec<u8> {
    format!("HTTP/1.1 200 OK\r\ncontent-type: {TYPE_REPONSE}\r\n{suite}\r\n").into_bytes()
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
    ClientHorodatage::new(autorites, ancres_par_defaut().unwrap(), limites)
        .unwrap()
        .avec_horloge(a_la_date_des_vecteurs)
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
async fn echec_unique_avec(client: &ClientHorodatage, temoin: &Temoin, n: Nonce) -> EchecAutorite {
    let mut echecs = echecs(client.horodater(temoin, n).await);
    assert_eq!(echecs.len(), 1);
    echecs.remove(0).1
}

async fn echec_unique(autorite: Autorite, temoin: &Temoin, n: Nonce) -> EchecAutorite {
    echec_unique_avec(&client(vec![autorite]), temoin, n).await
}

#[tokio::test]
async fn succes_sur_la_premiere_autorite() {
    assert_eq!(
        requete(&temoin(0), NONCE_VECTEURS).unwrap(),
        REQUETE_OPENSSL
    );
    let digicert = simuler(valide(DIGICERT)).await;
    let sectigo = simuler(valide(SECTIGO)).await;
    let obtenu = client(vec![digicert.autorite(), sectigo.autorite()])
        .horodater(&temoin(0), nonce())
        .await
        .unwrap();
    assert_eq!(obtenu.autorite, digicert.autorite());
    assert!(obtenu.verifie.autorite.contains("DigiCert"));
    assert_eq!(obtenu.verifie.date, date(0));
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
        .horodater(&temoin(0), nonce())
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
        echec_unique(simulee.autorite(), &temoin(0), nonce()).await,
        EchecAutorite::ReponseInvalide(ErreurJeton::SignatureInvalide(_))
    ));

    // Mauvais nonce : l'autorité renvoie une réponse à une autre requête.
    let simulee = simuler(valide(DIGICERT)).await;
    assert_eq!(
        echec_unique(
            simulee.autorite(),
            &temoin(0),
            Nonce::fixe(NONCE_VECTEURS ^ 1)
        )
        .await,
        EchecAutorite::ReponseInvalide(ErreurJeton::NonceDifferent)
    );

    // Mauvaise empreinte : la réponse porte sur un autre témoin.
    assert_eq!(
        echec_unique(simulee.autorite(), &temoin(1), nonce()).await,
        EchecAutorite::ReponseInvalide(ErreurJeton::EmpreinteDifferente)
    );

    // Corps illisible.
    let simulee = simuler(valide(b"pas du DER")).await;
    assert!(matches!(
        echec_unique(simulee.autorite(), &temoin(0), nonce()).await,
        EchecAutorite::ReponseInvalide(ErreurJeton::Illisible(_))
    ));

    // Une réponse invalide est une panne : l'autorité suivante est essayée.
    let invalide = simuler(valide(&alteree)).await;
    let sectigo = simuler(valide(SECTIGO)).await;
    let obtenu = client(vec![invalide.autorite(), sectigo.autorite()])
        .horodater(&temoin(0), nonce())
        .await
        .unwrap();
    assert_eq!(obtenu.autorite, sectigo.autorite());
}

/// Statut RFC 3161 de la réponse (`PKIStatusInfo`), fabriqué à la main : `TimeStampResp ::=
/// SEQUENCE { status SEQUENCE { INTEGER statut }, timeStampToken OPTIONAL }`.
#[tokio::test]
async fn statuts_rfc_3161() {
    // « granted with mods » (1) : la réponse figée, statut changé (octet 8 : `30 82 l l 30 03
    // 02 01 <statut>`), est acceptée si empreinte et nonce sont bons.
    assert_eq!(&DIGICERT[4..9], [0x30, 0x03, 0x02, 0x01, 0x00]);
    let mut modifiee = DIGICERT.to_vec();
    modifiee[8] = 0x01;
    let simulee = simuler(valide(&modifiee)).await;
    let obtenu = client(vec![simulee.autorite()])
        .horodater(&temoin(0), nonce())
        .await
        .unwrap();
    assert_eq!(obtenu.verifie.date, date(0));
    // Mais pas pour un autre nonce.
    assert_eq!(
        echec_unique(
            simulee.autorite(),
            &temoin(0),
            Nonce::fixe(NONCE_VECTEURS ^ 1)
        )
        .await,
        EchecAutorite::ReponseInvalide(ErreurJeton::NonceDifferent)
    );

    // « rejection » (2), sans jeton.
    let rejet = simuler(valide(&[0x30, 0x05, 0x30, 0x03, 0x02, 0x01, 0x02])).await;
    assert!(matches!(
        echec_unique(rejet.autorite(), &temoin(0), nonce()).await,
        EchecAutorite::ReponseInvalide(ErreurJeton::Refusee(_))
    ));

    // « granted » (0) sans jeton.
    let vide = simuler(valide(&[0x30, 0x05, 0x30, 0x03, 0x02, 0x01, 0x00])).await;
    assert!(matches!(
        echec_unique(vide.autorite(), &temoin(0), nonce()).await,
        EchecAutorite::ReponseInvalide(ErreurJeton::Illisible(_))
    ));
}

fn lendemain() -> OffsetDateTime {
    date(86_400)
}
fn plus_4_min() -> OffsetDateTime {
    date(4 * 60)
}
fn plus_6_min() -> OffsetDateTime {
    date(6 * 60)
}
fn moins_4_min() -> OffsetDateTime {
    date(-4 * 60)
}
fn moins_6_min() -> OffsetDateTime {
    date(-6 * 60)
}

/// Date du jeton (RFC 3161 §2.4.2) : acceptée à moins de 5 minutes de l'envoi, dans les deux
/// sens ; refusée au-delà, et en particulier pour un jeton ancien rejoué.
#[tokio::test]
async fn date_du_jeton_dans_la_fenetre() {
    let simulee = simuler(valide(DIGICERT)).await;
    let base = ClientHorodatage::new(
        vec![simulee.autorite()],
        ancres_par_defaut().unwrap(),
        Limites::default(),
    )
    .unwrap();
    for (horloge, admis) in [
        (a_la_date_des_vecteurs as fn() -> OffsetDateTime, true),
        (plus_4_min, true),
        (moins_4_min, true),
        (plus_6_min, false),
        (moins_6_min, false),
        (lendemain, false),
    ] {
        let client = base.clone().avec_horloge(horloge);
        let resultat = client.horodater(&temoin(0), nonce()).await;
        if admis {
            assert!(resultat.is_ok(), "{:?} : {resultat:?}", horloge());
        } else {
            assert_eq!(
                echecs(resultat),
                [(simulee.autorite(), EchecAutorite::DateHorsFenetre)],
                "{:?}",
                horloge()
            );
        }
    }
    // Fenêtre réglable.
    let large = Limites {
        ecart_date: Duration::from_hours(48),
        ..Limites::default()
    };
    assert!(
        client_limite(vec![simulee.autorite()], large)
            .avec_horloge(lendemain)
            .horodater(&temoin(0), nonce())
            .await
            .is_ok()
    );
    // L'horloge du système (2026-11-19 ou plus tard) refuse le jeton figé du 2026-11-18.
    let systeme = ClientHorodatage::new(
        vec![simulee.autorite()],
        ancres_par_defaut().unwrap(),
        Limites::default(),
    )
    .unwrap();
    assert_eq!(
        echec_unique_avec(&systeme, &temoin(0), nonce()).await,
        EchecAutorite::DateHorsFenetre
    );
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
            echec_unique(simulee.autorite(), &temoin(0), nonce()).await,
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
        echec_unique(panne.autorite(), &temoin(0), nonce()).await,
        EchecAutorite::StatutHttp(500)
    );
    let sectigo = simuler(valide(SECTIGO)).await;
    let obtenu = client(vec![panne.autorite(), sectigo.autorite()])
        .horodater(&temoin(0), nonce())
        .await
        .unwrap();
    assert_eq!(obtenu.autorite, sectigo.autorite());
}

fn limites_courtes() -> Limites {
    Limites {
        total: Duration::from_millis(400),
        ..Limites::default()
    }
}

#[tokio::test]
async fn delai_depasse() {
    // Aucune réponse.
    let lente = simuler(Comportement::Attendre(
        Duration::from_secs(30),
        DIGICERT.to_vec(),
    ))
    .await;
    let debut = Instant::now();
    let echecs_lente = echecs(
        client_limite(vec![lente.autorite()], limites_courtes())
            .horodater(&temoin(0), nonce())
            .await,
    );
    assert_eq!(
        echecs_lente,
        [(lente.autorite(), EchecAutorite::DelaiTotal)]
    );
    assert!(debut.elapsed() < Duration::from_secs(5));

    // Corps envoyé goutte à goutte : un octet toutes les 100 ms, sous la taille annoncée.
    let goutte = simuler_tcp(|mut flux| async move {
        let longueur = DIGICERT.len();
        if flux
            .write_all(&entete_200(&format!("content-length: {longueur}\r\n")))
            .await
            .is_err()
        {
            return;
        }
        for octet in DIGICERT {
            if flux.write_all(&[*octet]).await.is_err() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await;
    let debut = Instant::now();
    assert_eq!(
        echec_unique_avec(
            &client_limite(vec![goutte], limites_courtes()),
            &temoin(0),
            nonce()
        )
        .await,
        EchecAutorite::DelaiTotal
    );
    assert!(debut.elapsed() < Duration::from_secs(5));
}

/// Délai de connexion : un écouteur dont la file d'attente est pleine (il n'accepte jamais) ne
/// répond plus aux nouvelles demandes de connexion ; Linux les ignore (SYN abandonnés), et la
/// connexion reste en attente. Le test vérifie d'abord que la file est bien pleine (une sonde
/// ne se connecte pas) : sinon, il échoue au lieu de conclure à tort.
#[cfg(target_os = "linux")]
#[tokio::test]
async fn delai_de_connexion() {
    let prise = tokio::net::TcpSocket::new_v4().unwrap();
    prise.bind("127.0.0.1:0".parse().unwrap()).unwrap();
    let adresse = prise.local_addr().unwrap();
    let _ecouteur = prise.listen(1).unwrap();
    let mut remplissage = Vec::new();
    let mut pleine = false;
    for _ in 0..64 {
        match tokio::time::timeout(Duration::from_millis(300), TcpStream::connect(adresse)).await {
            Ok(Ok(flux)) => remplissage.push(flux),
            Ok(Err(e)) => panic!("sonde refusée : {e}"),
            Err(_) => {
                pleine = true;
                break;
            }
        }
    }
    assert!(pleine, "file d'attente jamais pleine");
    let limites = Limites {
        connexion: Duration::from_millis(300),
        ..Limites::default()
    };
    let autorite = Autorite::parse(&format!("http://{adresse}")).unwrap();
    assert_eq!(
        echec_unique_avec(&client_limite(vec![autorite], limites), &temoin(0), nonce()).await,
        EchecAutorite::DelaiConnexion
    );
}

#[tokio::test]
async fn reponse_trop_grande_refusee() {
    // Taille annoncée (`Content-Length`) au-delà de la limite : refus avant lecture.
    let annoncee = simuler(valide(&vec![0u8; 100 * 1024])).await;
    assert_eq!(
        echec_unique(annoncee.autorite(), &temoin(0), nonce()).await,
        EchecAutorite::ReponseTropGrande(Limites::default().taille_reponse)
    );

    // `Content-Length` énorme sans aucun octet de corps : refus immédiat, pas `DelaiTotal`.
    let enorme = simuler_tcp(|mut flux| async move {
        let entete = entete_200("content-length: 1000000000000000\r\n");
        if flux.write_all(&entete).await.is_ok() {
            tokio::time::sleep(Duration::from_secs(30)).await;
        }
    })
    .await;
    let debut = Instant::now();
    let limites = Limites {
        total: Duration::from_secs(10),
        ..Limites::default()
    };
    assert_eq!(
        echec_unique_avec(&client_limite(vec![enorme], limites), &temoin(0), nonce()).await,
        EchecAutorite::ReponseTropGrande(Limites::default().taille_reponse)
    );
    assert!(debut.elapsed() < Duration::from_secs(2));

    // Taille annoncée sous la limite (le vrai jeton), mais limite abaissée sous sa taille.
    let simulee = simuler(valide(DIGICERT)).await;
    let limites = Limites {
        taille_reponse: 1024,
        ..Limites::default()
    };
    assert_eq!(
        echec_unique_avec(
            &client_limite(vec![simulee.autorite()], limites),
            &temoin(0),
            nonce()
        )
        .await,
        EchecAutorite::ReponseTropGrande(1024)
    );

    // Corps sans `Content-Length` (par morceaux), 1 Mio : coupé à la limite pendant la lecture.
    let morceaux = simuler_tcp(|mut flux| async move {
        if flux
            .write_all(&entete_200("transfer-encoding: chunked\r\n"))
            .await
            .is_err()
        {
            return;
        }
        let morceau = [b"2000\r\n".as_slice(), &[0u8; 0x2000], b"\r\n"].concat();
        for _ in 0..128 {
            if flux.write_all(&morceau).await.is_err() {
                return;
            }
        }
        let _ = flux.write_all(b"0\r\n\r\n").await;
    })
    .await;
    assert_eq!(
        echec_unique(morceaux, &temoin(0), nonce()).await,
        EchecAutorite::ReponseTropGrande(Limites::default().taille_reponse)
    );
}

#[tokio::test]
async fn redirection_non_suivie() {
    let cible = simuler(valide(DIGICERT)).await;
    let redirige = simuler(Comportement::Rediriger(format!("{}/", cible.url))).await;
    assert_eq!(
        echec_unique(redirige.autorite(), &temoin(0), nonce()).await,
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
        .horodater(&temoin(0), nonce())
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

/// `Debug` d'un échec global : les autorités n'y montrent que leur nom, jamais leur chemin.
#[tokio::test]
async fn debug_sans_chemin() {
    let ferme = port_ferme().await;
    let avec_chemin = Autorite::parse(&format!("http://{}/cle-secrete", ferme.nom())).unwrap();
    let erreur = client(vec![avec_chemin])
        .horodater(&temoin(0), nonce())
        .await
        .unwrap_err();
    let texte = format!("{erreur:?} {erreur}");
    assert!(texte.contains(&ferme.nom()), "{texte}");
    assert!(!texte.contains("cle-secrete"), "{texte}");
}
