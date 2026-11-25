//! Cas d'isolation des routes du module `facteurs` (PR 8b ; ADR 0012, ADR 0017) : méthode de
//! secours, codes de récupération, ré-authentification, événements. Sessions réelles de deux
//! utilisateurs de cabinets différents (A, B) et du membre des deux cabinets : aucun ne lit,
//! n'utilise ni ne modifie les facteurs ou les événements d'un autre. Les connexions, sans
//! session, éprouvent les facteurs d'un compte contre le courriel d'un autre.
//!
//! « Cedar se trompe » : [`les_facteurs_restent_bornes_quand_cedar_se_trompe`].

use std::sync::Arc;

use api::base::IdUtilisateur;
use api::base::session::NatureSession;
use api::facteurs::totp::SecretTotp;
use api::serveur::Horloge;
use api::serveur::routes::{Methode, Route};
use axum::Router;
use axum::http::StatusCode;
use sqlx::PgPool;
use uuid::Uuid;

use super::commun::facteurs::{
    MOT_DE_PASSE, base32_decoder, code, connecter, cookie_de_reponse, corps_code, corps_secours,
    courriel, installer_codes, installer_secours,
};
use super::commun::serveur::{
    HorlogeTest, Reponse, ToujoursAutorise, appeler_session_corps, application_sessions, cookie_de,
    ouvrir,
};
use super::{CasModule, Ctx, Scenario, lire_s, monde, session_vue};
use api::serveur::session::SessionOuverte;

pub(super) const CAS: CasModule = CasModule {
    attentes: CasModule::AUCUN.attentes,
    hors_cabinet,
};

fn hors_cabinet(route: &Route) -> Option<Scenario> {
    Some(match (route.methode, route.chemin) {
        (Methode::Post, "/api/connexion/secours") => |c| Box::pin(cas_connexion_secours(c)),
        (Methode::Post, "/api/connexion/code-recuperation") => |c| Box::pin(cas_connexion_code(c)),
        (Methode::Post, "/api/session/reauthentification") => {
            |c| Box::pin(cas_reauthentification(c))
        }
        (Methode::Get, "/api/moi/facteurs") => |c| Box::pin(cas_consulter_facteurs(c)),
        (Methode::Get, "/api/moi/evenements") => |c| Box::pin(cas_evenements(c)),
        (Methode::Post, "/api/moi/facteurs/secours/preparation") => {
            |c| Box::pin(cas_preparation(c))
        }
        (Methode::Post, "/api/moi/facteurs/secours") => |c| Box::pin(cas_enrolement(c)),
        (Methode::Post, "/api/moi/facteurs/codes-recuperation") => {
            |c| Box::pin(cas_regeneration(c))
        }
        _ => return None,
    })
}

/// Mot de passe du secours de B, différent de celui de A.
const MOT_DE_PASSE_B: &str = "autre phrase de passe de B";

/// `POST` au nom d'une session réelle, avec son jeton anti-CSRF et un corps JSON.
async fn poster(routeur: &Router, chemin: &str, s: &SessionOuverte, corps: &str) -> Reponse {
    appeler_session_corps(
        routeur,
        "POST",
        chemin,
        Some(&cookie_de(s)),
        Some(&s.jeton.csrf()),
        corps,
    )
    .await
}

async fn retirer_facteurs(admin: &PgPool, utilisateur: IdUtilisateur) {
    for sql in [
        "DELETE FROM facteur_secours WHERE utilisateur_id = $1",
        "DELETE FROM code_recuperation WHERE utilisateur_id = $1",
    ] {
        sqlx::query(sql)
            .bind(utilisateur.uuid())
            .execute(admin)
            .await
            .unwrap();
    }
}

/// État du secours d'un utilisateur : (chaîne PHC, dernier pas), s'il en a un.
async fn secours_de(admin: &PgPool, utilisateur: IdUtilisateur) -> Option<(String, i64)> {
    sqlx::query_as(
        "SELECT mot_de_passe, totp_dernier_pas FROM facteur_secours WHERE utilisateur_id = $1",
    )
    .bind(utilisateur.uuid())
    .fetch_optional(admin)
    .await
    .unwrap()
}

async fn codes_inutilises(admin: &PgPool, utilisateur: IdUtilisateur) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM code_recuperation WHERE utilisateur_id = $1 AND utilise_le IS NULL",
    )
    .bind(utilisateur.uuid())
    .fetch_one(admin)
    .await
    .unwrap()
}

/// Titulaire d'une session ouverte par une connexion (identifiant rendu dans le corps).
async fn titulaire(admin: &PgPool, reponse: &Reponse) -> Uuid {
    assert_eq!(reponse.statut, StatusCode::OK, "{}", reponse.texte());
    sqlx::query_scalar("SELECT utilisateur_id FROM session WHERE id = $1::uuid")
        .bind(reponse.json()["session"].as_str().unwrap())
        .fetch_one(admin)
        .await
        .unwrap()
}

async fn cas_connexion_secours(c: &Ctx<'_>) {
    let Ctx {
        env,
        horloge,
        routeur,
        m,
        ..
    } = c;
    let (a, b) = (m.a.administrateur, m.b.administrateur);
    let secret_a = installer_secours(&env.admin, a, MOT_DE_PASSE).await;
    let secret_b = installer_secours(&env.admin, b, MOT_DE_PASSE_B).await;
    let maintenant = horloge.maintenant();
    // Les facteurs de B, ou un mélange, ne valent rien pour le compte de A.
    for (mdp, secret) in [
        (MOT_DE_PASSE_B, &secret_b),
        (MOT_DE_PASSE, &secret_b),
        (MOT_DE_PASSE_B, &secret_a),
    ] {
        let corps = corps_secours(&courriel(a), mdp, &code(secret, maintenant));
        let r = connecter(routeur, "/api/connexion/secours", &corps, "test").await;
        assert_eq!(r.statut, StatusCode::UNAUTHORIZED, "{}", r.texte());
    }
    assert_eq!(secours_de(&env.admin, b).await.unwrap().1, 0);
    // Les siens ouvrent une session de A, et seulement de A.
    let corps = corps_secours(&courriel(a), MOT_DE_PASSE, &code(&secret_a, maintenant));
    let r = connecter(routeur, "/api/connexion/secours", &corps, "test").await;
    assert_eq!(titulaire(&env.admin, &r).await, a.uuid());
    // Sans corps : rien.
    let r = connecter(routeur, "/api/connexion/secours", "", "test").await;
    assert_eq!(r.statut, StatusCode::UNAUTHORIZED);
}

async fn cas_connexion_code(c: &Ctx<'_>) {
    let Ctx {
        env, routeur, m, ..
    } = c;
    let (a, b) = (m.a.administrateur, m.b.administrateur);
    let codes_a = installer_codes(&env.admin, a).await;
    let codes_b = installer_codes(&env.admin, b).await;
    // Un code de B ne vaut rien pour A, et reste utilisable.
    let r = connecter(
        routeur,
        "/api/connexion/code-recuperation",
        &corps_code(&courriel(a), &codes_b[0]),
        "test",
    )
    .await;
    assert_eq!(r.statut, StatusCode::UNAUTHORIZED);
    assert_eq!(codes_inutilises(&env.admin, b).await, 10);
    let r = connecter(
        routeur,
        "/api/connexion/code-recuperation",
        &corps_code(&courriel(a), &codes_a[0]),
        "test",
    )
    .await;
    assert_eq!(titulaire(&env.admin, &r).await, a.uuid());
    assert_eq!(r.json()["nature"], "restreinte");
    assert_eq!(codes_inutilises(&env.admin, a).await, 9);
    assert_eq!(codes_inutilises(&env.admin, b).await, 10);
}

async fn cas_reauthentification(c: &Ctx<'_>) {
    let Ctx {
        env,
        horloge,
        routeur,
        m,
        sa,
        sb,
        ..
    } = c;
    let (a, b) = (m.a.administrateur, m.b.administrateur);
    let secret_a = installer_secours(&env.admin, a, MOT_DE_PASSE).await;
    let secret_b = installer_secours(&env.admin, b, MOT_DE_PASSE_B).await;
    let maintenant = horloge.maintenant();
    let corps = |mdp: &str, secret: &SecretTotp| {
        serde_json::json!({ "mot_de_passe": mdp, "code": code(secret, maintenant) }).to_string()
    };
    // Les facteurs de B ne ré-authentifient pas A, et B n'en est pas affecté.
    let r = poster(
        routeur,
        "/api/session/reauthentification",
        sa,
        &corps(MOT_DE_PASSE_B, &secret_b),
    )
    .await;
    assert_eq!(r.statut, StatusCode::UNPROCESSABLE_ENTITY, "{}", r.texte());
    assert_eq!(secours_de(&env.admin, b).await.unwrap().1, 0);
    let r = poster(
        routeur,
        "/api/session/reauthentification",
        sa,
        &corps(MOT_DE_PASSE, &secret_a),
    )
    .await;
    assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
    // Le jeton de A a tourné ; la session de B est intacte.
    assert_eq!(session_vue(routeur, sa).await, None);
    assert_eq!(session_vue(routeur, sb).await, Some(sb.id.to_string()));
    let nouveau = cookie_de_reponse(&r);
    let r = appeler_session_corps(routeur, "GET", "/api/session", Some(&nouveau), None, "").await;
    assert_eq!(r.json()["id"], sa.id.to_string());
}

async fn cas_consulter_facteurs(c: &Ctx<'_>) {
    let Ctx {
        env,
        routeur,
        m,
        sa,
        sb,
        ..
    } = c;
    let (a, b) = (m.a.administrateur, m.b.administrateur);
    retirer_facteurs(&env.admin, a).await;
    installer_secours(&env.admin, b, MOT_DE_PASSE_B).await;
    installer_codes(&env.admin, b).await;
    let r = lire_s(routeur, "/api/moi/facteurs", sa).await;
    assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
    assert_eq!(r.json()["secours"], serde_json::Value::Null);
    assert_eq!(r.json()["codes_restants"], 0);
    let r = lire_s(routeur, "/api/moi/facteurs", sb).await;
    assert_eq!(r.json()["secours"]["utilisable"], true);
    assert_eq!(r.json()["codes_restants"], 10);
}

async fn cas_evenements(c: &Ctx<'_>) {
    let Ctx {
        env,
        routeur,
        m,
        sa,
        sb,
        ..
    } = c;
    let marque = format!("appareil-de-B-{}", Uuid::new_v4());
    sqlx::query(
        "INSERT INTO evenement_authentification (id, utilisateur_id, nature, issue, instant, appareil) \
         VALUES ($1, $2, 'connexion', 'echec', now(), $3)",
    )
    .bind(Uuid::new_v4())
    .bind(m.b.administrateur.uuid())
    .bind(&marque)
    .execute(&env.admin)
    .await
    .unwrap();
    let r = lire_s(routeur, "/api/moi/evenements", sa).await;
    assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
    assert!(!r.texte().contains(&marque));
    let r = lire_s(routeur, "/api/moi/evenements", &c.s_multi).await;
    assert!(!r.texte().contains(&marque));
    let r = lire_s(routeur, "/api/moi/evenements", sb).await;
    assert!(r.texte().contains(&marque), "{}", r.texte());
}

/// Prépare un enrôlement au nom de `s` : (secret, jeton d'enrôlement).
async fn preparer(routeur: &Router, s: &SessionOuverte) -> (SecretTotp, String) {
    let r = poster(routeur, "/api/moi/facteurs/secours/preparation", s, "").await;
    assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
    let secret = SecretTotp::depuis_octets(
        base32_decoder(r.json()["secret"].as_str().unwrap())
            .try_into()
            .unwrap(),
    );
    (
        secret,
        r.json()["jeton_enrolement"].as_str().unwrap().to_owned(),
    )
}

fn corps_enrolement(jeton: &str, mdp: &str, code_saisi: &str) -> String {
    serde_json::json!({ "jeton_enrolement": jeton, "mot_de_passe": mdp, "code": code_saisi })
        .to_string()
}

async fn cas_preparation(c: &Ctx<'_>) {
    let Ctx {
        env,
        horloge,
        routeur,
        m,
        sa,
        sb,
        s_multi,
        ..
    } = c;
    let (secret, jeton) = preparer(routeur, sa).await;
    let code_a = code(&secret, horloge.maintenant());
    // Le jeton de A ne vaut ni pour B, ni pour le membre des deux cabinets.
    for s in [sb, s_multi] {
        let r = poster(
            routeur,
            "/api/moi/facteurs/secours",
            s,
            &corps_enrolement(&jeton, MOT_DE_PASSE, &code_a),
        )
        .await;
        assert_eq!(r.statut, StatusCode::UNPROCESSABLE_ENTITY, "{}", r.texte());
        assert_eq!(r.json()["erreur"], "jeton_enrolement_invalide");
    }
    // Ni pour une autre session de A.
    let autre = ouvrir(env, horloge, m.a.administrateur, NatureSession::Complete).await;
    let r = poster(
        routeur,
        "/api/moi/facteurs/secours",
        &autre,
        &corps_enrolement(&jeton, MOT_DE_PASSE, &code_a),
    )
    .await;
    assert_eq!(r.json()["erreur"], "jeton_enrolement_invalide");
}

async fn cas_enrolement(c: &Ctx<'_>) {
    let Ctx {
        env,
        horloge,
        routeur,
        m,
        sa,
        sb,
        s_multi,
        ..
    } = c;
    let (a, b) = (m.a.administrateur, m.b.administrateur);
    installer_secours(&env.admin, b, MOT_DE_PASSE_B).await;
    let secours_b = secours_de(&env.admin, b).await;
    let autre_a = ouvrir(env, horloge, a, NatureSession::Complete).await;
    let (secret, jeton) = preparer(routeur, sa).await;
    let r = poster(
        routeur,
        "/api/moi/facteurs/secours",
        sa,
        &corps_enrolement(&jeton, MOT_DE_PASSE, &code(&secret, horloge.maintenant())),
    )
    .await;
    assert_eq!(r.statut, StatusCode::CREATED, "{}", r.texte());
    // Le secours de B est intact ; seules les autres sessions de A sont révoquées.
    assert_eq!(secours_de(&env.admin, b).await, secours_b);
    assert_eq!(session_vue(routeur, &autre_a).await, None);
    assert!(session_vue(routeur, sa).await.is_some());
    assert!(session_vue(routeur, sb).await.is_some());
    assert!(session_vue(routeur, s_multi).await.is_some());
}

async fn cas_regeneration(c: &Ctx<'_>) {
    let Ctx {
        env,
        routeur,
        m,
        sa,
        sb,
        ..
    } = c;
    let b = m.b.administrateur;
    installer_codes(&env.admin, b).await;
    let r = poster(routeur, "/api/moi/facteurs/codes-recuperation", sa, "").await;
    assert_eq!(r.statut, StatusCode::CREATED, "{}", r.texte());
    assert_eq!(codes_inutilises(&env.admin, m.a.administrateur).await, 10);
    assert_eq!(codes_inutilises(&env.admin, b).await, 10);
    assert!(session_vue(routeur, sb).await.is_some());
}

/// Cedar forcé à « autorisé » : la nature de la session ouverte par un code reste fixée par
/// le code (restreinte), la portée des routes la borne, et la RLS et le filtre des requêtes
/// gardent les facteurs et les événements de chacun.
#[tokio::test]
async fn les_facteurs_restent_bornes_quand_cedar_se_trompe() {
    let env = super::commun::env().await;
    let m = monde(&env.admin).await;
    let (a, b) = (m.a.administrateur, m.b.administrateur);
    let horloge = HorlogeTest::alignee_sur_un_pas();
    let routeur = api::serveur::routeur(&application_sessions(
        &env,
        &horloge,
        Arc::new(ToujoursAutorise),
    ));
    let codes_a = installer_codes(&env.admin, a).await;
    installer_secours(&env.admin, a, MOT_DE_PASSE).await;
    retirer_facteurs(&env.admin, b).await;

    // Un code n'ouvre qu'une session restreinte, sans accès métier ni à ses ressources.
    let r = connecter(
        &routeur,
        "/api/connexion/code-recuperation",
        &corps_code(&courriel(a), &codes_a[0]),
        "test",
    )
    .await;
    assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
    assert_eq!(r.json()["nature"], "restreinte");
    let restreinte = cookie_de_reponse(&r);
    let csrf = r.json()["csrf"].as_str().unwrap().to_owned();
    for chemin in [
        format!("/api/cabinets/{}", m.a.cabinet),
        "/api/moi/cabinets".to_owned(),
        "/api/moi/evenements".to_owned(),
    ] {
        let r = appeler_session_corps(&routeur, "GET", &chemin, Some(&restreinte), None, "").await;
        assert_eq!(r.statut, StatusCode::UNAUTHORIZED, "{chemin}");
    }
    let r = appeler_session_corps(
        &routeur,
        "POST",
        "/api/session/reauthentification",
        Some(&restreinte),
        Some(&csrf),
        "{}",
    )
    .await;
    assert_eq!(r.statut, StatusCode::UNAUTHORIZED);

    // B ne voit ni le secours, ni les codes, ni les événements de A.
    let sb = ouvrir(&env, &horloge, b, NatureSession::Complete).await;
    let r = lire_s(&routeur, "/api/moi/facteurs", &sb).await;
    assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
    assert_eq!(r.json()["secours"], serde_json::Value::Null);
    assert_eq!(r.json()["codes_restants"], 0);
    let r = lire_s(&routeur, "/api/moi/evenements", &sb).await;
    assert_eq!(r.json().as_array().unwrap().len(), 0, "{}", r.texte());
    // B régénère ses codes : ceux de A restent.
    let r = poster(&routeur, "/api/moi/facteurs/codes-recuperation", &sb, "").await;
    assert_eq!(r.statut, StatusCode::CREATED, "{}", r.texte());
    assert_eq!(codes_inutilises(&env.admin, a).await, 9);
    // Le jeton d'enrôlement de A ne vaut rien pour B, même si Cedar autorise tout.
    let sa = ouvrir(&env, &horloge, a, NatureSession::Complete).await;
    let (secret, jeton) = preparer(&routeur, &sa).await;
    let r = poster(
        &routeur,
        "/api/moi/facteurs/secours",
        &sb,
        &corps_enrolement(&jeton, MOT_DE_PASSE, &code(&secret, horloge.maintenant())),
    )
    .await;
    assert_eq!(r.statut, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(secours_de(&env.admin, b).await.is_none());
}
