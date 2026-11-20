//! Pièces justificatives par l'API, contre PostgreSQL, avec un stockage **en mémoire**
//! (ADR 0015 ; L2 PR 12). Les propriétés propres au stockage S3 (verrou, rétention, version)
//! sont éprouvées contre un vrai service dans `tests/pieces_s3.rs`.
//!
//! Couvert ici : type par signature, limite de taille pendant la réception du flux, empreinte
//! annoncée, en-têtes du téléchargement, empreinte recalculée à la lecture (objet altéré ou
//! absent : refus, alerte, entrée d'audit), pièce d'autrui = 404 identique, doublon signalé
//! dans le seul dossier, pièce interne relue, écart, « Cedar se trompe », droits de la table,
//! fonctions d'accès sous un rôle qui contourne la RLS.
#![cfg(feature = "tests-postgres")]
// Fichier de test : les `expect`/`unwrap` y sont voulus.
#![allow(clippy::expect_used, clippy::unwrap_used)]

mod commun;

use std::sync::Arc;

use api::base::{ContexteTenant, IdDossier, IdUtilisateur, ouvrir_transaction};
use api::pieces::base::{lire_piece, pieces_de_meme_empreinte};
use api::pieces::interne::NoteOd;
use api::pieces::{Pieces, ReglagesPieces};
use api::serveur::{Application, Etat, Reglages, routeur};
use authz::Autorisation;
use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use commun::pieces::StockageMemoire;
use commun::serveur::{
    AuthentificationTest, EN_TETE_UTILISATEUR, ORIGINE, Reponse, ToujoursAutorise, appeler,
    envoyer, lire,
};
use commun::{CabinetTest, ajouter_affecte, code_sql, creer_cabinet, env};
use integrite::Empreinte;
use sqlx::Executor;
use sqlx::postgres::{PgPool, PgPoolOptions};
use uuid::Uuid;

const PDF: &[u8] = b"%PDF-1.7\n1 0 obj << >> endobj\n%%EOF\n";

/// Application de test (authentification de test) au décideur et aux réglages donnés, avec un
/// stockage en mémoire neuf.
fn application(
    env: &commun::Env,
    decideur: Arc<dyn api::serveur::Decideur>,
    reglages: ReglagesPieces,
) -> (Router, StockageMemoire, Etat) {
    let etat = Application::new(
        env.application.clone(),
        env.lecture_audit.clone(),
        decideur,
        Arc::new(AuthentificationTest::default()),
        Reglages::new(ORIGINE.to_owned()),
    );
    let stockage = StockageMemoire::default();
    etat.brancher_pieces(Pieces::new(Arc::new(stockage.clone()), reglages))
        .unwrap();
    (routeur(&etat), stockage, etat)
}

fn cedar(env: &commun::Env) -> (Router, StockageMemoire, Etat) {
    application(
        env,
        Arc::new(Autorisation::charger().unwrap()),
        ReglagesPieces::default(),
    )
}

fn base(c: &CabinetTest, dossier: IdDossier) -> String {
    format!("/api/cabinets/{}/dossiers/{}/pieces", c.cabinet, dossier)
}

/// Dépôt d'un fichier (corps brut), avec les paramètres `parametres`.
async fn deposer_avec(
    routeur: &Router,
    chemin: &str,
    utilisateur: IdUtilisateur,
    parametres: &str,
    corps: Body,
) -> Reponse {
    let uri = if parametres.is_empty() {
        chemin.to_owned()
    } else {
        format!("{chemin}?{parametres}")
    };
    let requete = Request::post(uri)
        .header("origin", ORIGINE)
        .header("sec-fetch-site", "same-origin")
        .header("x-sigma-csrf", "1")
        .header("content-type", "application/octet-stream")
        .header(EN_TETE_UTILISATEUR, utilisateur.to_string())
        .body(corps)
        .unwrap();
    appeler(routeur, requete).await
}

async fn deposer(
    routeur: &Router,
    c: &CabinetTest,
    dossier: IdDossier,
    utilisateur: IdUtilisateur,
    parametres: &str,
    octets: &[u8],
) -> Reponse {
    deposer_avec(
        routeur,
        &base(c, dossier),
        utilisateur,
        parametres,
        Body::from(octets.to_vec()),
    )
    .await
}

/// Identifiant d'une pièce créée (201).
fn cree(r: &Reponse) -> String {
    assert_eq!(r.statut, StatusCode::CREATED, "{}", r.texte());
    r.json()["id"].as_str().unwrap().to_owned()
}

async fn contenu(
    routeur: &Router,
    c: &CabinetTest,
    dossier: IdDossier,
    piece: &str,
    utilisateur: IdUtilisateur,
) -> Reponse {
    lire(
        routeur,
        &format!("{}/{piece}/contenu", base(c, dossier)),
        Some(utilisateur),
    )
    .await
}

/// Contenus des entrées d'audit d'un dossier, dans l'ordre.
async fn audit_du_dossier(admin: &PgPool, c: &CabinetTest, dossier: IdDossier) -> Vec<String> {
    sqlx::query_scalar(
        "SELECT contenu FROM audit_dossier WHERE cabinet_id = $1 AND dossier_id = $2 ORDER BY rang",
    )
    .bind(c.cabinet.uuid())
    .bind(dossier.uuid())
    .fetch_all(admin)
    .await
    .unwrap()
}

async fn compter_pieces(admin: &PgPool, c: &CabinetTest) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM piece WHERE cabinet_id = $1")
        .bind(c.cabinet.uuid())
        .fetch_one(admin)
        .await
        .unwrap()
}

#[tokio::test]
async fn le_type_vient_de_la_signature_seule() {
    let env = env().await;
    let c = creer_cabinet(&env.admin, "Cabinet").await;
    let (routeur, stockage, _) = cedar(&env);
    // Un exécutable nommé `.pdf`, un HTML, un SVG : refusés (415), rien n'est stocké.
    for (nom, octets) in [
        ("facture.pdf", &b"MZ\x90\0\x03\0\0\0\x04\0\0\0\xff\xff"[..]),
        ("facture.pdf", b"\x7fELF\x02\x01\x01\0"),
        ("page.html", b"<html><script>alert(1)</script>%PDF-1.7"),
        ("image.png", b"<svg xmlns=\"http://www.w3.org/2000/svg\"/>"),
    ] {
        let r = deposer(
            &routeur,
            &c,
            c.dossier,
            c.collaborateur,
            &format!("nom={nom}"),
            octets,
        )
        .await;
        assert_eq!(r.statut, StatusCode::UNSUPPORTED_MEDIA_TYPE, "{nom}");
        assert_eq!(r.texte(), r#"{"erreur":"type_non_admis"}"#);
    }
    assert_eq!(stockage.nombre(), 0);
    assert_eq!(compter_pieces(&env.admin, &c).await, 0);
    // Un PDF nommé `.png` est un PDF ; un JPEG et un PNG sont reconnus.
    for (nom, octets, attendu) in [
        ("photo.png", PDF, "application/pdf"),
        (
            "photo.jpg",
            &b"\xFF\xD8\xFF\xE0\0\x10JFIF\0"[..],
            "image/jpeg",
        ),
        ("scan", b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR", "image/png"),
    ] {
        let r = deposer(
            &routeur,
            &c,
            c.dossier,
            c.collaborateur,
            &format!("nom={nom}"),
            octets,
        )
        .await;
        cree(&r);
        assert_eq!(r.json()["type"], attendu, "{nom}");
        assert_eq!(r.json()["nom"], nom);
        assert_eq!(
            r.json()["empreinte"],
            Empreinte::calculer(octets).to_string()
        );
        assert_eq!(r.json()["taille"], octets.len());
    }
    // Corps vide : 400.
    let r = deposer(&routeur, &c, c.dossier, c.collaborateur, "", b"").await;
    assert_eq!(r.statut, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn la_limite_de_taille_s_applique_pendant_la_reception() {
    let env = env().await;
    let c = creer_cabinet(&env.admin, "Cabinet").await;
    let reglages = ReglagesPieces {
        taille_max: 4096,
        ..ReglagesPieces::default()
    };
    let (routeur, stockage, etat) =
        application(&env, Arc::new(Autorisation::charger().unwrap()), reglages);
    let mut gros = PDF.to_vec();
    gros.resize(4097, b'a');
    // Longueur annoncée au-delà de la limite : 413.
    let r = deposer(&routeur, &c, c.dossier, c.collaborateur, "", &gros).await;
    assert_eq!(r.statut, StatusCode::PAYLOAD_TOO_LARGE);
    // Flux sans longueur annoncée, par morceaux, sur une vraie connexion : coupé pendant la
    // réception (413).
    let reponse = envoyer_par_morceaux(&etat, &base(&c, c.dossier), c.collaborateur, &gros).await;
    assert!(reponse.starts_with("HTTP/1.1 413"), "{reponse}");
    assert_eq!(stockage.nombre(), 0);
    assert_eq!(compter_pieces(&env.admin, &c).await, 0);
    // À la limite exacte : admis.
    gros.truncate(4096);
    cree(&deposer(&routeur, &c, c.dossier, c.collaborateur, "", &gros).await);

    // Avec les réglages par défaut, le dépôt dépasse la limite commune de 64 Kio ; les autres
    // routes la gardent.
    let (routeur, _, _) = cedar(&env);
    let mut moyen = PDF.to_vec();
    moyen.resize(200 * 1024, b'b');
    let id = cree(&deposer(&routeur, &c, c.dossier, c.collaborateur, "", &moyen).await);
    let mut motif = String::from(r#"{"motif":""#);
    motif.push_str(&"m".repeat(100 * 1024));
    motif.push_str(r#""}"#);
    let r = envoyer(
        &routeur,
        &format!("{}/{id}/ecart", base(&c, c.dossier)),
        Some(c.collaborateur),
        &motif,
        None,
    )
    .await;
    assert_eq!(r.statut, StatusCode::PAYLOAD_TOO_LARGE);
}

/// Envoie un dépôt par morceaux (`Transfer-Encoding: chunked`, sans `Content-Length`) à un
/// serveur réel ; rend la réponse brute.
async fn envoyer_par_morceaux(
    etat: &Etat,
    chemin: &str,
    utilisateur: IdUtilisateur,
    octets: &[u8],
) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let reglages = etat.reglages().clone();
    let routeur = routeur(etat);
    let ecouteur = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let adresse = ecouteur.local_addr().unwrap();
    let (arreter, arret) = tokio::sync::oneshot::channel::<()>();
    let serveur = tokio::spawn(async move {
        api::serveur::servir(ecouteur, routeur, &reglages, async {
            let _ = arret.await;
        })
        .await
    });
    let mut flux = tokio::net::TcpStream::connect(adresse).await.unwrap();
    let mut requete = format!(
        "POST {chemin} HTTP/1.1\r\nHost: x\r\norigin: {ORIGINE}\r\nsec-fetch-site: same-origin\r\n\
         x-sigma-csrf: 1\r\n{EN_TETE_UTILISATEUR}: {utilisateur}\r\n\
         Transfer-Encoding: chunked\r\nConnection: close\r\n\r\n"
    )
    .into_bytes();
    for morceau in octets.chunks(512) {
        requete.extend_from_slice(format!("{:x}\r\n", morceau.len()).as_bytes());
        requete.extend_from_slice(morceau);
        requete.extend_from_slice(b"\r\n");
    }
    requete.extend_from_slice(b"0\r\n\r\n");
    // Le serveur peut fermer avant la fin de l'envoi : l'erreur d'écriture est attendue.
    let _ = flux.write_all(&requete).await;
    let mut reponse = Vec::new();
    let _ = flux.read_to_end(&mut reponse).await;
    arreter.send(()).unwrap();
    serveur.await.unwrap().unwrap();
    String::from_utf8_lossy(&reponse).into_owned()
}

#[tokio::test]
async fn l_empreinte_annoncee_doit_etre_celle_du_serveur() {
    let env = env().await;
    let c = creer_cabinet(&env.admin, "Cabinet").await;
    let (routeur, stockage, _) = cedar(&env);
    let juste = Empreinte::calculer(PDF).to_string();
    let fausse = Empreinte::calculer(b"autre").to_string();
    for parametre in [format!("empreinte={fausse}"), "empreinte=zz".to_owned()] {
        let r = deposer(&routeur, &c, c.dossier, c.collaborateur, &parametre, PDF).await;
        assert_eq!(r.statut, StatusCode::BAD_REQUEST, "{parametre}");
    }
    assert_eq!(stockage.nombre(), 0);
    let r = deposer(
        &routeur,
        &c,
        c.dossier,
        c.collaborateur,
        &format!("empreinte={}", juste.to_uppercase()),
        PDF,
    )
    .await;
    cree(&r);
    assert_eq!(r.json()["empreinte"], juste);
}

#[tokio::test]
async fn le_telechargement_est_une_piece_jointe_isolee() {
    let env = env().await;
    let c = creer_cabinet(&env.admin, "Cabinet").await;
    let auditeur = ajouter_affecte(&env.admin, &c, "auditeur").await;
    let (routeur, _, _) = cedar(&env);
    let id = cree(
        &deposer(
            &routeur,
            &c,
            c.dossier,
            c.collaborateur,
            "nom=Facture%20n%C2%B01%20%22mars%22.pdf",
            PDF,
        )
        .await,
    );
    let r = contenu(&routeur, &c, c.dossier, &id, auditeur).await;
    assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
    assert_eq!(r.corps, PDF);
    assert_eq!(r.entetes[header::CONTENT_TYPE], "application/pdf");
    assert_eq!(
        r.entetes[header::CONTENT_DISPOSITION],
        "attachment; filename=\"Facture n_1 _mars_.pdf\"; \
         filename*=UTF-8''Facture%20n%C2%B01%20%22mars%22.pdf"
    );
    assert_eq!(r.entetes[header::X_CONTENT_TYPE_OPTIONS], "nosniff");
    let csp = r.entetes[header::CONTENT_SECURITY_POLICY].to_str().unwrap();
    assert!(csp.starts_with("sandbox; default-src 'none'"), "{csp}");
    assert_eq!(r.entetes[header::CACHE_CONTROL], "no-store");
}

#[tokio::test]
async fn l_empreinte_est_recalculee_a_chaque_lecture() {
    let env = env().await;
    let c = creer_cabinet(&env.admin, "Cabinet").await;
    let (routeur, stockage, _) = cedar(&env);
    let id = cree(&deposer(&routeur, &c, c.dossier, c.collaborateur, "", PDF).await);
    let cle: String = sqlx::query_scalar("SELECT cle_objet FROM piece WHERE id = $1")
        .bind(Uuid::parse_str(&id).unwrap())
        .fetch_one(&env.admin)
        .await
        .unwrap();
    assert_eq!(
        contenu(&routeur, &c, c.dossier, &id, c.collaborateur)
            .await
            .statut,
        StatusCode::OK
    );
    let avant = audit_du_dossier(&env.admin, &c, c.dossier).await.len();

    // Objet altéré « à la main » : refus, erreur uniforme, alerte inscrite à l'audit.
    assert_eq!(stockage.alterer(&cle, b"%PDF-1.7\n% altere\n"), 1);
    let r = contenu(&routeur, &c, c.dossier, &id, c.collaborateur).await;
    assert_eq!(r.statut, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(r.texte(), r#"{"erreur":"erreur_interne"}"#);
    assert!(!r.corps.windows(4).any(|w| w == b"%PDF"));
    let audit = audit_du_dossier(&env.admin, &c, c.dossier).await;
    assert_eq!(audit.len(), avant + 1);
    let alerte = audit.last().unwrap();
    assert!(alerte.contains("action=ConsulterEditions\n"), "{alerte}");
    assert!(
        alerte.contains(&format!("ressource=piece:{id}\n")),
        "{alerte}"
    );
    assert!(
        alerte.contains("donnee=alerte\nvaleur=piece_alteree\n"),
        "{alerte}"
    );
    assert!(
        alerte.contains(&format!(
            "donnee=empreinte_attendue\nvaleur={}\n",
            Empreinte::calculer(PDF)
        )),
        "{alerte}"
    );

    // Objet disparu : même refus, alerte « piece_absente ».
    stockage.supprimer(&cle);
    let r = contenu(&routeur, &c, c.dossier, &id, c.collaborateur).await;
    assert_eq!(r.statut, StatusCode::INTERNAL_SERVER_ERROR);
    let audit = audit_du_dossier(&env.admin, &c, c.dossier).await;
    assert!(
        audit
            .last()
            .unwrap()
            .contains("donnee=alerte\nvaleur=piece_absente\n")
    );
}

#[tokio::test]
async fn la_piece_d_autrui_est_absente() {
    let env = env().await;
    let a = creer_cabinet(&env.admin, "A").await;
    let b = creer_cabinet(&env.admin, "B").await;
    let (routeur, _, _) = cedar(&env);
    let dans_a = cree(&deposer(&routeur, &a, a.dossier, a.administrateur, "", PDF).await);
    let autre_dossier =
        cree(&deposer(&routeur, &a, a.autre_dossier, a.administrateur, "", PDF).await);
    let dans_b = cree(&deposer(&routeur, &b, b.dossier, b.administrateur, "", PDF).await);

    let reference = contenu(
        &routeur,
        &a,
        a.dossier,
        &Uuid::new_v4().to_string(),
        a.administrateur,
    )
    .await;
    assert_eq!(reference.statut, StatusCode::NOT_FOUND);
    // Pièce d'un autre dossier du même cabinet, ou d'un autre cabinet, citée sous le dossier
    // de A : même 404, octet pour octet, en lecture comme en écart.
    for id in [&autre_dossier, &dans_b] {
        let r = contenu(&routeur, &a, a.dossier, id, a.administrateur).await;
        assert_eq!(r.statut, StatusCode::NOT_FOUND);
        assert_eq!(r.corps, reference.corps);
        let r = envoyer(
            &routeur,
            &format!("{}/{id}/ecart", base(&a, a.dossier)),
            Some(a.administrateur),
            r#"{"motif":"essai"}"#,
            None,
        )
        .await;
        assert_eq!(r.statut, StatusCode::NOT_FOUND);
        assert_eq!(r.corps, reference.corps);
    }
    // La pièce de B sous le cabinet B, par un membre de A seulement : 404 (cabinet d'autrui).
    let r = contenu(&routeur, &b, b.dossier, &dans_b, a.administrateur).await;
    assert_eq!(r.statut, StatusCode::NOT_FOUND);
    assert_eq!(r.corps, reference.corps);
    // Le collaborateur de A, non affecté au second dossier, n'en voit rien.
    let r = contenu(
        &routeur,
        &a,
        a.autre_dossier,
        &autre_dossier,
        a.collaborateur,
    )
    .await;
    assert_eq!(r.statut, StatusCode::NOT_FOUND);
    assert_eq!(r.corps, reference.corps);
    // Chaque liste ne montre que son dossier.
    let r = lire(&routeur, &base(&a, a.dossier), Some(a.collaborateur)).await;
    assert_eq!(r.json()["pieces"].as_array().unwrap().len(), 1);
    assert_eq!(r.json()["pieces"][0]["id"], dans_a.as_str());
}

#[tokio::test]
async fn le_doublon_n_est_signale_que_dans_le_meme_dossier() {
    let env = env().await;
    let a = creer_cabinet(&env.admin, "A").await;
    let b = creer_cabinet(&env.admin, "B").await;
    let (routeur, stockage, _) = cedar(&env);
    let premier = cree(&deposer(&routeur, &a, a.dossier, a.collaborateur, "", PDF).await);
    // Second dépôt du même fichier dans le même dossier : avertissement, rien de stocké.
    let r = deposer(&routeur, &a, a.dossier, a.collaborateur, "", PDF).await;
    assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
    assert_eq!(r.json()["doublon"], true);
    assert_eq!(r.json()["pieces"][0]["id"], premier.as_str());
    assert_eq!(stockage.nombre(), 1);
    let audit = audit_du_dossier(&env.admin, &a, a.dossier).await;
    assert!(
        audit
            .last()
            .unwrap()
            .contains("donnee=issue\nvaleur=doublon_signale\n")
    );
    // Confirmation explicite : ajout journalisé avec la pièce dont il double.
    let r = deposer(
        &routeur,
        &a,
        a.dossier,
        a.collaborateur,
        "doublon=confirme",
        PDF,
    )
    .await;
    let second = cree(&r);
    assert_ne!(second, premier);
    let audit = audit_du_dossier(&env.admin, &a, a.dossier).await;
    assert!(
        audit
            .last()
            .unwrap()
            .contains(&format!("donnee=doublon_confirme_de\nvaleur={premier}\n"))
    );
    // Autre dossier du cabinet, autre cabinet : aucun avertissement (F3.5).
    cree(&deposer(&routeur, &a, a.autre_dossier, a.administrateur, "", PDF).await);
    cree(&deposer(&routeur, &b, b.dossier, b.administrateur, "", PDF).await);
    // Une pièce écartée ne compte plus comme doublon.
    for id in [&premier, &second] {
        let r = envoyer(
            &routeur,
            &format!("{}/{id}/ecart", base(&a, a.dossier)),
            Some(a.collaborateur),
            r#"{"motif":"Dépôt en double"}"#,
            None,
        )
        .await;
        assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
    }
    cree(&deposer(&routeur, &a, a.dossier, a.collaborateur, "", PDF).await);
}

#[tokio::test]
async fn la_piece_interne_est_stockee_sous_sa_forme_canonique() {
    let env = env().await;
    let c = creer_cabinet(&env.admin, "Cabinet").await;
    let (routeur, _, _) = cedar(&env);
    let r = envoyer(
        &routeur,
        &format!("{}/internes", base(&c, c.dossier)),
        Some(c.collaborateur),
        r#"{"date":"2026-11-30","motif":"Dotation aux amortissements","lignes":["Matériel, 5 ans",""]}"#,
        None,
    )
    .await;
    let id = cree(&r);
    assert_eq!(r.json()["nature"], "interne");
    assert_eq!(r.json()["type"], "text/plain; charset=utf-8");
    assert_eq!(r.json()["nom"], "note-od-2026-11-30.txt");
    let t = contenu(&routeur, &c, c.dossier, &id, c.collaborateur).await;
    assert_eq!(t.statut, StatusCode::OK);
    let texte = t.texte();
    let note = NoteOd::lire(&texte).expect("forme canonique v1");
    assert_eq!(note.motif(), "Dotation aux amortissements");
    assert!(texte.starts_with("sigma-books piece-interne v1\nnature=note-od\n"));
    assert!(texte.contains(&format!("\nauteur={}\n", c.collaborateur)));
    assert!(
        texte.ends_with("ligne=Matériel, 5 ans\nligne=\nfin\n"),
        "{texte}"
    );
    assert_eq!(
        r.json()["empreinte"],
        Empreinte::calculer(&t.corps).to_string()
    );
    let audit = audit_du_dossier(&env.admin, &c, c.dossier).await;
    assert!(
        audit
            .last()
            .unwrap()
            .contains("motif=Dotation aux amortissements\n")
    );
    // Champs refusés : date invalide, motif blanc, champ inconnu.
    for corps in [
        r#"{"date":"2026-02-30","motif":"x"}"#,
        r#"{"date":"2026-11-30","motif":"  "}"#,
        r#"{"date":"2026-11-30","motif":"x","montant":1}"#,
        r#"{"date":"2026-11-30","motif":"x","lignes":["a\u0007"]}"#,
    ] {
        let r = envoyer(
            &routeur,
            &format!("{}/internes", base(&c, c.dossier)),
            Some(c.collaborateur),
            corps,
            None,
        )
        .await;
        assert_eq!(r.statut, StatusCode::BAD_REQUEST, "{corps}");
    }
}

#[tokio::test]
async fn l_ecart_est_definitif_et_journalise() {
    let env = env().await;
    let c = creer_cabinet(&env.admin, "Cabinet").await;
    let (routeur, _, _) = cedar(&env);
    let id = cree(&deposer(&routeur, &c, c.dossier, c.collaborateur, "", PDF).await);
    let ecart = format!("{}/{id}/ecart", base(&c, c.dossier));
    for corps in [r#"{"motif":"  "}"#, "{}", r#"{"motif":"a","x":1}"#] {
        let r = envoyer(&routeur, &ecart, Some(c.collaborateur), corps, None).await;
        assert_eq!(r.statut, StatusCode::BAD_REQUEST, "{corps}");
    }
    let r = envoyer(
        &routeur,
        &ecart,
        Some(c.collaborateur),
        r#"{"motif":"Dépôt erroné"}"#,
        None,
    )
    .await;
    assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
    assert_eq!(r.json()["ecartee"], true);
    let audit = audit_du_dossier(&env.admin, &c, c.dossier).await;
    let entree = audit.last().unwrap();
    assert!(entree.contains("motif=Dépôt erroné\n"), "{entree}");
    assert!(
        entree.contains("champ=ecartee\navant=non\napres=oui\n"),
        "{entree}"
    );
    // Hors des listes, encore téléchargeable ; un second écart ne trouve rien.
    let liste = lire(&routeur, &base(&c, c.dossier), Some(c.collaborateur)).await;
    assert_eq!(liste.json()["pieces"], serde_json::json!([]));
    assert_eq!(
        contenu(&routeur, &c, c.dossier, &id, c.collaborateur)
            .await
            .statut,
        StatusCode::OK
    );
    let r = envoyer(
        &routeur,
        &ecart,
        Some(c.collaborateur),
        r#"{"motif":"Encore"}"#,
        None,
    )
    .await;
    assert_eq!(r.statut, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn la_liste_est_paginee() {
    let env = env().await;
    let c = creer_cabinet(&env.admin, "Cabinet").await;
    let (routeur, _, _) = cedar(&env);
    let mut ids = Vec::new();
    for i in 0..3u8 {
        let mut octets = PDF.to_vec();
        octets.push(i);
        ids.push(cree(
            &deposer(&routeur, &c, c.dossier, c.collaborateur, "", &octets).await,
        ));
    }
    let page = lire(
        &routeur,
        &format!("{}?limite=2", base(&c, c.dossier)),
        Some(c.collaborateur),
    )
    .await;
    assert_eq!(page.json()["pieces"].as_array().unwrap().len(), 2);
    assert_eq!(page.json()["pieces"][0]["id"], ids[2].as_str());
    let suivant = page.json()["suivant"].as_str().unwrap().to_owned();
    assert_eq!(suivant, ids[1]);
    let page = lire(
        &routeur,
        &format!("{}?limite=2&apres={suivant}", base(&c, c.dossier)),
        Some(c.collaborateur),
    )
    .await;
    assert_eq!(page.json()["pieces"][0]["id"], ids[0].as_str());
    assert_eq!(page.json()["suivant"], serde_json::Value::Null);
    for parametres in ["limite=0", "limite=101", "limite=x", "apres=x", "autre=1"] {
        let r = lire(
            &routeur,
            &format!("{}?{parametres}", base(&c, c.dossier)),
            Some(c.collaborateur),
        )
        .await;
        assert_eq!(r.statut, StatusCode::BAD_REQUEST, "{parametres}");
    }
}

#[tokio::test]
async fn la_requete_tient_quand_cedar_se_trompe() {
    let env = env().await;
    let a = creer_cabinet(&env.admin, "A").await;
    let b = creer_cabinet(&env.admin, "B").await;
    let auditeur = ajouter_affecte(&env.admin, &a, "auditeur").await;
    let (routeur, stockage, _) =
        application(&env, Arc::new(ToujoursAutorise), ReglagesPieces::default());
    let reference = contenu(
        &routeur,
        &a,
        a.dossier,
        &Uuid::new_v4().to_string(),
        a.administrateur,
    )
    .await;
    let dans_a = cree(&deposer(&routeur, &a, a.dossier, a.administrateur, "", PDF).await);
    let autre = cree(
        &deposer(
            &routeur,
            &a,
            a.autre_dossier,
            a.administrateur,
            "",
            b"%PDF-x",
        )
        .await,
    );
    let dans_b = cree(&deposer(&routeur, &b, b.dossier, b.administrateur, "", PDF).await);
    let objets = stockage.nombre();
    // L'auditeur, autorisé à tort par Cedar, ne dépose ni n'écarte : rôle lu en base (403).
    let r = deposer(&routeur, &a, a.dossier, auditeur, "doublon=confirme", PDF).await;
    assert_eq!(r.statut, StatusCode::FORBIDDEN);
    let r = envoyer(
        &routeur,
        &format!("{}/{dans_a}/ecart", base(&a, a.dossier)),
        Some(auditeur),
        r#"{"motif":"x"}"#,
        None,
    )
    .await;
    assert_eq!(r.statut, StatusCode::FORBIDDEN);
    let r = envoyer(
        &routeur,
        &format!("{}/internes", base(&a, a.dossier)),
        Some(auditeur),
        r#"{"date":"2026-11-30","motif":"x"}"#,
        None,
    )
    .await;
    assert_eq!(r.statut, StatusCode::FORBIDDEN);
    assert_eq!(stockage.nombre(), objets);
    // Le collaborateur, non affecté au second dossier : ni liste, ni contenu, ni dépôt (404).
    let r = lire(&routeur, &base(&a, a.autre_dossier), Some(a.collaborateur)).await;
    assert_eq!(r.corps, reference.corps);
    let r = contenu(&routeur, &a, a.autre_dossier, &autre, a.collaborateur).await;
    assert_eq!(r.corps, reference.corps);
    let r = deposer(
        &routeur,
        &a,
        a.autre_dossier,
        a.collaborateur,
        "",
        b"%PDF-y",
    )
    .await;
    assert_eq!(r.statut, StatusCode::NOT_FOUND);
    assert_eq!(r.corps, reference.corps);
    // La pièce de B, citée sous un dossier de A : la requête et la RLS ne la trouvent pas.
    let r = contenu(&routeur, &a, a.dossier, &dans_b, a.administrateur).await;
    assert_eq!(r.statut, StatusCode::NOT_FOUND);
    assert_eq!(r.corps, reference.corps);
    assert_eq!(stockage.nombre(), objets);
}

/// Pool du super-utilisateur dont chaque connexion prend un rôle qui **contourne la RLS**
/// (créé ici, jamais par une migration, comme dans `tests/requete.rs`).
async fn pool_contournement(admin: &PgPool) -> PgPool {
    admin
        .execute(
            "DO $$ BEGIN \
                 CREATE ROLE sigma_test_contournement NOLOGIN BYPASSRLS; \
             EXCEPTION WHEN duplicate_object OR unique_violation THEN NULL; \
             END $$",
        )
        .await
        .unwrap();
    admin
        .execute("GRANT sigma_application TO sigma_test_contournement")
        .await
        .unwrap();
    PgPoolOptions::new()
        .max_connections(2)
        .after_connect(|connexion, _| {
            Box::pin(async move {
                connexion
                    .execute("SET ROLE sigma_test_contournement")
                    .await?;
                Ok(())
            })
        })
        .connect(&commun::variable("SIGMA_TEST_ADMIN_URL"))
        .await
        .unwrap()
}

#[tokio::test]
async fn les_fonctions_d_acces_filtrent_seules_par_dossier() {
    let env = env().await;
    let a = creer_cabinet(&env.admin, "A").await;
    let b = creer_cabinet(&env.admin, "B").await;
    let (routeur, _, _) = cedar(&env);
    let dans_a = cree(&deposer(&routeur, &a, a.dossier, a.administrateur, "", PDF).await);
    let autre = cree(&deposer(&routeur, &a, a.autre_dossier, a.administrateur, "", PDF).await);
    let dans_b = cree(&deposer(&routeur, &b, b.dossier, b.administrateur, "", PDF).await);
    let pool = pool_contournement(&env.admin).await;
    let mut tx = ouvrir_transaction(
        &pool,
        ContexteTenant::dossier(a.cabinet, a.dossier, a.administrateur),
    )
    .await
    .unwrap();
    // Le rôle voit bien tout : sans le filtre de la requête, B serait lisible.
    let toutes: i64 = sqlx::query_scalar("SELECT count(*) FROM piece WHERE id = ANY($1)")
        .bind(
            [&dans_a, &autre, &dans_b]
                .iter()
                .map(|i| Uuid::parse_str(i).unwrap())
                .collect::<Vec<_>>(),
        )
        .fetch_one(tx.connexion())
        .await
        .unwrap();
    assert_eq!(toutes, 3, "le rôle de test ne contourne pas la RLS");
    let id = |t: &str| Uuid::parse_str(t).unwrap();
    assert!(lire_piece(&mut tx, id(&dans_a)).await.unwrap().is_some());
    assert_eq!(lire_piece(&mut tx, id(&autre)).await.unwrap(), None);
    assert_eq!(lire_piece(&mut tx, id(&dans_b)).await.unwrap(), None);
    let memes = pieces_de_meme_empreinte(&mut tx, &Empreinte::calculer(PDF))
        .await
        .unwrap();
    assert_eq!(
        memes.iter().map(|p| p.id).collect::<Vec<_>>(),
        vec![id(&dans_a)]
    );
    tx.annuler().await.unwrap();
}

#[tokio::test]
async fn le_role_applicatif_ne_modifie_que_l_ecart() {
    let env = env().await;
    let c = creer_cabinet(&env.admin, "Cabinet").await;
    let (routeur, _, _) = cedar(&env);
    let id = Uuid::parse_str(&cree(
        &deposer(&routeur, &c, c.dossier, c.collaborateur, "", PDF).await,
    ))
    .unwrap();
    let mut tx = ouvrir_transaction(
        &env.application,
        ContexteTenant::dossier(c.cabinet, c.dossier, c.collaborateur),
    )
    .await
    .unwrap();
    for sql in [
        "UPDATE piece SET empreinte = '\\x00' WHERE id = $1",
        "UPDATE piece SET version_objet = 'autre' WHERE id = $1",
        "UPDATE piece SET retenue_jusqu_au = now() WHERE id = $1",
        "DELETE FROM piece WHERE id = $1",
    ] {
        let erreur = sqlx::query(sql)
            .bind(id)
            .execute(tx.connexion())
            .await
            .unwrap_err();
        assert_eq!(code_sql(&erreur).as_deref(), Some("42501"), "{sql}");
        tx.annuler().await.unwrap();
        tx = ouvrir_transaction(
            &env.application,
            ContexteTenant::dossier(c.cabinet, c.dossier, c.collaborateur),
        )
        .await
        .unwrap();
    }
    // L'écart passe ; le « rétablir » ensuite est refusé par le déclencheur.
    sqlx::query(
        "UPDATE piece SET ecartee_le = now(), ecartee_par = $2, motif_ecart = 'm' WHERE id = $1",
    )
    .bind(id)
    .bind(c.collaborateur.uuid())
    .execute(tx.connexion())
    .await
    .unwrap();
    tx.confirmer().await.unwrap();
    let mut tx = ouvrir_transaction(
        &env.application,
        ContexteTenant::dossier(c.cabinet, c.dossier, c.collaborateur),
    )
    .await
    .unwrap();
    let erreur = sqlx::query(
        "UPDATE piece SET ecartee_le = NULL, ecartee_par = NULL, motif_ecart = NULL WHERE id = $1",
    )
    .bind(id)
    .execute(tx.connexion())
    .await
    .unwrap_err();
    assert_eq!(code_sql(&erreur).as_deref(), Some("23514"));
    tx.annuler().await.unwrap();
    // Même le propriétaire des tables ne change pas une pièce : déclencheur.
    let erreur = sqlx::query("UPDATE piece SET taille = taille + 1 WHERE id = $1")
        .bind(id)
        .execute(&env.admin)
        .await
        .unwrap_err();
    assert_eq!(code_sql(&erreur).as_deref(), Some("23514"));
}

/// Stockage en mémoire **lent** au dépôt : élargit la fenêtre entre la recherche des doublons
/// et l'inscription de la pièce.
struct StockageLent(StockageMemoire);

impl api::pieces::Stockage for StockageLent {
    fn deposer<'a>(
        &'a self,
        objet: api::pieces::ObjetADeposer<'a>,
    ) -> api::pieces::stockage::Futur<'a, Result<api::pieces::Depot, api::pieces::ErreurStockage>>
    {
        Box::pin(async move {
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
            self.0.deposer(objet).await
        })
    }

    fn lire<'a>(
        &'a self,
        cle: &'a api::pieces::CleObjet,
        version: &'a str,
        taille_max: usize,
    ) -> api::pieces::stockage::Futur<'a, Result<Vec<u8>, api::pieces::ErreurStockage>> {
        self.0.lire(cle, version, taille_max)
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn deux_depots_concurrents_du_meme_fichier_un_seul_sans_avertissement() {
    let env = env().await;
    let c = creer_cabinet(&env.admin, "Cabinet").await;
    let valideur = ajouter_affecte(&env.admin, &c, "valideur").await;
    let etat = Application::new(
        env.application.clone(),
        env.lecture_audit.clone(),
        Arc::new(Autorisation::charger().unwrap()),
        Arc::new(AuthentificationTest::default()),
        Reglages::new(ORIGINE.to_owned()),
    );
    let stockage = StockageMemoire::default();
    etat.brancher_pieces(Pieces::new(
        Arc::new(StockageLent(stockage.clone())),
        ReglagesPieces::default(),
    ))
    .unwrap();
    let routeur = routeur(&etat);
    for tour in 0..3u8 {
        let mut octets = PDF.to_vec();
        octets.push(tour);
        let (r1, r2) = tokio::join!(
            deposer(&routeur, &c, c.dossier, c.collaborateur, "", &octets),
            deposer(&routeur, &c, c.dossier, valideur, "", &octets),
        );
        let mut statuts = [r1.statut, r2.statut];
        statuts.sort();
        assert_eq!(
            statuts,
            [StatusCode::OK, StatusCode::CREATED],
            "{} / {}",
            r1.texte(),
            r2.texte()
        );
        let avertissement = if r1.statut == StatusCode::OK {
            &r1
        } else {
            &r2
        };
        assert_eq!(avertissement.json()["doublon"], true);
    }
    assert_eq!(stockage.nombre(), 3);
    assert_eq!(compter_pieces(&env.admin, &c).await, 3);
}

#[tokio::test]
async fn le_nom_servi_porte_l_extension_du_type_reconnu() {
    let env = env().await;
    let c = creer_cabinet(&env.admin, "Cabinet").await;
    let (routeur, _, _) = cedar(&env);
    for (nom, servi) in [
        ("facture.cmd", "facture.cmd.pdf"),
        ("facture.hta", "facture.hta.pdf"),
        ("Facture.PDF", "Facture.PDF"),
    ] {
        let mut octets = PDF.to_vec();
        octets.extend_from_slice(nom.as_bytes());
        let id = cree(
            &deposer(
                &routeur,
                &c,
                c.dossier,
                c.collaborateur,
                &format!("nom={nom}"),
                &octets,
            )
            .await,
        );
        let r = contenu(&routeur, &c, c.dossier, &id, c.collaborateur).await;
        assert_eq!(
            r.entetes[header::CONTENT_DISPOSITION],
            format!("attachment; filename=\"{servi}\"; filename*=UTF-8''{servi}").as_str()
        );
    }
    // Caractères de format (U+202E, inversion d'affichage) et séparateurs : refusés au dépôt.
    for nom in [
        "facture%E2%80%AEfdp.cmd",
        "a%E2%80%A8b.pdf",
        "a%E2%80%A9b.pdf",
        "a%EF%BB%BFb.pdf",
    ] {
        let r = deposer(
            &routeur,
            &c,
            c.dossier,
            c.collaborateur,
            &format!("nom={nom}"),
            PDF,
        )
        .await;
        assert_eq!(r.statut, StatusCode::BAD_REQUEST, "{nom}");
    }
}
