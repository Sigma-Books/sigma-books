//! Avertissements de validation (règles alignées sur Odoo et Tryton, ADR 0029) : l'échéance de
//! clôture et le compte de trésorerie absent n'empêchent plus la validation ; l'avertissement est
//! renvoyé (validation, lot, contre-passation), tracé dans l'audit par ses **codes** seulement et
//! n'entre jamais dans le contenu haché. Verrouillage et clôture par un humain restent bloquants,
//! la contre-passation est libre de sa date (avant l'origine, autre exercice) sauf période ou
//! exercice clôturés.
#![cfg(feature = "tests-postgres")]
// Fichier de test : les `expect`/`unwrap` y sont voulus.
#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::too_many_lines,
    clippy::many_single_char_names
)]

mod commun;

use axum::http::StatusCode;
use commun::banc::{Banc, aujourd, banc, banc_avec, code, contenu, equilibre};
use commun::brouillard::journal_de_saisie;
use commun::serveur::{HorlogeTest, Reponse, envoyer, lire};
use uuid::Uuid;

const CODE_ECHEANCE: &str = "echeance_de_cloture_depassee";

async fn audits(b: &Banc) -> Vec<String> {
    sqlx::query_scalar("SELECT contenu FROM audit_dossier WHERE dossier_id = $1 ORDER BY rang")
        .bind(b.c.dossier.uuid())
        .fetch_all(&b.env.admin)
        .await
        .unwrap()
}

/// Le contenu haché ne porte aucune trace d'avertissement.
async fn canonique_sans_avertissement(b: &Banc) {
    for (_, numero, texte, _, _) in b.chaine().await {
        assert!(
            !texte.contains("avertissement") && !texte.contains("echeance"),
            "{numero} : {texte}"
        );
    }
    assert!(b.chaine_valide().await);
}

fn codes(r: &serde_json::Value) -> Vec<String> {
    r["avertissements"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["code"].as_str().unwrap().to_owned())
        .collect()
}

/// Validation par lot (ré-authentification exigée) : `anciennete` est l'âge de la dernière
/// vérification du facteur, mesuré à l'horloge du système ; après `horloge.avancer`, un âge négatif
/// de la même durée la maintient fraîche pour l'horloge du serveur.
async fn lot(b: &Banc, elements: &[(Uuid, i32)], anciennete: i64) -> Reponse {
    let liste: Vec<String> = elements
        .iter()
        .map(|(id, v)| format!(r#"{{"ecriture":"{id}","version":{v}}}"#))
        .collect();
    envoyer(
        &b.routeur,
        &format!("{}/validation-par-lot", b.base()),
        Some(b.valideur),
        &format!(r#"{{"ecritures":[{}]}}"#, liste.join(",")),
        Some(anciennete),
    )
    .await
}

async fn contre_passer(b: &Banc, id: &str, date: &str) -> Reponse {
    envoyer(
        &b.routeur,
        &format!("{}/{id}/contre-passation", b.ecritures()),
        Some(b.valideur),
        &format!(r#"{{"date":"{date}","libelle":"Contre-passation"}}"#),
        None,
    )
    .await
}

async fn debut_des_exercices(b: &Banc) -> Vec<String> {
    sqlx::query_scalar("SELECT debut::text FROM exercice WHERE dossier_id = $1 ORDER BY debut")
        .bind(b.c.dossier.uuid())
        .fetch_all(&b.env.admin)
        .await
        .unwrap()
}

/// Passe en `statut` les périodes qui débutent au plus tard à `jusqu_a`.
async fn periodes(b: &Banc, statut: &str, jusqu_a: &str) {
    sqlx::query(
        "UPDATE periode SET statut = $2, statut_par = $3, statut_le = now() \
         WHERE dossier_id = $1 AND debut <= $4::date AND statut <> 'cloturee'",
    )
    .bind(b.c.dossier.uuid())
    .bind(statut)
    .bind(b.c.administrateur.uuid())
    .bind(jusqu_a)
    .execute(&b.env.admin)
    .await
    .unwrap();
}

// --- Audit et contenu haché --------------------------------------------------------------------

#[tokio::test]
async fn la_validation_trace_ses_avertissements_par_leurs_codes() {
    let horloge = HorlogeTest::default();
    let b = banc_avec(Some(&horloge)).await;
    // Dans le délai : ni avertissement, ni donnée d'audit.
    let (id, v, _) = b.pret("Dans le délai").await;
    let r = b.valider(id, v).await;
    assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
    assert_eq!(codes(&r.json()), Vec::<String>::new());
    assert!(!audits(&b).await.last().unwrap().contains("avertissements"));
    // Après l'échéance : validée, avertie, tracée par son code seulement.
    let (id, v, _) = b.pret("Tardive").await;
    horloge.avancer(time::Duration::days(800));
    let r = b.valider(id, v).await;
    assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
    assert_eq!(codes(&r.json()), [CODE_ECHEANCE]);
    let audits = audits(&b).await;
    let derniere = audits.last().unwrap();
    assert!(derniere.contains("avertissements"), "{derniere}");
    assert!(derniere.contains(CODE_ECHEANCE), "{derniere}");
    // Codes seulement : ni la période ni l'échéance (dates) dans la trace.
    let echeance = r.json()["avertissements"][0]["echeance"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(!derniere.contains(&echeance), "{derniere}");
    canonique_sans_avertissement(&b).await;
}

// --- Lot ---------------------------------------------------------------------------------------

#[tokio::test]
async fn le_lot_renvoie_et_trace_les_avertissements() {
    let horloge = HorlogeTest::default();
    let b = banc_avec(Some(&horloge)).await;
    // Dans le délai : liste vide pour chaque écriture.
    let (a, va, _) = b.pret("A").await;
    let r = lot(&b, &[(a, va)], 10).await;
    assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
    assert_eq!(codes(&r.json()["ecritures"][0]), Vec::<String>::new());
    // Après l'échéance : chaque écriture du lot porte son avertissement, tracé en audit.
    let (c, vc, _) = b.pret("C").await;
    let (d, vd, _) = b.pret("D").await;
    horloge.avancer(time::Duration::days(800));
    let r = lot(&b, &[(c, vc), (d, vd)], 10 - 800 * 86_400).await;
    assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
    let ecritures = r.json()["ecritures"].as_array().unwrap().clone();
    assert_eq!(ecritures.len(), 2);
    for e in &ecritures {
        assert_eq!(codes(e), [CODE_ECHEANCE]);
    }
    let audits = audits(&b).await;
    let traces: Vec<&String> = audits
        .iter()
        .filter(|l| l.contains("validation_par_lot"))
        .collect();
    assert_eq!(traces.len(), 3);
    assert!(!traces[0].contains("avertissements"));
    for t in &traces[1..] {
        assert!(
            t.contains("avertissements") && t.contains(CODE_ECHEANCE),
            "{t}"
        );
    }
    canonique_sans_avertissement(&b).await;
}

// --- Contre-passation --------------------------------------------------------------------------

#[tokio::test]
async fn la_contre_passation_renvoie_et_trace_son_avertissement() {
    let horloge = HorlogeTest::default();
    let b = banc_avec(Some(&horloge)).await;
    let (id, v, _) = b.pret("Origine").await;
    assert_eq!(b.valider(id, v).await.statut, StatusCode::OK);
    // Dans le délai : aucun avertissement.
    let r = contre_passer(&b, &id.to_string(), &aujourd()).await;
    assert_eq!(r.statut, StatusCode::CREATED, "{}", r.texte());
    assert_eq!(codes(&r.json()), Vec::<String>::new());
    // Après l'échéance : contre-passée, avertie, tracée.
    horloge.avancer(time::Duration::days(800));
    let r = contre_passer(&b, &id.to_string(), &aujourd()).await;
    assert_eq!(r.statut, StatusCode::CREATED, "{}", r.texte());
    assert_eq!(codes(&r.json()), [CODE_ECHEANCE]);
    let audits = audits(&b).await;
    let derniere = audits.last().unwrap();
    assert!(derniere.contains("contre_passation"), "{derniere}");
    assert!(derniere.contains("avertissements") && derniere.contains(CODE_ECHEANCE));
    canonique_sans_avertissement(&b).await;
}

#[tokio::test]
async fn contre_passation_avant_l_origine_dans_le_meme_exercice() {
    let b = banc().await;
    let (id, v, _) = b.pret("Origine").await;
    assert_eq!(b.valider(id, v).await.statut, StatusCode::OK);
    let debut = debut_des_exercices(&b).await[0].clone();
    let r = contre_passer(&b, &id.to_string(), &debut).await;
    assert_eq!(r.statut, StatusCode::CREATED, "{}", r.texte());
    let nouveau = r.json()["id"].as_str().unwrap().to_owned();
    let e = lire(
        &b.routeur,
        &format!("{}/{nouveau}", b.ecritures()),
        Some(b.auditeur),
    )
    .await;
    assert_eq!(e.json()["date"], debut);
    assert_eq!(e.json()["origine"], id.to_string());
    assert!(b.chaine_valide().await);
}

#[tokio::test]
async fn contre_passation_avant_l_origine_et_d_un_exercice_a_l_autre() {
    let b = banc().await;
    // Ouvre l'exercice suivant (administrateur, session fraîche).
    let r = envoyer(
        &b.routeur,
        &format!(
            "/api/cabinets/{}/dossiers/{}/exercices/suivant",
            b.c.cabinet, b.c.dossier
        ),
        Some(b.c.administrateur),
        "{}",
        Some(10),
    )
    .await;
    assert_eq!(r.statut, StatusCode::CREATED, "{}", r.texte());
    let debuts = debut_des_exercices(&b).await;
    assert_eq!(debuts.len(), 2);
    let (n, suivant) = (debuts[0].clone(), debuts[1].clone());
    // Origine datée du 15 janvier de l'exercice suivant.
    let quinze = format!("{}15", &suivant[..8]);
    let corps = contenu(&journal_de_saisie(), &quinze, &equilibre("1000"), "");
    let (id, v) = b.pret_avec(&corps).await;
    let r = b.valider(id, v).await;
    assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
    // Avant l'origine, dans le même exercice (suivant) : acceptée.
    let r = contre_passer(&b, &id.to_string(), &suivant).await;
    assert_eq!(r.statut, StatusCode::CREATED, "{}", r.texte());
    assert!(r.json()["numero"].as_str().unwrap().contains(&suivant[..4]));
    // Avant l'origine, dans l'exercice précédent : acceptée, numérotée dans cet exercice.
    let r = contre_passer(&b, &id.to_string(), &n).await;
    assert_eq!(r.statut, StatusCode::CREATED, "{}", r.texte());
    assert!(r.json()["numero"].as_str().unwrap().contains(&n[..4]));
    assert!(b.chaine_valide().await);
    // Origine de l'exercice courant, contre-passée dans l'exercice suivant : acceptée aussi.
    let (id2, v2, _) = b.pret("Origine N").await;
    assert_eq!(b.valider(id2, v2).await.statut, StatusCode::OK);
    let r = contre_passer(&b, &id2.to_string(), &suivant).await;
    assert_eq!(r.statut, StatusCode::CREATED, "{}", r.texte());
    assert!(b.chaine_valide().await);
}

#[tokio::test]
async fn contre_passation_refusee_dans_une_periode_ou_un_exercice_clotures() {
    let b = banc().await;
    let (id, v, _) = b.pret("Origine").await;
    assert_eq!(b.valider(id, v).await.statut, StatusCode::OK);
    let avant = b.compter("ecriture_validee").await;
    let debut = debut_des_exercices(&b).await[0].clone();
    // Période du jour clôturée (et les précédentes) : refus 409, à la date du jour comme avant.
    periodes(&b, "cloturee", &aujourd()).await;
    for date in [aujourd(), debut.clone()] {
        let r = contre_passer(&b, &id.to_string(), &date).await;
        assert_eq!(r.statut, StatusCode::CONFLICT, "{date} : {}", r.texte());
        assert_eq!(code(&r), "periode_cloturee", "{date}");
    }
    // Exercice clôturé (toutes ses périodes le sont) : refus 409.
    periodes(&b, "cloturee", "9999-12-31").await;
    b.sql("UPDATE exercice SET statut = 'cloture' WHERE dossier_id = $1")
        .await;
    let r = contre_passer(&b, &id.to_string(), &aujourd()).await;
    assert_eq!(r.statut, StatusCode::CONFLICT, "{}", r.texte());
    assert_eq!(code(&r), "exercice_cloture");
    assert_eq!(b.compter("ecriture_validee").await, avant);
    assert!(b.chaine_valide().await);
}

// --- Verrouillage et clôture restent bloquants après l'échéance ---------------------------------

#[tokio::test]
async fn apres_l_echeance_le_verrouillage_et_la_cloture_bloquent_toujours() {
    let horloge = HorlogeTest::default();
    let b = banc_avec(Some(&horloge)).await;
    let (id, v, _) = b.pret("Tardive").await;
    horloge.avancer(time::Duration::days(800));
    for (statut, motif) in [
        ("verrouillee", "periode_verrouillee"),
        ("cloturee", "periode_cloturee"),
    ] {
        let borne = if statut == "cloturee" {
            "debut <= $4::date"
        } else {
            "debut <= $4::date AND fin >= $4::date"
        };
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "UPDATE periode SET statut = $2, statut_par = $3, statut_le = now() \
             WHERE dossier_id = $1 AND {borne}"
        )))
        .bind(b.c.dossier.uuid())
        .bind(statut)
        .bind(b.c.administrateur.uuid())
        .bind(aujourd())
        .execute(&b.env.admin)
        .await
        .unwrap();
        let r = b.valider(id, v).await;
        assert_eq!(r.statut, StatusCode::CONFLICT, "{statut} : {}", r.texte());
        assert_eq!(code(&r), motif);
        assert_eq!(b.compter("ecriture_validee").await, 0, "{statut}");
        assert_eq!(b.compter("brouillon").await, 1, "{statut}");
    }
}
