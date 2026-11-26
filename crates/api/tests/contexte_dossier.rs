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
use commun::brouillard::{aujourd_hui, journal_de_saisie, ligne, saisie};
use commun::creer_cabinet;
use commun::serveur::{HorlogeTest, envoyer, lire};
use integrite::Options;
use serde_json::{Value, json};
use time::{Date, OffsetDateTime};

/// Passe le dossier du banc en devise de tenue donnée (le propriétaire de la base le fait : le
/// rôle applicatif n'a aucun `UPDATE` sur ces colonnes).
async fn tenir_en(b: &Banc, pays: &str, devise: &str, decimales: i16, fuseau: &str) {
    // Le déclencheur exige le contexte du dossier modifié (comme la requête applicative).
    let mut tx = b.env.admin.begin().await.unwrap();
    sqlx::query(
        "SELECT set_config('sigma.cabinet', $1, true), set_config('sigma.dossier', $2, true)",
    )
    .bind(b.c.cabinet.to_string())
    .bind(b.c.dossier.to_string())
    .execute(&mut *tx)
    .await
    .unwrap();
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
    .execute(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();
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
    // Sans écriture validée, un identifiant se supprime (remplacement) : seulement ceux du dossier
    // du contexte, jamais ceux de l'autre cabinet.
    let supprimes = sqlx::query("DELETE FROM dossier_identifiant_legal")
        .execute(tx.connexion())
        .await
        .unwrap()
        .rows_affected();
    assert_eq!(supprimes, 1);
    tx.annuler().await.unwrap();
    // Dès qu'une écriture validée existe : la suppression est refusée par le déclencheur.
    valider(
        &a,
        &[
            ligne(caisse(), "debit", "1000"),
            ligne(banque(), "credit", "1000"),
        ],
    )
    .await;
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
    // En contexte de cabinet (sans dossier), le déclencheur ne laisse rien passer non plus.
    let mut tx = ouvrir_transaction(
        &a.env.application,
        ContexteTenant::cabinet(a.c.cabinet, a.c.administrateur),
    )
    .await
    .unwrap();
    let r = sqlx::query("DELETE FROM dossier_identifiant_legal")
        .execute(tx.connexion())
        .await;
    assert!(r.is_err() || r.unwrap().rows_affected() == 0);
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

// --- Modification du contexte (ADR 0027, point 1) ------------------------------------------

async fn modifier(
    b: &Banc,
    par: api::base::IdUtilisateur,
    corps: &str,
    anciennete: Option<i64>,
) -> commun::serveur::Reponse {
    envoyer(
        &b.routeur,
        &format!(
            "/api/cabinets/{}/dossiers/{}/contexte",
            b.c.cabinet, b.c.dossier
        ),
        Some(par),
        corps,
        anciennete,
    )
    .await
}

#[tokio::test]
async fn le_contexte_se_modifie_avant_la_premiere_ecriture() {
    let b = banc().await;
    let r = modifier(
        &b,
        b.c.administrateur,
        r#"{"pays":"CD","devise":"CDF","fuseau":"Africa/Lubumbashi","forme_juridique":"SA"}"#,
        None,
    )
    .await;
    assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
    let v = r.json();
    assert_eq!(
        (v["pays"].as_str(), v["devise"].as_str()),
        (Some("CD"), Some("CDF"))
    );
    assert_eq!(v["decimales"], 2, "l'échelle suit le pack");
    assert_eq!(v["fuseau"], "Africa/Lubumbashi");
    assert_eq!(v["forme_juridique"], "SA");
    assert_eq!(dossier(&b).await["devise"], "CDF");
    // Inscrit à l'audit chaîné du dossier, avec l'avant et l'après.
    let audit: Vec<String> =
        sqlx::query_scalar("SELECT contenu FROM audit_dossier WHERE dossier_id = $1")
            .bind(b.c.dossier.uuid())
            .fetch_all(&b.env.admin)
            .await
            .unwrap();
    let entree = audit
        .iter()
        .find(|c| c.contains("ModifierContexteDossier"))
        .expect("audit");
    assert!(
        entree.contains("devise_tenue") && entree.contains("XOF") && entree.contains("CDF"),
        "{entree}"
    );
    // Le dossier se tient désormais en CDF, deux décimales, format v4.
    let texte = valider(
        &b,
        &[
            ligne(caisse(), "debit", "10.55"),
            ligne(banque(), "credit", "10.55"),
        ],
    )
    .await;
    assert!(
        texte.starts_with("ecriture v4\n") && texte.contains("decimales-tenue=2\n"),
        "{texte}"
    );
    // Redire les mêmes valeurs ne change rien (409), champ inconnu ou corps vide : 400.
    let r = modifier(
        &b,
        b.c.administrateur,
        r#"{"fuseau":"Africa/Lubumbashi"}"#,
        None,
    )
    .await;
    assert_eq!(r.statut, StatusCode::CONFLICT);
    assert_eq!(code(&r), "aucun_changement");
    for corps in ["{}", r#"{"ninea":"1"}"#, "pas du json"] {
        assert_eq!(
            modifier(&b, b.c.administrateur, corps, None).await.statut,
            StatusCode::BAD_REQUEST,
            "{corps}"
        );
    }
}

#[tokio::test]
async fn pays_et_devise_se_figent_a_la_premiere_ecriture_validee() {
    let b = banc().await;
    valider(
        &b,
        &[
            ligne(caisse(), "debit", "1000"),
            ligne(banque(), "credit", "1000"),
        ],
    )
    .await;
    for corps in [
        r#"{"devise":"EUR"}"#,
        r#"{"pays":"CM"}"#,
        r#"{"pays":"CM","devise":"XAF"}"#,
    ] {
        let r = modifier(&b, b.c.administrateur, corps, None).await;
        assert_eq!(r.statut, StatusCode::CONFLICT, "{corps} : {}", r.texte());
        assert_eq!(code(&r), "dossier_deja_ecritures", "{corps}");
    }
    // Fuseau, langue et forme juridique restent modifiables (hors du haché) ; effacement par null.
    let r = modifier(
        &b,
        b.c.administrateur,
        r#"{"fuseau":"Africa/Douala","langue":"fr-SN","forme_juridique":"SARL"}"#,
        None,
    )
    .await;
    assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
    assert_eq!(r.json()["devise"], "XOF");
    let r = modifier(&b, b.c.administrateur, r#"{"forme_juridique":null}"#, None).await;
    assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
    assert!(r.json()["forme_juridique"].is_null());
    // Seconde barrière : même une requête SQL du rôle applicatif ne change plus la devise.
    let mut tx = ouvrir_transaction(
        &b.env.application,
        ContexteTenant::dossier(b.c.cabinet, b.c.dossier, b.c.administrateur),
    )
    .await
    .unwrap();
    assert!(
        sqlx::query("UPDATE dossier SET devise_tenue = 'EUR', decimales_tenue = 2")
            .execute(tx.connexion())
            .await
            .is_err()
    );
    tx.annuler().await.unwrap();
    // La chaîne et la devise de la validation suivante n'ont pas bougé.
    assert_eq!(dossier(&b).await["devise"], "XOF");
    assert!(b.chaine_valide().await);
}

#[tokio::test]
async fn un_brouillon_empeche_de_changer_la_devise() {
    let b = banc().await;
    let id = b.creer(&saisie("Brouillon", None)).await;
    let r = modifier(&b, b.c.administrateur, r#"{"devise":"EUR"}"#, None).await;
    assert_eq!(r.statut, StatusCode::CONFLICT, "{}", r.texte());
    assert_eq!(code(&r), "dossier_brouillard_non_vide");
    assert_eq!(
        modifier(
            &b,
            b.c.administrateur,
            r#"{"fuseau":"Africa/Douala"}"#,
            None
        )
        .await
        .statut,
        StatusCode::OK
    );
    // Le brouillon supprimé, la devise peut changer.
    for table in [
        "brouillon_ligne",
        "brouillon_contributeur",
        "brouillon_piece",
    ] {
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "DELETE FROM {table} WHERE brouillon_id = $1"
        )))
        .bind(id)
        .execute(&b.env.admin)
        .await
        .unwrap();
    }
    sqlx::query("DELETE FROM brouillon WHERE id = $1")
        .bind(id)
        .execute(&b.env.admin)
        .await
        .unwrap();
    // (les pièces liées au brouillon s'il y en avait : aucune ici)
    let r = modifier(&b, b.c.administrateur, r#"{"devise":"EUR"}"#, None).await;
    assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
    assert_eq!(r.json()["decimales"], 2);
}

#[tokio::test]
async fn pays_et_devise_exigent_une_session_fraiche_et_l_administrateur() {
    let b = banc().await;
    // Session vieille de dix minutes : le fuseau passe, la devise et le pays non (403).
    let vieille = Some(600);
    assert_eq!(
        modifier(&b, b.c.administrateur, r#"{"devise":"XAF"}"#, vieille)
            .await
            .statut,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        modifier(&b, b.c.administrateur, r#"{"pays":"CM"}"#, vieille)
            .await
            .statut,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        modifier(&b, b.c.administrateur, r#"{"langue":"fr-SN"}"#, vieille)
            .await
            .statut,
        StatusCode::OK
    );
    assert_eq!(dossier(&b).await["devise"], "XOF");
    // Ni le valideur ni le collaborateur ne modifient le contexte (Cedar, puis la requête).
    for qui in [b.valideur, b.c.collaborateur, b.auditeur] {
        let r = modifier(&b, qui, r#"{"fuseau":"Africa/Douala"}"#, None).await;
        assert_eq!(r.statut, StatusCode::FORBIDDEN, "{}", r.texte());
    }
    assert_eq!(dossier(&b).await["fuseau"], "Africa/Dakar");
}

#[tokio::test]
async fn refus_de_modification_a_codes_stables() {
    let b = banc().await;
    for (corps, attendu) in [
        (r#"{"pays":"FR"}"#, "pays_inconnu"),
        (r#"{"devise":"ZZZ"}"#, "devise_tenue_inconnue"),
        (r#"{"fuseau":"Europe/Paris"}"#, "fuseau_inconnu"),
        (r#"{"langue":"FR"}"#, "contexte_dossier_invalide"),
    ] {
        let r = modifier(&b, b.c.administrateur, corps, None).await;
        assert_eq!(r.statut, StatusCode::UNPROCESSABLE_ENTITY, "{corps}");
        assert_eq!(code(&r), attendu, "{corps}");
    }
    // Un identifiant légal propre au Sénégal ne suit pas le dossier dans un autre pays.
    sqlx::query(
        "INSERT INTO dossier_identifiant_legal (cabinet_id, dossier_id, type, valeur) \
         VALUES ($1, $2, 'NINEA', '0012345')",
    )
    .bind(b.c.cabinet.uuid())
    .bind(b.c.dossier.uuid())
    .execute(&b.env.admin)
    .await
    .unwrap();
    let r = modifier(
        &b,
        b.c.administrateur,
        r#"{"pays":"CM","devise":"XAF"}"#,
        None,
    )
    .await;
    assert_eq!(r.statut, StatusCode::UNPROCESSABLE_ENTITY, "{}", r.texte());
    assert_eq!(code(&r), "contexte_dossier_invalide");
    // Vers un pays qui l'admet encore (SN reste SN) : sans objet ; le dossier est inchangé.
    assert_eq!(dossier(&b).await["pays"], "SN");
}

// --- Sécurité de la modification : contexte du déclencheur, remplacement des identifiants,
// --- course entre une saisie et un changement de devise (relecture indépendante de #235) ------

/// Le déclencheur est aveugle hors contexte de dossier : les politiques des écritures et des
/// brouillons exigent `sigma.dossier`. Il refuse donc de changer pays, devise ou échelle sans le
/// contexte du dossier modifié, avant même de chercher des écritures.
#[tokio::test]
async fn le_declencheur_exige_le_contexte_du_dossier() {
    let b = banc().await;
    valider(
        &b,
        &[
            ligne(caisse(), "debit", "1000"),
            ligne(banque(), "credit", "1000"),
        ],
    )
    .await;
    for (contexte, nom) in [
        (
            ContexteTenant::cabinet(b.c.cabinet, b.c.administrateur),
            "cabinet",
        ),
        (
            ContexteTenant::dossier(b.c.cabinet, b.c.autre_dossier, b.c.administrateur),
            "autre dossier",
        ),
        (
            ContexteTenant::dossier(b.c.cabinet, b.c.dossier, b.c.administrateur),
            "bon dossier avec écritures",
        ),
    ] {
        let mut tx = ouvrir_transaction(&b.env.application, contexte)
            .await
            .unwrap();
        let r = sqlx::query(
            "UPDATE dossier SET devise_tenue = 'EUR', decimales_tenue = 2 WHERE cabinet_id = $1 AND id = $2",
        )
        .bind(b.c.cabinet.uuid())
        .bind(b.c.dossier.uuid())
        .execute(tx.connexion())
        .await;
        // Erreur du déclencheur, ou aucune ligne visible (dossier d'un autre contexte : la RLS).
        assert!(
            r.is_err() || r.unwrap().rows_affected() == 0,
            "{nom} : la devise ne change pas"
        );
        tx.annuler().await.unwrap();
    }
    // Un dossier sans écriture ni brouillon : refusé en contexte de cabinet aussi (contexte requis).
    let vide = banc().await;
    let mut tx = ouvrir_transaction(
        &vide.env.application,
        ContexteTenant::cabinet(vide.c.cabinet, vide.c.administrateur),
    )
    .await
    .unwrap();
    let r = sqlx::query(
        "UPDATE dossier SET devise_tenue = 'EUR', decimales_tenue = 2 WHERE cabinet_id = $1 AND id = $2",
    )
    .bind(vide.c.cabinet.uuid())
    .bind(vide.c.dossier.uuid())
    .execute(tx.connexion())
    .await;
    assert!(r.is_err());
    tx.annuler().await.unwrap();
    assert_eq!(dossier(&b).await["devise"], "XOF");
    // Le fuseau, lui, se change en contexte de cabinet (aucun effet sur le haché).
    let mut tx = ouvrir_transaction(
        &vide.env.application,
        ContexteTenant::cabinet(vide.c.cabinet, vide.c.administrateur),
    )
    .await
    .unwrap();
    sqlx::query("UPDATE dossier SET fuseau = 'Africa/Douala' WHERE cabinet_id = $1 AND id = $2")
        .bind(vide.c.cabinet.uuid())
        .bind(vide.c.dossier.uuid())
        .execute(tx.connexion())
        .await
        .unwrap();
    tx.annuler().await.unwrap();
}

#[tokio::test]
async fn les_identifiants_se_remplacent_avec_le_pays_tant_qu_aucune_ecriture_n_existe() {
    let b = banc().await;
    sqlx::query(
        "INSERT INTO dossier_identifiant_legal (cabinet_id, dossier_id, type, valeur) \
         VALUES ($1, $2, 'NINEA', '0012345'), ($1, $2, 'RCCM', 'SN-DKR-2020-B-1')",
    )
    .bind(b.c.cabinet.uuid())
    .bind(b.c.dossier.uuid())
    .execute(&b.env.admin)
    .await
    .unwrap();
    // Le NINEA n'est pas admis au Cameroun, et aucune liste de remplacement n'est fournie : 422.
    let r = modifier(
        &b,
        b.c.administrateur,
        r#"{"pays":"CM","devise":"XAF"}"#,
        None,
    )
    .await;
    assert_eq!(r.statut, StatusCode::UNPROCESSABLE_ENTITY, "{}", r.texte());
    // Avec une liste de remplacement admise : le pays, la devise et les identifiants changent.
    let r = modifier(
        &b,
        b.c.administrateur,
        r#"{"pays":"CM","devise":"XAF","identifiants_legaux":[{"type":"RCCM","valeur":"CM-DLA-2021-B-9"}]}"#,
        None,
    )
    .await;
    assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
    let v = r.json();
    assert_eq!(v["pays"], "CM");
    assert_eq!(v["identifiants_legaux"].as_array().unwrap().len(), 1);
    assert_eq!(v["identifiants_legaux"][0]["valeur"], "CM-DLA-2021-B-9");
    let n: i64 =
        sqlx::query_scalar("SELECT count(*) FROM dossier_identifiant_legal WHERE dossier_id = $1")
            .bind(b.c.dossier.uuid())
            .fetch_one(&b.env.admin)
            .await
            .unwrap();
    assert_eq!(n, 1);
    // Audit : les types avant et après, jamais les valeurs.
    let audit: Vec<String> =
        sqlx::query_scalar("SELECT contenu FROM audit_dossier WHERE dossier_id = $1")
            .bind(b.c.dossier.uuid())
            .fetch_all(&b.env.admin)
            .await
            .unwrap();
    let entree = audit
        .iter()
        .find(|c| c.contains("identifiants_legaux"))
        .expect("audit");
    assert!(
        entree.contains("NINEA,RCCM") && entree.contains("RCCM"),
        "{entree}"
    );
    assert!(
        !entree.contains("0012345") && !entree.contains("CM-DLA-2021-B-9"),
        "{entree}"
    );
    // La même liste redite : rien à changer. Un type inconnu du pays : 422.
    let r = modifier(
        &b,
        b.c.administrateur,
        r#"{"identifiants_legaux":[{"type":"RCCM","valeur":"CM-DLA-2021-B-9"}]}"#,
        None,
    )
    .await;
    assert_eq!(code(&r), "aucun_changement");
    let r = modifier(
        &b,
        b.c.administrateur,
        r#"{"identifiants_legaux":[{"type":"NINEA","valeur":"1"}]}"#,
        None,
    )
    .await;
    assert_eq!(code(&r), "contexte_dossier_invalide");
    // Une écriture validée : plus aucun remplacement (409), le fuseau reste modifiable.
    valider(
        &b,
        &[
            ligne(caisse(), "debit", "1000"),
            ligne(banque(), "credit", "1000"),
        ],
    )
    .await;
    let r = modifier(
        &b,
        b.c.administrateur,
        r#"{"identifiants_legaux":[]}"#,
        None,
    )
    .await;
    assert_eq!(r.statut, StatusCode::CONFLICT, "{}", r.texte());
    assert_eq!(code(&r), "dossier_deja_ecritures");
    assert_eq!(
        modifier(
            &b,
            b.c.administrateur,
            r#"{"fuseau":"Africa/Douala"}"#,
            None
        )
        .await
        .statut,
        StatusCode::OK
    );
}

/// Les verrous de la course : une saisie tient `FOR SHARE` sur la ligne du dossier jusqu'à sa
/// validation en base, la modification prend `FOR NO KEY UPDATE` ; chacun attend l'autre.
#[tokio::test]
async fn la_saisie_et_la_modification_se_serialisent_sur_la_ligne_du_dossier() {
    let b = banc().await;
    let ouvrir = || async {
        ouvrir_transaction(
            &b.env.application,
            ContexteTenant::dossier(b.c.cabinet, b.c.dossier, b.c.administrateur),
        )
        .await
        .unwrap()
    };
    let verrou = |mode: &'static str| {
        format!(
            "SELECT 1 FROM dossier WHERE cabinet_id = '{}' AND id = '{}' FOR {mode}",
            b.c.cabinet.uuid(),
            b.c.dossier.uuid()
        )
    };
    let essayer = |mut tx: api::base::TransactionTenant, mode: &'static str| {
        let requete = verrou(mode);
        async move {
            sqlx::query("SET LOCAL lock_timeout = '300ms'")
                .execute(tx.connexion())
                .await
                .unwrap();
            let r = sqlx::query(sqlx::AssertSqlSafe(requete))
                .execute(tx.connexion())
                .await;
            tx.annuler().await.unwrap();
            r.is_ok()
        }
    };
    // Une saisie en cours (SHARE) retient la modification (NO KEY UPDATE) ; deux saisies se
    // partagent la ligne.
    let mut saisie = ouvrir().await;
    sqlx::query(sqlx::AssertSqlSafe(verrou("SHARE")))
        .execute(saisie.connexion())
        .await
        .unwrap();
    assert!(
        !essayer(ouvrir().await, "NO KEY UPDATE").await,
        "la modification attend la saisie"
    );
    assert!(
        essayer(ouvrir().await, "SHARE").await,
        "deux saisies se partagent la ligne"
    );
    saisie.annuler().await.unwrap();
    // Une modification en cours retient les saisies, mais pas les clés étrangères (KEY SHARE).
    let mut modification = ouvrir().await;
    sqlx::query(sqlx::AssertSqlSafe(verrou("NO KEY UPDATE")))
        .execute(modification.connexion())
        .await
        .unwrap();
    assert!(
        !essayer(ouvrir().await, "SHARE").await,
        "la saisie attend la modification"
    );
    assert!(
        essayer(ouvrir().await, "KEY SHARE").await,
        "une clé étrangère passe"
    );
    modification.annuler().await.unwrap();
}

/// Course de bout en bout : une saisie de brouillon et un changement de devise lancés ensemble.
/// Quel que soit l'ordre, aucun brouillon ne reste saisi sous une autre devise que celle du dossier.
#[tokio::test]
async fn aucune_course_ne_laisse_un_brouillon_dans_la_mauvaise_devise() {
    for _ in 0..12 {
        let b = banc().await;
        let lignes = [
            ligne(caisse(), "debit", "1000"),
            ligne(banque(), "credit", "1000"),
        ];
        let saisie = poster(&b, &lignes);
        let changement = modifier(
            &b,
            b.c.administrateur,
            r#"{"pays":"CD","devise":"CDF"}"#,
            None,
        );
        let (s, m) = tokio::join!(saisie, changement);
        assert!(matches!(s.statut, StatusCode::CREATED), "{}", s.texte());
        let devise_du_dossier: String =
            sqlx::query_scalar("SELECT devise_tenue FROM dossier WHERE id = $1")
                .bind(b.c.dossier.uuid())
                .fetch_one(&b.env.admin)
                .await
                .unwrap();
        let brouillons: Vec<String> =
            sqlx::query_scalar("SELECT devise_tenue FROM brouillon WHERE dossier_id = $1")
                .bind(b.c.dossier.uuid())
                .fetch_all(&b.env.admin)
                .await
                .unwrap();
        assert_eq!(brouillons.len(), 1);
        assert!(
            brouillons.iter().all(|d| *d == devise_du_dossier),
            "brouillon en {brouillons:?}, dossier en {devise_du_dossier}"
        );
        // Soit la modification est passée avant la saisie (le brouillon est en CDF), soit la
        // saisie l'a précédée et la modification est refusée.
        if m.statut == StatusCode::OK {
            assert_eq!(devise_du_dossier, "CDF");
        } else {
            assert_eq!(code(&m), "dossier_brouillard_non_vide", "{}", m.texte());
            assert_eq!(devise_du_dossier, "XOF");
        }
    }
}

/// Un brouillon saisi sous une autre devise ou une autre échelle que celles du dossier ne se
/// valide pas, dans les deux sens (le contexte est changé ici en contournant les barrières).
#[tokio::test]
async fn un_brouillon_d_une_autre_echelle_ne_se_valide_pas() {
    for (avant, apres, montant) in [
        (
            ("SN", "XOF", 0_i16, "Africa/Dakar"),
            ("CD", "CDF", 2_i16, "Africa/Kinshasa"),
            "1000",
        ),
        (
            ("CD", "CDF", 2_i16, "Africa/Kinshasa"),
            ("SN", "XOF", 0_i16, "Africa/Dakar"),
            "1000.55",
        ),
    ] {
        let b = banc().await;
        tenir_en(&b, avant.0, avant.1, avant.2, avant.3).await;
        let (id, version) = b
            .pret_avec(&contenu(
                &journal_de_saisie(),
                &aujourd(),
                &[
                    ligne(caisse(), "debit", montant),
                    ligne(banque(), "credit", montant),
                ],
                "",
            ))
            .await;
        // Le propriétaire contourne le déclencheur (`session_replication_role`) pour changer la
        // devise sous le brouillon, comme une course ou une erreur d'exploitation.
        let mut conn = b.env.admin.acquire().await.unwrap();
        sqlx::query("SET session_replication_role = replica")
            .execute(&mut *conn)
            .await
            .unwrap();
        sqlx::query("UPDATE dossier SET pays = $3, devise_tenue = $4, decimales_tenue = $5, fuseau = $6 WHERE cabinet_id = $1 AND id = $2")
            .bind(b.c.cabinet.uuid())
            .bind(b.c.dossier.uuid())
            .bind(apres.0)
            .bind(apres.1)
            .bind(apres.2)
            .bind(apres.3)
            .execute(&mut *conn)
            .await
            .unwrap();
        sqlx::query("RESET session_replication_role")
            .execute(&mut *conn)
            .await
            .unwrap();
        drop(conn);
        let r = b.valider(id, version).await;
        assert_eq!(r.statut, StatusCode::CONFLICT, "{}", r.texte());
        assert_eq!(code(&r), "brouillon_devise_tenue_perimee");
        assert_eq!(b.compter("ecriture_validee").await, 0);
    }
}
