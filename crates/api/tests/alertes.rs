//! Alertes par courriel (PR 9c de `docs/lots/L2.md` ; ADR 0012, 0022, 0025 ; #157, F6.2, F7.4,
//! F1.12, F1.15, question 9b) : exports d'audit et d'intégrité, retard d'horodatage, témoin remis
//! au cabinet, étapes d'une réinitialisation, changements de droits, séparation des tâches
//! désactivée, connexion par un code de récupération. Chaque alerte est créée **dans la
//! transaction de l'action**, composée à l'envoi, et part vers les administrateurs actifs **du
//! seul cabinet concerné**.
#![cfg(feature = "tests-postgres")]
// Fichier de test : les `expect`/`unwrap` y sont voulus.
#![allow(clippy::expect_used, clippy::unwrap_used)]

mod commun;

use std::sync::Arc;

use api::base::session::NatureSession;
use api::base::taches::{SujetNotification, notifier};
use api::base::{ContexteTenant, IdUtilisateur, ouvrir_transaction};
use api::envoi::tache::{Expediteur, Reglages};
use api::envoi::transport::ErreurEnvoi;
use authz::Autorisation;
use axum::Router;
use axum::http::StatusCode;
use commun::envoi::{TransportMemoire, jeton_du_message};
use commun::facteurs::{connecter, corps_code, courriel, installer_codes};
use commun::serveur::{
    HorlogeTest, ORIGINE, Reponse, appeler, appeler_session_corps, application_sessions_avec,
    cookie_de, ouvrir, reglages_de_test, routeur_de,
};
use commun::{CabinetTest, ajouter_affecte, creer_cabinet, env};
use serde_json::json;
use time::Duration;
use uuid::Uuid;

/// La file est unique pour l'instance : voir `envoi.rs`.
static FILE: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

type Session = (String, String);

struct Banc {
    _verrou: tokio::sync::MutexGuard<'static, ()>,
    env: commun::Env,
    horloge: HorlogeTest,
    routeur: Router,
    c: CabinetTest,
    transport: TransportMemoire,
    expediteur: Expediteur,
}

async fn banc() -> Banc {
    let verrou = FILE.lock().await;
    let env = env().await;
    sqlx::query(
        "UPDATE tache SET etat = 'terminee', termine_le = now(), reserve_jusqua = NULL, prise = NULL \
         WHERE nature IN ('courriel', 'notification') AND etat <> 'terminee'",
    )
    .execute(&env.admin)
    .await
    .unwrap();
    let c = creer_cabinet(&env.admin, "Cabinet").await;
    let horloge = HorlogeTest::default();
    let mut reglages = reglages_de_test();
    for debit in [
        &mut reglages.facteurs.debit_connexion_adresse,
        &mut reglages.facteurs.debit_connexion_courriel,
        &mut reglages.facteurs.debit_verification,
    ] {
        debit.capacite = 10_000;
    }
    reglages.debit_adresse.capacite = 20_000;
    reglages.debit_utilisateur.capacite = 20_000;
    let etat = application_sessions_avec(
        &env,
        &horloge,
        Arc::new(Autorisation::charger().unwrap()),
        reglages,
    );
    let transport = TransportMemoire::default();
    let expediteur = Expediteur::new(
        env.application.clone(),
        Arc::new(transport.clone()),
        Arc::new(horloge.clone()),
        ORIGINE.to_owned(),
        Reglages::default(),
    );
    Banc {
        _verrou: verrou,
        env,
        horloge,
        routeur: routeur_de(&etat),
        c,
        transport,
        expediteur,
    }
}

impl Banc {
    async fn session(&self, u: IdUtilisateur) -> Session {
        let o = ouvrir(&self.env, &self.horloge, u, NatureSession::Complete).await;
        (cookie_de(&o), o.jeton.csrf())
    }

    async fn appel(&self, methode: &str, chemin: &str, s: &Session, corps: &str) -> Reponse {
        appeler_session_corps(
            &self.routeur,
            methode,
            chemin,
            Some(&s.0),
            Some(&s.1),
            corps,
        )
        .await
    }

    fn chemin(&self, suite: &str) -> String {
        format!("/api/cabinets/{}{suite}", self.c.cabinet)
    }

    /// Passe la file et rend les destinataires des messages remis, triés.
    async fn passer(&self) -> Vec<String> {
        self.expediteur.passer().await.unwrap();
        let mut d: Vec<String> = self
            .transport
            .remis()
            .into_iter()
            .map(|m| m.destinataire)
            .collect();
        d.sort();
        d
    }

    async fn utiliser_lien(&self, jeton: &str) -> Reponse {
        let requete = axum::http::Request::post("/api/liens/utilisation")
            .header("origin", ORIGINE)
            .header("sec-fetch-site", "same-origin")
            .header("x-sigma-csrf", "1")
            .header("content-type", "application/json")
            .body(axum::body::Body::from(
                json!({ "jeton": jeton }).to_string(),
            ))
            .unwrap();
        appeler(&self.routeur, requete).await
    }
}

fn attendus(utilisateurs: &[IdUtilisateur]) -> Vec<String> {
    let mut v = Vec::new();
    for u in utilisateurs {
        v.push(courriel(*u));
    }
    v.sort();
    v
}

#[tokio::test]
async fn un_export_d_audit_alerte_tous_les_administrateurs_du_cabinet_et_eux_seuls() {
    let b = banc().await;
    let second = ajouter_affecte(&b.env.admin, &b.c, "administrateur").await;
    // Un autre cabinet, avec son administrateur : il ne reçoit rien (isolation).
    let autre = creer_cabinet(&b.env.admin, "Autre").await;
    let admin = b.session(b.c.administrateur).await;
    let r = b
        .appel("POST", &b.chemin("/audit/export"), &admin, "")
        .await;
    assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
    // Rien ne part avant le passage de la file.
    assert!(b.transport.vide());
    let destinataires = b.passer().await;
    assert_eq!(destinataires, attendus(&[b.c.administrateur, second]));
    assert!(!destinataires.contains(&courriel(autre.administrateur)));
    assert!(!destinataires.contains(&courriel(b.c.collaborateur)));
    let m = &b.transport.remis()[0];
    assert_eq!(m.sujet, "Sigma Books : export de l'audit du cabinet");
    // Aucune donnée comptable, aucun jeton, aucun nom d'autrui.
    assert!(!m.texte.contains("lien#"));
    // Un second passage n'envoie plus rien (tâche terminée).
    b.expediteur.passer().await.unwrap();
    assert_eq!(b.transport.remis().len(), 2);
}

#[tokio::test]
async fn l_export_d_audit_d_un_dossier_cite_le_dossier_et_alerte() {
    let b = banc().await;
    let admin = b.session(b.c.administrateur).await;
    let r = b
        .appel(
            "POST",
            &b.chemin(&format!("/dossiers/{}/audit/export", b.c.dossier)),
            &admin,
            "",
        )
        .await;
    assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
    let d = b.passer().await;
    assert_eq!(d, attendus(&[b.c.administrateur]));
    let m = &b.transport.remis()[0];
    assert_eq!(m.sujet, "Sigma Books : export de l'audit d'un dossier");
    assert!(m.texte.contains(&b.c.dossier.to_string()));
}

#[tokio::test]
async fn un_changement_de_droits_alerte_les_administrateurs() {
    let b = banc().await;
    let second = ajouter_affecte(&b.env.admin, &b.c, "administrateur").await;
    let admin = b.session(b.c.administrateur).await;
    let r = b
        .appel(
            "POST",
            &b.chemin(&format!("/membres/{}/role", b.c.collaborateur)),
            &admin,
            &json!({"role": "valideur"}).to_string(),
        )
        .await;
    assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
    assert_eq!(b.passer().await, attendus(&[b.c.administrateur, second]));
    assert_eq!(
        b.transport.remis()[0].sujet,
        "Sigma Books : droits modifiés dans votre cabinet"
    );
}

#[tokio::test]
async fn desactiver_la_separation_des_taches_alerte() {
    let b = banc().await;
    let admin = b.session(b.c.administrateur).await;
    assert_eq!(
        b.appel("DELETE", &b.chemin("/separation-taches"), &admin, "")
            .await
            .statut,
        StatusCode::OK
    );
    assert_eq!(b.passer().await, attendus(&[b.c.administrateur]));
    assert_eq!(
        b.transport.remis()[0].sujet,
        "Sigma Books : séparation des tâches désactivée"
    );
}

#[tokio::test]
async fn une_alerte_refusee_par_une_action_annulee_n_existe_pas() {
    let b = banc().await;
    let admin = b.session(b.c.administrateur).await;
    // Rôle inconnu : 400, aucune alerte.
    let r = b
        .appel(
            "POST",
            &b.chemin(&format!("/membres/{}/role", b.c.collaborateur)),
            &admin,
            &json!({"role": "root"}).to_string(),
        )
        .await;
    assert_eq!(r.statut, StatusCode::BAD_REQUEST);
    // Soi-même : 409, aucune alerte.
    b.appel(
        "POST",
        &b.chemin(&format!("/membres/{}/suspension", b.c.administrateur)),
        &admin,
        "",
    )
    .await;
    assert_eq!(b.passer().await, Vec::<String>::new());
}

#[tokio::test]
async fn la_reinitialisation_alerte_a_chaque_etape_y_compris_le_membre_concerne() {
    let b = banc().await;
    let second = ajouter_affecte(&b.env.admin, &b.c, "administrateur").await;
    let cible = b.c.collaborateur;
    let admin = b.session(b.c.administrateur).await;
    let r = b
        .appel(
            "POST",
            &b.chemin(&format!("/membres/{cible}/reinitialisation")),
            &admin,
            &json!({"motif_code": "autre", "motif": "Appel de vérification", "verification_hors_bande": true}).to_string(),
        )
        .await;
    assert_eq!(r.statut, StatusCode::CREATED, "{}", r.texte());
    let id = r.json()["id"].as_str().unwrap().to_owned();
    // Demande : les deux administrateurs et le membre concerné (texte propre à lui).
    assert_eq!(
        b.passer().await,
        attendus(&[b.c.administrateur, second, cible])
    );
    let remis = b.transport.remis();
    let a_lui = remis
        .iter()
        .find(|m| m.destinataire == courriel(cible))
        .unwrap();
    assert_eq!(
        a_lui.sujet,
        "Sigma Books : réinitialisation de votre accès demandée"
    );
    assert!(a_lui.texte.contains("contestez"));
    let aux_admins = remis
        .iter()
        .find(|m| m.destinataire == courriel(second))
        .unwrap();
    assert_eq!(
        aux_admins.sujet,
        "Sigma Books : réinitialisation d'un accès demandée"
    );
    // Le membre conteste lui-même : alerte d'annulation.
    let s = b.session(cible).await;
    let r = b
        .appel(
            "POST",
            &b.chemin(&format!("/reinitialisations/{id}/contestation")),
            &s,
            "",
        )
        .await;
    assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
    b.expediteur.passer().await.unwrap();
    let annulations = b
        .transport
        .remis()
        .iter()
        .filter(|m| m.sujet == "Sigma Books : réinitialisation d'un accès annulée")
        .count();
    assert_eq!(annulations, 3);
}

#[tokio::test]
async fn seul_le_membre_concerne_conteste_sa_reinitialisation() {
    let b = banc().await;
    let admin = b.session(b.c.administrateur).await;
    let autre_membre = ajouter_affecte(&b.env.admin, &b.c, "auditeur").await;
    let r = b
        .appel(
            "POST",
            &b.chemin(&format!("/membres/{}/reinitialisation", b.c.collaborateur)),
            &admin,
            &json!({"motif_code": "autre", "motif": "Appel de vérification", "verification_hors_bande": true}).to_string(),
        )
        .await;
    let id = r.json()["id"].as_str().unwrap().to_owned();
    let chemin = b.chemin(&format!("/reinitialisations/{id}/contestation"));
    // Un autre membre (même administrateur) : 404, comme une réinitialisation absente.
    let s = b.session(autre_membre).await;
    let r1 = b.appel("POST", &chemin, &s, "").await;
    let r2 = b
        .appel(
            "POST",
            &b.chemin(&format!(
                "/reinitialisations/{}/contestation",
                Uuid::new_v4()
            )),
            &s,
            "",
        )
        .await;
    assert_eq!(
        (r1.statut, r2.statut),
        (StatusCode::NOT_FOUND, StatusCode::NOT_FOUND)
    );
    assert_eq!(r1.corps, r2.corps);
    assert_eq!(
        b.appel("POST", &chemin, &admin, "").await.statut,
        StatusCode::NOT_FOUND
    );
    // Un membre d'un autre cabinet : 404 aussi.
    let autre = creer_cabinet(&b.env.admin, "Autre").await;
    let sa = b.session(autre.administrateur).await;
    assert_eq!(
        b.appel("POST", &chemin, &sa, "").await.statut,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn l_usage_du_lien_de_reinitialisation_alerte() {
    let b = banc().await;
    let cible = b.c.collaborateur;
    let admin = b.session(b.c.administrateur).await;
    let r = b
        .appel(
            "POST",
            &b.chemin(&format!("/membres/{cible}/reinitialisation")),
            &admin,
            &json!({"motif_code": "autre", "motif": "Appel de vérification", "verification_hors_bande": true}).to_string(),
        )
        .await;
    assert_eq!(r.statut, StatusCode::CREATED);
    b.expediteur.passer().await.unwrap();
    let avant = b.transport.remis().len();
    b.horloge
        .avancer(Duration::hours(24) + Duration::minutes(1));
    b.expediteur.passer().await.unwrap();
    let lien = b
        .transport
        .remis()
        .iter()
        .find(|m| m.sujet == api::envoi::modele::SUJET_REINITIALISATION)
        .map(jeton_du_message)
        .unwrap();
    assert_eq!(b.transport.remis().len(), avant + 1);
    assert_eq!(b.utiliser_lien(&lien).await.statut, StatusCode::OK);
    b.expediteur.passer().await.unwrap();
    let vers = |sujet: &str| -> Vec<String> {
        b.transport
            .remis()
            .into_iter()
            .filter(|m| m.sujet == sujet)
            .map(|m| m.destinataire)
            .collect()
    };
    // Les administrateurs (texte propre aux administrateurs) et le membre concerné, une fois,
    // avec le texte qui lui dit que son accès vient d'être refait.
    assert_eq!(
        vers("Sigma Books : accès d'un membre réinitialisé"),
        vec![courriel(b.c.administrateur)]
    );
    assert_eq!(
        vers("Sigma Books : votre accès a été réinitialisé"),
        vec![courriel(cible)]
    );
}

#[tokio::test]
async fn un_code_de_recuperation_utilise_alerte_les_administrateurs() {
    let b = banc().await;
    let cible = b.c.collaborateur;
    let codes = installer_codes(&b.env.admin, cible).await;
    let r = connecter(
        &b.routeur,
        "/api/connexion/code-recuperation",
        &corps_code(&courriel(cible), &codes[0]),
        "Navigateur de test",
    )
    .await;
    assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
    assert_eq!(b.passer().await, attendus(&[b.c.administrateur]));
    assert_eq!(
        b.transport.remis()[0].sujet,
        "Sigma Books : code de récupération utilisé"
    );
}

#[tokio::test]
async fn le_retard_d_horodatage_et_le_temoin_sont_remis_au_cabinet() {
    let b = banc().await;
    commun::referentiel::doter(&b.env.admin, b.c.cabinet, b.c.dossier, b.c.administrateur).await;
    let validee = commun::brouillard::doter_validee(
        &b.env.admin,
        b.c.cabinet,
        b.c.dossier,
        b.c.administrateur,
    )
    .await;
    let _ = validee;
    // Tête de chaîne du dossier (la fixture n'en écrit pas) : 1 écriture, empreinte quelconque.
    sqlx::query(
        "INSERT INTO chaine_tete (cabinet_id, dossier_id, rang, empreinte) \
         VALUES ($1, $2, 1, decode(repeat('ab', 32), 'hex'))",
    )
    .bind(b.c.cabinet.uuid())
    .bind(b.c.dossier.uuid())
    .execute(&b.env.admin)
    .await
    .unwrap();
    let mut tx = ouvrir_transaction(
        &b.env.application,
        ContexteTenant::dossier(b.c.cabinet, b.c.dossier, b.c.administrateur),
    )
    .await
    .unwrap();
    notifier(&mut tx, SujetNotification::RetardHorodatage, None)
        .await
        .unwrap();
    notifier(&mut tx, SujetNotification::Temoin, None)
        .await
        .unwrap();
    tx.confirmer().await.unwrap();
    assert_eq!(b.passer().await.len(), 2);
    let remis = b.transport.remis();
    let retard = remis
        .iter()
        .find(|m| m.sujet == "Sigma Books : horodatage en retard")
        .unwrap();
    assert!(retard.texte.contains(&b.c.dossier.to_string()));
    let temoin = remis
        .iter()
        .find(|m| m.sujet == "Sigma Books : témoin de la chaîne d'un dossier")
        .unwrap();
    assert!(
        temoin.texte.contains("Écritures chaînées : 1"),
        "{}",
        temoin.texte
    );
    assert!(
        temoin
            .texte
            .contains(&format!("Empreinte de tête : {}", "ab".repeat(32)))
    );
    assert!(
        temoin
            .texte
            .contains("Aucun jeton d'horodatage n'est encore obtenu")
    );
    // Des empreintes et des nombres, rien d'autre.
    assert!(!temoin.texte.contains("FCFA"));
}

#[tokio::test]
async fn un_echec_provisoire_rejoue_l_alerte() {
    let b = banc().await;
    let admin = b.session(b.c.administrateur).await;
    b.transport.programmer([Err(ErreurEnvoi::Temporaire)]);
    b.appel("POST", &b.chemin("/audit/export"), &admin, "")
        .await;
    assert_eq!(b.passer().await, Vec::<String>::new());
    let (etat, issue): (String, Option<String>) = sqlx::query_as(
        "SELECT etat, derniere_issue FROM tache WHERE nature = 'notification' AND cabinet_cible = $1",
    )
    .bind(b.c.cabinet.uuid())
    .fetch_one(&b.env.admin)
    .await
    .unwrap();
    assert_eq!(
        (etat.as_str(), issue.as_deref()),
        ("en_attente", Some("smtp_temporaire"))
    );
    b.horloge.avancer(Duration::seconds(61));
    assert_eq!(b.passer().await.len(), 1);
}

#[tokio::test]
async fn l_administrateur_vise_par_un_autre_administrateur_est_prevenu_et_peut_contester() {
    let b = banc().await;
    let second = ajouter_affecte(&b.env.admin, &b.c, "administrateur").await;
    let s1 = b.session(b.c.administrateur).await;
    let r = b
        .appel(
            "POST",
            &b.chemin(&format!("/membres/{second}/reinitialisation")),
            &s1,
            &json!({"motif_code": "compte_compromis", "motif": "Appel de vérification", "verification_hors_bande": true}).to_string(),
        )
        .await;
    assert_eq!(r.statut, StatusCode::CREATED, "{}", r.texte());
    let id = r.json()["id"].as_str().unwrap().to_owned();
    b.expediteur.passer().await.unwrap();
    let remis = b.transport.remis();
    // L'administrateur visé reçoit le texte qui lui est propre (une seule fois), l'autre celui des
    // administrateurs.
    let au_vise: Vec<_> = remis
        .iter()
        .filter(|m| m.destinataire == courriel(second))
        .collect();
    assert_eq!(au_vise.len(), 1);
    assert_eq!(
        au_vise[0].sujet,
        "Sigma Books : réinitialisation de votre accès demandée"
    );
    // Il conteste sa propre réinitialisation, par sa route de membre (non d'administration).
    let s2 = b.session(second).await;
    let r = b
        .appel(
            "POST",
            &b.chemin(&format!("/reinitialisations/{id}/contestation")),
            &s2,
            "",
        )
        .await;
    assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
    let etat: String = sqlx::query_scalar("SELECT etat FROM reinitialisation WHERE id = $1::uuid")
        .bind(&id)
        .fetch_one(&b.env.admin)
        .await
        .unwrap();
    assert_eq!(etat, "annulee");
}

#[tokio::test]
async fn les_alertes_d_un_sujet_sont_plafonnees_par_cabinet_sur_24_heures() {
    let b = banc().await;
    let admin = b.session(b.c.administrateur).await;
    // Dix exports déjà alertés aujourd'hui dans ce cabinet.
    for _ in 0..10 {
        sqlx::query(
            "INSERT INTO tache (id, nature, sujet, cabinet_cible, etat, cree_le, prochaine_tentative, \
                                termine_le, derniere_issue) \
             VALUES ($1, 'notification', 'export_audit_cabinet', $2, 'terminee', now(), now(), now(), 'envoye')",
        )
        .bind(Uuid::new_v4())
        .bind(b.c.cabinet.uuid())
        .execute(&b.env.admin)
        .await
        .unwrap();
    }
    b.appel("POST", &b.chemin("/audit/export"), &admin, "")
        .await;
    assert_eq!(b.passer().await, Vec::<String>::new());
    let issue: Option<String> = sqlx::query_scalar(
        "SELECT derniere_issue FROM tache WHERE nature = 'notification' AND cabinet_cible = $1 \
         AND etat = 'terminee' AND derniere_issue <> 'envoye'",
    )
    .bind(b.c.cabinet.uuid())
    .fetch_one(&b.env.admin)
    .await
    .unwrap();
    assert_eq!(issue.as_deref(), Some("plafond"));
    // Un autre sujet n'est pas concerné.
    b.appel("DELETE", &b.chemin("/separation-taches"), &admin, "")
        .await;
    assert_eq!(b.passer().await.len(), 1);
}

#[tokio::test]
async fn un_echec_partiel_rejoue_a_tous_et_les_doublons_sont_bornes() {
    let b = banc().await;
    let second = ajouter_affecte(&b.env.admin, &b.c, "administrateur").await;
    let admin = b.session(b.c.administrateur).await;
    // Le premier envoi réussit, le second échoue de façon provisoire : l'alerte est rejouée à
    // tous (au moins une fois) ; le destinataire déjà servi reçoit un doublon, une fois par rejeu.
    b.transport
        .programmer([Ok(()), Err(ErreurEnvoi::Temporaire), Ok(()), Ok(())]);
    b.appel("POST", &b.chemin("/audit/export"), &admin, "")
        .await;
    b.expediteur.passer().await.unwrap();
    assert_eq!(b.transport.remis().len(), 1);
    b.horloge.avancer(Duration::seconds(61));
    b.expediteur.passer().await.unwrap();
    let remis = b.transport.remis();
    assert_eq!(
        remis.len(),
        3,
        "un doublon au plus pour le destinataire déjà servi"
    );
    for u in [b.c.administrateur, second] {
        assert!(remis.iter().any(|m| m.destinataire == courriel(u)));
    }
    // Bornés : jamais plus de `tentatives_max` envois à un même destinataire.
    assert!(remis.len() <= 2 * 8);
}

#[tokio::test]
async fn un_envoi_trop_lent_est_interrompu_avant_la_fin_du_bail() {
    let b = banc().await;
    let admin = b.session(b.c.administrateur).await;
    let lent = Expediteur::new(
        b.env.application.clone(),
        Arc::new(b.transport.avec_lenteur(std::time::Duration::from_secs(30))),
        Arc::new(b.horloge.clone()),
        ORIGINE.to_owned(),
        Reglages {
            budget: std::time::Duration::from_millis(150),
            ..Reglages::default()
        },
    );
    b.appel("POST", &b.chemin("/audit/export"), &admin, "")
        .await;
    let debut = std::time::Instant::now();
    let bilan = lent.passer().await.unwrap();
    assert!(debut.elapsed() < std::time::Duration::from_secs(5));
    assert_eq!((bilan.prises, bilan.reportes), (1, 1));
    let issue: Option<String> = sqlx::query_scalar(
        "SELECT derniere_issue FROM tache WHERE nature = 'notification' AND cabinet_cible = $1",
    )
    .bind(b.c.cabinet.uuid())
    .fetch_one(&b.env.admin)
    .await
    .unwrap();
    assert_eq!(issue.as_deref(), Some("delai_depasse"));
}
