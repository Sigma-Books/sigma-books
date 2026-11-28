//! Lettrage manuel, dé-lettrage par rupture, lettres (L3-18, PR 9a de `docs/lots/L3.md` ;
//! ADR 0032 ; spec §10.1) : lettrage « au kilomètre » partiel puis total, conditions de Tryton,
//! rupture en ajout seul et refus après clôture de l'exercice (B2 option 1), concurrence (verrou
//! du compte), RLS avec le rôle applicatif, seconde barrière, audit sans valeur, aucune écriture
//! validée modifiée.
//!
//! Écritures validées par les routes (chaîne réelle). Aucun numéro de compte écrit en dur : compte
//! collectif des tiers, caisse et banque tirés des données de `syscohada`
//! (`commun::referentiel::comptes`).
#![cfg(feature = "tests-postgres")]
// Fichier de test : les `expect`/`unwrap` y sont voulus.
#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::too_many_lines,
    clippy::many_single_char_names
)]

mod commun;

use api::base::referentiel::TypeTiers;
use api::base::{ContexteTenant, IdUtilisateur, ouvrir_transaction};
use axum::http::StatusCode;
use commun::audit_valeurs::verifier_sans_valeur_metier;
use commun::banc::{Banc, aujourd, banc, caisse, code, contenu};
use commun::brouillard::{journal_de_saisie, ligne};
use commun::referentiel::comptes;
use commun::serveur::{Reponse, envoyer, lire};
use compta_core::TexteCourt;
use serde_json::{Value, json};
use uuid::Uuid;

const MOTIF: &str = "Règlement imputé sur la mauvaise facture";

fn chemin(b: &Banc, suffixe: &str) -> String {
    format!(
        "/api/cabinets/{}/dossiers/{}/lettrage{suffixe}",
        b.c.cabinet, b.c.dossier
    )
}

fn ligne_tiers(sens: &str, montant: &str, tiers: Uuid) -> String {
    format!(
        r#"{{"compte":"{}","sens":"{sens}","montant":"{montant}","tiers":"{tiers}"}}"#,
        comptes().collectif
    )
}

/// Valide une écriture du jour par les routes (brouillon, pièce, valideur) ; rend son id.
async fn valider(b: &Banc, lignes: &[String]) -> Uuid {
    let corps = contenu(&journal_de_saisie(), &aujourd(), lignes, "");
    let (id, version) = b.pret_avec(&corps).await;
    let r = b.valider(id, version).await;
    assert!(r.statut.is_success(), "{}", r.texte());
    r.json()["id"].as_str().unwrap().parse().unwrap()
}

/// Facture du tiers : ligne 1 au débit du compte collectif.
async fn facture(b: &Banc, tiers: Uuid, montant: &str) -> Uuid {
    valider(
        b,
        &[
            ligne_tiers("debit", montant, tiers),
            ligne(caisse(), "credit", montant),
        ],
    )
    .await
}

/// Règlement du tiers : ligne 1 au crédit du compte collectif.
async fn reglement(b: &Banc, tiers: Uuid, montant: &str) -> Uuid {
    valider(
        b,
        &[
            ligne_tiers("credit", montant, tiers),
            ligne(caisse(), "debit", montant),
        ],
    )
    .await
}

fn corps_lignes(lignes: &[(Uuid, u16)]) -> String {
    json!({"lignes": lignes
        .iter()
        .map(|(e, p)| json!({"ecriture": e.to_string(), "position": p}))
        .collect::<Vec<_>>()})
    .to_string()
}

async fn lettrer_par(b: &Banc, u: IdUtilisateur, lignes: &[(Uuid, u16)]) -> Reponse {
    envoyer(
        &b.routeur,
        &chemin(b, "/lettrages"),
        Some(u),
        &corps_lignes(lignes),
        None,
    )
    .await
}

async fn lettrer(b: &Banc, lignes: &[(Uuid, u16)]) -> Value {
    let r = lettrer_par(b, b.c.collaborateur, lignes).await;
    assert_eq!(r.statut, StatusCode::CREATED, "{}", r.texte());
    r.json()
}

async fn rompre(b: &Banc, lettrage: &str) -> Reponse {
    envoyer(
        &b.routeur,
        &chemin(b, &format!("/lettrages/{lettrage}/rupture")),
        Some(b.valideur),
        &json!({ "motif": MOTIF }).to_string(),
        None,
    )
    .await
}

async fn lire_tiers(b: &Banc, tiers: Uuid) -> Value {
    let r = lire(
        &b.routeur,
        &chemin(b, &format!("/tiers/{tiers}")),
        Some(b.auditeur),
    )
    .await;
    assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
    r.json()
}

/// Résiduel de la ligne (écriture, position) dans une réponse qui porte `lignes`.
fn residuel(lignes: &Value, ecriture: Uuid, position: i64) -> String {
    lignes
        .as_array()
        .unwrap()
        .iter()
        .find(|l| l["ecriture"] == ecriture.to_string() && l["position"] == position)
        .unwrap()["residuel"]
        .as_str()
        .unwrap()
        .to_owned()
}

fn lettres(lignes: &Value, ecriture: Uuid) -> Vec<String> {
    lignes
        .as_array()
        .unwrap()
        .iter()
        .find(|l| l["ecriture"] == ecriture.to_string())
        .unwrap()["lettres"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_str().unwrap().to_owned())
        .collect()
}

/// Empreinte de toutes les écritures validées et de leurs lignes du dossier (tous les champs).
async fn instantane_valide(b: &Banc) -> String {
    sqlx::query_scalar(
        "SELECT md5(COALESCE(string_agg(t, '|' ORDER BY t), '')) FROM ( \
           SELECT e::text AS t FROM ecriture_validee e WHERE dossier_id = $1 \
           UNION ALL SELECT l::text FROM ecriture_validee_ligne l WHERE dossier_id = $1) x",
    )
    .bind(b.c.dossier.uuid())
    .fetch_one(&b.env.admin)
    .await
    .unwrap()
}

/// Un second tiers client du dossier (code de la production, sans l'API).
async fn autre_tiers(b: &Banc) -> Uuid {
    let id = Uuid::new_v4();
    let mut tx = ouvrir_transaction(
        &b.env.application,
        ContexteTenant::dossier(b.c.cabinet, b.c.dossier, b.c.administrateur),
    )
    .await
    .unwrap();
    api::base::tiers::creer(
        &mut tx,
        &api::base::tiers::NouveauTiers {
            id,
            code: None,
            nom: TexteCourt::parse("Second client").unwrap(),
            genre: TypeTiers::Client,
            compte_collectif: comptes().collectif.clone(),
            devise: None,
            assujetti_tva: false,
            soumis_retenue: false,
        },
    )
    .await
    .unwrap();
    tx.confirmer().await.unwrap();
    id
}

#[tokio::test]
async fn lettrage_au_kilometre_partiel_puis_total_sans_toucher_les_ecritures() {
    let b = banc().await;
    let t = b.f.tiers;
    let f1 = facture(&b, t, "1000").await;
    let f2 = facture(&b, t, "500").await;
    let r1 = reglement(&b, t, "1200").await;
    let avant = instantane_valide(&b).await;

    // Un appel, trois lignes ; ordre de la sélection indifférent : imputation du plus ancien au
    // plus récent (Tryton `move.py:1873-1874`, Odoo `:2735-2745`).
    let a = lettrer(&b, &[(r1, 1), (f2, 1), (f1, 1)]).await;
    assert_eq!(a["lettre"], "A");
    assert_eq!(a["origine"], "manuelle");
    assert_eq!(a["date"], aujourd());
    assert_eq!(a["total"], false);
    assert_eq!(a["avertissements"], json!([]));
    assert_eq!(
        a["liens"],
        json!([
            {"debit": {"ecriture": f1.to_string(), "position": 1},
             "credit": {"ecriture": r1.to_string(), "position": 1}, "montant": "1000"},
            {"debit": {"ecriture": f2.to_string(), "position": 1},
             "credit": {"ecriture": r1.to_string(), "position": 1}, "montant": "200"},
        ])
    );
    assert_eq!(residuel(&a["lignes"], f1, 1), "0");
    assert_eq!(residuel(&a["lignes"], f2, 1), "300");
    assert_eq!(residuel(&a["lignes"], r1, 1), "0");

    // Le complément : nouvelle lettre ; la facture partielle porte deux lettres.
    assert_eq!(instantane_valide(&b).await, avant);
    let r2 = reglement(&b, t, "300").await;
    let avant = instantane_valide(&b).await;
    let bb = lettrer(&b, &[(f2, 1), (r2, 1)]).await;
    assert_eq!(bb["lettre"], "B");
    assert_eq!(bb["total"], true);
    let lu = lire_tiers(&b, t).await;
    assert_eq!(lu["id"], t.to_string());
    let compte = &lu["comptes"][0];
    assert_eq!(compte["compte"]["numero"], comptes().collectif.to_string());
    for e in [f1, f2, r1, r2] {
        assert_eq!(residuel(&compte["lignes"], e, 1), "0");
    }
    assert_eq!(lettres(&compte["lignes"], f2), ["A", "B"]);
    assert_eq!(lettres(&compte["lignes"], f1), ["A"]);
    let lettrages = compte["lettrages"].as_array().unwrap();
    assert_eq!(lettrages.len(), 2);
    // A est devenu total avec le complément B : statut calculé, jamais stocké.
    assert!(lettrages.iter().all(|l| l["total"] == true));
    assert_eq!(lettrages[0]["date_effet"], aujourd());

    // Somme des liens = montant imputé ; débit imputé = crédit imputé, résiduel jamais négatif.
    let (debit, credit): (String, String) = sqlx::query_as(
        "SELECT sum(montant)::text, sum(montant)::text FROM lettrage_lien WHERE dossier_id = $1",
    )
    .bind(b.c.dossier.uuid())
    .fetch_one(&b.env.admin)
    .await
    .unwrap();
    assert_eq!((debit.as_str(), credit.as_str()), ("1500", "1500"));
    let negatifs: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM ecriture_validee_ligne l WHERE l.dossier_id = $1 AND l.montant < \
         (SELECT COALESCE(sum(k.montant), 0) FROM lettrage_lien k WHERE k.dossier_id = l.dossier_id \
          AND ((k.debit_ecriture_id, k.debit_position) = (l.ecriture_id, l.position) \
               OR (k.credit_ecriture_id, k.credit_position) = (l.ecriture_id, l.position)))",
    )
    .bind(b.c.dossier.uuid())
    .fetch_one(&b.env.admin)
    .await
    .unwrap();
    assert_eq!(negatifs, 0);

    // Aucune écriture validée ni aucun contenu haché n'a changé ; la chaîne se recalcule.
    assert_eq!(instantane_valide(&b).await, avant);
    assert!(b.chaine_valide().await);

    // Une ligne soldée ne se relettre pas (Tryton : `msg_line_already_reconciled`).
    let r3 = reglement(&b, t, "100").await;
    let r = lettrer_par(&b, b.c.administrateur, &[(f1, 1), (r3, 1)]).await;
    assert_eq!(r.statut, StatusCode::CONFLICT, "{}", r.texte());
    assert_eq!(code(&r), "lettrage_ligne_soldee");
}

#[tokio::test]
async fn conditions_de_tryton_verifiees() {
    let b = banc().await;
    let t = b.f.tiers;
    let f = facture(&b, t, "1000").await;
    let r = reglement(&b, t, "1000").await;
    let refus = |reponse: &Reponse, statut: StatusCode, attendu: &str| {
        assert_eq!(reponse.statut, statut, "{}", reponse.texte());
        assert_eq!(code(reponse), attendu);
    };
    let u = b.c.administrateur;
    let n = StatusCode::UNPROCESSABLE_ENTITY;

    // Ligne inexistante, ou d'un autre dossier : même refus.
    refus(
        &lettrer_par(&b, u, &[(f, 1), (Uuid::new_v4(), 1)]).await,
        n,
        "lettrage_ligne_inconnue",
    );
    commun::referentiel::doter(&b.env.application, b.c.cabinet, b.c.autre_dossier, u).await;
    let ailleurs =
        commun::brouillard::doter_validee(&b.env.admin, b.c.cabinet, b.c.autre_dossier, u).await;
    refus(
        &lettrer_par(&b, u, &[(f, 1), (ailleurs, 2)]).await,
        n,
        "lettrage_ligne_inconnue",
    );
    // Comptes différents (le compte collectif et la caisse).
    refus(
        &lettrer_par(&b, u, &[(f, 1), (f, 2)]).await,
        n,
        "lettrage_comptes_differents",
    );
    // Compte non lettrable (caisse : marque `lettrable` absente des données).
    refus(
        &lettrer_par(&b, u, &[(f, 2), (r, 2)]).await,
        n,
        "lettrage_compte_non_lettrable",
    );
    // Un seul sens ; ligne en double.
    let f2 = facture(&b, t, "200").await;
    refus(
        &lettrer_par(&b, u, &[(f, 1), (f2, 1)]).await,
        n,
        "lettrage_sens_manquant",
    );
    refus(
        &lettrer_par(&b, u, &[(f, 1), (r, 1), (f, 1)]).await,
        n,
        "lettrage_ligne_en_double",
    );
    // Tiers différents (Tryton `move.py:676-682`).
    let autre = autre_tiers(&b).await;
    let r_autre = reglement(&b, autre, "1000").await;
    refus(
        &lettrer_par(&b, u, &[(f, 1), (r_autre, 1)]).await,
        n,
        "lettrage_tiers_differents",
    );
    // Forme : position nulle, champ inconnu, trop de lignes.
    let r0 = envoyer(
        &b.routeur,
        &chemin(&b, "/lettrages"),
        Some(u),
        &json!({"lignes": [{"ecriture": f.to_string(), "position": 0}]}).to_string(),
        None,
    )
    .await;
    assert_eq!(r0.statut, StatusCode::BAD_REQUEST);
    let r0 = envoyer(
        &b.routeur,
        &chemin(&b, "/lettrages"),
        Some(u),
        &json!({"lignes": [], "compte": "x"}).to_string(),
        None,
    )
    .await;
    assert_eq!(r0.statut, StatusCode::BAD_REQUEST);
    let trop: Vec<(Uuid, u16)> = (0..=200).map(|_| (f, 1)).collect();
    refus(
        &lettrer_par(&b, u, &trop).await,
        n,
        "lettrage_trop_de_lignes",
    );
    // Rôle : l'auditeur ne lettre pas (Cedar, puis rôle relu en base).
    let a = lettrer_par(&b, b.auditeur, &[(f, 1), (r, 1)]).await;
    assert_eq!(a.statut, StatusCode::FORBIDDEN);
    // Rien n'a été écrit par les refus.
    assert_eq!(b.compter("lettrage").await, 0);
    assert_eq!(b.compter("lettrage_lien").await, 0);
    // Une ligne d'un compte lettrable sans tiers ni période contrôlée : le lettrage réussit.
    lettrer(&b, &[(f, 1), (r, 1)]).await;
}

#[tokio::test]
async fn rupture_en_ajout_seul_puis_relettrage() {
    let b = banc().await;
    let t = b.f.tiers;
    let f = facture(&b, t, "1000").await;
    let r = reglement(&b, t, "1000").await;
    let a = lettrer(&b, &[(f, 1), (r, 1)]).await;
    let id = a["id"].as_str().unwrap().to_owned();
    let avant = instantane_valide(&b).await;
    let liens: Vec<(Uuid, i16, String)> = sqlx::query_as(
        "SELECT lettrage_id, position, montant::text FROM lettrage_lien WHERE dossier_id = $1",
    )
    .bind(b.c.dossier.uuid())
    .fetch_all(&b.env.admin)
    .await
    .unwrap();

    // Motif exigé, contrôlé (vide, trop long, caractère de format).
    for motif in ["", "   ", &"x".repeat(401), "a\u{202E}b"] {
        let rep = envoyer(
            &b.routeur,
            &chemin(&b, &format!("/lettrages/{id}/rupture")),
            Some(b.valideur),
            &json!({ "motif": motif }).to_string(),
            None,
        )
        .await;
        assert_eq!(rep.statut, StatusCode::UNPROCESSABLE_ENTITY, "{motif}");
        assert_eq!(code(&rep), "lettrage_motif_invalide");
    }
    let rep = rompre(&b, &id).await;
    assert_eq!(rep.statut, StatusCode::CREATED, "{}", rep.texte());
    let rupture = rep.json();
    assert_eq!(rupture["lettrage"], id);
    assert_eq!(rupture["lettre"], "A");
    assert_eq!(rupture["date"], aujourd());
    assert_eq!(residuel(&rupture["lignes"], f, 1), "1000");
    assert_eq!(residuel(&rupture["lignes"], r, 1), "1000");

    // Ajout seul : le lettrage et ses liens sont toujours là, intacts ; une rupture de plus.
    assert_eq!(b.compter("lettrage").await, 1);
    let apres: Vec<(Uuid, i16, String)> = sqlx::query_as(
        "SELECT lettrage_id, position, montant::text FROM lettrage_lien WHERE dossier_id = $1",
    )
    .bind(b.c.dossier.uuid())
    .fetch_all(&b.env.admin)
    .await
    .unwrap();
    assert_eq!(apres, liens);
    assert_eq!(b.compter("lettrage_rupture").await, 1);
    assert_eq!(instantane_valide(&b).await, avant);

    // Au plus une rupture.
    let rep = rompre(&b, &id).await;
    assert_eq!(rep.statut, StatusCode::CONFLICT);
    assert_eq!(code(&rep), "lettrage_deja_rompu");

    // Re-lettrer crée un nouveau lettrage, nouvelle lettre ; A n'est jamais réutilisée.
    let bb = lettrer(&b, &[(f, 1), (r, 1)]).await;
    assert_eq!(bb["lettre"], "B");
    let lu = lire_tiers(&b, t).await;
    let compte = &lu["comptes"][0];
    assert_eq!(lettres(&compte["lignes"], f), ["B"]);
    let lettrages = compte["lettrages"].as_array().unwrap();
    assert_eq!(lettrages[0]["lettre"], "A");
    assert_eq!(lettrages[0]["rupture"]["motif"], MOTIF);
    assert_eq!(lettrages[0]["total"], false);
    assert_eq!(lettrages[1]["lettre"], "B");
    assert_eq!(lettrages[1]["rupture"], Value::Null);
    // Rangs d'opération persistés : lettrage A (1), rupture (2), lettrage B (3).
    let rangs: Vec<i64> = sqlx::query_scalar(
        "SELECT rang FROM (SELECT rang FROM lettrage WHERE dossier_id = $1 \
         UNION ALL SELECT rang FROM lettrage_rupture WHERE dossier_id = $1) x ORDER BY rang",
    )
    .bind(b.c.dossier.uuid())
    .fetch_all(&b.env.admin)
    .await
    .unwrap();
    assert_eq!(rangs, [1, 2, 3]);

    // Le rôle applicatif ne peut ni modifier ni supprimer un lien, un lettrage ou une rupture
    // (F2.15) : la rupture est la seule voie.
    for requete in [
        "UPDATE lettrage_lien SET montant = 1",
        "DELETE FROM lettrage_lien",
        "UPDATE lettrage SET lettre = 'Z'",
        "DELETE FROM lettrage",
        "DELETE FROM lettrage_rupture",
        "UPDATE lettrage_rupture SET motif = 'autre'",
        "DELETE FROM compteur_lettrage",
    ] {
        let mut tx = ouvrir_transaction(
            &b.env.application,
            ContexteTenant::dossier(b.c.cabinet, b.c.dossier, b.c.administrateur),
        )
        .await
        .unwrap();
        let e = sqlx::query(sqlx::AssertSqlSafe(requete.to_owned()))
            .execute(tx.connexion())
            .await
            .unwrap_err();
        assert!(
            e.to_string().contains("permission denied"),
            "{requete} : {e}"
        );
    }
    // Lecture sans paramètre ; compte non lettrable : 422 ; compte lettrable : ses lignes.
    let collectif: Uuid =
        sqlx::query_scalar("SELECT id FROM compte WHERE dossier_id = $1 AND numero = $2")
            .bind(b.c.dossier.uuid())
            .bind(comptes().collectif.to_string())
            .fetch_one(&b.env.admin)
            .await
            .unwrap();
    let rep = lire(
        &b.routeur,
        &chemin(&b, &format!("/comptes/{collectif}")),
        Some(b.auditeur),
    )
    .await;
    assert_eq!(rep.statut, StatusCode::OK, "{}", rep.texte());
    assert_eq!(
        rep.json()["comptes"][0]["lettrages"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    let rep = lire(
        &b.routeur,
        &chemin(&b, &format!("/comptes/{}", b.f.compte)),
        Some(b.auditeur),
    )
    .await;
    assert_eq!(rep.statut, StatusCode::UNPROCESSABLE_ENTITY);
    let rep = lire(
        &b.routeur,
        &chemin(&b, &format!("/tiers/{t}?x=1")),
        Some(b.auditeur),
    )
    .await;
    assert_eq!(rep.statut, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn delettrage_refuse_apres_cloture_de_l_exercice_du_lettrage() {
    let b = banc().await;
    let t = b.f.tiers;
    let f = facture(&b, t, "1000").await;
    let r = reglement(&b, t, "1000").await;
    let f2 = facture(&b, t, "400").await;
    let r2 = reglement(&b, t, "400").await;
    let a = lettrer(&b, &[(f, 1), (r, 1)]).await;
    let id = a["id"].as_str().unwrap().to_owned();
    // Exercice de la date de l'opération clos (B2, option 1 ; spec §10.1).
    b.sql(&format!(
        "UPDATE periode SET statut = 'cloturee', statut_par = '{}', statut_le = now() \
         WHERE dossier_id = $1",
        b.c.administrateur
    ))
    .await;
    b.sql("UPDATE exercice SET statut = 'cloture' WHERE dossier_id = $1")
        .await;
    let rep = rompre(&b, &id).await;
    assert_eq!(rep.statut, StatusCode::CONFLICT, "{}", rep.texte());
    assert_eq!(code(&rep), "lettrage_exercice_clos");
    assert_eq!(b.compter("lettrage_rupture").await, 0);
    // Lettrer reste possible (aucun contrôle de période au lettrage, consensus des moteurs) ;
    // l'avertissement signale qu'il ne pourra pas être rompu.
    let bb = lettrer(&b, &[(f2, 1), (r2, 1)]).await;
    assert_eq!(bb["lettre"], "B");
    assert_eq!(bb["avertissements"], json!(["lettrage_non_rompable"]));
}

#[tokio::test]
async fn rupture_acceptee_avant_cloture() {
    let b = banc().await;
    let t = b.f.tiers;
    let f = facture(&b, t, "1000").await;
    let r = reglement(&b, t, "1000").await;
    let a = lettrer(&b, &[(f, 1), (r, 1)]).await;
    // Une période verrouillée ne gêne ni la rupture ni le lettrage (aucun contrôle de verrou).
    b.sql("UPDATE periode SET statut = 'verrouillee' WHERE dossier_id = $1")
        .await;
    let rep = rompre(&b, a["id"].as_str().unwrap()).await;
    assert_eq!(rep.statut, StatusCode::CREATED, "{}", rep.texte());
}

#[tokio::test]
async fn lettrages_concurrents_sans_double_imputation_ni_lettre_en_double() {
    let b = banc().await;
    let t = b.f.tiers;
    for _ in 0..3 {
        // Deux règlements concurrents de la même facture : un seul l'impute.
        let f = facture(&b, t, "1000").await;
        let r1 = reglement(&b, t, "1000").await;
        let r2 = reglement(&b, t, "1000").await;
        let (s1, s2) = ([(f, 1), (r1, 1)], [(f, 1), (r2, 1)]);
        let (x, y) = tokio::join!(
            lettrer_par(&b, b.c.collaborateur, &s1),
            lettrer_par(&b, b.valideur, &s2),
        );
        let mut statuts = [x.statut, y.statut];
        statuts.sort();
        assert_eq!(
            statuts,
            [StatusCode::CREATED, StatusCode::CONFLICT],
            "{} / {}",
            x.texte(),
            y.texte()
        );
        // Deux lettrages concurrents de lignes distinctes du même compte : deux lettres.
        let (f1, f2) = (facture(&b, t, "10").await, facture(&b, t, "20").await);
        let (g1, g2) = (reglement(&b, t, "10").await, reglement(&b, t, "20").await);
        let (s1, s2) = ([(f1, 1), (g1, 1)], [(f2, 1), (g2, 1)]);
        let (x, y) = tokio::join!(
            lettrer_par(&b, b.c.collaborateur, &s1),
            lettrer_par(&b, b.valideur, &s2),
        );
        assert_eq!(x.statut, StatusCode::CREATED, "{}", x.texte());
        assert_eq!(y.statut, StatusCode::CREATED, "{}", y.texte());
        assert_ne!(x.json()["lettre"], y.json()["lettre"]);
    }
    // Suite continue, sans doublon : A, B, … ; aucun résiduel négatif.
    let lettres: Vec<String> =
        sqlx::query_scalar("SELECT lettre FROM lettrage WHERE dossier_id = $1 ORDER BY rang")
            .bind(b.c.dossier.uuid())
            .fetch_all(&b.env.admin)
            .await
            .unwrap();
    assert_eq!(lettres, ["A", "B", "C", "D", "E", "F", "G", "H", "I"]);
    let depasses: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM ecriture_validee_ligne l WHERE l.dossier_id = $1 AND l.montant < \
         (SELECT COALESCE(sum(k.montant), 0) FROM lettrage_lien k WHERE k.dossier_id = l.dossier_id \
          AND ((k.debit_ecriture_id, k.debit_position) = (l.ecriture_id, l.position) \
               OR (k.credit_ecriture_id, k.credit_position) = (l.ecriture_id, l.position)))",
    )
    .bind(b.c.dossier.uuid())
    .fetch_one(&b.env.admin)
    .await
    .unwrap();
    assert_eq!(depasses, 0);
}

#[tokio::test]
async fn rls_et_seconde_barriere_avec_le_role_applicatif() {
    let b = banc().await;
    let t = b.f.tiers;
    let f = facture(&b, t, "1000").await;
    let r = reglement(&b, t, "600").await;
    let a = lettrer(&b, &[(f, 1), (r, 1)]).await;
    let lettrage: Uuid = a["id"].as_str().unwrap().parse().unwrap();
    let r2 = reglement(&b, t, "1000").await;
    // Une écriture validée dans l'autre dossier du cabinet.
    let u = b.c.administrateur;
    commun::referentiel::doter(&b.env.application, b.c.cabinet, b.c.autre_dossier, u).await;
    let ailleurs =
        commun::brouillard::doter_validee(&b.env.admin, b.c.cabinet, b.c.autre_dossier, u).await;
    let contexte = ContexteTenant::dossier(b.c.cabinet, b.c.dossier, u);
    let essai = |requete: &'static str, liaisons: Vec<Uuid>| {
        let pool = b.env.application.clone();
        async move {
            let mut tx = ouvrir_transaction(&pool, contexte).await.unwrap();
            let mut q = sqlx::query(requete);
            for l in liaisons {
                q = q.bind(l);
            }
            q.execute(tx.connexion()).await.map(|_| ())
        }
    };
    let (cab, dos, autre) = (
        b.c.cabinet.uuid(),
        b.c.dossier.uuid(),
        b.c.autre_dossier.uuid(),
    );
    // Lien vers une ligne d'un autre dossier : clé étrangère composite (dossier compris).
    let e = essai(
        "INSERT INTO lettrage_lien (cabinet_id, dossier_id, lettrage_id, position, \
         debit_ecriture_id, debit_position, credit_ecriture_id, credit_position, montant) \
         VALUES ($1, $2, $3, 9, $4, 1, $5, 1, 1)",
        vec![cab, dos, lettrage, ailleurs, r2],
    )
    .await
    .unwrap_err();
    assert!(e.to_string().contains("foreign key"), "{e}");
    // Ligne écrite sous le dossier de l'autre : refusée par la politique (`WITH CHECK`).
    let e = essai(
        "INSERT INTO lettrage (cabinet_id, dossier_id, id, compte_id, lettre, rang, \
         date_operation, origine, auteur, cree_le) \
         SELECT $1, $2, gen_random_uuid(), compte_id, 'ZZ', 99, current_date, 'manuelle', \
                auteur, now() FROM lettrage WHERE id = $3",
        vec![cab, autre, lettrage],
    )
    .await
    .unwrap_err();
    assert!(e.to_string().contains("row-level security"), "{e}");
    // Les tables du lettrage d'un autre dossier sont invisibles.
    let mut tx = ouvrir_transaction(
        &b.env.application,
        ContexteTenant::dossier(b.c.cabinet, b.c.autre_dossier, u),
    )
    .await
    .unwrap();
    for table in ["lettrage", "lettrage_lien", "compteur_lettrage"] {
        let n: i64 =
            sqlx::query_scalar(sqlx::AssertSqlSafe(format!("SELECT count(*) FROM {table}")))
                .fetch_one(tx.connexion())
                .await
                .unwrap();
        assert_eq!(n, 0, "{table}");
    }
    drop(tx);
    // Seconde barrière : un lien qui dépasserait le résiduel (400 restants sur la facture) ou au
    // mauvais sens est refusé par le déclencheur, même sans passer par l'API.
    let e = essai(
        "INSERT INTO lettrage_lien (cabinet_id, dossier_id, lettrage_id, position, \
         debit_ecriture_id, debit_position, credit_ecriture_id, credit_position, montant) \
         VALUES ($1, $2, $3, 9, $4, 1, $5, 1, 401)",
        vec![cab, dos, lettrage, f, r2],
    )
    .await
    .unwrap_err();
    assert!(e.to_string().contains("résiduel négatif"), "{e}");
    let e = essai(
        "INSERT INTO lettrage_lien (cabinet_id, dossier_id, lettrage_id, position, \
         debit_ecriture_id, debit_position, credit_ecriture_id, credit_position, montant) \
         VALUES ($1, $2, $3, 9, $5, 1, $4, 1, 1)",
        vec![cab, dos, lettrage, f, r2],
    )
    .await
    .unwrap_err();
    assert!(e.to_string().contains("incohérent"), "{e}");
}

#[tokio::test]
async fn audit_chaine_sans_valeur_personnelle() {
    let b = banc().await;
    let t = b.f.tiers;
    let f = facture(&b, t, "1234").await;
    let r = reglement(&b, t, "1234").await;
    let a = lettrer(&b, &[(f, 1), (r, 1)]).await;
    let rep = rompre(&b, a["id"].as_str().unwrap()).await;
    assert_eq!(rep.statut, StatusCode::CREATED);
    let entrees: Vec<String> = sqlx::query_scalar(
        "SELECT contenu FROM audit_dossier WHERE dossier_id = $1 \
         AND (contenu LIKE '%ressource=lettrage:%' OR contenu LIKE '%ressource=lettrage_rupture:%') \
         ORDER BY rang",
    )
    .bind(b.c.dossier.uuid())
    .fetch_all(&b.env.admin)
    .await
    .unwrap();
    assert_eq!(entrees.len(), 2, "{entrees:?}");
    for e in &entrees {
        verifier_sans_valeur_metier(e, &[MOTIF, "Écriture de test", "Tiers fixe"], &["1234"]);
        assert!(e.contains("lettre"), "{e}");
    }
    assert!(entrees[0].contains("origine"));
    assert!(entrees[1].contains(a["id"].as_str().unwrap()));
    // Les lectures ne s'inscrivent pas ; la chaîne d'audit du dossier se vérifie.
    lire_tiers(&b, t).await;
    let rep = lire(
        &b.routeur,
        &format!(
            "/api/cabinets/{}/dossiers/{}/audit/verification",
            b.c.cabinet, b.c.dossier
        ),
        Some(b.c.administrateur),
    )
    .await;
    assert_eq!(rep.statut, StatusCode::OK, "{}", rep.texte());
    assert_eq!(rep.json()["integre"], true);
}

/// Résiduels de l'interface de l'échéancier (`base::lettrage::residuels`), dans le dossier.
async fn residuels_base(b: &Banc, refs: &[(Uuid, i16)]) -> Vec<String> {
    let mut tx = ouvrir_transaction(
        &b.env.application,
        ContexteTenant::dossier(b.c.cabinet, b.c.dossier, b.c.administrateur),
    )
    .await
    .unwrap();
    let lus = api::base::lettrage::residuels(&mut tx, refs).await.unwrap();
    refs.iter().map(|r| lus[r].clone()).collect()
}

#[tokio::test]
async fn residuels_par_agregat_egaux_a_ceux_du_registre() {
    let b = banc().await;
    let t = b.f.tiers;
    let f = facture(&b, t, "1000").await;
    let r1 = reglement(&b, t, "250").await;
    let r2 = reglement(&b, t, "100").await;
    let refs = [(f, 1), (r1, 1), (r2, 1)];
    assert_eq!(residuels_base(&b, &refs).await, ["1000", "250", "100"]);
    let a = lettrer(&b, &[(f, 1), (r1, 1)]).await;
    lettrer(&b, &[(f, 1), (r2, 1)]).await;
    let lu = lire_tiers(&b, t).await;
    let lignes = &lu["comptes"][0]["lignes"];
    let du_registre: Vec<String> = refs
        .iter()
        .map(|(e, p)| residuel(lignes, *e, i64::from(*p)))
        .collect();
    assert_eq!(du_registre, ["650", "0", "0"]);
    assert_eq!(residuels_base(&b, &refs).await, du_registre);
    // La rupture rétablit le résiduel, des deux côtés.
    let rep = rompre(&b, a["id"].as_str().unwrap()).await;
    assert_eq!(rep.statut, StatusCode::CREATED);
    assert_eq!(residuels_base(&b, &refs).await, ["900", "250", "0"]);
    // Une ligne d'un autre dossier ou inexistante est absente.
    let mut tx = ouvrir_transaction(
        &b.env.application,
        ContexteTenant::dossier(b.c.cabinet, b.c.dossier, b.c.administrateur),
    )
    .await
    .unwrap();
    assert!(
        api::base::lettrage::residuels(&mut tx, &[(Uuid::new_v4(), 1)])
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn revision_d_echeance_et_lettrage_concurrents_se_suivent() {
    let b = banc().await;
    let t = b.f.tiers;
    let f = facture(&b, t, "1000").await;
    let r = reglement(&b, t, "1000").await;
    // Une révision d'échéance (ADR 0033 point 2) tient le verrou du compte de la ligne et lit son
    // résiduel sous ce verrou.
    let mut revision = ouvrir_transaction(
        &b.env.application,
        ContexteTenant::dossier(b.c.cabinet, b.c.dossier, b.c.administrateur),
    )
    .await
    .unwrap();
    let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(revision.connexion())
        .await
        .unwrap();
    assert!(
        api::base::lettrage::verrouiller_compte_de_ligne(&mut revision, f, 1)
            .await
            .unwrap()
            .is_some()
    );
    let lu = api::base::lettrage::residuels(&mut revision, &[(f, 1)])
        .await
        .unwrap();
    assert_eq!(lu[&(f, 1)], "1000");
    let selection = [(f, 1), (r, 1)];
    let lettrage = lettrer_par(&b, b.c.collaborateur, &selection);
    let revision_terminee = async {
        // Le lettrage attend le verrou tenu par la révision (constaté, sans mesure de durée),
        // puis la révision se termine sur le résiduel lu.
        loop {
            let bloques: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM pg_stat_activity WHERE $1 = ANY(pg_blocking_pids(pid))",
            )
            .bind(pid)
            .fetch_one(&b.env.admin)
            .await
            .unwrap();
            if bloques > 0 {
                break;
            }
            tokio::task::yield_now().await;
        }
        let relu = api::base::lettrage::residuels(&mut revision, &[(f, 1)])
            .await
            .unwrap();
        assert_eq!(relu[&(f, 1)], "1000");
        revision.confirmer().await.unwrap();
    };
    let (rep, ()) = tokio::join!(lettrage, revision_terminee);
    assert_eq!(rep.statut, StatusCode::CREATED, "{}", rep.texte());
    // Une révision suivante voit la ligne soldée.
    let mut suivante = ouvrir_transaction(
        &b.env.application,
        ContexteTenant::dossier(b.c.cabinet, b.c.dossier, b.c.administrateur),
    )
    .await
    .unwrap();
    assert!(
        api::base::lettrage::verrouiller_compte_de_ligne(&mut suivante, f, 1)
            .await
            .unwrap()
            .is_some()
    );
    let lu = api::base::lettrage::residuels(&mut suivante, &[(f, 1)])
        .await
        .unwrap();
    assert_eq!(lu[&(f, 1)], "0");
    assert!(
        api::base::lettrage::verrouiller_compte_de_ligne(&mut suivante, Uuid::new_v4(), 1)
            .await
            .unwrap()
            .is_none()
    );
}
