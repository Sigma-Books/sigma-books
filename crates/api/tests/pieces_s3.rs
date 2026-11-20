//! Pièces contre un **vrai** service compatible S3 avec verrouillage d'objet (ADR 0015).
//!
//! Fonctionnalité `tests-s3` ; chaque test est `#[ignore]` : `--all-features` (CI, sans service
//! S3) les compte « ignorés » sans les exécuter. Les lancer :
//! `cargo test -p api --features tests-s3 --test pieces_s3 -- --include-ignored`
//! (README du crate, « Pièces »). Sans service ni variables, ils **échouent**.
//!
//! Variables : `SIGMA_TEST_S3_POINT_ACCES` (`http://127.0.0.1:9100`), `SIGMA_TEST_S3_CLE` et
//! `SIGMA_TEST_S3_SECRET` (rôle applicatif : ni suppression de version, ni contournement du
//! verrou), `SIGMA_TEST_S3_CLE_ADMIN` et `SIGMA_TEST_S3_SECRET_ADMIN` (administrateur du service,
//! qui crée les compartiments du test et « altère à la main »), plus celles de `tests-postgres`.
#![cfg(feature = "tests-s3")]
// Fichier de test : les `expect`/`unwrap` y sont voulus.
#![allow(clippy::expect_used, clippy::unwrap_used)]

mod commun;

use std::sync::Arc;

use api::pieces::s3::{ConfigurationS3, StockageS3, VALIDITE_SIGNATURE};
use api::pieces::{CleObjet, ErreurStockage, ObjetADeposer, Pieces, ReglagesPieces, Stockage};
use api::serveur::{Application, Reglages, routeur};
use authz::Autorisation;
use axum::Router;
use axum::body::{Body, Bytes};
use axum::http::{Method, Request, StatusCode, header};
use base64ct::{Base64, Encoding};
use commun::serveur::{
    AuthentificationTest, EN_TETE_UTILISATEUR, ORIGINE, Reponse, appeler, envoyer, lire,
};
use commun::{CabinetTest, creer_cabinet, env, variable};
use http_body_util::{BodyExt, Full};
use hyper_util::rt::TokioIo;
use integrite::Empreinte;
use rusty_s3::actions::{CreateBucket, DeleteObject, GetObject, PutObject};
use rusty_s3::{Bucket, Credentials, S3Action, UrlStyle};
use time::OffsetDateTime;
use url::Url;
use uuid::Uuid;

const PDF: &[u8] = b"%PDF-1.7\n% piece S3\n%%EOF\n";

fn point_acces() -> Url {
    variable("SIGMA_TEST_S3_POINT_ACCES").parse().unwrap()
}

fn identifiants_application() -> Credentials {
    Credentials::new(
        variable("SIGMA_TEST_S3_CLE"),
        variable("SIGMA_TEST_S3_SECRET"),
    )
}

fn identifiants_admin() -> Credentials {
    Credentials::new(
        variable("SIGMA_TEST_S3_CLE_ADMIN"),
        variable("SIGMA_TEST_S3_SECRET_ADMIN"),
    )
}

fn compartiment(nom: &str) -> Bucket {
    Bucket::new(point_acces(), UrlStyle::Path, nom.to_owned(), "us-east-1").unwrap()
}

/// Réponse brute d'un échange d'administration.
struct Brute {
    statut: StatusCode,
    entetes: axum::http::HeaderMap,
    corps: Vec<u8>,
}

/// Échange HTTP/1.1 signé (URL présignée) avec le service, hors de l'application.
async fn echanger(methode: Method, url: &Url, entetes: &[(&str, String)], corps: Vec<u8>) -> Brute {
    let hote = url.host_str().unwrap();
    let port = url.port_or_known_default().unwrap();
    let flux = tokio::net::TcpStream::connect((hote, port))
        .await
        .unwrap_or_else(|e| panic!("service S3 injoignable ({e}) : voir le README du crate"));
    let (mut envoi, connexion) = hyper::client::conn::http1::handshake(TokioIo::new(flux))
        .await
        .unwrap();
    tokio::spawn(connexion);
    let cible = match url.query() {
        Some(q) => format!("{}?{q}", url.path()),
        None => url.path().to_owned(),
    };
    let mut requete = Request::builder()
        .method(methode)
        .uri(cible)
        .header(header::HOST, format!("{hote}:{port}"));
    for (nom, valeur) in entetes {
        requete = requete.header(*nom, valeur);
    }
    let reponse = envoi
        .send_request(requete.body(Full::new(Bytes::from(corps))).unwrap())
        .await
        .unwrap();
    let statut = reponse.status();
    let entetes = reponse.headers().clone();
    let corps = reponse
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes()
        .to_vec();
    Brute {
        statut,
        entetes,
        corps,
    }
}

/// Signe `action` avec ses en-têtes, puis l'envoie.
async fn signer_et_envoyer<'a, A: S3Action<'a>>(
    mut action: A,
    entetes: &'a [(&'a str, String)],
    corps: Vec<u8>,
) -> Brute {
    for (nom, valeur) in entetes {
        action.headers_mut().insert(*nom, valeur.as_str());
    }
    let url = action.sign(VALIDITE_SIGNATURE);
    echanger(A::METHOD.into_methode(), &url, entetes, corps).await
}

trait EnMethode {
    fn into_methode(self) -> Method;
}

impl EnMethode for rusty_s3::Method {
    fn into_methode(self) -> Method {
        match self {
            rusty_s3::Method::Head => Method::HEAD,
            rusty_s3::Method::Get => Method::GET,
            rusty_s3::Method::Post => Method::POST,
            rusty_s3::Method::Put => Method::PUT,
            rusty_s3::Method::Delete => Method::DELETE,
        }
    }
}

/// Crée un compartiment neuf (`sigma-test-…`), avec ou sans verrouillage d'objet.
async fn nouveau_compartiment(verrou: bool) -> String {
    let nom = format!("sigma-test-{}", Uuid::new_v4().simple());
    let admin = identifiants_admin();
    let bucket = compartiment(&nom);
    let entetes = if verrou {
        vec![("x-amz-bucket-object-lock-enabled", "true".to_owned())]
    } else {
        vec![]
    };
    let r = signer_et_envoyer(CreateBucket::new(&bucket, &admin), &entetes, Vec::new()).await;
    assert_eq!(
        r.statut,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&r.corps)
    );
    nom
}

fn stockage(nom: &str, identifiants: Credentials) -> StockageS3 {
    StockageS3::new(ConfigurationS3 {
        point_acces: point_acces(),
        compartiment: nom.to_owned(),
        region: "us-east-1".to_owned(),
        style_chemin: true,
        identifiants,
    })
    .unwrap()
}

/// Dépose `contenu` sous une clé neuve ; rend la clé et la version.
async fn deposer_objet(s: &StockageS3, contenu: &[u8]) -> (CleObjet, String) {
    let cle = CleObjet::nouvelle(
        api::base::IdCabinet::nouveau(),
        api::base::IdDossier::nouveau(),
    );
    let depot = s
        .deposer(ObjetADeposer {
            cle: &cle,
            contenu,
            empreinte: Empreinte::calculer(contenu),
            type_contenu: "application/pdf",
            retenir_jusqu_au: api::pieces::ajouter_annees(OffsetDateTime::now_utc(), 10)
                + time::Duration::SECOND,
        })
        .await
        .unwrap();
    (cle, depot.version)
}

/// Rétention de la version (`?retention`), lue par l'administrateur : (mode, fin).
async fn retention(nom: &str, cle: &CleObjet, version: &str) -> String {
    let admin = identifiants_admin();
    let bucket = compartiment(nom);
    let mut action = GetObject::new(&bucket, Some(&admin), cle.as_str());
    action.query_mut().insert("retention", "");
    action.query_mut().insert("versionId", version);
    let r = signer_et_envoyer(action, &[], Vec::new()).await;
    assert_eq!(r.statut, StatusCode::OK);
    String::from_utf8(r.corps).unwrap()
}

/// Corps XML de `PutObjectRetention`, avec sa somme SHA-256.
fn corps_retention(fin: &str) -> (Vec<u8>, String) {
    let corps = format!(
        "<Retention xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\"><Mode>GOVERNANCE</Mode>\
         <RetainUntilDate>{fin}</RetainUntilDate></Retention>"
    )
    .into_bytes();
    let somme = Base64::encode_string(Empreinte::calculer(&corps).octets());
    (corps, somme)
}

#[tokio::test]
#[ignore = "service S3 avec verrouillage d'objet exigé : -- --include-ignored (README)"]
async fn le_demarrage_exige_versionnement_et_verrou() {
    let avec = nouveau_compartiment(true).await;
    assert_eq!(
        stockage(&avec, identifiants_application())
            .verifier_worm()
            .await,
        Ok(())
    );
    // Compartiment ordinaire : ni versionnement, ni verrou.
    let sans = nouveau_compartiment(false).await;
    assert_eq!(
        stockage(&sans, identifiants_admin()).verifier_worm().await,
        Err(ErreurStockage::SansVersionnement)
    );
    // Versionné mais sans verrou.
    let admin = identifiants_admin();
    let bucket = compartiment(&sans);
    let mut action = CreateBucket::new(&bucket, &admin);
    action.query_mut().insert("versioning", "");
    let corps = b"<VersioningConfiguration xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\">\
                  <Status>Enabled</Status></VersioningConfiguration>"
        .to_vec();
    let somme = Base64::encode_string(Empreinte::calculer(&corps).octets());
    let r = signer_et_envoyer(action, &[("x-amz-checksum-sha256", somme)], corps).await;
    assert_eq!(
        r.statut,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&r.corps)
    );
    assert_eq!(
        stockage(&sans, identifiants_admin()).verifier_worm().await,
        Err(ErreurStockage::SansVerrouillage)
    );
    // Service injoignable.
    let injoignable = StockageS3::new(ConfigurationS3 {
        point_acces: "http://127.0.0.1:1".parse().unwrap(),
        compartiment: avec,
        region: "us-east-1".to_owned(),
        style_chemin: true,
        identifiants: identifiants_application(),
    })
    .unwrap();
    assert_eq!(
        injoignable.verifier_worm().await,
        Err(ErreurStockage::Injoignable)
    );
}

#[tokio::test]
#[ignore = "service S3 avec verrouillage d'objet exigé : -- --include-ignored (README)"]
async fn le_depot_pose_une_retention_de_gouvernance_de_dix_ans() {
    let nom = nouveau_compartiment(true).await;
    let s = stockage(&nom, identifiants_application());
    let avant = OffsetDateTime::now_utc();
    let (cle, version) = deposer_objet(&s, PDF).await;
    assert_eq!(s.lire(&cle, &version, PDF.len()).await.unwrap(), PDF);
    let texte = retention(&nom, &cle, &version).await;
    assert!(texte.contains("<Mode>GOVERNANCE</Mode>"), "{texte}");
    let fin = texte
        .split("<RetainUntilDate>")
        .nth(1)
        .and_then(|r| r.split('T').next())
        .unwrap()
        .to_owned();
    let annee: i32 = fin[..4].parse().unwrap();
    assert!(annee >= avant.year() + 10, "{texte}");
    // Lecture bornée : un objet plus grand qu'attendu est refusé.
    assert_eq!(
        s.lire(&cle, &version, PDF.len() - 1).await,
        Err(ErreurStockage::TropGrand)
    );
    // Clé déjà écrite : jamais écrasée (`If-None-Match: *`).
    let refus = s
        .deposer(ObjetADeposer {
            cle: &cle,
            contenu: b"%PDF-autre",
            empreinte: Empreinte::calculer(b"%PDF-autre"),
            type_contenu: "application/pdf",
            retenir_jusqu_au: OffsetDateTime::now_utc() + time::Duration::days(3653),
        })
        .await;
    assert!(
        matches!(refus, Err(ErreurStockage::Refus(412))),
        "{refus:?}"
    );
    // Somme annoncée fausse : le service refuse l'objet.
    let autre = CleObjet::nouvelle(
        api::base::IdCabinet::nouveau(),
        api::base::IdDossier::nouveau(),
    );
    let refus = s
        .deposer(ObjetADeposer {
            cle: &autre,
            contenu: PDF,
            empreinte: Empreinte::calculer(b"autre contenu"),
            type_contenu: "application/pdf",
            retenir_jusqu_au: OffsetDateTime::now_utc() + time::Duration::days(3653),
        })
        .await;
    assert!(
        matches!(refus, Err(ErreurStockage::Refus(400))),
        "{refus:?}"
    );
}

/// Refus pour cause de verrou : 400 (`InvalidRequest`, « WORM protected », `MinIO`) ou 403
/// (`AccessDenied`, AWS), jamais un succès.
fn refuse_par_le_verrou(r: &Brute) {
    let texte = String::from_utf8_lossy(&r.corps);
    assert!(
        matches!(r.statut, StatusCode::BAD_REQUEST | StatusCode::FORBIDDEN)
            && (texte.contains("WORM") || texte.contains("AccessDenied")),
        "{} : {texte}",
        r.statut
    );
}

#[tokio::test]
#[ignore = "service S3 avec verrouillage d'objet exigé : -- --include-ignored (README)"]
async fn le_verrou_refuse_suppression_et_raccourcissement() {
    let nom = nouveau_compartiment(true).await;
    let s = stockage(&nom, identifiants_application());
    let (cle, version) = deposer_objet(&s, PDF).await;
    let bucket = compartiment(&nom);
    for identifiants in [identifiants_application(), identifiants_admin()] {
        // Suppression de la version : refusée, même à l'administrateur sans contournement
        // explicite du mode gouvernance.
        let mut action = DeleteObject::new(&bucket, Some(&identifiants), cle.as_str());
        action.query_mut().insert("versionId", version.as_str());
        let r = signer_et_envoyer(action, &[], Vec::new()).await;
        refuse_par_le_verrou(&r);
        // Raccourcir la rétention : refusé.
        let (corps, somme) = corps_retention("2027-01-01T00:00:00Z");
        let mut action = PutObject::new(&bucket, Some(&identifiants), cle.as_str());
        action.query_mut().insert("retention", "");
        action.query_mut().insert("versionId", version.as_str());
        let r = signer_et_envoyer(action, &[("x-amz-checksum-sha256", somme)], corps).await;
        refuse_par_le_verrou(&r);
        assert_eq!(s.lire(&cle, &version, PDF.len()).await.unwrap(), PDF);
    }
    // La prolonger : admis (le verrou ne peut que s'allonger).
    let (corps, somme) = corps_retention("2060-01-01T00:00:00Z");
    let app = identifiants_application();
    let mut action = PutObject::new(&bucket, Some(&app), cle.as_str());
    action.query_mut().insert("retention", "");
    action.query_mut().insert("versionId", version.as_str());
    let r = signer_et_envoyer(action, &[("x-amz-checksum-sha256", somme)], corps).await;
    assert_eq!(
        r.statut,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&r.corps)
    );
    assert_eq!(s.lire(&cle, &version, PDF.len()).await.unwrap(), PDF);
}

// --- Par l'API -------------------------------------------------------------------------------

fn application_s3(env: &commun::Env, nom: &str) -> Router {
    let etat = Application::new(
        env.application.clone(),
        env.lecture_audit.clone(),
        Arc::new(Autorisation::charger().unwrap()),
        Arc::new(AuthentificationTest::default()),
        Reglages::new(ORIGINE.to_owned()),
    );
    etat.brancher_pieces(Pieces::new(
        Arc::new(stockage(nom, identifiants_application())),
        ReglagesPieces::default(),
    ))
    .unwrap();
    routeur(&etat)
}

fn base(c: &CabinetTest) -> String {
    format!("/api/cabinets/{}/dossiers/{}/pieces", c.cabinet, c.dossier)
}

async fn deposer_api(routeur: &Router, c: &CabinetTest, octets: &[u8]) -> Reponse {
    let requete = Request::post(format!("{}?nom=releve.pdf", base(c)))
        .header("origin", ORIGINE)
        .header("sec-fetch-site", "same-origin")
        .header("x-sigma-csrf", "1")
        .header(EN_TETE_UTILISATEUR, c.collaborateur.to_string())
        .body(Body::from(octets.to_vec()))
        .unwrap();
    appeler(routeur, requete).await
}

async fn contenu(routeur: &Router, c: &CabinetTest, id: &str) -> Reponse {
    lire(
        routeur,
        &format!("{}/{id}/contenu", base(c)),
        Some(c.collaborateur),
    )
    .await
}

/// Clé et version de la pièce, lues en base par le super-utilisateur.
async fn objet_de(env: &commun::Env, id: &str) -> (String, String) {
    sqlx::query_as("SELECT cle_objet, version_objet FROM piece WHERE id = $1")
        .bind(Uuid::parse_str(id).unwrap())
        .fetch_one(&env.admin)
        .await
        .unwrap()
}

#[tokio::test]
#[ignore = "service S3 avec verrouillage d'objet exigé : -- --include-ignored (README)"]
async fn depot_et_telechargement_par_l_api() {
    let env = env().await;
    let c = creer_cabinet(&env.admin, "Cabinet S3").await;
    let nom = nouveau_compartiment(true).await;
    let routeur = application_s3(&env, &nom);
    let r = deposer_api(&routeur, &c, PDF).await;
    assert_eq!(r.statut, StatusCode::CREATED, "{}", r.texte());
    let id = r.json()["id"].as_str().unwrap().to_owned();
    let (cle, version) = objet_de(&env, &id).await;
    // Clé `<cabinet>/<dossier>/<UUID v4>` : ni le nom ni l'empreinte.
    assert!(
        cle.starts_with(&format!("{}/{}/", c.cabinet, c.dossier)),
        "{cle}"
    );
    assert!(!cle.contains("releve"));
    assert!(!cle.contains(&Empreinte::calculer(PDF).to_string()));
    let t = contenu(&routeur, &c, &id).await;
    assert_eq!(t.statut, StatusCode::OK, "{}", t.texte());
    assert_eq!(t.corps, PDF);
    assert_eq!(
        t.entetes[header::CONTENT_DISPOSITION],
        "attachment; filename=\"releve.pdf\"; filename*=UTF-8''releve.pdf"
    );
    // Pièce interne, par le même stockage.
    let r = envoyer(
        &routeur,
        &format!("{}/internes", base(&c)),
        Some(c.collaborateur),
        r#"{"date":"2026-11-30","motif":"Note S3"}"#,
        None,
    )
    .await;
    assert_eq!(r.statut, StatusCode::CREATED, "{}", r.texte());
    let interne = r.json()["id"].as_str().unwrap().to_owned();
    let t = contenu(&routeur, &c, &interne).await;
    assert!(t.texte().starts_with("sigma-books piece-interne v1\n"));
    let texte = retention(
        &nom,
        &CleObjet::relue(&cle, c.cabinet, c.dossier).unwrap(),
        &version,
    )
    .await;
    assert!(texte.contains("<Mode>GOVERNANCE</Mode>"));
}

#[tokio::test]
#[ignore = "service S3 avec verrouillage d'objet exigé : -- --include-ignored (README)"]
async fn un_objet_altere_par_l_administrateur_est_detecte() {
    let env = env().await;
    let c = creer_cabinet(&env.admin, "Cabinet S3").await;
    let nom = nouveau_compartiment(true).await;
    let routeur = application_s3(&env, &nom);
    let r = deposer_api(&routeur, &c, PDF).await;
    assert_eq!(r.statut, StatusCode::CREATED, "{}", r.texte());
    let id = r.json()["id"].as_str().unwrap().to_owned();
    let (cle, version) = objet_de(&env, &id).await;
    let bucket = compartiment(&nom);
    let admin = identifiants_admin();

    // 1. L'administrateur réécrit la clé : une nouvelle version, jamais servie (lecture de la
    //    version écrite au dépôt).
    let altere = b"%PDF-1.7\n% faux\n".to_vec();
    let r = signer_et_envoyer(
        PutObject::new(&bucket, Some(&admin), &cle),
        &[],
        altere.clone(),
    )
    .await;
    assert_eq!(r.statut, StatusCode::OK);
    let nouvelle = r.entetes["x-amz-version-id"].to_str().unwrap().to_owned();
    assert_ne!(nouvelle, version);
    assert_eq!(contenu(&routeur, &c, &id).await.corps, PDF);

    // 2. Avec en plus les droits du super-utilisateur de la base, il fait pointer la pièce sur
    //    sa version : l'empreinte recalculée diffère, le téléchargement est refusé et l'alerte
    //    est inscrite à l'audit.
    let mut tx = env.admin.begin().await.unwrap();
    sqlx::query("SET LOCAL session_replication_role = replica")
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query("UPDATE piece SET version_objet = $2 WHERE id = $1")
        .bind(Uuid::parse_str(&id).unwrap())
        .bind(&nouvelle)
        .execute(&mut *tx)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    let r = contenu(&routeur, &c, &id).await;
    assert_eq!(r.statut, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(r.texte(), r#"{"erreur":"erreur_interne"}"#);
    let derniere: String = sqlx::query_scalar(
        "SELECT contenu FROM audit_dossier WHERE cabinet_id = $1 AND dossier_id = $2 \
         ORDER BY rang DESC LIMIT 1",
    )
    .bind(c.cabinet.uuid())
    .bind(c.dossier.uuid())
    .fetch_one(&env.admin)
    .await
    .unwrap();
    assert!(
        derniere.contains("donnee=alerte\nvaleur=piece_alteree\n"),
        "{derniere}"
    );
    assert!(derniere.contains(&format!(
        "donnee=empreinte_lue\nvaleur={}\n",
        Empreinte::calculer(&altere)
    )));

    // 3. Il supprime la version d'origine en contournant le mode gouvernance (droit que le rôle
    //    applicatif n'a pas) et y repointe la pièce : objet absent, refus et alerte.
    let mut action = DeleteObject::new(&bucket, Some(&admin), &cle);
    action.query_mut().insert("versionId", version.as_str());
    let r = signer_et_envoyer(
        action,
        &[("x-amz-bypass-governance-retention", "true".to_owned())],
        Vec::new(),
    )
    .await;
    assert_eq!(
        r.statut,
        StatusCode::NO_CONTENT,
        "{}",
        String::from_utf8_lossy(&r.corps)
    );
    let mut tx = env.admin.begin().await.unwrap();
    sqlx::query("SET LOCAL session_replication_role = replica")
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query("UPDATE piece SET version_objet = $2 WHERE id = $1")
        .bind(Uuid::parse_str(&id).unwrap())
        .bind(&version)
        .execute(&mut *tx)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    let r = contenu(&routeur, &c, &id).await;
    assert_eq!(r.statut, StatusCode::INTERNAL_SERVER_ERROR);
}
