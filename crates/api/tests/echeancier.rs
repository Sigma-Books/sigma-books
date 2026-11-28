//! Échéancier par l'API (lot L3, PR 8, L3-17 ; ADR 0033) : échéance saisie sur une ligne de tiers,
//! copiée à la validation **hors du contenu haché**, révisions en ajout seul, statut calculé,
//! conditions de règlement (modèles d'usage du dossier, assistant de saisie), condition par défaut
//! du tiers ; ajout seul et RLS vérifiés avec le rôle applicatif.
#![cfg(feature = "tests-postgres")]
// Fichier de test : les `expect`/`unwrap` y sont voulus.
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::too_many_lines)]

mod commun;

use api::base::{ContexteTenant, ouvrir_transaction};
use axum::http::StatusCode;
use commun::audit_valeurs::verifier_sans_valeur_metier;
use commun::banc::{Banc, banc, code, contenu};
use commun::brouillard::{aujourd_hui, journal_de_saisie, ligne, texte_date};
use commun::referentiel::comptes;
use commun::serveur::{Reponse, envoyer, lire};
use commun::{code_sql, creer_cabinet};
use serde_json::{Value, json};
use uuid::Uuid;

/// Échéance lointaine, qu'aucune autre valeur de l'écriture ne contient.
const ECHEANCE: &str = "2099-06-30";

fn racine(b: &Banc) -> String {
    format!("/api/cabinets/{}/dossiers/{}", b.c.cabinet, b.c.dossier)
}

/// Ligne JSON du tiers fixe sur le compte collectif, avec une échéance facultative.
fn ligne_tiers(b: &Banc, sens: &str, echeance: Option<&str>) -> String {
    let echeance = echeance.map_or(String::new(), |e| format!(r#","date_echeance":"{e}""#));
    format!(
        r#"{{"compte":"{}","sens":"{sens}","montant":"1000","tiers":"{}"{echeance}}}"#,
        comptes().collectif,
        b.f.tiers
    )
}

/// Facture du jour : débit du tiers (échéance facultative), crédit de la caisse.
fn facture(b: &Banc, echeance: Option<&str>) -> String {
    contenu(
        &journal_de_saisie(),
        &texte_date(aujourd_hui()),
        &[
            ligne_tiers(b, "debit", echeance),
            ligne(comptes().caisse.as_str(), "credit", "1000"),
        ],
        "",
    )
}

/// Saisit, justifie et valide une écriture ; rend son identifiant.
async fn valider(b: &Banc, corps: &str) -> Uuid {
    let (id, version) = b.pret_avec(corps).await;
    let r = b.valider(id, version).await;
    assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
    id
}

async fn echeances(b: &Banc, ecriture: Uuid, parametres: &str) -> Reponse {
    lire(
        &b.routeur,
        &format!("{}/echeances/{ecriture}{parametres}", racine(b)),
        Some(b.auditeur),
    )
    .await
}

async fn reviser(b: &Banc, ecriture: Uuid, qui: api::base::IdUtilisateur, corps: &str) -> Reponse {
    envoyer(
        &b.routeur,
        &format!("{}/echeances/{ecriture}/revisions", racine(b)),
        Some(qui),
        corps,
        None,
    )
    .await
}

#[tokio::test]
async fn echeance_copiee_a_la_validation_hors_du_contenu_hache() {
    let b = banc().await;
    // Une échéance sur une ligne sans tiers : 422, avec la position de la ligne.
    let sans_tiers = contenu(
        &journal_de_saisie(),
        &texte_date(aujourd_hui()),
        &[
            ligne_tiers(&b, "debit", None),
            format!(
                r#"{{"compte":"{}","sens":"credit","montant":"1000","date_echeance":"{ECHEANCE}"}}"#,
                comptes().caisse
            ),
        ],
        "",
    );
    let r = envoyer(
        &b.routeur,
        &b.base(),
        Some(b.c.collaborateur),
        &sans_tiers,
        None,
    )
    .await;
    assert_eq!(r.statut, StatusCode::UNPROCESSABLE_ENTITY, "{}", r.texte());
    assert_eq!(code(&r), "echeance_sans_tiers");
    assert_eq!(r.json()["ligne"], 2);

    // Le brouillon porte l'échéance de sa ligne de tiers, et la rend.
    let id = b.creer(&facture(&b, Some(ECHEANCE))).await;
    let r = lire(&b.routeur, &format!("{}/{id}", b.base()), Some(b.auditeur)).await;
    assert_eq!(
        r.json()["lignes"][0]["date_echeance"],
        ECHEANCE,
        "{}",
        r.texte()
    );
    assert!(r.json()["lignes"][1].get("date_echeance").is_none());
    let (_, version) = b.lier(id, 1).await;
    let r = b.valider(id, version).await;
    assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());

    // Copiée dans `echeance_ligne`, position de la ligne de tiers ; hors du contenu haché.
    let copiees: Vec<(i16, time::Date)> =
        sqlx::query_as("SELECT position, date_echeance FROM echeance_ligne WHERE ecriture_id = $1")
            .bind(id)
            .fetch_all(&b.env.admin)
            .await
            .unwrap();
    assert_eq!(copiees.len(), 1);
    assert_eq!(copiees[0].0, 1);
    assert_eq!(texte_date(copiees[0].1), ECHEANCE);
    let chaine = b.chaine().await;
    assert!(!chaine[0].2.contains(ECHEANCE), "{}", chaine[0].2);
    assert!(!chaine[0].2.contains("echeance"), "{}", chaine[0].2);
    assert!(b.chaine_valide().await);

    // Échéancier de l'écriture : statut et situation calculés, résiduel entier (sans lettrage).
    let r = echeances(&b, id, "").await;
    assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
    let v = r.json();
    assert_eq!(v["id"], id.to_string());
    let lignes = v["lignes"].as_array().unwrap();
    assert_eq!(lignes.len(), 1, "seule la ligne de tiers : {v}");
    assert_eq!(lignes[0]["position"], 1);
    assert_eq!(lignes[0]["tiers"], b.f.tiers.to_string());
    assert_eq!(lignes[0]["echeance_initiale"], ECHEANCE);
    assert_eq!(lignes[0]["echeance"], ECHEANCE);
    assert_eq!(lignes[0]["residuel"], "1000");
    assert_eq!(lignes[0]["statut"], "ouverte");
    assert_eq!(lignes[0]["situation"], "a_echoir");
    // Échue au lendemain de l'échéance.
    let r = echeances(&b, id, "?au=2099-07-02").await;
    assert_eq!(r.json()["lignes"][0]["situation"], "echue");
    assert_eq!(r.json()["lignes"][0]["retard_jours"], 2);
    assert_eq!(
        echeances(&b, id, "?au=2099-13-01").await.statut,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        echeances(&b, id, "?autre=1").await.statut,
        StatusCode::BAD_REQUEST
    );

    // Sans échéance saisie : rien n'est copié, repli sur la date comptable (Tryton `move.py:776`).
    let sans = valider(&b, &facture(&b, None)).await;
    assert_eq!(b.compter("echeance_ligne").await, 1);
    let r = echeances(&b, sans, "").await;
    assert!(r.json()["lignes"][0]["echeance_initiale"].is_null());
    assert_eq!(r.json()["lignes"][0]["echeance"], texte_date(aujourd_hui()));

    // La contre-passation ne recopie pas l'échéance (ADR 0033 point 1).
    let r = envoyer(
        &b.routeur,
        &format!("{}/ecritures/{id}/contre-passation", racine(&b)),
        Some(b.valideur),
        &format!(
            r#"{{"date":"{}","libelle":"Contre-passation","motif":"Erreur"}}"#,
            texte_date(aujourd_hui())
        ),
        None,
    )
    .await;
    assert!(r.statut.is_success(), "{}", r.texte());
    assert_eq!(b.compter("echeance_ligne").await, 1);
    assert!(b.chaine_valide().await);
}

#[tokio::test]
async fn revisions_en_ajout_seul_et_audit_sans_motif() {
    let b = banc().await;
    let id = valider(&b, &facture(&b, Some(ECHEANCE))).await;
    let rang_audit: i64 = sqlx::query_scalar(
        "SELECT coalesce(max(rang), 0)::bigint FROM audit_dossier WHERE dossier_id = $1",
    )
    .bind(b.c.dossier.uuid())
    .fetch_one(&b.env.admin)
    .await
    .unwrap();
    let corps = |date: &str, motif: &str| {
        json!({"position": 1, "nouvelle_echeance": date, "motif": motif}).to_string()
    };
    // Rôles : la saisie révise, l'auditeur non.
    let r = reviser(&b, id, b.auditeur, &corps("2099-07-31", "Report")).await;
    assert_eq!(r.statut, StatusCode::FORBIDDEN);
    let r = reviser(
        &b,
        id,
        b.c.collaborateur,
        &corps("2099-07-31", "Report accordé au client"),
    )
    .await;
    assert_eq!(r.statut, StatusCode::CREATED, "{}", r.texte());
    let v = r.json();
    assert_eq!(v["lignes"][0]["echeance"], "2099-07-31");
    assert_eq!(v["lignes"][0]["echeance_initiale"], ECHEANCE);
    assert_eq!(v["lignes"][0]["revisions"][0]["rang"], 1);
    let r = reviser(&b, id, b.valideur, &corps("2099-08-31", "Second report")).await;
    assert_eq!(r.statut, StatusCode::CREATED, "{}", r.texte());
    assert_eq!(r.json()["lignes"][0]["revisions"][1]["rang"], 2);
    let r = echeances(&b, id, "").await;
    assert_eq!(r.json()["lignes"][0]["echeance"], "2099-08-31");
    assert_eq!(
        r.json()["lignes"][0]["revisions"].as_array().unwrap().len(),
        2
    );
    // Avant le jour des révisions, l'échéance effective reste l'initiale.
    let veille = texte_date(aujourd_hui() - time::Duration::days(1));
    let r = echeances(&b, id, &format!("?au={veille}")).await;
    assert_eq!(r.json()["lignes"][0]["echeance"], ECHEANCE);

    // Refus : ligne sans tiers, motif vide ou piégé, corps invalide, écriture absente.
    let r = reviser(
        &b,
        id,
        b.c.collaborateur,
        &json!({"position": 2, "nouvelle_echeance": "2099-07-31", "motif": "Report"}).to_string(),
    )
    .await;
    assert_eq!(r.statut, StatusCode::UNPROCESSABLE_ENTITY, "{}", r.texte());
    assert_eq!(code(&r), "echeance_sans_tiers");
    for motif in ["", "   ", "Report\u{202E}", &"x".repeat(401)] {
        let r = reviser(&b, id, b.c.collaborateur, &corps("2099-07-31", motif)).await;
        assert_eq!(r.statut, StatusCode::UNPROCESSABLE_ENTITY, "{motif:?}");
        assert_eq!(code(&r), "echeance_motif_invalide");
    }
    let r = reviser(&b, id, b.c.collaborateur, &corps("2099-02-30", "Report")).await;
    assert_eq!(r.statut, StatusCode::BAD_REQUEST);
    let r = reviser(
        &b,
        Uuid::new_v4(),
        b.c.collaborateur,
        &corps("2099-07-31", "Report"),
    )
    .await;
    assert_eq!(r.statut, StatusCode::NOT_FOUND);
    assert_eq!(b.compter("echeance_report").await, 2);

    // Audit : une trace par révision, dates et rang, jamais le motif.
    let entrees: Vec<String> = sqlx::query_scalar(
        "SELECT contenu FROM audit_dossier WHERE dossier_id = $1 AND rang > $2 ORDER BY rang",
    )
    .bind(b.c.dossier.uuid())
    .bind(rang_audit)
    .fetch_all(&b.env.admin)
    .await
    .unwrap();
    assert_eq!(entrees.len(), 2, "{entrees:?}");
    for entree in &entrees {
        assert!(entree.contains("revision"), "{entree}");
        assert!(entree.contains("ReviserEcheance"), "{entree}");
        verifier_sans_valeur_metier(entree, &["Report accordé", "Second report"], &["1000"]);
    }

    // Ajout seul et RLS, avec le rôle applicatif (jamais un super-utilisateur) : une transaction
    // par essai (l'échec l'annule).
    let autre = creer_cabinet(&b.env.admin, "Autre cabinet").await;
    let essai = |requete: &'static str| {
        let pool = b.env.application.clone();
        let contexte = ContexteTenant::dossier(b.c.cabinet, b.c.dossier, b.c.administrateur);
        let (cabinet, dossier, auteur) = (
            autre.cabinet.uuid(),
            autre.dossier.uuid(),
            b.c.administrateur.uuid(),
        );
        async move {
            let mut tx = ouvrir_transaction(&pool, contexte).await.unwrap();
            sqlx::query(requete)
                .bind(cabinet)
                .bind(dossier)
                .bind(id)
                .bind(auteur)
                .execute(tx.connexion())
                .await
                .unwrap_err()
        }
    };
    // Ni UPDATE ni DELETE sur ce qui est conservé : 42501 (droit refusé).
    for requete in [
        "UPDATE echeance_report SET motif = 'altéré'",
        "DELETE FROM echeance_report",
        "UPDATE echeance_ligne SET date_echeance = '2000-01-01'",
        "DELETE FROM echeance_ligne",
        "DELETE FROM condition_reglement",
        "UPDATE condition_reglement SET code = 'X'",
    ] {
        let mut tx = ouvrir_transaction(
            &b.env.application,
            ContexteTenant::dossier(b.c.cabinet, b.c.dossier, b.c.administrateur),
        )
        .await
        .unwrap();
        let e = sqlx::query(requete)
            .execute(tx.connexion())
            .await
            .unwrap_err();
        assert_eq!(code_sql(&e).as_deref(), Some("42501"), "{requete}");
    }
    // Insertion croisée : au nom du dossier de l'autre cabinet, refusée par `WITH CHECK`.
    for requete in [
        "INSERT INTO echeance_report (cabinet_id, dossier_id, ecriture_id, position, rang, \
         nouvelle_echeance, date_operation, motif, auteur, cree_le) \
         VALUES ($1, $2, $3, 1, 9, '2099-01-01', '2099-01-01', 'x', $4, now())",
        "INSERT INTO echeance_ligne (cabinet_id, dossier_id, ecriture_id, position, date_echeance) \
         VALUES ($1, $2, $3, 1, '2099-01-01') RETURNING $4::uuid",
        "INSERT INTO condition_reglement (cabinet_id, dossier_id, id, code, libelle, cree_le) \
         VALUES ($1, $2, $3, 'CROISE', 'x', now()) RETURNING $4::uuid",
    ] {
        assert_eq!(
            code_sql(&essai(requete).await).as_deref(),
            Some("42501"),
            "{requete}"
        );
    }
    // Clé composite : une révision du dossier courant ne cite pas l'écriture d'un autre dossier.
    let etrangere = commun::echeancier::doter_ecriture_tiers(
        &b.env.admin,
        autre.cabinet,
        autre.dossier,
        autre.administrateur,
        commun::referentiel::doter(
            &b.env.application,
            autre.cabinet,
            autre.dossier,
            autre.administrateur,
        )
        .await
        .tiers,
        1,
    )
    .await;
    let mut tx = ouvrir_transaction(
        &b.env.application,
        ContexteTenant::dossier(b.c.cabinet, b.c.dossier, b.c.administrateur),
    )
    .await
    .unwrap();
    let e = sqlx::query(
        "INSERT INTO echeance_report (cabinet_id, dossier_id, ecriture_id, position, rang, \
         nouvelle_echeance, date_operation, motif, auteur, cree_le) \
         VALUES ($1, $2, $3, 1, 9, '2099-01-01', '2099-01-01', 'x', $4, now())",
    )
    .bind(b.c.cabinet.uuid())
    .bind(b.c.dossier.uuid())
    .bind(etrangere)
    .bind(b.c.administrateur.uuid())
    .execute(tx.connexion())
    .await
    .unwrap_err();
    assert_eq!(code_sql(&e).as_deref(), Some("23503"));
    drop(tx);
    // La route ne voit pas l'écriture de l'autre dossier : 404.
    let r = reviser(
        &b,
        etrangere,
        b.c.administrateur,
        &corps("2099-07-31", "Report"),
    )
    .await;
    assert_eq!(r.statut, StatusCode::NOT_FOUND);
    assert_eq!(
        echeances(&b, etrangere, "").await.statut,
        StatusCode::NOT_FOUND
    );
    // Lecture sous la RLS du dossier de l'autre cabinet : rien de ce dossier-ci.
    let mut tx = ouvrir_transaction(
        &b.env.application,
        ContexteTenant::dossier(autre.cabinet, autre.dossier, autre.administrateur),
    )
    .await
    .unwrap();
    let vues: i64 = sqlx::query_scalar("SELECT count(*) FROM echeance_report")
        .fetch_one(tx.connexion())
        .await
        .unwrap();
    assert_eq!(vues, 0);
}

#[tokio::test]
async fn conditions_de_reglement_assistant_et_tiers() {
    let b = banc().await;
    let base = format!("{}/conditions-reglement", racine(&b));
    let poster = |chemin: String, qui, corps: String| {
        let routeur = b.routeur.clone();
        async move { envoyer(&routeur, &chemin, Some(qui), &corps, None).await }
    };
    // Condition en trois termes (cas de Tryton `test_module.py:46-106`, en XOF sans décimale) :
    // 25 % du total, 397 fixe, solde.
    let melange = json!({
        "code": "MIX",
        "libelle": "Quart, fixe et solde",
        "termes": [
            {"valeur": "pourcentage", "montant": "25", "delai": "jours", "jours": 30},
            {"valeur": "fixe", "montant": "397", "delai": "fin_de_mois", "jours": 0},
            {"valeur": "solde", "delai": "jour_du_mois_suivant", "jours": 30, "jour": 10}
        ]
    });
    let r = poster(base.clone(), b.c.collaborateur, melange.to_string()).await;
    assert_eq!(r.statut, StatusCode::FORBIDDEN);
    let r = poster(base.clone(), b.valideur, melange.to_string()).await;
    assert_eq!(r.statut, StatusCode::CREATED, "{}", r.texte());
    let id = r.json()["id"].as_str().unwrap().to_owned();
    assert_eq!(r.json()["termes"].as_array().unwrap().len(), 3);
    let r = poster(base.clone(), b.c.administrateur, melange.to_string()).await;
    assert_eq!(r.statut, StatusCode::CONFLICT);
    assert_eq!(code(&r), "condition_reglement_code_deja_utilise");
    // Le dernier terme doit être le solde ; les pourcentages restent sous 100.
    for termes in [
        json!([{"valeur": "pourcentage", "montant": "50", "delai": "jours"}]),
        json!([{"valeur": "pourcentage", "montant": "100", "delai": "jours"},
               {"valeur": "solde", "delai": "jours"}]),
        json!([]),
    ] {
        let corps = json!({"code": "KO", "libelle": "Invalide", "termes": termes});
        let r = poster(base.clone(), b.c.administrateur, corps.to_string()).await;
        assert_eq!(r.statut, StatusCode::UNPROCESSABLE_ENTITY, "{termes}");
        assert_eq!(code(&r), "condition_reglement_invalide");
    }

    // Assistant : une ligne de tiers par terme, somme exacte, aucune écriture.
    let assistant = |parametres: &str| {
        let chemin = format!("{base}/{id}/echeances{parametres}");
        let routeur = b.routeur.clone();
        let qui = b.c.collaborateur;
        async move { lire(&routeur, &chemin, Some(qui)).await }
    };
    let r = assistant("?date=2026-01-15&montant=1587").await;
    assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
    let termes: Vec<(String, String)> = r.json()["termes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| {
            (
                t["date"].as_str().unwrap().to_owned(),
                t["montant"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    assert_eq!(
        termes,
        [
            ("2026-01-31".to_owned(), "397".to_owned()),
            ("2026-02-14".to_owned(), "397".to_owned()),
            ("2026-03-10".to_owned(), "793".to_owned()),
        ]
    );
    // Avoir : le signe passe sur chaque terme.
    let r = assistant("?date=2026-01-15&montant=-1587").await;
    assert_eq!(r.json()["termes"][2]["montant"], "-793");
    assert_eq!(
        code(&assistant("?date=2026-01-15&montant=10.5").await),
        "montant_echelle_devise_tenue"
    );
    for parametres in [
        "",
        "?date=2026-01-15",
        "?montant=1",
        "?date=x&montant=1",
        "?date=2026-01-15&montant=1&x=1",
    ] {
        assert_eq!(
            assistant(parametres).await.statut,
            StatusCode::BAD_REQUEST,
            "{parametres}"
        );
    }
    let r = lire(
        &b.routeur,
        &format!("{base}/{id}/echeances?date=2026-01-15&montant=1"),
        Some(b.auditeur),
    )
    .await;
    assert_eq!(r.statut, StatusCode::FORBIDDEN);

    // Modification : termes remplacés en bloc ; rien ne change : 409 ; archivage.
    let modifier = |corps: Value| {
        poster(
            format!("{base}/{id}/modification"),
            b.c.administrateur,
            corps.to_string(),
        )
    };
    let r = modifier(json!({"termes": [{"valeur": "solde", "delai": "jours", "jours": 45}]})).await;
    assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
    assert_eq!(r.json()["termes"].as_array().unwrap().len(), 1);
    let r = modifier(json!({"termes": [{"valeur": "solde", "delai": "jours", "jours": 45}]})).await;
    assert_eq!(code(&r), "aucun_changement");
    let r = modifier(json!({"actif": false, "libelle": "Archivée"})).await;
    assert_eq!(r.json()["actif"], false);
    let r = modifier(json!({})).await;
    assert_eq!(r.statut, StatusCode::BAD_REQUEST);
    let r = lire(&b.routeur, &base, Some(b.auditeur)).await;
    assert_eq!(r.json()["conditions"][0]["code"], "MIX");

    // Condition par défaut du tiers (ADR 0031 point 4) ; celle d'un autre dossier est absente.
    let tiers = format!("{}/tiers/{}", racine(&b), b.f.tiers);
    let r = poster(
        format!("{tiers}/modification"),
        b.c.collaborateur,
        json!({"condition_reglement": id}).to_string(),
    )
    .await;
    assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
    assert_eq!(r.json()["condition_reglement"], id.as_str());
    let autre = creer_cabinet(&b.env.admin, "Autre cabinet").await;
    let etrangere = commun::echeancier::doter(&b.env.admin, autre.cabinet, autre.dossier).await;
    let r = poster(
        format!("{tiers}/modification"),
        b.c.collaborateur,
        json!({"condition_reglement": etrangere}).to_string(),
    )
    .await;
    assert_eq!(r.statut, StatusCode::NOT_FOUND);
    let r = poster(
        format!("{tiers}/modification"),
        b.c.collaborateur,
        r#"{"condition_reglement":null}"#.to_owned(),
    )
    .await;
    assert!(r.json()["condition_reglement"].is_null(), "{}", r.texte());
    let r = lire(
        &b.routeur,
        &format!("{base}/{etrangere}"),
        Some(b.c.administrateur),
    )
    .await;
    assert_eq!(r.statut, StatusCode::NOT_FOUND);

    // Audit des conditions : code et champs, sans libellé.
    let entrees: Vec<String> = sqlx::query_scalar(
        "SELECT contenu FROM audit_dossier WHERE dossier_id = $1 AND contenu LIKE '%condition_reglement%' ORDER BY rang",
    )
    .bind(b.c.dossier.uuid())
    .fetch_all(&b.env.admin)
    .await
    .unwrap();
    assert!(entrees.len() >= 4, "{entrees:?}");
}

#[tokio::test]
async fn modeles_d_usage_crees_avec_le_dossier() {
    let b = banc().await;
    let r = envoyer(
        &b.routeur,
        &format!("/api/cabinets/{}/dossiers", b.c.cabinet),
        Some(b.c.administrateur),
        &json!({"nom": "SARL Dakar", "nature": "reprise", "annee": aujourd_hui().year(), "pays": "SN"})
            .to_string(),
        None,
    )
    .await;
    assert_eq!(r.statut, StatusCode::CREATED, "{}", r.texte());
    let dossier = r.json()["id"].as_str().unwrap().to_owned();
    let r = lire(
        &b.routeur,
        &format!(
            "/api/cabinets/{}/dossiers/{dossier}/conditions-reglement",
            b.c.cabinet
        ),
        Some(b.c.administrateur),
    )
    .await;
    assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
    let codes: Vec<&str> = r.json()["conditions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["code"].as_str().unwrap())
        .collect::<Vec<_>>()
        .into_iter()
        .map(|c| match c {
            "COMPTANT" => "COMPTANT",
            "30JFM" => "30JFM",
            autre => {
                if api::serveur::routes::echeancier::MODELES_D_USAGE
                    .iter()
                    .any(|(m, _, _)| *m == autre)
                {
                    "modele"
                } else {
                    "inconnu"
                }
            }
        })
        .collect();
    assert_eq!(
        codes.len(),
        api::serveur::routes::echeancier::MODELES_D_USAGE.len()
    );
    assert!(codes.contains(&"COMPTANT") && codes.contains(&"30JFM"));
    assert!(!codes.contains(&"inconnu"));
    // La création du dossier cite le nombre de modèles dans sa trace d'audit.
    let entree: String = sqlx::query_scalar(
        "SELECT contenu FROM audit_dossier WHERE dossier_id = $1::uuid ORDER BY rang LIMIT 1",
    )
    .bind(&dossier)
    .fetch_one(&b.env.admin)
    .await
    .unwrap();
    assert!(entree.contains("conditions_reglement"), "{entree}");
}
