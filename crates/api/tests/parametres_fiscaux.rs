//! Paramètres fiscaux par pays (ADR 0030) : version choisie par la date comptable, enregistrée à
//! la validation (colonne, contenu haché, lecture), contre-passation, validité dans le temps,
//! pays sans version, paquet d'un autre pays, isolation entre dossiers de pays différents.
#![cfg(feature = "tests-postgres")]
// Fichier de test : les `expect`/`unwrap` y sont voulus.
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::too_many_lines)]

mod commun;

use axum::http::StatusCode;
use commun::banc::{Banc, banc, banc_avec_registre, banque, caisse, code, contenu};
use commun::brouillard::{aujourd_hui, journal_de_saisie, ligne, texte_date};
use commun::paquets_de_test::registre_cm;
use commun::serveur::{Reponse, envoyer, lire};
use sqlx::Row;
use time::Date;
use uuid::Uuid;

async fn tenir_en_cm(b: &Banc) {
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
        "UPDATE dossier SET pays = 'CM', devise_tenue = 'XAF', decimales_tenue = 0, \
                            fuseau = 'Africa/Douala' WHERE cabinet_id = $1 AND id = $2",
    )
    .bind(b.c.cabinet.uuid())
    .bind(b.c.dossier.uuid())
    .execute(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();
}

fn etiquetee(taux: &str, regime: &str) -> String {
    format!(
        r#"{{"compte":"{}","sens":"debit","montant":"1000","etiquette":{{"nature":"base","sens":"collectee","regime":"{regime}","taux":"{taux}","exigibilite":"debit"}}}}"#,
        caisse()
    )
}

fn exoneree(regime: &str, motif: &str) -> String {
    format!(
        r#"{{"compte":"{}","sens":"debit","montant":"1000","etiquette":{{"nature":"base","sens":"collectee","regime":"{regime}","motif":"{motif}"}}}}"#,
        caisse()
    )
}

fn corps(date: &str, premiere: String) -> String {
    contenu(
        &journal_de_saisie(),
        date,
        &[premiere, ligne(banque(), "credit", "1000")],
        "",
    )
}

async fn creer(b: &Banc, corps: &str) -> Reponse {
    envoyer(&b.routeur, &b.base(), Some(b.c.collaborateur), corps, None).await
}

fn jour(annee: i32, mois: u8, j: u8) -> Date {
    Date::from_calendar_date(annee, time::Month::try_from(mois).unwrap(), j).unwrap()
}

async fn version_validee(b: &Banc, rang: i64) -> Option<String> {
    sqlx::query(
        "SELECT version_parametres FROM ecriture_validee WHERE dossier_id = $1 AND rang = $2",
    )
    .bind(b.c.dossier.uuid())
    .bind(rang)
    .fetch_one(&b.env.admin)
    .await
    .unwrap()
    .get(0)
}

#[tokio::test]
async fn la_version_est_enregistree_a_la_validation_et_relue() {
    let b = banc().await;
    let annee = aujourd_hui().year();
    let date = texte_date(jour(annee, 3, 15));
    let r = creer(&b, &corps(&date, etiquetee("18", "TAXABLE_NORMAL"))).await;
    assert_eq!(r.statut, StatusCode::CREATED, "{}", r.texte());
    // Le brouillon porte la version appliquée à sa saisie.
    assert_eq!(r.json()["version_parametres"], "sn-2025.1");
    let id: Uuid = r.json()["id"].as_str().unwrap().parse().unwrap();
    let (_, version) = b.lier(id, 1).await;
    let v = b.valider(id, version).await;
    assert_eq!(v.statut, StatusCode::OK, "{}", v.texte());
    // Colonne, contenu haché et lecture disent la même version.
    assert_eq!(version_validee(&b, 1).await.as_deref(), Some("sn-2025.1"));
    let chaine = b.chaine().await;
    assert!(
        chaine[0].2.contains("\nparametres=sn-2025.1\n"),
        "{}",
        chaine[0].2
    );
    assert!(b.chaine_valide().await);
    let lu = lire(
        &b.routeur,
        &format!("{}/{id}", b.ecritures()),
        Some(b.auditeur),
    )
    .await;
    assert_eq!(lu.json()["version_parametres"], "sn-2025.1");
    // Une écriture sans étiquette enregistre aussi la version applicable (spec §5.4).
    let (id2, v2, _) = b.pret("Sans étiquette").await;
    assert_eq!(b.valider(id2, v2).await.statut, StatusCode::OK);
    assert_eq!(version_validee(&b, 2).await.as_deref(), Some("sn-2025.1"));
    // Contre-passation : la version de l'origine est reprise, l'étiquette copiée telle quelle.
    let cp = envoyer(
        &b.routeur,
        &format!("{}/{id}/contre-passation", b.ecritures()),
        Some(b.valideur),
        &format!(r#"{{"date":"{date}","libelle":"Contre-passation","motif":"Erreur"}}"#),
        None,
    )
    .await;
    assert_eq!(cp.statut, StatusCode::CREATED, "{}", cp.texte());
    assert_eq!(version_validee(&b, 3).await.as_deref(), Some("sn-2025.1"));
    assert!(b.chaine_valide().await);
}

#[tokio::test]
async fn une_etiquette_echue_est_refusee_apres_sa_fin_mais_pas_avant() {
    let b = banc().await;
    let annee = aujourd_hui().year();
    // EXO_AVEC_DEDUCTION-e finit le 2026-12-31 : valide en 2026, refusé à partir de 2027 ; le
    // test suit l'année courante du banc (exercice de l'année en cours).
    let date = texte_date(jour(annee, 6, 15));
    let r = creer(
        &b,
        &corps(
            &date,
            exoneree("EXO_AVEC_DEDUCTION", "EXO_AVEC_DEDUCTION-e"),
        ),
    )
    .await;
    if annee <= 2026 {
        assert_eq!(r.statut, StatusCode::CREATED, "{}", r.texte());
    } else {
        assert_eq!(r.statut, StatusCode::UNPROCESSABLE_ENTITY, "{}", r.texte());
        assert_eq!(code(&r), "etiquette_motif_hors_regime");
    }
    // Un motif sans fin reste admis à toute date.
    let r = creer(
        &b,
        &corps(
            &date,
            exoneree("EXO_AVEC_DEDUCTION", "EXO_AVEC_DEDUCTION-a"),
        ),
    )
    .await;
    assert_eq!(r.statut, StatusCode::CREATED, "{}", r.texte());
    // Les anciens codes propres au Sénégal sont retirés.
    let r = creer(&b, &corps(&date, etiquetee("18", "IMP18"))).await;
    assert_eq!(r.statut, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(code(&r), "etiquette_regime_inconnu");
}

#[tokio::test]
async fn un_autre_pays_applique_son_paquet_sans_toucher_au_socle() {
    let annee = aujourd_hui().year();
    let registre = registre_cm(&[
        ("2024.1", jour(annee - 1, 1, 1), "15"),
        ("2026.1", jour(annee, 7, 1), "16"),
    ]);
    let b = banc_avec_registre(registre).await;
    tenir_en_cm(&b).await;
    // Avant le 1er juillet : version 2024.1, taux 15 ; le taux du Sénégal (18) est refusé.
    let mars = texte_date(jour(annee, 3, 15));
    let r = creer(&b, &corps(&mars, etiquetee("18", "TAXABLE_NORMAL"))).await;
    assert_eq!(r.statut, StatusCode::UNPROCESSABLE_ENTITY, "{}", r.texte());
    assert_eq!(code(&r), "etiquette_taux_non_admis");
    let r = creer(&b, &corps(&mars, etiquetee("15", "TAXABLE_NORMAL"))).await;
    assert_eq!(r.statut, StatusCode::CREATED, "{}", r.texte());
    assert_eq!(r.json()["version_parametres"], "cm-2024.1");
    let id: Uuid = r.json()["id"].as_str().unwrap().parse().unwrap();
    let (_, v) = b.lier(id, 1).await;
    assert_eq!(b.valider(id, v).await.statut, StatusCode::OK);
    assert_eq!(version_validee(&b, 1).await.as_deref(), Some("cm-2024.1"));
    // Après : version 2026.1, taux 16.
    let aout = texte_date(jour(annee, 8, 15));
    let r = creer(&b, &corps(&aout, etiquetee("15", "TAXABLE_NORMAL"))).await;
    assert_eq!(code(&r), "etiquette_taux_non_admis");
    let r = creer(&b, &corps(&aout, etiquetee("16", "TAXABLE_NORMAL"))).await;
    assert_eq!(r.statut, StatusCode::CREATED, "{}", r.texte());
    assert_eq!(r.json()["version_parametres"], "cm-2026.1");
    let id: Uuid = r.json()["id"].as_str().unwrap().parse().unwrap();
    let (_, v) = b.lier(id, 1).await;
    assert_eq!(b.valider(id, v).await.statut, StatusCode::OK);
    assert_eq!(version_validee(&b, 2).await.as_deref(), Some("cm-2026.1"));
    // Validité dans le temps : le motif x1 du paquet factice finit le 2025-12-31 (refusé en
    // cours d'année), le motif x2 n'a pas de fin.
    let ancien = exoneree("EXO_SANS_DEDUCTION", "EXO_SANS_DEDUCTION-x1");
    let r = creer(&b, &corps(&aout, ancien)).await;
    assert_eq!(r.statut, StatusCode::UNPROCESSABLE_ENTITY, "{}", r.texte());
    assert_eq!(code(&r), "etiquette_motif_hors_regime");
    let r = creer(
        &b,
        &corps(
            &aout,
            exoneree("EXO_SANS_DEDUCTION", "EXO_SANS_DEDUCTION-x2"),
        ),
    )
    .await;
    assert_eq!(r.statut, StatusCode::CREATED, "{}", r.texte());
    // Un régime que le pays n'offre pas est inconnu.
    let r = creer(&b, &corps(&aout, etiquetee("5", "TAXABLE_REDUIT"))).await;
    assert_eq!(code(&r), "etiquette_regime_inconnu");
    // La relecture suit la version enregistrée de chaque écriture.
    let r = lire(&b.routeur, &b.ecritures(), Some(b.auditeur)).await;
    assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
    assert!(b.chaine_valide().await);
}

#[tokio::test]
async fn aucune_version_a_la_date_refuse_l_etiquette_seulement() {
    let annee = aujourd_hui().year();
    // Une seule version, qui prend effet en juillet : mars n'en a aucune.
    let registre = registre_cm(&[("2024.1", jour(annee, 7, 1), "15")]);
    let b = banc_avec_registre(registre).await;
    tenir_en_cm(&b).await;
    let mars = texte_date(jour(annee, 3, 15));
    let r = creer(&b, &corps(&mars, etiquetee("15", "TAXABLE_NORMAL"))).await;
    assert_eq!(r.statut, StatusCode::UNPROCESSABLE_ENTITY, "{}", r.texte());
    assert_eq!(code(&r), "parametres_fiscaux_absents");
    assert_eq!(r.json()["ligne"], 1);
    // Sans étiquette, l'écriture se valide, sans version (contenu haché inchangé).
    let r = creer(&b, &corps(&mars, ligne(caisse(), "debit", "1000"))).await;
    assert_eq!(r.statut, StatusCode::CREATED, "{}", r.texte());
    assert!(r.json().get("version_parametres").is_none());
    let id: Uuid = r.json()["id"].as_str().unwrap().parse().unwrap();
    let (_, v) = b.lier(id, 1).await;
    let v = b.valider(id, v).await;
    assert_eq!(v.statut, StatusCode::OK, "{}", v.texte());
    assert_eq!(version_validee(&b, 1).await, None);
    assert!(!b.chaine().await[0].2.contains("parametres="));
}

#[tokio::test]
async fn un_pays_sans_paquet_n_a_pas_de_version() {
    let b = banc().await;
    // Pays du pack de l'ADR 0027 sans version de paramètres : GA (le registre officiel ne
    // contient que le Sénégal).
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
        "UPDATE dossier SET pays = 'GA', devise_tenue = 'XAF', decimales_tenue = 0, \
                            fuseau = 'Africa/Libreville' WHERE cabinet_id = $1 AND id = $2",
    )
    .bind(b.c.cabinet.uuid())
    .bind(b.c.dossier.uuid())
    .execute(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();
    let date = texte_date(aujourd_hui());
    let r = creer(&b, &corps(&date, etiquetee("18", "TAXABLE_NORMAL"))).await;
    assert_eq!(code(&r), "parametres_fiscaux_absents");
    let r = creer(&b, &corps(&date, ligne(caisse(), "debit", "1000"))).await;
    assert_eq!(r.statut, StatusCode::CREATED, "{}", r.texte());
}

#[tokio::test]
async fn deux_dossiers_de_pays_differents_ne_se_melangent_pas() {
    let annee = aujourd_hui().year();
    let registre = registre_cm(&[("2024.1", jour(annee - 1, 1, 1), "15")]);
    let b = banc_avec_registre(registre).await;
    // Le dossier du banc reste au Sénégal ; l'autre dossier du cabinet passe au Cameroun (factice).
    let mut tx = b.env.admin.begin().await.unwrap();
    sqlx::query(
        "SELECT set_config('sigma.cabinet', $1, true), set_config('sigma.dossier', $2, true)",
    )
    .bind(b.c.cabinet.to_string())
    .bind(b.c.autre_dossier.to_string())
    .execute(&mut *tx)
    .await
    .unwrap();
    sqlx::query("UPDATE dossier SET pays = 'CM' WHERE cabinet_id = $1 AND id = $2")
        .bind(b.c.cabinet.uuid())
        .bind(b.c.autre_dossier.uuid())
        .execute(&mut *tx)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    let date = texte_date(jour(annee, 3, 15));
    // Dossier du Sénégal : taux 18 admis, 15 refusé.
    let r = creer(&b, &corps(&date, etiquetee("18", "TAXABLE_NORMAL"))).await;
    assert_eq!(r.statut, StatusCode::CREATED, "{}", r.texte());
    assert_eq!(r.json()["version_parametres"], "sn-2025.1");
    let r = creer(&b, &corps(&date, etiquetee("15", "TAXABLE_NORMAL"))).await;
    assert_eq!(code(&r), "etiquette_taux_non_admis");
}
