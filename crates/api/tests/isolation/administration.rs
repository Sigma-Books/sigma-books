//! Cas d'isolation des routes du module `administration` (PR 9a ; ADR 0012, 0023) : membres,
//! rôles, suspension, affectations, sessions d'un membre, invitations, réinitialisation,
//! séparation des tâches ; usage d'un lien et acceptation d'une invitation (hors cabinet).
//!
//! Monde : un membre jetable (`{membre}`), une invitation (`{invitation}`) et une
//! réinitialisation (`{reinitialisation}`) en attente dans chaque cabinet, sous le premier
//! dossier. Un membre, une invitation ou une réinitialisation de B placés dans le chemin de A :
//! même 404 qu'un identifiant inexistant (généré par le harnais).

#![allow(clippy::many_single_char_names)]

use api::base::liens::{self, NatureLien};
use api::base::{ContexteTenant, IdCabinet, IdUtilisateur, ouvrir_transaction};
use api::serveur::routes::Methode::{Delete, Get, Post};
use api::serveur::routes::{Methode, Route};
use axum::Router;
use axum::http::StatusCode;
use sqlx::PgPool;
use uuid::Uuid;

use super::commun::serveur::{Reponse, envoyer, lire};
use super::{Attendu, CasModule, Ctx, Monde, Resultat, Scenario, attendu};

pub(super) const CAS: CasModule = CasModule {
    attentes,
    hors_cabinet,
};

/// Crée dans le cabinet un membre jetable (auditeur), une invitation en attente et une
/// réinitialisation en attente (du collaborateur) ; rend leurs identifiants.
pub(super) async fn doter(
    admin: &PgPool,
    cabinet: IdCabinet,
    collaborateur: IdUtilisateur,
) -> (Uuid, Uuid, Uuid) {
    let membre = IdUtilisateur::nouveau();
    let (invitation, reinitialisation) = (Uuid::new_v4(), Uuid::new_v4());
    let mut tx = admin.begin().await.unwrap();
    super::creer_utilisateur(&mut tx, membre, "Membre jetable").await;
    super::ajouter_membre(&mut tx, cabinet, membre, "auditeur").await;
    sqlx::query(
        "INSERT INTO invitation (cabinet_id, id, courriel, nom, role, cree_le, etat) \
         VALUES ($1, $2, $3, 'Invité', 'auditeur', now(), 'en_attente')",
    )
    .bind(cabinet.uuid())
    .bind(invitation)
    .bind(format!("{invitation}@exemple.invalid"))
    .execute(&mut *tx)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO reinitialisation (cabinet_id, id, utilisateur_id, motif, \
                                       verification_hors_bande, cree_le, echeance, etat) \
         VALUES ($1, $2, $3, 'Motif', true, now(), now() + interval '24 hours', 'en_attente')",
    )
    .bind(cabinet.uuid())
    .bind(reinitialisation)
    .bind(collaborateur.uuid())
    .execute(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();
    (membre.uuid(), invitation, reinitialisation)
}

fn attentes(route: &Route, m: &Monde) -> Option<Vec<Attendu>> {
    use Resultat::{Cree, Statut, Tableau};
    let (a, b) = (&m.a, &m.b);
    let interdit = || Statut(StatusCode::FORBIDDEN);
    let absent = || Statut(StatusCode::NOT_FOUND);
    let ok = || Statut(StatusCode::OK);
    let conflit = || Statut(StatusCode::CONFLICT);
    // Tout le monde, sauf l'administrateur du cabinet du chemin, est refusé : collaborateur,
    // valideur, auditeur de A, et le membre des deux cabinets (collaborateur dans B).
    let refus = |cabinet: &super::CabinetTest| -> Vec<Attendu> {
        vec![
            attendu(
                cabinet.collaborateur,
                cabinet.cabinet,
                cabinet.dossier,
                interdit(),
            ),
            attendu(m.valideur, a.cabinet, a.dossier, interdit()),
            attendu(m.auditeur, a.cabinet, a.dossier, interdit()),
            attendu(m.multi, b.cabinet, b.dossier, interdit()),
        ]
    };
    let admin_a = |resultat: Resultat| attendu(a.administrateur, a.cabinet, a.dossier, resultat);
    let multi_a = |resultat: Resultat| attendu(m.multi, a.cabinet, a.dossier, resultat);
    let base = "/api/cabinets/{cabinet}";
    let chemin = route.chemin.strip_prefix(base)?;
    let mut cas = match (route.methode, chemin) {
        (Get, "/membres") => vec![admin_a(Tableau {
            present: a.administrateur.uuid(),
            absents: vec![b.administrateur.uuid(), b.collaborateur.uuid()],
        })],
        // Premier appel : l'état change ; le second (même effet) ne change plus rien. Désactivée
        // puis réactivée (l'ordre de la table), la séparation des tâches retrouve son état.
        (
            Post,
            "/membres/{membre}/role" | "/membres/{membre}/suspension" | "/separation-taches",
        )
        | (
            Delete,
            "/membres/{membre}/suspension"
            | "/separation-taches"
            | "/invitations/{invitation}"
            | "/reinitialisations/{reinitialisation}",
        ) => vec![admin_a(ok()), multi_a(conflit())],
        // Dossier inexistant dans le corps d'exemple, accès ou session inexistants : 404.
        (Post, "/membres/{membre}/acces")
        | (Delete, "/membres/{membre}/acces/{acces}" | "/membres/{membre}/sessions/{session}") => {
            vec![admin_a(absent())]
        }
        (Get, "/membres/{membre}/sessions" | "/invitations" | "/reinitialisations") => {
            vec![admin_a(ok())]
        }
        (Post, "/invitations" | "/membres/{membre}/reinitialisation") => {
            vec![admin_a(Cree), multi_a(conflit())]
        }
        _ => return None,
    };
    cas.extend(refus(a));
    Some(cas)
}

fn hors_cabinet(route: &Route) -> Option<Scenario> {
    Some(match (route.methode, route.chemin) {
        (Methode::Post, "/api/liens/utilisation") => |c| Box::pin(cas_utiliser_lien(c)),
        (Methode::Post, "/api/moi/invitations/acceptation") => |c| Box::pin(cas_accepter(c)),
        _ => return None,
    })
}

/// Invitation en attente dans `cabinet` pour `courriel` et lien émis par l'administrateur.
async fn inviter(c: &Ctx<'_>, cabinet: IdCabinet, admin: IdUtilisateur, courriel: &str) -> String {
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO invitation (cabinet_id, id, courriel, nom, role, cree_le, etat) \
         VALUES ($1, $2, $3, 'Invité', 'auditeur', now(), 'en_attente')",
    )
    .bind(cabinet.uuid())
    .bind(id)
    .bind(courriel)
    .execute(&c.env.admin)
    .await
    .unwrap();
    let mut tx = ouvrir_transaction(&c.env.application, ContexteTenant::cabinet(cabinet, admin))
        .await
        .unwrap();
    let jeton = liens::emettre(
        &mut tx,
        NatureLien::Invitation,
        id,
        time::OffsetDateTime::now_utc(),
    )
    .await
    .unwrap();
    tx.confirmer().await.unwrap();
    jeton.texte()
}

async fn appartenances(env: &super::commun::Env, courriel: &str) -> Vec<(Uuid, String)> {
    sqlx::query_as(
        "SELECT a.cabinet_id, a.role FROM appartenance a JOIN utilisateur u ON u.id = a.utilisateur_id \
         WHERE u.courriel = $1 ORDER BY a.cabinet_id",
    )
    .bind(courriel)
    .fetch_all(&env.admin)
    .await
    .unwrap()
}

/// Un lien du cabinet A ne donne que l'appartenance à A : jamais B, jamais ses données.
async fn cas_utiliser_lien(c: &Ctx<'_>) {
    let (a, b) = (&c.m.a, &c.m.b);
    let courriel = format!("{}@exemple.invalid", Uuid::new_v4());
    let lien = inviter(c, a.cabinet, a.administrateur, &courriel).await;
    let r: Reponse = envoyer(
        c.routeur,
        "/api/liens/utilisation",
        None,
        &serde_json::json!({ "jeton": lien }).to_string(),
        None,
    )
    .await;
    assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
    assert_eq!(
        appartenances(c.env, &courriel).await,
        vec![(a.cabinet.uuid(), "auditeur".to_owned())]
    );
    // La session d'enrôlement ne lit rien, ni de A ni de B.
    let cookie = r.entetes["set-cookie"]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_owned();
    for chemin in [
        format!("/api/cabinets/{}", a.cabinet),
        format!("/api/cabinets/{}", b.cabinet),
        format!("/api/cabinets/{}/dossiers", b.cabinet),
    ] {
        let r =
            super::commun::serveur::appeler_session(c.routeur, "GET", &chemin, Some(&cookie), None)
                .await;
        assert_eq!(r.statut, StatusCode::UNAUTHORIZED, "{chemin}");
    }
    // Les invitations de B n'ont pas bougé.
    let n: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM invitation WHERE cabinet_id = $1 AND etat <> 'en_attente'",
    )
    .bind(b.cabinet.uuid())
    .fetch_one(&c.env.admin)
    .await
    .unwrap();
    assert_eq!(n, 0);
    // Un lien de A, rejoué : refusé.
    let r = envoyer(
        c.routeur,
        "/api/liens/utilisation",
        None,
        &serde_json::json!({ "jeton": lien }).to_string(),
        None,
    )
    .await;
    assert_eq!(r.statut, StatusCode::UNAUTHORIZED);
}

async fn poster(c: &Ctx<'_>, s: &api::serveur::session::SessionOuverte, corps: &str) -> Reponse {
    let (cookie, csrf) = (super::cookie_de(s), s.jeton.csrf());
    super::commun::serveur::appeler_session_corps(
        c.routeur,
        "POST",
        "/api/moi/invitations/acceptation",
        Some(&cookie),
        Some(&csrf),
        corps,
    )
    .await
}

/// Seul le titulaire du courriel invité accepte ; l'acceptation n'ajoute que le cabinet du lien.
async fn cas_accepter(c: &Ctx<'_>) {
    let (a, b) = (&c.m.a, &c.m.b);
    let courriel = format!("{}@exemple.invalid", c.m.b.administrateur);
    let lien = inviter(c, a.cabinet, a.administrateur, &courriel).await;
    let corps = serde_json::json!({ "jeton": lien }).to_string();
    let chemin = "/api/moi/invitations/acceptation";
    // L'administrateur de A (invitant) et le membre des deux cabinets ne sont pas l'invité.
    for s in [&c.sa, &c.s_multi] {
        let r = poster(c, s, &corps).await;
        assert_eq!(r.statut, StatusCode::UNPROCESSABLE_ENTITY, "{}", r.texte());
    }
    assert_eq!(
        appartenances(c.env, &courriel).await,
        vec![(b.cabinet.uuid(), "administrateur".to_owned())]
    );
    let r = poster(c, &c.sb, &corps).await;
    assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
    let mut attendues = vec![
        (a.cabinet.uuid(), "auditeur".to_owned()),
        (b.cabinet.uuid(), "administrateur".to_owned()),
    ];
    attendues.sort();
    assert_eq!(appartenances(c.env, &courriel).await, attendues);
    // Sans session : refusé.
    let r = envoyer(c.routeur, chemin, None, &corps, None).await;
    assert_eq!(r.statut, StatusCode::UNAUTHORIZED);
    let _ = lire;
}

/// Cedar forcé à « autorisé » : le collaborateur n'agit toujours pas en administrateur (lecture
/// de l'appartenance en base) et ne lit rien de l'administration ; un cabinet d'autrui reste 404.
pub(super) async fn quand_cedar_se_trompe(routeur: &Router, m: &Monde) {
    use api::serveur::routes::ROUTES;
    let (a, b) = (&m.a, &m.b);
    let administration = ROUTES
        .iter()
        .filter(|r| r.cas_usage.module() == "administration" && r.portee.du_cabinet());
    for route in administration {
        let chemin = super::chemin(route, a.cabinet.uuid(), a.dossier.uuid());
        let reponse = super::appel(routeur, route, &chemin, Some(a.collaborateur)).await;
        assert!(
            matches!(
                reponse.statut,
                StatusCode::FORBIDDEN | StatusCode::NOT_FOUND
            ),
            "{} {} : {}",
            route.methode.nom(),
            route.chemin,
            reponse.statut
        );
        assert_ne!(reponse.statut, StatusCode::OK);
        let autrui = super::chemin(route, b.cabinet.uuid(), b.dossier.uuid());
        let reponse = super::appel(routeur, route, &autrui, Some(a.collaborateur)).await;
        assert_eq!(reponse.statut, StatusCode::NOT_FOUND, "{}", route.chemin);
    }
    // Rien n'a bougé pour le collaborateur de A.
    let r = lire(
        routeur,
        &format!("/api/cabinets/{}/membres", a.cabinet),
        Some(a.collaborateur),
    )
    .await;
    assert_eq!(r.statut, StatusCode::FORBIDDEN, "{}", r.texte());
}
