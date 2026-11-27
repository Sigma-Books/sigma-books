//! Comptes de trésorerie, avertissements de solde, virements internes et arrêté de caisse
//! (L3-21, PR 11 de `docs/lots/L3.md` ; ADR 0034 points 1 à 3) : création assistée du
//! sous-compte et du journal, reprise d'un journal existant, avertissements **jamais bloquants**,
//! 585, arrêté figé en pièce interne et écart proposé au brouillard, audit, RLS.
//!
//! Aucun numéro de compte écrit en dur : comptes par défaut des types et compte de virements
//! internes tirés des données de `syscohada`, autres comptes de `commun::referentiel::comptes`.
#![cfg(feature = "tests-postgres")]
// Fichier de test : les `expect`/`unwrap` y sont voulus.
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::too_many_lines)]

mod commun;

use api::base::{ContexteTenant, ouvrir_transaction};
use axum::http::StatusCode;
use commun::banc::{Banc, aujourd, banc, banque, caisse, code, contenu};
use commun::brouillard::{journal_de_saisie, ligne};
use commun::referentiel::sous_compte_fixe;
use commun::serveur::{Reponse, envoyer, lire};
use serde_json::{Value, json};
use syscohada::TypeTresorerie;

fn defaut(t: TypeTresorerie) -> String {
    syscohada::Referentiel::officiel()
        .unwrap()
        .compte_par_defaut_tresorerie(t)
        .unwrap()
        .to_string()
}

/// Le compte de virements internes de la liste officielle (585, marque `virement_interne`).
fn virement_interne() -> String {
    let r = syscohada::Referentiel::officiel().unwrap();
    r.comptes()
        .find(|c| c.marque_virement_interne().is_some())
        .unwrap()
        .numero()
        .to_string()
}

fn chemin(b: &Banc, suffixe: &str) -> String {
    format!(
        "/api/cabinets/{}/dossiers/{}/tresorerie{suffixe}",
        b.c.cabinet, b.c.dossier
    )
}

async fn poster(b: &Banc, u: api::base::IdUtilisateur, suffixe: &str, corps: &Value) -> Reponse {
    envoyer(
        &b.routeur,
        &chemin(b, suffixe),
        Some(u),
        &corps.to_string(),
        None,
    )
    .await
}

async fn creer(b: &Banc, corps: &Value) -> Value {
    let r = poster(b, b.c.administrateur, "/comptes", corps).await;
    assert_eq!(r.statut, StatusCode::CREATED, "{}", r.texte());
    r.json()
}

async fn fiche(b: &Banc, id: &str, parametres: &str) -> Value {
    let r = lire(
        &b.routeur,
        &chemin(b, &format!("/comptes/{id}{parametres}")),
        Some(b.auditeur),
    )
    .await;
    assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
    r.json()
}

/// Aucun avertissement.
const AUCUN: [&str; 0] = [];

fn codes(v: &Value) -> Vec<String> {
    v["avertissements"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["code"].as_str().unwrap().to_owned())
        .collect()
}

/// Valide une écriture du jour (journal de saisie, pièce liée) : la validation **réussit**.
async fn valider(b: &Banc, lignes: &[String]) {
    let corps = contenu(&journal_de_saisie(), &aujourd(), lignes, "");
    let (id, version) = b.pret_avec(&corps).await;
    let r = b.valider(id, version).await;
    assert!(r.statut.is_success(), "{}", r.texte());
}

async fn audit(b: &Banc, genre: &str) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM audit_dossier WHERE dossier_id = $1 \
         AND contenu LIKE '%' || chr(10) || 'ressource=' || $2 || ':%'",
    )
    .bind(b.c.dossier.uuid())
    .bind(genre)
    .fetch_one(&b.env.admin)
    .await
    .unwrap()
}

#[tokio::test]
async fn creation_assistee_du_sous_compte_et_du_journal() {
    let banc_de_test = banc().await;
    let b = &banc_de_test;
    let banque1 = creer(b, &json!({"type":"banque","nom":"Banque principale"})).await;
    let d = defaut(TypeTresorerie::Banque);
    assert_eq!(banque1["compte"]["numero"], format!("{d}-01"));
    assert_eq!(banque1["type"], "banque");
    assert_eq!(banque1["devise"], "XOF");
    assert_eq!(banque1["solde"], "0");
    assert_eq!(codes(&banque1), AUCUN);
    let code1 = banque1["journal"]["code"].as_str().unwrap().to_owned();
    assert!(code1.starts_with("BQ"), "{code1}");
    let banque2 = creer(b, &json!({"type":"banque","nom":"Banque secondaire"})).await;
    assert_eq!(banque2["compte"]["numero"], format!("{d}-02"));
    assert_ne!(banque2["journal"]["code"], code1);
    let caisse = creer(
        b,
        &json!({"type":"caisse","nom":"Caisse du siège","plafond":"500"}),
    )
    .await;
    assert_eq!(
        caisse["compte"]["numero"],
        format!("{}-01", defaut(TypeTresorerie::Caisse))
    );
    assert_eq!(caisse["journal"]["code"], "CA1");
    assert_eq!(caisse["plafond"], "500");
    let mm = creer(
        b,
        &json!({"type":"portefeuille","nom":"Wave","segment":"WAVE","code_journal":"MMW"}),
    )
    .await;
    assert_eq!(
        mm["compte"]["numero"],
        format!("{}-WAVE", defaut(TypeTresorerie::Portefeuille))
    );
    assert_eq!(mm["journal"]["code"], "MMW");

    // Le sous-compte et le journal sont ceux du référentiel du dossier (routes de L2).
    let r = lire(
        &b.routeur,
        &format!(
            "/api/cabinets/{}/dossiers/{}/journaux",
            b.c.cabinet, b.c.dossier
        ),
        Some(b.c.administrateur),
    )
    .await;
    let journaux = r.json()["journaux"].as_array().unwrap().clone();
    let mmw = journaux.iter().find(|j| j["code"] == "MMW").unwrap();
    assert_eq!(mmw["type"], "tresorerie");
    assert_eq!(mmw["compte_tresorerie"], mm["compte"]["numero"]);
    // Audit : le compte, le journal et le compte de trésorerie, chacun inscrit.
    assert_eq!(audit(b, "compte_tresorerie").await, 4);
    assert!(audit(b, "journal").await >= 4);

    // Liste, triée par nom.
    let r = lire(&b.routeur, &chemin(b, "/comptes"), Some(b.c.collaborateur)).await;
    assert_eq!(r.statut, StatusCode::OK);
    let noms: Vec<String> = r.json()["comptes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["nom"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(
        noms,
        [
            "Banque principale",
            "Banque secondaire",
            "Caisse du siège",
            "Wave"
        ]
    );

    // Refus : segment pris (400), devise étrangère (422, L6), plafond hors caisse (422), type
    // inconnu, champ inconnu (400), journal fourni déjà pris (400, règle des journaux).
    let refus =
        |corps: Value| async move { poster(b, b.c.administrateur, "/comptes", &corps).await };
    let r = refus(json!({"type":"portefeuille","nom":"Autre","segment":"WAVE"})).await;
    assert_eq!(r.statut, StatusCode::BAD_REQUEST, "{}", r.texte());
    let r = refus(json!({"type":"banque","nom":"Devise","devise":"EUR"})).await;
    assert_eq!(r.statut, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(code(&r), "tresorerie_devise_etrangere");
    let r = refus(json!({"type":"banque","nom":"Plafond","plafond":"10"})).await;
    assert_eq!(code(&r), "tresorerie_parametre_invalide");
    let r = refus(json!({"type":"caisse","nom":"Plafond nul","plafond":"0"})).await;
    assert_eq!(code(&r), "tresorerie_parametre_invalide");
    for corps in [
        json!({"type":"coffre","nom":"X"}),
        json!({"type":"banque","nom":"X","iban":"1"}),
        json!({"type":"banque","nom":"X","code_journal":"MMW"}),
        json!({"type":"banque","nom":"X","journal_existant":"CA","segment":"A"}),
    ] {
        let r = refus(corps.clone()).await;
        assert_eq!(r.statut, StatusCode::BAD_REQUEST, "{corps} : {}", r.texte());
    }
    // Rien de plus n'a été créé.
    assert_eq!(audit(b, "compte_tresorerie").await, 4);
}

#[tokio::test]
async fn reprise_d_un_journal_de_tresorerie_existant() {
    let banc_de_test = banc().await;
    let b = &banc_de_test;
    let caisse_par_defaut = creer(
        b,
        &json!({"type":"caisse","nom":"Caisse principale","journal_existant":"CA"}),
    )
    .await;
    assert_eq!(caisse_par_defaut["compte"]["numero"], caisse());
    assert_eq!(caisse_par_defaut["journal"]["code"], "CA");
    for journal in ["CA", journal_de_saisie().as_str()] {
        let r = poster(
            b,
            b.c.administrateur,
            "/comptes",
            &json!({"type":"caisse","nom":"Encore","journal_existant": journal}),
        )
        .await;
        assert_eq!(r.statut, StatusCode::CONFLICT, "{journal} : {}", r.texte());
        assert_eq!(code(&r), "journal_deja_tresorerie");
    }
    let r = poster(
        b,
        b.c.administrateur,
        "/comptes",
        &json!({"type":"caisse","nom":"Inconnu","journal_existant":"ZZ9"}),
    )
    .await;
    assert_eq!(r.statut, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn les_avertissements_de_solde_ne_refusent_jamais_rien() {
    let banc_de_test = banc().await;
    let b = &banc_de_test;
    let caisse = creer(b, &json!({"type":"caisse","nom":"Caisse","plafond":"500"})).await;
    let bq = creer(b, &json!({"type":"banque","nom":"Banque"})).await;
    let mm = creer(b, &json!({"type":"portefeuille","nom":"Orange Money"})).await;
    let (nc, nb, nm) = (
        caisse["compte"]["numero"].as_str().unwrap().to_owned(),
        bq["compte"]["numero"].as_str().unwrap().to_owned(),
        mm["compte"]["numero"].as_str().unwrap().to_owned(),
    );
    let id = |v: &Value| v["id"].as_str().unwrap().to_owned();

    // Caisse, banque et portefeuille créditeurs : la validation réussit.
    valider(
        b,
        &[
            ligne(banque(), "debit", "3000"),
            ligne(&nc, "credit", "1000"),
            ligne(&nb, "credit", "1500"),
            ligne(&nm, "credit", "500"),
        ],
    )
    .await;
    let c = fiche(b, &id(&caisse), "").await;
    assert_eq!(c["solde"], "-1000");
    assert_eq!(codes(&c), ["caisse_solde_crediteur"]);
    assert_eq!(c["avertissements"][0]["solde"], "-1000");
    // Banque créditrice : un découvert, sans avertissement (fiche 52, p. PDF 416).
    let bq = fiche(b, &id(&bq), "").await;
    assert_eq!(bq["solde"], "-1500");
    assert_eq!(codes(&bq), AUCUN);
    let m = fiche(b, &id(&mm), "").await;
    assert_eq!(codes(&m), ["portefeuille_solde_crediteur"]);

    // Caisse au-delà de son plafond.
    valider(
        b,
        &[
            ligne(&nc, "debit", "2000"),
            ligne(banque(), "credit", "2000"),
        ],
    )
    .await;
    let c = fiche(b, &id(&caisse), "").await;
    assert_eq!(c["solde"], "1000");
    assert_eq!(codes(&c), ["caisse_plafond_depasse"]);
    assert_eq!(c["avertissements"][0]["plafond"], "500");
    // À une date antérieure aux écritures : solde nul, aucun avertissement.
    let c = fiche(b, &id(&caisse), "?au=2000-01-01").await;
    assert_eq!(c["solde"], "0");
    assert_eq!(c["au"], "2000-01-01");
    assert_eq!(codes(&c), AUCUN);
    let r = lire(
        &b.routeur,
        &chemin(b, &format!("/comptes/{}?au=hier", id(&caisse))),
        Some(b.auditeur),
    )
    .await;
    assert_eq!(r.statut, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn le_compte_de_virements_internes_non_solde_avertit() {
    let banc_de_test = banc().await;
    let b = &banc_de_test;
    let v = virement_interne();
    let lire_virements = || async {
        let r = lire(
            &b.routeur,
            &chemin(b, "/virements-internes"),
            Some(b.auditeur),
        )
        .await;
        assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
        r.json()
    };
    let l = lire_virements().await;
    assert!(
        l["comptes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["numero"] == v)
    );
    assert_eq!(codes(&l), AUCUN);
    // Une sortie de banque vers 585, sans l'entrée correspondante : 585 n'est pas soldé.
    valider(
        b,
        &[
            ligne(&v, "debit", "1000"),
            ligne(banque(), "credit", "1000"),
        ],
    )
    .await;
    let l = lire_virements().await;
    assert_eq!(codes(&l), ["virement_interne_non_solde"]);
    assert_eq!(l["avertissements"][0]["compte"], v.as_str());
    assert_eq!(l["avertissements"][0]["solde"], "1000");
    // L'entrée sur la caisse solde 585 : plus d'avertissement.
    valider(
        b,
        &[
            ligne(caisse(), "debit", "1000"),
            ligne(&v, "credit", "1000"),
        ],
    )
    .await;
    let l = lire_virements().await;
    assert_eq!(codes(&l), AUCUN);
}

#[tokio::test]
async fn modification_du_compte_de_tresorerie() {
    let banc_de_test = banc().await;
    let b = &banc_de_test;
    let caisse = creer(b, &json!({"type":"caisse","nom":"Caisse"})).await;
    let bq = creer(b, &json!({"type":"banque","nom":"Banque"})).await;
    let suffixe = |v: &Value| format!("/comptes/{}/modification", v["id"].as_str().unwrap());
    let base = sous_compte_fixe().to_string();
    let r = poster(
        b,
        b.valideur,
        &suffixe(&caisse),
        &json!({"nom":"Caisse du siège","plafond":"250000","compte_manquant": base}),
    )
    .await;
    assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
    let v = r.json();
    assert_eq!(v["nom"], "Caisse du siège");
    assert_eq!(v["plafond"], "250000");
    assert_eq!(v["compte_manquant"]["numero"], base.as_str());
    assert!(v["compte_excedent"].is_null());
    // Effacement par `null`.
    let r = poster(b, b.valideur, &suffixe(&caisse), &json!({"plafond": null})).await;
    assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
    assert!(r.json()["plafond"].is_null());
    // Rien à changer : 409 ; corps vide : 400.
    let r = poster(b, b.valideur, &suffixe(&caisse), &json!({"plafond": null})).await;
    assert_eq!(code(&r), "aucun_changement");
    let r = poster(b, b.valideur, &suffixe(&caisse), &json!({})).await;
    assert_eq!(r.statut, StatusCode::BAD_REQUEST);
    // Compte d'écart de regroupement (le compte par défaut a des sous-comptes) : 422.
    let r = poster(
        b,
        b.valideur,
        &suffixe(&caisse),
        &json!({"compte_excedent": defaut(TypeTresorerie::Banque)}),
    )
    .await;
    assert_eq!(code(&r), "validation_compte_de_regroupement");
    // Plafond ou compte d'écart sur une banque : 422 ; archivage : admis.
    let r = poster(b, b.valideur, &suffixe(&bq), &json!({"plafond":"10"})).await;
    assert_eq!(code(&r), "tresorerie_parametre_invalide");
    let r = poster(b, b.valideur, &suffixe(&bq), &json!({"archive": true})).await;
    assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
    assert_eq!(r.json()["archive"], true);
    // Le collaborateur ne paramètre pas.
    let r = poster(
        b,
        b.c.collaborateur,
        &suffixe(&bq),
        &json!({"archive": false}),
    )
    .await;
    assert_eq!(r.statut, StatusCode::FORBIDDEN);
    // Audit : une trace par modification enregistrée (deux pour la caisse, une pour la banque),
    // en plus des deux créations ; rien pour les refus.
    assert_eq!(audit(b, "compte_tresorerie").await, 5);
}

#[tokio::test]
async fn arrete_de_caisse_fige_en_piece_interne_et_ecart_propose() {
    let banc_de_test = banc().await;
    let b = &banc_de_test;
    let caisse = creer(b, &json!({"type":"caisse","nom":"Caisse"})).await;
    let bq = creer(b, &json!({"type":"banque","nom":"Banque"})).await;
    let id = caisse["id"].as_str().unwrap().to_owned();
    let nc = caisse["compte"]["numero"].as_str().unwrap().to_owned();
    valider(
        b,
        &[
            ligne(&nc, "debit", "1000"),
            ligne(banque(), "credit", "1000"),
        ],
    )
    .await;
    let arreter = |u, montant: &str| {
        let corps = json!({
            "date": aujourd(),
            "montant_compte": montant,
            "detail": ["5 billets de 200"],
        });
        let suffixe = format!("/comptes/{id}/arretes");
        async move { poster(b, u, &suffixe, &corps).await }
    };
    let pieces_avant = b.compter("piece").await;
    let brouillons_avant = b.compter("brouillon").await;

    // Comptage égal au solde : arrêté et pièce interne, aucun brouillon.
    let r = arreter(b.c.collaborateur, "1000").await;
    assert_eq!(r.statut, StatusCode::CREATED, "{}", r.texte());
    let a = r.json();
    assert_eq!(
        (
            a["montant_compte"].as_str(),
            a["solde_comptable"].as_str(),
            a["ecart"].as_str()
        ),
        (Some("1000"), Some("1000"), Some("0"))
    );
    assert!(a["brouillon"].is_null());
    assert_eq!(codes(&a), AUCUN);
    assert_eq!(b.compter("piece").await, pieces_avant + 1);
    assert_eq!(b.compter("brouillon").await, brouillons_avant);
    // La pièce est une note d'OD canonique qui cite le comptage.
    let piece = a["piece"].as_str().unwrap();
    let r = lire(
        &b.routeur,
        &format!(
            "/api/cabinets/{}/dossiers/{}/pieces/{piece}/contenu",
            b.c.cabinet, b.c.dossier
        ),
        Some(b.auditeur),
    )
    .await;
    assert_eq!(r.statut, StatusCode::OK);
    let texte = r.texte();
    assert!(
        texte.starts_with("sigma-books piece-interne v1\nnature=note-od\n"),
        "{texte}"
    );
    assert!(texte.contains("motif=Arrêté de caisse\n"));
    assert!(texte.contains(&format!("ligne=compte={nc}\n")));
    assert!(
        texte.contains("ligne=montant_compte=1000\nligne=solde_comptable=1000\nligne=ecart=0\n")
    );
    assert!(texte.contains("ligne=detail=5 billets de 200\n"));

    // Manquant sans compte désigné : arrêté enregistré, avertissement, aucun brouillon.
    let r = arreter(b.c.collaborateur, "900").await;
    assert_eq!(r.statut, StatusCode::CREATED, "{}", r.texte());
    assert_eq!(r.json()["ecart"], "-100");
    assert_eq!(codes(&r.json()), ["compte_ecart_non_designe"]);
    assert!(r.json()["brouillon"].is_null());
    assert_eq!(b.compter("brouillon").await, brouillons_avant);

    // Comptes d'écart désignés : manquant et excédent proposés au brouillard, note en pièce
    // principale ; la validation humaine les rend définitifs.
    let base = sous_compte_fixe().to_string();
    let r = poster(
        b,
        b.valideur,
        &format!("/comptes/{id}/modification"),
        &json!({"compte_manquant": base, "compte_excedent": base}),
    )
    .await;
    assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
    let r = arreter(b.c.collaborateur, "900").await;
    assert_eq!(r.statut, StatusCode::CREATED, "{}", r.texte());
    let a = r.json();
    let brouillon = a["brouillon"].as_str().unwrap().to_owned();
    let r = lire(
        &b.routeur,
        &format!("{}/{brouillon}", b.base()),
        Some(b.c.collaborateur),
    )
    .await;
    assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
    let br = r.json();
    assert_eq!(br["journal"], caisse["journal"]["code"]);
    assert_eq!(br["lignes"][0]["compte"], base.as_str());
    assert_eq!(br["lignes"][0]["sens"], "debit");
    assert_eq!(br["lignes"][0]["montant"], "100");
    assert_eq!(br["lignes"][1]["compte"], nc.as_str());
    assert_eq!(br["lignes"][1]["sens"], "credit");
    assert_eq!(br["pieces"][0]["piece"], a["piece"]);
    assert_eq!(br["pieces"][0]["role"], "principale");
    let version = i32::try_from(br["version"].as_i64().unwrap()).unwrap();
    let r = b.valider(brouillon.parse().unwrap(), version).await;
    assert!(r.statut.is_success(), "{}", r.texte());
    let c = fiche(b, &id, "").await;
    assert_eq!(c["solde"], "900");
    // Excédent : débit de la caisse.
    let r = arreter(b.c.administrateur, "950").await;
    let a = r.json();
    assert_eq!(a["ecart"], "50");
    let r = lire(
        &b.routeur,
        &format!("{}/{}", b.base(), a["brouillon"].as_str().unwrap()),
        Some(b.c.collaborateur),
    )
    .await;
    assert_eq!(r.json()["lignes"][0]["compte"], nc.as_str());
    assert_eq!(r.json()["lignes"][0]["sens"], "debit");

    // Liste des arrêtés, du plus récent au plus ancien.
    let r = lire(
        &b.routeur,
        &chemin(b, &format!("/comptes/{id}/arretes")),
        Some(b.auditeur),
    )
    .await;
    assert_eq!(r.statut, StatusCode::OK);
    assert_eq!(r.json()["arretes"].as_array().unwrap().len(), 4);
    assert_eq!(audit(b, "arrete_caisse").await, 4);

    // Refus : montant négatif ou mal formé (400), auditeur (403), banque (422), caisse
    // archivée (409).
    for montant in ["-1", "1 000", "abc"] {
        let r = arreter(b.c.collaborateur, montant).await;
        assert_eq!(r.statut, StatusCode::BAD_REQUEST, "{montant}");
    }
    let r = arreter(b.auditeur, "1").await;
    assert_eq!(r.statut, StatusCode::FORBIDDEN);
    let r = poster(
        b,
        b.c.collaborateur,
        &format!("/comptes/{}/arretes", bq["id"].as_str().unwrap()),
        &json!({"date": aujourd(), "montant_compte": "1"}),
    )
    .await;
    assert_eq!(code(&r), "tresorerie_pas_une_caisse");
    let r = poster(
        b,
        b.valideur,
        &format!("/comptes/{id}/modification"),
        &json!({"archive": true}),
    )
    .await;
    assert_eq!(r.statut, StatusCode::OK);
    let r = arreter(b.c.collaborateur, "1").await;
    assert_eq!(code(&r), "compte_tresorerie_archive");
    assert_eq!(audit(b, "arrete_caisse").await, 4);
}

#[tokio::test]
async fn rls_et_ajout_seul_des_tables_de_tresorerie() {
    let banc_de_test = banc().await;
    let b = &banc_de_test;
    let caisse = creer(b, &json!({"type":"caisse","nom":"Caisse"})).await;
    let r = poster(
        b,
        b.c.collaborateur,
        &format!("/comptes/{}/arretes", caisse["id"].as_str().unwrap()),
        &json!({"date": aujourd(), "montant_compte": "0"}),
    )
    .await;
    assert_eq!(r.statut, StatusCode::CREATED, "{}", r.texte());

    // Sans contexte : rien n'est visible.
    let mut brute = b.env.application.begin().await.unwrap();
    for table in ["compte_tresorerie", "arrete_caisse"] {
        let n: i64 =
            sqlx::query_scalar(sqlx::AssertSqlSafe(format!("SELECT count(*) FROM {table}")))
                .fetch_one(&mut *brute)
                .await
                .unwrap();
        assert_eq!(n, 0, "{table}");
    }
    drop(brute);

    // Contexte d'un autre dossier du même cabinet : rien n'est visible ni modifiable.
    let mut tx = ouvrir_transaction(
        &b.env.application,
        ContexteTenant::cabinet(b.c.cabinet, b.c.administrateur),
    )
    .await
    .unwrap();
    tx.restreindre_au_dossier(b.c.autre_dossier).await.unwrap();
    for table in ["compte_tresorerie", "arrete_caisse"] {
        let n: i64 =
            sqlx::query_scalar(sqlx::AssertSqlSafe(format!("SELECT count(*) FROM {table}")))
                .fetch_one(tx.connexion())
                .await
                .unwrap();
        assert_eq!(n, 0, "{table}");
    }
    let n = sqlx::query("UPDATE compte_tresorerie SET archive = true")
        .execute(tx.connexion())
        .await
        .unwrap()
        .rows_affected();
    assert_eq!(n, 0);
    drop(tx);

    // Dans son dossier : l'arrêté est en ajout seul (ni UPDATE ni DELETE).
    for requete in [
        "UPDATE arrete_caisse SET montant_compte = 1",
        "DELETE FROM arrete_caisse",
        "DELETE FROM compte_tresorerie",
        "UPDATE compte_tresorerie SET compte_id = compte_id",
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
        assert_eq!(
            e.as_database_error()
                .and_then(sqlx::error::DatabaseError::code)
                .as_deref(),
            Some("42501"),
            "{requete}"
        );
    }

    // La devise de tenue d'un dossier qui a un compte de trésorerie ne change plus.
    let mut admin = b.env.admin.begin().await.unwrap();
    sqlx::query("SELECT set_config('sigma.dossier', $1::text, true)")
        .bind(b.c.dossier.uuid())
        .execute(&mut *admin)
        .await
        .unwrap();
    let e =
        sqlx::query("UPDATE dossier SET devise_tenue = 'EUR', decimales_tenue = 2 WHERE id = $1")
            .bind(b.c.dossier.uuid())
            .execute(&mut *admin)
            .await
            .unwrap_err();
    assert!(e.to_string().contains("comptes de trésorerie"), "{e}");
}
