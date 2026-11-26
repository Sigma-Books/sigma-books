//! Validation par lot et contre-passation (PR 13c, L2-27) : tout ou rien, ré-authentification,
//! exclusion des écritures « à revoir », une seule transaction, contre-passation sérialisée par la
//! tête de chaîne (une seule par origine), justification et étiquette reprises, chaîne et audit.
//!
//! Isolation de chaque route et « Cedar se trompe » : `tests/isolation/brouillard.rs`. Propriété
//! contre `Livre` : tests de `api::serveur::routes::brouillard::validation`.
#![cfg(feature = "tests-postgres")]
// Fichier de test : les `expect`/`unwrap` y sont voulus.
#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::cast_possible_truncation,
    clippy::many_single_char_names,
    clippy::too_many_lines
)]

mod commun;

use axum::http::StatusCode;
use commun::banc::{Banc, aujourd, banc, banque, caisse, code, contenu};
use commun::brouillard::{journal_de_saisie, ligne};
use commun::serveur::{Reponse, envoyer, lire};
use uuid::Uuid;

impl Banc {
    fn lot_chemin(&self) -> String {
        format!("{}/validation-par-lot", self.base())
    }

    async fn lot(
        &self,
        u: api::base::IdUtilisateur,
        anciennete: Option<i64>,
        elements: &[(Uuid, i32)],
    ) -> Reponse {
        let liste: Vec<String> = elements
            .iter()
            .map(|(id, v)| format!(r#"{{"ecriture":"{id}","version":{v}}}"#))
            .collect();
        envoyer(
            &self.routeur,
            &self.lot_chemin(),
            Some(u),
            &format!(r#"{{"ecritures":[{}]}}"#, liste.join(",")),
            anciennete,
        )
        .await
    }

    async fn contre_passer_le(
        &self,
        u: api::base::IdUtilisateur,
        id: Uuid,
        date: &str,
        motif: &str,
    ) -> Reponse {
        envoyer(
            &self.routeur,
            &format!("{}/{id}/contre-passation", self.ecritures()),
            Some(u),
            &format!(
                r#"{{"date":"{date}","libelle":"Contre-passation de test","motif":"{motif}"}}"#
            ),
            None,
        )
        .await
    }

    async fn contre_passer(&self, id: Uuid) -> Reponse {
        self.contre_passer_le(self.valideur, id, &aujourd(), "Erreur de saisie")
            .await
    }

    async fn a_revoir(&self, id: Uuid) {
        let r = envoyer(
            &self.routeur,
            &format!("{}/{id}/a-revoir", self.base()),
            Some(self.valideur),
            "{}",
            None,
        )
        .await;
        assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
    }

    async fn audits(&self) -> Vec<String> {
        sqlx::query_scalar("SELECT contenu FROM audit_dossier WHERE dossier_id = $1 ORDER BY rang")
            .bind(self.c.dossier.uuid())
            .fetch_all(&self.env.admin)
            .await
            .unwrap()
    }
}

fn refus(r: &Reponse) -> Vec<(String, String)> {
    r.json()["refus"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| {
            (
                x["ecriture"].as_str().unwrap().to_owned(),
                x["code"].as_str().unwrap().to_owned(),
            )
        })
        .collect()
}

// --- Lot -----------------------------------------------------------------------------------

#[tokio::test]
async fn lot_nominal_numerote_dans_l_ordre_d_attribution_et_exclut_les_a_revoir() {
    let b = banc().await;
    let (a, va, _) = b.pret("Première").await;
    let (c, vc, _) = b.pret("Seconde").await;
    let (r, vr, _) = b.pret("À revoir").await;
    b.a_revoir(r).await;
    // Corps dans l'ordre inverse : l'attribution suit la date, la création, l'identifiant.
    let rep = b.lot(b.valideur, None, &[(r, vr), (c, vc), (a, va)]).await;
    assert_eq!(rep.statut, StatusCode::OK, "{}", rep.texte());
    let j = rep.json();
    assert_eq!(j["exclues_a_revoir"], 1);
    let ecritures = j["ecritures"].as_array().unwrap();
    assert_eq!(ecritures.len(), 2);
    assert_eq!(ecritures[0]["id"], a.to_string());
    assert_eq!(ecritures[1]["id"], c.to_string());
    assert_eq!(ecritures[0]["rang"], 1);
    assert_eq!(ecritures[1]["rang"], 2);
    assert!(
        ecritures[1]["numero"]
            .as_str()
            .unwrap()
            .ends_with("-000002")
    );
    // L'écriture « à revoir » est restée au brouillard.
    assert_eq!(b.compter("brouillon").await, 1);
    assert_eq!(b.compter("ecriture_validee").await, 2);
    assert_eq!(b.compteur().await, vec![2]);
    assert!(b.chaine_valide().await);
    // Une trace d'audit par écriture validée, dans l'ordre, sans libellé ni montant.
    let audits = b.audits().await;
    let lot: Vec<&String> = audits
        .iter()
        .filter(|e| e.contains("ValiderParLot"))
        .collect();
    assert_eq!(lot.len(), 2);
    assert!(lot.iter().all(|e| e.contains("validation_par_lot")));
    assert!(
        lot.iter()
            .all(|e| !e.contains("Première") && !e.contains("1000"))
    );
}

#[tokio::test]
async fn lot_partiellement_invalide_n_ecrit_rien() {
    let b = banc().await;
    let (bon, vbon, _) = b.pret("Valide").await;
    // Sans pièce : refusée à la validation (justification manquante).
    let sans_piece = b
        .creer(&commun::brouillard::saisie("Sans pièce", None))
        .await;
    // Déséquilibrée.
    let (desequilibre, vd) = b
        .pret_avec(&contenu(
            &journal_de_saisie(),
            &aujourd(),
            &[
                ligne(caisse(), "debit", "1000"),
                ligne(banque(), "credit", "900"),
            ],
            "",
        ))
        .await;
    let (perime, vp, _) = b.pret("Périmée").await;
    let audits_avant = b.audits().await.len();
    let rep = b
        .lot(
            b.valideur,
            None,
            &[
                (bon, vbon),
                (sans_piece, 1),
                (desequilibre, vd),
                (perime, vp + 5),
            ],
        )
        .await;
    assert_eq!(rep.statut, StatusCode::CONFLICT, "{}", rep.texte());
    assert_eq!(code(&rep), "lot_refuse");
    let mut attendu = vec![
        (sans_piece.to_string(), "justification_manquante".to_owned()),
        (
            desequilibre.to_string(),
            "ecriture_desequilibree".to_owned(),
        ),
        (perime.to_string(), "brouillon_perime".to_owned()),
    ];
    let mut obtenu = refus(&rep);
    attendu.sort();
    obtenu.sort();
    assert_eq!(obtenu, attendu);
    // Rien n'est écrit : ni écriture, ni numéro consommé, ni chaîne, ni trace ; brouillons intacts.
    assert_eq!(b.compter("ecriture_validee").await, 0);
    assert_eq!(b.compter("brouillon").await, 4);
    assert!(b.compteur().await.iter().all(|d| *d == 0));
    assert_eq!(b.audits().await.len(), audits_avant);
    // Le lot valide ensuite, une fois corrigé : les numéros commencent à 1 (aucun trou).
    let rep = b.lot(b.valideur, None, &[(bon, vbon)]).await;
    assert_eq!(rep.statut, StatusCode::OK, "{}", rep.texte());
    assert!(
        rep.json()["ecritures"][0]["numero"]
            .as_str()
            .unwrap()
            .ends_with("-000001")
    );
    assert!(b.chaine_valide().await);
}

#[tokio::test]
async fn lot_exige_la_reauthentification() {
    let b = banc().await;
    let (a, va, _) = b.pret("Une").await;
    // Dernière vérification de facteur vieille d'une heure : session non fraîche.
    let r = b.lot(b.valideur, Some(3600), &[(a, va)]).await;
    assert_eq!(r.statut, StatusCode::FORBIDDEN, "{}", r.texte());
    assert_eq!(b.compter("ecriture_validee").await, 0);
    assert_eq!(b.compter("brouillon").await, 1);
    // Fraîche : accepté.
    let r = b.lot(b.valideur, Some(10), &[(a, va)]).await;
    assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
}

#[tokio::test]
async fn lot_bornes_roles_et_separation_des_taches() {
    let b = banc().await;
    let (a, va, _) = b.pret("Une").await;
    // Lot vide, doublon, plus de 100, identifiant mal formé : 400.
    for corps in [
        r#"{"ecritures":[]}"#.to_owned(),
        format!(
            r#"{{"ecritures":[{{"ecriture":"{a}","version":{va}}},{{"ecriture":"{a}","version":{va}}}]}}"#
        ),
        format!(
            r#"{{"ecritures":[{}]}}"#,
            (0..101)
                .map(|_| format!(r#"{{"ecriture":"{}","version":1}}"#, Uuid::new_v4()))
                .collect::<Vec<_>>()
                .join(",")
        ),
        r#"{"ecritures":[{"ecriture":"pas-un-uuid","version":1}]}"#.to_owned(),
        r#"{"ecritures":[{"ecriture":"x","version":1}],"autre":1}"#.to_owned(),
    ] {
        let r = envoyer(&b.routeur, &b.lot_chemin(), Some(b.valideur), &corps, None).await;
        assert_eq!(r.statut, StatusCode::BAD_REQUEST, "{corps}");
    }
    // 100 écritures inexistantes : borne admise, ressource absente (404).
    let cent: Vec<(Uuid, i32)> = (0..100).map(|_| (Uuid::new_v4(), 1)).collect();
    assert_eq!(
        b.lot(b.valideur, None, &cent).await.statut,
        StatusCode::NOT_FOUND
    );
    // Collaborateur (même délégué) et auditeur : refusés. Le créateur ne valide pas son brouillon.
    for u in [b.c.collaborateur, b.auditeur] {
        assert_eq!(
            b.lot(u, None, &[(a, va)]).await.statut,
            StatusCode::FORBIDDEN
        );
    }
    let propre = envoyer(
        &b.routeur,
        &b.base(),
        Some(b.valideur),
        &commun::brouillard::saisie("Du valideur", None),
        None,
    )
    .await;
    let propre: Uuid = propre.json()["id"].as_str().unwrap().parse().unwrap();
    assert_eq!(
        b.lot(b.valideur, None, &[(a, va), (propre, 1)])
            .await
            .statut,
        StatusCode::FORBIDDEN
    );
    assert_eq!(b.compter("ecriture_validee").await, 0);
    // Un lot de brouillons tous « à revoir » : rien à valider.
    b.a_revoir(a).await;
    let r = b.lot(b.valideur, None, &[(a, va)]).await;
    assert_eq!(r.statut, StatusCode::CONFLICT);
    assert_eq!(code(&r), "lot_sans_ecriture_a_valider");
}

#[tokio::test]
async fn lot_isolation_entre_tenants() {
    let b = banc().await;
    let autre = banc().await;
    let (mien, vm, _) = b.pret("Mien").await;
    let (etranger, ve, _) = autre.pret("Étranger").await;
    // Le brouillon d'un autre cabinet dans le lot : 404 pour tout le lot, rien n'est écrit.
    let r = b.lot(b.valideur, None, &[(mien, vm), (etranger, ve)]).await;
    assert_eq!(r.statut, StatusCode::NOT_FOUND, "{}", r.texte());
    assert_eq!(b.compter("ecriture_validee").await, 0);
    assert_eq!(autre.compter("ecriture_validee").await, 0);
    assert_eq!(autre.compter("brouillon").await, 1);
    // Écriture d'un autre cabinet à contre-passer : même 404 qu'une écriture inexistante.
    let (e, ve, _) = autre.pret("À valider").await;
    assert_eq!(autre.valider(e, ve).await.statut, StatusCode::OK);
    let r = b.contre_passer(e).await;
    let inexistante = b.contre_passer(Uuid::new_v4()).await;
    assert_eq!(r.statut, StatusCode::NOT_FOUND);
    assert_eq!(r.texte(), inexistante.texte());
    assert_eq!(autre.compter("ecriture_validee").await, 1);
}

#[tokio::test]
async fn lot_concurrent_sans_trou() {
    let b = banc().await;
    let mut taches = Vec::new();
    for n in 0..4 {
        let (x, vx, _) = b.pret(&format!("Lot {n} a")).await;
        let (y, vy, _) = b.pret(&format!("Lot {n} b")).await;
        let (routeur, chemin, u) = (b.routeur.clone(), b.lot_chemin(), b.valideur);
        let corps = format!(
            r#"{{"ecritures":[{{"ecriture":"{x}","version":{vx}}},{{"ecriture":"{y}","version":{vy}}}]}}"#
        );
        taches.push(tokio::spawn(async move {
            envoyer(&routeur, &chemin, Some(u), &corps, None).await
        }));
    }
    for t in taches {
        let r = t.await.unwrap();
        assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
    }
    assert_eq!(b.compteur().await, vec![8]);
    assert!(b.chaine_valide().await);
}

// --- Contre-passation --------------------------------------------------------------------------

#[tokio::test]
async fn contre_passation_nominale() {
    let b = banc().await;
    // Une écriture avec étiquette fiscale (format v3), justifiée par une pièce.
    let etiquetee = format!(
        r#"{{"compte":"{}","sens":"debit","montant":"1000","etiquette":{{"nature":"base","sens":"collectee","regime":"TAXABLE_NORMAL","taux":"18","exigibilite":"debit"}}}}"#,
        caisse()
    );
    let (id, v) = b
        .pret_avec(&contenu(
            &journal_de_saisie(),
            &aujourd(),
            &[etiquetee, ligne(banque(), "credit", "1000")],
            "",
        ))
        .await;
    assert_eq!(b.valider(id, v).await.statut, StatusCode::OK);
    let r = b.contre_passer(id).await;
    assert_eq!(r.statut, StatusCode::CREATED, "{}", r.texte());
    let j = r.json();
    assert_eq!(j["origine"], id.to_string());
    assert_eq!(j["rang"], 2);
    assert_eq!(j["format"], 3);
    let nouveau: Uuid = j["id"].as_str().unwrap().parse().unwrap();
    // Sens inversés, mêmes comptes, étiquette contre-passée avec la ligne, justification reprise.
    let lignes: Vec<(String, String, Option<String>)> = sqlx::query_as(
        "SELECT compte_numero, sens, fiscal_regime FROM ecriture_validee_ligne \
         WHERE dossier_id = $1 AND ecriture_id = $2 ORDER BY position",
    )
    .bind(b.c.dossier.uuid())
    .bind(nouveau)
    .fetch_all(&b.env.admin)
    .await
    .unwrap();
    assert_eq!(
        lignes,
        vec![
            (
                caisse().to_owned(),
                "credit".to_owned(),
                Some("TAXABLE_NORMAL".to_owned())
            ),
            (banque().to_owned(), "debit".to_owned(), None),
        ]
    );
    let pieces: Vec<(Uuid, Uuid)> = sqlx::query_as(
        "SELECT ecriture_id, piece_id FROM ecriture_validee_piece WHERE dossier_id = $1 \
         ORDER BY ecriture_id, piece_id",
    )
    .bind(b.c.dossier.uuid())
    .fetch_all(&b.env.admin)
    .await
    .unwrap();
    assert_eq!(pieces.len(), 2);
    assert_eq!(pieces[0].1, pieces[1].1);
    // Chaîne, compteur, lecture (origine et contre-passée par), audit avec motif.
    assert!(b.chaine_valide().await);
    assert_eq!(b.compteur().await, vec![2]);
    let o = lire(
        &b.routeur,
        &format!("{}/{id}", b.ecritures()),
        Some(b.auditeur),
    )
    .await;
    assert_eq!(
        o.json()["contre_passations"],
        serde_json::json!([j["numero"]])
    );
    let n = lire(
        &b.routeur,
        &format!("{}/{nouveau}", b.ecritures()),
        Some(b.auditeur),
    )
    .await;
    assert_eq!(n.json()["origine"], id.to_string());
    assert!(n.json()["date_valeur_comptable"].is_null());
    let audits = b.audits().await;
    let derniere = audits.last().unwrap();
    assert!(derniere.contains("ContrePasser"));
    assert!(derniere.contains("contre_passation"));
    assert!(derniere.contains("Erreur de saisie"));
    assert!(!derniere.contains("Contre-passation de test") && !derniere.contains("1000"));
    // Contre-passer une contre-passation est admis, comme contre-passer deux fois la même
    // origine (ADR 0021 point 4, révisé le 2026-11-25).
    let encore = b.contre_passer(nouveau).await;
    assert_eq!(encore.statut, StatusCode::CREATED, "{}", encore.texte());
    let seconde = b.contre_passer(id).await;
    assert_eq!(seconde.statut, StatusCode::CREATED, "{}", seconde.texte());
    assert_eq!(seconde.json()["origine"], id.to_string());
    assert_eq!(seconde.json()["rang"], 4);
    assert_ne!(seconde.json()["numero"], j["numero"]);
    assert!(b.chaine_valide().await);
    assert_eq!(b.compter("ecriture_validee").await, 4);
    assert_eq!(b.compteur().await, vec![4]);
    let o = lire(
        &b.routeur,
        &format!("{}/{id}", b.ecritures()),
        Some(b.auditeur),
    )
    .await;
    assert_eq!(
        o.json()["contre_passations"],
        serde_json::json!([j["numero"], seconde.json()["numero"]])
    );
    // Chaque contre-passation a sa trace d'audit, avec son numéro.
    let audits = b.audits().await;
    let numero_seconde = seconde.json()["numero"].as_str().unwrap().to_owned();
    assert_eq!(
        audits
            .iter()
            .filter(|a| a.contains("contre_passation"))
            .count(),
        3
    );
    assert!(audits.last().unwrap().contains(&numero_seconde));
}

/// Motif facultatif (Odoo `account_move_reversal.py:18`, Tryton `CancelMoves`) : absent, `null`,
/// vide ou blanc, le serveur inscrit « Contre-passation de <numéro de l'origine> » en audit.
#[tokio::test]
async fn contre_passation_sans_motif_prend_le_motif_par_defaut() {
    let b = banc().await;
    let (id, v, _) = b.pret("Origine").await;
    assert_eq!(b.valider(id, v).await.statut, StatusCode::OK);
    let origine = lire(
        &b.routeur,
        &format!("{}/{id}", b.ecritures()),
        Some(b.auditeur),
    )
    .await
    .json()["numero"]
        .as_str()
        .unwrap()
        .to_owned();
    for (i, motif) in [
        "",
        r#","motif":null"#,
        r#","motif":"""#,
        r#","motif":"   ""#,
    ]
    .iter()
    .enumerate()
    {
        let corps = format!(r#"{{"date":"{}","libelle":"x"{motif}}}"#, aujourd());
        let r = envoyer(
            &b.routeur,
            &format!("{}/{id}/contre-passation", b.ecritures()),
            Some(b.valideur),
            &corps,
            None,
        )
        .await;
        assert_eq!(r.statut, StatusCode::CREATED, "{corps} : {}", r.texte());
        let audits = b.audits().await;
        let derniere = audits.last().unwrap();
        assert!(
            derniere.contains(&format!("Contre-passation de {origine}")),
            "{i} : {derniere}"
        );
    }
    assert_eq!(b.compter("ecriture_validee").await, 5);
    // Un motif de 400 octets exactement est accepté.
    let r = b
        .contre_passer_le(b.valideur, id, &aujourd(), &"a".repeat(400))
        .await;
    assert_eq!(r.statut, StatusCode::CREATED, "{}", r.texte());
    assert!(b.chaine_valide().await);
}

#[tokio::test]
async fn contre_passation_refus() {
    let b = banc().await;
    let (id, v, _) = b.pret("Origine").await;
    assert_eq!(b.valider(id, v).await.statut, StatusCode::OK);
    // Rôles : collaborateur et auditeur refusés.
    for u in [b.c.collaborateur, b.auditeur] {
        let r = b.contre_passer_le(u, id, &aujourd(), "Motif").await;
        assert_eq!(r.statut, StatusCode::FORBIDDEN);
    }
    // Corps invalides : date mal formée, champ inconnu, motif de plus de 400 octets ou avec
    // un caractère de contrôle (un motif absent ou vide est accepté : voir
    // `contre_passation_sans_motif_prend_le_motif_par_defaut`).
    let trop_long = format!(
        r#"{{"date":"2026-11-20","libelle":"x","motif":"{}"}}"#,
        "a".repeat(401)
    );
    let avec_controle = r#"{"date":"2026-11-20","libelle":"x","motif":"a\nb"}"#.to_owned();
    for corps in [
        r#"{"date":"20/11/2026","libelle":"x","motif":"m"}"#.to_owned(),
        r#"{"date":"2026-11-20","libelle":"x","motif":"m","autre":1}"#.to_owned(),
        trop_long,
        avec_controle,
    ] {
        let corps = corps.as_str();
        let r = envoyer(
            &b.routeur,
            &format!("{}/{id}/contre-passation", b.ecritures()),
            Some(b.valideur),
            corps,
            None,
        )
        .await;
        assert_eq!(r.statut, StatusCode::BAD_REQUEST, "{corps}");
    }
    // Date hors de tout exercice : 422 (une date antérieure à l'origine, elle, est acceptée).
    let r = b
        .contre_passer_le(b.valideur, id, "2000-01-01", "Motif")
        .await;
    assert_eq!(r.statut, StatusCode::UNPROCESSABLE_ENTITY, "{}", r.texte());
    assert_eq!(b.compter("ecriture_validee").await, 1);
    assert!(b.chaine_valide().await);
}

#[tokio::test]
async fn deux_contre_passations_simultanees_aboutissent_toutes_deux() {
    let b = banc().await;
    let (id, v, _) = b.pret("Origine").await;
    assert_eq!(b.valider(id, v).await.statut, StatusCode::OK);
    let mut taches = Vec::new();
    for _ in 0..4 {
        let (routeur, chemin, u) = (
            b.routeur.clone(),
            format!("{}/{id}/contre-passation", b.ecritures()),
            b.valideur,
        );
        let corps = format!(
            r#"{{"date":"{}","libelle":"Contre-passation","motif":"Concurrence"}}"#,
            aujourd()
        );
        taches.push(tokio::spawn(async move {
            envoyer(&routeur, &chemin, Some(u), &corps, None).await
        }));
    }
    let mut numeros = std::collections::BTreeSet::new();
    let mut rangs = std::collections::BTreeSet::new();
    for t in taches {
        let r = t.await.unwrap();
        assert_eq!(r.statut, StatusCode::CREATED, "{}", r.texte());
        numeros.insert(r.json()["numero"].as_str().unwrap().to_owned());
        rangs.insert(r.json()["rang"].as_i64().unwrap());
    }
    // Aucun doublon de numéro ni de rang de chaîne.
    assert_eq!(numeros.len(), 4);
    assert_eq!(rangs, (2..=5).collect());
    assert_eq!(b.compter("ecriture_validee").await, 5);
    assert_eq!(b.compteur().await, vec![5]);
    assert!(b.chaine_valide().await);
    let audits = b.audits().await;
    assert_eq!(
        audits
            .iter()
            .filter(|a| a.contains("contre_passation"))
            .count(),
        4
    );
}

// --- Base ------------------------------------------------------------------------------------

#[tokio::test]
async fn l_unicite_de_l_origine_est_levee_et_les_garde_fous_restent() {
    let b = banc().await;
    let (id, v, _) = b.pret("Origine").await;
    assert_eq!(b.valider(id, v).await.statut, StatusCode::OK);
    assert_eq!(b.contre_passer(id).await.statut, StatusCode::CREATED);
    // `0205` : plus d'index unique, un index non unique sur l'origine.
    let (unique, simple): (i64, i64) = sqlx::query_as(
        "SELECT count(*) FILTER (WHERE indexname = 'ecriture_validee_origine_unique'), \
                count(*) FILTER (WHERE indexname = 'ecriture_validee_origine_idx' \
                                   AND indexdef NOT LIKE '%UNIQUE%') \
         FROM pg_indexes",
    )
    .fetch_one(&b.env.admin)
    .await
    .unwrap();
    assert_eq!((unique, simple), (0, 1));
    // Une écriture ne se cite pas elle-même ; la date de valeur est exclue d'une contre-passation.
    let auto = sqlx::query(
        "UPDATE ecriture_validee SET origine_id = id WHERE dossier_id = $1 AND origine_id IS NULL",
    )
    .bind(b.c.dossier.uuid())
    .execute(&b.env.admin)
    .await;
    assert!(auto.is_err());
}

#[test]
fn les_routes_du_lot_et_de_la_contre_passation_sont_declarees() {
    use api::serveur::routes::{Methode, ROUTES};
    let attendues = [
        "/api/cabinets/{cabinet}/dossiers/{dossier}/brouillard/validation-par-lot",
        "/api/cabinets/{cabinet}/dossiers/{dossier}/ecritures/{ecriture}/contre-passation",
    ];
    for chemin in attendues {
        let r = ROUTES.iter().find(|r| r.chemin == chemin).expect(chemin);
        assert_eq!(r.methode, Methode::Post);
    }
}
