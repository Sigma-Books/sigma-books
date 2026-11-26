//! Intégrité d'un dossier (PR 15b, L2-31, #188 ; ADR 0007, 0010 ; F2.2, F2.4, F2.5, F7.4) : état,
//! jetons, vérification de la chaîne depuis la base (y compris une altération faite par le
//! propriétaire des tables), export d'intégrité (identique à celui d'`integrite`, vérifiable par
//! `sigma-verifier`, ré-authentification, audit, alerte), témoin.
//!
//! Isolation de chaque route : `tests/isolation/integrite.rs`. Rapprochement des colonnes, des
//! jetons et de la tête : tests de `api::verification`.
#![cfg(feature = "tests-postgres")]
// Fichier de test : les `expect`/`unwrap` y sont voulus.
#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::too_many_lines,
    clippy::format_collect,
    clippy::assert_is_empty,
    clippy::similar_names
)]

mod commun;

use api::base::validation as base;
use api::base::{ContexteTenant, ouvrir_transaction};
use api::serveur::routes::brouillard::validation::reconstituer;
use axum::http::StatusCode;
use commun::banc::{Banc, aujourd, banc, banc_avec, banque, caisse, contenu};
use commun::brouillard::{journal_de_saisie, ligne};
use commun::serveur::{HorlogeTest, envoyer, lire};
use commun::{ROLE_MIGRATION, pool_sous_role};
use integrite::{IdDossier, Options};
use serde_json::Value;
use time::Duration;
use uuid::Uuid;

fn chemin(b: &Banc, suite: &str) -> String {
    format!(
        "/api/cabinets/{}/dossiers/{}/integrite{suite}",
        b.c.cabinet, b.c.dossier
    )
}

async fn valider_n(b: &Banc, n: usize) {
    for i in 1..=n {
        let (id, v, _) = b.pret(&format!("Écriture {i}")).await;
        assert_eq!(b.valider(id, v).await.statut, StatusCode::OK);
    }
}

async fn json(b: &Banc, suite: &str) -> Value {
    let r = lire(&b.routeur, &chemin(b, suite), Some(b.auditeur)).await;
    assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
    r.json()
}

/// Exécute `sql` sous le rôle propriétaire des tables, au contexte du dossier : la RLS forcée
/// s'applique aussi à lui, mais il a les droits d'écriture que n'a pas le rôle applicatif.
async fn sous_proprietaire(b: &Banc, sql: &str) -> u64 {
    let pool = pool_sous_role(ROLE_MIGRATION).await;
    let mut tx = pool.begin().await.unwrap();
    sqlx::query(
        "SELECT set_config('sigma.cabinet', $1, true), set_config('sigma.dossier', $2, true)",
    )
    .bind(b.c.cabinet.to_string())
    .bind(b.c.dossier.to_string())
    .execute(&mut *tx)
    .await
    .unwrap();
    let n = sqlx::query(sqlx::AssertSqlSafe(sql.to_owned()))
        .execute(&mut *tx)
        .await
        .unwrap()
        .rows_affected();
    tx.commit().await.unwrap();
    n
}

async fn inserer_jeton(b: &Banc, nombre: i64) {
    sqlx::query(
        "INSERT INTO jeton_horodatage (cabinet_id, dossier_id, id, nombre, tete, autorite, \
                                       date_jeton, obtenu_le, declencheur, jeton) \
         SELECT $1, $2, gen_random_uuid(), $3, empreinte, 'tsa.exemple.invalide:80', now(), now(), \
                'quotidien', '\\x0102'::bytea \
         FROM ecriture_validee WHERE dossier_id = $2 AND rang = $3",
    )
    .bind(b.c.cabinet.uuid())
    .bind(b.c.dossier.uuid())
    .bind(nombre)
    .execute(&b.env.admin)
    .await
    .unwrap();
}

#[tokio::test]
async fn l_etat_d_un_dossier_vide_et_d_un_dossier_valide() {
    let horloge = HorlogeTest::default();
    let b = banc_avec(Some(&horloge)).await;
    let genese = integrite::genese(&IdDossier::parse(&b.c.dossier.to_string()).unwrap());
    let e = json(&b, "").await;
    assert_eq!(e["nombre"], 0);
    assert_eq!(e["tete"], genese.to_string());
    assert_eq!(e["jetons"], 0);
    assert_eq!(e["horodatage"]["retard"], false);
    assert!(e["horodatage"]["en_attente_depuis"].is_null());
    assert_eq!(
        e["temoin"]["contenu"],
        format!(
            "sigma-books temoin v1\ndossier={}\ntete={genese}\nnombre=0\n",
            b.c.dossier
        )
    );
    valider_n(&b, 2).await;
    inserer_jeton(&b, 1).await;
    let e = json(&b, "").await;
    let chaine = b.chaine().await;
    assert_eq!(e["nombre"], 2);
    assert_eq!(e["jetons"], 1);
    assert_eq!(e["dernier_jeton"]["nombre"], 1);
    assert_eq!(e["ecritures_sans_jeton"], 1);
    // Une écriture attend un jeton, sans retard encore ; au-delà de 24 h : retard (F6.2).
    assert!(e["horodatage"]["en_attente_depuis"].is_string());
    assert_eq!(e["horodatage"]["retard"], false);
    horloge.avancer(Duration::hours(25));
    let e = json(&b, "").await;
    assert_eq!(e["horodatage"]["retard"], true);
    // Le témoin courant est celui de la tête.
    let tete: String = chaine[1].3.iter().map(|o| format!("{o:02x}")).collect();
    assert_eq!(e["temoin"]["tete"], tete);
    let t = lire(&b.routeur, &chemin(&b, "/temoin"), Some(b.auditeur)).await;
    assert_eq!(t.statut, StatusCode::OK);
    assert_eq!(t.texte(), e["temoin"]["contenu"].as_str().unwrap());
    let liste = json(&b, "/jetons").await;
    assert_eq!(liste["total"], 1);
    assert_eq!(liste["jetons"][0]["nombre"], 1);
}

#[tokio::test]
async fn la_verification_depuis_la_base_reconnait_une_chaine_integre() {
    let b = banc().await;
    // Dossier vide : intègre.
    let v = json(&b, "/verification").await;
    assert_eq!(
        (v["integre"].as_bool(), v["nombre"].as_u64()),
        (Some(true), Some(0))
    );
    valider_n(&b, 3).await;
    let v = json(&b, "/verification").await;
    assert_eq!(v["integre"], true, "{v}");
    assert_eq!(v["nombre"], 3);
    let chaine = b.chaine().await;
    let tete: String = chaine[2].3.iter().map(|o| format!("{o:02x}")).collect();
    assert_eq!(v["tete"], tete);
    assert!(v["premiere_alteration"].is_null());
}

#[tokio::test]
async fn une_alteration_faite_par_le_proprietaire_est_localisee() {
    let b = banc().await;
    valider_n(&b, 3).await;
    let numero = b.chaine().await[1].1.clone();
    // Texte canonique de l'écriture 2 réécrit : empreinte alterée, localisée.
    let n = sous_proprietaire(
        &b,
        "UPDATE ecriture_validee SET contenu_canonique = replace(contenu_canonique, 'Écriture 2', 'Écriture X') WHERE rang = 2",
    )
    .await;
    assert_eq!(n, 1);
    let v = json(&b, "/verification").await;
    assert_eq!(v["integre"], false, "{v}");
    assert_eq!(v["premiere_alteration"], 2);
    assert_eq!(v["anomalies"][0]["code"], "empreinte_alteree");
    assert_eq!(v["anomalies"][0]["rang"], 2);
    assert_eq!(v["anomalies"][0]["numero"], numero);
    // Pas de cascade sur l'écriture 3, ni de libellé dans le rapport.
    assert!(
        v["anomalies"]
            .as_array()
            .unwrap()
            .iter()
            .all(|a| a["rang"] == 2)
    );
    assert!(!v.to_string().contains("Écriture"));
}

#[tokio::test]
async fn colonne_tete_et_troncature_sont_detectees() {
    // Colonne retouchée (le texte haché ne change pas).
    let b = banc().await;
    valider_n(&b, 2).await;
    sous_proprietaire(
        &b,
        "UPDATE ecriture_validee SET libelle = 'Autre' WHERE rang = 1",
    )
    .await;
    let v = json(&b, "/verification").await;
    assert_eq!(v["anomalies"][0]["code"], "colonne_differente");
    assert_eq!(v["anomalies"][0]["detail"], "libelle");
    assert_eq!(v["premiere_alteration"], 1);
    // Montant d'une ligne retouché.
    let b = banc().await;
    valider_n(&b, 1).await;
    sous_proprietaire(
        &b,
        "UPDATE ecriture_validee_ligne SET montant = montant + 1 WHERE position = 1",
    )
    .await;
    let v = json(&b, "/verification").await;
    assert_eq!(v["anomalies"][0]["detail"], "lignes");
    // Tête stockée retouchée.
    let b = banc().await;
    valider_n(&b, 2).await;
    sous_proprietaire(
        &b,
        "UPDATE chaine_tete SET empreinte = decode(repeat('ab', 32), 'hex')",
    )
    .await;
    let v = json(&b, "/verification").await;
    assert_eq!(v["anomalies"][0]["code"], "tete_differente");
    assert_eq!(v["integre"], false);
    // Dernière écriture supprimée (retour en arrière) : la tête stockée la connaît encore.
    let b = banc().await;
    valider_n(&b, 3).await;
    sous_proprietaire(&b, "DELETE FROM ecriture_validee_piece WHERE ecriture_id IN (SELECT id FROM ecriture_validee WHERE rang = 3)").await;
    sous_proprietaire(&b, "DELETE FROM ecriture_validee_ligne WHERE ecriture_id IN (SELECT id FROM ecriture_validee WHERE rang = 3)").await;
    sous_proprietaire(&b, "DELETE FROM ecriture_validee WHERE rang = 3").await;
    let v = json(&b, "/verification").await;
    assert_eq!(v["nombre"], 2);
    assert_eq!(v["anomalies"][0]["code"], "tete_rang_different");
}

#[tokio::test]
async fn un_jeton_qui_ne_se_verifie_pas_ou_hors_chaine_est_signale() {
    let b = banc().await;
    valider_n(&b, 2).await;
    // Témoin cohérent avec la chaîne, mais jeton illisible : signature non vérifiable.
    inserer_jeton(&b, 2).await;
    let v = json(&b, "/verification").await;
    assert_eq!(
        (v["jetons"].as_u64(), v["jetons_valides"].as_u64()),
        (Some(1), Some(0))
    );
    assert_eq!(v["anomalies"][0]["code"], "jeton_invalide");
    // Témoin d'une tête que la chaîne n'a pas (chaîne réécrite depuis le jeton).
    sous_proprietaire(
        &b,
        "UPDATE jeton_horodatage SET tete = decode(repeat('cd', 32), 'hex')",
    )
    .await;
    let v = json(&b, "/verification").await;
    let codes: Vec<_> = v["anomalies"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["code"].clone())
        .collect();
    assert_eq!(codes, ["jeton_hors_chaine"]);
}

async fn export(
    b: &Banc,
    par: api::base::IdUtilisateur,
    anciennete: Option<i64>,
) -> commun::serveur::Reponse {
    envoyer(&b.routeur, &chemin(b, "/export"), Some(par), "", anciennete).await
}

/// Écritures relues de la base et reconstituées par `compta-core`.
async fn relues(b: &Banc) -> Vec<compta_core::EcritureValidee> {
    let ids: Vec<Uuid> =
        sqlx::query_scalar("SELECT id FROM ecriture_validee WHERE dossier_id = $1 ORDER BY rang")
            .bind(b.c.dossier.uuid())
            .fetch_all(&b.env.admin)
            .await
            .unwrap();
    let mut tx = ouvrir_transaction(
        &b.env.application,
        ContexteTenant::dossier(b.c.cabinet, b.c.dossier, b.c.administrateur),
    )
    .await
    .unwrap();
    let mut rendues = Vec::new();
    for id in ids {
        let e = base::lire(&mut tx, id).await.unwrap().unwrap();
        rendues.push(reconstituer(&e, &commun::fiscal_sn(), &commun::tenue_xof()).unwrap());
    }
    tx.annuler().await.unwrap();
    rendues
}

#[tokio::test]
async fn l_export_est_celui_d_integrite_et_sigma_verifier_le_juge_integre() {
    let b = banc().await;
    valider_n(&b, 3).await;
    inserer_jeton(&b, 2).await;
    let r = export(&b, b.auditeur, None).await;
    assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
    assert_eq!(
        r.entetes["content-disposition"],
        format!(
            "attachment; filename=\"integrite-dossier-{}.txt\"",
            b.c.dossier
        )
        .as_str()
    );
    let texte = r.texte();
    assert!(texte.starts_with("sigma-books export v3\n"));
    // Identique octet pour octet à l'export de `integrite` sur les écritures reconstituées,
    // jeton compris.
    let dossier = IdDossier::parse(&b.c.dossier.to_string()).unwrap();
    let j = sqlx::query_as::<_, (i64, Vec<u8>, Vec<u8>)>(
        "SELECT nombre, tete, jeton FROM jeton_horodatage WHERE dossier_id = $1",
    )
    .bind(b.c.dossier.uuid())
    .fetch_one(&b.env.admin)
    .await
    .unwrap();
    let jeton = integrite::JetonExporte {
        nombre: u64::try_from(j.0).unwrap(),
        tete: integrite::Empreinte::depuis_octets(j.1.try_into().unwrap()),
        der: j.2,
    };
    let mut attendu = Vec::new();
    integrite::exporter(&dossier, &relues(&b).await, &[jeton], &mut attendu).unwrap();
    assert_eq!(texte.as_bytes(), attendu.as_slice());
    // `sigma-verifier` (bibliothèque) : forme, chaîne, numérotation, équilibre ; seul le jeton de
    // test, sans signature, est refusé.
    let tete =
        integrite::Empreinte::depuis_octets(b.chaine().await[2].3.clone().try_into().unwrap());
    let rapport = integrite::verifier(
        texte.as_bytes(),
        &Options {
            dossier: Some(dossier),
            tete: Some(tete),
            ..Options::default()
        },
    )
    .unwrap();
    assert_eq!(rapport.nombre, 3);
    assert!(
        rapport
            .anomalies
            .iter()
            .all(|a| format!("{a:?}").contains("Jeton")),
        "{:?}",
        rapport.anomalies
    );
    assert!(!rapport.anomalies.is_empty());
}

#[tokio::test]
async fn un_export_sans_jeton_est_integre_et_en_v4_avec_une_etiquette_fiscale() {
    let b = banc().await;
    valider_n(&b, 2).await;
    let r = export(&b, b.valideur, None).await;
    let dossier = IdDossier::parse(&b.c.dossier.to_string()).unwrap();
    let rapport = integrite::verifier(r.corps.as_slice(), &Options::default()).unwrap();
    assert!(rapport.anomalies.is_empty(), "{:?}", rapport.anomalies);
    // Une écriture à étiquette fiscale : export v4.
    let etiquetee = format!(
        r#"{{"compte":"{}","sens":"debit","montant":"500","etiquette":{{"nature":"base","sens":"collectee","regime":"TAXABLE_NORMAL","taux":"18","exigibilite":"debit"}}}}"#,
        caisse()
    );
    let (id, v) = b
        .pret_avec(&contenu(
            &journal_de_saisie(),
            &aujourd(),
            &[etiquetee, ligne(banque(), "credit", "500")],
            "",
        ))
        .await;
    assert_eq!(b.valider(id, v).await.statut, StatusCode::OK);
    let r = export(&b, b.valideur, None).await;
    assert!(r.texte().starts_with("sigma-books export v4\n"));
    let mut attendu = Vec::new();
    integrite::exporter(&dossier, &relues(&b).await, &[], &mut attendu).unwrap();
    assert_eq!(r.corps, attendu);
    let rapport = integrite::verifier(r.corps.as_slice(), &Options::default()).unwrap();
    assert!(rapport.anomalies.is_empty(), "{:?}", rapport.anomalies);
}

#[tokio::test]
async fn l_export_exige_la_reauthentification_s_inscrit_a_l_audit_et_alerte() {
    let b = banc().await;
    valider_n(&b, 1).await;
    let alertes = || async {
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM tache WHERE sujet = 'export_integrite' AND dossier_cible = $1",
        )
        .bind(b.c.dossier.uuid())
        .fetch_one(&b.env.admin)
        .await
        .unwrap()
    };
    let entrees = || async {
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM audit_dossier WHERE dossier_id = $1 AND contenu LIKE '%ExporterIntegrite%'",
        )
        .bind(b.c.dossier.uuid())
        .fetch_one(&b.env.admin)
        .await
        .unwrap()
    };
    // Session non fraîche, collaborateur : refusés, ni audit ni alerte.
    assert_eq!(
        export(&b, b.valideur, Some(600)).await.statut,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        export(&b, b.c.collaborateur, None).await.statut,
        StatusCode::FORBIDDEN
    );
    assert_eq!((alertes().await, entrees().await), (0, 0));
    // Accepté : une entrée d'audit (tête, nombre, jetons) et une alerte.
    let r = export(&b, b.valideur, None).await;
    assert_eq!(r.statut, StatusCode::OK);
    assert_eq!((alertes().await, entrees().await), (1, 1));
    let entree: String = sqlx::query_scalar(
        "SELECT contenu FROM audit_dossier WHERE dossier_id = $1 AND contenu LIKE '%ExporterIntegrite%'",
    )
    .bind(b.c.dossier.uuid())
    .fetch_one(&b.env.admin)
    .await
    .unwrap();
    assert!(
        entree.contains("export=integrite") || entree.contains("integrite"),
        "{entree}"
    );
    assert!(
        entree.contains("nombre") && entree.contains("tete"),
        "{entree}"
    );
    assert!(!entree.contains("Écriture"), "{entree}");
    // Un second export : une seconde alerte.
    export(&b, b.valideur, None).await;
    assert_eq!(alertes().await, 2);
}

#[tokio::test]
async fn aucune_route_ne_modifie_une_ecriture_ni_un_jeton() {
    for r in api::serveur::routes::integrite::ROUTES {
        // Seule l'exportation est un POST : aucune route DELETE ni de modification.
        assert!(
            r.methode == api::serveur::routes::Methode::Get || r.chemin.ends_with("/export"),
            "{}",
            r.chemin
        );
    }
}

#[tokio::test]
async fn les_requetes_simultanees_au_dela_du_pool_aboutissent_toutes() {
    // Le pool du rôle applicatif des tests a 4 connexions ; chaque requête en tient une (la
    // transaction du pipeline) et lit son instantané sur le pool des lectures : 16 requêtes
    // simultanées (état, vérification, export, jetons) ne s'interbloquent pas.
    let b = banc().await;
    valider_n(&b, 2).await;
    let mut taches = tokio::task::JoinSet::new();
    for i in 0..16 {
        let routeur = b.routeur.clone();
        let (cabinet, dossier, u) = (b.c.cabinet, b.c.dossier, b.valideur);
        taches.spawn(async move {
            let suite = ["", "/verification", "/jetons", "/export"][i % 4];
            let chemin = format!("/api/cabinets/{cabinet}/dossiers/{dossier}/integrite{suite}");
            if suite == "/export" {
                envoyer(&routeur, &chemin, Some(u), "", None).await
            } else {
                lire(&routeur, &chemin, Some(u)).await
            }
        });
    }
    let mut reussies = 0;
    while let Some(r) = taches.join_next().await {
        let r = r.unwrap();
        assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
        reussies += 1;
    }
    assert_eq!(reussies, 16);
}

/// Prolonge la chaîne jusqu'à `n` écritures par copie de la première (numéro et empreintes
/// recalculés, colonnes et lignes cohérentes) : une chaîne de plusieurs pages sans 600
/// validations par l'API.
async fn prolonger(b: &Banc, n: i64) {
    let premiere = b.chaine().await.remove(0);
    let (id1,): (Uuid,) =
        sqlx::query_as("SELECT id FROM ecriture_validee WHERE dossier_id = $1 AND rang = 1")
            .bind(b.c.dossier.uuid())
            .fetch_one(&b.env.admin)
            .await
            .unwrap();
    let (mut precedente, mut dernier) = (
        integrite::Empreinte::depuis_octets(premiere.3.clone().try_into().unwrap()),
        premiere.1.clone(),
    );
    let racine = premiere.1.trim_end_matches("000001").to_owned();
    for i in 2..=n {
        let numero = format!("{racine}{i:06}");
        let texte = premiere.2.replace(&premiere.1, &numero);
        let empreinte = integrite::chainer(&precedente, texte.as_bytes());
        let id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO ecriture_validee (cabinet_id, dossier_id, id, journal_id, exercice_id, \
                 sequence, numero, date_comptable, date_valeur_comptable, libelle, \
                 version_parametres, format, contenu_canonique, empreinte, empreinte_precedente, \
                 rang, valide_par, date_validation, valide_le) \
             SELECT cabinet_id, dossier_id, $2, journal_id, exercice_id, $3, $4, date_comptable, \
                 date_valeur_comptable, libelle, version_parametres, format, $5, $6, $7, $3, \
                 valide_par, date_validation, valide_le \
             FROM ecriture_validee WHERE id = $1",
        )
        .bind(id1)
        .bind(id)
        .bind(i32::try_from(i).unwrap())
        .bind(&numero)
        .bind(&texte)
        .bind(empreinte.octets().as_slice())
        .bind(precedente.octets().as_slice())
        .execute(&b.env.admin)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO ecriture_validee_ligne (cabinet_id, dossier_id, ecriture_id, position, \
                 compte_numero, sens, montant, devise, montant_devise, cours, tiers_id) \
             SELECT cabinet_id, dossier_id, $2, position, compte_numero, sens, montant, devise, \
                 montant_devise, cours, tiers_id \
             FROM ecriture_validee_ligne WHERE ecriture_id = $1",
        )
        .bind(id1)
        .bind(id)
        .execute(&b.env.admin)
        .await
        .unwrap();
        precedente = empreinte;
        dernier = numero;
    }
    let _ = dernier;
    sqlx::query("UPDATE chaine_tete SET rang = $2, empreinte = $3 WHERE dossier_id = $1")
        .bind(b.c.dossier.uuid())
        .bind(n)
        .bind(precedente.octets().as_slice())
        .execute(&b.env.admin)
        .await
        .unwrap();
}

#[tokio::test]
async fn une_chaine_de_plusieurs_pages_se_verifie_et_s_exporte() {
    let b = banc().await;
    valider_n(&b, 1).await;
    prolonger(&b, 620).await;
    // Plus de 500 écritures : deux pages de lecture.
    let v = json(&b, "/verification").await;
    assert_eq!(v["integre"], true, "{v}");
    assert_eq!(v["nombre"], 620);
    let r = export(&b, b.valideur, None).await;
    let rapport = integrite::verifier(r.corps.as_slice(), &Options::default()).unwrap();
    assert_eq!(rapport.nombre, 620);
    assert!(rapport.anomalies.is_empty(), "{:?}", rapport.anomalies);
    // Une altération sur la seconde page est localisée à son rang.
    sous_proprietaire(
        &b,
        "UPDATE ecriture_validee SET contenu_canonique = replace(contenu_canonique, 'Écriture 1', 'Écriture Z') WHERE rang = 550",
    )
    .await;
    let v = json(&b, "/verification").await;
    assert_eq!(v["premiere_alteration"], 550, "{v}");
}

#[tokio::test]
async fn l_export_signale_la_troncature_des_jetons_et_l_audit_cite_le_total() {
    let b = banc().await;
    valider_n(&b, 1).await;
    // Un export normal : rien de tronqué.
    let r = export(&b, b.valideur, None).await;
    assert_eq!(r.entetes["x-sigma-export-tronque"], "false");
    // Plus de jetons que ne le permet un export (10 000).
    sqlx::query(
        "INSERT INTO jeton_horodatage (cabinet_id, dossier_id, id, nombre, tete, autorite, \
                                       date_jeton, obtenu_le, declencheur, jeton) \
         SELECT $1, $2, gen_random_uuid(), 1, empreinte, 'tsa.exemple.invalide:80', now(), \
                now() + g * interval '1 second', 'quotidien', '\\x0102'::bytea \
         FROM ecriture_validee, generate_series(1, 10001) g \
         WHERE dossier_id = $2 AND rang = 1",
    )
    .bind(b.c.cabinet.uuid())
    .bind(b.c.dossier.uuid())
    .execute(&b.env.admin)
    .await
    .unwrap();
    let r = export(&b, b.valideur, None).await;
    assert_eq!(r.statut, StatusCode::OK);
    assert_eq!(r.entetes["x-sigma-export-tronque"], "true");
    assert_eq!(r.entetes["x-sigma-jetons-total"], "10001");
    assert_eq!(r.entetes["x-sigma-jetons-exportes"], "10000");
    let entree: String = sqlx::query_scalar(
        "SELECT contenu FROM audit_dossier WHERE dossier_id = $1 \
         AND contenu LIKE '%ExporterIntegrite%' ORDER BY rang DESC LIMIT 1",
    )
    .bind(b.c.dossier.uuid())
    .fetch_one(&b.env.admin)
    .await
    .unwrap();
    assert!(
        entree.contains("10001") && entree.contains("10000"),
        "{entree}"
    );
    // L'état ne charge que le compte et le dernier jeton.
    let e = json(&b, "").await;
    assert_eq!(e["jetons"], 10001);
}
