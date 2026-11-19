//! Sessions réelles (ADR 0012 ; L2-17 ; L2, « Tests exigés », « Authentification et sessions » ;
//! F1.2, F1.4, F1.6, F1.12, F2.11) : cookie, expiration (horloge injectée), nature, rotation,
//! anti-CSRF lié à la session, fraîcheur, révocation immédiate, purge, RLS de la table
//! `session` et des cabinets sans cabinet en contexte, journaux sans jeton.
//!
//! Les cas d'isolation de chaque route hors cabinet (un utilisateur ne voit ni ne révoque les
//! sessions d'un autre ; `/api/moi/cabinets` sans fuite ni appartenance suspendue) sont dans
//! `isolation.rs`, avec ceux de toutes les routes.
#![cfg(feature = "tests-postgres")]
// Fichier de test : les `expect`/`unwrap` y sont voulus.
#![allow(clippy::expect_used, clippy::unwrap_used)]

mod commun;

use std::sync::Arc;

use api::base::session::NatureSession;
use api::base::{ContexteTenant, IdUtilisateur, ouvrir_transaction};
use api::serveur::Horloge;
use api::serveur::session::SessionOuverte;
use authz::Autorisation;
use axum::Router;
use axum::http::StatusCode;
use commun::serveur::{
    HorlogeTest, Reponse, appeler_session, appeler_session_corps, application_sessions,
    capture_globale, cookie_de, evenements_de, id_requete, ouvrir, routeur_de,
    verifier_liste_blanche,
};
use commun::{CabinetTest, creer_cabinet, env};
use sqlx::{Connection, PgConnection};
use time::Duration;
use uuid::Uuid;

fn routeur_s(env: &commun::Env, horloge: &HorlogeTest) -> Router {
    routeur_de(&application_sessions(
        env,
        horloge,
        Arc::new(Autorisation::charger().unwrap()),
    ))
}

/// `GET` d'une route métier (paramètres du cabinet, administrateur) avec un cookie brut.
async fn metier(routeur: &Router, c: &CabinetTest, cookie: Option<&str>) -> StatusCode {
    appeler_session(
        routeur,
        "GET",
        &format!("/api/cabinets/{}", c.cabinet),
        cookie,
        None,
    )
    .await
    .statut
}

async fn etat(routeur: &Router, s: &SessionOuverte) -> Reponse {
    appeler_session(routeur, "GET", "/api/session", Some(&cookie_de(s)), None).await
}

/// Attributs d'un `Set-Cookie` de session : (valeur, Max-Age).
fn attributs(set_cookie: &str) -> (String, i64) {
    let mut morceaux = set_cookie.split("; ");
    let (nom, valeur) = morceaux.next().unwrap().split_once('=').unwrap();
    assert_eq!(nom, "__Host-sigma_session");
    let reste: Vec<&str> = morceaux.collect();
    let max_age = reste
        .iter()
        .find_map(|a| a.strip_prefix("Max-Age="))
        .unwrap()
        .parse()
        .unwrap();
    assert_eq!(
        reste
            .iter()
            .filter(|a| !a.starts_with("Max-Age="))
            .copied()
            .collect::<Vec<_>>(),
        vec!["Path=/", "Secure", "HttpOnly", "SameSite=Strict"],
        "{set_cookie}"
    );
    assert!(!set_cookie.to_ascii_lowercase().contains("domain"));
    (valeur.to_owned(), max_age)
}

#[tokio::test]
async fn attributs_du_cookie_a_l_ouverture_et_a_la_rotation() {
    let env = env().await;
    let c = creer_cabinet(&env.admin, "Cabinet").await;
    let horloge = HorlogeTest::default();
    let routeur = routeur_s(&env, &horloge);
    let s = ouvrir(&env, &horloge, c.administrateur, NatureSession::Complete).await;
    let (valeur, max_age) = attributs(s.cookie(horloge.maintenant()).unwrap().to_str().unwrap());
    // 256 bits en base64url sans remplissage ; Max-Age : les 12 heures entières.
    assert_eq!(valeur.len(), 43);
    assert!(
        valeur
            .bytes()
            .all(|o| o.is_ascii_alphanumeric() || o == b'-' || o == b'_')
    );
    assert!((43_195..=43_200).contains(&max_age), "{max_age}");

    // Vingt minutes plus tard, la rotation pose un nouveau jeton ; Max-Age : le reste des
    // 12 heures.
    horloge.avancer(Duration::minutes(20));
    let r = appeler_session(
        &routeur,
        "POST",
        "/api/session/rotation",
        Some(&cookie_de(&s)),
        Some(&s.jeton.csrf()),
    )
    .await;
    assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
    let (nouvelle, max_age) = attributs(r.entetes["set-cookie"].to_str().unwrap());
    assert_ne!(nouvelle, valeur);
    assert!((41_995..=42_000).contains(&max_age), "{max_age}");
    // Le corps porte le nouveau jeton anti-CSRF, jamais le jeton.
    assert!(!r.texte().contains(&nouvelle));
    let csrf = r.json()["csrf"].as_str().unwrap().to_owned();
    let cookie = format!("__Host-sigma_session={nouvelle}");
    let r = appeler_session(&routeur, "GET", "/api/session", Some(&cookie), None).await;
    assert_eq!(r.json()["csrf"], csrf);
    assert_eq!(r.json()["id"], s.id.to_string());
}

#[tokio::test]
async fn jeton_absent_faux_ou_revoque_donne_401() {
    let env = env().await;
    let c = creer_cabinet(&env.admin, "Cabinet").await;
    let horloge = HorlogeTest::default();
    let routeur = routeur_s(&env, &horloge);
    let s = ouvrir(&env, &horloge, c.administrateur, NatureSession::Complete).await;
    let cookie = cookie_de(&s);
    assert_eq!(metier(&routeur, &c, Some(&cookie)).await, StatusCode::OK);

    let faux = format!("__Host-sigma_session={}", "A".repeat(43));
    let double = format!("{cookie}; {cookie}");
    let sans_prefixe = cookie.replace("__Host-", "");
    for invalide in [
        None,
        Some(faux.as_str()),
        Some("__Host-sigma_session=court"),
        Some(double.as_str()),
        Some(sans_prefixe.as_str()),
    ] {
        assert_eq!(
            metier(&routeur, &c, invalide).await,
            StatusCode::UNAUTHORIZED,
            "{invalide:?}"
        );
    }
    // Révoquée en base (par exemple par un administrateur, PR 9) : 401 dès la requête suivante.
    sqlx::query(
        "UPDATE session SET revoquee_le = now(), motif_revocation = 'revocation_utilisateur' \
         WHERE id = $1",
    )
    .bind(s.id.uuid())
    .execute(&env.admin)
    .await
    .unwrap();
    assert_eq!(
        metier(&routeur, &c, Some(&cookie)).await,
        StatusCode::UNAUTHORIZED
    );
    let r = etat(&routeur, &s).await;
    assert_eq!(r.statut, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn expiration_par_inactivite_puis_au_dela_de_douze_heures() {
    let env = env().await;
    let c = creer_cabinet(&env.admin, "Cabinet").await;
    let horloge = HorlogeTest::default();
    let routeur = routeur_s(&env, &horloge);

    // Inactivité : 29 minutes, admise ; 30 minutes sans requête, refusée.
    let s = ouvrir(&env, &horloge, c.administrateur, NatureSession::Complete).await;
    let cookie = cookie_de(&s);
    horloge.avancer(Duration::minutes(29));
    assert_eq!(metier(&routeur, &c, Some(&cookie)).await, StatusCode::OK);
    // L'activité a été notée : 29 minutes de plus restent admises.
    horloge.avancer(Duration::minutes(29));
    assert_eq!(metier(&routeur, &c, Some(&cookie)).await, StatusCode::OK);
    horloge.avancer(Duration::minutes(30));
    assert_eq!(
        metier(&routeur, &c, Some(&cookie)).await,
        StatusCode::UNAUTHORIZED
    );

    // Au-delà de 12 heures, même active.
    let s = ouvrir(&env, &horloge, c.administrateur, NatureSession::Complete).await;
    let cookie = cookie_de(&s);
    for _ in 0..35 {
        horloge.avancer(Duration::minutes(20));
        assert_eq!(metier(&routeur, &c, Some(&cookie)).await, StatusCode::OK);
    }
    // 11 h 40 écoulées ; 11 h 59 encore admise, 12 h refusée.
    horloge.avancer(Duration::minutes(19));
    assert_eq!(metier(&routeur, &c, Some(&cookie)).await, StatusCode::OK);
    horloge.avancer(Duration::minutes(1));
    assert_eq!(
        metier(&routeur, &c, Some(&cookie)).await,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn l_activite_n_est_ecrite_qu_au_dela_de_soixante_secondes() {
    let env = env().await;
    let c = creer_cabinet(&env.admin, "Cabinet").await;
    let horloge = HorlogeTest::default();
    let routeur = routeur_s(&env, &horloge);
    let s = ouvrir(&env, &horloge, c.administrateur, NatureSession::Complete).await;
    let cookie = cookie_de(&s);
    let activite = || async {
        sqlx::query_scalar::<_, time::OffsetDateTime>(
            "SELECT derniere_activite FROM session WHERE id = $1",
        )
        .bind(s.id.uuid())
        .fetch_one(&env.admin)
        .await
        .unwrap()
    };
    let initiale = activite().await;
    horloge.avancer(Duration::seconds(30));
    assert_eq!(metier(&routeur, &c, Some(&cookie)).await, StatusCode::OK);
    assert_eq!(activite().await, initiale);
    horloge.avancer(Duration::seconds(31));
    assert_eq!(metier(&routeur, &c, Some(&cookie)).await, StatusCode::OK);
    assert!(activite().await >= initiale + Duration::seconds(61));
}

#[tokio::test]
async fn une_session_restreinte_n_ouvre_aucune_route_metier() {
    let env = env().await;
    let c = creer_cabinet(&env.admin, "Cabinet").await;
    let horloge = HorlogeTest::default();
    let routeur = routeur_s(&env, &horloge);
    for nature in [NatureSession::Restreinte, NatureSession::Enrolement] {
        let s = ouvrir(&env, &horloge, c.administrateur, nature).await;
        let cookie = cookie_de(&s);
        let csrf = s.jeton.csrf();
        assert_eq!(
            metier(&routeur, &c, Some(&cookie)).await,
            StatusCode::UNAUTHORIZED
        );
        for (methode, chemin) in [
            ("GET", "/api/moi/cabinets".to_owned()),
            ("GET", "/api/moi/sessions".to_owned()),
            ("POST", "/api/session/rotation".to_owned()),
            ("POST", format!("/api/cabinets/{}/dossiers", c.cabinet)),
            ("DELETE", format!("/api/moi/sessions/{}", s.id)),
        ] {
            let r = appeler_session(&routeur, methode, &chemin, Some(&cookie), Some(&csrf)).await;
            assert_eq!(
                r.statut,
                StatusCode::UNAUTHORIZED,
                "{nature:?} {methode} {chemin}"
            );
        }
        // Son état et sa fermeture, seulement.
        let r = etat(&routeur, &s).await;
        assert_eq!(r.statut, StatusCode::OK);
        assert_eq!(r.json()["nature"], nature.nom());
        // Durée maximale : 15 minutes.
        horloge.avancer(Duration::minutes(10));
        assert_eq!(etat(&routeur, &s).await.statut, StatusCode::OK);
        horloge.avancer(Duration::minutes(5));
        assert_eq!(etat(&routeur, &s).await.statut, StatusCode::UNAUTHORIZED);
        let s = ouvrir(&env, &horloge, c.administrateur, nature).await;
        let r = appeler_session(
            &routeur,
            "POST",
            "/api/session/deconnexion",
            Some(&cookie_de(&s)),
            Some(&s.jeton.csrf()),
        )
        .await;
        assert_eq!(r.statut, StatusCode::NO_CONTENT);
        assert_eq!(etat(&routeur, &s).await.statut, StatusCode::UNAUTHORIZED);
    }
}

#[tokio::test]
async fn rotation_l_ancien_jeton_donne_401() {
    let env = env().await;
    let c = creer_cabinet(&env.admin, "Cabinet").await;
    let horloge = HorlogeTest::default();
    let routeur = routeur_s(&env, &horloge);
    let s = ouvrir(&env, &horloge, c.administrateur, NatureSession::Complete).await;
    let ancien = cookie_de(&s);
    let r = appeler_session(
        &routeur,
        "POST",
        "/api/session/rotation",
        Some(&ancien),
        Some(&s.jeton.csrf()),
    )
    .await;
    assert_eq!(r.statut, StatusCode::OK);
    let nouveau = r.entetes["set-cookie"]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_owned();
    assert_eq!(
        metier(&routeur, &c, Some(&ancien)).await,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(metier(&routeur, &c, Some(&nouveau)).await, StatusCode::OK);
    // L'ancien jeton anti-CSRF ne vaut plus pour une mutation.
    let r = appeler_session(
        &routeur,
        "POST",
        &format!("/api/cabinets/{}/dossiers", c.cabinet),
        Some(&nouveau),
        Some(&s.jeton.csrf()),
    )
    .await;
    assert_eq!(r.statut, StatusCode::FORBIDDEN);
    // Même identifiant public : l'audit cite toujours la même session.
    let r = appeler_session(&routeur, "GET", "/api/session", Some(&nouveau), None).await;
    assert_eq!(r.json()["id"], s.id.to_string());
}

#[tokio::test]
async fn csrf_absent_faux_ou_d_une_autre_session_donne_403() {
    let env = env().await;
    let c = creer_cabinet(&env.admin, "Cabinet").await;
    let horloge = HorlogeTest::default();
    let routeur = routeur_s(&env, &horloge);
    let s = ouvrir(&env, &horloge, c.administrateur, NatureSession::Complete).await;
    let autre = ouvrir(&env, &horloge, c.administrateur, NatureSession::Complete).await;
    let cookie = cookie_de(&s);
    let creation = format!("/api/cabinets/{}/dossiers", c.cabinet);
    let compter = || async {
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM dossier WHERE cabinet_id = $1")
            .bind(c.cabinet.uuid())
            .fetch_one(&env.admin)
            .await
            .unwrap()
    };
    let avant = compter().await;
    let mal_forme = format!("{}x", s.jeton.csrf());
    let tronque = s.jeton.csrf()[1..].to_owned();
    let csrf_autre = autre.jeton.csrf();
    let jeton_brut = cookie.split_once('=').unwrap().1.to_owned();
    for csrf in [
        None,
        Some("1"),
        Some(mal_forme.as_str()),
        Some(tronque.as_str()),
        // Le jeton de session lui-même n'est pas le jeton anti-CSRF.
        Some(jeton_brut.as_str()),
        // Celui d'une autre session du même utilisateur.
        Some(csrf_autre.as_str()),
    ] {
        for (methode, chemin) in [
            ("POST", creation.clone()),
            ("POST", "/api/session/rotation".to_owned()),
            ("POST", "/api/session/deconnexion".to_owned()),
            ("DELETE", format!("/api/moi/sessions/{}", autre.id)),
        ] {
            let r = appeler_session(&routeur, methode, &chemin, Some(&cookie), csrf).await;
            assert_eq!(
                r.statut,
                StatusCode::FORBIDDEN,
                "{methode} {chemin} {csrf:?}"
            );
        }
    }
    assert_eq!(compter().await, avant);
    assert_eq!(etat(&routeur, &s).await.statut, StatusCode::OK);
    assert_eq!(etat(&routeur, &autre).await.statut, StatusCode::OK);
    // Le bon jeton : la mutation aboutit.
    let r = appeler_session_corps(
        &routeur,
        "POST",
        &creation,
        Some(&cookie),
        Some(&s.jeton.csrf()),
        r#"{"nom":"Dossier"}"#,
    )
    .await;
    assert_eq!(r.statut, StatusCode::CREATED, "{}", r.texte());
    // Une lecture n'en demande pas.
    assert_eq!(metier(&routeur, &c, Some(&cookie)).await, StatusCode::OK);
}

#[tokio::test]
async fn une_verification_vieille_de_six_minutes_refuse_l_action_sensible() {
    let env = env().await;
    let c = creer_cabinet(&env.admin, "Cabinet").await;
    let horloge = HorlogeTest::default();
    let routeur = routeur_s(&env, &horloge);
    let export = format!("/api/cabinets/{}/audit/export", c.cabinet);
    let s = ouvrir(&env, &horloge, c.administrateur, NatureSession::Complete).await;
    let r = etat(&routeur, &s).await;
    assert_eq!(r.json()["fraiche"], true);
    let exporter = |s: &SessionOuverte| {
        let (cookie, csrf) = (cookie_de(s), s.jeton.csrf());
        let (routeur, export) = (routeur.clone(), export.clone());
        async move {
            appeler_session(&routeur, "POST", &export, Some(&cookie), Some(&csrf))
                .await
                .statut
        }
    };
    assert_eq!(exporter(&s).await, StatusCode::OK);
    horloge.avancer(Duration::minutes(6));
    assert_eq!(exporter(&s).await, StatusCode::FORBIDDEN);
    assert_eq!(etat(&routeur, &s).await.json()["fraiche"], false);
    // Une lecture reste admise.
    assert_eq!(
        metier(&routeur, &c, Some(&cookie_de(&s))).await,
        StatusCode::OK
    );
}

#[tokio::test]
async fn la_revocation_est_immediate() {
    let env = env().await;
    let c = creer_cabinet(&env.admin, "Cabinet").await;
    let horloge = HorlogeTest::default();
    let routeur = routeur_s(&env, &horloge);
    let ici = ouvrir(&env, &horloge, c.administrateur, NatureSession::Complete).await;
    let ailleurs = ouvrir(&env, &horloge, c.administrateur, NatureSession::Complete).await;
    assert_eq!(
        metier(&routeur, &c, Some(&cookie_de(&ailleurs))).await,
        StatusCode::OK
    );
    let r = appeler_session(
        &routeur,
        "DELETE",
        &format!("/api/moi/sessions/{}", ailleurs.id),
        Some(&cookie_de(&ici)),
        Some(&ici.jeton.csrf()),
    )
    .await;
    assert_eq!(r.statut, StatusCode::NO_CONTENT, "{}", r.texte());
    assert_eq!(
        metier(&routeur, &c, Some(&cookie_de(&ailleurs))).await,
        StatusCode::UNAUTHORIZED
    );
    // La liste ne la montre plus ; la session courante y est marquée.
    let r = appeler_session(
        &routeur,
        "GET",
        "/api/moi/sessions",
        Some(&cookie_de(&ici)),
        None,
    )
    .await;
    let liste = r.json();
    let liste = liste.as_array().unwrap();
    assert_eq!(liste.len(), 1, "{}", r.texte());
    assert_eq!(liste[0]["id"], ici.id.to_string());
    assert_eq!(liste[0]["courante"], true);
    assert_eq!(liste[0]["adresse"], "192.0.2.7");
    let motif: String = sqlx::query_scalar("SELECT motif_revocation FROM session WHERE id = $1")
        .bind(ailleurs.id.uuid())
        .fetch_one(&env.admin)
        .await
        .unwrap();
    assert_eq!(motif, "revocation_utilisateur");
}

#[tokio::test]
async fn la_purge_ne_porte_que_sur_les_sessions_closes_de_l_utilisateur() {
    let env = env().await;
    let c = creer_cabinet(&env.admin, "Cabinet").await;
    let horloge = HorlogeTest::default();
    // Il y a 40 jours : une session de chacun des deux utilisateurs, et une révoquée de
    // l'administrateur ; il y a 10 jours, une révoquée de l'administrateur (gardée).
    horloge.avancer(-Duration::days(40));
    let ancienne = ouvrir(&env, &horloge, c.administrateur, NatureSession::Complete).await;
    let ancienne_revoquee = ouvrir(&env, &horloge, c.administrateur, NatureSession::Complete).await;
    let autre_utilisateur = ouvrir(&env, &horloge, c.collaborateur, NatureSession::Complete).await;
    horloge.avancer(Duration::days(30));
    let recente_revoquee = ouvrir(&env, &horloge, c.administrateur, NatureSession::Complete).await;
    for (s, il_y_a) in [(&ancienne_revoquee, 40), (&recente_revoquee, 10)] {
        sqlx::query(
            "UPDATE session SET revoquee_le = now() - make_interval(days => $2), \
             motif_revocation = 'deconnexion' WHERE id = $1",
        )
        .bind(s.id.uuid())
        .bind(il_y_a)
        .execute(&env.admin)
        .await
        .unwrap();
    }
    horloge.avancer(Duration::days(10));
    // L'ouverture d'une session de l'administrateur purge ses sessions closes depuis 30 jours.
    let nouvelle = ouvrir(&env, &horloge, c.administrateur, NatureSession::Complete).await;
    let presentes: Vec<Uuid> =
        sqlx::query_scalar("SELECT id FROM session WHERE utilisateur_id = ANY($1) ORDER BY id")
            .bind(vec![c.administrateur.uuid(), c.collaborateur.uuid()])
            .fetch_all(&env.admin)
            .await
            .unwrap();
    let mut attendues = vec![
        recente_revoquee.id.uuid(),
        nouvelle.id.uuid(),
        // Rien ne traverse les utilisateurs : la vieille session du collaborateur reste.
        autre_utilisateur.id.uuid(),
    ];
    attendues.sort();
    assert_eq!(presentes, attendues);
    let _ = ancienne;
}

/// Identifiants lus par `sql`, sous le rôle applicatif, avec ces seuls réglages de contexte.
async fn lus(
    connexion: &mut PgConnection,
    reglages: &[(&str, String)],
    sql: &'static str,
) -> Vec<Uuid> {
    let mut tx = connexion.begin().await.unwrap();
    for (nom, valeur) in reglages {
        sqlx::query("SELECT set_config($1, $2, true)")
            .bind(*nom)
            .bind(valeur)
            .execute(&mut *tx)
            .await
            .unwrap();
    }
    let mut ids: Vec<Uuid> = sqlx::query_scalar(sql).fetch_all(&mut *tx).await.unwrap();
    tx.rollback().await.unwrap();
    ids.sort();
    ids
}

#[tokio::test]
async fn rls_de_la_table_session_et_des_cabinets_sans_contexte() {
    let env = env().await;
    let a = creer_cabinet(&env.admin, "A").await;
    let b = creer_cabinet(&env.admin, "B").await;
    // Un membre des deux cabinets.
    let multi = IdUtilisateur::nouveau();
    let mut tx = env.admin.begin().await.unwrap();
    commun::creer_utilisateur(&mut tx, multi, "Membre des deux").await;
    commun::ajouter_membre(&mut tx, a.cabinet, multi, "collaborateur").await;
    commun::ajouter_membre(&mut tx, b.cabinet, multi, "collaborateur").await;
    tx.commit().await.unwrap();
    let horloge = HorlogeTest::default();
    let sa = ouvrir(&env, &horloge, a.administrateur, NatureSession::Complete).await;
    let sb = ouvrir(&env, &horloge, b.administrateur, NatureSession::Complete).await;

    let mut connexion = env.application.acquire().await.unwrap();
    let hex_a: String =
        sqlx::query_scalar("SELECT encode(jeton_hache, 'hex') FROM session WHERE id = $1")
            .bind(sa.id.uuid())
            .fetch_one(&env.admin)
            .await
            .unwrap();
    let sessions = "SELECT id FROM session";
    // Sans contexte : rien.
    assert_eq!(lus(&mut connexion, &[], sessions).await, Vec::<Uuid>::new());
    // L'empreinte présentée : cette session seule.
    assert_eq!(
        lus(&mut connexion, &[("sigma.jeton_session", hex_a)], sessions).await,
        vec![sa.id.uuid()]
    );
    // L'utilisateur du contexte : ses sessions seules.
    assert_eq!(
        lus(
            &mut connexion,
            &[("sigma.utilisateur", b.administrateur.to_string())],
            sessions
        )
        .await,
        vec![sb.id.uuid()]
    );
    // Écrire la session d'un autre : refusé par la RLS (aucune ligne touchée).
    let mut tx = connexion.begin().await.unwrap();
    sqlx::query("SELECT set_config('sigma.utilisateur', $1, true)")
        .bind(a.administrateur.to_string())
        .execute(&mut *tx)
        .await
        .unwrap();
    let touchees = sqlx::query(
        "UPDATE session SET revoquee_le = now(), motif_revocation = 'deconnexion' WHERE id = $1",
    )
    .bind(sb.id.uuid())
    .execute(&mut *tx)
    .await
    .unwrap()
    .rows_affected();
    assert_eq!(touchees, 0);
    let supprimees = sqlx::query("DELETE FROM session WHERE id = $1")
        .bind(sb.id.uuid())
        .execute(&mut *tx)
        .await
        .unwrap()
        .rows_affected();
    assert_eq!(supprimees, 0);
    tx.rollback().await.unwrap();

    // Sans cabinet en contexte, l'utilisateur voit ses appartenances et ses cabinets.
    let mut cabinets_attendus = vec![a.cabinet.uuid(), b.cabinet.uuid()];
    cabinets_attendus.sort();
    let moi = [("sigma.utilisateur", multi.to_string())];
    assert_eq!(
        lus(&mut connexion, &moi, "SELECT id FROM cabinet").await,
        cabinets_attendus
    );
    assert_eq!(
        lus(&mut connexion, &moi, "SELECT cabinet_id FROM appartenance").await,
        cabinets_attendus
    );
    // Rien sans utilisateur ; rien des autres membres ; aucun dossier.
    assert_eq!(
        lus(&mut connexion, &[], "SELECT id FROM cabinet").await,
        Vec::<Uuid>::new()
    );
    assert_eq!(
        lus(
            &mut connexion,
            &moi,
            "SELECT utilisateur_id FROM appartenance"
        )
        .await,
        vec![multi.uuid(); 2]
    );
    assert_eq!(
        lus(&mut connexion, &moi, "SELECT id FROM dossier").await,
        Vec::<Uuid>::new()
    );

    // Avec un cabinet en contexte, rien ne change : le cabinet du contexte seul (F1.14).
    let mut tx = ouvrir_transaction(&env.application, ContexteTenant::cabinet(a.cabinet, multi))
        .await
        .unwrap();
    let cabinets: Vec<Uuid> = sqlx::query_scalar("SELECT id FROM cabinet")
        .fetch_all(tx.connexion())
        .await
        .unwrap();
    assert_eq!(cabinets, vec![a.cabinet.uuid()]);
    let appartenances: Vec<Uuid> = sqlx::query_scalar("SELECT cabinet_id FROM appartenance")
        .fetch_all(tx.connexion())
        .await
        .unwrap();
    assert!(appartenances.iter().all(|c| *c == a.cabinet.uuid()));
    tx.annuler().await.unwrap();
}

#[tokio::test]
async fn aucun_jeton_dans_les_journaux() {
    let capture = capture_globale();
    let env = env().await;
    let c = creer_cabinet(&env.admin, "Cabinet").await;
    let horloge = HorlogeTest::default();
    let routeur = routeur_s(&env, &horloge);
    let s = ouvrir(&env, &horloge, c.administrateur, NatureSession::Complete).await;
    let autre = ouvrir(&env, &horloge, c.administrateur, NatureSession::Complete).await;
    let cookie = cookie_de(&s);
    let csrf = s.jeton.csrf();
    let mut reponses = Vec::new();
    for (methode, chemin, csrf) in [
        ("GET", "/api/session".to_owned(), None),
        ("GET", "/api/moi/sessions".to_owned(), None),
        ("GET", "/api/moi/cabinets".to_owned(), None),
        ("GET", format!("/api/cabinets/{}", c.cabinet), None),
        (
            "POST",
            format!("/api/cabinets/{}/dossiers", c.cabinet),
            Some("faux"),
        ),
        (
            "DELETE",
            format!("/api/moi/sessions/{}", autre.id),
            Some(csrf.as_str()),
        ),
        (
            "POST",
            "/api/session/rotation".to_owned(),
            Some(csrf.as_str()),
        ),
    ] {
        reponses.push(appeler_session(&routeur, methode, &chemin, Some(&cookie), csrf).await);
    }
    let r = appeler_session(
        &routeur,
        "POST",
        "/api/session/rotation",
        Some(&cookie),
        Some(&csrf),
    )
    .await;
    reponses.push(r);
    assert!(reponses.iter().any(|r| r.statut == StatusCode::OK));
    let ids: Vec<String> = reponses.iter().map(id_requete).collect();
    let evenements = evenements_de(capture, &ids);
    assert_eq!(evenements.len(), ids.len(), "une ligne par requête");
    let jeton = cookie.split_once('=').unwrap().1.to_owned();
    let empreinte: String =
        sqlx::query_scalar("SELECT encode(jeton_hache, 'hex') FROM session WHERE id = $1")
            .bind(autre.id.uuid())
            .fetch_one(&env.admin)
            .await
            .unwrap();
    let interdits = vec![
        jeton,
        csrf,
        cookie_de(&autre).split_once('=').unwrap().1.to_owned(),
        autre.jeton.csrf(),
        empreinte,
        s.id.to_string(),
        autre.id.to_string(),
        c.administrateur.to_string(),
    ];
    verifier_liste_blanche(&evenements, &interdits);
}
