//! Cas d'isolation des routes du module `passkeys` (PR 8c ; ADR 0012) : passkeys et cérémonies
//! WebAuthn. Sessions réelles de deux utilisateurs de cabinets différents (A, B) et du membre des
//! deux cabinets, avec un authentificateur logiciel par utilisateur : aucun ne voit, n'utilise,
//! ne termine ni ne supprime la passkey ou la cérémonie d'un autre.
//!
//! « Cedar se trompe » : [`les_passkeys_restent_bornees_quand_cedar_se_trompe`].
// Fichier de test : A, B, a, b, d, r y sont les noms usuels des deux mondes et des réponses.
#![allow(clippy::many_single_char_names)]

use std::sync::Arc;

use api::base::IdUtilisateur;
use api::base::session::NatureSession;
use api::serveur::routes::{Methode, Route};
use axum::Router;
use axum::http::StatusCode;
use sqlx::PgPool;
use uuid::Uuid;
use webauthn_authenticator_rs::WebauthnAuthenticator;
use webauthn_authenticator_rs::softpasskey::SoftPasskey;
use webauthn_rs::prelude::{CreationChallengeResponse, RequestChallengeResponse, Url};

use super::commun::facteurs::courriel;
use super::commun::serveur::{
    HorlogeTest, ORIGINE, Reponse, ToujoursAutorise, appeler, appeler_session_corps,
    application_sessions, cookie_de, ouvrir,
};
use super::{CasModule, Ctx, Scenario, lire_s, monde};
use api::serveur::session::SessionOuverte;

pub(super) const CAS: CasModule = CasModule {
    attentes: CasModule::AUCUN.attentes,
    hors_cabinet,
};

fn hors_cabinet(route: &Route) -> Option<Scenario> {
    Some(match (route.methode, route.chemin) {
        (Methode::Post, "/api/connexion/passkey/defi") => |c| Box::pin(cas_defi_connexion(c)),
        (Methode::Post, "/api/connexion/passkey") => |c| Box::pin(cas_connexion(c)),
        (Methode::Post, "/api/session/reauthentification/passkey/defi") => {
            |c| Box::pin(cas_defi_reauthentification(c))
        }
        (Methode::Post, "/api/session/reauthentification/passkey") => {
            |c| Box::pin(cas_reauthentification(c))
        }
        (Methode::Post, "/api/moi/facteurs/passkeys/defi") => |c| Box::pin(cas_defi_enrolement(c)),
        (Methode::Post, "/api/moi/facteurs/passkeys") => |c| Box::pin(cas_enrolement(c)),
        (Methode::Delete, "/api/moi/facteurs/passkeys/{passkey}") => {
            |c| Box::pin(cas_suppression(c))
        }
        _ => return None,
    })
}

fn origine() -> Url {
    Url::parse(ORIGINE).unwrap()
}

/// Requête au nom d'une session réelle (jeton anti-CSRF) avec, en plus, un cookie de cérémonie.
async fn appeler_s(
    routeur: &Router,
    methode: &str,
    chemin: &str,
    s: &SessionOuverte,
    ceremonie: Option<&str>,
    corps: &str,
) -> Reponse {
    let cookie = match ceremonie {
        Some(c) => format!("{}; {c}", cookie_de(s)),
        None => cookie_de(s),
    };
    appeler_session_corps(
        routeur,
        methode,
        chemin,
        Some(&cookie),
        Some(&s.jeton.csrf()),
        corps,
    )
    .await
}

/// `POST` sans session sur une route de connexion (mêmes marques que les autres connexions).
async fn anonyme(routeur: &Router, chemin: &str, corps: &str, cookie: Option<&str>) -> Reponse {
    let mut requete = axum::http::Request::post(chemin)
        .header("origin", ORIGINE)
        .header("sec-fetch-site", "same-origin")
        .header("x-sigma-csrf", "1")
        .header("content-type", "application/json");
    if let Some(c) = cookie {
        requete = requete.header("cookie", c);
    }
    appeler(
        routeur,
        requete
            .body(axum::body::Body::from(corps.to_owned()))
            .unwrap(),
    )
    .await
}

fn cookie_ceremonie(r: &Reponse) -> String {
    r.entetes
        .get_all("set-cookie")
        .iter()
        .filter_map(|v| v.to_str().ok())
        .find(|v| v.starts_with("__Host-sigma_ceremonie=") && !v.contains("Max-Age=0"))
        .expect("cookie de cérémonie")
        .split(';')
        .next()
        .unwrap()
        .to_owned()
}

/// Enrôle une passkey de l'authentificateur `auth` au nom de `s` ; rend son identifiant.
async fn enroler(routeur: &Router, auth: &mut SoftPasskey, s: &SessionOuverte) -> String {
    let d = appeler_s(
        routeur,
        "POST",
        "/api/moi/facteurs/passkeys/defi",
        s,
        None,
        "",
    )
    .await;
    assert_eq!(d.statut, StatusCode::OK, "{}", d.texte());
    let options: CreationChallengeResponse =
        serde_json::from_value(d.json()["defi"].clone()).unwrap();
    let credential = auth.do_registration(origine(), options).unwrap();
    let corps = serde_json::json!({ "nom": "Test", "credential": credential }).to_string();
    let r = appeler_s(
        routeur,
        "POST",
        "/api/moi/facteurs/passkeys",
        s,
        Some(&cookie_ceremonie(&d)),
        &corps,
    )
    .await;
    assert_eq!(r.statut, StatusCode::CREATED, "{}", r.texte());
    r.json()["id"].as_str().unwrap().to_owned()
}

/// Retire les passkeys et les cérémonies d'un utilisateur (le monde de test est partagé).
async fn nettoyer(admin: &PgPool, utilisateur: IdUtilisateur) {
    for sql in [
        "DELETE FROM defi_webauthn WHERE utilisateur_id = $1",
        "DELETE FROM facteur_passkey WHERE utilisateur_id = $1",
    ] {
        sqlx::query(sql)
            .bind(utilisateur.uuid())
            .execute(admin)
            .await
            .unwrap();
    }
}

async fn passkeys_de(admin: &PgPool, utilisateur: IdUtilisateur) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM facteur_passkey WHERE utilisateur_id = $1")
        .bind(utilisateur.uuid())
        .fetch_one(admin)
        .await
        .unwrap()
}

async fn defis_de(admin: &PgPool, utilisateur: IdUtilisateur) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM defi_webauthn WHERE utilisateur_id = $1")
        .bind(utilisateur.uuid())
        .fetch_one(admin)
        .await
        .unwrap()
}

/// Défi de connexion de `courriel`, puis la réponse de `auth` : (cookie de cérémonie, corps de la
/// connexion au nom de `courriel_presente`).
async fn preparer_connexion(
    routeur: &Router,
    auth: &mut SoftPasskey,
    courriel_defi: &str,
    courriel_presente: &str,
) -> (String, String) {
    let d = anonyme(
        routeur,
        "/api/connexion/passkey/defi",
        &serde_json::json!({ "courriel": courriel_defi }).to_string(),
        None,
    )
    .await;
    assert_eq!(d.statut, StatusCode::OK, "{}", d.texte());
    let options: RequestChallengeResponse =
        serde_json::from_value(d.json()["defi"].clone()).unwrap();
    let credential = auth.do_authentication(origine(), options).unwrap();
    (
        cookie_ceremonie(&d),
        serde_json::json!({ "courriel": courriel_presente, "credential": credential }).to_string(),
    )
}

async fn titulaire(admin: &PgPool, reponse: &Reponse) -> Uuid {
    assert_eq!(reponse.statut, StatusCode::OK, "{}", reponse.texte());
    sqlx::query_scalar("SELECT utilisateur_id FROM session WHERE id = $1::uuid")
        .bind(reponse.json()["session"].as_str().unwrap())
        .fetch_one(admin)
        .await
        .unwrap()
}

async fn cas_defi_connexion(c: &Ctx<'_>) {
    let Ctx {
        env,
        routeur,
        m,
        sa,
        sb,
        ..
    } = c;
    let (a, b) = (m.a.administrateur, m.b.administrateur);
    nettoyer(&env.admin, a).await;
    nettoyer(&env.admin, b).await;
    let ida = enroler(routeur, &mut SoftPasskey::new(true), sa).await;
    enroler(routeur, &mut SoftPasskey::new(true), sb).await;
    // Le défi du courriel de A ne cite que les passkeys de A, et ne crée d'état que pour A.
    let d = anonyme(
        routeur,
        "/api/connexion/passkey/defi",
        &serde_json::json!({ "courriel": courriel(a) }).to_string(),
        None,
    )
    .await;
    assert_eq!(d.statut, StatusCode::OK, "{}", d.texte());
    let autorisees = d.json()["defi"]["publicKey"]["allowCredentials"]
        .as_array()
        .unwrap()
        .len();
    assert_eq!(autorisees, 1);
    assert_eq!(defis_de(&env.admin, a).await, 1);
    assert_eq!(defis_de(&env.admin, b).await, 0);
    assert!(
        !d.texte().contains(&ida),
        "identifiant interne jamais rendu"
    );
}

async fn cas_connexion(c: &Ctx<'_>) {
    let Ctx {
        env,
        routeur,
        m,
        sa,
        sb,
        ..
    } = c;
    let (a, b) = (m.a.administrateur, m.b.administrateur);
    nettoyer(&env.admin, a).await;
    nettoyer(&env.admin, b).await;
    let (mut auth_a, mut auth_b) = (SoftPasskey::new(true), SoftPasskey::new(true));
    enroler(routeur, &mut auth_a, sa).await;
    enroler(routeur, &mut auth_b, sb).await;
    // La passkey de B ne vaut rien au nom du courriel de A, et inversement.
    let (cookie, corps) =
        preparer_connexion(routeur, &mut auth_b, &courriel(b), &courriel(a)).await;
    let r = anonyme(routeur, "/api/connexion/passkey", &corps, Some(&cookie)).await;
    assert_eq!(r.statut, StatusCode::UNAUTHORIZED, "{}", r.texte());
    let (cookie_a, _) = preparer_connexion(routeur, &mut auth_a, &courriel(a), &courriel(a)).await;
    let (_, corps_b) = preparer_connexion(routeur, &mut auth_b, &courriel(b), &courriel(a)).await;
    let r = anonyme(routeur, "/api/connexion/passkey", &corps_b, Some(&cookie_a)).await;
    assert_eq!(r.statut, StatusCode::UNAUTHORIZED, "{}", r.texte());
    // La sienne ouvre une session de A, et seulement de A.
    let (cookie, corps) =
        preparer_connexion(routeur, &mut auth_a, &courriel(a), &courriel(a)).await;
    let r = anonyme(routeur, "/api/connexion/passkey", &corps, Some(&cookie)).await;
    assert_eq!(titulaire(&env.admin, &r).await, a.uuid());
    // Sans corps, sans cookie : rien.
    let r = anonyme(routeur, "/api/connexion/passkey", "", None).await;
    assert_eq!(r.statut, StatusCode::UNAUTHORIZED);
}

async fn cas_defi_reauthentification(c: &Ctx<'_>) {
    let Ctx {
        env,
        routeur,
        m,
        sa,
        sb,
        ..
    } = c;
    let (a, b) = (m.a.administrateur, m.b.administrateur);
    nettoyer(&env.admin, a).await;
    nettoyer(&env.admin, b).await;
    // B n'a aucune passkey : pas de défi, quoi que A possède.
    enroler(routeur, &mut SoftPasskey::new(true), sa).await;
    let r = appeler_s(
        routeur,
        "POST",
        "/api/session/reauthentification/passkey/defi",
        sb,
        None,
        "",
    )
    .await;
    assert_eq!(r.statut, StatusCode::UNPROCESSABLE_ENTITY, "{}", r.texte());
    assert_eq!(r.json()["erreur"], "aucune_passkey");
    assert_eq!(defis_de(&env.admin, b).await, 0);
    let r = appeler_s(
        routeur,
        "POST",
        "/api/session/reauthentification/passkey/defi",
        sa,
        None,
        "",
    )
    .await;
    assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
    assert_eq!(
        r.json()["defi"]["publicKey"]["allowCredentials"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    // La cérémonie est celle de la session de A.
    let session_du_defi: Option<Uuid> = sqlx::query_scalar(
        "SELECT session_id FROM defi_webauthn WHERE utilisateur_id = $1 AND nature = 'reauthentification'",
    )
    .bind(a.uuid())
    .fetch_one(&env.admin)
    .await
    .unwrap();
    assert_eq!(session_du_defi, Some(sa.id.uuid()));
}

async fn cas_reauthentification(c: &Ctx<'_>) {
    let Ctx {
        env,
        routeur,
        m,
        sa,
        sb,
        ..
    } = c;
    let (a, b) = (m.a.administrateur, m.b.administrateur);
    nettoyer(&env.admin, a).await;
    nettoyer(&env.admin, b).await;
    let (mut auth_a, mut auth_b) = (SoftPasskey::new(true), SoftPasskey::new(true));
    enroler(routeur, &mut auth_a, sa).await;
    enroler(routeur, &mut auth_b, sb).await;
    let chemin_defi = "/api/session/reauthentification/passkey/defi";
    let chemin = "/api/session/reauthentification/passkey";
    // Défi de A : la réponse de A présentée par la session de B est refusée, et B n'est pas touché.
    let d = appeler_s(routeur, "POST", chemin_defi, sa, None, "").await;
    let options: RequestChallengeResponse =
        serde_json::from_value(d.json()["defi"].clone()).unwrap();
    let credential = auth_a.do_authentication(origine(), options).unwrap();
    let corps = serde_json::json!({ "credential": credential }).to_string();
    let r = appeler_s(
        routeur,
        "POST",
        chemin,
        sb,
        Some(&cookie_ceremonie(&d)),
        &corps,
    )
    .await;
    assert_eq!(r.statut, StatusCode::UNPROCESSABLE_ENTITY, "{}", r.texte());
    assert_eq!(r.json()["erreur"], "facteurs_refuses");
    assert_eq!(
        super::session_vue(routeur, sb).await,
        Some(sb.id.to_string())
    );
    // Défi de B signé par A (A n'a pas la clé de B) : l'authentificateur de A refuse déjà.
    let d = appeler_s(routeur, "POST", chemin_defi, sb, None, "").await;
    let options: RequestChallengeResponse =
        serde_json::from_value(d.json()["defi"].clone()).unwrap();
    assert!(auth_a.do_authentication(origine(), options).is_err());
    // La sienne ré-authentifie A, et seulement A.
    let d = appeler_s(routeur, "POST", chemin_defi, sa, None, "").await;
    let options: RequestChallengeResponse =
        serde_json::from_value(d.json()["defi"].clone()).unwrap();
    let credential = auth_a.do_authentication(origine(), options).unwrap();
    let corps = serde_json::json!({ "credential": credential }).to_string();
    let r = appeler_s(
        routeur,
        "POST",
        chemin,
        sa,
        Some(&cookie_ceremonie(&d)),
        &corps,
    )
    .await;
    assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
    assert_eq!(
        super::session_vue(routeur, sa).await,
        None,
        "le jeton de A a tourné"
    );
    assert_eq!(
        super::session_vue(routeur, sb).await,
        Some(sb.id.to_string())
    );
}

async fn cas_defi_enrolement(c: &Ctx<'_>) {
    let Ctx {
        env,
        routeur,
        m,
        sa,
        sb,
        s_multi,
        ..
    } = c;
    let (a, b) = (m.a.administrateur, m.b.administrateur);
    nettoyer(&env.admin, a).await;
    nettoyer(&env.admin, b).await;
    enroler(routeur, &mut SoftPasskey::new(true), sa).await;
    // Le défi de B n'exclut aucune passkey de A (aucune fuite d'identifiants entre comptes).
    let d = appeler_s(
        routeur,
        "POST",
        "/api/moi/facteurs/passkeys/defi",
        sb,
        None,
        "",
    )
    .await;
    assert_eq!(d.statut, StatusCode::OK, "{}", d.texte());
    let options: CreationChallengeResponse =
        serde_json::from_value(d.json()["defi"].clone()).unwrap();
    assert_eq!(
        options
            .public_key
            .exclude_credentials
            .unwrap_or_default()
            .len(),
        0
    );
    assert_eq!(defis_de(&env.admin, b).await, 1);
    // Le défi n'est créé que pour l'utilisateur de la session, même pour un membre de deux cabinets.
    let multi = lire_s(routeur, "/api/moi/facteurs", s_multi).await;
    assert_eq!(multi.json()["passkeys"].as_array().unwrap().len(), 0);
}

async fn cas_enrolement(c: &Ctx<'_>) {
    let Ctx {
        env,
        routeur,
        m,
        sa,
        sb,
        ..
    } = c;
    let (a, b) = (m.a.administrateur, m.b.administrateur);
    nettoyer(&env.admin, a).await;
    nettoyer(&env.admin, b).await;
    // Cérémonie de A, terminée depuis la session de B : refusée, rien enregistré, et la
    // cérémonie de A reste utilisable par A.
    let d = appeler_s(
        routeur,
        "POST",
        "/api/moi/facteurs/passkeys/defi",
        sa,
        None,
        "",
    )
    .await;
    let options: CreationChallengeResponse =
        serde_json::from_value(d.json()["defi"].clone()).unwrap();
    let mut auth = SoftPasskey::new(true);
    let credential = auth.do_registration(origine(), options).unwrap();
    let corps = serde_json::json!({ "nom": "Volée", "credential": credential }).to_string();
    let ceremonie = cookie_ceremonie(&d);
    let r = appeler_s(
        routeur,
        "POST",
        "/api/moi/facteurs/passkeys",
        sb,
        Some(&ceremonie),
        &corps,
    )
    .await;
    assert_eq!(r.statut, StatusCode::UNPROCESSABLE_ENTITY, "{}", r.texte());
    assert_eq!(r.json()["erreur"], "passkey_refusee");
    assert_eq!(passkeys_de(&env.admin, b).await, 0);
    assert_eq!(passkeys_de(&env.admin, a).await, 0);
    let r = appeler_s(
        routeur,
        "POST",
        "/api/moi/facteurs/passkeys",
        sa,
        Some(&ceremonie),
        &corps,
    )
    .await;
    assert_eq!(r.statut, StatusCode::CREATED, "{}", r.texte());
    assert_eq!(passkeys_de(&env.admin, a).await, 1);
    assert_eq!(passkeys_de(&env.admin, b).await, 0);
}

async fn cas_suppression(c: &Ctx<'_>) {
    let Ctx {
        env,
        routeur,
        m,
        sa,
        sb,
        s_multi,
        ..
    } = c;
    let (a, b) = (m.a.administrateur, m.b.administrateur);
    nettoyer(&env.admin, a).await;
    nettoyer(&env.admin, b).await;
    let ida = enroler(routeur, &mut SoftPasskey::new(true), sa).await;
    enroler(routeur, &mut SoftPasskey::new(true), sa).await;
    enroler(routeur, &mut SoftPasskey::new(true), sb).await;
    // Ni B, ni le membre des deux cabinets ne supprime une passkey de A : introuvable.
    for s in [sb, s_multi] {
        let r = appeler_s(
            routeur,
            "DELETE",
            &format!("/api/moi/facteurs/passkeys/{ida}"),
            s,
            None,
            "",
        )
        .await;
        assert_eq!(r.statut, StatusCode::NOT_FOUND, "{}", r.texte());
    }
    assert_eq!(passkeys_de(&env.admin, a).await, 2);
    assert_eq!(passkeys_de(&env.admin, b).await, 1);
    // A supprime la sienne.
    let r = appeler_s(
        routeur,
        "DELETE",
        &format!("/api/moi/facteurs/passkeys/{ida}"),
        sa,
        None,
        "",
    )
    .await;
    assert_eq!(r.statut, StatusCode::NO_CONTENT, "{}", r.texte());
    assert_eq!(passkeys_de(&env.admin, a).await, 1);
    assert_eq!(passkeys_de(&env.admin, b).await, 1);
}

#[tokio::test]
async fn les_passkeys_restent_bornees_quand_cedar_se_trompe() {
    let env = super::commun::env().await;
    let m = monde(&env.admin).await;
    let (a, b) = (m.a.administrateur, m.b.administrateur);
    let horloge = HorlogeTest::default();
    let routeur = api::serveur::routeur(&application_sessions(
        &env,
        &horloge,
        Arc::new(ToujoursAutorise),
    ));
    let sa = ouvrir(&env, &horloge, a, NatureSession::Complete).await;
    let sb = ouvrir(&env, &horloge, b, NatureSession::Complete).await;
    nettoyer(&env.admin, a).await;
    nettoyer(&env.admin, b).await;
    let (mut auth_a, mut auth_b) = (SoftPasskey::new(true), SoftPasskey::new(true));
    let ida = enroler(&routeur, &mut auth_a, &sa).await;
    enroler(&routeur, &mut auth_b, &sb).await;

    // B ne voit pas la passkey de A, ne la supprime pas (RLS), n'ouvre pas de session de A avec la sienne.
    let r = lire_s(&routeur, "/api/moi/facteurs", &sb).await;
    assert_eq!(r.json()["passkeys"].as_array().unwrap().len(), 1);
    assert!(!r.texte().contains(&ida));
    let r = appeler_s(
        &routeur,
        "DELETE",
        &format!("/api/moi/facteurs/passkeys/{ida}"),
        &sb,
        None,
        "",
    )
    .await;
    assert_eq!(r.statut, StatusCode::NOT_FOUND, "{}", r.texte());
    assert_eq!(passkeys_de(&env.admin, a).await, 1);
    let (cookie, corps) =
        preparer_connexion(&routeur, &mut auth_b, &courriel(b), &courriel(a)).await;
    let r = anonyme(&routeur, "/api/connexion/passkey", &corps, Some(&cookie)).await;
    assert_eq!(r.statut, StatusCode::UNAUTHORIZED, "{}", r.texte());
    // Une session restreinte reste cantonnée à l'enrôlement, même si Cedar autorise tout.
    let restreinte = ouvrir(&env, &horloge, a, NatureSession::Restreinte).await;
    for (methode, chemin) in [
        (
            "POST",
            "/api/session/reauthentification/passkey/defi".to_owned(),
        ),
        ("DELETE", format!("/api/moi/facteurs/passkeys/{ida}")),
    ] {
        let r = appeler_s(&routeur, methode, &chemin, &restreinte, None, "").await;
        assert_eq!(r.statut, StatusCode::UNAUTHORIZED, "{chemin}");
    }
}
