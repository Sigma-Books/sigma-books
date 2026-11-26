//! Versions de paramètres fiscaux propres à un dossier (ADR 0030) : création, lecture, règles,
//! audit sans donnée libre, ajout seul, sélection de bout en bout (saisie, validation, relecture,
//! contre-passation), deux dossiers d'un même cabinet, cabinets distincts.
#![cfg(feature = "tests-postgres")]
// Fichier de test : les `expect`/`unwrap` y sont voulus.
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::too_many_lines)]

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
    assert_eq!(l["versions"].as_array().unwrap().len(), 1);
    assert_eq!(l["versions"][0]["identifiant"], "sn-2025.1");
    assert_eq!(l["versions"][0]["portee"], "pays");
    let a = applicable(&b, b.c.dossier, "2026-03-15").await;
    assert_eq!(a["version"]["identifiant"], "sn-2025.1");
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
        &json!({"date_effet":"2027-01-01","motif":motif,"taux":{"normal":"19"},"regimes_retires":["MARGE"]}),
    )
    .await;
    assert_eq!(r.statut, StatusCode::CREATED, "{}", r.texte());
    let v = r.json();
    assert_eq!(v["identifiant"], "sn-2025.1+d1");
    assert_eq!(
        (v["portee"].as_str(), v["numero"].as_i64()),
        (Some("dossier"), Some(1))
    );
    assert_eq!(v["empreinte"].as_str().unwrap().len(), 64);
    assert_eq!(v["motif"], motif);
    assert_eq!(v["cree_par"], b.c.administrateur.to_string());
    // Lecture : deux versions, la seconde est celle du dossier.
    let l = liste(&b, b.c.dossier).await;
    assert_eq!(l["versions"].as_array().unwrap().len(), 2);
    assert_eq!(l["versions"][1]["identifiant"], "sn-2025.1+d1");
    // Application par date : la version du dossier prend effet le 1er janvier 2027 et seulement
    // pour ce dossier ; le paquet du pays n'a pas bougé.
    let avant = applicable(&b, b.c.dossier, "2026-12-31").await;
    assert_eq!(avant["version"]["identifiant"], "sn-2025.1");
    let apres = applicable(&b, b.c.dossier, "2027-06-01").await;
    assert_eq!(apres["version"]["identifiant"], "sn-2025.1+d1");
    assert_eq!(taux_normal(&apres).as_deref(), Some("19"));
    assert_eq!(apres["regimes"].as_array().unwrap().len(), 12);
    let autre = applicable(&b, b.c.autre_dossier, "2027-06-01").await;
    assert_eq!(autre["version"]["identifiant"], "sn-2025.1");
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
        entree.contains("sn-2025.1+d1") && entree.contains("2027-01-01"),
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
            json!({"date_effet":"2027-01-01","motif":"x"}),
            StatusCode::UNPROCESSABLE_ENTITY,
            "parametres_aucun_changement",
        ),
        (
            json!({"date_effet":"2027-01-01","motif":"x","taux":{"normal":"0"}}),
            StatusCode::UNPROCESSABLE_ENTITY,
            "parametres_taux_invalide",
        ),
        (
            json!({"date_effet":"2027-01-01","motif":"x","taux":{"normal":"100.5"}}),
            StatusCode::UNPROCESSABLE_ENTITY,
            "parametres_taux_invalide",
        ),
        (
            json!({"date_effet":"2027-01-01","motif":"x","taux":{"normal":"dix"}}),
            StatusCode::UNPROCESSABLE_ENTITY,
            "parametres_taux_invalide",
        ),
        (
            json!({"date_effet":"2027-01-01","motif":"x","taux":{"exotique":"5"}}),
            StatusCode::UNPROCESSABLE_ENTITY,
            "parametres_role_inconnu",
        ),
        (
            json!({"date_effet":"2027-01-01","motif":"x","regimes_retires":["NOUVEAU"]}),
            StatusCode::UNPROCESSABLE_ENTITY,
            "parametres_regime_inconnu",
        ),
        (
            json!({"date_effet":"2027-01-01","motif":"x","regimes_retires":["IMP18"]}),
            StatusCode::UNPROCESSABLE_ENTITY,
            "parametres_regime_inconnu",
        ),
        (
            json!({"date_effet":"2027-01-01","motif":"x","regimes_retires":["EXO_SANS_DEDUCTION"]}),
            StatusCode::UNPROCESSABLE_ENTITY,
            "parametres_regime_non_retirable",
        ),
        (
            json!({"date_effet":"2027-01-01","motif":" ","taux":{"normal":"19"}}),
            StatusCode::UNPROCESSABLE_ENTITY,
            "parametres_motif_invalide",
        ),
        (
            json!({"date_effet":"2027-01-01","motif":"é".repeat(201),"taux":{"normal":"19"}}),
            StatusCode::UNPROCESSABLE_ENTITY,
            "parametres_motif_invalide",
        ),
        (
            json!({"date_effet":"2027-01-01","motif":"a\nb","taux":{"normal":"19"}}),
            StatusCode::UNPROCESSABLE_ENTITY,
            "parametres_motif_invalide",
        ),
        (
            json!({"date_effet":"2027-01-01","date_fin":"2026-12-31","motif":"x","taux":{"normal":"19"}}),
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
            json!({"date_effet":"2027-01-01","motif":"x","taux":{"normal":"19"},"autre":1}),
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
            json!({"date_effet":"2027-01-01","taux":{"normal":"19"}}),
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
        &json!({"date_effet":"2027-01-01","motif":"x","regimes_retires":trop}),
    )
    .await;
    assert_eq!(r.statut, StatusCode::BAD_REQUEST);
    assert_eq!(b.compter("parametres_fiscaux_dossier").await, 0);
    // Un motif de 400 octets exactement est admis.
    let r = creer(
        &b,
        b.c.dossier,
        &json!({"date_effet":"2027-01-01","motif":"a".repeat(400),"taux":{"normal":"19"}}),
    )
    .await;
    assert_eq!(r.statut, StatusCode::CREATED, "{}", r.texte());
}

#[tokio::test]
async fn les_conflits_d_etat_sont_refuses() {
    let b = banc().await;
    // Date d'effet jamais antérieure ni égale à la dernière version du dossier.
    let v1 = json!({"date_effet":"2027-06-01","motif":"Première","taux":{"normal":"19"}});
    assert_eq!(
        creer(&b, b.c.dossier, &v1).await.statut,
        StatusCode::CREATED
    );
    for date in ["2027-06-01", "2027-01-01"] {
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
        &json!({"date_effet":"2028-01-01","motif":"Seconde","regimes_retires":["MARGE"]}),
    )
    .await;
    assert_eq!(r.statut, StatusCode::CREATED, "{}", r.texte());
    assert_eq!(r.json()["identifiant"], "sn-2025.1+d2");
    let a = applicable(&b, b.c.dossier, "2028-02-01").await;
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
        &json!({"date_effet":"2099-01-01","motif":"x","taux":{"normal":"19"}}),
    )
    .await;
    assert_eq!(r.statut, StatusCode::UNPROCESSABLE_ENTITY, "{}", r.texte());
    assert_eq!(code(&r), "parametres_pays_sans_paquet");
}

#[tokio::test]
async fn un_regime_utilise_par_une_ecriture_validee_n_est_jamais_retire() {
    let b = banc().await;
    let annee = aujourd_hui().year();
    let etiquetee = format!(
        r#"{{"compte":"{}","sens":"debit","montant":"1000","etiquette":{{"nature":"base","sens":"collectee","regime":"TAXABLE_NORMAL","taux":"18","exigibilite":"debit"}}}}"#,
        caisse()
    );
    let corps = contenu(
        &journal_de_saisie(),
        &texte_date(jour(annee, 3, 15)),
        &[etiquetee, ligne(banque(), "credit", "1000")],
        "",
    );
    let (id, v) = b.pret_avec(&corps).await;
    assert_eq!(b.valider(id, v).await.statut, StatusCode::OK);
    let r = creer(
        &b,
        b.c.dossier,
        &json!({"date_effet":"2099-01-01","motif":"x","regimes_retires":["TAXABLE_NORMAL"]}),
    )
    .await;
    assert_eq!(r.statut, StatusCode::CONFLICT, "{}", r.texte());
    assert_eq!(code(&r), "parametres_regime_utilise");
    assert_eq!(b.compter("parametres_fiscaux_dossier").await, 0);
    // Un régime que nulle écriture validée n'utilise peut être retiré.
    let r = creer(
        &b,
        b.c.dossier,
        &json!({"date_effet":"2099-01-01","motif":"x","regimes_retires":["MARGE"]}),
    )
    .await;
    assert_eq!(r.statut, StatusCode::CREATED, "{}", r.texte());
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
        &json!({"date_effet":"2200-01-01","motif":"x","taux":{"normal":"19"}}),
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
    let corps = json!({"date_effet":"2027-01-01","motif":"x","taux":{"normal":"19"}}).to_string();
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
        &json!({"date_effet":"2027-01-01","motif":"x","taux":{"normal":"19"}}),
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
    assert_eq!(r.json()["version_parametres"], "sn-2025.1");
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
        chaine[0].2.contains("\nparametres=sn-2025.1+d1\n"),
        "{}",
        chaine[0].2
    );
    let lue = lire(
        &b.routeur,
        &format!("{}/{id}", b.ecritures()),
        Some(b.auditeur),
    )
    .await;
    assert_eq!(lue.json()["version_parametres"], "sn-2025.1+d1");
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
    assert_eq!(rang2.as_deref(), Some("sn-2025.1+d1"));
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
    let rang4: Option<String> = sqlx::query_scalar(
        "SELECT version_parametres FROM ecriture_validee WHERE dossier_id = $1 AND rang = 4",
    )
    .bind(b.c.dossier.uuid())
    .fetch_one(&b.env.admin)
    .await
    .unwrap();
    assert_eq!(rang4.as_deref(), Some("sn-2025.1+d1"));
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
        assert_eq!(liste(&b, d).await["versions"].as_array().unwrap().len(), 2);
    }
}

#[tokio::test]
async fn un_cabinet_ne_voit_ni_ne_cree_les_versions_d_un_autre() {
    let b = banc().await;
    let r = creer(
        &b,
        b.c.dossier,
        &json!({"date_effet":"2027-01-01","motif":"A","taux":{"normal":"19"}}),
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
    // Le cabinet tiers ne voit que les siennes (aucune).
    let chemin_tiers = format!(
        "/api/cabinets/{}/dossiers/{}/parametres-fiscaux",
        tiers.cabinet, tiers.dossier
    );
    let l = lire(&b.routeur, &chemin_tiers, Some(tiers.administrateur)).await;
    assert_eq!(l.json()["versions"].as_array().unwrap().len(), 1);
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
        &json!({"date_effet":"2027-01-01","motif":"x","taux":{"normal":"19"}}).to_string(),
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
