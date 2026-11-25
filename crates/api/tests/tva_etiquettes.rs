//! Étiquettes fiscales et comptes de TVA à la validation (issue #189, ADR 0026) : une ligne
//! étiquetée « taxe » ne porte que le compte de TVA de son sens, le total des étiquettes égale
//! le solde des comptes de TVA, à la validation unitaire comme par lot (tout ou rien), avec un
//! 409 et un code stable. Les comptes viennent des données de `syscohada` (443, 444, 445).
#![cfg(feature = "tests-postgres")]
// Fichier de test : les `expect`/`unwrap` y sont voulus.
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::too_many_lines)]

mod commun;

use axum::http::StatusCode;
use commun::banc::{Banc, aujourd, banc, banque, caisse, code, contenu};
use commun::brouillard::{journal_de_saisie, ligne};
use commun::serveur::{Reponse, envoyer};
use uuid::Uuid;

impl Banc {
    async fn lot(&self, elements: &[(Uuid, i32)]) -> Reponse {
        let liste: Vec<String> = elements
            .iter()
            .map(|(id, v)| format!(r#"{{"ecriture":"{id}","version":{v}}}"#))
            .collect();
        envoyer(
            &self.routeur,
            &format!("{}/validation-par-lot", self.base()),
            Some(self.valideur),
            &format!(r#"{{"ecritures":[{}]}}"#, liste.join(",")),
            None,
        )
        .await
    }

    async fn audits(&self) -> Vec<String> {
        sqlx::query_scalar("SELECT contenu FROM audit_dossier WHERE dossier_id = $1 ORDER BY rang")
            .bind(self.c.dossier.uuid())
            .fetch_all(&self.env.admin)
            .await
            .unwrap()
    }
}

/// Ligne étiquetée : nature (`base` ou `taxe`), sens de TVA (`collectee` ou `deductible`).
fn etiquetee(compte: &str, sens: &str, montant: &str, nature: &str, sens_tva: &str) -> String {
    format!(
        r#"{{"compte":"{compte}","sens":"{sens}","montant":"{montant}","etiquette":{{"nature":"{nature}","sens":"{sens_tva}","regime":"IMP18","taux":"18","exigibilite":"debit"}}}}"#
    )
}

/// Vente de 1 000 hors taxe, 180 de TVA, encaissée en caisse.
fn vente(compte_taxe: &str, nature_taxe: &str, sens_tva_taxe: &str) -> Vec<String> {
    vec![
        ligne(caisse(), "debit", "1180"),
        etiquetee("7011", "credit", "1000", "base", "collectee"),
        etiquetee(compte_taxe, "credit", "180", nature_taxe, sens_tva_taxe),
    ]
}

async fn brouillon(b: &Banc, lignes: &[String]) -> (Uuid, i32) {
    b.pret_avec(&contenu(&journal_de_saisie(), &aujourd(), lignes, ""))
        .await
}

async fn rien_n_est_ecrit(b: &Banc) {
    assert_eq!(b.compter("ecriture_validee").await, 0);
    assert!(b.compteur().await.iter().all(|d| *d == 0));
    assert!(b.chaine().await.is_empty());
}

#[tokio::test]
async fn ecritures_de_tva_coherentes_validees() {
    let b = banc().await;
    // Vente : TVA facturée au 443x.
    let (id, v) = brouillon(&b, &vente("4431", "taxe", "collectee")).await;
    let r = b.valider(id, v).await;
    assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
    assert_eq!(r.json()["format"], 3);
    // Achat : TVA récupérable au 445x.
    let achat = vec![
        etiquetee("6011", "debit", "1000", "base", "deductible"),
        etiquetee("4452", "debit", "180", "taxe", "deductible"),
        ligne(banque(), "credit", "1180"),
    ];
    let (id, v) = brouillon(&b, &achat).await;
    assert_eq!(b.valider(id, v).await.statut, StatusCode::OK);
    // Centralisation de la période : 443 et 445 virés au 444, sans étiquette.
    let centralisation = vec![
        ligne("4431", "debit", "180"),
        ligne("4452", "credit", "100"),
        ligne("4441", "credit", "80"),
    ];
    let (id, v) = brouillon(&b, &centralisation).await;
    let r = b.valider(id, v).await;
    assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
    // Règlement de la TVA due : ne touche ni 443 ni 445.
    let reglement = vec![
        ligne("4441", "debit", "10"),
        ligne(banque(), "credit", "10"),
    ];
    let (id, v) = brouillon(&b, &reglement).await;
    assert_eq!(b.valider(id, v).await.statut, StatusCode::OK);
    assert_eq!(b.compter("ecriture_validee").await, 4);
}

#[tokio::test]
async fn taxe_collectee_sur_un_compte_de_charge_refusee_en_409() {
    let b = banc().await;
    // Le total des étiquettes (180) égale le solde des comptes de TVA (180, ligne 4431 sans
    // étiquette) : seul le rapprochement ligne à ligne voit la « taxe collectée » sur le 6011.
    let lignes = vec![
        ligne(caisse(), "debit", "1360"),
        etiquetee("7011", "credit", "1000", "base", "collectee"),
        etiquetee("6011", "credit", "180", "taxe", "collectee"),
        ligne("4431", "credit", "180"),
    ];
    let (id, v) = brouillon(&b, &lignes).await;
    let r = b.valider(id, v).await;
    assert_eq!(r.statut, StatusCode::CONFLICT, "{}", r.texte());
    assert_eq!(code(&r), "etiquette_tva_incoherente");
    rien_n_est_ecrit(&b).await;
    assert_eq!(b.compter("brouillon").await, 1);
}

#[tokio::test]
async fn taxe_du_mauvais_sens_ou_sur_le_compte_du_solde_refusee() {
    let b = banc().await;
    for (compte, sens_tva) in [
        ("4431", "deductible"),
        ("4452", "collectee"),
        ("4441", "collectee"),
    ] {
        let (id, v) = brouillon(&b, &vente(compte, "taxe", sens_tva)).await;
        let r = b.valider(id, v).await;
        assert_eq!(r.statut, StatusCode::CONFLICT, "{compte} : {}", r.texte());
        assert_eq!(code(&r), "etiquette_tva_incoherente", "{compte}");
    }
    // Une base portée par un compte de TVA.
    let base_sur_tva = vec![
        ligne(caisse(), "debit", "1000"),
        etiquetee("4431", "credit", "1000", "base", "collectee"),
    ];
    let (id, v) = brouillon(&b, &base_sur_tva).await;
    let r = b.valider(id, v).await;
    assert_eq!(r.statut, StatusCode::CONFLICT, "{}", r.texte());
    assert_eq!(code(&r), "etiquette_tva_incoherente");
    rien_n_est_ecrit(&b).await;
}

#[tokio::test]
async fn ligne_de_tva_sans_etiquette_ou_en_ecart_refusee_en_409() {
    let b = banc().await;
    // Ligne sur le 4431 sans étiquette « taxe ».
    let sans = vec![
        ligne(caisse(), "debit", "1180"),
        ligne("7011", "credit", "1000"),
        ligne("4431", "credit", "180"),
    ];
    let (id, v) = brouillon(&b, &sans).await;
    let r = b.valider(id, v).await;
    assert_eq!(r.statut, StatusCode::CONFLICT, "{}", r.texte());
    assert_eq!(code(&r), "ecart_tva_etiquettes");
    // Étiquette de 100 pour 180 de TVA au compte (deux lignes de TVA, une seule étiquetée).
    let ecart = vec![
        ligne(caisse(), "debit", "1180"),
        ligne("7011", "credit", "1000"),
        etiquetee("4431", "credit", "100", "taxe", "collectee"),
        ligne("4432", "credit", "80"),
    ];
    let (id, v) = brouillon(&b, &ecart).await;
    let r = b.valider(id, v).await;
    assert_eq!(r.statut, StatusCode::CONFLICT, "{}", r.texte());
    assert_eq!(code(&r), "ecart_tva_etiquettes");
    rien_n_est_ecrit(&b).await;
}

#[tokio::test]
async fn lot_refuse_les_ecritures_de_tva_incoherentes_et_n_ecrit_rien() {
    let b = banc().await;
    let (bonne, vb) = brouillon(&b, &vente("4431", "taxe", "collectee")).await;
    let (charge, vc) = brouillon(
        &b,
        &[
            ligne(caisse(), "debit", "1360"),
            etiquetee("7011", "credit", "1000", "base", "collectee"),
            etiquetee("6011", "credit", "180", "taxe", "collectee"),
            ligne("4431", "credit", "180"),
        ],
    )
    .await;
    let (sans, vs) = brouillon(
        &b,
        &[
            ligne(caisse(), "debit", "1180"),
            ligne("7011", "credit", "1000"),
            ligne("4431", "credit", "180"),
        ],
    )
    .await;
    let audits_avant = b.audits().await.len();
    let r = b.lot(&[(bonne, vb), (charge, vc), (sans, vs)]).await;
    assert_eq!(r.statut, StatusCode::CONFLICT, "{}", r.texte());
    assert_eq!(code(&r), "lot_refuse");
    let mut obtenu: Vec<(String, String)> = r.json()["refus"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| {
            (
                x["ecriture"].as_str().unwrap().to_owned(),
                x["code"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    let mut attendu = vec![
        (charge.to_string(), "etiquette_tva_incoherente".to_owned()),
        (sans.to_string(), "ecart_tva_etiquettes".to_owned()),
    ];
    obtenu.sort();
    attendu.sort();
    assert_eq!(obtenu, attendu);
    // Tout ou rien : même la bonne écriture n'est pas validée.
    rien_n_est_ecrit(&b).await;
    assert_eq!(b.audits().await.len(), audits_avant);
    // Le lot de la seule écriture cohérente passe.
    let r = b.lot(&[(bonne, vb)]).await;
    assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
    assert_eq!(b.compter("ecriture_validee").await, 1);
}

#[tokio::test]
async fn la_contre_passation_d_une_ecriture_de_tva_reste_possible() {
    let b = banc().await;
    let (id, v) = brouillon(&b, &vente("4431", "taxe", "collectee")).await;
    assert_eq!(b.valider(id, v).await.statut, StatusCode::OK);
    // Le miroir copie les étiquettes, sens D/C inversés : la correction n'est jamais bloquée
    // par le rapprochement (y compris pour une écriture validée avant ce contrôle).
    let r = envoyer(
        &b.routeur,
        &format!("{}/{id}/contre-passation", b.ecritures()),
        Some(b.valideur),
        &format!(
            r#"{{"date":"{}","libelle":"Contre-passation de test","motif":"Erreur de saisie"}}"#,
            aujourd()
        ),
        None,
    )
    .await;
    assert_eq!(r.statut, StatusCode::CREATED, "{}", r.texte());
}
