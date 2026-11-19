//! Cas d'isolation des routes du module `moi` : ses sessions et ses cabinets, avec des sessions
//! réelles ; aucun utilisateur ne voit ni n'agit sur ce qui est à un autre.

use api::base::session::NatureSession;
use api::serveur::routes::{Methode, Route};
use axum::http::StatusCode;
use uuid::Uuid;

use super::{
    CasModule, Ctx, Scenario, cookie_de, ids_json, lire_s, muter_s, ouvrir, session_vue,
    verifier_absent,
};

pub(super) const CAS: CasModule = CasModule {
    attentes: CasModule::AUCUN.attentes,
    hors_cabinet,
};

fn hors_cabinet(route: &Route) -> Option<Scenario> {
    Some(match (route.methode, route.chemin) {
        (Methode::Get, "/api/moi/sessions") => |c| Box::pin(cas_liste_sessions(c)),
        (Methode::Delete, "/api/moi/sessions/{session}") => |c| Box::pin(cas_revocation(c)),
        (Methode::Get, "/api/moi/cabinets") => |c| Box::pin(cas_cabinets(c)),
        _ => return None,
    })
}

async fn cas_liste_sessions(c: &Ctx<'_>) {
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
    let a = &m.a;
    let complete = NatureSession::Complete;
    let sa2 = ouvrir(env, horloge, a.administrateur, complete).await;
    let r = lire_s(routeur, "/api/moi/sessions", sa).await;
    let vues = ids_json(&r);
    for s in [sa, &sa2] {
        assert!(vues.contains(&s.id.to_string()), "{}", r.texte());
    }
    for autre in [&sb, &s_multi] {
        assert!(!vues.contains(&autre.id.to_string()), "{}", r.texte());
    }
    // Ni jeton, ni empreinte, ni jeton anti-CSRF.
    for s in [sa, &sa2] {
        let cookie = cookie_de(s);
        let jeton = cookie.split_once('=').unwrap().1;
        assert!(!r.texte().contains(jeton));
        assert!(!r.texte().contains(&s.jeton.csrf()));
    }
    let cles: Vec<String> = r.json()[0].as_object().unwrap().keys().cloned().collect();
    for cle in &cles {
        assert!(
            [
                "id",
                "nature",
                "methode",
                "cree_le",
                "derniere_activite",
                "echeance",
                "appareil",
                "adresse",
                "courante"
            ]
            .contains(&cle.as_str()),
            "champ inattendu {cle}"
        );
    }
    // B (qui a d'autres sessions ouvertes par les cas précédents) : aucune de A.
    let vues_b = ids_json(&lire_s(routeur, "/api/moi/sessions", sb).await);
    assert!(vues_b.contains(&sb.id.to_string()));
    for autre in [sa, &sa2, s_multi] {
        assert!(!vues_b.contains(&autre.id.to_string()));
    }
}

async fn cas_revocation(c: &Ctx<'_>) {
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
    let a = &m.a;
    let complete = NatureSession::Complete;
    let sa2 = ouvrir(env, horloge, a.administrateur, complete).await;
    let csrf = sa.jeton.csrf();
    let reference = muter_s(
        routeur,
        "DELETE",
        &format!("/api/moi/sessions/{}", Uuid::new_v4()),
        sa,
        &csrf,
    )
    .await;
    assert_eq!(reference.statut, StatusCode::NOT_FOUND);
    // La session de B, ou celle du membre des deux cabinets : absente pour A.
    for autre in [&sb, &s_multi] {
        let r = muter_s(
            routeur,
            "DELETE",
            &format!("/api/moi/sessions/{}", autre.id),
            sa,
            &csrf,
        )
        .await;
        verifier_absent(&r, &reference, "session d'autrui");
        assert!(session_vue(routeur, autre).await.is_some());
    }
    // Sa propre autre session : révoquée aussitôt ; une seconde fois, absente.
    let chemin = format!("/api/moi/sessions/{}", sa2.id);
    let r = muter_s(routeur, "DELETE", &chemin, sa, &csrf).await;
    assert_eq!(r.statut, StatusCode::NO_CONTENT, "{}", r.texte());
    assert_eq!(session_vue(routeur, &sa2).await, None);
    let r = muter_s(routeur, "DELETE", &chemin, sa, &csrf).await;
    verifier_absent(&r, &reference, "session déjà révoquée");
    assert!(session_vue(routeur, sa).await.is_some());
}

async fn cas_cabinets(c: &Ctx<'_>) {
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
    let (a, b) = (&m.a, &m.b);
    let complete = NatureSession::Complete;
    let s_collab = ouvrir(env, horloge, a.collaborateur, complete).await;
    for (s, attendus) in [
        (sa, vec![a.cabinet]),
        (sb, vec![b.cabinet]),
        (&s_collab, vec![a.cabinet]),
        (s_multi, vec![a.cabinet, b.cabinet]),
    ] {
        let r = lire_s(routeur, "/api/moi/cabinets", s).await;
        let mut attendus: Vec<String> = attendus.iter().map(ToString::to_string).collect();
        attendus.sort();
        assert_eq!(ids_json(&r), attendus, "{}", r.texte());
    }
    let r = lire_s(routeur, "/api/moi/cabinets", s_multi).await;
    let json = r.json();
    let roles: Vec<&str> = json
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["role"].as_str().unwrap())
        .collect();
    assert_eq!(roles, vec!["administrateur", "collaborateur"]);
}
