//! Paramètres fiscaux par pays (ADR 0030) : version choisie par la date comptable, enregistrée à
//! la validation (colonne, contenu haché, lecture), contre-passation, validité dans le temps,
//! pays sans version, paquet d'un autre pays, isolation entre dossiers de pays différents.
#![cfg(feature = "tests-postgres")]
// Fichier de test : les `expect`/`unwrap` y sont voulus.
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::too_many_lines)]

mod commun;

use axum::http::StatusCode;
use commun::banc::{Banc, banc, banc_avec_registre, banc_repris, banque, caisse, code, contenu};
use commun::brouillard::{aujourd_hui, journal_de_saisie, ligne, texte_date};
use commun::paquets_de_test::{registre_cm, registre_cm_evolutif};
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

/// Bascule du paquet du Sénégal (loi n° 2025-17) : une écriture datée la veille de la date
/// d'effet de 2025.2 (2025-10-02, à confirmer par le fiscaliste) se valide et enregistre
/// `sn-2025.1` ; une écriture datée de ce jour enregistre `sn-2025.2` (colonne et contenu haché).
#[tokio::test]
async fn une_ecriture_suit_la_version_du_paquet_a_sa_date() {
    let b = banc_repris(jour(2025, 12, 1)).await;
    for (rang, (date, attendue)) in [("2025-10-01", "sn-2025.1"), ("2025-10-02", "sn-2025.2")]
        .into_iter()
        .enumerate()
    {
        let r = creer(&b, &corps(date, etiquetee("18", "TAXABLE_NORMAL"))).await;
        assert_eq!(r.statut, StatusCode::CREATED, "{}", r.texte());
        assert_eq!(r.json()["version_parametres"], attendue);
        let id: Uuid = r.json()["id"].as_str().unwrap().parse().unwrap();
        let (_, version) = b.lier(id, 1).await;
        let v = b.valider(id, version).await;
        assert_eq!(v.statut, StatusCode::OK, "{date} : {}", v.texte());
        let rang = i64::try_from(rang).unwrap() + 1;
        assert_eq!(version_validee(&b, rang).await.as_deref(), Some(attendue));
        let chaine = b.chaine().await;
        let ligne = usize::try_from(rang - 1).unwrap();
        assert!(
            chaine[ligne]
                .2
                .contains(&format!("\nparametres={attendue}\n")),
            "{}",
            chaine[ligne].2
        );
    }
    assert!(b.chaine_valide().await);
}

#[tokio::test]
async fn la_version_est_enregistree_a_la_validation_et_relue() {
    let b = banc().await;
    let annee = aujourd_hui().year();
    let date = texte_date(jour(annee, 3, 15));
    let r = creer(&b, &corps(&date, etiquetee("18", "TAXABLE_NORMAL"))).await;
    assert_eq!(r.statut, StatusCode::CREATED, "{}", r.texte());
    // Le brouillon porte la version appliquée à sa saisie.
    assert_eq!(r.json()["version_parametres"], "sn-2025.2");
    let id: Uuid = r.json()["id"].as_str().unwrap().parse().unwrap();
    let (_, version) = b.lier(id, 1).await;
    let v = b.valider(id, version).await;
    assert_eq!(v.statut, StatusCode::OK, "{}", v.texte());
    // Colonne, contenu haché et lecture disent la même version.
    assert_eq!(version_validee(&b, 1).await.as_deref(), Some("sn-2025.2"));
    let chaine = b.chaine().await;
    assert!(
        chaine[0].2.contains("\nparametres=sn-2025.2\n"),
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
    assert_eq!(lu.json()["version_parametres"], "sn-2025.2");
    // Une écriture sans étiquette enregistre aussi la version applicable (spec §5.4).
    let (id2, v2, _) = b.pret("Sans étiquette").await;
    assert_eq!(b.valider(id2, v2).await.statut, StatusCode::OK);
    assert_eq!(version_validee(&b, 2).await.as_deref(), Some("sn-2025.2"));
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
    assert_eq!(version_validee(&b, 3).await.as_deref(), Some("sn-2025.2"));
    assert!(b.chaine_valide().await);
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

async fn passer_en_cm(b: &Banc, cabinet: api::base::IdCabinet, dossier: api::base::IdDossier) {
    let mut tx = b.env.admin.begin().await.unwrap();
    sqlx::query(
        "SELECT set_config('sigma.cabinet', $1, true), set_config('sigma.dossier', $2, true)",
    )
    .bind(cabinet.to_string())
    .bind(dossier.to_string())
    .execute(&mut *tx)
    .await
    .unwrap();
    sqlx::query("UPDATE dossier SET pays = 'CM' WHERE cabinet_id = $1 AND id = $2")
        .bind(cabinet.uuid())
        .bind(dossier.uuid())
        .execute(&mut *tx)
        .await
        .unwrap();
    tx.commit().await.unwrap();
}

async fn creer_sur(
    b: &Banc,
    cabinet: api::base::IdCabinet,
    dossier: api::base::IdDossier,
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
async fn les_dossiers_de_pays_differents_et_les_cabinets_ne_se_melangent_pas() {
    let annee = aujourd_hui().year();
    let b = banc_avec_registre(registre_cm(&[("2024.1", jour(annee - 1, 1, 1), "15")])).await;
    let date = texte_date(jour(annee, 3, 15));
    // 1. Dossier du banc (Sénégal) : taux 18 admis, 15 refusé.
    let r = creer(&b, &corps(&date, etiquetee("18", "TAXABLE_NORMAL"))).await;
    assert_eq!(r.statut, StatusCode::CREATED, "{}", r.texte());
    assert_eq!(r.json()["version_parametres"], "sn-2025.2");
    let r = creer(&b, &corps(&date, etiquetee("15", "TAXABLE_NORMAL"))).await;
    assert_eq!(code(&r), "etiquette_taux_non_admis");
    // 2. Autre dossier du même cabinet, tenu au Cameroun (factice) : taux 15 admis, 18 refusé.
    commun::referentiel::doter(
        &b.env.application,
        b.c.cabinet,
        b.c.autre_dossier,
        b.c.administrateur,
    )
    .await;
    passer_en_cm(&b, b.c.cabinet, b.c.autre_dossier).await;
    let r = creer_sur(
        &b,
        b.c.cabinet,
        b.c.autre_dossier,
        b.c.administrateur,
        &corps(&date, etiquetee("15", "TAXABLE_NORMAL")),
    )
    .await;
    assert_eq!(r.statut, StatusCode::CREATED, "{}", r.texte());
    assert_eq!(r.json()["version_parametres"], "cm-2024.1");
    let r = creer_sur(
        &b,
        b.c.cabinet,
        b.c.autre_dossier,
        b.c.administrateur,
        &corps(&date, etiquetee("18", "TAXABLE_NORMAL")),
    )
    .await;
    assert_eq!(code(&r), "etiquette_taux_non_admis");
    // 3. Cabinet tiers, dossier au Cameroun : même résultat, et le cabinet du banc n'y accède pas.
    let tiers = commun::creer_cabinet(&b.env.admin, "Tiers").await;
    commun::referentiel::doter(
        &b.env.application,
        tiers.cabinet,
        tiers.dossier,
        tiers.administrateur,
    )
    .await;
    passer_en_cm(&b, tiers.cabinet, tiers.dossier).await;
    let r = creer_sur(
        &b,
        tiers.cabinet,
        tiers.dossier,
        tiers.collaborateur,
        &corps(&date, etiquetee("15", "TAXABLE_NORMAL")),
    )
    .await;
    assert_eq!(r.statut, StatusCode::CREATED, "{}", r.texte());
    assert_eq!(r.json()["version_parametres"], "cm-2024.1");
    let r = creer_sur(
        &b,
        tiers.cabinet,
        tiers.dossier,
        b.c.collaborateur,
        &corps(&date, etiquetee("15", "TAXABLE_NORMAL")),
    )
    .await;
    assert!(
        r.statut == StatusCode::NOT_FOUND || r.statut == StatusCode::FORBIDDEN,
        "{} {}",
        r.statut,
        r.texte()
    );
    // 4. Le dossier du banc n'a pas bougé : toujours le Sénégal.
    let r = creer(&b, &corps(&date, etiquetee("18", "TAXABLE_NORMAL"))).await;
    assert_eq!(r.json()["version_parametres"], "sn-2025.2");
}

// --- Validité dans le temps, recontrôle à la validation, contre-passation ----------------------

/// Lot de validation (un brouillon), session fraîche par défaut.
async fn lot(b: &Banc, id: Uuid, version: i32) -> Reponse {
    envoyer(
        &b.routeur,
        &format!("{}/validation-par-lot", b.base()),
        Some(b.valideur),
        &format!(r#"{{"ecritures":[{{"ecriture":"{id}","version":{version}}}]}}"#),
        None,
    )
    .await
}

async fn brouillon_en(b: &Banc, date: &str, etiquette: String) -> (Uuid, i32) {
    let r = creer(b, &corps(date, etiquette)).await;
    assert_eq!(r.statut, StatusCode::CREATED, "{}", r.texte());
    let id: Uuid = r.json()["id"].as_str().unwrap().parse().unwrap();
    let (_, version) = b.lier(id, 1).await;
    (id, version)
}

#[tokio::test]
async fn un_motif_echu_est_refuse_a_la_saisie_apres_sa_fin() {
    let annee = aujourd_hui().year();
    let b = banc_avec_registre(registre_cm_evolutif(annee)).await;
    tenir_en_cm(&b).await;
    let mars = texte_date(jour(annee, 3, 15));
    let aout = texte_date(jour(annee, 8, 15));
    // Le motif x1 finit le 30 juin : valide en mars, refusé en août ; x2 n'a pas de fin.
    let x1 = || exoneree("EXO_SANS_DEDUCTION", "EXO_SANS_DEDUCTION-x1");
    assert_eq!(
        creer(&b, &corps(&mars, x1())).await.statut,
        StatusCode::CREATED
    );
    let r = creer(&b, &corps(&aout, x1())).await;
    assert_eq!(r.statut, StatusCode::UNPROCESSABLE_ENTITY, "{}", r.texte());
    assert_eq!(code(&r), "etiquette_motif_hors_regime");
    let x2 = exoneree("EXO_SANS_DEDUCTION", "EXO_SANS_DEDUCTION-x2");
    assert_eq!(
        creer(&b, &corps(&aout, x2)).await.statut,
        StatusCode::CREATED
    );
    // Un régime disparu dans la version d'août est inconnu à cette date.
    let r = creer(&b, &corps(&aout, etiquetee("16", "TAXABLE_NORMAL"))).await;
    assert_eq!(code(&r), "etiquette_regime_inconnu");
}

#[tokio::test]
async fn la_validation_recontrole_les_etiquettes_contre_la_version_de_la_date() {
    let annee = aujourd_hui().year();
    let b = banc_avec_registre(registre_cm_evolutif(annee)).await;
    tenir_en_cm(&b).await;
    let (id, v) = brouillon_en(
        &b,
        &texte_date(jour(annee, 3, 15)),
        etiquetee("15", "TAXABLE_NORMAL"),
    )
    .await;
    // La date passe en août, où le régime n'existe plus : la version applicable le refuse.
    b.sql(&format!(
        "UPDATE brouillon SET date_comptable = '{annee}-08-15' WHERE dossier_id = $1"
    ))
    .await;
    let r = b.valider(id, v).await;
    assert_eq!(r.statut, StatusCode::UNPROCESSABLE_ENTITY, "{}", r.texte());
    assert_eq!(code(&r), "etiquette_invalide");
    assert_eq!(r.json()["ligne"], 1);
    assert_eq!(b.compter("ecriture_validee").await, 0);
    assert_eq!(b.compter("brouillon").await, 1);
    // Par lot : tout ou rien, rien n'est écrit.
    let r = lot(&b, id, v).await;
    assert_ne!(r.statut, StatusCode::OK, "{}", r.texte());
    assert!(r.texte().contains("etiquette_invalide"), "{}", r.texte());
    assert_eq!(b.compter("ecriture_validee").await, 0);
}

#[tokio::test]
async fn la_validation_refuse_une_etiquette_sans_version_a_la_date() {
    let b = banc().await;
    let annee = aujourd_hui().year();
    let (id, v) = brouillon_en(
        &b,
        &texte_date(jour(annee, 3, 15)),
        etiquetee("18", "TAXABLE_NORMAL"),
    )
    .await;
    // Avant la première version du Sénégal (2025-01-01) : aucune version ne s'applique.
    b.sql("UPDATE brouillon SET date_comptable = '2024-06-15' WHERE dossier_id = $1")
        .await;
    let r = b.valider(id, v).await;
    assert_eq!(r.statut, StatusCode::UNPROCESSABLE_ENTITY, "{}", r.texte());
    assert_eq!(code(&r), "validation_parametres_fiscaux_absents");
    assert_eq!(r.json()["ligne"], 1);
    assert_eq!(b.compter("ecriture_validee").await, 0);
    let r = lot(&b, id, v).await;
    assert_ne!(r.statut, StatusCode::OK, "{}", r.texte());
    assert!(
        r.texte().contains("validation_parametres_fiscaux_absents"),
        "{}",
        r.texte()
    );
    assert_eq!(b.compter("ecriture_validee").await, 0);
}

#[tokio::test]
async fn la_contre_passation_copie_la_version_meme_si_le_regime_a_disparu() {
    let annee = aujourd_hui().year();
    let b = banc_avec_registre(registre_cm_evolutif(annee)).await;
    tenir_en_cm(&b).await;
    let mars = texte_date(jour(annee, 3, 15));
    let (id, v) = brouillon_en(&b, &mars, etiquetee("15", "TAXABLE_NORMAL")).await;
    assert_eq!(b.valider(id, v).await.statut, StatusCode::OK);
    // Contre-passation en août : la version d'août n'a plus le régime, la version de l'origine est
    // copiée avec l'étiquette, sans recontrôle.
    let aout = texte_date(jour(annee, 8, 15));
    let cp = envoyer(
        &b.routeur,
        &format!("{}/{id}/contre-passation", b.ecritures()),
        Some(b.valideur),
        &format!(r#"{{"date":"{aout}","libelle":"Contre-passation","motif":"Erreur"}}"#),
        None,
    )
    .await;
    assert_eq!(cp.statut, StatusCode::CREATED, "{}", cp.texte());
    assert_eq!(version_validee(&b, 2).await.as_deref(), Some("cm-2024.1"));
    assert!(b.chaine_valide().await);
    // La relecture (éditions, liste) suit la version enregistrée.
    let r = lire(&b.routeur, &b.ecritures(), Some(b.auditeur)).await;
    assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
}

#[tokio::test]
async fn le_pays_ne_change_plus_des_qu_une_version_propre_existe() {
    let b = banc().await;
    let chemin = format!(
        "/api/cabinets/{}/dossiers/{}/contexte",
        b.c.cabinet, b.c.dossier
    );
    let modifier =
        |corps: &'static str| envoyer(&b.routeur, &chemin, Some(b.c.administrateur), corps, None);
    sqlx::query(
        "INSERT INTO parametres_fiscaux_dossier \
             (cabinet_id, dossier_id, id, numero, pays, version, date_effet, contenu, empreinte, motif, cree_par) \
         VALUES ($1, $2, gen_random_uuid(), 1, 'SN', '2025.1', '2026-01-01', 'contenu', decode(repeat('00', 32), 'hex'), 'motif', $3)",
    )
    .bind(b.c.cabinet.uuid())
    .bind(b.c.dossier.uuid())
    .bind(b.c.administrateur.uuid())
    .execute(&b.env.admin)
    .await
    .unwrap();
    // Changer le pays : refusé (409) ; la langue ou la forme juridique restent modifiables.
    let r = modifier(r#"{"pays":"CM","devise":"XAF","fuseau":"Africa/Douala"}"#).await;
    assert_eq!(r.statut, StatusCode::CONFLICT, "{}", r.texte());
    assert_eq!(code(&r), "dossier_parametres_fiscaux_propres");
    let r = modifier(r#"{"forme_juridique":"SARL"}"#).await;
    assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
    // Seconde barrière : le déclencheur refuse aussi un UPDATE direct.
    let mut tx = b.env.admin.begin().await.unwrap();
    sqlx::query(
        "SELECT set_config('sigma.cabinet', $1, true), set_config('sigma.dossier', $2, true)",
    )
    .bind(b.c.cabinet.to_string())
    .bind(b.c.dossier.to_string())
    .execute(&mut *tx)
    .await
    .unwrap();
    let direct = sqlx::query("UPDATE dossier SET pays = 'CM' WHERE cabinet_id = $1 AND id = $2")
        .bind(b.c.cabinet.uuid())
        .bind(b.c.dossier.uuid())
        .execute(&mut *tx)
        .await;
    assert!(direct.is_err());
}

#[tokio::test]
async fn le_pays_ne_change_plus_des_qu_une_piece_existe() {
    let b = banc().await;
    let corps = r#"{"pays":"CM","devise":"XAF","fuseau":"Africa/Douala"}"#;
    let chemin =
        |d: api::base::IdDossier| format!("/api/cabinets/{}/dossiers/{d}/contexte", b.c.cabinet);
    // Une pièce déposée dans le dossier : sa rétention est celle du pays au dépôt.
    commun::pieces::doter(&b.env.admin, b.c.cabinet, b.c.dossier, b.c.administrateur).await;
    let r = envoyer(
        &b.routeur,
        &chemin(b.c.dossier),
        Some(b.c.administrateur),
        corps,
        None,
    )
    .await;
    assert_eq!(r.statut, StatusCode::CONFLICT, "{}", r.texte());
    assert_eq!(code(&r), "dossier_pieces_presentes");
    // Seconde barrière : le déclencheur refuse aussi un UPDATE direct.
    let mut tx = b.env.admin.begin().await.unwrap();
    sqlx::query(
        "SELECT set_config('sigma.cabinet', $1, true), set_config('sigma.dossier', $2, true)",
    )
    .bind(b.c.cabinet.to_string())
    .bind(b.c.dossier.to_string())
    .execute(&mut *tx)
    .await
    .unwrap();
    let direct = sqlx::query("UPDATE dossier SET pays = 'CM' WHERE cabinet_id = $1 AND id = $2")
        .bind(b.c.cabinet.uuid())
        .bind(b.c.dossier.uuid())
        .execute(&mut *tx)
        .await;
    assert!(direct.is_err());
    // Un dossier sans pièce peut encore changer de pays.
    let r = envoyer(
        &b.routeur,
        &chemin(b.c.autre_dossier),
        Some(b.c.administrateur),
        corps,
        None,
    )
    .await;
    assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
}
