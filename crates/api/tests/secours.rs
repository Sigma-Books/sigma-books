//! Méthode de secours (ADR 0012, ADR 0017 ; L2 PR 8b ; F1.1, F1.6, F1.12, F2.11) : enrôlement,
//! connexion par le secours et par un code de récupération, anti-rejeu TOTP (concurrence sur le
//! même pas comprise), codes à usage unique, session restreinte sans accès métier,
//! anti-énumération, ré-authentification, événements d'authentification et audit des cabinets,
//! RLS de la connexion, limitation de débit.
//!
//! Les cas d'isolation de chaque route (et « Cedar se trompe ») sont dans
//! `isolation/facteurs.rs`, avec ceux de toutes les routes.
#![cfg(feature = "tests-postgres")]
// Fichier de test : les `expect`/`unwrap` y sont voulus.
#![allow(clippy::expect_used, clippy::unwrap_used)]

mod commun;

use std::sync::Arc;
use std::time::{Duration as DureeStd, Instant};

use api::base::IdUtilisateur;
use api::base::session::NatureSession;
use api::facteurs::totp::SecretTotp;
use api::serveur::Horloge;
use authz::Autorisation;
use axum::Router;
use axum::http::StatusCode;
use commun::facteurs::{
    MOT_DE_PASSE, base32_decoder, code, connecter, cookie_de_reponse, corps_code, corps_secours,
    courriel, installer_codes, installer_secours,
};
use commun::serveur::{
    HorlogeTest, Reponse, appeler_session, appeler_session_corps, application_sessions,
    application_sessions_avec, cookie_de, ouvrir, reglages_de_test, routeur_de,
};
use commun::{CabinetTest, ajouter_membre, code_sql, creer_cabinet, creer_utilisateur, env};
use sqlx::PgPool;
use time::Duration;
use uuid::Uuid;

const SECOURS: &str = "/api/connexion/secours";
const CODE: &str = "/api/connexion/code-recuperation";

fn routeur_s(env: &commun::Env, horloge: &HorlogeTest) -> Router {
    routeur_de(&application_sessions(
        env,
        horloge,
        Arc::new(Autorisation::charger().unwrap()),
    ))
}

/// Session tirée d'une réponse de connexion : (cookie, jeton anti-CSRF).
fn session_de(reponse: &Reponse) -> (String, String) {
    assert_eq!(reponse.statut, StatusCode::OK, "{}", reponse.texte());
    (
        cookie_de_reponse(reponse),
        reponse.json()["csrf"].as_str().unwrap().to_owned(),
    )
}

async fn get(routeur: &Router, chemin: &str, cookie: &str) -> Reponse {
    appeler_session(routeur, "GET", chemin, Some(cookie), None).await
}

async fn post(routeur: &Router, chemin: &str, cookie: &str, csrf: &str, corps: &str) -> Reponse {
    appeler_session_corps(routeur, "POST", chemin, Some(cookie), Some(csrf), corps).await
}

/// Connexion par le secours avec le code de l'instant de l'horloge.
async fn connexion_secours(
    routeur: &Router,
    horloge: &HorlogeTest,
    utilisateur: IdUtilisateur,
    secret: &SecretTotp,
) -> Reponse {
    let corps = corps_secours(
        &courriel(utilisateur),
        MOT_DE_PASSE,
        &code(secret, horloge.maintenant()),
    );
    connecter(routeur, SECOURS, &corps, "Navigateur de test").await
}

async fn compter(admin: &PgPool, sql: &'static str, id: Uuid) -> i64 {
    sqlx::query_scalar(sql)
        .bind(id)
        .fetch_one(admin)
        .await
        .unwrap()
}

/// Entrées d'audit du cabinet dont l'action est `action` et l'acteur `utilisateur`.
async fn audit_de(
    admin: &PgPool,
    c: &CabinetTest,
    utilisateur: IdUtilisateur,
    action: &str,
) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM audit_cabinet WHERE cabinet_id = $1 \
         AND contenu LIKE '%acteur=' || $2 || '%' AND contenu LIKE '%action=' || $3 || '%'",
    )
    .bind(c.cabinet.uuid())
    .bind(utilisateur.to_string())
    .bind(action)
    .fetch_one(admin)
    .await
    .unwrap()
}

/// En base : chaîne PHC Argon2id aux paramètres de l'ADR, secret chiffré, codes hachés.
async fn verifier_stockage(admin: &PgPool, u: IdUtilisateur, secret: &SecretTotp) {
    let (phc, chiffre): (String, Vec<u8>) = sqlx::query_as(
        "SELECT mot_de_passe, totp_chiffre FROM facteur_secours WHERE utilisateur_id = $1",
    )
    .bind(u.uuid())
    .fetch_one(admin)
    .await
    .unwrap();
    assert!(phc.starts_with("$argon2id$v=19$m=19456,t=2,p=1$"), "{phc}");
    assert!(!phc.contains(MOT_DE_PASSE));
    assert_eq!(chiffre.len(), 48);
    assert!(!chiffre.windows(20).any(|w| w == secret.octets()));
    let empreintes: Vec<Vec<u8>> =
        sqlx::query_scalar("SELECT empreinte FROM code_recuperation WHERE utilisateur_id = $1")
            .bind(u.uuid())
            .fetch_all(admin)
            .await
            .unwrap();
    assert_eq!(empreintes.len(), 10);
    assert!(empreintes.iter().all(|e| e.len() == 32));
}

/// Enrôlement par l'API au nom de la session (`cookie`, `csrf`) de `u` : préparation, refus
/// (mot de passe, code, jeton), puis réussite ; vérifie le stockage.
struct Enrole {
    secret: SecretTotp,
    secret_b32: String,
    code_confirmation: String,
    codes: Vec<String>,
}

async fn enroler_par_l_api(
    admin: &PgPool,
    routeur: &Router,
    horloge: &HorlogeTest,
    u: IdUtilisateur,
    (cookie, csrf): (&str, &str),
) -> Enrole {
    // Préparation : secret, URI otpauth, jeton d'enrôlement ; rien n'est stocké.
    let r = post(
        routeur,
        "/api/moi/facteurs/secours/preparation",
        cookie,
        csrf,
        "",
    )
    .await;
    assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
    let vue = r.json();
    let secret_b32 = vue["secret"].as_str().unwrap().to_owned();
    assert!(
        vue["uri"]
            .as_str()
            .unwrap()
            .starts_with("otpauth://totp/Sigma%20Books:")
    );
    assert!(
        vue["uri"]
            .as_str()
            .unwrap()
            .contains(&format!("secret={secret_b32}"))
    );
    let jeton = vue["jeton_enrolement"].as_str().unwrap().to_owned();
    let secret = SecretTotp::depuis_octets(base32_decoder(&secret_b32).try_into().unwrap());
    assert_eq!(
        compter(
            admin,
            "SELECT count(*) FROM facteur_secours WHERE utilisateur_id = $1",
            u.uuid()
        )
        .await,
        0
    );

    let enroler = |mdp: &str, code_saisi: &str, jeton: &str| {
        serde_json::json!({ "jeton_enrolement": jeton, "mot_de_passe": mdp, "code": code_saisi })
            .to_string()
    };
    let bon_code = code(&secret, horloge.maintenant());
    for (corps, motif) in [
        (
            enroler("onze caract", &bon_code, &jeton),
            "mot_de_passe_trop_court",
        ),
        (
            enroler("q1w2e3r4t5y6", &bon_code, &jeton),
            "mot_de_passe_compromis",
        ),
        (
            enroler(&"x".repeat(129), &bon_code, &jeton),
            "mot_de_passe_trop_long",
        ),
        (enroler(MOT_DE_PASSE, "000000", &jeton), "code_invalide"),
        (
            enroler(MOT_DE_PASSE, &bon_code, "1.AAAA"),
            "jeton_enrolement_invalide",
        ),
        (
            enroler(MOT_DE_PASSE, &bon_code, &format!("9{jeton}")),
            "jeton_enrolement_invalide",
        ),
    ] {
        let r = post(routeur, "/api/moi/facteurs/secours", cookie, csrf, &corps).await;
        assert_eq!(
            r.statut,
            StatusCode::UNPROCESSABLE_ENTITY,
            "{motif} : {}",
            r.texte()
        );
        assert_eq!(r.json()["erreur"], motif);
    }
    let r = post(
        routeur,
        "/api/moi/facteurs/secours",
        cookie,
        csrf,
        &enroler(MOT_DE_PASSE, &bon_code, &jeton),
    )
    .await;
    assert_eq!(r.statut, StatusCode::CREATED, "{}", r.texte());
    let codes: Vec<String> = r.json()["codes_recuperation"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c.as_str().unwrap().to_owned())
        .collect();
    assert_eq!(codes.len(), 10);

    verifier_stockage(admin, u, &secret).await;
    Enrole {
        secret,
        secret_b32,
        code_confirmation: bon_code,
        codes,
    }
}

/// Événements de l'utilisateur et audit du cabinet après l'enrôlement et les connexions : les
/// natures attendues, aucun secret.
async fn verifier_traces(
    admin: &PgPool,
    routeur: &Router,
    cookie: &str,
    c: &CabinetTest,
    secrets: &[&str],
) {
    let u = c.administrateur;
    // Événements : ceux de l'utilisateur, sans secret.
    let r = get(routeur, "/api/moi/evenements", cookie).await;
    assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
    let texte = r.texte();
    for secret_texte in secrets {
        assert!(!texte.contains(secret_texte));
    }
    let natures: Vec<(String, String)> = r
        .json()
        .as_array()
        .unwrap()
        .iter()
        .map(|e| {
            (
                e["nature"].as_str().unwrap().to_owned(),
                e["issue"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    for attendu in [
        ("enrolement_secours", "succes"),
        ("connexion", "succes"),
        ("connexion", "echec"),
    ] {
        assert!(
            natures.contains(&(attendu.0.to_owned(), attendu.1.to_owned())),
            "{attendu:?} absent de {natures:?}"
        );
    }
    // Audit du cabinet : enrôlement et connexions par le secours, sans secret.
    assert_eq!(audit_de(admin, c, u, "EnrolerSecours").await, 1);
    assert_eq!(audit_de(admin, c, u, "OuvrirSession").await, 2);
    let contenus: Vec<String> =
        sqlx::query_scalar("SELECT contenu FROM audit_cabinet WHERE cabinet_id = $1")
            .bind(c.cabinet.uuid())
            .fetch_all(admin)
            .await
            .unwrap();
    for contenu in &contenus {
        for secret_texte in secrets {
            assert!(!contenu.contains(secret_texte));
        }
    }
}

#[tokio::test]
async fn enrolement_du_secours_puis_connexion_et_anti_rejeu() {
    let env = env().await;
    let c = creer_cabinet(&env.admin, "Cabinet").await;
    let u = c.administrateur;
    let horloge = HorlogeTest::default();
    let routeur = routeur_s(&env, &horloge);
    // Session complète fraîche (passkey, fixture) et une autre session du même utilisateur.
    let ouverte = ouvrir(&env, &horloge, u, NatureSession::Complete).await;
    let autre = ouvrir(&env, &horloge, u, NatureSession::Complete).await;
    let (cookie, csrf) = (cookie_de(&ouverte), ouverte.jeton.csrf());

    let Enrole {
        secret,
        secret_b32,
        code_confirmation: bon_code,
        codes,
    } = enroler_par_l_api(&env.admin, &routeur, &horloge, u, (&cookie, &csrf)).await;

    // Changement de facteur : l'autre session est révoquée, la courante reste.
    assert_eq!(
        get(&routeur, "/api/session", &cookie_de(&autre))
            .await
            .statut,
        StatusCode::UNAUTHORIZED
    );
    let r = get(&routeur, "/api/moi/facteurs", &cookie).await;
    assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
    assert_eq!(r.json()["secours"]["utilisable"], true);
    assert_eq!(r.json()["codes_restants"], 10);
    // Le code de confirmation est consommé : il n'ouvre pas de session.
    let r = connecter(
        &routeur,
        SECOURS,
        &corps_secours(&courriel(u), MOT_DE_PASSE, &bon_code),
        "test",
    )
    .await;
    assert_eq!(r.statut, StatusCode::UNAUTHORIZED, "{}", r.texte());

    // Connexion au pas suivant : session complète, cookie et anti-CSRF.
    horloge.avancer(Duration::seconds(30));
    let r = connexion_secours(&routeur, &horloge, u, &secret).await;
    let (nouveau, csrf_nouveau) = session_de(&r);
    assert_eq!(r.json()["nature"], "complete");
    assert_eq!(r.json()["methode"], "secours");
    let etat = get(&routeur, "/api/session", &nouveau).await;
    assert_eq!(etat.json()["csrf"], csrf_nouveau);
    assert_eq!(etat.json()["methode"], "secours");
    // Rejeu du même code : refusé, même dans la tolérance d'un pas.
    let r = connexion_secours(&routeur, &horloge, u, &secret).await;
    assert_eq!(r.statut, StatusCode::UNAUTHORIZED, "{}", r.texte());
    assert_eq!(r.json()["erreur"], "non_authentifie");
    // Mot de passe faux avec un code neuf : refusé, et le pas n'est pas consommé.
    horloge.avancer(Duration::seconds(30));
    let neuf = code(&secret, horloge.maintenant());
    let r = connecter(
        &routeur,
        SECOURS,
        &corps_secours(&courriel(u), "autre mot de passe long", &neuf),
        "test",
    )
    .await;
    assert_eq!(r.statut, StatusCode::UNAUTHORIZED);
    let r = connecter(
        &routeur,
        SECOURS,
        &corps_secours(&courriel(u).to_uppercase(), MOT_DE_PASSE, &neuf),
        "test",
    )
    .await;
    assert_eq!(
        r.statut,
        StatusCode::OK,
        "courriel sans casse : {}",
        r.texte()
    );

    verifier_traces(
        &env.admin,
        &routeur,
        &nouveau,
        &c,
        &[MOT_DE_PASSE, &bon_code, &neuf, &secret_b32, &codes[0]],
    )
    .await;
}

#[tokio::test]
async fn deux_connexions_concurrentes_sur_le_meme_pas_une_seule_passe() {
    let env = env().await;
    let c = creer_cabinet(&env.admin, "Cabinet").await;
    let secret = installer_secours(&env.admin, c.administrateur, MOT_DE_PASSE).await;
    let horloge = HorlogeTest::default();
    let routeur = routeur_s(&env, &horloge);
    for _ in 0..3 {
        horloge.avancer(Duration::seconds(90));
        let (a, b) = tokio::join!(
            connexion_secours(&routeur, &horloge, c.administrateur, &secret),
            connexion_secours(&routeur, &horloge, c.administrateur, &secret),
        );
        let mut statuts = [a.statut, b.statut];
        statuts.sort();
        assert_eq!(statuts, [StatusCode::OK, StatusCode::UNAUTHORIZED]);
    }
}

/// Session restreinte (F1.12) : sa propre session, l'enrôlement, rien d'autre.
async fn verifier_session_restreinte(routeur: &Router, cookie: &str, csrf: &str, a: &CabinetTest) {
    let etat = get(routeur, "/api/session", cookie).await;
    assert_eq!(etat.json()["nature"], "restreinte");
    for chemin in [
        format!("/api/cabinets/{}", a.cabinet),
        format!("/api/cabinets/{}/dossiers", a.cabinet),
        "/api/moi/cabinets".to_owned(),
        "/api/moi/sessions".to_owned(),
        "/api/moi/facteurs".to_owned(),
        "/api/moi/evenements".to_owned(),
    ] {
        assert_eq!(
            get(routeur, &chemin, cookie).await.statut,
            StatusCode::UNAUTHORIZED,
            "{chemin}"
        );
    }
    for chemin in [
        "/api/moi/facteurs/codes-recuperation",
        "/api/session/reauthentification",
        "/api/session/rotation",
    ] {
        let r = post(routeur, chemin, cookie, csrf, "{}").await;
        assert_eq!(r.statut, StatusCode::UNAUTHORIZED, "{chemin}");
    }
}

#[tokio::test]
async fn code_de_recuperation_usage_unique_et_session_restreinte() {
    let env = env().await;
    let a = creer_cabinet(&env.admin, "A").await;
    let b = creer_cabinet(&env.admin, "B").await;
    let suspendu = creer_cabinet(&env.admin, "Suspendu").await;
    // Membre actif de A et de B, suspendu dans le troisième.
    let u = IdUtilisateur::nouveau();
    let mut tx = env.admin.begin().await.unwrap();
    creer_utilisateur(&mut tx, u, "Membre").await;
    ajouter_membre(&mut tx, a.cabinet, u, "collaborateur").await;
    ajouter_membre(&mut tx, b.cabinet, u, "valideur").await;
    ajouter_membre(&mut tx, suspendu.cabinet, u, "administrateur").await;
    sqlx::query("UPDATE appartenance SET actif = false WHERE cabinet_id = $1")
        .bind(suspendu.cabinet.uuid())
        .execute(&mut *tx)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    let codes = installer_codes(&env.admin, u).await;
    let horloge = HorlogeTest::default();
    let routeur = routeur_s(&env, &horloge);

    // Saisie tolérante : minuscules, sans tirets.
    let saisi = codes[0].to_lowercase().replace('-', "");
    let r = connecter(&routeur, CODE, &corps_code(&courriel(u), &saisi), "test").await;
    let (cookie, csrf) = session_de(&r);
    assert_eq!(r.json()["nature"], "restreinte");
    // Usage unique, même concurrent : le même code est refusé.
    let r = connecter(&routeur, CODE, &corps_code(&courriel(u), &codes[0]), "test").await;
    assert_eq!(r.statut, StatusCode::UNAUTHORIZED);
    let corps = corps_code(&courriel(u), &codes[1]);
    let (premiere, seconde) = tokio::join!(
        connecter(&routeur, CODE, &corps, "test"),
        connecter(&routeur, CODE, &corps, "test"),
    );
    let mut statuts = [premiere.statut, seconde.statut];
    statuts.sort();
    assert_eq!(statuts, [StatusCode::OK, StatusCode::UNAUTHORIZED]);

    verifier_session_restreinte(&routeur, &cookie, &csrf, &a).await;
    let r = post(
        &routeur,
        "/api/moi/facteurs/secours/preparation",
        &cookie,
        &csrf,
        "",
    )
    .await;
    assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
    let secret = SecretTotp::depuis_octets(
        base32_decoder(r.json()["secret"].as_str().unwrap())
            .try_into()
            .unwrap(),
    );
    let corps = serde_json::json!({
        "jeton_enrolement": r.json()["jeton_enrolement"],
        "mot_de_passe": MOT_DE_PASSE,
        "code": code(&secret, horloge.maintenant()),
    })
    .to_string();
    let r = post(
        &routeur,
        "/api/moi/facteurs/secours",
        &cookie,
        &csrf,
        &corps,
    )
    .await;
    assert_eq!(r.statut, StatusCode::CREATED, "{}", r.texte());
    // La nouvelle série annule l'ancienne.
    let r = connecter(&routeur, CODE, &corps_code(&courriel(u), &codes[2]), "test").await;
    assert_eq!(r.statut, StatusCode::UNAUTHORIZED);

    // Audit de chaque cabinet actif (usages des codes, enrôlement), rien dans le suspendu.
    for cabinet in [&a, &b] {
        assert_eq!(audit_de(&env.admin, cabinet, u, "OuvrirSession").await, 2);
        assert_eq!(audit_de(&env.admin, cabinet, u, "EnrolerSecours").await, 1);
    }
    assert_eq!(
        compter(
            &env.admin,
            "SELECT count(*) FROM audit_cabinet WHERE cabinet_id = $1",
            suspendu.cabinet.uuid()
        )
        .await,
        0
    );
    // Les rôles de chaque cabinet sont ceux de l'appartenance.
    let contenu: String = sqlx::query_scalar(
        "SELECT contenu FROM audit_cabinet WHERE cabinet_id = $1 ORDER BY rang LIMIT 1",
    )
    .bind(b.cabinet.uuid())
    .fetch_one(&env.admin)
    .await
    .unwrap();
    assert!(contenu.contains("role=valideur"), "{contenu}");
    assert!(contenu.contains("valeur=code_recuperation"), "{contenu}");
}

#[tokio::test]
async fn courriel_inconnu_meme_reponse_duree_comparable_et_rien_d_inscrit() {
    let env = env().await;
    let c = creer_cabinet(&env.admin, "Cabinet").await;
    let secret = installer_secours(&env.admin, c.administrateur, MOT_DE_PASSE).await;
    // Un compte sans secours.
    let sans_secours = c.collaborateur;
    let horloge = HorlogeTest::default();
    let mut reglages = reglages_de_test();
    reglages.facteurs.debit_connexion_courriel.capacite = 1000;
    reglages.facteurs.debit_connexion_adresse.capacite = 1000;
    let routeur = routeur_de(&application_sessions_avec(
        &env,
        &horloge,
        Arc::new(Autorisation::charger().unwrap()),
        reglages,
    ));
    let code_faux = code(&secret, horloge.maintenant() + Duration::hours(1));
    let appareil = format!("appareil-{}", Uuid::new_v4());
    let inconnu = format!("{}@exemple.invalid", Uuid::new_v4());
    let mesurer = |courriel: String| {
        let routeur = routeur.clone();
        let appareil = appareil.clone();
        let code_faux = code_faux.clone();
        async move {
            let mut durees = Vec::new();
            let mut reponses = Vec::new();
            for _ in 0..5 {
                let debut = Instant::now();
                let r = connecter(
                    &routeur,
                    SECOURS,
                    &corps_secours(&courriel, "mauvais mot de passe", &code_faux),
                    &appareil,
                )
                .await;
                durees.push(debut.elapsed());
                reponses.push(r);
            }
            durees.sort();
            (durees[2], reponses)
        }
    };
    let (d_inconnu, r_inconnu) = mesurer(inconnu.clone()).await;
    let (d_connu, r_connu) = mesurer(courriel(c.administrateur)).await;
    let (d_sans, r_sans) = mesurer(courriel(sans_secours)).await;
    for r in r_inconnu.iter().chain(&r_connu).chain(&r_sans) {
        assert_eq!(r.statut, StatusCode::UNAUTHORIZED);
        assert_eq!(r.corps, r_connu[0].corps);
        assert!(r.entetes.get("set-cookie").is_none());
    }
    // Durée comparable : Argon2id est exécuté dans les trois cas (médianes).
    let rapport = |a: DureeStd, b: DureeStd| a.as_secs_f64() / b.as_secs_f64();
    for (nom, d) in [("inconnu", d_inconnu), ("sans secours", d_sans)] {
        let r = rapport(d, d_connu);
        assert!((0.5..2.0).contains(&r), "{nom} : {d:?} contre {d_connu:?}");
    }
    // Rien pour le courriel inconnu ; un échec par tentative pour les comptes connus.
    let par_appareil = |id: Option<Uuid>| {
        let admin = env.admin.clone();
        let appareil = appareil.clone();
        async move {
            sqlx::query_scalar::<_, i64>(
                "SELECT count(*) FROM evenement_authentification \
                 WHERE appareil = $1 AND ($2::uuid IS NULL OR utilisateur_id = $2)",
            )
            .bind(appareil)
            .bind(id)
            .fetch_one(&admin)
            .await
            .unwrap()
        }
    };
    assert_eq!(par_appareil(None).await, 10);
    assert_eq!(par_appareil(Some(c.administrateur.uuid())).await, 5);
    assert_eq!(par_appareil(Some(sans_secours.uuid())).await, 5);
    // Code de récupération pour un courriel inconnu : même réponse, rien d'inscrit.
    let r = connecter(
        &routeur,
        CODE,
        &corps_code(&inconnu, "0000-0000-0000-0000-0000-0000"),
        &appareil,
    )
    .await;
    assert_eq!(r.statut, StatusCode::UNAUTHORIZED);
    assert_eq!(r.corps, r_connu[0].corps);
    assert_eq!(par_appareil(None).await, 10);
}

#[tokio::test]
async fn reauthentification_par_le_secours() {
    let env = env().await;
    let c = creer_cabinet(&env.admin, "Cabinet").await;
    let u = c.administrateur;
    let secret = installer_secours(&env.admin, u, MOT_DE_PASSE).await;
    let horloge = HorlogeTest::default();
    let routeur = routeur_s(&env, &horloge);
    let r = connexion_secours(&routeur, &horloge, u, &secret).await;
    let (cookie, csrf) = session_de(&r);
    let id = r.json()["session"].as_str().unwrap().to_owned();

    // Six minutes plus tard : session non fraîche, nouvelle série de codes refusée (403).
    horloge.avancer(Duration::minutes(6));
    assert_eq!(
        get(&routeur, "/api/session", &cookie).await.json()["fraiche"],
        false
    );
    let r = post(
        &routeur,
        "/api/moi/facteurs/codes-recuperation",
        &cookie,
        &csrf,
        "",
    )
    .await;
    assert_eq!(r.statut, StatusCode::FORBIDDEN, "{}", r.texte());
    let avant: time::OffsetDateTime =
        sqlx::query_scalar("SELECT derniere_verification FROM session WHERE id = $1::uuid")
            .bind(&id)
            .fetch_one(&env.admin)
            .await
            .unwrap();

    // Facteurs faux : 422, rien ne change.
    let neuf = code(&secret, horloge.maintenant());
    let faux =
        serde_json::json!({ "mot_de_passe": "autre mot de passe long", "code": neuf }).to_string();
    let r = post(
        &routeur,
        "/api/session/reauthentification",
        &cookie,
        &csrf,
        &faux,
    )
    .await;
    assert_eq!(r.statut, StatusCode::UNPROCESSABLE_ENTITY, "{}", r.texte());
    assert_eq!(r.json()["erreur"], "facteurs_refuses");
    // Jeton anti-CSRF absent : 403.
    let bon = serde_json::json!({ "mot_de_passe": MOT_DE_PASSE, "code": neuf }).to_string();
    let r = post(
        &routeur,
        "/api/session/reauthentification",
        &cookie,
        "faux",
        &bon,
    )
    .await;
    assert_eq!(r.statut, StatusCode::FORBIDDEN);
    // Facteurs justes : session fraîche, jeton tourné.
    let r = post(
        &routeur,
        "/api/session/reauthentification",
        &cookie,
        &csrf,
        &bon,
    )
    .await;
    assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
    let nouveau = cookie_de_reponse(&r);
    let csrf_nouveau = r.json()["csrf"].as_str().unwrap().to_owned();
    assert_eq!(
        get(&routeur, "/api/session", &cookie).await.statut,
        StatusCode::UNAUTHORIZED
    );
    let etat = get(&routeur, "/api/session", &nouveau).await;
    assert_eq!(etat.json()["id"], id);
    assert_eq!(etat.json()["fraiche"], true);
    let apres: time::OffsetDateTime =
        sqlx::query_scalar("SELECT derniere_verification FROM session WHERE id = $1::uuid")
            .bind(&id)
            .fetch_one(&env.admin)
            .await
            .unwrap();
    assert!(apres > avant + Duration::minutes(5), "{avant} -> {apres}");
    // Rejeu du même code : refusé.
    let r = post(
        &routeur,
        "/api/session/reauthentification",
        &nouveau,
        &csrf_nouveau,
        &bon,
    )
    .await;
    assert_eq!(r.statut, StatusCode::UNPROCESSABLE_ENTITY);
    // Fraîche : nouvelle série de codes admise.
    let r = post(
        &routeur,
        "/api/moi/facteurs/codes-recuperation",
        &nouveau,
        &csrf_nouveau,
        "",
    )
    .await;
    assert_eq!(r.statut, StatusCode::CREATED, "{}", r.texte());
    assert_eq!(r.json()["codes_recuperation"].as_array().unwrap().len(), 10);
    assert_eq!(
        audit_de(&env.admin, &c, u, "RegenererCodesRecuperation").await,
        1
    );
}

#[tokio::test]
async fn les_tentatives_de_connexion_sont_limitees() {
    let env = env().await;
    let c = creer_cabinet(&env.admin, "Cabinet").await;
    let horloge = HorlogeTest::default();
    let mut reglages = reglages_de_test();
    reglages.facteurs.debit_connexion_courriel.capacite = 3;
    reglages.facteurs.debit_connexion_adresse.capacite = 5;
    let routeur = routeur_de(&application_sessions_avec(
        &env,
        &horloge,
        Arc::new(Autorisation::charger().unwrap()),
        reglages,
    ));
    let tenter = |courriel: String| {
        let routeur = routeur.clone();
        async move {
            connecter(
                &routeur,
                CODE,
                &corps_code(&courriel, "0000-0000-0000-0000-0000-0000"),
                "test",
            )
            .await
            .statut
        }
    };
    // Par courriel (connu ou non, casse comprise) : trois essais, puis 429.
    let cible = courriel(c.administrateur);
    for _ in 0..3 {
        assert_eq!(tenter(cible.clone()).await, StatusCode::UNAUTHORIZED);
    }
    let r = connecter(
        &routeur,
        CODE,
        &corps_code(&cible.to_uppercase(), "x"),
        "test",
    )
    .await;
    assert_eq!(r.statut, StatusCode::TOO_MANY_REQUESTS);
    assert!(r.entetes.get("retry-after").is_some());
    // Par adresse : cinq essais en tout, quel que soit le courriel.
    assert_eq!(
        tenter("a@exemple.invalid".to_owned()).await,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        tenter("b@exemple.invalid".to_owned()).await,
        StatusCode::TOO_MANY_REQUESTS
    );
}

#[tokio::test]
async fn la_rls_de_la_connexion_ne_voit_que_le_compte_du_courriel() {
    let env = env().await;
    let a = creer_cabinet(&env.admin, "A").await;
    let b = creer_cabinet(&env.admin, "B").await;
    installer_secours(&env.admin, a.administrateur, MOT_DE_PASSE).await;
    installer_secours(&env.admin, b.administrateur, MOT_DE_PASSE).await;
    installer_codes(&env.admin, a.administrateur).await;
    installer_codes(&env.admin, b.administrateur).await;
    let horloge = HorlogeTest::default();
    // Une session ouverte de A : invisible depuis la connexion.
    ouvrir(&env, &horloge, a.administrateur, NatureSession::Complete).await;

    let mut tx = env.application.begin().await.unwrap();
    sqlx::query("SELECT set_config('sigma.courriel_connexion', $1, true)")
        .bind(courriel(a.administrateur).to_uppercase())
        .execute(&mut *tx)
        .await
        .unwrap();
    for (sql, attendu) in [
        ("SELECT count(*) FROM utilisateur", 1),
        ("SELECT count(*) FROM facteur_secours", 1),
        ("SELECT count(*) FROM code_recuperation", 10),
        ("SELECT count(*) FROM evenement_authentification", 0),
        ("SELECT count(*) FROM session", 0),
        ("SELECT count(*) FROM appartenance", 0),
        ("SELECT count(*) FROM cabinet", 0),
    ] {
        let n: i64 = sqlx::query_scalar(sql).fetch_one(&mut *tx).await.unwrap();
        assert_eq!(n, attendu, "{sql}");
    }
    let vus: Vec<Uuid> = sqlx::query_scalar("SELECT utilisateur_id FROM facteur_secours")
        .fetch_all(&mut *tx)
        .await
        .unwrap();
    assert_eq!(vus, vec![a.administrateur.uuid()]);
    // Le compte de B : ni lu, ni modifié, ni ses codes consommés.
    let n =
        sqlx::query("UPDATE facteur_secours SET totp_dernier_pas = 99 WHERE utilisateur_id = $1")
            .bind(b.administrateur.uuid())
            .execute(&mut *tx)
            .await
            .unwrap()
            .rows_affected();
    assert_eq!(n, 0);
    let n =
        sqlx::query("UPDATE code_recuperation SET utilise_le = now() WHERE utilisateur_id = $1")
            .bind(b.administrateur.uuid())
            .execute(&mut *tx)
            .await
            .unwrap()
            .rows_affected();
    assert_eq!(n, 0);
    // Suppression des codes de A : aucune politique ne l'ouvre à la connexion.
    let n = sqlx::query("DELETE FROM code_recuperation")
        .execute(&mut *tx)
        .await
        .unwrap()
        .rows_affected();
    assert_eq!(n, 0);
    tx.rollback().await.unwrap();
}

#[tokio::test]
async fn aucune_creation_de_facteur_depuis_une_connexion_et_evenements_en_ajout_seul() {
    let env = env().await;
    let a = creer_cabinet(&env.admin, "A").await;
    let b = creer_cabinet(&env.admin, "B").await;
    // Aucune création de facteur depuis une connexion, même pour le compte du courriel saisi ;
    // aucun événement pour un autre compte que le sien.
    for (sql, cible, courriel_saisi) in [
        (
            "INSERT INTO facteur_secours (utilisateur_id, mot_de_passe, totp_chiffre, totp_cle_id, \
             totp_dernier_pas, cree_le, modifie_le) \
             VALUES ($1, '$argon2id$x', decode(repeat('00', 48), 'hex'), \
                     decode('0000000000000000', 'hex'), 0, now(), now())",
            b.collaborateur,
            courriel(b.collaborateur),
        ),
        (
            "INSERT INTO code_recuperation (id, utilisateur_id, empreinte, cree_le) \
             VALUES (gen_random_uuid(), $1, decode(repeat('ab', 32), 'hex'), now())",
            a.collaborateur,
            courriel(a.collaborateur),
        ),
        (
            "INSERT INTO evenement_authentification (id, utilisateur_id, nature, issue, instant) \
             VALUES (gen_random_uuid(), $1, 'connexion', 'echec', now())",
            b.administrateur,
            courriel(a.administrateur),
        ),
    ] {
        let mut tx = env.application.begin().await.unwrap();
        sqlx::query("SELECT set_config('sigma.courriel_connexion', $1, true)")
            .bind(courriel_saisi)
            .execute(&mut *tx)
            .await
            .unwrap();
        let erreur = sqlx::query(sql)
            .bind(cible.uuid())
            .execute(&mut *tx)
            .await
            .unwrap_err();
        assert_eq!(code_sql(&erreur).as_deref(), Some("42501"), "{sql}");
    }
    // Les événements sont en ajout seul pour le rôle applicatif.
    let mut tx = env.application.begin().await.unwrap();
    sqlx::query("SELECT set_config('sigma.utilisateur', $1, true)")
        .bind(a.administrateur.to_string())
        .execute(&mut *tx)
        .await
        .unwrap();
    for sql in [
        "UPDATE evenement_authentification SET issue = 'succes'",
        "DELETE FROM evenement_authentification",
    ] {
        let erreur = sqlx::query(sql).execute(&mut *tx).await.unwrap_err();
        assert_eq!(code_sql(&erreur).as_deref(), Some("42501"), "{sql}");
        tx.rollback().await.unwrap();
        tx = env.application.begin().await.unwrap();
    }
}

#[tokio::test]
async fn les_evenements_ne_portent_aucun_secret() {
    let env = env().await;
    let colonnes: Vec<String> = sqlx::query_scalar(
        "SELECT column_name::text FROM information_schema.columns \
         WHERE table_name = 'evenement_authentification' ORDER BY ordinal_position",
    )
    .fetch_all(&env.admin)
    .await
    .unwrap();
    assert_eq!(
        colonnes,
        [
            "id",
            "utilisateur_id",
            "session_id",
            "nature",
            "methode",
            "issue",
            "instant",
            "appareil",
            "adresse"
        ]
    );
}

#[tokio::test]
async fn deconnexion_et_revocation_sont_des_evenements_d_authentification() {
    let env = env().await;
    let c = creer_cabinet(&env.admin, "Cabinet").await;
    let horloge = HorlogeTest::default();
    let routeur = routeur_s(&env, &horloge);
    let courante = ouvrir(&env, &horloge, c.administrateur, NatureSession::Complete).await;
    let autre = ouvrir(&env, &horloge, c.administrateur, NatureSession::Complete).await;
    let (cookie, csrf) = (cookie_de(&courante), courante.jeton.csrf());
    let r = appeler_session(
        &routeur,
        "DELETE",
        &format!("/api/moi/sessions/{}", autre.id),
        Some(&cookie),
        Some(&csrf),
    )
    .await;
    assert_eq!(r.statut, StatusCode::NO_CONTENT, "{}", r.texte());
    let r = post(&routeur, "/api/session/deconnexion", &cookie, &csrf, "").await;
    assert_eq!(r.statut, StatusCode::NO_CONTENT, "{}", r.texte());
    let evenements: Vec<(String, Option<Uuid>)> = sqlx::query_as(
        "SELECT nature, session_id FROM evenement_authentification \
         WHERE utilisateur_id = $1 ORDER BY instant, nature",
    )
    .bind(c.administrateur.uuid())
    .fetch_all(&env.admin)
    .await
    .unwrap();
    assert!(evenements.contains(&("revocation_session".to_owned(), Some(autre.id.uuid()))));
    assert!(evenements.contains(&("deconnexion".to_owned(), Some(courante.id.uuid()))));
}
