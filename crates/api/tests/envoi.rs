//! Envoi des liens par courriel (PR 9b de `docs/lots/L2.md` ; ADR 0012, 0022, 0024 ; F12.1,
//! F12.2) : la file de tâches émet le lien au moment de l'envoi, le remet au transport hors
//! transaction, rejoue avec un lien neuf, n'envoie rien pour une cible qui n'est plus en
//! attente, et respecte le délai de contestation de 24 heures d'une réinitialisation.
//! Transport en mémoire : l'échange SMTP lui-même est éprouvé dans `envoi_smtp.rs`.
#![cfg(feature = "tests-postgres")]
// Fichier de test : les `expect`/`unwrap` y sont voulus.
#![allow(clippy::expect_used, clippy::unwrap_used)]

mod commun;

use std::sync::Arc;

use api::base::IdUtilisateur;
use api::base::session::NatureSession;
use api::envoi::tache::{Expediteur, Reglages};
use api::envoi::transport::ErreurEnvoi;
use api::serveur::Horloge;
use authz::Autorisation;
use axum::Router;
use axum::http::StatusCode;
use commun::envoi::{TransportMemoire, jeton_du_message};
use commun::serveur::{
    HorlogeTest, ORIGINE, Reponse, appeler, appeler_session_corps, application_sessions_avec,
    cookie_de, ouvrir, reglages_de_test, routeur_de,
};
use commun::{CabinetTest, creer_cabinet, env};
use serde_json::json;
use time::Duration;
use uuid::Uuid;

/// La file de tâches est **unique pour l'instance** : deux tests qui la parcourent en même temps
/// se prendraient leurs tâches. Un verrou les sérialise, et chaque banc commence par clore les
/// envois laissés par les autres binaires de test.
static FILE: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

struct Banc {
    _verrou: tokio::sync::MutexGuard<'static, ()>,
    env: commun::Env,
    horloge: HorlogeTest,
    routeur: Router,
    c: CabinetTest,
    transport: TransportMemoire,
    expediteur: Expediteur,
}

type Session = (String, String);

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
    /// Passage de la file **des liens seulement** : les alertes (PR 9c, `alertes.rs`) que
    /// produisent les mêmes actions sont closes avant, pour ne compter que les liens.
    async fn passer(&self) -> api::envoi::tache::Bilan {
        sqlx::query(
            "UPDATE tache SET etat = 'terminee', termine_le = now(), reserve_jusqua = NULL, \
                              prise = NULL WHERE nature = 'notification' AND etat <> 'terminee'",
        )
        .execute(&self.env.admin)
        .await
        .unwrap();
        self.expediteur.passer().await.unwrap()
    }

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

    /// Invite un courriel neuf ; rend (identifiant de l'invitation, courriel).
    async fn inviter(&self, admin: &Session) -> (Uuid, String) {
        let courriel = format!("invite.{}@cabinet.example", Uuid::new_v4().simple());
        let r = self
            .appel(
                "POST",
                &self.chemin("/invitations"),
                admin,
                &json!({"courriel": courriel, "nom": "Awa Diop", "role": "collaborateur"})
                    .to_string(),
            )
            .await;
        assert_eq!(r.statut, StatusCode::CREATED, "{}", r.texte());
        (
            Uuid::parse_str(r.json()["id"].as_str().unwrap()).unwrap(),
            courriel,
        )
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

    async fn tache(&self, objet: Uuid) -> (String, Option<String>, i32) {
        sqlx::query_as(
            "SELECT etat, derniere_issue, tentatives FROM tache \
             WHERE nature = 'courriel' AND objet = $1 ORDER BY cree_le DESC LIMIT 1",
        )
        .bind(objet)
        .fetch_one(&self.env.admin)
        .await
        .unwrap()
    }

    async fn liens_fermes(&self, cible: Uuid, raison: &str) -> i64 {
        sqlx::query_scalar("SELECT count(*) FROM lien_acces WHERE cible = $1 AND raison = $2")
            .bind(cible)
            .bind(raison)
            .fetch_one(&self.env.admin)
            .await
            .unwrap()
    }
}

#[tokio::test]
async fn l_invitation_part_par_courriel_avec_un_lien_qui_fonctionne() {
    let b = banc().await;
    let admin = b.session(b.c.administrateur).await;
    let (id, courriel) = b.inviter(&admin).await;
    // Rien n'est parti avant le passage de la file ; la tâche attend, sans lien ni courriel.
    assert!(b.transport.vide());
    assert_eq!(b.tache(id).await.0, "en_attente");
    let bilan = b.passer().await;
    assert_eq!((bilan.prises, bilan.envoyes), (1, 1));
    let remis = b.transport.remis();
    assert_eq!(remis.len(), 1);
    assert_eq!(remis[0].destinataire, courriel);
    assert_eq!(remis[0].sujet, api::envoi::modele::SUJET_INVITATION);
    assert!(remis[0].texte.contains("Awa Diop"));
    assert!(remis[0].texte.contains(&format!("{ORIGINE}/lien#")));
    let (etat, issue, _) = b.tache(id).await;
    assert_eq!(
        (etat.as_str(), issue.as_deref()),
        ("terminee", Some("envoye"))
    );
    // Le lien remis ouvre une session d'enrôlement ; la base n'a gardé que son empreinte.
    let jeton = jeton_du_message(&remis[0]);
    let n: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM lien_acces WHERE encode(jeton_hache, 'escape') = $1",
    )
    .bind(&jeton)
    .fetch_one(&b.env.admin)
    .await
    .unwrap();
    assert_eq!(n, 0);
    let r = b.utiliser_lien(&jeton).await;
    assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
    assert_eq!(r.json()["nature"], "enrolement");
    // Un second passage n'envoie plus rien.
    assert_eq!(b.passer().await.prises, 0);
    assert_eq!(b.transport.remis().len(), 1);
}

#[tokio::test]
async fn le_renvoi_remplace_le_lien() {
    let b = banc().await;
    let admin = b.session(b.c.administrateur).await;
    let (id, _) = b.inviter(&admin).await;
    b.passer().await;
    let premier = jeton_du_message(&b.transport.remis()[0]);
    let r = b
        .appel(
            "POST",
            &b.chemin(&format!("/invitations/{id}/renvoi")),
            &admin,
            "",
        )
        .await;
    assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
    assert_eq!(r.json()["envoi_planifie"], true);
    // Une demande de plus pendant que l'envoi attend : idempotente.
    let r = b
        .appel(
            "POST",
            &b.chemin(&format!("/invitations/{id}/renvoi")),
            &admin,
            "",
        )
        .await;
    assert_eq!(r.json()["envoi_planifie"], false);
    b.passer().await;
    let remis = b.transport.remis();
    assert_eq!(remis.len(), 2);
    let second = jeton_du_message(&remis[1]);
    assert_ne!(premier, second);
    // Le premier lien est fermé, le second fonctionne.
    assert_eq!(
        b.utiliser_lien(&premier).await.statut,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(b.utiliser_lien(&second).await.statut, StatusCode::OK);
    assert_eq!(b.liens_fermes(id, "remplace").await, 1);
    // Une invitation acceptée ne se renvoie plus ; un inconnu : 404.
    let r = b
        .appel(
            "POST",
            &b.chemin(&format!("/invitations/{id}/renvoi")),
            &admin,
            "",
        )
        .await;
    assert_eq!(r.statut, StatusCode::CONFLICT);
    let r = b
        .appel(
            "POST",
            &b.chemin(&format!("/invitations/{}/renvoi", Uuid::new_v4())),
            &admin,
            "",
        )
        .await;
    assert_eq!(r.statut, StatusCode::NOT_FOUND);
    // Un collaborateur ne le peut pas.
    let s = b.session(b.c.collaborateur).await;
    let r = b
        .appel(
            "POST",
            &b.chemin(&format!("/invitations/{id}/renvoi")),
            &s,
            "",
        )
        .await;
    assert_eq!(r.statut, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn un_echec_provisoire_est_rejoue_avec_un_lien_neuf() {
    let b = banc().await;
    let admin = b.session(b.c.administrateur).await;
    let (id, _) = b.inviter(&admin).await;
    b.transport
        .programmer([Err(ErreurEnvoi::Temporaire), Err(ErreurEnvoi::Tls)]);
    let bilan = b.passer().await;
    assert_eq!((bilan.prises, bilan.reportes, bilan.envoyes), (1, 1, 0));
    let (etat, issue, _) = b.tache(id).await;
    assert_eq!(
        (etat.as_str(), issue.as_deref()),
        ("en_attente", Some("smtp_temporaire"))
    );
    // Pas de rejeu avant le délai (une minute).
    assert_eq!(b.passer().await.prises, 0);
    b.horloge.avancer(Duration::seconds(61));
    b.passer().await;
    let (_, issue, _) = b.tache(id).await;
    assert_eq!(issue.as_deref(), Some("smtp_tls"));
    // Délai doublé : deux minutes.
    b.horloge.avancer(Duration::seconds(100));
    assert_eq!(b.passer().await.prises, 0);
    b.horloge.avancer(Duration::seconds(30));
    let bilan = b.passer().await;
    assert_eq!(bilan.envoyes, 1);
    assert_eq!(b.tache(id).await.0, "terminee");
    // Trois liens émis (deux jamais remis) : seul le dernier est vivant.
    assert_eq!(b.liens_fermes(id, "remplace").await, 2);
    let jeton = jeton_du_message(&b.transport.remis()[0]);
    assert_eq!(b.utiliser_lien(&jeton).await.statut, StatusCode::OK);
}

#[tokio::test]
async fn un_refus_definitif_n_est_pas_rejoue_et_les_echecs_s_arretent() {
    let b = banc().await;
    let admin = b.session(b.c.administrateur).await;
    let (id, _) = b.inviter(&admin).await;
    b.transport.programmer([Err(ErreurEnvoi::Refus)]);
    let bilan = b.passer().await;
    assert_eq!((bilan.clos, bilan.envoyes), (1, 0));
    let (etat, issue, _) = b.tache(id).await;
    assert_eq!(
        (etat.as_str(), issue.as_deref()),
        ("terminee", Some("smtp_refus"))
    );
    b.horloge.avancer(Duration::hours(2));
    assert_eq!(b.passer().await.prises, 0);
    assert_eq!(b.transport.tentatives(), 1);
    // Échecs provisoires répétés : abandon à la huitième tentative.
    let admin = b.session(b.c.administrateur).await;
    let (id2, _) = b.inviter(&admin).await;
    b.transport
        .programmer((0..20).map(|_| Err(ErreurEnvoi::Temporaire)));
    for _ in 0..12 {
        b.passer().await;
        b.horloge.avancer(Duration::hours(2));
    }
    let (etat, issue, tentatives) = b.tache(id2).await;
    assert_eq!(
        (etat.as_str(), issue.as_deref(), tentatives),
        ("terminee", Some("abandon"), 8)
    );
}

#[tokio::test]
async fn une_invitation_revoquee_avant_l_envoi_ne_part_pas() {
    let b = banc().await;
    let admin = b.session(b.c.administrateur).await;
    let (id, _) = b.inviter(&admin).await;
    let r = b
        .appel(
            "DELETE",
            &b.chemin(&format!("/invitations/{id}")),
            &admin,
            "",
        )
        .await;
    assert_eq!(r.statut, StatusCode::OK);
    let bilan = b.passer().await;
    assert_eq!((bilan.prises, bilan.clos, bilan.envoyes), (1, 1, 0));
    assert!(b.transport.vide());
    assert_eq!(b.tache(id).await.1.as_deref(), Some("sans_objet"));
}

#[tokio::test]
async fn deux_expediteurs_concurrents_n_envoient_qu_une_fois() {
    let b = banc().await;
    let admin = b.session(b.c.administrateur).await;
    b.inviter(&admin).await;
    let second = b.expediteur.clone();
    let (a, c) = tokio::join!(b.expediteur.passer(), second.passer());
    assert_eq!(a.unwrap().prises + c.unwrap().prises, 1);
    assert_eq!(b.transport.remis().len(), 1);
}

#[tokio::test]
async fn la_reinitialisation_ne_part_qu_apres_le_delai_de_contestation() {
    let b = banc().await;
    let admin = b.session(b.c.administrateur).await;
    let cible = b.c.collaborateur;
    let r = b
        .appel(
            "POST",
            &b.chemin(&format!("/membres/{cible}/reinitialisation")),
            &admin,
            &json!({"motif_code": "autre", "motif": "Appel de vérification", "verification_hors_bande": true}).to_string(),
        )
        .await;
    assert_eq!(r.statut, StatusCode::CREATED, "{}", r.texte());
    let id = Uuid::parse_str(r.json()["id"].as_str().unwrap()).unwrap();
    // Pendant le délai : aucune prise, aucun courriel, aucun lien.
    assert_eq!(b.passer().await.prises, 0);
    b.horloge.avancer(Duration::hours(23));
    assert_eq!(b.passer().await.prises, 0);
    // Une tâche rendue échue trop tôt (horloge, rattrapage) ne part pas non plus.
    sqlx::query(
        "UPDATE tache SET prochaine_tentative = now() - interval '1 minute' WHERE objet = $1",
    )
    .bind(id)
    .execute(&b.env.admin)
    .await
    .unwrap();
    let bilan = b.passer().await;
    assert_eq!((bilan.prises, bilan.reportes, bilan.envoyes), (1, 1, 0));
    assert!(b.transport.vide());
    assert_eq!(b.liens_fermes(id, "remplace").await, 0);
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM lien_acces WHERE cible = $1")
        .bind(id)
        .fetch_one(&b.env.admin)
        .await
        .unwrap();
    assert_eq!(n, 0, "aucun lien émis avant le terme du délai");
    // Au terme du délai : le courriel part, le lien dure une heure.
    b.horloge.avancer(Duration::hours(1) + Duration::minutes(2));
    let bilan = b.passer().await;
    assert_eq!(bilan.envoyes, 1, "{bilan:?}");
    let remis = b.transport.remis();
    assert_eq!(remis[0].sujet, api::envoi::modele::SUJET_REINITIALISATION);
    let jeton = jeton_du_message(&remis[0]);
    let expire: bool = sqlx::query_scalar(
        "SELECT expire_le - cree_le = interval '1 hour' FROM lien_acces WHERE cible = $1",
    )
    .bind(id)
    .fetch_one(&b.env.admin)
    .await
    .unwrap();
    assert!(expire);
    assert_eq!(b.utiliser_lien(&jeton).await.statut, StatusCode::OK);
}

#[tokio::test]
async fn une_reinitialisation_annulee_ne_part_pas() {
    let b = banc().await;
    let admin = b.session(b.c.administrateur).await;
    let cible = b.c.collaborateur;
    let r = b
        .appel(
            "POST",
            &b.chemin(&format!("/membres/{cible}/reinitialisation")),
            &admin,
            &json!({"motif_code": "autre", "motif": "Appel de vérification", "verification_hors_bande": true}).to_string(),
        )
        .await;
    let id = r.json()["id"].as_str().unwrap().to_owned();
    let r = b
        .appel(
            "DELETE",
            &b.chemin(&format!("/reinitialisations/{id}")),
            &admin,
            "",
        )
        .await;
    assert_eq!(r.statut, StatusCode::OK);
    b.horloge.avancer(Duration::hours(25));
    let bilan = b.passer().await;
    assert_eq!((bilan.clos, bilan.envoyes), (1, 0));
    assert!(b.transport.vide());
}

#[tokio::test]
async fn une_reinitialisation_perimee_ne_bloque_pas_une_nouvelle_demande() {
    let b = banc().await;
    let cible = b.c.collaborateur;
    let corps =
        json!({"motif_code": "autre", "motif": "Appel de vérification", "verification_hors_bande": true}).to_string();
    let chemin = b.chemin(&format!("/membres/{cible}/reinitialisation"));
    let admin = b.session(b.c.administrateur).await;
    let r = b.appel("POST", &chemin, &admin, &corps).await;
    let premiere = r.json()["id"].as_str().unwrap().to_owned();
    assert_eq!(
        b.appel("POST", &chemin, &admin, &corps).await.statut,
        StatusCode::CONFLICT
    );
    // 24 h de délai, puis plus de 24 h sans usage du lien : périmée.
    b.horloge.avancer(Duration::hours(49));
    let admin = b.session(b.c.administrateur).await;
    let r = b.appel("POST", &chemin, &admin, &corps).await;
    assert_eq!(r.statut, StatusCode::CREATED, "{}", r.texte());
    let etat: String = sqlx::query_scalar("SELECT etat FROM reinitialisation WHERE id = $1::uuid")
        .bind(premiere)
        .fetch_one(&b.env.admin)
        .await
        .unwrap();
    assert_eq!(etat, "expiree");
}

#[tokio::test]
async fn la_file_de_courriels_est_isolee_par_cabinet() {
    let b = banc().await;
    let autre = creer_cabinet(&b.env.admin, "Autre").await;
    let admin_autre = b.session(autre.administrateur).await;
    let admin = b.session(b.c.administrateur).await;
    let (_, mon_courriel) = b.inviter(&admin).await;
    let courriel_autre = format!("autre.{}@cabinet.example", Uuid::new_v4().simple());
    let r = b
        .appel(
            "POST",
            &format!("/api/cabinets/{}/invitations", autre.cabinet),
            &admin_autre,
            &json!({"courriel": courriel_autre, "nom": "Autre", "role": "auditeur"}).to_string(),
        )
        .await;
    assert_eq!(r.statut, StatusCode::CREATED);
    // La file est unique pour l'instance : un passage prend les tâches de tous les cabinets, une
    // par une. Le test n'attend pas qu'un seul passage les prenne toutes (l'ordre et l'instant de
    // prise ne sont pas garantis) ; il vide la file, et vérifie l'invariant d'isolation : chaque
    // courriel part vers l'invité de **son** cabinet, jamais deux fois, et son lien n'ouvre que ce
    // cabinet.
    let mut bilans = Vec::new();
    for _ in 0..20 {
        bilans.push(b.passer().await);
        if b.transport.remis().len() >= 2 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    let remis = b.transport.remis();
    let taches: Vec<(Uuid, String, Option<String>, i32)> = sqlx::query_as(
        "SELECT cabinet_cible, etat, derniere_issue, tentatives FROM tache \
         WHERE nature = 'courriel' AND cabinet_cible = ANY($1)",
    )
    .bind(vec![b.c.cabinet.uuid(), autre.cabinet.uuid()])
    .fetch_all(&b.env.admin)
    .await
    .unwrap();
    assert_eq!(remis.len(), 2, "{bilans:?} {taches:?}");
    let a_moi = remis
        .iter()
        .find(|m| m.destinataire == mon_courriel)
        .unwrap();
    let a_autre = remis
        .iter()
        .find(|m| m.destinataire == courriel_autre)
        .unwrap();
    assert_eq!(
        b.utiliser_lien(&jeton_du_message(a_moi)).await.statut,
        StatusCode::OK
    );
    assert_eq!(
        b.utiliser_lien(&jeton_du_message(a_autre)).await.statut,
        StatusCode::OK
    );
    let cabinets: Vec<(Uuid, String)> = sqlx::query_as(
        "SELECT a.cabinet_id, u.courriel FROM appartenance a JOIN utilisateur u ON u.id = a.utilisateur_id \
         WHERE u.courriel = ANY($1) ORDER BY u.courriel",
    )
    .bind(vec![mon_courriel.clone(), courriel_autre.clone()])
    .fetch_all(&b.env.admin)
    .await
    .unwrap();
    for (cabinet, courriel) in cabinets {
        let attendu = if courriel == mon_courriel {
            b.c.cabinet.uuid()
        } else {
            autre.cabinet.uuid()
        };
        assert_eq!(cabinet, attendu, "{courriel}");
    }
    let _ = b.horloge.maintenant();
}

#[tokio::test]
async fn les_envois_a_une_meme_adresse_sont_plafonnes_sur_24_heures() {
    let b = banc().await;
    let admin = b.session(b.c.administrateur).await;
    let (id, _) = b.inviter(&admin).await;
    let renvoi = b.chemin(&format!("/invitations/{id}/renvoi"));
    // Création + deux renvois = trois envois vers l'adresse ; chaque envoi est traité avant le
    // suivant (un envoi en file vaut déjà renvoi).
    b.expediteur.passer().await.unwrap();
    for _ in 0..2 {
        assert_eq!(
            b.appel("POST", &renvoi, &admin, "").await.statut,
            StatusCode::OK
        );
        b.expediteur.passer().await.unwrap();
    }
    assert_eq!(b.transport.remis().len(), 3);
    // Le quatrième est refusé (429), sans tâche ni courriel de plus.
    let r = b.appel("POST", &renvoi, &admin, "").await;
    assert_eq!(r.statut, StatusCode::TOO_MANY_REQUESTS, "{}", r.texte());
    assert!(r.entetes.contains_key("retry-after"));
    assert_eq!(b.expediteur.passer().await.unwrap().prises, 0);
    // Une invitation à une autre adresse n'est pas touchée par le plafond de la première.
    b.inviter(&admin).await;
}

#[tokio::test]
async fn les_envois_d_un_cabinet_sont_plafonnes_sur_24_heures() {
    let b = banc().await;
    let admin = b.session(b.c.administrateur).await;
    // Cent envois déjà faits aujourd'hui par ce cabinet.
    for _ in 0..api::base::taches::INVITATIONS_MAX_PAR_CABINET_24H {
        sqlx::query(
            "INSERT INTO tache (id, nature, sujet, cabinet_cible, objet, etat, cree_le, \
                                prochaine_tentative, termine_le, derniere_issue) \
             VALUES ($1, 'courriel', 'invitation', $2, $3, 'terminee', now(), now(), now(), 'envoye')",
        )
        .bind(Uuid::new_v4())
        .bind(b.c.cabinet.uuid())
        .bind(Uuid::new_v4())
        .execute(&b.env.admin)
        .await
        .unwrap();
    }
    let r = b
        .appel(
            "POST",
            &b.chemin("/invitations"),
            &admin,
            &json!({"courriel": format!("x.{}@cabinet.example", Uuid::new_v4().simple()), "nom": "X", "role": "auditeur"})
                .to_string(),
        )
        .await;
    assert_eq!(r.statut, StatusCode::TOO_MANY_REQUESTS, "{}", r.texte());
    // Le plafond est celui du cabinet : un autre cabinet n'est pas gêné.
    let autre = creer_cabinet(&b.env.admin, "Autre").await;
    let sa = b.session(autre.administrateur).await;
    let r = b
        .appel(
            "POST",
            &format!("/api/cabinets/{}/invitations", autre.cabinet),
            &sa,
            &json!({"courriel": format!("y.{}@cabinet.example", Uuid::new_v4().simple()), "nom": "Y", "role": "auditeur"})
                .to_string(),
        )
        .await;
    assert_eq!(r.statut, StatusCode::CREATED, "{}", r.texte());
}

#[tokio::test]
async fn une_tache_qui_a_epuise_ses_tentatives_est_abandonnee_sans_traitement() {
    let b = banc().await;
    let admin = b.session(b.c.administrateur).await;
    let (id, _) = b.inviter(&admin).await;
    // Huit prises déjà faites (par exemple huit erreurs internes reprises au bail).
    sqlx::query("UPDATE tache SET tentatives = 8 WHERE objet = $1")
        .bind(id)
        .execute(&b.env.admin)
        .await
        .unwrap();
    let bilan = b.expediteur.passer().await.unwrap();
    assert_eq!((bilan.prises, bilan.clos, bilan.envoyes), (1, 1, 0));
    assert!(b.transport.vide());
    let (etat, issue, _) = b.tache(id).await;
    assert_eq!(
        (etat.as_str(), issue.as_deref()),
        ("terminee", Some("abandon"))
    );
}

#[tokio::test]
async fn une_prise_ne_depasse_jamais_sa_limite() {
    let b = banc().await;
    let admin = b.session(b.c.administrateur).await;
    // Cinq envois en attente ; un expéditeur qui n'en traite qu'un par passage (`lot` = 1) : la
    // prise (`LIMIT 1`) ne met « en cours » qu'une tâche, jamais plus. Une tâche rendue « en
    // cours » en surplus resterait sans travailleur jusqu'à l'expiration de son bail.
    for _ in 0..5 {
        b.inviter(&admin).await;
    }
    // Pool dont chaque connexion **impose le plan** qui rescanne la sous-requête de la prise pour
    // chaque ligne de la table (nested loop sans agrégat ni hachage) : avec l'ancienne écriture
    // (`WHERE id IN (SELECT … LIMIT 1 FOR UPDATE SKIP LOCKED)`), toutes les lignes échues étaient
    // mises « en cours » d'un seul coup (cause de l'échec observé en CI). Cause établie par
    // 512 combinaisons de réglages du planificateur : 6 donnaient « UPDATE 4 » pour `LIMIT 1`.
    let pool_plan_force = sqlx::postgres::PgPoolOptions::new()
        .max_connections(4)
        .after_connect(|c, _| {
            Box::pin(async move {
                sqlx::raw_sql(
                    "SET enable_hashagg = off; SET enable_hashjoin = off; SET enable_material = off; \
                     SET enable_seqscan = off; SET enable_sort = off",
                )
                .execute(&mut *c)
                .await?;
                Ok(())
            })
        })
        .connect(&commun::variable("SIGMA_TEST_APPLICATION_URL"))
        .await
        .unwrap();
    let lent = Expediteur::new(
        pool_plan_force,
        Arc::new(b.transport.clone()),
        Arc::new(b.horloge.clone()),
        ORIGINE.to_owned(),
        Reglages {
            lot: 1,
            ..Reglages::default()
        },
    );
    for attendu in 1..=5_i64 {
        let bilan = lent.passer().await.unwrap();
        assert_eq!(bilan.prises, 1, "passage {attendu}");
        let (terminees, en_cours, en_attente): (i64, i64, i64) = sqlx::query_as(
            "SELECT count(*) FILTER (WHERE etat = 'terminee'), \
                    count(*) FILTER (WHERE etat = 'en_cours'), \
                    count(*) FILTER (WHERE etat = 'en_attente') \
             FROM tache WHERE nature = 'courriel' AND cabinet_cible = $1",
        )
        .bind(b.c.cabinet.uuid())
        .fetch_one(&b.env.admin)
        .await
        .unwrap();
        assert_eq!(
            (terminees, en_cours, en_attente),
            (attendu, 0, 5 - attendu),
            "passage {attendu}"
        );
    }
    assert_eq!(b.transport.remis().len(), 5);
}

#[tokio::test]
async fn les_taches_d_un_cabinet_ne_sont_ni_lues_ni_comptees_pour_un_autre() {
    use api::base::{ContexteTenant, ouvrir_transaction};
    let b = banc().await;
    let autre = creer_cabinet(&b.env.admin, "Autre").await;
    let (sa, sb) = (
        b.session(b.c.administrateur).await,
        b.session(autre.administrateur).await,
    );
    b.inviter(&sa).await;
    for _ in 0..2 {
        let r = b
            .appel(
                "POST",
                &format!("/api/cabinets/{}/invitations", autre.cabinet),
                &sb,
                &json!({"courriel": format!("i.{}@cabinet.example", Uuid::new_v4().simple()), "nom": "I", "role": "auditeur"}).to_string(),
            )
            .await;
        assert_eq!(r.statut, StatusCode::CREATED);
    }
    // Sous la RLS du rôle applicatif, chaque cabinet ne voit que les tâches de sa cible exacte :
    // 1 pour le premier, 2 pour le second, jamais le total.
    for (cabinet, utilisateur, attendu) in [
        (b.c.cabinet, b.c.administrateur, 1_i64),
        (autre.cabinet, autre.administrateur, 2),
    ] {
        let mut tx = ouvrir_transaction(
            &b.env.application,
            ContexteTenant::cabinet(cabinet, utilisateur),
        )
        .await
        .unwrap();
        let visibles: i64 =
            sqlx::query_scalar("SELECT count(*) FROM tache WHERE nature = 'courriel'")
                .fetch_one(tx.connexion())
                .await
                .unwrap();
        assert_eq!(visibles, attendu, "{cabinet}");
        // Même en demandant explicitement le cabinet voisin : rien.
        let voisin = if cabinet == b.c.cabinet {
            autre.cabinet
        } else {
            b.c.cabinet
        };
        let voisines: i64 =
            sqlx::query_scalar("SELECT count(*) FROM tache WHERE cabinet_cible = $1")
                .bind(voisin.uuid())
                .fetch_one(tx.connexion())
                .await
                .unwrap();
        assert_eq!(voisines, 0);
        // Le plafond d'envoi d'un cabinet ne compte que ses propres tâches.
        let plafond = api::base::taches::plafond_invitation(&mut tx, "inconnu@cabinet.example")
            .await
            .unwrap();
        assert_eq!(plafond, None);
        tx.confirmer().await.unwrap();
    }
}
