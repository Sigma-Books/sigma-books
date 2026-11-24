//! Passkeys, WebAuthn (ADR 0006, ADR 0012, ADR 0018 ; L2 PR 8c ; F1.1, F1.6, F1.12) : enrôlement,
//! connexion, ré-authentification et suppression de bout en bout avec un **authentificateur
//! logiciel** (`SoftPasskey`) qui signe vraiment les défis ; défi à usage unique, expiré, d'une
//! autre cérémonie ou d'un autre compte ; vérification de l'utilisateur exigée ; anti-énumération ;
//! session restreinte ; RLS de la connexion ; événements d'authentification et audit des
//! cabinets.
//!
//! Les cas d'isolation de chaque route sont dans `isolation/facteurs.rs`, avec ceux de toutes
//! les routes.
#![cfg(feature = "tests-postgres")]
// Fichier de test : les `expect`/`unwrap` y sont voulus.
#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::many_single_char_names
)]

mod commun;

use std::sync::Arc;

use api::base::IdUtilisateur;
use api::base::session::NatureSession;
use api::serveur::Reglages;
use authz::Autorisation;
use axum::Router;
use axum::http::StatusCode;
use commun::facteurs::{courriel, installer_codes, installer_secours};
use commun::serveur::{
    HorlogeTest, ORIGINE, Reponse, appeler, appeler_session_corps, application_sessions_avec,
    cookie_de, ouvrir, reglages_de_test, routeur_de,
};
use commun::{CabinetTest, creer_cabinet, env};
use sqlx::PgPool;
use time::Duration;
use uuid::Uuid;
use webauthn_authenticator_rs::WebauthnAuthenticator;
use webauthn_authenticator_rs::softpasskey::SoftPasskey;
use webauthn_rs::prelude::{
    CreationChallengeResponse, PublicKeyCredential, RegisterPublicKeyCredential,
    RequestChallengeResponse, Url,
};

const DEFI_CONNEXION: &str = "/api/connexion/passkey/defi";
const CONNEXION: &str = "/api/connexion/passkey";
const DEFI_ENROLEMENT: &str = "/api/moi/facteurs/passkeys/defi";
const ENROLEMENT: &str = "/api/moi/facteurs/passkeys";
const DEFI_REAUTH: &str = "/api/session/reauthentification/passkey/defi";
const REAUTH: &str = "/api/session/reauthentification/passkey";
const CEREMONIE: &str = "__Host-sigma_ceremonie";

fn origine() -> Url {
    Url::parse(ORIGINE).expect("origine")
}

/// Réglages de test aux débits larges : ces tests enchaînent bien plus d'appels que la limite
/// d'un utilisateur réel (le débit a ses propres tests dans `secours.rs`).
fn reglages_larges() -> Reglages {
    let mut reglages = reglages_de_test();
    for debit in [
        &mut reglages.facteurs.debit_connexion_adresse,
        &mut reglages.facteurs.debit_connexion_courriel,
        &mut reglages.facteurs.debit_verification,
    ] {
        debit.capacite = 10_000;
    }
    reglages
}

struct Banc {
    env: commun::Env,
    horloge: HorlogeTest,
    routeur: Router,
    c: CabinetTest,
}

async fn banc() -> Banc {
    let env = env().await;
    let c = creer_cabinet(&env.admin, "Cabinet").await;
    let horloge = HorlogeTest::default();
    let etat = application_sessions_avec(
        &env,
        &horloge,
        Arc::new(Autorisation::charger().unwrap()),
        reglages_larges(),
    );
    Banc {
        env,
        horloge,
        routeur: routeur_de(&etat),
        c,
    }
}

/// Session de l'utilisateur : (cookie, jeton anti-CSRF).
type Session = (String, String);

async fn session(b: &Banc, utilisateur: IdUtilisateur, nature: NatureSession) -> Session {
    let ouverte = ouvrir(&b.env, &b.horloge, utilisateur, nature).await;
    (cookie_de(&ouverte), ouverte.jeton.csrf())
}

async fn envoyer(
    b: &Banc,
    methode: &str,
    chemin: &str,
    cookie: Option<&str>,
    csrf: Option<&str>,
    corps: &str,
) -> Reponse {
    appeler_session_corps(&b.routeur, methode, chemin, cookie, csrf, corps).await
}

/// Cookie de cérémonie posé par une réponse (le dernier `Set-Cookie` qui n'efface pas).
fn cookie_ceremonie(r: &Reponse) -> String {
    r.entetes
        .get_all("set-cookie")
        .iter()
        .filter_map(|v| v.to_str().ok())
        .find(|v| v.starts_with(&format!("{CEREMONIE}=")) && !v.contains("Max-Age=0"))
        .unwrap_or_else(|| panic!("cookie de cérémonie absent : {:?}", r.entetes))
        .split(';')
        .next()
        .unwrap()
        .to_owned()
}

fn cookies(session: &Session, ceremonie: &str) -> String {
    format!("{}; {ceremonie}", session.0)
}

/// Enrôlement complet d'une passkey au nom de `session`, avec l'authentificateur `auth`.
async fn enroler(b: &Banc, auth: &mut SoftPasskey, s: &Session, nom: &str) -> Reponse {
    let d = envoyer(b, "POST", DEFI_ENROLEMENT, Some(&s.0), Some(&s.1), "").await;
    assert_eq!(d.statut, StatusCode::OK, "{}", d.texte());
    let ceremonie = cookie_ceremonie(&d);
    let options: CreationChallengeResponse =
        serde_json::from_value(d.json()["defi"].clone()).expect("options d'enrôlement");
    let credential: RegisterPublicKeyCredential = auth
        .do_registration(origine(), options)
        .expect("enregistrement");
    let corps = serde_json::json!({ "nom": nom, "credential": credential }).to_string();
    envoyer(
        b,
        "POST",
        ENROLEMENT,
        Some(&cookies(s, &ceremonie)),
        Some(&s.1),
        &corps,
    )
    .await
}

/// `POST` sans session sur une route de connexion : mêmes marques que `commun::facteurs::connecter`
/// (même origine, en-tête anti-CSRF de connexion), avec en plus le cookie de cérémonie.
async fn anonyme(b: &Banc, chemin: &str, corps: &str, cookie: Option<&str>) -> Reponse {
    let mut requete = axum::http::Request::post(chemin)
        .header("origin", ORIGINE)
        .header("sec-fetch-site", "same-origin")
        .header("x-sigma-csrf", "1")
        .header("content-type", "application/json")
        .header("user-agent", "Navigateur de test");
    if let Some(c) = cookie {
        requete = requete.header("cookie", c);
    }
    appeler(
        &b.routeur,
        requete
            .body(axum::body::Body::from(corps.to_owned()))
            .unwrap(),
    )
    .await
}

async fn envoyer_connexion(b: &Banc, corps: &str, cookie: Option<&str>) -> Reponse {
    anonyme(b, CONNEXION, corps, cookie).await
}

/// Défi de connexion pour `courriel`, puis la réponse de l'authentificateur : rend (défi,
/// cookie de cérémonie, credential signé si l'authentificateur a pu répondre).
async fn defi_connexion(
    b: &Banc,
    auth: &mut SoftPasskey,
    courriel: &str,
) -> (Reponse, String, Option<PublicKeyCredential>) {
    let d = anonyme(
        b,
        DEFI_CONNEXION,
        &serde_json::json!({ "courriel": courriel }).to_string(),
        None,
    )
    .await;
    assert_eq!(d.statut, StatusCode::OK, "{}", d.texte());
    let ceremonie = cookie_ceremonie(&d);
    let options: RequestChallengeResponse =
        serde_json::from_value(d.json()["defi"].clone()).expect("options d'authentification");
    let credential = auth.do_authentication(origine(), options).ok();
    (d, ceremonie, credential)
}

fn corps_connexion(courriel: &str, credential: &PublicKeyCredential) -> String {
    serde_json::json!({ "courriel": courriel, "credential": credential }).to_string()
}

/// Connexion complète par passkey.
async fn connexion(b: &Banc, auth: &mut SoftPasskey, courriel: &str) -> Reponse {
    let (_, ceremonie, credential) = defi_connexion(b, auth, courriel).await;
    let credential = credential.expect("l'authentificateur connaît cette passkey");
    envoyer_connexion(b, &corps_connexion(courriel, &credential), Some(&ceremonie)).await
}

async fn compter(admin: &PgPool, sql: &'static str, id: Uuid) -> i64 {
    sqlx::query_scalar(sql)
        .bind(id)
        .fetch_one(admin)
        .await
        .unwrap()
}

async fn passkeys_de(b: &Banc, u: IdUtilisateur) -> i64 {
    compter(
        &b.env.admin,
        "SELECT count(*) FROM facteur_passkey WHERE utilisateur_id = $1",
        u.uuid(),
    )
    .await
}

async fn evenements(b: &Banc, u: IdUtilisateur, nature: &str, issue: &str) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM evenement_authentification \
         WHERE utilisateur_id = $1 AND nature = $2 AND issue = $3",
    )
    .bind(u.uuid())
    .bind(nature)
    .bind(issue)
    .fetch_one(&b.env.admin)
    .await
    .unwrap()
}

async fn audit_de(b: &Banc, u: IdUtilisateur, action: &str) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM audit_cabinet WHERE cabinet_id = $1 \
         AND contenu LIKE '%acteur=' || $2 || '%' AND contenu LIKE '%action=' || $3 || '%'",
    )
    .bind(b.c.cabinet.uuid())
    .bind(u.to_string())
    .bind(action)
    .fetch_one(&b.env.admin)
    .await
    .unwrap()
}

// --- Parcours complet ------------------------------------------------------------------------

#[tokio::test]
async fn enrolement_puis_connexion_par_passkey() {
    let b = banc().await;
    let u = b.c.administrateur;
    let s = session(&b, u, NatureSession::Complete).await;
    let mut auth = SoftPasskey::new(true);

    // Enrôlement : 201, la passkey est listée sans clé ni compteur.
    let r = enroler(&b, &mut auth, &s, "Téléphone").await;
    assert_eq!(r.statut, StatusCode::CREATED, "{}", r.texte());
    assert_eq!(r.json()["nom"], "Téléphone");
    assert_eq!(passkeys_de(&b, u).await, 1);
    let etat = envoyer(&b, "GET", "/api/moi/facteurs", Some(&s.0), None, "").await;
    assert_eq!(etat.statut, StatusCode::OK, "{}", etat.texte());
    let liste = etat.json()["passkeys"].as_array().unwrap().clone();
    assert_eq!(liste.len(), 1);
    assert_eq!(liste[0]["nom"], "Téléphone");
    assert_eq!(liste[0]["derniere_utilisation"], serde_json::Value::Null);
    let texte = etat.texte();
    for interdit in ["counter", "cred", "public", "\"cle\"", "identifiant"] {
        assert!(!texte.contains(interdit), "{interdit} dans {texte}");
    }
    // Un enrôlement révoque les autres sessions (ADR 0012), pas celle qui enrôle.
    let autre = session(&b, u, NatureSession::Complete).await;
    let r = enroler(&b, &mut SoftPasskey::new(true), &s, "Clé de secours").await;
    assert_eq!(r.statut, StatusCode::CREATED, "{}", r.texte());
    let r = envoyer(&b, "GET", "/api/session", Some(&autre.0), None, "").await;
    assert_eq!(r.statut, StatusCode::UNAUTHORIZED);
    let r = envoyer(&b, "GET", "/api/session", Some(&s.0), None, "").await;
    assert_eq!(r.statut, StatusCode::OK);

    // Connexion : session complète par passkey, cookie de cérémonie effacé, usage daté.
    let r = connexion(&b, &mut auth, &courriel(u)).await;
    assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
    assert_eq!(r.json()["nature"], "complete");
    assert_eq!(r.json()["methode"], "passkey");
    assert!(
        r.entetes
            .get_all("set-cookie")
            .iter()
            .any(|v| v.to_str().unwrap().starts_with(&format!("{CEREMONIE}=;")))
    );
    let utilisee: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM facteur_passkey WHERE utilisateur_id = $1 \
         AND derniere_utilisation IS NOT NULL",
    )
    .bind(u.uuid())
    .fetch_one(&b.env.admin)
    .await
    .unwrap();
    assert_eq!(utilisee, 1);

    // Traces : événements de l'utilisateur et audit du cabinet, sans rien de secret.
    assert_eq!(evenements(&b, u, "enrolement_passkey", "succes").await, 2);
    assert_eq!(evenements(&b, u, "connexion", "succes").await, 1);
    assert_eq!(audit_de(&b, u, "EnrolerPasskey").await, 2);
    assert_eq!(audit_de(&b, u, "OuvrirSession").await, 1);
    let contenus: Vec<String> =
        sqlx::query_scalar("SELECT contenu FROM audit_cabinet WHERE cabinet_id = $1")
            .bind(b.c.cabinet.uuid())
            .fetch_all(&b.env.admin)
            .await
            .unwrap();
    assert!(contenus.iter().all(|c| !c.contains("credential")));
}

#[tokio::test]
async fn l_etat_stocke_ne_contient_aucun_secret_et_l_identifiant_est_unique() {
    let b = banc().await;
    let u = b.c.administrateur;
    let s = session(&b, u, NatureSession::Complete).await;
    let mut auth = SoftPasskey::new(true);
    assert_eq!(
        enroler(&b, &mut auth, &s, "Téléphone").await.statut,
        StatusCode::CREATED
    );
    let cle: String =
        sqlx::query_scalar("SELECT cle::text FROM facteur_passkey WHERE utilisateur_id = $1")
            .bind(u.uuid())
            .fetch_one(&b.env.admin)
            .await
            .unwrap();
    // La clé publique et le compteur, jamais de clé privée.
    assert!(!cle.to_lowercase().contains("private"), "{cle}");
    // Les passkeys existantes sont exclues du défi d'enrôlement suivant : un même authentificateur
    // ne s'enregistre pas deux fois.
    let d = envoyer(&b, "POST", DEFI_ENROLEMENT, Some(&s.0), Some(&s.1), "").await;
    let options: CreationChallengeResponse =
        serde_json::from_value(d.json()["defi"].clone()).unwrap();
    assert_eq!(
        options
            .public_key
            .exclude_credentials
            .unwrap_or_default()
            .len(),
        1
    );
    // Et la base interdit qu'un identifiant de credential serve à deux comptes.
    let doublon = sqlx::query(
        "INSERT INTO facteur_passkey (id, utilisateur_id, identifiant_credential, cle, nom, cree_le) \
         SELECT $1, $2, identifiant_credential, cle, 'Copie', now() FROM facteur_passkey \
         WHERE utilisateur_id = $3",
    )
    .bind(Uuid::new_v4())
    .bind(b.c.collaborateur.uuid())
    .bind(u.uuid())
    .execute(&b.env.admin)
    .await;
    assert!(
        doublon.is_err(),
        "identifiant de credential unique dans l'instance"
    );
    assert_eq!(passkeys_de(&b, u).await, 1);
}

// --- Rejeu, expiration, cérémonie d'un autre ---------------------------------------------------

#[tokio::test]
async fn une_reponse_rejouee_ou_un_defi_perime_ne_passe_pas() {
    let b = banc().await;
    let u = b.c.administrateur;
    let s = session(&b, u, NatureSession::Complete).await;
    let mut auth = SoftPasskey::new(true);
    assert_eq!(
        enroler(&b, &mut auth, &s, "T").await.statut,
        StatusCode::CREATED
    );
    let mail = courriel(u);

    // Rejeu : la même réponse, avec le même cookie, une seconde fois.
    let (_, ceremonie, credential) = defi_connexion(&b, &mut auth, &mail).await;
    let corps = corps_connexion(&mail, &credential.unwrap());
    let premiere = envoyer_connexion(&b, &corps, Some(&ceremonie)).await;
    assert_eq!(premiere.statut, StatusCode::OK, "{}", premiere.texte());
    let rejeu = envoyer_connexion(&b, &corps, Some(&ceremonie)).await;
    assert_eq!(rejeu.statut, StatusCode::UNAUTHORIZED, "{}", rejeu.texte());
    assert_eq!(rejeu.json()["erreur"], "non_authentifie");

    // Défi périmé : plus de 5 minutes entre le défi et la réponse.
    let (_, ceremonie, credential) = defi_connexion(&b, &mut auth, &mail).await;
    b.horloge.avancer(Duration::minutes(6));
    let r = envoyer_connexion(
        &b,
        &corps_connexion(&mail, &credential.unwrap()),
        Some(&ceremonie),
    )
    .await;
    assert_eq!(r.statut, StatusCode::UNAUTHORIZED, "{}", r.texte());

    // Sans cookie de cérémonie, ou avec un cookie inconnu, malformé ou double.
    let (_, ceremonie, credential) = defi_connexion(&b, &mut auth, &mail).await;
    let corps = corps_connexion(&mail, &credential.unwrap());
    let inconnu = format!("{CEREMONIE}={}", "A".repeat(43));
    let double = format!("{ceremonie}; {ceremonie}");
    for cookie in [
        None,
        Some(inconnu.as_str()),
        Some("x=1"),
        Some(double.as_str()),
    ] {
        let r = envoyer_connexion(&b, &corps, cookie).await;
        assert_eq!(
            r.statut,
            StatusCode::UNAUTHORIZED,
            "{cookie:?} : {}",
            r.texte()
        );
    }
    // La cérémonie n'est pas consommée par ces refus (le défi n'a pas été présenté) : la bonne
    // réponse passe encore.
    let r = envoyer_connexion(&b, &corps, Some(&ceremonie)).await;
    assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
    // Échecs inscrits : le rejeu, le défi périmé et le jeton inconnu (de forme valide) ; un cookie
    // absent, malformé ou double est refusé avant toute lecture et n'inscrit rien.
    assert_eq!(evenements(&b, u, "connexion", "echec").await, 3);
}

#[tokio::test]
async fn la_cerémonie_d_un_autre_compte_ou_d_un_autre_courriel_est_refusee() {
    let b = banc().await;
    let (a, autre) = (b.c.administrateur, b.c.collaborateur);
    let mut auth_a = SoftPasskey::new(true);
    let mut auth_autre = SoftPasskey::new(true);
    let sa = session(&b, a, NatureSession::Complete).await;
    let sb = session(&b, autre, NatureSession::Complete).await;
    assert_eq!(
        enroler(&b, &mut auth_a, &sa, "A").await.statut,
        StatusCode::CREATED
    );
    assert_eq!(
        enroler(&b, &mut auth_autre, &sb, "B").await.statut,
        StatusCode::CREATED
    );

    // Défi de A, réponse signée par A, mais présentée au nom du courriel de B.
    let (_, ceremonie_a, credential_a) = defi_connexion(&b, &mut auth_a, &courriel(a)).await;
    let r = envoyer_connexion(
        &b,
        &corps_connexion(&courriel(autre), &credential_a.clone().unwrap()),
        Some(&ceremonie_a),
    )
    .await;
    assert_eq!(r.statut, StatusCode::UNAUTHORIZED, "{}", r.texte());
    // Défi de B signé par l'authentificateur de A : A n'a pas la clé de B (refus à la source) ;
    // et la réponse de A au défi de A reste refusée avec le cookie de B.
    let (_, ceremonie_b, _) = defi_connexion(&b, &mut auth_autre, &courriel(autre)).await;
    let (_, _, credential_a) = defi_connexion(&b, &mut auth_a, &courriel(a)).await;
    let r = envoyer_connexion(
        &b,
        &corps_connexion(&courriel(a), &credential_a.unwrap()),
        Some(&ceremonie_b),
    )
    .await;
    assert_eq!(r.statut, StatusCode::UNAUTHORIZED, "{}", r.texte());
    assert_eq!(evenements(&b, a, "connexion", "succes").await, 0);
    assert_eq!(evenements(&b, autre, "connexion", "succes").await, 0);
}

#[tokio::test]
async fn deux_reponses_concurrentes_sur_le_meme_defi_une_seule_passe() {
    let b = banc().await;
    let u = b.c.administrateur;
    let s = session(&b, u, NatureSession::Complete).await;
    let mut auth = SoftPasskey::new(true);
    assert_eq!(
        enroler(&b, &mut auth, &s, "T").await.statut,
        StatusCode::CREATED
    );
    let mail = courriel(u);
    for _ in 0..3 {
        let (_, ceremonie, credential) = defi_connexion(&b, &mut auth, &mail).await;
        let corps = corps_connexion(&mail, &credential.unwrap());
        let (x, y) = tokio::join!(
            envoyer_connexion(&b, &corps, Some(&ceremonie)),
            envoyer_connexion(&b, &corps, Some(&ceremonie)),
        );
        let reussites = [x.statut, y.statut]
            .iter()
            .filter(|s| **s == StatusCode::OK)
            .count();
        assert_eq!(reussites, 1, "{} / {}", x.texte(), y.texte());
    }
}

// --- Vérification de l'utilisateur -------------------------------------------------------------

#[tokio::test]
async fn les_defis_exigent_la_verification_de_l_utilisateur() {
    let b = banc().await;
    let u = b.c.administrateur;
    let s = session(&b, u, NatureSession::Complete).await;
    // Un authentificateur qui ne vérifie pas l'utilisateur refuse lui-même la cérémonie ; côté
    // serveur, la vérification est exigée par chaque défi (et contrôlée sur le résultat).
    let d = envoyer(&b, "POST", DEFI_ENROLEMENT, Some(&s.0), Some(&s.1), "").await;
    let selection = &d.json()["defi"]["publicKey"]["authenticatorSelection"];
    assert_eq!(selection["userVerification"], "required", "{}", d.texte());
    assert!(
        SoftPasskey::new(false)
            .do_registration(
                origine(),
                serde_json::from_value(d.json()["defi"].clone()).unwrap()
            )
            .is_err()
    );
    let mut auth = SoftPasskey::new(true);
    assert_eq!(
        enroler(&b, &mut auth, &s, "T").await.statut,
        StatusCode::CREATED
    );
    let (d, _, _) = defi_connexion(&b, &mut auth, &courriel(u)).await;
    assert_eq!(
        d.json()["defi"]["publicKey"]["userVerification"],
        "required"
    );
}

// --- Anti-énumération ------------------------------------------------------------------------

#[tokio::test]
async fn le_defi_ne_dit_pas_si_le_compte_existe_ni_s_il_a_une_passkey() {
    let b = banc().await;
    let (avec, sans) = (b.c.administrateur, b.c.collaborateur);
    let s = session(&b, avec, NatureSession::Complete).await;
    let mut auth = SoftPasskey::new(true);
    assert_eq!(
        enroler(&b, &mut auth, &s, "T").await.statut,
        StatusCode::CREATED
    );
    let inconnu = format!("{}@exemple.invalid", Uuid::new_v4());

    let mut formes = Vec::new();
    for mail in [courriel(avec), courriel(sans), inconnu.clone()] {
        let (d, ceremonie, _) = defi_connexion(&b, &mut SoftPasskey::new(true), &mail).await;
        assert!(ceremonie.starts_with(&format!("{CEREMONIE}=")));
        let options = &d.json()["defi"]["publicKey"];
        let ids = options["allowCredentials"].as_array().unwrap().len();
        assert!((1..=3).contains(&ids), "{ids} identifiants");
        assert_eq!(options["userVerification"], "required");
        let mut cles: Vec<String> = options.as_object().unwrap().keys().cloned().collect();
        cles.sort();
        formes.push((d.statut, cles));
    }
    assert!(formes.windows(2).all(|w| w[0] == w[1]), "{formes:?}");

    // Le faux défi est stable pour un courriel (sans casse), et rien n'est stocké pour lui.
    let ids = |r: &Reponse| r.json()["defi"]["publicKey"]["allowCredentials"].clone();
    let (d1, _, _) = defi_connexion(&b, &mut SoftPasskey::new(true), &inconnu).await;
    let (d2, _, _) = defi_connexion(&b, &mut SoftPasskey::new(true), &inconnu.to_uppercase()).await;
    assert_eq!(ids(&d1), ids(&d2));
    let defis: i64 =
        sqlx::query_scalar("SELECT count(*) FROM defi_webauthn WHERE utilisateur_id = $1")
            .bind(sans.uuid())
            .fetch_one(&b.env.admin)
            .await
            .unwrap();
    assert_eq!(defis, 0);
    // Réponse à un faux défi : même refus qu'une mauvaise réponse sur un vrai compte ; rien
    // n'est inscrit pour un courriel inconnu, un échec l'est pour un compte connu.
    let (_, ceremonie, _) = defi_connexion(&b, &mut SoftPasskey::new(true), &inconnu).await;
    let (_, _, credential) = defi_connexion(&b, &mut auth, &courriel(avec)).await;
    let credential = credential.unwrap();
    let refus_inconnu = envoyer_connexion(
        &b,
        &corps_connexion(&inconnu, &credential),
        Some(&ceremonie),
    )
    .await;
    let (_, ceremonie, _) = defi_connexion(&b, &mut SoftPasskey::new(true), &courriel(sans)).await;
    let refus_sans_passkey = envoyer_connexion(
        &b,
        &corps_connexion(&courriel(sans), &credential),
        Some(&ceremonie),
    )
    .await;
    for r in [&refus_inconnu, &refus_sans_passkey] {
        assert_eq!(r.statut, StatusCode::UNAUTHORIZED, "{}", r.texte());
        assert_eq!(r.json()["erreur"], "non_authentifie");
    }
    assert_eq!(refus_inconnu.corps, refus_sans_passkey.corps);
    assert_eq!(evenements(&b, sans, "connexion", "echec").await, 1);
    let evenements_inconnus: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM evenement_authentification e JOIN utilisateur u ON u.id = e.utilisateur_id \
         WHERE lower(u.courriel) = lower($1)",
    )
    .bind(&inconnu)
    .fetch_one(&b.env.admin)
    .await
    .unwrap();
    assert_eq!(evenements_inconnus, 0);
}

#[tokio::test]
async fn les_defis_de_connexion_conserves_par_compte_sont_bornes() {
    let b = banc().await;
    let u = b.c.administrateur;
    let s = session(&b, u, NatureSession::Complete).await;
    let mut auth = SoftPasskey::new(true);
    assert_eq!(
        enroler(&b, &mut auth, &s, "T").await.statut,
        StatusCode::CREATED
    );
    for _ in 0..10 {
        defi_connexion(&b, &mut auth, &courriel(u)).await;
    }
    let defis = compter(
        &b.env.admin,
        "SELECT count(*) FROM defi_webauthn WHERE utilisateur_id = $1",
        u.uuid(),
    )
    .await;
    assert_eq!(defis, 4, "au plus 4 défis de connexion par compte");
}

// --- Ré-authentification ---------------------------------------------------------------------

#[tokio::test]
async fn la_reauthentification_par_passkey_rend_la_session_fraiche_et_tourne_le_jeton() {
    let b = banc().await;
    let u = b.c.administrateur;
    let s = session(&b, u, NatureSession::Complete).await;
    let mut auth = SoftPasskey::new(true);
    assert_eq!(
        enroler(&b, &mut auth, &s, "T").await.statut,
        StatusCode::CREATED
    );

    // Session plus fraîche : l'enrôlement d'une autre passkey est refusé (F1.6).
    b.horloge.avancer(Duration::minutes(6));
    let d = envoyer(&b, "POST", DEFI_ENROLEMENT, Some(&s.0), Some(&s.1), "").await;
    assert_eq!(d.statut, StatusCode::FORBIDDEN, "{}", d.texte());

    // Ré-authentification par passkey : nouveau jeton, ancien invalide, session fraîche.
    let d = envoyer(&b, "POST", DEFI_REAUTH, Some(&s.0), Some(&s.1), "").await;
    assert_eq!(d.statut, StatusCode::OK, "{}", d.texte());
    let ceremonie = cookie_ceremonie(&d);
    let options: RequestChallengeResponse =
        serde_json::from_value(d.json()["defi"].clone()).unwrap();
    let credential = auth.do_authentication(origine(), options).unwrap();
    let corps = serde_json::json!({ "credential": credential }).to_string();
    let r = envoyer(
        &b,
        "POST",
        REAUTH,
        Some(&cookies(&s, &ceremonie)),
        Some(&s.1),
        &corps,
    )
    .await;
    assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
    let nouveau = r
        .entetes
        .get_all("set-cookie")
        .iter()
        .find(|v| v.to_str().unwrap().starts_with("__Host-sigma_session="))
        .map(|v| v.to_str().unwrap().split(';').next().unwrap().to_owned())
        .expect("nouveau jeton de session");
    let csrf = r.json()["csrf"].as_str().unwrap().to_owned();
    let ancien = envoyer(&b, "GET", "/api/session", Some(&s.0), None, "").await;
    assert_eq!(ancien.statut, StatusCode::UNAUTHORIZED);
    // Rejeu de la même réponse : refusé.
    let rejeu = envoyer(
        &b,
        "POST",
        REAUTH,
        Some(&format!("{nouveau}; {ceremonie}")),
        Some(&csrf),
        &corps,
    )
    .await;
    assert_eq!(
        rejeu.statut,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        rejeu.texte()
    );
    assert_eq!(rejeu.json()["erreur"], "facteurs_refuses");
    assert_eq!(evenements(&b, u, "reauthentification", "succes").await, 1);
    assert_eq!(evenements(&b, u, "reauthentification", "echec").await, 1);
    // Fraîche : l'enrôlement d'une seconde passkey passe à nouveau.
    let r = enroler(&b, &mut SoftPasskey::new(true), &(nouveau, csrf), "Seconde").await;
    assert_eq!(r.statut, StatusCode::CREATED, "{}", r.texte());
}

#[tokio::test]
async fn la_reauthentification_exige_une_passkey_et_la_cerémonie_de_la_session() {
    let b = banc().await;
    let u = b.c.administrateur;
    let s = session(&b, u, NatureSession::Complete).await;
    // Sans passkey : pas de défi de ré-authentification.
    let d = envoyer(&b, "POST", DEFI_REAUTH, Some(&s.0), Some(&s.1), "").await;
    assert_eq!(d.statut, StatusCode::UNPROCESSABLE_ENTITY, "{}", d.texte());
    assert_eq!(d.json()["erreur"], "aucune_passkey");
    let mut auth = SoftPasskey::new(true);
    assert_eq!(
        enroler(&b, &mut auth, &s, "T").await.statut,
        StatusCode::CREATED
    );

    // Défi demandé dans une session, réponse envoyée depuis une autre : refusée.
    let d = envoyer(&b, "POST", DEFI_REAUTH, Some(&s.0), Some(&s.1), "").await;
    let ceremonie = cookie_ceremonie(&d);
    let options: RequestChallengeResponse =
        serde_json::from_value(d.json()["defi"].clone()).unwrap();
    let credential = auth.do_authentication(origine(), options).unwrap();
    let autre = session(&b, u, NatureSession::Complete).await;
    let corps = serde_json::json!({ "credential": credential }).to_string();
    let r = envoyer(
        &b,
        "POST",
        REAUTH,
        Some(&cookies(&autre, &ceremonie)),
        Some(&autre.1),
        &corps,
    )
    .await;
    assert_eq!(r.statut, StatusCode::UNPROCESSABLE_ENTITY, "{}", r.texte());
    assert_eq!(r.json()["erreur"], "facteurs_refuses");
}

// --- Suppression -----------------------------------------------------------------------------

async fn identifiants(b: &Banc, s: &Session) -> Vec<String> {
    let etat = envoyer(b, "GET", "/api/moi/facteurs", Some(&s.0), None, "").await;
    etat.json()["passkeys"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["id"].as_str().unwrap().to_owned())
        .collect()
}

#[tokio::test]
async fn le_dernier_facteur_fort_ne_se_supprime_pas() {
    let b = banc().await;
    let u = b.c.administrateur;
    let s = session(&b, u, NatureSession::Complete).await;
    let mut auth = SoftPasskey::new(true);
    assert_eq!(
        enroler(&b, &mut auth, &s, "Première").await.statut,
        StatusCode::CREATED
    );
    let ids = identifiants(&b, &s).await;
    // Seule passkey, pas de secours : refusée (F1.1).
    let r = envoyer(
        &b,
        "DELETE",
        &format!("{ENROLEMENT}/{}", ids[0]),
        Some(&s.0),
        Some(&s.1),
        "",
    )
    .await;
    assert_eq!(r.statut, StatusCode::UNPROCESSABLE_ENTITY, "{}", r.texte());
    assert_eq!(r.json()["erreur"], "dernier_facteur");
    assert_eq!(passkeys_de(&b, u).await, 1);

    // Avec une seconde passkey : la première se supprime (204), les autres sessions sont
    // révoquées, la suppression est inscrite.
    assert_eq!(
        enroler(&b, &mut SoftPasskey::new(true), &s, "Seconde")
            .await
            .statut,
        StatusCode::CREATED
    );
    let r = envoyer(
        &b,
        "DELETE",
        &format!("{ENROLEMENT}/{}", ids[0]),
        Some(&s.0),
        Some(&s.1),
        "",
    )
    .await;
    assert_eq!(r.statut, StatusCode::NO_CONTENT, "{}", r.texte());
    assert_eq!(passkeys_de(&b, u).await, 1);
    assert_eq!(evenements(&b, u, "suppression_passkey", "succes").await, 1);
    assert_eq!(audit_de(&b, u, "SupprimerPasskey").await, 1);
    // L'authentificateur supprimé n'ouvre plus de session.
    let (_, _, credential) = defi_connexion(&b, &mut auth, &courriel(u)).await;
    if let Some(credential) = credential {
        let (_, ceremonie, _) = defi_connexion(&b, &mut SoftPasskey::new(true), &courriel(u)).await;
        let r = envoyer_connexion(
            &b,
            &corps_connexion(&courriel(u), &credential),
            Some(&ceremonie),
        )
        .await;
        assert_eq!(r.statut, StatusCode::UNAUTHORIZED, "{}", r.texte());
    }

    // Un secours utilisable libère la dernière passkey.
    installer_secours(&b.env.admin, u, "cheval agrafe pile correcte").await;
    let reste = identifiants(&b, &s).await;
    let r = envoyer(
        &b,
        "DELETE",
        &format!("{ENROLEMENT}/{}", reste[0]),
        Some(&s.0),
        Some(&s.1),
        "",
    )
    .await;
    assert_eq!(r.statut, StatusCode::NO_CONTENT, "{}", r.texte());
}

#[tokio::test]
async fn on_ne_supprime_que_ses_passkeys_et_une_session_perimee_ne_supprime_rien() {
    let b = banc().await;
    let (a, autre) = (b.c.administrateur, b.c.collaborateur);
    let sa = session(&b, a, NatureSession::Complete).await;
    let sb = session(&b, autre, NatureSession::Complete).await;
    assert_eq!(
        enroler(&b, &mut SoftPasskey::new(true), &sa, "A")
            .await
            .statut,
        StatusCode::CREATED
    );
    assert_eq!(
        enroler(&b, &mut SoftPasskey::new(true), &sb, "B")
            .await
            .statut,
        StatusCode::CREATED
    );
    assert_eq!(
        enroler(&b, &mut SoftPasskey::new(true), &sb, "B2")
            .await
            .statut,
        StatusCode::CREATED
    );
    let de_b = identifiants(&b, &sb).await;
    // A vise une passkey de B : 404, rien n'est touché.
    let r = envoyer(
        &b,
        "DELETE",
        &format!("{ENROLEMENT}/{}", de_b[0]),
        Some(&sa.0),
        Some(&sa.1),
        "",
    )
    .await;
    assert_eq!(r.statut, StatusCode::NOT_FOUND, "{}", r.texte());
    assert_eq!(passkeys_de(&b, autre).await, 2);
    // Identifiant malformé ou nul.
    for mauvais in ["pas-un-uuid", "00000000-0000-0000-0000-000000000000"] {
        let r = envoyer(
            &b,
            "DELETE",
            &format!("{ENROLEMENT}/{mauvais}"),
            Some(&sa.0),
            Some(&sa.1),
            "",
        )
        .await;
        assert!(
            r.statut == StatusCode::BAD_REQUEST || r.statut == StatusCode::NOT_FOUND,
            "{mauvais} : {}",
            r.statut
        );
    }
    // Session non fraîche : la suppression est refusée par Cedar.
    b.horloge.avancer(Duration::minutes(6));
    let r = envoyer(
        &b,
        "DELETE",
        &format!("{ENROLEMENT}/{}", de_b[0]),
        Some(&sb.0),
        Some(&sb.1),
        "",
    )
    .await;
    assert_eq!(r.statut, StatusCode::FORBIDDEN, "{}", r.texte());
    assert_eq!(passkeys_de(&b, autre).await, 2);
}

// --- Session restreinte (code de récupération) -----------------------------------------------

#[tokio::test]
async fn une_session_restreinte_enrole_une_passkey_et_ne_fait_rien_d_autre() {
    let b = banc().await;
    let u = b.c.administrateur;
    installer_codes(&b.env.admin, u).await;
    let r = session(&b, u, NatureSession::Restreinte).await;
    // Elle peut enrôler (F1.12)…
    let mut auth = SoftPasskey::new(true);
    let enrole = enroler(&b, &mut auth, &r, "Nouvelle").await;
    assert_eq!(enrole.statut, StatusCode::CREATED, "{}", enrole.texte());
    // …mais ni se ré-authentifier, ni supprimer, ni lire ses facteurs.
    for (methode, chemin) in [
        ("POST", DEFI_REAUTH.to_owned()),
        ("POST", REAUTH.to_owned()),
        ("GET", "/api/moi/facteurs".to_owned()),
    ] {
        let rep = envoyer(&b, methode, &chemin, Some(&r.0), Some(&r.1), "{}").await;
        assert_eq!(
            rep.statut,
            StatusCode::UNAUTHORIZED,
            "{chemin} : {}",
            rep.texte()
        );
    }
    // Et la passkey enrôlée ouvre ensuite une vraie session complète.
    let rep = connexion(&b, &mut auth, &courriel(u)).await;
    assert_eq!(rep.statut, StatusCode::OK, "{}", rep.texte());
    assert_eq!(rep.json()["nature"], "complete");
}

#[tokio::test]
async fn pas_plus_de_dix_passkeys() {
    let b = banc().await;
    let u = b.c.administrateur;
    let s = session(&b, u, NatureSession::Complete).await;
    for i in 0..10 {
        let r = enroler(&b, &mut SoftPasskey::new(true), &s, &format!("Clé {i}")).await;
        assert_eq!(r.statut, StatusCode::CREATED, "{i} : {}", r.texte());
    }
    let d = envoyer(&b, "POST", DEFI_ENROLEMENT, Some(&s.0), Some(&s.1), "").await;
    assert_eq!(d.statut, StatusCode::UNPROCESSABLE_ENTITY, "{}", d.texte());
    assert_eq!(d.json()["erreur"], "passkeys_max");
    assert_eq!(passkeys_de(&b, u).await, 10);
}

#[tokio::test]
async fn le_nom_d_une_passkey_est_borne() {
    let b = banc().await;
    let s = session(&b, b.c.administrateur, NatureSession::Complete).await;
    for nom in ["", "   ", "a\nb", &"x".repeat(101)] {
        let r = enroler(&b, &mut SoftPasskey::new(true), &s, nom).await;
        assert_eq!(
            r.statut,
            StatusCode::UNPROCESSABLE_ENTITY,
            "{nom:?} : {}",
            r.texte()
        );
        assert_eq!(r.json()["erreur"], "passkey_refusee");
    }
}

// --- RLS de la connexion ---------------------------------------------------------------------

/// Transaction du rôle applicatif au contexte d'une connexion (`sigma.courriel_connexion`).
async fn contexte_connexion(b: &Banc, mail: &str) -> sqlx::Transaction<'static, sqlx::Postgres> {
    let mut tx = b.env.application.begin().await.unwrap();
    sqlx::query(
        "SELECT set_config('sigma.cabinet', '', true), set_config('sigma.dossier', '', true), \
                set_config('sigma.utilisateur', '', true), \
                set_config('sigma.courriel_connexion', $1, true)",
    )
    .bind(mail)
    .execute(&mut *tx)
    .await
    .unwrap();
    tx
}

#[tokio::test]
async fn la_connexion_ne_voit_que_les_passkeys_de_son_courriel_et_n_en_change_que_l_etat() {
    let b = banc().await;
    let (a, autre) = (b.c.administrateur, b.c.collaborateur);
    let sa = session(&b, a, NatureSession::Complete).await;
    let sb = session(&b, autre, NatureSession::Complete).await;
    assert_eq!(
        enroler(&b, &mut SoftPasskey::new(true), &sa, "A")
            .await
            .statut,
        StatusCode::CREATED
    );
    assert_eq!(
        enroler(&b, &mut SoftPasskey::new(true), &sb, "B")
            .await
            .statut,
        StatusCode::CREATED
    );

    let mut tx = contexte_connexion(&b, &courriel(a)).await;
    let vues: Vec<Uuid> = sqlx::query_scalar("SELECT utilisateur_id FROM facteur_passkey")
        .fetch_all(&mut *tx)
        .await
        .unwrap();
    assert_eq!(vues, vec![a.uuid()]);
    // Autre colonne que l'état et la dernière utilisation : refusé (droit de colonne).
    let e = sqlx::query("UPDATE facteur_passkey SET nom = 'x'")
        .execute(&mut *tx)
        .await
        .unwrap_err();
    assert!(e.to_string().contains("permission"), "{e}");
    tx.rollback().await.unwrap();
    // Passkey d'un autre compte : invisible, donc aucune ligne touchée.
    let mut tx = contexte_connexion(&b, &courriel(a)).await;
    let touchees = sqlx::query(
        "UPDATE facteur_passkey SET derniere_utilisation = now() WHERE utilisateur_id = $1",
    )
    .bind(autre.uuid())
    .execute(&mut *tx)
    .await
    .unwrap()
    .rows_affected();
    assert_eq!(touchees, 0);
    // Pas de création de passkey ni de défi d'enrôlement depuis une connexion.
    let e = sqlx::query("DELETE FROM facteur_passkey")
        .execute(&mut *tx)
        .await
        .unwrap();
    assert_eq!(
        e.rows_affected(),
        0,
        "la connexion ne supprime aucune passkey"
    );
    let e = sqlx::query(
        "INSERT INTO defi_webauthn (id, jeton_hache, utilisateur_id, nature, session_id, etat, cree_le, expire_le) \
         VALUES ($1, $2, $3, 'enrolement', NULL, '{}', now(), now() + interval '1 minute')",
    )
    .bind(Uuid::new_v4())
    .bind(vec![7u8; 32])
    .bind(a.uuid())
    .execute(&mut *tx)
    .await;
    assert!(e.is_err(), "défi d'enrôlement refusé à la connexion");
}

#[tokio::test]
async fn la_connexion_ne_voit_que_les_defis_de_connexion_de_son_courriel() {
    let b = banc().await;
    let (a, autre) = (b.c.administrateur, b.c.collaborateur);
    let sa = session(&b, a, NatureSession::Complete).await;
    let sb = session(&b, autre, NatureSession::Complete).await;
    let mut auth = SoftPasskey::new(true);
    assert_eq!(
        enroler(&b, &mut auth, &sa, "T").await.statut,
        StatusCode::CREATED
    );
    assert_eq!(
        enroler(&b, &mut SoftPasskey::new(true), &sb, "B")
            .await
            .statut,
        StatusCode::CREATED
    );
    defi_connexion(&b, &mut auth, &courriel(a)).await;
    // Un défi d'enrôlement et un de ré-authentification de A, en cours.
    envoyer(&b, "POST", DEFI_ENROLEMENT, Some(&sa.0), Some(&sa.1), "").await;
    envoyer(&b, "POST", DEFI_REAUTH, Some(&sa.0), Some(&sa.1), "").await;
    let mut tx = contexte_connexion(&b, &courriel(a)).await;
    let natures: Vec<(String, Uuid)> =
        sqlx::query_as("SELECT nature, utilisateur_id FROM defi_webauthn")
            .fetch_all(&mut *tx)
            .await
            .unwrap();
    assert_eq!(natures, vec![("connexion".to_owned(), a.uuid())]);
    tx.rollback().await.unwrap();
    // Le courriel d'un autre compte ne montre rien de A.
    let mut tx = contexte_connexion(&b, &courriel(autre)).await;
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM defi_webauthn WHERE utilisateur_id = $1")
        .bind(a.uuid())
        .fetch_one(&mut *tx)
        .await
        .unwrap();
    assert_eq!(n, 0);
}

// --- Configuration ---------------------------------------------------------------------------

#[test]
fn une_origine_sans_hote_nomme_n_est_pas_une_partie_de_confiance() {
    // `Configuration::depuis` refuse de démarrer sur cette erreur (`serveur::config`).
    for origine in ["https://127.0.0.1:8443", "https://[::1]"] {
        assert!(api::facteurs::passkey::Passkeys::depuis_origine(origine).is_err());
    }
    assert!(api::facteurs::passkey::Passkeys::depuis_origine(ORIGINE).is_ok());
}
