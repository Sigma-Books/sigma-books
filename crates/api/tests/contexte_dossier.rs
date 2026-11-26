//! Contexte du dossier (ADR 0027) : pays, devise de tenue et échelle, fuseau, identifiants légaux.
//! Dossiers en XOF (existant), XAF et CDF (deux décimales) de bout en bout : saisie, validation,
//! chaîne, export d'intégrité vérifié par `integrite`, lecture par l'API, isolation entre cabinets,
//! date du jour dans le fuseau du dossier.
//!
//! Isolation de chaque route : `tests/isolation/organisation.rs`.
#![cfg(feature = "tests-postgres")]
// Fichier de test : les `expect`/`unwrap` y sont voulus.
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::too_many_lines)]

mod commun;

use api::base::organisation::lire_identifiants_legaux;
use api::base::{ContexteTenant, ouvrir_transaction};
use axum::http::StatusCode;
use commun::banc::{Banc, aujourd, banc, banc_avec, banque, caisse, code, contenu};
use commun::brouillard::{aujourd_hui, journal_de_saisie, ligne};
use commun::creer_cabinet;
use commun::serveur::{HorlogeTest, envoyer, lire};
use integrite::Options;
use serde_json::{Value, json};
use time::{Date, OffsetDateTime};

/// Passe le dossier du banc en devise de tenue donnée (le propriétaire de la base le fait : le
/// rôle applicatif n'a aucun `UPDATE` sur ces colonnes).
async fn tenir_en(b: &Banc, pays: &str, devise: &str, decimales: i16, fuseau: &str) {
    sqlx::query(
        "UPDATE dossier SET pays = $3, devise_tenue = $4, decimales_tenue = $5, fuseau = $6 \
         WHERE cabinet_id = $1 AND id = $2",
    )
    .bind(b.c.cabinet.uuid())
    .bind(b.c.dossier.uuid())
    .bind(pays)
    .bind(devise)
    .bind(decimales)
    .bind(fuseau)
    .execute(&b.env.admin)
    .await
    .unwrap();
}

async fn dossier(b: &Banc) -> Value {
    let r = lire(
        &b.routeur,
        &format!("/api/cabinets/{}/dossiers/{}", b.c.cabinet, b.c.dossier),
        Some(b.c.administrateur),
    )
    .await;
    assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
    r.json()
}

fn ligne_devise(compte: &str, sens: &str, code: &str, montant: &str, cours: &str) -> String {
    format!(
        r#"{{"compte":"{compte}","sens":"{sens}","devise":{{"code":"{code}","montant":"{montant}","cours":"{cours}"}}}}"#
    )
}

async fn poster(b: &Banc, lignes: &[String]) -> commun::serveur::Reponse {
    envoyer(
        &b.routeur,
        &b.base(),
        Some(b.c.collaborateur),
        &contenu(&journal_de_saisie(), &aujourd(), lignes, ""),
        None,
    )
    .await
}

/// Crée, lie une pièce et valide un brouillon ; rend le texte canonique stocké.
async fn valider(b: &Banc, lignes: &[String]) -> String {
    let (id, version) = b
        .pret_avec(&contenu(&journal_de_saisie(), &aujourd(), lignes, ""))
        .await;
    let r = b.valider(id, version).await;
    assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
    b.chaine().await.last().unwrap().2.clone()
}

/// Export d'intégrité du dossier, relu par `integrite` : intègre, de la version attendue.
async fn exporter_et_verifier(b: &Banc, version: &str, nombre: u64) {
    let r = envoyer(
        &b.routeur,
        &format!(
            "/api/cabinets/{}/dossiers/{}/integrite/export",
            b.c.cabinet, b.c.dossier
        ),
        Some(b.valideur),
        "",
        None,
    )
    .await;
    assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
    let texte = r.texte();
    assert!(
        texte.starts_with(&format!("sigma-books export {version}\n")),
        "{}",
        texte.lines().next().unwrap()
    );
    let rapport = integrite::verifier(texte.as_bytes(), &Options::default()).unwrap();
    assert!(rapport.est_integre(), "{rapport}");
    assert_eq!(rapport.nombre, nombre);
}

#[tokio::test]
async fn dossier_xof_existant_garde_ses_formats() {
    let b = banc().await;
    let d = dossier(&b).await;
    assert_eq!(d["pays"], "SN");
    assert_eq!(d["devise"], "XOF");
    assert_eq!(d["decimales"], 0);
    assert_eq!(d["fuseau"], "Africa/Dakar");
    assert_eq!(d["langue"], "fr");
    // Des centimes en XOF : refusés à la saisie ; une ligne « en devise XOF » aussi.
    let r = poster(
        &b,
        &[
            ligne(caisse(), "debit", "1000.5"),
            ligne(banque(), "credit", "1000.5"),
        ],
    )
    .await;
    assert_eq!(r.statut, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(code(&r), "montant_echelle_devise_tenue");
    let r = poster(
        &b,
        &[
            ligne_devise(caisse(), "debit", "XOF", "1000", "1"),
            ligne(banque(), "credit", "1000"),
        ],
    )
    .await;
    assert_eq!(r.statut, StatusCode::UNPROCESSABLE_ENTITY, "{}", r.texte());
    assert_eq!(code(&r), "montant_invalide");
    // Écriture entière : format v1 à l'identique ; export v3 comme avant.
    let texte = valider(
        &b,
        &[
            ligne(caisse(), "debit", "1000"),
            ligne(banque(), "credit", "1000"),
        ],
    )
    .await;
    assert!(texte.starts_with("ecriture v1\n"), "{texte}");
    assert!(!texte.contains("devise-tenue"));
    exporter_et_verifier(&b, "v3", 1).await;
    assert!(b.chaine_valide().await);
}

#[tokio::test]
async fn dossier_cdf_deux_decimales_de_bout_en_bout() {
    let b = banc().await;
    tenir_en(&b, "CD", "CDF", 2, "Africa/Kinshasa").await;
    let d = dossier(&b).await;
    assert_eq!(
        (d["pays"].as_str(), d["devise"].as_str()),
        (Some("CD"), Some("CDF"))
    );
    assert_eq!(d["decimales"], 2);
    assert_eq!(d["fuseau"], "Africa/Kinshasa");
    // Trois décimales : refusées ; deux : acceptées et conservées exactes.
    let r = poster(
        &b,
        &[
            ligne(caisse(), "debit", "1000.555"),
            ligne(banque(), "credit", "1000.555"),
        ],
    )
    .await;
    assert_eq!(r.statut, StatusCode::UNPROCESSABLE_ENTITY, "{}", r.texte());
    assert_eq!(code(&r), "montant_echelle_devise_tenue");
    let texte = valider(
        &b,
        &[
            ligne(caisse(), "debit", "1000.55"),
            ligne(banque(), "credit", "1000.55"),
        ],
    )
    .await;
    assert!(texte.starts_with("ecriture v4\n"), "{texte}");
    assert!(
        texte.contains("devise-tenue=CDF\ndecimales-tenue=2\n"),
        "{texte}"
    );
    assert!(texte.contains("montant=1000.55\n"), "{texte}");
    // Ligne en EUR : contre-valeur arrondie au centime par le moteur (100 EUR à 2,5551).
    let texte = valider(
        &b,
        &[
            ligne_devise(caisse(), "debit", "EUR", "100", "2.5551"),
            ligne(banque(), "credit", "255.51"),
        ],
    )
    .await;
    assert!(
        texte.contains("montant=255.51\ndevise=EUR\nmontant-devise=100\ncours=2.5551\n"),
        "{texte}"
    );
    // Le XOF est une devise étrangère pour ce dossier : accepté en ligne en devise (1000 XOF à 4,5).
    let texte = valider(
        &b,
        &[
            ligne_devise(caisse(), "debit", "XOF", "1000", "4.5"),
            ligne(banque(), "credit", "4500"),
        ],
    )
    .await;
    assert!(texte.contains("devise=XOF\n"), "{texte}");
    // Un déséquilibre d'un centime est refusé à la validation.
    let (id, v) = b
        .pret_avec(&contenu(
            &journal_de_saisie(),
            &aujourd(),
            &[
                ligne(caisse(), "debit", "10.01"),
                ligne(banque(), "credit", "10.00"),
            ],
            "",
        ))
        .await;
    let r = b.valider(id, v).await;
    assert_eq!(r.statut, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(code(&r), "ecriture_desequilibree");
    assert!(b.chaine_valide().await);
    exporter_et_verifier(&b, "v5", 3).await;
}

#[tokio::test]
async fn dossier_xaf_de_bout_en_bout() {
    let b = banc().await;
    tenir_en(&b, "CM", "XAF", 0, "Africa/Douala").await;
    let d = dossier(&b).await;
    assert_eq!(
        (d["devise"].as_str(), d["decimales"].as_u64()),
        (Some("XAF"), Some(0))
    );
    let r = poster(
        &b,
        &[
            ligne(caisse(), "debit", "1000.5"),
            ligne(banque(), "credit", "1000.5"),
        ],
    )
    .await;
    assert_eq!(code(&r), "montant_echelle_devise_tenue");
    // Mêmes montants qu'un dossier XOF, autre devise : écriture v4, donc autre empreinte.
    let texte = valider(
        &b,
        &[
            ligne(caisse(), "debit", "1000"),
            ligne(banque(), "credit", "1000"),
        ],
    )
    .await;
    assert!(texte.starts_with("ecriture v4\n"), "{texte}");
    assert!(
        texte.contains("devise-tenue=XAF\ndecimales-tenue=0\n"),
        "{texte}"
    );
    // Le XOF est ici une devise étrangère.
    valider(
        &b,
        &[
            ligne_devise(caisse(), "debit", "XOF", "1000", "1"),
            ligne(banque(), "credit", "1000"),
        ],
    )
    .await;
    assert!(b.chaine_valide().await);
    exporter_et_verifier(&b, "v5", 2).await;
}

#[tokio::test]
async fn la_base_refuse_une_ligne_dans_la_devise_de_tenue() {
    // Seconde barrière : même un `INSERT` direct d'une ligne « en devise » égale à la devise de
    // tenue du dossier est refusé par le déclencheur (au lieu du `CHECK devise <> 'XOF'`).
    let b = banc().await;
    tenir_en(&b, "CM", "XAF", 0, "Africa/Douala").await;
    let id = b
        .creer(&commun::brouillard::saisie("Seconde barrière", None))
        .await;
    let essai = |devise: &'static str| {
        let (pool, cabinet, dossier) =
            (b.env.admin.clone(), b.c.cabinet.uuid(), b.c.dossier.uuid());
        async move {
            sqlx::query(
                "INSERT INTO brouillon_ligne (cabinet_id, dossier_id, brouillon_id, position, \
                 compte_numero, sens, montant, devise, montant_devise, cours) \
                 SELECT cabinet_id, dossier_id, brouillon_id, 50, compte_numero, sens, montant, \
                        $4, 1, 1 FROM brouillon_ligne \
                 WHERE cabinet_id = $1 AND dossier_id = $2 AND brouillon_id = $3 AND position = 1",
            )
            .bind(cabinet)
            .bind(dossier)
            .bind(id)
            .bind(devise)
            .execute(&pool)
            .await
        }
    };
    assert!(essai("XAF").await.is_err(), "devise de tenue refusée");
    assert!(essai("XOF").await.is_ok(), "autre devise admise");
}

#[tokio::test]
async fn creation_et_lecture_du_contexte_par_l_api() {
    let b = banc().await;
    let chemin = format!("/api/cabinets/{}/dossiers", b.c.cabinet);
    let annee = aujourd_hui().year();
    let corps = |extra: Value| {
        let mut c = json!({"nom": "SARL Kinshasa", "nature": "reprise", "annee": annee});
        c.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        c.to_string()
    };
    // Valeurs par défaut du pays (RDC), forme juridique et identifiants du pays.
    let r = envoyer(
        &b.routeur,
        &chemin,
        Some(b.c.administrateur),
        &corps(json!({
            "pays": "CD",
            "forme_juridique": "SARL",
            "identifiants_legaux": [{"type": "RCCM", "valeur": "CD/KIN/RCCM/20-B-01234"}]
        })),
        None,
    )
    .await;
    assert_eq!(r.statut, StatusCode::CREATED, "{}", r.texte());
    let v = r.json();
    assert_eq!(v["pays"], "CD");
    assert_eq!(v["devise"], "CDF");
    assert_eq!(v["decimales"], 2);
    assert_eq!(v["fuseau"], "Africa/Kinshasa");
    assert_eq!(v["langue"], "fr");
    assert_eq!(v["forme_juridique"], "SARL");
    assert_eq!(v["identifiants_legaux"][0]["type"], "RCCM");
    let id = v["id"].as_str().unwrap().to_owned();
    // Lecture : mêmes champs, identifiants compris ; la liste n'a pas les identifiants.
    let r = lire(
        &b.routeur,
        &format!("{chemin}/{id}"),
        Some(b.c.administrateur),
    )
    .await;
    assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
    assert_eq!(r.json()["decimales"], 2);
    assert_eq!(
        r.json()["identifiants_legaux"][0]["valeur"],
        "CD/KIN/RCCM/20-B-01234"
    );
    let r = lire(&b.routeur, &chemin, Some(b.c.administrateur)).await;
    let liste = r.json();
    let cree = liste
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["id"] == id)
        .unwrap();
    assert_eq!(cree["devise"], "CDF");
    assert!(cree.get("identifiants_legaux").is_none());
    // Refus à codes stables.
    for (extra, attendu) in [
        (json!({"pays": "FR"}), "pays_inconnu"),
        (
            json!({"pays": "SN", "devise": "ZZZ"}),
            "devise_tenue_inconnue",
        ),
        (
            json!({"pays": "SN", "fuseau": "Europe/Paris"}),
            "fuseau_inconnu",
        ),
        (
            json!({"pays": "SN", "langue": "FR"}),
            "contexte_dossier_invalide",
        ),
        (
            json!({"pays": "CM", "identifiants_legaux": [{"type": "NINEA", "valeur": "1"}]}),
            "contexte_dossier_invalide",
        ),
        (
            json!({"pays": "SN", "identifiants_legaux": [{"type": "NINEA", "valeur": "1"}, {"type": "NINEA", "valeur": "2"}]}),
            "contexte_dossier_invalide",
        ),
    ] {
        let r = envoyer(
            &b.routeur,
            &chemin,
            Some(b.c.administrateur),
            &corps(extra),
            None,
        )
        .await;
        assert_eq!(r.statut, StatusCode::UNPROCESSABLE_ENTITY, "{}", r.texte());
        assert_eq!(code(&r), attendu);
    }
    // Pays obligatoire, colonne propre à un pays inconnue : 400.
    let sans_pays = json!({"nom": "X", "nature": "reprise", "annee": annee}).to_string();
    let r = envoyer(
        &b.routeur,
        &chemin,
        Some(b.c.administrateur),
        &sans_pays,
        None,
    )
    .await;
    assert_eq!(r.statut, StatusCode::BAD_REQUEST);
    let r = envoyer(
        &b.routeur,
        &chemin,
        Some(b.c.administrateur),
        &corps(json!({"pays": "SN", "ninea": "1"})),
        None,
    )
    .await;
    assert_eq!(r.statut, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn le_pack_pays_est_lu_par_la_route() {
    let b = banc().await;
    let r = lire(
        &b.routeur,
        &format!("/api/cabinets/{}/pays", b.c.cabinet),
        Some(b.c.administrateur),
    )
    .await;
    assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
    let v = r.json();
    assert_eq!(v["pays"].as_array().unwrap().len(), 17);
    let cd = v["pays"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["code"] == "CD")
        .unwrap();
    assert_eq!(cd["devise"], "CDF");
    let sn = v["pays"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["code"] == "SN")
        .unwrap();
    let types: Vec<&str> = sn["identifiants"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["code"].as_str().unwrap())
        .collect();
    assert!(types.contains(&"NINEA") && types.contains(&"RCCM"));
    let cdf = v["devises"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["code"] == "CDF")
        .unwrap();
    assert_eq!(cdf["decimales"], 2);
    // Un collaborateur ne crée pas de dossier : il ne lit pas le pack.
    let r = lire(
        &b.routeur,
        &format!("/api/cabinets/{}/pays", b.c.cabinet),
        Some(b.c.collaborateur),
    )
    .await;
    assert_eq!(r.statut, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn les_identifiants_legaux_restent_dans_leur_dossier_et_leur_cabinet() {
    let a = banc().await;
    let autre = creer_cabinet(&a.env.admin, "Cabinet B").await;
    for (c, valeur) in [(&a.c, "A-0001"), (&autre, "B-0001")] {
        sqlx::query(
            "INSERT INTO dossier_identifiant_legal (cabinet_id, dossier_id, type, valeur) \
             VALUES ($1, $2, 'AUTRE', $3)",
        )
        .bind(c.cabinet.uuid())
        .bind(c.dossier.uuid())
        .bind(valeur)
        .execute(&a.env.admin)
        .await
        .unwrap();
    }
    // Sous le rôle applicatif, dans le contexte du dossier de A : seul son identifiant.
    let mut tx = ouvrir_transaction(
        &a.env.application,
        ContexteTenant::dossier(a.c.cabinet, a.c.dossier, a.c.administrateur),
    )
    .await
    .unwrap();
    let lus = lire_identifiants_legaux(&mut tx).await.unwrap();
    assert_eq!(
        lus.iter().map(|i| i.valeur.as_str()).collect::<Vec<_>>(),
        ["A-0001"]
    );
    // Dossier de A cité avec le cabinet de B : rien (RLS et filtre).
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM dossier_identifiant_legal")
        .fetch_one(tx.connexion())
        .await
        .unwrap();
    assert_eq!(n, 1);
    // Ajout seul : ni modification ni suppression par le rôle applicatif.
    assert!(
        sqlx::query("UPDATE dossier_identifiant_legal SET valeur = 'X'")
            .execute(tx.connexion())
            .await
            .is_err()
    );
    tx.annuler().await.unwrap();
    let mut tx = ouvrir_transaction(
        &a.env.application,
        ContexteTenant::dossier(a.c.cabinet, a.c.dossier, a.c.administrateur),
    )
    .await
    .unwrap();
    assert!(
        sqlx::query("DELETE FROM dossier_identifiant_legal")
            .execute(tx.connexion())
            .await
            .is_err()
    );
    tx.annuler().await.unwrap();
    // Le contexte du dossier (devise de tenue…) n'est modifiable par aucune requête applicative.
    let mut tx = ouvrir_transaction(
        &a.env.application,
        ContexteTenant::dossier(a.c.cabinet, a.c.dossier, a.c.administrateur),
    )
    .await
    .unwrap();
    assert!(
        sqlx::query("UPDATE dossier SET devise_tenue = 'EUR'")
            .execute(tx.connexion())
            .await
            .is_err()
    );
    tx.annuler().await.unwrap();
}

/// 23 h 30 UTC du jour : encore le même jour à Dakar (UTC+0), déjà le lendemain à Douala (UTC+1).
#[tokio::test]
async fn la_date_de_validation_est_celle_du_fuseau_du_dossier() {
    let aujourdhui: Date = aujourd_hui();
    let instant: OffsetDateTime = aujourdhui.with_hms(23, 30, 0).unwrap().assume_utc();
    let horloge = HorlogeTest::default();
    horloge.fixer(instant);
    let dakar = banc_avec(Some(&horloge)).await;
    let t = valider(
        &dakar,
        &[
            ligne(caisse(), "debit", "1000"),
            ligne(banque(), "credit", "1000"),
        ],
    )
    .await;
    assert!(
        t.contains(&format!("date-validation={aujourdhui}\n")),
        "{t}"
    );

    let douala = banc_avec(Some(&horloge)).await;
    tenir_en(&douala, "CM", "XAF", 0, "Africa/Douala").await;
    let t = valider(
        &douala,
        &[
            ligne(caisse(), "debit", "1000"),
            ligne(banque(), "credit", "1000"),
        ],
    )
    .await;
    let lendemain = aujourdhui.next_day().unwrap();
    assert!(t.contains(&format!("date-validation={lendemain}\n")), "{t}");
}
