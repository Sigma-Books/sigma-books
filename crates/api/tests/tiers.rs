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
    // Valeurs mal formées : 422 ; code (immuable), `actif` ou corps vide : 400.
    for corps in [
        r#"{"devise":"euro"}"#,
        r#"{"adresses":[{"pays":"Sénégal"}]}"#,
        r#"{"contacts":[{"type":"pigeon","valeur":"x"}]}"#,
    ] {
        let r = modifier(corps).await;
        assert_eq!(r.statut, StatusCode::UNPROCESSABLE_ENTITY, "{corps}");
        assert_eq!(code(&r), "tiers_valeur_invalide");
    }
    for corps in [r#"{"code":"X"}"#, r#"{"actif":false}"#, "{}"] {
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
    drop(tx);
    // Insertion pour le dossier de B, depuis le contexte de A : refusée par `WITH CHECK`, pour
    // chaque table de la fiche (une transaction par essai : l'échec l'annule).
    for requete in [
        "INSERT INTO tiers (cabinet_id, dossier_id, id, code, nom, type, compte_collectif, actif) \
         VALUES ($1, $2, gen_random_uuid(), 'CROISE', 'X', 'client', $4, true)",
        "INSERT INTO tiers_identifiant (cabinet_id, dossier_id, id, tiers_id, type, valeur) \
         VALUES ($1, $2, gen_random_uuid(), $3, 'AUTRE', 'X')",
        "INSERT INTO tiers_adresse (cabinet_id, dossier_id, id, tiers_id, ville) \
         VALUES ($1, $2, gen_random_uuid(), $3, 'X')",
        "INSERT INTO tiers_contact (cabinet_id, dossier_id, id, tiers_id, type, valeur) \
         VALUES ($1, $2, gen_random_uuid(), $3, 'autre', 'X')",
        "INSERT INTO tiers_compte_bancaire (cabinet_id, dossier_id, id, tiers_id, type, numero) \
         VALUES ($1, $2, gen_random_uuid(), $3, 'RIB', 'X')",
    ] {
        let mut tx = ouvrir_transaction(
            &env.application,
            ContexteTenant::dossier(a.cabinet, a.dossier, a.administrateur),
        )
        .await
        .unwrap();
        let erreur = sqlx::query(requete)
            .bind(b.cabinet.uuid())
            .bind(b.dossier.uuid())
            .bind(fb.tiers)
            .bind(comptes().collectif.as_str())
            .execute(tx.connexion())
            .await
            .unwrap_err();
        assert_eq!(code_sql(&erreur).as_deref(), Some("42501"), "{requete}");
    }
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
    // Le rôle applicatif ne supprime jamais un tiers ni ne change son code (le type et le compte
    // collectif changent avant le premier mouvement : contrôle de l'API).
    for (droit, colonne) in [("DELETE", None), ("UPDATE", Some("code"))] {
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

/// Type et compte collectif : modifiables tant qu'aucune ligne ne cite le tiers, sous le contrôle
/// croisé du référentiel ; figés dès la première ligne (ADR 0031 point 1, amendement du
/// 2026-11-27) ; changement journalisé par les noms de champs.
#[tokio::test]
async fn classement_modifiable_jusqu_au_premier_mouvement() {
    let b = banc().await;
    let id = b.f.tiers;
    let k = comptes();
    let rang = rang_audit(&b).await;
    let modifier = |corps: String| {
        let b = &b;
        async move { poster(b, &format!("/{id}/modification"), b.c.collaborateur, &corps).await }
    };
    // Paire incohérente (un client sur un compte de la fiche 40) : refusée, rien ne change.
    let r = modifier(format!(r#"{{"compte_collectif":"{}"}}"#, k.fournisseur)).await;
    assert_eq!(r.statut, StatusCode::BAD_REQUEST, "{}", r.texte());
    // Correction d'un tiers créé avec le mauvais rôle : admise avant tout mouvement.
    let r = modifier(format!(
        r#"{{"type":"fournisseur","compte_collectif":"{}"}}"#,
        k.fournisseur
    ))
    .await;
    assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
    assert_eq!(r.json()["type"], "fournisseur");
    assert_eq!(r.json()["compte_collectif"], k.fournisseur.to_string());
    let tout = audit_depuis(&b, rang).await.join("\n");
    assert!(
        tout.contains("donnee=operation\nvaleur=classement\n")
            && tout.contains("donnee=champs\nvaleur=type,compte_collectif\n"),
        "{tout}"
    );
    // Une ligne de brouillon cite le tiers : le classement est figé (409), le nom reste libre.
    b.creer(&contenu(
        &journal_de_saisie(),
        &texte_date(aujourd_hui()),
        &[
            ligne(commun::banc::caisse(), "debit", "1000"),
            format!(
                r#"{{"compte":"{}","sens":"credit","montant":"1000","tiers":"{id}"}}"#,
                k.fournisseur
            ),
        ],
        "",
    ))
    .await;
    let r = modifier(format!(
        r#"{{"type":"client","compte_collectif":"{}"}}"#,
        k.collectif
    ))
    .await;
    assert_eq!(r.statut, StatusCode::CONFLICT, "{}", r.texte());
    assert_eq!(code(&r), "tiers_deja_mouvemente");
    assert_eq!(
        modifier(r#"{"nom":"Fournisseur renommé"}"#.to_owned())
            .await
            .statut,
        StatusCode::OK
    );
    // Demande identique à l'état : aucun changement, même figé.
    let r = modifier(format!(r#"{{"compte_collectif":"{}"}}"#, k.fournisseur)).await;
    assert_eq!(code(&r), "aucun_changement");
}

/// Tout caractère de contrôle (C0, C1) ou de format Unicode (`Cf`) est refusé dans chaque texte de
/// la fiche (F11.8), avec un code qui ne le cite pas ; la comparaison des doublons ignore les
/// blancs, espaces insécables et caractères de format (un numéro qui n'en diffère que par eux
/// est signalé).
#[tokio::test]
async fn caracteres_de_controle_et_de_format_refuses_doublons_normalises() {
    let b = banc().await;
    let id = b.f.tiers;
    let interdits = [
        "\\u200B", "\\u200D", "\\u2060", "\\uFEFF", "\\u00AD", "\\u061C", "\\u202E", "\\u0007",
        "\\u0085",
    ];
    for c in interdits {
        for corps in [
            format!(r#"{{"nom":"A{c}B"}}"#),
            format!(r#"{{"adresses":[{{"rue":"1{c} rue"}}]}}"#),
            format!(r#"{{"contacts":[{{"type":"telephone","valeur":"33{c}1"}}]}}"#),
            format!(r#"{{"identifiants":[{{"type":"RCCM","valeur":"SN{c}1"}}]}}"#),
        ] {
            let r = poster(
                &b,
                &format!("/{id}/modification"),
                b.c.collaborateur,
                &corps,
            )
            .await;
            assert_eq!(r.statut, StatusCode::UNPROCESSABLE_ENTITY, "{corps}");
            assert_eq!(code(&r), "tiers_caractere_invalide", "{corps}");
            assert!(!r.texte().contains('\u{200B}'));
        }
        let r = poster(
            &b,
            &format!("/{id}/comptes-bancaires"),
            b.c.collaborateur,
            &format!(r#"{{"comptes":[{{"type":"RIB","numero":"SN01{c}2345"}}]}}"#),
        )
        .await;
        assert_eq!(code(&r), "tiers_caractere_invalide", "{c}");
    }
    // L'espace insécable est admis à la saisie, ignoré à la comparaison.
    let autre = creer_tiers(&b, "Autre", "").await["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let r = poster(
        &b,
        &format!("/{id}/comptes-bancaires"),
        b.c.collaborateur,
        r#"{"comptes":[{"type":"RIB","numero":"SN01 2345"}]}"#,
    )
    .await;
    assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
    let r = poster(
        &b,
        &format!("/{autre}/comptes-bancaires"),
        b.c.collaborateur,
        "{\"comptes\":[{\"type\":\"RIB\",\"numero\":\"sn01\\u00A02345\"}]}",
    )
    .await;
    assert_eq!(
        r.json()["avertissements"][0]["code"],
        "compte_bancaire_doublon"
    );
    // Un numéro inscrit hors de l'API avec un caractère invisible reste signalé.
    let troisieme = creer_tiers(&b, "Troisième", "").await["id"]
        .as_str()
        .unwrap()
        .to_owned();
    sqlx::query(
        "INSERT INTO tiers_compte_bancaire (cabinet_id, dossier_id, id, tiers_id, type, numero) \
         VALUES ($1, $2, gen_random_uuid(), $3::uuid, 'RIB', $4)",
    )
    .bind(b.c.cabinet.uuid())
    .bind(b.c.dossier.uuid())
    .bind(&troisieme)
    .bind("SN01\u{200B}23\u{FEFF}45")
    .execute(&b.env.admin)
    .await
    .unwrap();
    let r = lire(
        &b.routeur,
        &format!("{}/{troisieme}/comptes-bancaires", base_tiers(&b.c)),
        Some(b.c.collaborateur),
    )
    .await;
    let autres = &r.json()["avertissements"][0]["tiers"];
    assert_eq!(autres.as_array().map(Vec::len), Some(2), "{}", r.texte());
}

/// Bornes de la fiche (limites de sécurité, F2.16) : un code par limite dépassée.
#[tokio::test]
async fn limites_de_la_fiche_nommees() {
    let b = banc().await;
    let id = b.f.tiers;
    let liste = |n: usize, element: &str| vec![element; n].join(",");
    let identifiants = (0..11)
        .map(|i| format!(r#"{{"type":"T{i}","valeur":"1"}}"#))
        .collect::<Vec<_>>()
        .join(",");
    for (corps, attendu) in [
        (
            format!(r#"{{"identifiants":[{identifiants}]}}"#),
            "tiers_trop_d_identifiants",
        ),
        (
            format!(r#"{{"adresses":[{}]}}"#, liste(21, r#"{"ville":"V"}"#)),
            "tiers_trop_d_adresses",
        ),
        (
            format!(
                r#"{{"contacts":[{}]}}"#,
                liste(51, r#"{"type":"autre","valeur":"x"}"#)
            ),
            "tiers_trop_de_contacts",
        ),
    ] {
        let r = poster(
            &b,
            &format!("/{id}/modification"),
            b.c.collaborateur,
            &corps,
        )
        .await;
        assert_eq!(code(&r), attendu);
    }
    let r = poster(
        &b,
        &format!("/{id}/comptes-bancaires"),
        b.c.collaborateur,
        &format!(
            r#"{{"comptes":[{}]}}"#,
            liste(21, r#"{"type":"RIB","numero":"1"}"#)
        ),
    )
    .await;
    assert_eq!(code(&r), "tiers_trop_de_comptes_bancaires");
}

/// Un code saisi qui épuise la suite (`T` et 18 neuf) ne fait jamais tirer un code hors de la
/// suite ni un doublon (500) : 409 typé, et un code saisi reste possible.
#[tokio::test]
async fn tirage_du_code_borne() {
    let b = banc().await;
    creer_tiers(&b, "Dernier", r#","code":"T999999999999999999""#).await;
    let corps = |extra: &str| {
        format!(
            r#"{{"nom":"X","type":"client","compte_collectif":"{}"{extra}}}"#,
            comptes().collectif
        )
    };
    for _ in 0..2 {
        let r = poster(&b, "", b.c.administrateur, &corps("")).await;
        assert_eq!(r.statut, StatusCode::CONFLICT, "{}", r.texte());
        assert_eq!(code(&r), "tiers_code_tirage_impossible");
    }
    let r = poster(&b, "", b.c.administrateur, &corps(r#","code":"SAISI-1""#)).await;
    assert_eq!(r.statut, StatusCode::CREATED);
}

/// Appartenance suspendue : plus rien, ni lecture ni mutation (la requête relit le rôle actif).
#[tokio::test]
async fn appartenance_suspendue_refusee() {
    let b = banc().await;
    sqlx::query("UPDATE appartenance SET actif = false WHERE utilisateur_id = $1")
        .bind(b.c.collaborateur.uuid())
        .execute(&b.env.admin)
        .await
        .unwrap();
    let id = b.f.tiers;
    for r in [
        lire(&b.routeur, &base_tiers(&b.c), Some(b.c.collaborateur)).await,
        lire(
            &b.routeur,
            &format!("{}/{id}", base_tiers(&b.c)),
            Some(b.c.collaborateur),
        )
        .await,
        poster(
            &b,
            &format!("/{id}/modification"),
            b.c.collaborateur,
            r#"{"nom":"X"}"#,
        )
        .await,
    ] {
        assert!(
            [StatusCode::FORBIDDEN, StatusCode::NOT_FOUND].contains(&r.statut),
            "{}",
            r.texte()
        );
    }
    let nom: String = sqlx::query_scalar("SELECT nom FROM tiers WHERE id = $1")
        .bind(id)
        .fetch_one(&b.env.admin)
        .await
        .unwrap();
    assert_eq!(nom, "Tiers fixe");
}

/// Numérotation des tiers existants par la migration `0500` : rejouée telle quelle (texte de la
/// migration) sur une table temporaire `tiers` qui masque la vraie dans la session : plusieurs
/// tiers dans un dossier, deux dossiers, ordre de création puis identifiant.
#[tokio::test]
async fn numerotation_des_tiers_existants_par_la_migration() {
    let env = env().await;
    let migration = include_str!("../migrations/0500_tiers_fiche.sql");
    let debut = migration.find("UPDATE tiers t SET code").unwrap();
    let fin = debut + migration[debut..].find(';').unwrap() + 1;
    let numerotation = &migration[debut..fin];
    let mut cx = env.admin.acquire().await.unwrap();
    sqlx::query(
        "CREATE TEMP TABLE tiers (cabinet_id uuid, dossier_id uuid, id uuid, cree_le timestamptz, \
         code text)",
    )
    .execute(&mut *cx)
    .await
    .unwrap();
    let (cabinet, d1, d2) = (
        "00000000-0000-4000-8000-000000000001",
        "00000000-0000-4000-8000-0000000000d1",
        "00000000-0000-4000-8000-0000000000d2",
    );
    // Dossier 1 : trois tiers (deux au même instant, départagés par l'identifiant) ; dossier 2 :
    // deux tiers.
    for (dossier, id, instant) in [
        (d1, "00000000-0000-4000-8000-0000000000a2", "2026-01-02"),
        (d1, "00000000-0000-4000-8000-0000000000a1", "2026-01-03"),
        (d1, "00000000-0000-4000-8000-0000000000a0", "2026-01-03"),
        (d2, "00000000-0000-4000-8000-0000000000b9", "2026-01-01"),
        (d2, "00000000-0000-4000-8000-0000000000b1", "2026-02-01"),
    ] {
        sqlx::query(
            "INSERT INTO tiers (cabinet_id, dossier_id, id, cree_le) \
             VALUES ($1::uuid, $2::uuid, $3::uuid, $4::timestamptz)",
        )
        .bind(cabinet)
        .bind(dossier)
        .bind(id)
        .bind(instant)
        .execute(&mut *cx)
        .await
        .unwrap();
    }
    sqlx::query(sqlx::AssertSqlSafe(numerotation.to_owned()))
        .execute(&mut *cx)
        .await
        .unwrap();
    let codes: Vec<(String, String)> =
        sqlx::query_as("SELECT id::text, code FROM tiers ORDER BY dossier_id, code")
            .fetch_all(&mut *cx)
            .await
            .unwrap();
    let attendu = [
        ("00000000-0000-4000-8000-0000000000a2", "T000001"),
        ("00000000-0000-4000-8000-0000000000a0", "T000002"),
        ("00000000-0000-4000-8000-0000000000a1", "T000003"),
        ("00000000-0000-4000-8000-0000000000b9", "T000001"),
        ("00000000-0000-4000-8000-0000000000b1", "T000002"),
    ]
    .map(|(i, c)| (i.to_owned(), c.to_owned()));
    assert_eq!(codes, attendu);
    sqlx::query("DROP TABLE pg_temp.tiers")
        .execute(&mut *cx)
        .await
        .unwrap();
}
