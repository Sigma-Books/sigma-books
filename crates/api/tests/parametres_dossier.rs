//! Versions de paramètres fiscaux propres à un dossier (ADR 0030) : création, lecture, règles,
//! audit sans donnée libre, ajout seul, sélection de bout en bout (saisie, validation, relecture,
//! contre-passation), deux dossiers d'un même cabinet, cabinets distincts.
#![cfg(feature = "tests-postgres")]
// Fichier de test : les `expect`/`unwrap` y sont voulus.
#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::too_many_lines,
    clippy::many_single_char_names
)]

mod commun;

use api::base::{IdCabinet, IdDossier};
use axum::http::StatusCode;
use commun::banc::{Banc, banc, banque, caisse, code, contenu};
use commun::brouillard::{aujourd_hui, journal_de_saisie, ligne, texte_date};
use commun::serveur::{Reponse, envoyer, lire};
use serde_json::{Value, json};
use sqlx::Row;
use time::Date;
use uuid::Uuid;

/// Date de `n` jours après aujourd'hui, au format `AAAA-MM-JJ` (les dates d'effet sont bornées à
/// un an : les tests ne dépendent pas de l'année courante).
fn t(n: i64) -> String {
    texte_date(aujourd_hui() + time::Duration::days(n))
}

fn jour(annee: i32, mois: u8, j: u8) -> Date {
    Date::from_calendar_date(annee, time::Month::try_from(mois).unwrap(), j).unwrap()
}

fn chemin(b: &Banc, dossier: IdDossier) -> String {
    format!(
        "/api/cabinets/{}/dossiers/{dossier}/parametres-fiscaux",
        b.c.cabinet
    )
}

async fn creer(b: &Banc, dossier: IdDossier, corps: &Value) -> Reponse {
    envoyer(
        &b.routeur,
        &chemin(b, dossier),
        Some(b.c.administrateur),
        &corps.to_string(),
        None,
    )
    .await
}

async fn liste(b: &Banc, dossier: IdDossier) -> Value {
    let r = lire(&b.routeur, &chemin(b, dossier), Some(b.c.administrateur)).await;
    assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
    r.json()
}

async fn applicable(b: &Banc, dossier: IdDossier, date: &str) -> Value {
    let r = lire(
        &b.routeur,
        &format!("{}/applicable?date={date}", chemin(b, dossier)),
        Some(b.c.administrateur),
    )
    .await;
    assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
    r.json()
}

fn taux_normal(applicable: &Value) -> Option<String> {
    applicable["taux"]
        .as_array()?
        .iter()
        .find(|t| t["role"] == "normal")
        .map(|t| t["valeur"].as_str().unwrap().to_owned())
}

#[tokio::test]
async fn la_lecture_rend_le_paquet_du_pays_et_la_version_applicable() {
    let b = banc().await;
    let l = liste(&b, b.c.dossier).await;
    assert_eq!(l["pays"], "SN");
    // Paquet du Sénégal : 2025.1, puis 2025.2 (loi n° 2025-17, prélèvements).
    assert_eq!(l["versions"].as_array().unwrap().len(), 2);
    assert_eq!(l["versions"][0]["identifiant"], "sn-2025.1");
    assert_eq!(l["versions"][0]["portee"], "pays");
    assert_eq!(l["versions"][1]["identifiant"], "sn-2025.2");
    assert_eq!(l["versions"][1]["portee"], "pays");
    let a = applicable(&b, b.c.dossier, "2026-03-15").await;
    assert_eq!(a["version"]["identifiant"], "sn-2025.2");
    assert_eq!(taux_normal(&a).as_deref(), Some("18"));
    assert_eq!(a["regimes"].as_array().unwrap().len(), 13);
    assert_eq!(a["retention_annees"], 10);
    // Avant la première version du pays : aucune version.
    let avant = applicable(&b, b.c.dossier, "2024-06-15").await;
    assert!(avant["version"].is_null());
    // Sans date : le jour du dossier ; paramètre inconnu ou date mal formée : 400.
    let r = lire(
        &b.routeur,
        &format!("{}/applicable", chemin(&b, b.c.dossier)),
        Some(b.c.administrateur),
    )
    .await;
    assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
    for mauvais in ["?date=2026-13-01", "?jour=2026-01-01", "?date=2026-1-1&x=1"] {
        let r = lire(
            &b.routeur,
            &format!("{}/applicable{mauvais}", chemin(&b, b.c.dossier)),
            Some(b.c.administrateur),
        )
        .await;
        assert_eq!(r.statut, StatusCode::BAD_REQUEST, "{mauvais}");
    }
    let r = lire(
        &b.routeur,
        &format!("{}?x=1", chemin(&b, b.c.dossier)),
        Some(b.c.administrateur),
    )
    .await;
    assert_eq!(r.statut, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn une_version_propre_se_cree_se_lit_et_s_audite_sans_le_motif() {
    let b = banc().await;
    let motif = "Convention particulière du client Dupont";
    let r = creer(
        &b,
        b.c.dossier,
        &json!({"date_effet":t(100),"motif":motif,"taux":{"normal":"19"},"regimes_retires":["MARGE"]}),
    )
    .await;
    assert_eq!(r.statut, StatusCode::CREATED, "{}", r.texte());
    let v = r.json();
    assert_eq!(v["identifiant"], "sn-2025.2+d1");
    assert_eq!(
        (v["portee"].as_str(), v["numero"].as_i64()),
        (Some("dossier"), Some(1))
    );
    assert_eq!(v["empreinte"].as_str().unwrap().len(), 64);
    assert_eq!(v["motif"], motif);
    assert_eq!(v["cree_par"], b.c.administrateur.to_string());
    // Lecture : les deux versions du paquet, puis celle du dossier.
    let l = liste(&b, b.c.dossier).await;
    assert_eq!(l["versions"].as_array().unwrap().len(), 3);
    assert_eq!(l["versions"][2]["identifiant"], "sn-2025.2+d1");
    // Application par date : la version du dossier prend effet le 1er janvier 2027 et seulement
    // pour ce dossier ; le paquet du pays n'a pas bougé.
    let avant = applicable(&b, b.c.dossier, &t(99)).await;
    assert_eq!(avant["version"]["identifiant"], "sn-2025.2");
    let apres = applicable(&b, b.c.dossier, &t(150)).await;
    assert_eq!(apres["version"]["identifiant"], "sn-2025.2+d1");
    assert_eq!(taux_normal(&apres).as_deref(), Some("19"));
    assert_eq!(apres["regimes"].as_array().unwrap().len(), 12);
    let autre = applicable(&b, b.c.autre_dossier, &t(150)).await;
    assert_eq!(autre["version"]["identifiant"], "sn-2025.2");
    // Audit chaîné : identifiants, dates, codes et empreinte ; jamais le motif ni un nom.
    let audit: Vec<String> =
        sqlx::query_scalar("SELECT contenu FROM audit_dossier WHERE dossier_id = $1 ORDER BY rang")
            .bind(b.c.dossier.uuid())
            .fetch_all(&b.env.admin)
            .await
            .unwrap();
    let entree = audit
        .iter()
        .find(|c| c.contains("CreerVersionParametresFiscaux"))
        .expect("audit");
    assert!(
        entree.contains("sn-2025.2+d1") && entree.contains(&t(100)),
        "{entree}"
    );
    assert!(
        entree.contains("MARGE") && entree.contains("normal"),
        "{entree}"
    );
    assert!(
        entree.contains(v["empreinte"].as_str().unwrap()),
        "{entree}"
    );
    assert!(
        !entree.contains("Dupont") && !entree.contains(motif),
        "{entree}"
    );
    // Ligne stockée : contenu relu, empreinte du contenu.
    let ligne = sqlx::query(
        "SELECT contenu, motif, pays, version FROM parametres_fiscaux_dossier WHERE dossier_id = $1",
    )
    .bind(b.c.dossier.uuid())
    .fetch_one(&b.env.admin)
    .await
    .unwrap();
    let contenu: String = ligne.get("contenu");
    assert!(contenu.contains("valeur = \"19\""));
    assert_eq!(ligne.get::<String, _>("motif"), motif);
}

#[tokio::test]
async fn les_demandes_invalides_sont_refusees_avec_un_code_stable() {
    let b = banc().await;
    let cas: Vec<(Value, StatusCode, &str)> = vec![
        (
            json!({"date_effet":t(100),"motif":"x"}),
            StatusCode::UNPROCESSABLE_ENTITY,
            "parametres_aucun_changement",
        ),
        (
            json!({"date_effet":t(100),"motif":"x","taux":{"normal":"0"}}),
            StatusCode::UNPROCESSABLE_ENTITY,
            "parametres_taux_invalide",
        ),
        (
            json!({"date_effet":t(100),"motif":"x","taux":{"normal":"100.5"}}),
            StatusCode::UNPROCESSABLE_ENTITY,
            "parametres_taux_invalide",
        ),
        (
            json!({"date_effet":t(100),"motif":"x","taux":{"normal":"dix"}}),
            StatusCode::UNPROCESSABLE_ENTITY,
            "parametres_taux_invalide",
        ),
        (
            json!({"date_effet":t(100),"motif":"x","taux":{"exotique":"5"}}),
            StatusCode::UNPROCESSABLE_ENTITY,
            "parametres_role_inconnu",
        ),
        (
            json!({"date_effet":t(100),"motif":"x","regimes_retires":["NOUVEAU"]}),
            StatusCode::UNPROCESSABLE_ENTITY,
            "parametres_regime_inconnu",
        ),
        (
            json!({"date_effet":t(100),"motif":"x","regimes_retires":["IMP18"]}),
            StatusCode::UNPROCESSABLE_ENTITY,
            "parametres_regime_inconnu",
        ),
        (
            json!({"date_effet":t(100),"motif":"x","regimes_retires":["EXO_SANS_DEDUCTION"]}),
            StatusCode::UNPROCESSABLE_ENTITY,
            "parametres_regime_non_retirable",
        ),
        (
            json!({"date_effet":t(100),"motif":" ","taux":{"normal":"19"}}),
            StatusCode::UNPROCESSABLE_ENTITY,
            "parametres_motif_invalide",
        ),
        (
            json!({"date_effet":t(100),"motif":"é".repeat(201),"taux":{"normal":"19"}}),
            StatusCode::UNPROCESSABLE_ENTITY,
            "parametres_motif_invalide",
        ),
        (
            json!({"date_effet":t(100),"motif":"a\nb","taux":{"normal":"19"}}),
            StatusCode::UNPROCESSABLE_ENTITY,
            "parametres_motif_invalide",
        ),
        (
            json!({"date_effet":t(100),"date_fin":t(99),"motif":"x","taux":{"normal":"19"}}),
            StatusCode::UNPROCESSABLE_ENTITY,
            "parametres_periode_invalide",
        ),
        (
            json!({"date_effet":"2024-06-01","motif":"x","taux":{"normal":"19"}}),
            StatusCode::UNPROCESSABLE_ENTITY,
            "parametres_aucune_base",
        ),
        // Forme : champ inconnu, date mal formée ou absente, motif absent, trop de taux.
        (
            json!({"date_effet":t(100),"motif":"x","taux":{"normal":"19"},"autre":1}),
            StatusCode::BAD_REQUEST,
            "requete_invalide",
        ),
        (
            json!({"date_effet":"2027-13-01","motif":"x","taux":{"normal":"19"}}),
            StatusCode::BAD_REQUEST,
            "requete_invalide",
        ),
        (
            json!({"motif":"x","taux":{"normal":"19"}}),
            StatusCode::BAD_REQUEST,
            "requete_invalide",
        ),
        (
            json!({"date_effet":t(100),"taux":{"normal":"19"}}),
            StatusCode::BAD_REQUEST,
            "requete_invalide",
        ),
    ];
    for (corps, statut, motif) in cas {
        let r = creer(&b, b.c.dossier, &corps).await;
        assert_eq!(r.statut, statut, "{corps} : {}", r.texte());
        assert_eq!(code(&r), motif, "{corps}");
    }
    // Beaucoup de régimes ou de taux : 400, sans rien créer.
    let trop: Vec<String> = (0..40).map(|i| format!("R{i}")).collect();
    let r = creer(
        &b,
        b.c.dossier,
        &json!({"date_effet":t(100),"motif":"x","regimes_retires":trop}),
    )
    .await;
    assert_eq!(r.statut, StatusCode::BAD_REQUEST);
    assert_eq!(b.compter("parametres_fiscaux_dossier").await, 0);
    // Un motif de 400 octets exactement est admis.
    let r = creer(
        &b,
        b.c.dossier,
        &json!({"date_effet":t(100),"motif":"a".repeat(400),"taux":{"normal":"19"}}),
    )
    .await;
    assert_eq!(r.statut, StatusCode::CREATED, "{}", r.texte());
}

#[tokio::test]
async fn les_conflits_d_etat_sont_refuses() {
    let b = banc().await;
    // Date d'effet jamais antérieure ni égale à la dernière version du dossier.
    let v1 = json!({"date_effet":t(200),"motif":"Première","taux":{"normal":"19"}});
    assert_eq!(
        creer(&b, b.c.dossier, &v1).await.statut,
        StatusCode::CREATED
    );
    for date in [t(200), t(100)] {
        let r = creer(
            &b,
            b.c.dossier,
            &json!({"date_effet":date,"motif":"Seconde","taux":{"normal":"20"}}),
        )
        .await;
        assert_eq!(r.statut, StatusCode::CONFLICT, "{date}: {}", r.texte());
        assert_eq!(code(&r), "parametres_date_effet_anterieure");
    }
    // Une date postérieure est admise ; la version dérive alors de la précédente (taux 19 gardé).
    let r = creer(
        &b,
        b.c.dossier,
        &json!({"date_effet":t(300),"motif":"Seconde","regimes_retires":["MARGE"]}),
    )
    .await;
    assert_eq!(r.statut, StatusCode::CREATED, "{}", r.texte());
    assert_eq!(r.json()["identifiant"], "sn-2025.2+d2");
    let a = applicable(&b, b.c.dossier, &t(310)).await;
    assert_eq!(taux_normal(&a).as_deref(), Some("19"));
    assert_eq!(a["regimes"].as_array().unwrap().len(), 12);
    // Période clôturée : la période du jour et celles qui la précèdent.
    let autre = &b.c.autre_dossier;
    commun::referentiel::doter(&b.env.application, b.c.cabinet, *autre, b.c.administrateur).await;
    sqlx::query(
        "UPDATE periode SET statut = 'cloturee', statut_par = $2, statut_le = now() \
         WHERE dossier_id = $1 AND debut <= $3",
    )
    .bind(autre.uuid())
    .bind(b.c.administrateur.uuid())
    .bind(aujourd_hui())
    .execute(&b.env.admin)
    .await
    .unwrap();
    let r = creer(
        &b,
        *autre,
        &json!({"date_effet":texte_date(aujourd_hui()),"motif":"x","taux":{"normal":"19"}}),
    )
    .await;
    assert_eq!(r.statut, StatusCode::CONFLICT, "{}", r.texte());
    assert_eq!(code(&r), "parametres_date_effet_cloturee");
    // Pays sans paquet.
    let mut tx = b.env.admin.begin().await.unwrap();
    sqlx::query(
        "SELECT set_config('sigma.cabinet', $1, true), set_config('sigma.dossier', $2, true)",
    )
    .bind(b.c.cabinet.to_string())
    .bind(autre.to_string())
    .execute(&mut *tx)
    .await
    .unwrap();
    sqlx::query("UPDATE dossier SET pays = 'GA', devise_tenue = 'XAF', decimales_tenue = 0, fuseau = 'Africa/Libreville' WHERE cabinet_id = $1 AND id = $2")
        .bind(b.c.cabinet.uuid())
        .bind(autre.uuid())
        .execute(&mut *tx)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    let r = creer(
        &b,
        *autre,
        &json!({"date_effet":t(100),"motif":"x","taux":{"normal":"19"}}),
    )
    .await;
    assert_eq!(r.statut, StatusCode::UNPROCESSABLE_ENTITY, "{}", r.texte());
    assert_eq!(code(&r), "parametres_pays_sans_paquet");
}

async fn ecriture_validee_en(b: &Banc, date: &str, regime: &str, taux: &str) {
    let etiquettee = format!(
        r#"{{"compte":"{}","sens":"debit","montant":"1000","etiquette":{{"nature":"base","sens":"collectee","regime":"{regime}","taux":"{taux}","exigibilite":"debit"}}}}"#,
        caisse()
    );
    let corps = contenu(
        &journal_de_saisie(),
        date,
        &[etiquettee, ligne(banque(), "credit", "1000")],
        "",
    );
    let (id, v) = b.pret_avec(&corps).await;
    let r = b.valider(id, v).await;
    assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
}

#[tokio::test]
async fn un_regime_utilise_dans_la_periode_d_application_n_est_jamais_retire() {
    let b = banc().await;
    let annee = aujourd_hui().year();
    let mars = texte_date(jour(annee, 3, 15));
    ecriture_validee_en(&b, &mars, "TAXABLE_REDUIT", "10").await;
    // Retrait à partir d'avant l'écriture, ou de sa date : refusé.
    for effet in [texte_date(jour(annee, 3, 1)), mars.clone()] {
        let r = creer(
            &b,
            b.c.dossier,
            &json!({"date_effet":effet,"motif":"x","regimes_retires":["TAXABLE_REDUIT"]}),
        )
        .await;
        assert_eq!(r.statut, StatusCode::CONFLICT, "{effet} : {}", r.texte());
        assert_eq!(code(&r), "parametres_regime_utilise");
    }
    // Avec une fin antérieure à l'écriture : elle n'est pas dans la période d'application.
    let r = creer(
        &b,
        b.c.dossier,
        &json!({"date_effet":texte_date(jour(annee, 1, 10)),"date_fin":texte_date(jour(annee, 2, 1)),"motif":"x","regimes_retires":["TAXABLE_REDUIT"]}),
    )
    .await;
    assert_eq!(r.statut, StatusCode::CREATED, "{}", r.texte());
    assert_eq!(b.compter("parametres_fiscaux_dossier").await, 1);
}

#[tokio::test]
async fn un_regime_utilise_seulement_avant_la_date_d_effet_peut_etre_retire() {
    let b = banc().await;
    let annee = aujourd_hui().year();
    ecriture_validee_en(&b, &texte_date(jour(annee, 3, 15)), "TAXABLE_REDUIT", "10").await;
    // Retrait à partir de demain : l'écriture de mars garde sa version enregistrée.
    let r = creer(
        &b,
        b.c.dossier,
        &json!({"date_effet":t(100),"motif":"x","regimes_retires":["TAXABLE_REDUIT"]}),
    )
    .await;
    assert_eq!(r.statut, StatusCode::CREATED, "{}", r.texte());
    assert_eq!(r.json()["avertissements"], json!([]));
    // L'écriture se relit (détail, éditions) et se contre-passe sous sa version d'origine.
    let id: String =
        sqlx::query_scalar("SELECT id::text FROM ecriture_validee WHERE dossier_id = $1")
            .bind(b.c.dossier.uuid())
            .fetch_one(&b.env.admin)
            .await
            .unwrap();
    let lue = lire(
        &b.routeur,
        &format!("{}/{id}", b.ecritures()),
        Some(b.auditeur),
    )
    .await;
    assert_eq!(lue.json()["version_parametres"], "sn-2025.2");
    let e = lire(
        &b.routeur,
        &format!(
            "/api/cabinets/{}/dossiers/{}/editions/balance?mode=definitif",
            b.c.cabinet, b.c.dossier
        ),
        Some(b.auditeur),
    )
    .await;
    assert_eq!(e.statut, StatusCode::OK, "{}", e.texte());
}

#[tokio::test]
async fn une_version_retroactive_avertit_et_ne_change_pas_les_ecritures_validees() {
    let b = banc().await;
    let annee = aujourd_hui().year();
    let (mars, juin) = (
        texte_date(jour(annee, 3, 15)),
        texte_date(jour(annee, 6, 15)),
    );
    ecriture_validee_en(&b, &mars, "TAXABLE_NORMAL", "18").await;
    // Version rétroactive au 1er mars, en période ouverte, alors qu'une écriture de mars est
    // validée : acceptée, avec un avertissement (nombre d'écritures déjà validées dans la période).
    let r = creer(&b, b.c.dossier, &json!({"date_effet":texte_date(jour(annee, 3, 1)),"motif":"Rétroactif","taux":{"normal":"19"}})).await;
    assert_eq!(r.statut, StatusCode::CREATED, "{}", r.texte());
    assert_eq!(
        r.json()["avertissements"],
        json!([{"code":"ecritures_deja_validees","nombre":1}])
    );
    let audit: Vec<String> =
        sqlx::query_scalar("SELECT contenu FROM audit_dossier WHERE dossier_id = $1")
            .bind(b.c.dossier.uuid())
            .fetch_all(&b.env.admin)
            .await
            .unwrap();
    assert!(audit.iter().any(|c| c.contains("ecritures_deja_validees")));
    // L'écriture validée garde sa version ; une écriture ultérieure de la MÊME période est sous la
    // nouvelle (deux taux dans un même mois : 18 % le 15, 19 % le 20) — comportement documenté.
    let id: String =
        sqlx::query_scalar("SELECT id::text FROM ecriture_validee WHERE dossier_id = $1")
            .bind(b.c.dossier.uuid())
            .fetch_one(&b.env.admin)
            .await
            .unwrap();
    let lue = lire(
        &b.routeur,
        &format!("{}/{id}", b.ecritures()),
        Some(b.auditeur),
    )
    .await;
    assert_eq!(lue.json()["version_parametres"], "sn-2025.2");
    let apres_mars = texte_date(jour(annee, 3, 20));
    let r = saisir(
        &b,
        b.c.cabinet,
        b.c.dossier,
        b.c.collaborateur,
        &corps_du(&apres_mars, "19"),
    )
    .await;
    assert_eq!(r.json()["version_parametres"], "sn-2025.2+d1");
    let r = saisir(
        &b,
        b.c.cabinet,
        b.c.dossier,
        b.c.collaborateur,
        &corps_du(&apres_mars, "18"),
    )
    .await;
    assert_eq!(code(&r), "etiquette_taux_non_admis");
    // Éditions et contre-passation de l'écriture de mars : sous l'ancienne version.
    let e = lire(
        &b.routeur,
        &format!(
            "/api/cabinets/{}/dossiers/{}/editions/balance?mode=definitif",
            b.c.cabinet, b.c.dossier
        ),
        Some(b.auditeur),
    )
    .await;
    assert_eq!(e.statut, StatusCode::OK, "{}", e.texte());
    let cp = envoyer(
        &b.routeur,
        &format!("{}/{id}/contre-passation", b.ecritures()),
        Some(b.valideur),
        &format!(r#"{{"date":"{juin}","libelle":"Contre-passation","motif":"Erreur"}}"#),
        None,
    )
    .await;
    assert_eq!(cp.statut, StatusCode::CREATED, "{}", cp.texte());
    let versions: Vec<Option<String>> = sqlx::query_scalar(
        "SELECT version_parametres FROM ecriture_validee WHERE dossier_id = $1 ORDER BY rang",
    )
    .bind(b.c.dossier.uuid())
    .fetch_all(&b.env.admin)
    .await
    .unwrap();
    assert_eq!(
        versions,
        [Some("sn-2025.2".to_owned()), Some("sn-2025.2".to_owned())]
    );
    assert!(b.chaine_valide().await);
}

#[tokio::test]
async fn un_lot_traverse_deux_versions_et_le_jour_d_effet_est_inclus() {
    let b = banc().await;
    let annee = aujourd_hui().year();
    let effet = texte_date(jour(annee, 7, 1));
    let r = creer(
        &b,
        b.c.dossier,
        &json!({"date_effet":effet,"motif":"x","taux":{"normal":"19"}}),
    )
    .await;
    assert_eq!(r.statut, StatusCode::CREATED, "{}", r.texte());
    // La veille : paquet du pays ; le jour même et après : version du dossier.
    let (veille, jour_meme, apres) = (
        texte_date(jour(annee, 6, 30)),
        effet.clone(),
        texte_date(jour(annee, 8, 15)),
    );
    let (i1, v1) = b.pret_avec(&corps_du(&veille, "18")).await;
    let (i2, v2) = b.pret_avec(&corps_du(&jour_meme, "19")).await;
    let (i3, v3) = b.pret_avec(&corps_du(&apres, "19")).await;
    let liste: Vec<String> = [(i1, v1), (i2, v2), (i3, v3)]
        .iter()
        .map(|(id, v)| format!(r#"{{"ecriture":"{id}","version":{v}}}"#))
        .collect();
    let lot = envoyer(
        &b.routeur,
        &format!("{}/validation-par-lot", b.base()),
        Some(b.valideur),
        &format!(r#"{{"ecritures":[{}]}}"#, liste.join(",")),
        None,
    )
    .await;
    assert_eq!(lot.statut, StatusCode::OK, "{}", lot.texte());
    let versions: Vec<Option<String>> = sqlx::query_scalar("SELECT version_parametres FROM ecriture_validee WHERE dossier_id = $1 ORDER BY date_comptable")
        .bind(b.c.dossier.uuid())
        .fetch_all(&b.env.admin)
        .await
        .unwrap();
    let attendues: Vec<Option<String>> = ["sn-2025.2", "sn-2025.2+d1", "sn-2025.2+d1"]
        .iter()
        .map(|v| Some((*v).to_owned()))
        .collect();
    assert_eq!(versions, attendues);
    assert!(b.chaine_valide().await);
    // Les mêmes bornes à la saisie : la veille 19 % refusé, le jour même 18 % refusé.
    assert_eq!(
        code(
            &saisir(
                &b,
                b.c.cabinet,
                b.c.dossier,
                b.c.collaborateur,
                &corps_du(&veille, "19")
            )
            .await
        ),
        "etiquette_taux_non_admis"
    );
    assert_eq!(
        code(
            &saisir(
                &b,
                b.c.cabinet,
                b.c.dossier,
                b.c.collaborateur,
                &corps_du(&jour_meme, "18")
            )
            .await
        ),
        "etiquette_taux_non_admis"
    );
}

#[tokio::test]
async fn le_nombre_de_versions_est_borne() {
    let b = banc().await;
    for n in 1..=50_i32 {
        sqlx::query(
            "INSERT INTO parametres_fiscaux_dossier \
                 (cabinet_id, dossier_id, id, numero, pays, version, date_effet, contenu, empreinte, motif, cree_par) \
             VALUES ($1, $2, gen_random_uuid(), $3, 'SN', '2025.1', '2100-01-01'::date + $3, 'x', decode(repeat('00', 32), 'hex'), 'x', $4)",
        )
        .bind(b.c.cabinet.uuid())
        .bind(b.c.dossier.uuid())
        .bind(n)
        .bind(b.c.administrateur.uuid())
        .execute(&b.env.admin)
        .await
        .unwrap();
    }
    let r = creer(
        &b,
        b.c.dossier,
        &json!({"date_effet":t(100),"motif":"x","taux":{"normal":"19"}}),
    )
    .await;
    assert_eq!(r.statut, StatusCode::CONFLICT, "{}", r.texte());
    assert_eq!(code(&r), "parametres_trop_de_versions");
    // Seconde barrière : la base refuse un 51e numéro.
    let direct = sqlx::query(
        "INSERT INTO parametres_fiscaux_dossier \
             (cabinet_id, dossier_id, id, numero, pays, version, date_effet, contenu, empreinte, motif, cree_par) \
         VALUES ($1, $2, gen_random_uuid(), 51, 'SN', '2025.1', '2300-01-01', 'x', decode(repeat('00', 32), 'hex'), 'x', $3)",
    )
    .bind(b.c.cabinet.uuid())
    .bind(b.c.dossier.uuid())
    .bind(b.c.administrateur.uuid())
    .execute(&b.env.admin)
    .await;
    assert!(direct.is_err());
}

#[tokio::test]
async fn droits_et_reauthentification() {
    let b = banc().await;
    let corps = json!({"date_effet":t(100),"motif":"x","taux":{"normal":"19"}}).to_string();
    // Seul l'administrateur crée ; les autres rôles lisent (collaborateur et auditeur compris).
    for u in [b.valideur, b.c.collaborateur, b.auditeur] {
        let r = envoyer(&b.routeur, &chemin(&b, b.c.dossier), Some(u), &corps, None).await;
        assert_eq!(r.statut, StatusCode::FORBIDDEN, "{}", r.texte());
    }
    for u in [b.valideur, b.c.collaborateur, b.auditeur] {
        let r = lire(&b.routeur, &chemin(&b, b.c.dossier), Some(u)).await;
        assert_eq!(r.statut, StatusCode::OK);
    }
    // Session non fraîche (dernière vérification d'il y a une heure) : refusée.
    let r = envoyer(
        &b.routeur,
        &chemin(&b, b.c.dossier),
        Some(b.c.administrateur),
        &corps,
        Some(3600),
    )
    .await;
    assert_eq!(r.statut, StatusCode::FORBIDDEN, "{}", r.texte());
    assert_eq!(b.compter("parametres_fiscaux_dossier").await, 0);
    // Sans authentification : 401.
    let r = envoyer(&b.routeur, &chemin(&b, b.c.dossier), None, &corps, None).await;
    assert_eq!(r.statut, StatusCode::UNAUTHORIZED);
    // Aucune route de modification ni de suppression : la méthode n'existe pas.
    for methode in ["PUT", "PATCH", "DELETE"] {
        let requete = axum::http::Request::builder()
            .method(methode)
            .uri(chemin(&b, b.c.dossier))
            .header("origin", commun::serveur::ORIGINE)
            .header("sec-fetch-site", "same-origin")
            .header("x-sigma-csrf", "1")
            .header(
                commun::serveur::EN_TETE_UTILISATEUR,
                b.c.administrateur.to_string(),
            )
            .body(axum::body::Body::empty())
            .unwrap();
        let r = commun::serveur::appeler(&b.routeur, requete).await;
        assert!(
            r.statut == StatusCode::METHOD_NOT_ALLOWED || r.statut == StatusCode::NOT_FOUND,
            "{methode} : {}",
            r.statut
        );
    }
}

#[tokio::test]
async fn la_table_est_en_ajout_seul_pour_le_role_applicatif() {
    let b = banc().await;
    let r = creer(
        &b,
        b.c.dossier,
        &json!({"date_effet":t(100),"motif":"x","taux":{"normal":"19"}}),
    )
    .await;
    assert_eq!(r.statut, StatusCode::CREATED);
    for requete in [
        "UPDATE parametres_fiscaux_dossier SET motif = 'altéré'",
        "DELETE FROM parametres_fiscaux_dossier",
    ] {
        let refus = sqlx::query(requete).execute(&b.env.application).await;
        assert!(refus.is_err(), "{requete}");
    }
    assert_eq!(b.compter("parametres_fiscaux_dossier").await, 1);
}

// --- Sélection de bout en bout -----------------------------------------------------------

fn etiquetee(taux: &str) -> String {
    format!(
        r#"{{"compte":"{}","sens":"debit","montant":"1000","etiquette":{{"nature":"base","sens":"collectee","regime":"TAXABLE_NORMAL","taux":"{taux}","exigibilite":"debit"}}}}"#,
        caisse()
    )
}

fn corps_du(date: &str, taux: &str) -> String {
    contenu(
        &journal_de_saisie(),
        date,
        &[etiquetee(taux), ligne(banque(), "credit", "1000")],
        "",
    )
}

async fn saisir(
    b: &Banc,
    cabinet: IdCabinet,
    dossier: IdDossier,
    par: api::base::IdUtilisateur,
    corps: &str,
) -> Reponse {
    envoyer(
        &b.routeur,
        &format!("/api/cabinets/{cabinet}/dossiers/{dossier}/brouillard"),
        Some(par),
        corps,
        None,
    )
    .await
}

#[tokio::test]
async fn la_version_du_dossier_s_applique_de_la_saisie_a_la_contre_passation() {
    let b = banc().await;
    let annee = aujourd_hui().year();
    let r = creer(&b, b.c.dossier, &json!({"date_effet":format!("{annee}-07-01"),"motif":"Taux conventionnel","taux":{"normal":"19"}})).await;
    assert_eq!(r.statut, StatusCode::CREATED, "{}", r.texte());
    let (mars, aout) = (
        texte_date(jour(annee, 3, 15)),
        texte_date(jour(annee, 8, 15)),
    );
    // Avant la date d'effet : le paquet du pays (18) ; après : la version du dossier (19).
    let r = saisir(
        &b,
        b.c.cabinet,
        b.c.dossier,
        b.c.collaborateur,
        &corps_du(&mars, "19"),
    )
    .await;
    assert_eq!(code(&r), "etiquette_taux_non_admis");
    let r = saisir(
        &b,
        b.c.cabinet,
        b.c.dossier,
        b.c.collaborateur,
        &corps_du(&mars, "18"),
    )
    .await;
    assert_eq!(r.json()["version_parametres"], "sn-2025.2");
    let r = saisir(
        &b,
        b.c.cabinet,
        b.c.dossier,
        b.c.collaborateur,
        &corps_du(&aout, "18"),
    )
    .await;
    assert_eq!(code(&r), "etiquette_taux_non_admis");
    let (id, v) = b.pret_avec(&corps_du(&aout, "19")).await;
    // Validation unitaire : la version du dossier est enregistrée dans l'écriture et le haché.
    let r = b.valider(id, v).await;
    assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
    let chaine = b.chaine().await;
    assert!(
        chaine[0].2.contains("\nparametres=sn-2025.2+d1\n"),
        "{}",
        chaine[0].2
    );
    let lue = lire(
        &b.routeur,
        &format!("{}/{id}", b.ecritures()),
        Some(b.auditeur),
    )
    .await;
    assert_eq!(lue.json()["version_parametres"], "sn-2025.2+d1");
    // Relecture : la liste, puis les éditions, rebâtissent l'écriture sous sa version enregistrée.
    let l = lire(&b.routeur, &b.ecritures(), Some(b.auditeur)).await;
    assert_eq!(l.statut, StatusCode::OK, "{}", l.texte());
    let e = lire(
        &b.routeur,
        &format!(
            "/api/cabinets/{}/dossiers/{}/editions/balance?mode=definitif",
            b.c.cabinet, b.c.dossier
        ),
        Some(b.auditeur),
    )
    .await;
    assert_eq!(e.statut, StatusCode::OK, "{}", e.texte());
    // Contre-passation : la version de l'origine est copiée.
    let cp = envoyer(
        &b.routeur,
        &format!("{}/{id}/contre-passation", b.ecritures()),
        Some(b.valideur),
        &format!(r#"{{"date":"{aout}","libelle":"Contre-passation","motif":"Erreur"}}"#),
        None,
    )
    .await;
    assert_eq!(cp.statut, StatusCode::CREATED, "{}", cp.texte());
    let rang2: Option<String> = sqlx::query_scalar(
        "SELECT version_parametres FROM ecriture_validee WHERE dossier_id = $1 AND rang = 2",
    )
    .bind(b.c.dossier.uuid())
    .fetch_one(&b.env.admin)
    .await
    .unwrap();
    assert_eq!(rang2.as_deref(), Some("sn-2025.2+d1"));
    assert!(b.chaine_valide().await);
    // Validation par lot : même sélection.
    let (id2, v2) = b.pret_avec(&corps_du(&aout, "19")).await;
    let lot = envoyer(
        &b.routeur,
        &format!("{}/validation-par-lot", b.base()),
        Some(b.valideur),
        &format!(r#"{{"ecritures":[{{"ecriture":"{id2}","version":{v2}}}]}}"#),
        None,
    )
    .await;
    assert_eq!(lot.statut, StatusCode::OK, "{}", lot.texte());
    let rang3: Option<String> = sqlx::query_scalar(
        "SELECT version_parametres FROM ecriture_validee WHERE dossier_id = $1 AND rang = 3",
    )
    .bind(b.c.dossier.uuid())
    .fetch_one(&b.env.admin)
    .await
    .unwrap();
    assert_eq!(rang3.as_deref(), Some("sn-2025.2+d1"));
}

#[tokio::test]
async fn deux_dossiers_d_un_meme_cabinet_ont_leurs_propres_versions() {
    let b = banc().await;
    let annee = aujourd_hui().year();
    commun::referentiel::doter(
        &b.env.application,
        b.c.cabinet,
        b.c.autre_dossier,
        b.c.administrateur,
    )
    .await;
    let effet = format!("{annee}-07-01");
    let a = creer(
        &b,
        b.c.dossier,
        &json!({"date_effet":effet,"motif":"A","taux":{"normal":"19"}}),
    )
    .await;
    let c = creer(
        &b,
        b.c.autre_dossier,
        &json!({"date_effet":effet,"motif":"B","taux":{"normal":"17"}}),
    )
    .await;
    assert_eq!(a.statut, StatusCode::CREATED);
    assert_eq!(c.statut, StatusCode::CREATED);
    // Même identifiant (`+d1` dans chaque dossier), contenus et empreintes distincts.
    assert_eq!(a.json()["identifiant"], c.json()["identifiant"]);
    assert_ne!(a.json()["empreinte"], c.json()["empreinte"]);
    let aout = texte_date(jour(annee, 8, 15));
    let admin = b.c.administrateur;
    let sur = |d: IdDossier, taux: &'static str| {
        let corps = corps_du(&aout, taux);
        let b = &b;
        async move { saisir(b, b.c.cabinet, d, admin, &corps).await }
    };
    assert_eq!(sur(b.c.dossier, "19").await.statut, StatusCode::CREATED);
    assert_eq!(
        code(&sur(b.c.dossier, "17").await),
        "etiquette_taux_non_admis"
    );
    assert_eq!(
        sur(b.c.autre_dossier, "17").await.statut,
        StatusCode::CREATED
    );
    assert_eq!(
        code(&sur(b.c.autre_dossier, "19").await),
        "etiquette_taux_non_admis"
    );
    // Chaque dossier ne liste que les siennes.
    for (d, taux) in [(b.c.dossier, "19"), (b.c.autre_dossier, "17")] {
        let apres = applicable(&b, d, &aout).await;
        assert_eq!(taux_normal(&apres).as_deref(), Some(taux));
        assert_eq!(liste(&b, d).await["versions"].as_array().unwrap().len(), 3);
    }
}

#[tokio::test]
async fn un_cabinet_ne_voit_ni_ne_cree_les_versions_d_un_autre() {
    let b = banc().await;
    let r = creer(
        &b,
        b.c.dossier,
        &json!({"date_effet":t(100),"motif":"A","taux":{"normal":"19"}}),
    )
    .await;
    assert_eq!(r.statut, StatusCode::CREATED);
    let tiers = commun::creer_cabinet(&b.env.admin, "Tiers").await;
    commun::referentiel::doter(
        &b.env.application,
        tiers.cabinet,
        tiers.dossier,
        tiers.administrateur,
    )
    .await;
    // Le cabinet tiers ne voit que les siennes (aucune) : les deux versions du paquet.
    let chemin_tiers = format!(
        "/api/cabinets/{}/dossiers/{}/parametres-fiscaux",
        tiers.cabinet, tiers.dossier
    );
    let l = lire(&b.routeur, &chemin_tiers, Some(tiers.administrateur)).await;
    assert_eq!(l.json()["versions"].as_array().unwrap().len(), 2);
    // L'administrateur du premier cabinet n'accède pas au dossier du tiers : ni lecture ni création.
    let r = lire(&b.routeur, &chemin_tiers, Some(b.c.administrateur)).await;
    assert!(
        r.statut == StatusCode::NOT_FOUND || r.statut == StatusCode::FORBIDDEN,
        "{}",
        r.statut
    );
    let r = envoyer(
        &b.routeur,
        &chemin_tiers,
        Some(b.c.administrateur),
        &json!({"date_effet":t(100),"motif":"x","taux":{"normal":"19"}}).to_string(),
        None,
    )
    .await;
    assert!(
        r.statut == StatusCode::NOT_FOUND || r.statut == StatusCode::FORBIDDEN,
        "{}",
        r.statut
    );
    // Ni la RLS : le rôle applicatif, dans le contexte du tiers, ne voit aucune ligne du premier.
    let mut tx = b.env.application.begin().await.unwrap();
    sqlx::query(
        "SELECT set_config('sigma.cabinet', $1, true), set_config('sigma.dossier', $2, true)",
    )
    .bind(tiers.cabinet.to_string())
    .bind(tiers.dossier.to_string())
    .execute(&mut *tx)
    .await
    .unwrap();
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM parametres_fiscaux_dossier")
        .fetch_one(&mut *tx)
        .await
        .unwrap();
    assert_eq!(n, 0);
    let _ = Uuid::nil();
}

// --- Bornes, motif, ligne illisible, rôles ----------------------------------------------

#[tokio::test]
async fn une_date_d_effet_lointaine_est_refusee() {
    let b = banc().await;
    for lointaine in ["9999-12-31", &t(400)] {
        let r = creer(
            &b,
            b.c.dossier,
            &json!({"date_effet":lointaine,"motif":"x","taux":{"normal":"19"}}),
        )
        .await;
        assert_eq!(
            r.statut,
            StatusCode::UNPROCESSABLE_ENTITY,
            "{lointaine} : {}",
            r.texte()
        );
        assert_eq!(code(&r), "parametres_date_effet_lointaine");
    }
    assert_eq!(b.compter("parametres_fiscaux_dossier").await, 0);
    // Un an moins un jour : admis ; la surcharge suivante reste possible plus tard.
    let r = creer(
        &b,
        b.c.dossier,
        &json!({"date_effet":t(360),"motif":"x","taux":{"normal":"19"}}),
    )
    .await;
    assert_eq!(r.statut, StatusCode::CREATED, "{}", r.texte());
}

#[tokio::test]
async fn le_motif_refuse_les_caracteres_de_controle_et_bidirectionnels() {
    let b = banc().await;
    for motif in [
        "a\u{0085}b",
        "a\u{200E}b",
        "a\u{202E}b",
        "a\u{2066}b",
        "a\u{2069}b",
        "a\tb",
    ] {
        let r = creer(
            &b,
            b.c.dossier,
            &json!({"date_effet":t(100),"motif":motif,"taux":{"normal":"19"}}),
        )
        .await;
        assert_eq!(code(&r), "parametres_motif_invalide", "{motif:?}");
    }
    // Seconde barrière : la base refuse les mêmes caractères (sans dépendre de la locale).
    for motif in ["a\u{0085}b", "a\u{200F}b", "a\u{202A}b", "a\u{2067}b"] {
        let direct = sqlx::query(
            "INSERT INTO parametres_fiscaux_dossier \
                 (cabinet_id, dossier_id, id, numero, pays, version, date_effet, contenu, empreinte, motif, cree_par) \
             VALUES ($1, $2, gen_random_uuid(), 1, 'SN', '2025.1', '2030-01-01', 'x', decode(repeat('00', 32), 'hex'), $3, $4)",
        )
        .bind(b.c.cabinet.uuid())
        .bind(b.c.dossier.uuid())
        .bind(motif)
        .bind(b.c.administrateur.uuid())
        .execute(&b.env.admin)
        .await;
        assert!(direct.is_err(), "{motif:?}");
    }
}

#[tokio::test]
async fn une_ligne_illisible_ferme_la_lecture_sans_toucher_aux_autres_dossiers() {
    let b = banc().await;
    commun::referentiel::doter(
        &b.env.application,
        b.c.cabinet,
        b.c.autre_dossier,
        b.c.administrateur,
    )
    .await;
    for d in [b.c.dossier, b.c.autre_dossier] {
        let r = creer(
            &b,
            d,
            &json!({"date_effet":t(100),"motif":"x","taux":{"normal":"19"}}),
        )
        .await;
        assert_eq!(r.statut, StatusCode::CREATED, "{}", r.texte());
    }
    // Contenu altéré en base (hors de toute route) : la lecture échoue (500, fermeture sûre).
    sqlx::query("UPDATE parametres_fiscaux_dossier SET contenu = replace(contenu, 'valeur = \"19\"', 'valeur = \"5\"') WHERE dossier_id = $1")
        .bind(b.c.dossier.uuid())
        .execute(&b.env.admin)
        .await
        .unwrap();
    let r = lire(
        &b.routeur,
        &chemin(&b, b.c.dossier),
        Some(b.c.administrateur),
    )
    .await;
    assert_eq!(r.statut, StatusCode::INTERNAL_SERVER_ERROR, "{}", r.texte());
    assert_eq!(code(&r), "erreur_interne");
    assert!(
        !r.texte().contains("parametres"),
        "aucun détail ne fuit : {}",
        r.texte()
    );
    // La création et la saisie du dossier altéré échouent de même ; l'autre dossier est intact.
    let r = creer(
        &b,
        b.c.dossier,
        &json!({"date_effet":t(200),"motif":"x","taux":{"normal":"20"}}),
    )
    .await;
    assert_eq!(r.statut, StatusCode::INTERNAL_SERVER_ERROR);
    let ok = lire(
        &b.routeur,
        &chemin(&b, b.c.autre_dossier),
        Some(b.c.administrateur),
    )
    .await;
    assert_eq!(ok.statut, StatusCode::OK, "{}", ok.texte());
}

// Rôle `client` : la base n'admet pas encore ce rôle (portail, L4) ; la matrice des politiques
// (`crates/authz/tests/politiques.rs`) le refuse pour les deux actions.

#[tokio::test]
async fn le_pays_est_fige_des_qu_une_version_propre_existe_par_la_route() {
    let b = banc().await;
    let r = creer(
        &b,
        b.c.dossier,
        &json!({"date_effet":t(100),"motif":"x","taux":{"normal":"19"}}),
    )
    .await;
    assert_eq!(r.statut, StatusCode::CREATED, "{}", r.texte());
    let r = envoyer(
        &b.routeur,
        &format!(
            "/api/cabinets/{}/dossiers/{}/contexte",
            b.c.cabinet, b.c.dossier
        ),
        Some(b.c.administrateur),
        r#"{"pays":"CM","devise":"XAF","fuseau":"Africa/Douala"}"#,
        None,
    )
    .await;
    assert_eq!(r.statut, StatusCode::CONFLICT, "{}", r.texte());
    assert_eq!(code(&r), "dossier_parametres_fiscaux_propres");
}

// --- Concurrence --------------------------------------------------------------------------

#[tokio::test]
async fn deux_creations_simultanees_ne_font_qu_une_version_a_la_meme_date() {
    let b = banc().await;
    let corps = |taux: &str| json!({"date_effet":t(100),"motif":"x","taux":{"normal":taux}});
    let (c19, c20) = (corps("19"), corps("20"));
    let (a, c) = tokio::join!(creer(&b, b.c.dossier, &c19), creer(&b, b.c.dossier, &c20));
    let mut statuts = [a.statut, c.statut];
    statuts.sort();
    assert_eq!(
        statuts,
        [StatusCode::CREATED, StatusCode::CONFLICT],
        "{} / {}",
        a.texte(),
        c.texte()
    );
    let refusee = if a.statut == StatusCode::CONFLICT {
        &a
    } else {
        &c
    };
    assert_eq!(code(refusee), "parametres_date_effet_anterieure");
    assert_eq!(b.compter("parametres_fiscaux_dossier").await, 1);
    // À deux dates croissantes : serialisées, numéros 1 et 2.
    let (c21, c22) = (
        json!({"date_effet":t(200),"motif":"x","taux":{"normal":"21"}}),
        json!({"date_effet":t(300),"motif":"x","taux":{"normal":"22"}}),
    );
    let (d, e) = tokio::join!(creer(&b, b.c.dossier, &c21), creer(&b, b.c.dossier, &c22));
    // Une seule des deux peut passer si la plus tardive a précédé : jamais deux numéros égaux.
    assert!(d.statut == StatusCode::CREATED || e.statut == StatusCode::CREATED);
    let numeros: Vec<i32> = sqlx::query_scalar(
        "SELECT numero FROM parametres_fiscaux_dossier WHERE dossier_id = $1 ORDER BY numero",
    )
    .bind(b.c.dossier.uuid())
    .fetch_all(&b.env.admin)
    .await
    .unwrap();
    assert_eq!(
        numeros,
        (1..=i32::try_from(numeros.len()).unwrap()).collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn une_creation_attend_la_cloture_de_la_periode_visee() {
    let b = banc().await;
    // Clôture en cours : la transaction tient la période de la date d'effet (`FOR UPDATE`).
    let mut cloture = b.env.admin.begin().await.unwrap();
    sqlx::query(
        "UPDATE periode SET statut = 'cloturee', statut_par = $2, statut_le = now() \
         WHERE dossier_id = $1 AND debut <= $3",
    )
    .bind(b.c.dossier.uuid())
    .bind(b.c.administrateur.uuid())
    .bind(aujourd_hui())
    .execute(&mut *cloture)
    .await
    .unwrap();
    let routeur = b.routeur.clone();
    let (chemin, administrateur) = (chemin(&b, b.c.dossier), b.c.administrateur);
    let corps = json!({"date_effet":texte_date(aujourd_hui()),"motif":"x","taux":{"normal":"19"}})
        .to_string();
    let creation = tokio::spawn(async move {
        envoyer(&routeur, &chemin, Some(administrateur), &corps, None).await
    });
    tokio::time::sleep(std::time::Duration::from_millis(400)).await;
    assert!(
        !creation.is_finished(),
        "la création doit attendre la clôture"
    );
    cloture.commit().await.unwrap();
    // La clôture validée, la création voit la période clôturée et refuse.
    let r = creation.await.unwrap();
    assert_eq!(r.statut, StatusCode::CONFLICT, "{}", r.texte());
    assert_eq!(code(&r), "parametres_date_effet_cloturee");
    assert_eq!(b.compter("parametres_fiscaux_dossier").await, 0);
}

#[tokio::test]
async fn une_creation_attend_la_tete_de_chaine_d_une_validation_en_cours() {
    let b = banc().await;
    let annee = aujourd_hui().year();
    ecriture_validee_en(&b, &texte_date(jour(annee, 3, 15)), "TAXABLE_NORMAL", "18").await;
    // La tête de chaîne est tenue (validation en cours) : la création attend, puis passe.
    let mut validation = b.env.admin.begin().await.unwrap();
    sqlx::query("SELECT 1 FROM chaine_tete WHERE dossier_id = $1 FOR UPDATE")
        .bind(b.c.dossier.uuid())
        .fetch_one(&mut *validation)
        .await
        .unwrap();
    let routeur = b.routeur.clone();
    let (chemin, administrateur) = (chemin(&b, b.c.dossier), b.c.administrateur);
    let corps = json!({"date_effet":t(100),"motif":"x","taux":{"normal":"19"}}).to_string();
    let creation = tokio::spawn(async move {
        envoyer(&routeur, &chemin, Some(administrateur), &corps, None).await
    });
    tokio::time::sleep(std::time::Duration::from_millis(400)).await;
    assert!(
        !creation.is_finished(),
        "la création doit attendre la tête de chaîne"
    );
    validation.commit().await.unwrap();
    let r = creation.await.unwrap();
    assert_eq!(r.statut, StatusCode::CREATED, "{}", r.texte());
}

#[tokio::test]
async fn une_creation_et_une_validation_simultanees_restent_coherentes() {
    let b = banc().await;
    let annee = aujourd_hui().year();
    let aout = texte_date(jour(annee, 8, 15));
    let (id, v) = b.pret_avec(&corps_du(&aout, "18")).await;
    let effet = texte_date(jour(annee, 7, 1));
    let corps_creation = json!({"date_effet":effet,"motif":"x","taux":{"normal":"19"}});
    let (c, val) = tokio::join!(creer(&b, b.c.dossier, &corps_creation), b.valider(id, v));
    // Soit la validation passe d'abord (écriture sous le paquet, la création suit, avec un
    // avertissement), soit la création passe d'abord et la validation, rejouée ou recontrôlée,
    // refuse le taux 18 : jamais d'incohérence, jamais de 500.
    assert_eq!(c.statut, StatusCode::CREATED, "{}", c.texte());
    assert!(
        val.statut == StatusCode::OK
            || val.statut == StatusCode::CONFLICT
            || val.statut == StatusCode::UNPROCESSABLE_ENTITY,
        "{} {}",
        val.statut,
        val.texte()
    );
    if val.statut == StatusCode::OK {
        let version: Option<String> = sqlx::query_scalar(
            "SELECT version_parametres FROM ecriture_validee WHERE dossier_id = $1",
        )
        .bind(b.c.dossier.uuid())
        .fetch_one(&b.env.admin)
        .await
        .unwrap();
        assert_eq!(version.as_deref(), Some("sn-2025.2"));
        assert_eq!(c.json()["avertissements"][0]["nombre"], 1);
    }
    assert!(b.chaine_valide().await);
}
