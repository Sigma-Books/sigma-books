//! Fiche des tiers par l'API (lot L3, PR 7a, L3-15 ; ADR 0031) : identifiants typés par le pays
//! du dossier, doublons signalés, adresses et contacts, coordonnées bancaires masquées à
//! l'auditeur et alerte aux administrateurs, archivage, code unique, audit sans valeur
//! personnelle ; isolation au niveau de la requête (Cedar forcé) et de la RLS.
#![cfg(feature = "tests-postgres")]
// Fichier de test : les `expect`/`unwrap` y sont voulus.
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::too_many_lines)]

mod commun;

use std::sync::Arc;

use api::base::{ContexteTenant, IdUtilisateur, ouvrir_transaction};
use api::serveur::{Application, Reglages, routeur};
use axum::Router;
use axum::http::StatusCode;
use commun::banc::{Banc, banc, code, contenu};
use commun::brouillard::{aujourd_hui, journal_de_saisie, ligne, texte_date};
use commun::referentiel::comptes;
use commun::serveur::{AuthentificationTest, ORIGINE, Reponse, ToujoursAutorise, envoyer, lire};
use commun::{CabinetTest, code_sql, creer_cabinet, env};
use serde_json::Value;

fn base_tiers(c: &CabinetTest) -> String {
    format!("/api/cabinets/{}/dossiers/{}/tiers", c.cabinet, c.dossier)
}

async fn poster(b: &Banc, suffixe: &str, qui: IdUtilisateur, corps: &str) -> Reponse {
    envoyer(
        &b.routeur,
        &format!("{}{suffixe}", base_tiers(&b.c)),
        Some(qui),
        corps,
        None,
    )
    .await
}

async fn creer_tiers(b: &Banc, nom: &str, extra: &str) -> Value {
    let r = poster(
        b,
        "",
        b.c.administrateur,
        &format!(
            r#"{{"nom":"{nom}","type":"client","compte_collectif":"{}"{extra}}}"#,
            comptes().collectif
        ),
    )
    .await;
    assert_eq!(r.statut, StatusCode::CREATED, "{}", r.texte());
    r.json()
}

/// Entrées d'audit du dossier, de rang supérieur à `depuis`, en texte.
async fn audit_depuis(b: &Banc, depuis: i64) -> Vec<String> {
    sqlx::query_scalar(
        "SELECT contenu FROM audit_dossier WHERE dossier_id = $1 AND rang > $2 ORDER BY rang",
    )
    .bind(b.c.dossier.uuid())
    .bind(depuis)
    .fetch_all(&b.env.admin)
    .await
    .unwrap()
}

async fn rang_audit(b: &Banc) -> i64 {
    sqlx::query_scalar(
        "SELECT coalesce(max(rang), 0)::bigint FROM audit_dossier WHERE dossier_id = $1",
    )
    .bind(b.c.dossier.uuid())
    .fetch_one(&b.env.admin)
    .await
    .unwrap()
}

#[tokio::test]
async fn fiche_complete_doublons_signales_et_audit_sans_valeur_personnelle() {
    let b = banc().await;
    let rang = rang_audit(&b).await;
    let premier = creer_tiers(&b, "Société Ndiaye", r#","code":"CLI-001","devise":"EUR""#).await;
    assert_eq!(premier["code"], "CLI-001");
    assert_eq!(premier["devise"], "EUR");
    assert_eq!(premier["actif"], true);
    assert_eq!(premier["assujetti_tva"], false);
    let id = premier["id"].as_str().unwrap().to_owned();
    // Code tiré d'office quand il n'est pas fourni : `T` et six chiffres, à la suite du tiers fixe.
    let second = creer_tiers(&b, "Société Ndiaye", "").await;
    assert_eq!(second["code"], "T000002", "{second}");
    let id2 = second["id"].as_str().unwrap().to_owned();

    // Fiche complète : identifiants du pack du Sénégal, adresse, contacts (un courriel mal formé
    // est signalé, non refusé), indicateurs fiscaux.
    let r = poster(
        &b,
        &format!("/{id}/modification"),
        b.c.collaborateur,
        r#"{"assujetti_tva":true,
            "identifiants":[{"type":"NINEA","valeur":"0012345 2G3"},{"type":"RCCM","valeur":"SN-DKR-2020-B-1"}],
            "adresses":[{"libelle":"Siège","rue":"12 rue Carnot","ville":"Dakar","pays":"SN"}],
            "contacts":[{"type":"courriel","valeur":"compta@ndiaye.sn","nom":"Awa Ndiaye"},
                        {"type":"courriel","valeur":"pas-un-courriel"},
                        {"type":"telephone","valeur":"+221 33 000 00 00"}]}"#,
    )
    .await;
    assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
    let fiche = r.json();
    assert_eq!(fiche["assujetti_tva"], true);
    assert_eq!(fiche["identifiants"].as_array().unwrap().len(), 2);
    assert_eq!(fiche["adresses"][0]["ville"], "Dakar");
    let avertissements = fiche["avertissements"].as_array().unwrap();
    assert_eq!(avertissements.len(), 1, "{fiche}");
    assert_eq!(avertissements[0]["code"], "courriel_forme");

    // Même NINEA (espaces et casse ignorés) chez un autre tiers : avertissement, jamais refus.
    let r = poster(
        &b,
        &format!("/{id2}/modification"),
        b.c.collaborateur,
        r#"{"identifiants":[{"type":"NINEA","valeur":"00123452g3"}]}"#,
    )
    .await;
    assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
    let a = &r.json()["avertissements"][0];
    assert_eq!(a["code"], "identifiant_doublon");
    assert_eq!(a["type"], "NINEA");
    assert_eq!(a["tiers"], serde_json::json!([id.clone()]));
    // Le premier le voit aussi à la lecture de sa fiche (même nom toléré, sans avertissement).
    let r = lire(
        &b.routeur,
        &format!("{}/{id}", base_tiers(&b.c)),
        Some(b.auditeur),
    )
    .await;
    assert_eq!(r.statut, StatusCode::OK);
    let fiche_lue = r.json();
    let codes: Vec<&str> = fiche_lue["avertissements"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["code"].as_str().unwrap())
        .collect();
    assert_eq!(codes, ["identifiant_doublon", "courriel_forme"]);

    // Une liste remplace la liste entière : l'adresse est gardée par son `id` et son pays change
    // (journalisé), le RCCM est retiré, le NINEA modifié.
    let adresse = fiche["adresses"][0]["id"].as_str().unwrap().to_owned();
    let r = poster(
        &b,
        &format!("/{id}/modification"),
        b.c.collaborateur,
        &format!(
            r#"{{"identifiants":[{{"type":"NINEA","valeur":"0099999"}}],
                "adresses":[{{"id":"{adresse}","libelle":"Siège","rue":"12 rue Carnot","ville":"Bamako","pays":"ML"}}]}}"#
        ),
    )
    .await;
    assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
    let fiche = r.json();
    assert_eq!(fiche["identifiants"].as_array().unwrap().len(), 1);
    assert_eq!(fiche["adresses"][0]["id"], adresse.as_str());
    assert_eq!(fiche["adresses"][0]["pays"], "ML");
    // Rejouer la même demande : aucun changement (409).
    let r2 = poster(
        &b,
        &format!("/{id}/modification"),
        b.c.collaborateur,
        r#"{"identifiants":[{"type":"NINEA","valeur":"0099999"}]}"#,
    )
    .await;
    assert_eq!(r2.statut, StatusCode::CONFLICT);
    assert_eq!(code(&r2), "aucun_changement");

    // Audit : une entrée par changement, aucune valeur personnelle (ADR 0031 point 8).
    let entrees = audit_depuis(&b, rang).await;
    let tout = entrees.join("\n");
    // Les valeurs saisies ne sont jamais affichées par le test (rang dans la liste seulement).
    let saisies = [
        "Ndiaye", "0012345", "0099999", "SN-DKR", "Carnot", "Dakar", "Bamako", "compta@", "+221",
        "CLI-001",
    ];
    for (rang_saisie, saisie) in saisies.iter().enumerate() {
        assert!(
            !tout.contains(saisie),
            "valeur n° {rang_saisie} dans l'audit"
        );
    }
    // Le changement du pays de l'adresse et celui d'un identifiant sont journalisés, par leurs
    // noms (ADR 0031 point 2).
    assert!(
        tout.contains("donnee=champs\nvaleur=ville,pays\n"),
        "{tout}"
    );
    assert!(tout.contains("donnee=type\nvaleur=NINEA\n"), "{tout}");
    assert!(
        tout.contains("donnee=operation\nvaleur=retrait\n"),
        "{tout}"
    );
    // Chaîne d'audit du dossier intègre.
    let r = lire(
        &b.routeur,
        &format!(
            "/api/cabinets/{}/dossiers/{}/audit/verification",
            b.c.cabinet, b.c.dossier
        ),
        Some(b.c.administrateur),
    )
    .await;
    assert_eq!(r.json()["integre"], true, "{}", r.texte());
}

#[tokio::test]
async fn contraintes_de_la_fiche_selon_le_pays_du_dossier() {
    let b = banc().await;
    let id = b.f.tiers;
    let modifier = |corps: &'static str| {
        let b = &b;
        async move { poster(b, &format!("/{id}/modification"), b.c.collaborateur, corps).await }
    };
    // Type d'identifiant d'un autre pays (aucun au Sénégal), ou en double : 422.
    for corps in [
        r#"{"identifiants":[{"type":"NIU","valeur":"1"}]}"#,
        r#"{"identifiants":[{"type":"RCCM","valeur":"1"},{"type":"RCCM","valeur":"2"}]}"#,
    ] {
        let r = modifier(corps).await;
        assert_eq!(r.statut, StatusCode::UNPROCESSABLE_ENTITY, "{corps}");
        assert_eq!(code(&r), "tiers_identifiant_type_inconnu");
    }
    // Valeurs mal formées : 422 ; champ figé ou inconnu : 400.
    for corps in [
        "{\"nom\":\"a\\u202Eb\"}",
        r#"{"devise":"euro"}"#,
        r#"{"adresses":[{"pays":"Sénégal"}]}"#,
        r#"{"contacts":[{"type":"pigeon","valeur":"x"}]}"#,
    ] {
        let r = modifier(corps).await;
        assert_eq!(r.statut, StatusCode::UNPROCESSABLE_ENTITY, "{corps}");
        assert_eq!(code(&r), "tiers_valeur_invalide");
    }
    for corps in [
        r#"{"type":"fournisseur"}"#,
        r#"{"compte_collectif":"401"}"#,
        r#"{"code":"X"}"#,
        r#"{"actif":false}"#,
        "{}",
    ] {
        assert_eq!(
            modifier(corps).await.statut,
            StatusCode::BAD_REQUEST,
            "{corps}"
        );
    }
    // Adresse d'un autre tiers citée : absente.
    let r = modifier(r#"{"adresses":[{"id":"7b0e8b0a-3f4e-4d4c-9c43-0d1f7a2b5c6d"}]}"#).await;
    assert_eq!(r.statut, StatusCode::NOT_FOUND);

    // NINEA admis au Sénégal ; le même dossier passé au Cameroun ne l'admet plus (types du pack du
    // pays **du dossier**), AUTRE reste admis (tiers étranger).
    assert_eq!(
        modifier(r#"{"identifiants":[{"type":"NINEA","valeur":"1"}]}"#)
            .await
            .statut,
        StatusCode::OK
    );
    // Changement de pays hors des routes (le dossier n'a ni écriture ni brouillon) : le
    // déclencheur `dossier_contexte_fige` exige le dossier en contexte.
    let mut tx = b.env.admin.begin().await.unwrap();
    sqlx::query("SELECT set_config('sigma.dossier', $1, true)")
        .bind(b.c.dossier.to_string())
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query(
        "UPDATE dossier SET pays = 'CM', devise_tenue = 'XAF', fuseau = 'Africa/Douala' \
         WHERE id = $1",
    )
    .bind(b.c.dossier.uuid())
    .execute(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();
    let r = modifier(r#"{"identifiants":[{"type":"NINEA","valeur":"2"}]}"#).await;
    assert_eq!(code(&r), "tiers_identifiant_type_inconnu");
    assert_eq!(
        modifier(r#"{"identifiants":[{"type":"AUTRE","valeur":"FR 123"}]}"#)
            .await
            .statut,
        StatusCode::OK
    );

    // Code unique dans le dossier (409) et de forme fermée (422).
    let corps = |code: &str| {
        format!(
            r#"{{"nom":"X","type":"client","compte_collectif":"{}","code":"{code}"}}"#,
            comptes().collectif
        )
    };
    assert_eq!(
        poster(&b, "", b.c.administrateur, &corps("DOUBLE"))
            .await
            .statut,
        StatusCode::CREATED
    );
    let r = poster(&b, "", b.valideur, &corps("DOUBLE")).await;
    assert_eq!(r.statut, StatusCode::CONFLICT);
    assert_eq!(code(&r), "tiers_code_deja_utilise");
    let r = poster(&b, "", b.valideur, &corps("min")).await;
    assert_eq!(code(&r), "tiers_valeur_invalide");
    // La création reste au paramétrage : le collaborateur est refusé.
    let r = poster(&b, "", b.c.collaborateur, &corps("COLLAB")).await;
    assert_eq!(r.statut, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn coordonnees_bancaires_masquees_a_l_auditeur_et_alerte_aux_administrateurs() {
    let b = banc().await;
    let id = b.f.tiers;
    let autre = creer_tiers(&b, "Autre", "").await["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let chemin = |t: &str| format!("{}/{t}/comptes-bancaires", base_tiers(&b.c));
    let alertes = || async {
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM tache WHERE nature = 'notification' \
             AND sujet = 'coordonnees_bancaires_tiers' AND dossier_cible = $1",
        )
        .bind(b.c.dossier.uuid())
        .fetch_one(&b.env.admin)
        .await
        .unwrap()
    };
    assert_eq!(alertes().await, 0);
    let rang = rang_audit(&b).await;
    // Le collaborateur (saisie) ajoute un RIB ; un type hors du pack est refusé.
    let r = envoyer(
        &b.routeur,
        &chemin(&id.to_string()),
        Some(b.c.collaborateur),
        r#"{"comptes":[{"type":"SWIFT","numero":"1"}]}"#,
        None,
    )
    .await;
    assert_eq!(code(&r), "tiers_compte_bancaire_type_inconnu");
    let r = envoyer(
        &b.routeur,
        &chemin(&id.to_string()),
        Some(b.c.collaborateur),
        r#"{"comptes":[{"type":"RIB","numero":"SN012 01001 000123456789 12","etablissement":"Banque du test","devise":"XOF"}]}"#,
        None,
    )
    .await;
    assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
    assert_eq!(r.json()["id"], id.to_string());
    assert_eq!(r.json()["comptes"].as_array().unwrap().len(), 1);
    assert_eq!(alertes().await, 1);
    // Même numéro chez un autre tiers : signalé, jamais refusé (Odoo).
    let r = envoyer(
        &b.routeur,
        &chemin(&autre),
        Some(b.valideur),
        r#"{"comptes":[{"type":"RIB","numero":"sn01201001000123456789 12"}]}"#,
        None,
    )
    .await;
    assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
    assert_eq!(
        r.json()["avertissements"][0]["code"],
        "compte_bancaire_doublon"
    );
    assert_eq!(
        r.json()["avertissements"][0]["tiers"],
        serde_json::json!([id.to_string()])
    );
    assert_eq!(alertes().await, 2);

    // Lecture : rôles de saisie oui, auditeur non (VP17) ; la fiche ne porte jamais le numéro.
    for qui in [b.c.administrateur, b.valideur, b.c.collaborateur] {
        let r = lire(&b.routeur, &chemin(&id.to_string()), Some(qui)).await;
        assert_eq!(r.statut, StatusCode::OK);
    }
    let r = lire(&b.routeur, &chemin(&id.to_string()), Some(b.auditeur)).await;
    assert_eq!(r.statut, StatusCode::FORBIDDEN);
    let r = lire(
        &b.routeur,
        &format!("{}/{id}", base_tiers(&b.c)),
        Some(b.auditeur),
    )
    .await;
    assert_eq!(r.statut, StatusCode::OK);
    assert!(!r.texte().contains("000123456789"), "{}", r.texte());
    // L'auditeur ne modifie rien.
    let r = envoyer(
        &b.routeur,
        &chemin(&id.to_string()),
        Some(b.auditeur),
        r#"{"comptes":[]}"#,
        None,
    )
    .await;
    assert_eq!(r.statut, StatusCode::FORBIDDEN);

    // Retrait : journalisé et alerté ; aucun numéro dans l'audit ni dans la file.
    let r = envoyer(
        &b.routeur,
        &chemin(&id.to_string()),
        Some(b.c.administrateur),
        r#"{"comptes":[]}"#,
        None,
    )
    .await;
    assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
    assert_eq!(alertes().await, 3);
    let tout = audit_depuis(&b, rang).await.join("\n");
    assert!(
        !tout.contains("000123456789") && !tout.contains("Banque du test"),
        "{tout}"
    );
    assert!(
        tout.contains("donnee=operation\nvaleur=retrait\n"),
        "{tout}"
    );
}

#[tokio::test]
async fn archivage_libre_et_reversible_sans_nouvelle_ligne_au_brouillon() {
    let b = banc().await;
    let id = b.f.tiers;
    let collectif = comptes().collectif.to_string();
    let saisie = || {
        contenu(
            &journal_de_saisie(),
            &texte_date(aujourd_hui()),
            &[
                format!(
                    r#"{{"compte":"{collectif}","sens":"debit","montant":"1000","tiers":"{id}"}}"#
                ),
                ligne(commun::banc::caisse(), "credit", "1000"),
            ],
            "",
        )
    };
    // Motif exigé.
    let r = poster(&b, &format!("/{id}/archivage"), b.c.collaborateur, "{}").await;
    assert_eq!(r.statut, StatusCode::BAD_REQUEST);
    let r = poster(
        &b,
        &format!("/{id}/archivage"),
        b.c.collaborateur,
        r#"{"motif":"Client parti"}"#,
    )
    .await;
    assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
    assert_eq!(r.json()["actif"], false);
    assert_eq!(r.json()["motif_archivage"], "Client parti");
    assert!(r.json()["archive_le"].is_string());
    // Archivé : aucune nouvelle ligne au brouillon ; déjà archivé : 409.
    let r = envoyer(
        &b.routeur,
        &b.base(),
        Some(b.c.collaborateur),
        &saisie(),
        None,
    )
    .await;
    assert_eq!(r.statut, StatusCode::UNPROCESSABLE_ENTITY, "{}", r.texte());
    assert_eq!(code(&r), "tiers_inactif");
    let r = poster(
        &b,
        &format!("/{id}/archivage"),
        b.c.collaborateur,
        r#"{"motif":"Encore"}"#,
    )
    .await;
    assert_eq!(code(&r), "aucun_changement");
    // Toujours lisible et listé.
    let r = lire(&b.routeur, &base_tiers(&b.c), Some(b.auditeur)).await;
    assert!(r.texte().contains(&id.to_string()));
    // Désarchivage : le motif s'efface, la saisie reprend.
    let r = poster(&b, &format!("/{id}/desarchivage"), b.c.collaborateur, "{}").await;
    assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
    assert_eq!(r.json()["actif"], true);
    assert!(r.json()["motif_archivage"].is_null());
    let r = envoyer(
        &b.routeur,
        &b.base(),
        Some(b.c.collaborateur),
        &saisie(),
        None,
    )
    .await;
    assert_eq!(r.statut, StatusCode::CREATED, "{}", r.texte());
    let r = poster(&b, &format!("/{id}/desarchivage"), b.c.collaborateur, "{}").await;
    assert_eq!(code(&r), "aucun_changement");
    // L'auditeur n'archive pas.
    let r = poster(
        &b,
        &format!("/{id}/archivage"),
        b.auditeur,
        r#"{"motif":"Non"}"#,
    )
    .await;
    assert_eq!(r.statut, StatusCode::FORBIDDEN);
}

/// Cedar forcé à « autorisé » : la requête tient seule. Le rôle relu en base refuse la création
/// au collaborateur et à l'auditeur, les coordonnées bancaires et la tenue de la fiche à
/// l'auditeur ; un tiers d'un autre dossier reste absent et inchangé.
#[tokio::test]
async fn la_requete_tient_quand_cedar_se_trompe() {
    let b = banc().await;
    let routeur: Router = routeur(&Application::new(
        b.env.application.clone(),
        b.env.lecture_audit.clone(),
        Arc::new(ToujoursAutorise),
        Arc::new(AuthentificationTest::default()),
        Reglages::new(ORIGINE.to_owned()),
    ));
    let base = base_tiers(&b.c);
    let id = b.f.tiers;
    let creation = format!(
        r#"{{"nom":"X","type":"client","compte_collectif":"{}"}}"#,
        comptes().collectif
    );
    for qui in [b.c.collaborateur, b.auditeur] {
        let r = envoyer(&routeur, &base, Some(qui), &creation, None).await;
        assert_eq!(r.statut, StatusCode::FORBIDDEN, "{}", r.texte());
    }
    let r = lire(
        &routeur,
        &format!("{base}/{id}/comptes-bancaires"),
        Some(b.auditeur),
    )
    .await;
    assert_eq!(r.statut, StatusCode::FORBIDDEN);
    for (suffixe, corps) in [
        ("modification", r#"{"nom":"Pirate"}"#),
        ("archivage", r#"{"motif":"Pirate"}"#),
        ("comptes-bancaires", r#"{"comptes":[]}"#),
    ] {
        let r = envoyer(
            &routeur,
            &format!("{base}/{id}/{suffixe}"),
            Some(b.auditeur),
            corps,
            None,
        )
        .await;
        assert_eq!(r.statut, StatusCode::FORBIDDEN, "{suffixe}");
    }
    // Tiers d'un autre cabinet sous le dossier du banc : absent, inchangé.
    let autre = creer_cabinet(&b.env.admin, "Autre").await;
    let f = commun::referentiel::doter(
        &b.env.application,
        autre.cabinet,
        autre.dossier,
        autre.administrateur,
    )
    .await;
    let r = envoyer(
        &routeur,
        &format!("{base}/{}/modification", f.tiers),
        Some(b.c.administrateur),
        r#"{"nom":"Pirate"}"#,
        None,
    )
    .await;
    assert_eq!(r.statut, StatusCode::NOT_FOUND);
    let r = lire(
        &routeur,
        &format!("{base}/{}", f.tiers),
        Some(b.c.administrateur),
    )
    .await;
    assert_eq!(r.statut, StatusCode::NOT_FOUND);
    let nom: String = sqlx::query_scalar("SELECT nom FROM tiers WHERE id = $1")
        .bind(f.tiers)
        .fetch_one(&b.env.admin)
        .await
        .unwrap();
    assert_eq!(nom, "Tiers fixe");
    // Dossier non affecté au collaborateur : absent.
    let r = lire(
        &routeur,
        &format!(
            "/api/cabinets/{}/dossiers/{}/tiers",
            b.c.cabinet, b.c.autre_dossier
        ),
        Some(b.c.collaborateur),
    )
    .await;
    assert_eq!(r.statut, StatusCode::NOT_FOUND);
}

/// Nombre de lignes visibles d'une table (nom constant, choisi par le test).
async fn compter(tx: &mut api::base::TransactionTenant, table: &str) -> i64 {
    let requete = match table {
        "tiers" => "SELECT count(*) FROM tiers",
        "tiers_identifiant" => "SELECT count(*) FROM tiers_identifiant",
        "tiers_adresse" => "SELECT count(*) FROM tiers_adresse",
        "tiers_contact" => "SELECT count(*) FROM tiers_contact",
        _ => "SELECT count(*) FROM tiers_compte_bancaire",
    };
    sqlx::query_scalar::<_, i64>(requete)
        .fetch_one(tx.connexion())
        .await
        .unwrap()
}

/// RLS forcée sur les tables de la fiche : hors contexte de dossier, rien n'est visible ; dans
/// le contexte d'un dossier, rien d'un autre ; une insertion pour un autre dossier est refusée,
/// une clé composite ne cite pas le tiers d'un autre dossier ; `UPDATE` et `DELETE` bruts sur un
/// autre dossier n'affectent rien.
#[tokio::test]
async fn rls_des_tables_de_la_fiche() {
    let env = env().await;
    let a = creer_cabinet(&env.admin, "A").await;
    let b = creer_cabinet(&env.admin, "B").await;
    let fa =
        commun::referentiel::doter(&env.application, a.cabinet, a.dossier, a.administrateur).await;
    let fb =
        commun::referentiel::doter(&env.application, b.cabinet, b.dossier, b.administrateur).await;
    // Une ligne de chaque table dans le dossier de B.
    for requete in [
        "INSERT INTO tiers_identifiant (cabinet_id, dossier_id, id, tiers_id, type, valeur) \
         VALUES ($1, $2, gen_random_uuid(), $3, 'RCCM', 'B-1')",
        "INSERT INTO tiers_adresse (cabinet_id, dossier_id, id, tiers_id, ville) \
         VALUES ($1, $2, gen_random_uuid(), $3, 'Ville B')",
        "INSERT INTO tiers_contact (cabinet_id, dossier_id, id, tiers_id, type, valeur) \
         VALUES ($1, $2, gen_random_uuid(), $3, 'telephone', '1')",
        "INSERT INTO tiers_compte_bancaire (cabinet_id, dossier_id, id, tiers_id, type, numero) \
         VALUES ($1, $2, gen_random_uuid(), $3, 'RIB', 'B-RIB')",
    ] {
        sqlx::query(requete)
            .bind(b.cabinet.uuid())
            .bind(b.dossier.uuid())
            .bind(fb.tiers)
            .execute(&env.admin)
            .await
            .unwrap();
    }
    let tables = [
        "tiers",
        "tiers_identifiant",
        "tiers_adresse",
        "tiers_contact",
        "tiers_compte_bancaire",
    ];
    // Contexte du cabinet A sans dossier : rien (les politiques exigent le dossier).
    let mut tx = ouvrir_transaction(
        &env.application,
        ContexteTenant::cabinet(a.cabinet, a.administrateur),
    )
    .await
    .unwrap();
    for table in tables {
        assert_eq!(compter(&mut tx, table).await, 0, "{table} sans dossier");
    }
    drop(tx);
    // Contexte du dossier de A : seul son tiers, rien de B.
    let mut tx = ouvrir_transaction(
        &env.application,
        ContexteTenant::dossier(a.cabinet, a.dossier, a.administrateur),
    )
    .await
    .unwrap();
    assert_eq!(compter(&mut tx, "tiers").await, 1);
    for table in &tables[1..] {
        assert_eq!(compter(&mut tx, table).await, 0, "{table} de B vu depuis A");
    }
    // UPDATE et DELETE bruts sur les lignes de B : aucune ligne touchée.
    for requete in [
        "UPDATE tiers_identifiant SET valeur = 'X' WHERE tiers_id = $1",
        "UPDATE tiers_compte_bancaire SET numero = 'X' WHERE tiers_id = $1",
        "DELETE FROM tiers_adresse WHERE tiers_id = $1",
        "DELETE FROM tiers_contact WHERE tiers_id = $1",
        "UPDATE tiers SET nom = 'X' WHERE id = $1",
    ] {
        let n = sqlx::query(requete)
            .bind(fb.tiers)
            .execute(tx.connexion())
            .await
            .unwrap()
            .rows_affected();
        assert_eq!(n, 0, "{requete}");
    }
    // Insertion pour le dossier de B : refusée par `WITH CHECK`.
    let erreur = sqlx::query(
        "INSERT INTO tiers_identifiant (cabinet_id, dossier_id, id, tiers_id, type, valeur) \
         VALUES ($1, $2, gen_random_uuid(), $3, 'AUTRE', 'X')",
    )
    .bind(b.cabinet.uuid())
    .bind(b.dossier.uuid())
    .bind(fb.tiers)
    .execute(tx.connexion())
    .await
    .unwrap_err();
    assert_eq!(code_sql(&erreur).as_deref(), Some("42501"));
    drop(tx);
    // Clé composite : une ligne du dossier de A ne cite pas le tiers de B.
    let mut tx = ouvrir_transaction(
        &env.application,
        ContexteTenant::dossier(a.cabinet, a.dossier, a.administrateur),
    )
    .await
    .unwrap();
    let erreur = sqlx::query(
        "INSERT INTO tiers_compte_bancaire (cabinet_id, dossier_id, id, tiers_id, type, numero) \
         VALUES ($1, $2, gen_random_uuid(), $3, 'RIB', 'X')",
    )
    .bind(a.cabinet.uuid())
    .bind(a.dossier.uuid())
    .bind(fb.tiers)
    .execute(tx.connexion())
    .await
    .unwrap_err();
    assert_eq!(code_sql(&erreur).as_deref(), Some("23503"));
    drop(tx);
    // Les lignes de B sont intactes ; le tiers de A n'a rien reçu.
    let n: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM tiers_identifiant WHERE tiers_id = $1 AND valeur = 'B-1'",
    )
    .bind(fb.tiers)
    .fetch_one(&env.admin)
    .await
    .unwrap();
    assert_eq!(n, 1);
    let n: i64 =
        sqlx::query_scalar("SELECT count(*) FROM tiers_compte_bancaire WHERE tiers_id = $1")
            .bind(fa.tiers)
            .fetch_one(&env.admin)
            .await
            .unwrap();
    assert_eq!(n, 0);
    // Le rôle applicatif ne supprime jamais un tiers, ni ne change son type, son compte
    // collectif ou son code.
    for (droit, colonne) in [
        ("DELETE", None),
        ("UPDATE", Some("type")),
        ("UPDATE", Some("compte_collectif")),
        ("UPDATE", Some("code")),
    ] {
        let a_le_droit: bool = match colonne {
            None => {
                sqlx::query_scalar("SELECT has_table_privilege('sigma_application', 'tiers', $1)")
                    .bind(droit)
                    .fetch_one(&env.admin)
                    .await
                    .unwrap()
            }
            Some(c) => sqlx::query_scalar(
                "SELECT has_column_privilege('sigma_application', 'tiers', $1, $2)",
            )
            .bind(c)
            .bind(droit)
            .fetch_one(&env.admin)
            .await
            .unwrap(),
        };
        assert!(!a_le_droit, "{droit} {colonne:?}");
    }
}

/// La migration `0500` numérote les tiers existants (`T` et six chiffres, par dossier, dans
/// l'ordre de création) : la forme que l'API reprend pour les tiers suivants.
#[tokio::test]
async fn le_code_tire_d_office_suit_les_codes_existants() {
    let b = banc().await;
    // Le tiers fixe a reçu `T000001` ; un code `T000041` saisi à la main déplace la suite.
    let fixe: String = sqlx::query_scalar("SELECT code FROM tiers WHERE id = $1")
        .bind(b.f.tiers)
        .fetch_one(&b.env.admin)
        .await
        .unwrap();
    assert_eq!(fixe, "T000001");
    creer_tiers(&b, "Manuel", r#","code":"T000041""#).await;
    assert_eq!(creer_tiers(&b, "Suivant", "").await["code"], "T000042");
    // Le code ne change jamais, même au désarchivage.
    let id = b.f.tiers;
    poster(
        &b,
        &format!("/{id}/archivage"),
        b.valideur,
        r#"{"motif":"x"}"#,
    )
    .await;
    let r = poster(&b, &format!("/{id}/desarchivage"), b.valideur, "{}").await;
    assert_eq!(r.json()["code"], "T000001");
}
