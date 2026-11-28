//! Relevés importés, partie 1/2 (L3-22, PR 12a de `docs/lots/L3.md` ; ADR 0034 points 4 à 6) :
//! profils versionnés en ajout seul, dépôt du CSV comme pièce « importé, non certifié », analyse
//! (aperçu) **sans aucune écriture**, limites, refus par rôle, audit sans valeur, RLS et ajout seul.
//!
//! Cas de référence : les quatre fichiers synthétiques de `crates/formats/tests/releves/`, déposés
//! et analysés par l'API, rendent les lignes, montants et alertes de leur fichier `.attendu`.
#![cfg(feature = "tests-postgres")]
// Fichier de test : les `expect`/`unwrap` y sont voulus.
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::too_many_lines)]

mod commun;

use api::base::{ContexteTenant, IdUtilisateur, ouvrir_transaction};
use api::serveur::{Reglages, routeur};
use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use commun::audit_valeurs::verifier_sans_valeur_metier;
use commun::banc::{Banc, banc, code};
use commun::code_sql;
use commun::releves::{corps_profil, releve};
use commun::serveur::{
    EN_TETE_UTILISATEUR, ORIGINE, Reponse, appeler, application_base, envoyer, lire,
};
use formats::releve::{ProfilBrut, ProfilReleve};
use serde_json::{Value, json};
use uuid::Uuid;

fn base(b: &Banc) -> String {
    format!(
        "/api/cabinets/{}/dossiers/{}/releves",
        b.c.cabinet, b.c.dossier
    )
}

/// Caisse du dossier du banc (journal de caisse par défaut et son compte).
async fn caisse(b: &Banc) -> Uuid {
    commun::tresorerie::doter(&b.env.admin, b.c.cabinet, b.c.dossier, b.c.administrateur).await
}

/// Dépôt brut (octets quelconques) sur le compte de trésorerie `compte`.
async fn deposer_sur(
    routeur: &Router,
    b: &Banc,
    u: IdUtilisateur,
    compte: Uuid,
    octets: &[u8],
    parametres: &str,
) -> Reponse {
    let requete = Request::post(format!("{}/comptes/{compte}/depots{parametres}", base(b)))
        .header("origin", ORIGINE)
        .header("sec-fetch-site", "same-origin")
        .header("x-sigma-csrf", "1")
        .header("content-type", "text/csv")
        .header(EN_TETE_UTILISATEUR, u.to_string())
        .body(Body::from(octets.to_vec()))
        .unwrap();
    appeler(routeur, requete).await
}

async fn deposer(b: &Banc, u: IdUtilisateur, compte: Uuid, octets: &[u8]) -> Reponse {
    deposer_sur(&b.routeur, b, u, compte, octets, "").await
}

async fn creer_profil(b: &Banc, corps: &str) -> Value {
    let r = envoyer(
        &b.routeur,
        &format!("{}/profils", base(b)),
        Some(b.valideur),
        corps,
        None,
    )
    .await;
    assert_eq!(r.statut, StatusCode::CREATED, "{}", r.texte());
    r.json()
}

async fn analyser(b: &Banc, u: IdUtilisateur, depot: &str, profil: &str, p: &str) -> Reponse {
    lire(
        &b.routeur,
        &format!("{}/depots/{depot}/analyse/{profil}{p}", base(b)),
        Some(u),
    )
    .await
}

async fn compter(b: &Banc, table: &str) -> i64 {
    sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "SELECT count(*) FROM {table} WHERE cabinet_id = $1"
    )))
    .bind(b.c.cabinet.uuid())
    .fetch_one(&b.env.admin)
    .await
    .unwrap()
}

/// Lignes du cabinet du banc dans toutes les tables que ces routes pourraient toucher.
async fn etat_de_la_base(b: &Banc) -> Vec<i64> {
    let mut v = Vec::new();
    for table in [
        "piece",
        "depot_releve",
        "profil_releve",
        "audit_dossier",
        "brouillon",
    ] {
        v.push(compter(b, table).await);
    }
    v
}

/// Corps de profil JSON tiré d'un profil TOML de `formats` (fichiers de référence).
fn corps_depuis_toml(nom: &str, toml: &str) -> String {
    let brut = ProfilBrut::from(ProfilReleve::depuis_toml(toml).unwrap());
    json!({"nom": nom, "profil": brut}).to_string()
}

#[tokio::test]
async fn profil_versionne_en_ajout_seul() {
    let banc_de_test = banc().await;
    let b = &banc_de_test;
    let v1 = creer_profil(b, &corps_profil("Banque A")).await;
    let id = v1["id"].as_str().unwrap().to_owned();
    assert_eq!(v1["version"], 1);
    assert_eq!(v1["profil"]["encodage"], "utf-8");
    let ligne_v1: (String, Vec<u8>) = sqlx::query_as(
        "SELECT definition, empreinte FROM profil_releve WHERE id = $1 AND version = 1",
    )
    .bind(Uuid::parse_str(&id).unwrap())
    .fetch_one(&b.env.admin)
    .await
    .unwrap();

    // Nouvelle version : autre séparateur et autre nom ; la version 1 ne change pas.
    let corps_v2 =
        corps_profil("Banque A (2026)").replace(r#""delimiteur":";""#, r#""delimiteur":",""#);
    let r = envoyer(
        &b.routeur,
        &format!("{}/profils/{id}/versions", base(b)),
        Some(b.c.administrateur),
        &corps_v2,
        None,
    )
    .await;
    assert_eq!(r.statut, StatusCode::CREATED, "{}", r.texte());
    assert_eq!(r.json()["id"], id.as_str());
    assert_eq!(r.json()["version"], 2);
    assert_ne!(r.json()["empreinte"], v1["empreinte"]);

    let lu = lire(
        &b.routeur,
        &format!("{}/profils/{id}?version=1", base(b)),
        Some(b.auditeur),
    )
    .await;
    assert_eq!(lu.statut, StatusCode::OK, "{}", lu.texte());
    assert_eq!(lu.json()["profil"], v1["profil"]);
    assert_eq!(lu.json()["nom"], "Banque A");
    assert_eq!(lu.json()["empreinte"], v1["empreinte"]);
    assert_eq!(lu.json()["versions"].as_array().unwrap().len(), 2);
    let dernier = lire(
        &b.routeur,
        &format!("{}/profils/{id}", base(b)),
        Some(b.auditeur),
    )
    .await;
    assert_eq!(dernier.json()["version"], 2);
    assert_eq!(dernier.json()["profil"]["delimiteur"], ",");
    let liste = lire(
        &b.routeur,
        &format!("{}/profils", base(b)),
        Some(b.c.collaborateur),
    )
    .await;
    assert_eq!(liste.json()["profils"].as_array().unwrap().len(), 1);
    assert_eq!(liste.json()["profils"][0]["version"], 2);
    assert_eq!(liste.json()["suivant"], Value::Null);
    let apres: (String, Vec<u8>) = sqlx::query_as(
        "SELECT definition, empreinte FROM profil_releve WHERE id = $1 AND version = 1",
    )
    .bind(Uuid::parse_str(&id).unwrap())
    .fetch_one(&b.env.admin)
    .await
    .unwrap();
    assert_eq!(apres, ligne_v1);
    let absente = lire(
        &b.routeur,
        &format!("{}/profils/{id}?version=3", base(b)),
        Some(b.auditeur),
    )
    .await;
    assert_eq!(absente.statut, StatusCode::NOT_FOUND);

    // Refus : rôles (collaborateur, auditeur : 403), profil invalide (422, code stable).
    for u in [b.c.collaborateur, b.auditeur] {
        let r = envoyer(
            &b.routeur,
            &format!("{}/profils", base(b)),
            Some(u),
            &corps_profil("X"),
            None,
        )
        .await;
        assert_eq!(r.statut, StatusCode::FORBIDDEN);
    }
    for (corps, attendu) in [
        (
            corps_profil("X").replace("%d/%m/%Y", "%d/%m"),
            "releve_profil_date_sans_annee",
        ),
        (
            corps_profil("X").replace("utf-8", "utf-16"),
            "releve_profil_encodage",
        ),
        (
            corps_profil("X").replace(r#""montant":"Montant""#, r#""debit":"Débit""#),
            "releve_profil_colonnes_montant",
        ),
        (
            corps_profil("X").replace(r#""lignes_entete":1,"#, ""),
            "releve_profil_titre",
        ),
        (corps_profil(""), "releve_profil_nom"),
    ] {
        let r = envoyer(
            &b.routeur,
            &format!("{}/profils", base(b)),
            Some(b.valideur),
            &corps,
            None,
        )
        .await;
        assert_eq!(r.statut, StatusCode::UNPROCESSABLE_ENTITY, "{corps}");
        assert_eq!(code(&r), attendu, "{corps}");
    }
    // Ajout seul : le rôle applicatif ne modifie ni ne supprime une version (42501).
    for requete in [
        "UPDATE profil_releve SET nom = 'Autre'",
        "DELETE FROM profil_releve",
    ] {
        let mut tx = ouvrir_transaction(
            &b.env.application,
            ContexteTenant::cabinet(b.c.cabinet, b.c.administrateur),
        )
        .await
        .unwrap();
        tx.restreindre_au_dossier(b.c.dossier).await.unwrap();
        let e = sqlx::query(requete)
            .execute(tx.connexion())
            .await
            .unwrap_err();
        assert_eq!(code_sql(&e).as_deref(), Some("42501"), "{requete}");
    }
}

#[tokio::test]
async fn nombre_de_profils_borne() {
    let banc_de_test = banc().await;
    let b = &banc_de_test;
    let mut reglages = Reglages::new(ORIGINE.to_owned());
    reglages.releves.profils_max = 2;
    reglages.releves.versions_max = 2;
    let routeur = routeur(&application_base(&b.env, reglages));
    let creer = |nom: &'static str| {
        let routeur = routeur.clone();
        let chemin = format!("{}/profils", base(b));
        let u = b.valideur;
        async move { envoyer(&routeur, &chemin, Some(u), &corps_profil(nom), None).await }
    };
    assert_eq!(creer("P1").await.statut, StatusCode::CREATED);
    let p2 = creer("P2").await;
    assert_eq!(p2.statut, StatusCode::CREATED);
    let r = creer("P3").await;
    assert_eq!(r.statut, StatusCode::CONFLICT);
    assert_eq!(
        r.json(),
        json!({"erreur": "releve_profils_max", "nombre": 2})
    );
    // Une nouvelle version n'est pas un nouveau profil.
    let r = envoyer(
        &routeur,
        &format!(
            "{}/profils/{}/versions",
            base(b),
            p2.json()["id"].as_str().unwrap()
        ),
        Some(b.valideur),
        &corps_profil("P2 bis"),
        None,
    )
    .await;
    assert_eq!(r.statut, StatusCode::CREATED, "{}", r.texte());
    // Versions d'un profil bornées (409 typé, avec la borne) ; la version 2 reste la dernière.
    let r = envoyer(
        &routeur,
        &format!(
            "{}/profils/{}/versions",
            base(b),
            p2.json()["id"].as_str().unwrap()
        ),
        Some(b.valideur),
        &corps_profil("P2 ter"),
        None,
    )
    .await;
    assert_eq!(r.statut, StatusCode::CONFLICT);
    assert_eq!(
        r.json(),
        json!({"erreur": "releve_profil_versions_max", "nombre": 2})
    );
    // Liste paginée par nom : P1, puis P2 bis (dernière version de P2).
    let page = lire(
        &routeur,
        &format!("{}/profils?limite=1", base(b)),
        Some(b.auditeur),
    )
    .await;
    assert_eq!(page.statut, StatusCode::OK, "{}", page.texte());
    let page = page.json();
    assert_eq!(page["profils"].as_array().unwrap().len(), 1);
    assert_eq!(page["profils"][0]["nom"], "P1");
    let suivant = page["suivant"].as_str().unwrap();
    let page = lire(
        &routeur,
        &format!("{}/profils?limite=1&apres={suivant}", base(b)),
        Some(b.auditeur),
    )
    .await
    .json();
    assert_eq!(page["profils"][0]["nom"], "P2 bis");
    assert_eq!(page["profils"][0]["version"], 2);
    for p in ["?limite=0", "?limite=101", "?apres=x"] {
        let r = lire(
            &routeur,
            &format!("{}/profils{p}", base(b)),
            Some(b.auditeur),
        )
        .await;
        assert_eq!(r.statut, StatusCode::BAD_REQUEST, "{p}");
    }
}

#[tokio::test]
async fn depot_comme_piece_importee_non_certifiee() {
    let banc_de_test = banc().await;
    let b = &banc_de_test;
    let compte = caisse(b).await;
    let fichier = releve("dépôt");
    let r = deposer_sur(
        &b.routeur,
        b,
        b.c.collaborateur,
        compte,
        fichier.as_bytes(),
        "?nom=releve-SN0000000001.csv",
    )
    .await;
    assert_eq!(r.statut, StatusCode::CREATED, "{}", r.texte());
    let depot = r.json();
    assert_eq!(depot["statut_probant"], "importe_non_certifie");
    assert_eq!(depot["compte_tresorerie"], compte.to_string());
    assert_eq!(
        depot["empreinte"],
        integrite::Empreinte::calculer(fichier.as_bytes()).to_string()
    );
    // La pièce : externe, type texte ; téléchargée en pièce jointe, jamais rendue.
    let (nature, type_contenu): (String, String) =
        sqlx::query_as("SELECT nature, type_contenu FROM piece WHERE id = $1")
            .bind(Uuid::parse_str(depot["piece"].as_str().unwrap()).unwrap())
            .fetch_one(&b.env.admin)
            .await
            .unwrap();
    assert_eq!(
        (nature.as_str(), type_contenu.as_str()),
        ("externe", "text/csv")
    );
    let t = lire(
        &b.routeur,
        &format!(
            "/api/cabinets/{}/dossiers/{}/pieces/{}/contenu",
            b.c.cabinet,
            b.c.dossier,
            depot["piece"].as_str().unwrap()
        ),
        Some(b.auditeur),
    )
    .await;
    assert_eq!(t.statut, StatusCode::OK);
    assert_eq!(t.corps, fichier.as_bytes());
    assert_eq!(t.entetes["content-type"], "application/octet-stream");
    assert_eq!(t.entetes["x-content-type-options"], "nosniff");
    assert!(
        t.entetes["content-disposition"]
            .to_str()
            .unwrap()
            .starts_with("attachment;")
    );
    // Lecture du dépôt et liste.
    let id = depot["id"].as_str().unwrap();
    let lu = lire(
        &b.routeur,
        &format!("{}/depots/{id}", base(b)),
        Some(b.auditeur),
    )
    .await;
    assert_eq!(lu.json()["nom"], "releve-SN0000000001.csv");
    let liste = lire(
        &b.routeur,
        &format!("{}/depots?compte={compte}", base(b)),
        Some(b.auditeur),
    )
    .await;
    assert_eq!(liste.json().as_array().unwrap().len(), 1);

    // Même fichier : doublon signalé, rien de stocké (ni pièce, ni dépôt).
    let avant = etat_de_la_base(b).await;
    let r = deposer(b, b.c.administrateur, compte, fichier.as_bytes()).await;
    assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
    assert_eq!(r.json()["doublon"], true);
    assert_eq!(r.json()["depots"][0]["id"], id);
    let apres = etat_de_la_base(b).await;
    assert_eq!(apres[..3], avant[..3]);
    assert_eq!(apres[3], avant[3] + 1, "doublon signalé au journal d'audit");

    // Audit : identifiants, empreinte, taille ; jamais le nom du fichier, un libellé ni un
    // montant du relevé.
    let entrees: Vec<String> =
        sqlx::query_scalar("SELECT contenu FROM audit_dossier WHERE dossier_id = $1 ORDER BY rang")
            .bind(b.c.dossier.uuid())
            .fetch_all(&b.env.admin)
            .await
            .unwrap();
    let depot_audit: Vec<&String> = entrees
        .iter()
        .filter(|e| e.contains("ressource=depot_releve:") || e.contains("ressource=piece:"))
        .collect();
    assert_eq!(depot_audit.len(), 3);
    for e in &depot_audit {
        verifier_sans_valeur_metier(
            e,
            &["SN0000000001", "Versement", "FRAIS", "Virement"],
            &["150000", "150000.00", "-2500", "12345.5"],
        );
    }
    assert!(depot_audit.iter().any(|e| e.contains("statut_probant")));
}

#[tokio::test]
async fn depots_refuses() {
    let banc_de_test = banc().await;
    let b = &banc_de_test;
    let compte = caisse(b).await;
    let avant = etat_de_la_base(b).await;
    // Rôle : l'auditeur ne dépose pas (403).
    let r = deposer(b, b.auditeur, compte, releve("a").as_bytes()).await;
    assert_eq!(r.statut, StatusCode::FORBIDDEN);
    // Types : signature (PDF), octet nul, caractère de contrôle, binaire : 415 ; vide : 400.
    for (octets, statut) in [
        (
            &b"%PDF-1.7\nDate;Montant\n"[..],
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
        ),
        (
            b"Date;Montant\n01/01/2026;\x001\n",
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
        ),
        (b"Date;Montant\x1b[2J\n", StatusCode::UNSUPPORTED_MEDIA_TYPE),
        (b"\x81\x8d\x8f\x90\x9d", StatusCode::UNSUPPORTED_MEDIA_TYPE),
        (b"", StatusCode::BAD_REQUEST),
    ] {
        let r = deposer(b, b.c.administrateur, compte, octets).await;
        assert_eq!(r.statut, statut, "{octets:?} : {}", r.texte());
    }
    // Empreinte annoncée différente : 400.
    let r = deposer_sur(
        &b.routeur,
        b,
        b.c.administrateur,
        compte,
        releve("b").as_bytes(),
        &format!("?empreinte={}", "0".repeat(64)),
    )
    .await;
    assert_eq!(r.statut, StatusCode::BAD_REQUEST);
    // Compte inexistant (ou d'un autre dossier) : 404 ; compte archivé : 409.
    let r = deposer(
        b,
        b.c.administrateur,
        Uuid::new_v4(),
        releve("c").as_bytes(),
    )
    .await;
    assert_eq!(r.statut, StatusCode::NOT_FOUND);
    sqlx::query("UPDATE compte_tresorerie SET archive = true WHERE id = $1")
        .bind(compte)
        .execute(&b.env.admin)
        .await
        .unwrap();
    let r = deposer(b, b.c.administrateur, compte, releve("d").as_bytes()).await;
    assert_eq!(r.statut, StatusCode::CONFLICT);
    assert_eq!(code(&r), "compte_tresorerie_archive");
    assert_eq!(etat_de_la_base(b).await, avant, "aucun refus n'écrit");
}

#[tokio::test]
async fn depot_trop_gros_refuse_pendant_la_reception() {
    let banc_de_test = banc().await;
    let b = &banc_de_test;
    let compte = caisse(b).await;
    let mut reglages = Reglages::new(ORIGINE.to_owned());
    reglages.releves.limites.taille_fichier = 1024;
    reglages.releves.limites.longueur_ligne = 64;
    let routeur = routeur(&application_base(&b.env, reglages));
    let avant = etat_de_la_base(b).await;
    // Corps plus grand que la limite, sans longueur annoncée : coupé à la lecture (413).
    let gros = format!("Date;Montant\n{}", "01/01/2026;1\n".repeat(200));
    assert!(gros.len() > 1024);
    let r = deposer_sur(&routeur, b, b.c.administrateur, compte, gros.as_bytes(), "").await;
    assert_eq!(r.statut, StatusCode::PAYLOAD_TOO_LARGE, "{}", r.texte());
    // Longueur annoncée trop grande : refusée avant toute lecture (413).
    let requete = Request::post(format!("{}/comptes/{compte}/depots", base(b)))
        .header("origin", ORIGINE)
        .header("sec-fetch-site", "same-origin")
        .header("x-sigma-csrf", "1")
        .header("content-length", "1025")
        .header(EN_TETE_UTILISATEUR, b.c.administrateur.to_string())
        .body(Body::from(vec![b'a'; 1025]))
        .unwrap();
    assert_eq!(
        appeler(&routeur, requete).await.statut,
        StatusCode::PAYLOAD_TOO_LARGE
    );
    // Ligne trop longue : 422 avec le rang de la ligne.
    let r = deposer_sur(
        &routeur,
        b,
        b.c.administrateur,
        compte,
        format!("Date;Montant\n{}\n", "x".repeat(65)).as_bytes(),
        "",
    )
    .await;
    assert_eq!(r.statut, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        r.json(),
        json!({"erreur": "releve_ligne_trop_longue", "ligne": 2})
    );
    // La limite de 5 Mio par défaut : 5 Mio + 1 refusé (413).
    let defaut = 5 * 1024 * 1024 + 1;
    let r = deposer(b, b.c.administrateur, compte, &vec![b'a'; defaut]).await;
    assert_eq!(r.statut, StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(etat_de_la_base(b).await, avant);
}

#[tokio::test]
async fn analyse_sans_aucune_ecriture() {
    let banc_de_test = banc().await;
    let b = &banc_de_test;
    let compte = caisse(b).await;
    let profil = creer_profil(b, &corps_profil("Banque test")).await;
    let profil = profil["id"].as_str().unwrap().to_owned();
    let r = deposer(b, b.c.collaborateur, compte, releve("analyse").as_bytes()).await;
    assert_eq!(r.statut, StatusCode::CREATED);
    let depot = r.json()["id"].as_str().unwrap().to_owned();

    let avant = etat_de_la_base(b).await;
    let r = analyser(b, b.c.collaborateur, &depot, &profil, "").await;
    assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
    let a = r.json();
    assert_eq!(etat_de_la_base(b).await, avant, "l'analyse n'écrit rien");
    assert_eq!(a["nombre_lignes"], 3);
    assert_eq!(a["version"], 1);
    assert_eq!(a["statut_probant"], "importe_non_certifie");
    assert_eq!(
        a["totaux"],
        json!({"entrees": "162345.5", "sorties": "2500", "net": "159845.5"})
    );
    assert_eq!(a["premiere_date"], "2026-01-02");
    assert_eq!(a["derniere_date"], "2026-01-05");
    assert_eq!(a["premier_solde"], "150000");
    assert_eq!(a["dernier_solde"], "159845.5");
    assert_eq!(a["colonnes"][0], json!({"champ": "date", "titre": "Date"}));
    // Libellé à formule : rendu tel quel, mais jamais sans son alerte.
    let ligne = &a["lignes"][1];
    assert_eq!(ligne["libelle"], "=FRAIS TENUE");
    assert_eq!(ligne["montant"], "-2500");
    assert_eq!(
        ligne["alertes"],
        json!([{"champ": "libelle", "code": "libelle_formule"}])
    );
    assert_eq!(a["nombre_alertes"], 1);
    assert_eq!(a["alertes"][0]["ligne"], 3);
    // Aperçu borné.
    let r = analyser(b, b.c.collaborateur, &depot, &profil, "?lignes=1").await;
    assert_eq!(r.json()["lignes"].as_array().unwrap().len(), 1);
    assert_eq!(r.json()["nombre_lignes"], 3);
    for p in ["?lignes=101", "?version=0", "?autre=1", "?lignes=-1"] {
        let r = analyser(b, b.c.collaborateur, &depot, &profil, p).await;
        assert_eq!(r.statut, StatusCode::BAD_REQUEST, "{p}");
    }
    // Profil qui ne convient pas au fichier : 422 et la ligne fautive, sans écriture.
    let autre = creer_profil(
        b,
        &corps_profil("Format ISO").replace("%d/%m/%Y", "%Y-%m-%d"),
    )
    .await;
    let r = analyser(b, b.valideur, &depot, autre["id"].as_str().unwrap(), "").await;
    assert_eq!(r.statut, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        r.json(),
        json!({"erreur": "releve_date_invalide", "ligne": 2})
    );
    let avant = etat_de_la_base(b).await;
    // Rôle : l'auditeur n'analyse pas (403) ; profil ou dépôt inexistant : 404.
    let r = analyser(b, b.auditeur, &depot, &profil, "").await;
    assert_eq!(r.statut, StatusCode::FORBIDDEN);
    let r = analyser(
        b,
        b.c.administrateur,
        &depot,
        &Uuid::new_v4().to_string(),
        "",
    )
    .await;
    assert_eq!(r.statut, StatusCode::NOT_FOUND);
    let r = analyser(
        b,
        b.c.administrateur,
        &Uuid::new_v4().to_string(),
        &profil,
        "",
    )
    .await;
    assert_eq!(r.statut, StatusCode::NOT_FOUND);
    assert_eq!(etat_de_la_base(b).await, avant);

    // Limites d'analyse de l'instance (ici trois lignes au plus) : 422 avec la ligne fautive.
    let mut reglages = Reglages::new(ORIGINE.to_owned());
    reglages.releves.limites.lignes = 3;
    let routeur = routeur(&application_base(&b.env, reglages));
    let r = lire(
        &routeur,
        &format!("{}/depots/{depot}/analyse/{profil}", base(b)),
        Some(b.c.administrateur),
    )
    .await;
    assert_eq!(r.statut, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        r.json(),
        json!({"erreur": "releve_trop_de_lignes", "ligne": 4})
    );
}

/// Montants et lignes de `NOM.attendu` (sortie de référence de `formats`).
/// Empreinte du fichier, (ligne, montant) et (ligne, champ, code) des alertes.
type Attendu = (String, Vec<(usize, String)>, Vec<(usize, String, String)>);

fn attendu(texte: &str) -> Attendu {
    let mut fichier = String::new();
    let mut lignes = Vec::new();
    let mut alertes = Vec::new();
    let mut courante = 0;
    for l in texte.lines() {
        if let Some(e) = l.strip_prefix("fichier ") {
            e.clone_into(&mut fichier);
        } else if let Some(n) = l.strip_prefix("ligne ") {
            courante = n.parse().unwrap();
        } else if let Some(m) = l.strip_prefix("  montant ") {
            let m: rust_decimal::Decimal = m.parse().unwrap();
            lignes.push((courante, m.normalize().to_string()));
        } else if let Some(a) = l.strip_prefix("alerte ligne ") {
            let mut p = a.split(' ');
            alertes.push((
                p.next().unwrap().parse().unwrap(),
                p.next().unwrap().to_owned(),
                p.next().unwrap().to_owned(),
            ));
        }
    }
    (fichier, lignes, alertes)
}

#[tokio::test]
async fn cas_de_reference_des_fichiers_synthetiques_de_formats() {
    let banc_de_test = banc().await;
    let b = &banc_de_test;
    let compte = caisse(b).await;
    let racine = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../formats/tests/releves");
    for nom in [
        "banque-utf8-bom",
        "banque-windows-1252",
        "carte-signe-inverse",
        "mobile-money",
    ] {
        let lire_fichier = |ext: &str| std::fs::read(racine.join(format!("{nom}.{ext}"))).unwrap();
        let csv = lire_fichier("csv");
        let toml = String::from_utf8(lire_fichier("toml")).unwrap();
        let (fichier, lignes, alertes) =
            attendu(&String::from_utf8(lire_fichier("attendu")).unwrap());
        let profil = creer_profil(b, &corps_depuis_toml(nom, &toml)).await;
        let r = deposer(b, b.c.collaborateur, compte, &csv).await;
        assert_eq!(r.statut, StatusCode::CREATED, "{nom} : {}", r.texte());
        assert_eq!(r.json()["empreinte"], fichier.as_str(), "{nom}");
        let r = analyser(
            b,
            b.c.collaborateur,
            r.json()["id"].as_str().unwrap(),
            profil["id"].as_str().unwrap(),
            "?lignes=100",
        )
        .await;
        assert_eq!(r.statut, StatusCode::OK, "{nom} : {}", r.texte());
        let a = r.json();
        assert_eq!(a["empreinte"], fichier.as_str(), "{nom}");
        assert_eq!(a["nombre_lignes"], lignes.len(), "{nom}");
        let obtenues: Vec<(usize, String)> = a["lignes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|l| {
                (
                    usize::try_from(l["ligne"].as_u64().unwrap()).unwrap(),
                    l["montant"].as_str().unwrap().to_owned(),
                )
            })
            .collect();
        assert_eq!(obtenues, lignes, "{nom}");
        let obtenues: Vec<(usize, String, String)> = a["alertes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|l| {
                (
                    usize::try_from(l["ligne"].as_u64().unwrap()).unwrap(),
                    l["champ"].as_str().unwrap().to_owned(),
                    l["code"].as_str().unwrap().to_owned(),
                )
            })
            .collect();
        assert_eq!(obtenues, alertes, "{nom}");
    }
}

#[tokio::test]
async fn rls_et_ecritures_croisees_refusees() {
    let banc_de_test = banc().await;
    let b = &banc_de_test;
    let compte = caisse(b).await;
    let profil = creer_profil(b, &corps_profil("Banque")).await;
    let r = deposer(b, b.c.collaborateur, compte, releve("rls").as_bytes()).await;
    assert_eq!(r.statut, StatusCode::CREATED);
    let piece = Uuid::parse_str(r.json()["piece"].as_str().unwrap()).unwrap();
    let profil = Uuid::parse_str(profil["id"].as_str().unwrap()).unwrap();

    // Contexte d'un autre dossier du même cabinet : rien n'est visible.
    let mut tx = ouvrir_transaction(
        &b.env.application,
        ContexteTenant::cabinet(b.c.cabinet, b.c.administrateur),
    )
    .await
    .unwrap();
    tx.restreindre_au_dossier(b.c.autre_dossier).await.unwrap();
    for table in ["profil_releve", "depot_releve"] {
        let n: i64 =
            sqlx::query_scalar(sqlx::AssertSqlSafe(format!("SELECT count(*) FROM {table}")))
                .fetch_one(tx.connexion())
                .await
                .unwrap();
        assert_eq!(n, 0, "{table}");
    }
    drop(tx);

    // Rôle applicatif au contexte du dossier : une ligne d'un autre dossier est refusée par
    // `WITH CHECK` (42501) ; le dépôt est en ajout seul (42501).
    let cas: [(&str, Uuid); 4] = [
        (
            "INSERT INTO depot_releve (cabinet_id, dossier_id, id, compte_tresorerie_id, \
             piece_id, depose_par, depose_le) \
             VALUES ($1, $2, gen_random_uuid(), $3, $4, $5, now())",
            b.c.autre_dossier.uuid(),
        ),
        (
            "INSERT INTO profil_releve (cabinet_id, dossier_id, id, version, nom, definition, \
             empreinte, cree_par, cree_le) \
             SELECT $1, $2, $4, 2, 'x', '{}', sha256('x'), $5, now() WHERE $3::uuid IS NOT NULL",
            b.c.autre_dossier.uuid(),
        ),
        (
            "UPDATE depot_releve SET depose_le = now() WHERE $1::uuid IS NOT NULL AND $2::uuid IS NOT NULL AND $3::uuid IS NOT NULL AND $4::uuid IS NOT NULL AND $5::uuid IS NOT NULL",
            b.c.dossier.uuid(),
        ),
        (
            "DELETE FROM depot_releve WHERE $1::uuid IS NOT NULL AND $2::uuid IS NOT NULL AND $3::uuid IS NOT NULL AND $4::uuid IS NOT NULL AND $5::uuid IS NOT NULL",
            b.c.dossier.uuid(),
        ),
    ];
    for (requete, dossier) in cas {
        let mut tx = ouvrir_transaction(
            &b.env.application,
            ContexteTenant::cabinet(b.c.cabinet, b.c.administrateur),
        )
        .await
        .unwrap();
        tx.restreindre_au_dossier(b.c.dossier).await.unwrap();
        let ressource = if requete.contains("profil_releve") {
            profil
        } else {
            piece
        };
        let e = sqlx::query(requete)
            .bind(b.c.cabinet.uuid())
            .bind(dossier)
            .bind(compte)
            .bind(ressource)
            .bind(b.c.administrateur.uuid())
            .execute(tx.connexion())
            .await
            .unwrap_err();
        assert_eq!(code_sql(&e).as_deref(), Some("42501"), "{requete} : {e}");
    }

    // Seconde barrière : un dépôt dont la pièce n'est pas un texte externe est refusé (23514).
    let pdf =
        commun::pieces::doter(&b.env.admin, b.c.cabinet, b.c.dossier, b.c.administrateur).await;
    let mut tx = ouvrir_transaction(
        &b.env.application,
        ContexteTenant::cabinet(b.c.cabinet, b.c.administrateur),
    )
    .await
    .unwrap();
    tx.restreindre_au_dossier(b.c.dossier).await.unwrap();
    let e = sqlx::query(
        "INSERT INTO depot_releve (cabinet_id, dossier_id, id, compte_tresorerie_id, piece_id, \
         depose_par, depose_le) VALUES ($1, $2, gen_random_uuid(), $3, $4, $5, now())",
    )
    .bind(b.c.cabinet.uuid())
    .bind(b.c.dossier.uuid())
    .bind(compte)
    .bind(pdf)
    .bind(b.c.administrateur.uuid())
    .execute(tx.connexion())
    .await
    .unwrap_err();
    assert_eq!(code_sql(&e).as_deref(), Some("23514"), "{e}");
}

/// Objets du stockage partagé dont le contenu est `octets`.
fn objets_de(octets: &[u8]) -> usize {
    commun::pieces::StockageMemoire::partage()
        .0
        .lock()
        .unwrap()
        .values()
        .filter(|v| v.as_slice() == octets)
        .count()
}

#[tokio::test]
async fn depots_concurrents_du_meme_fichier_un_seul_objet() {
    let banc_de_test = banc().await;
    let b = &banc_de_test;
    let compte = caisse(b).await;
    let fichier = releve(&format!("concurrence {}", Uuid::new_v4()));
    // Deux dépôts simultanés du même fichier : le verrou du dossier les sérialise ; un seul crée
    // le dépôt et l'objet WORM, l'autre est signalé en doublon.
    let (r1, r2) = tokio::join!(
        deposer(b, b.c.administrateur, compte, fichier.as_bytes()),
        deposer(b, b.c.collaborateur, compte, fichier.as_bytes()),
    );
    let mut statuts = [r1.statut, r2.statut];
    statuts.sort();
    assert_eq!(statuts, [StatusCode::OK, StatusCode::CREATED]);
    assert_eq!(compter(b, "depot_releve").await, 1);
    assert_eq!(objets_de(fichier.as_bytes()), 1);
    // Un doublon séquentiel n'écrit pas non plus de second objet.
    let r = deposer(b, b.valideur, compte, fichier.as_bytes()).await;
    assert_eq!(r.json()["doublon"], true);
    assert_eq!(objets_de(fichier.as_bytes()), 1);
    assert_eq!(compter(b, "depot_releve").await, 1);
}

#[tokio::test]
async fn analyse_refusee_pour_une_piece_ecartee() {
    let banc_de_test = banc().await;
    let b = &banc_de_test;
    let compte = caisse(b).await;
    let profil = creer_profil(b, &corps_profil("Banque")).await;
    let r = deposer(b, b.c.collaborateur, compte, releve("écart").as_bytes()).await;
    assert_eq!(r.statut, StatusCode::CREATED);
    let depot = r.json();
    let r = envoyer(
        &b.routeur,
        &format!(
            "/api/cabinets/{}/dossiers/{}/pieces/{}/ecart",
            b.c.cabinet,
            b.c.dossier,
            depot["piece"].as_str().unwrap()
        ),
        Some(b.c.administrateur),
        r#"{"motif":"Mauvais relevé"}"#,
        None,
    )
    .await;
    assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
    let avant = etat_de_la_base(b).await;
    let r = analyser(
        b,
        b.c.collaborateur,
        depot["id"].as_str().unwrap(),
        profil["id"].as_str().unwrap(),
        "",
    )
    .await;
    assert_eq!(r.statut, StatusCode::CONFLICT);
    assert_eq!(code(&r), "piece_ecartee");
    assert_eq!(etat_de_la_base(b).await, avant);
    // La pièce écartée ne compte plus comme doublon : le même fichier se redépose.
    let r = deposer(b, b.c.collaborateur, compte, releve("écart").as_bytes()).await;
    assert_eq!(r.statut, StatusCode::CREATED, "{}", r.texte());
}

#[tokio::test]
async fn cles_etrangeres_composites_et_versions() {
    let banc_de_test = banc().await;
    let b = &banc_de_test;
    let compte = caisse(b).await;
    let profil = creer_profil(b, &corps_profil("Banque")).await;
    let profil = Uuid::parse_str(profil["id"].as_str().unwrap()).unwrap();
    let r = deposer(b, b.c.collaborateur, compte, releve("clés").as_bytes()).await;
    assert_eq!(r.statut, StatusCode::CREATED);
    // Pièce texte libre (sans dépôt) du dossier : seule sa clé composite est en jeu.
    let piece =
        commun::releves::piece_texte(&b.env.admin, b.c.cabinet, b.c.dossier, b.c.administrateur)
            .await;
    // Un compte de trésorerie et une pièce texte réels dans l'AUTRE dossier du cabinet.
    commun::referentiel::doter(
        &b.env.application,
        b.c.cabinet,
        b.c.autre_dossier,
        b.c.administrateur,
    )
    .await;
    let compte_autre = commun::tresorerie::doter(
        &b.env.admin,
        b.c.cabinet,
        b.c.autre_dossier,
        b.c.administrateur,
    )
    .await;
    let (_, depot_autre) = commun::releves::doter(
        &b.env.admin,
        b.c.cabinet,
        b.c.autre_dossier,
        b.c.administrateur,
        compte_autre,
    )
    .await;
    let piece_autre: Uuid = sqlx::query_scalar("SELECT piece_id FROM depot_releve WHERE id = $1")
        .bind(depot_autre)
        .fetch_one(&b.env.admin)
        .await
        .unwrap();

    let depot = "INSERT INTO depot_releve (cabinet_id, dossier_id, id, compte_tresorerie_id, \
                 piece_id, depose_par, depose_le) \
                 VALUES ($1, $2, gen_random_uuid(), $3, $4, $5, now())";
    let version = "INSERT INTO profil_releve (cabinet_id, dossier_id, id, version, nom, \
                   definition, empreinte, cree_par, cree_le) \
                   VALUES ($1, $2, $3, $4, 'v', '{}', sha256('v'), $5, now())";
    // (requête, compte ou profil, pièce ou version, SQLSTATE attendu) : chaque clé composite seule.
    let cas: [(&str, Uuid, Value, &str); 5] = [
        // Compte de trésorerie d'un autre dossier, sous son propre dossier.
        (depot, compte_autre, json!(piece.to_string()), "23503"),
        // Pièce d'un autre dossier, sous son propre dossier.
        (depot, compte, json!(piece_autre.to_string()), "23503"),
        // Version 3 sans version 2.
        (version, profil, json!(3), "23503"),
        // Version 1 dupliquée.
        (version, profil, json!(1), "23505"),
        // Version 0 : hors domaine.
        (version, profil, json!(0), "23514"),
    ];
    for (requete, cible, valeur, sqlstate) in cas {
        let mut tx = ouvrir_transaction(
            &b.env.application,
            ContexteTenant::cabinet(b.c.cabinet, b.c.administrateur),
        )
        .await
        .unwrap();
        tx.restreindre_au_dossier(b.c.dossier).await.unwrap();
        let q = sqlx::query(requete)
            .bind(b.c.cabinet.uuid())
            .bind(b.c.dossier.uuid())
            .bind(cible);
        let q = match valeur.as_i64() {
            Some(n) => q.bind(i32::try_from(n).unwrap()),
            None => q.bind(Uuid::parse_str(valeur.as_str().unwrap()).unwrap()),
        };
        let erreur = q
            .bind(b.c.administrateur.uuid())
            .execute(tx.connexion())
            .await
            .unwrap_err();
        assert_eq!(
            code_sql(&erreur).as_deref(),
            Some(sqlstate),
            "{requete} {valeur} : {erreur}"
        );
    }
    // Contrôle : la version 2 suivant la version 1 passe (rôle applicatif, même dossier).
    let mut tx = ouvrir_transaction(
        &b.env.application,
        ContexteTenant::cabinet(b.c.cabinet, b.c.administrateur),
    )
    .await
    .unwrap();
    tx.restreindre_au_dossier(b.c.dossier).await.unwrap();
    sqlx::query(version)
        .bind(b.c.cabinet.uuid())
        .bind(b.c.dossier.uuid())
        .bind(profil)
        .bind(2_i32)
        .bind(b.c.administrateur.uuid())
        .execute(tx.connexion())
        .await
        .unwrap();
}
