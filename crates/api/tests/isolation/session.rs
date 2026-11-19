//! Cas d'isolation des routes du module `session` : la session courante, avec des sessions
//! réelles de deux utilisateurs de cabinets différents et du membre des deux cabinets.

use api::serveur::routes::{Methode, Route};
use axum::http::StatusCode;

use super::{CasModule, Ctx, Scenario, appeler_session, lire_s, muter_s, session_vue};

pub(super) const CAS: CasModule = CasModule {
    attentes: CasModule::AUCUN.attentes,
    hors_cabinet,
};

fn hors_cabinet(route: &Route) -> Option<Scenario> {
    Some(match (route.methode, route.chemin) {
        (Methode::Get, "/api/session") => |c| Box::pin(cas_etat_session(c)),
        (Methode::Post, "/api/session/deconnexion") => |c| Box::pin(cas_deconnexion(c)),
        (Methode::Post, "/api/session/rotation") => |c| Box::pin(cas_rotation(c)),
        _ => return None,
    })
}

async fn cas_etat_session(c: &Ctx<'_>) {
    let Ctx {
        routeur, sa, sb, ..
    } = c;
    assert_eq!(session_vue(routeur, sa).await, Some(sa.id.to_string()));
    assert_eq!(session_vue(routeur, sb).await, Some(sb.id.to_string()));
    let r = lire_s(routeur, "/api/session", sa).await;
    assert_eq!(r.json()["csrf"], sa.jeton.csrf());
    let r = appeler_session(routeur, "GET", "/api/session", None, None).await;
    assert_eq!(r.statut, StatusCode::UNAUTHORIZED);
}

async fn cas_deconnexion(c: &Ctx<'_>) {
    let Ctx {
        routeur,
        sa,
        sb,
        s_multi,
        ..
    } = c;
    let chemin = "/api/session/deconnexion";
    // Jeton anti-CSRF d'une autre session : 403, rien n'est révoqué.
    let r = muter_s(routeur, "POST", chemin, sa, &sb.jeton.csrf()).await;
    assert_eq!(r.statut, StatusCode::FORBIDDEN, "{}", r.texte());
    assert!(session_vue(routeur, sa).await.is_some());
    let r = muter_s(routeur, "POST", chemin, sa, &sa.jeton.csrf()).await;
    assert_eq!(r.statut, StatusCode::NO_CONTENT, "{}", r.texte());
    assert!(
        r.entetes["set-cookie"]
            .to_str()
            .unwrap()
            .starts_with("__Host-sigma_session=; Max-Age=0;")
    );
    assert_eq!(session_vue(routeur, sa).await, None);
    // Les sessions des autres restent ouvertes.
    assert!(session_vue(routeur, sb).await.is_some());
    assert!(session_vue(routeur, s_multi).await.is_some());
    let r = appeler_session(routeur, "POST", chemin, None, Some("x")).await;
    assert_eq!(r.statut, StatusCode::UNAUTHORIZED);
}

async fn cas_rotation(c: &Ctx<'_>) {
    let Ctx {
        routeur,
        sa,
        sb,
        s_multi,
        ..
    } = c;
    let chemin = "/api/session/rotation";
    let r = muter_s(routeur, "POST", chemin, sa, &sb.jeton.csrf()).await;
    assert_eq!(r.statut, StatusCode::FORBIDDEN, "{}", r.texte());
    let r = muter_s(routeur, "POST", chemin, sa, &sa.jeton.csrf()).await;
    assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
    let nouveau = r.entetes["set-cookie"]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_owned();
    // L'ancien jeton : 401 ; le nouveau désigne la même session.
    assert_eq!(session_vue(routeur, sa).await, None);
    let r = appeler_session(routeur, "GET", "/api/session", Some(&nouveau), None).await;
    assert_eq!(r.json()["id"], sa.id.to_string());
    // Les sessions des autres sont intactes.
    assert_eq!(session_vue(routeur, sb).await, Some(sb.id.to_string()));
    assert!(session_vue(routeur, s_multi).await.is_some());
}
