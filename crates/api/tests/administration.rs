//! Administration du cabinet (PR 9a de `docs/lots/L2.md` ; ADR 0012, 0014, 0017, 0023 ; F1.6,
//! F1.10, F1.12, F1.15) : membres, rôles, suspension, affectations, sessions d'un membre,
//! séparation des tâches, invitations, réinitialisation d'accès par un administrateur, usage des
//! liens à usage unique. Sessions **réelles** (cookie, jeton anti-CSRF), horloge réglable,
//! politiques Cedar réelles.
//!
//! Les cas d'isolation de chaque route sont dans `isolation/administration.rs`.
#![cfg(feature = "tests-postgres")]
// Fichier de test : les `expect`/`unwrap` y sont voulus.
#![allow(clippy::expect_used, clippy::unwrap_used)]

mod commun;

use std::sync::Arc;

use api::base::liens::{self, NatureLien};
use api::base::session::NatureSession;
use api::base::{ContexteTenant, IdUtilisateur, ouvrir_transaction};
use api::serveur::Horloge;
use authz::Autorisation;
use axum::Router;
use axum::http::StatusCode;
use commun::facteurs::{installer_codes, installer_secours};
use commun::serveur::{
    HorlogeTest, ORIGINE, Reponse, appeler, appeler_session_corps, application_sessions_avec,
    cookie_de, ouvrir, reglages_de_test, routeur_de,
};
use commun::{CabinetTest, ajouter_affecte, creer_cabinet, env};
use serde_json::{Value, json};
use time::Duration;
use uuid::Uuid;

struct Banc {
    env: commun::Env,
    horloge: HorlogeTest,
    routeur: Router,
    c: CabinetTest,
}

type Session = (String, String);

async fn banc() -> Banc {
    let env = env().await;
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
    Banc {
        env,
        horloge,
        routeur: routeur_de(&etat),
        c,
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

    /// Ajoute un membre du cabinet (rôle donné, affecté au premier dossier).
    async fn membre(&self, role: &str) -> IdUtilisateur {
        ajouter_affecte(&self.env.admin, &self.c, role).await
    }

    async fn compter(&self, sql: &'static str, id: Uuid) -> i64 {
        sqlx::query_scalar::<_, i64>(sql)
            .bind(id)
            .fetch_one(&self.env.admin)
            .await
            .unwrap()
    }

    /// Émet un lien pour une cible du cabinet, comme le fera la file de tâches (PR 9b).
    async fn lien(&self, nature: NatureLien, cible: Uuid, admin: IdUtilisateur) -> String {
        let mut tx = ouvrir_transaction(
            &self.env.application,
            ContexteTenant::cabinet(self.c.cabinet, admin),
        )
        .await
        .unwrap();
        let jeton = liens::emettre(&mut tx, nature, cible, self.horloge.maintenant())
            .await
            .unwrap();
        tx.confirmer().await.unwrap();
        jeton.texte()
    }

    async fn utiliser_lien(&self, jeton: &str) -> Reponse {
        let requete = axum::http::Request::post("/api/liens/utilisation")
            .header("origin", ORIGINE)
            .header("sec-fetch-site", "same-origin")
            .header("x-sigma-csrf", "1")
            .header("content-type", "application/json")
            .header("user-agent", "Navigateur de test")
            .body(axum::body::Body::from(
                json!({ "jeton": jeton }).to_string(),
            ))
            .unwrap();
        appeler(&self.routeur, requete).await
    }

    /// Entrées du journal d'audit du cabinet, en texte.
    async fn audit(&self) -> Vec<String> {
        let lignes: Vec<(String,)> =
            sqlx::query_as("SELECT contenu FROM audit_cabinet WHERE cabinet_id = $1 ORDER BY rang")
                .bind(self.c.cabinet.uuid())
                .fetch_all(&self.env.admin)
                .await
                .unwrap();
        lignes.into_iter().map(|(c,)| c).collect()
    }

    async fn chaine_verifiee(&self, admin: &Session) {
        let r = self
            .appel("GET", &self.chemin("/audit/verification"), admin, "")
            .await;
        assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
        assert_eq!(r.json()["integre"], true, "{}", r.texte());
    }
}

fn corps(v: &Value) -> String {
    v.to_string()
}

// --- Membres, rôles, suspension ----------------------------------------------------------------

#[tokio::test]
async fn les_membres_ne_se_listent_que_par_un_administrateur() {
    let b = banc().await;
    let admin = b.session(b.c.administrateur).await;
    let r = b.appel("GET", &b.chemin("/membres"), &admin, "").await;
    assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
    let vues = r.json();
    assert_eq!(vues.as_array().unwrap().len(), 2);
    for v in vues.as_array().unwrap() {
        let mut cles: Vec<&String> = v.as_object().unwrap().keys().collect();
        cles.sort();
        assert_eq!(cles, ["actif", "courriel", "id", "nom", "role"]);
    }
    for role in ["valideur", "collaborateur", "auditeur"] {
        let u = b.membre(role).await;
        let s = b.session(u).await;
        let r = b.appel("GET", &b.chemin("/membres"), &s, "").await;
        assert_eq!(r.statut, StatusCode::FORBIDDEN, "{role}");
    }
}

#[tokio::test]
async fn un_changement_de_role_revoque_les_sessions_et_s_inscrit_a_l_audit() {
    let b = banc().await;
    let admin = b.session(b.c.administrateur).await;
    let cible = b.c.collaborateur;
    let s_cible = b.session(cible).await;
    let chemin = b.chemin(&format!("/membres/{cible}/role"));
    let r = b
        .appel(
            "POST",
            &chemin,
            &admin,
            &corps(&json!({"role": "valideur"})),
        )
        .await;
    assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
    assert_eq!(r.json()["role"], "valideur");
    // Ses sessions sont révoquées (ADR 0012).
    let r = b.appel("GET", "/api/moi/sessions", &s_cible, "").await;
    assert_eq!(r.statut, StatusCode::UNAUTHORIZED);
    // Même rôle : rien ne change ; soi-même : refusé ; rôle inconnu et champ inconnu : 400.
    let r = b
        .appel(
            "POST",
            &chemin,
            &admin,
            &corps(&json!({"role": "valideur"})),
        )
        .await;
    assert_eq!(r.statut, StatusCode::CONFLICT);
    assert_eq!(r.json()["erreur"], "aucun_changement");
    let soi = b.chemin(&format!("/membres/{}/role", b.c.administrateur));
    let r = b
        .appel("POST", &soi, &admin, &corps(&json!({"role": "auditeur"})))
        .await;
    assert_eq!(r.statut, StatusCode::CONFLICT);
    assert_eq!(r.json()["erreur"], "action_sur_soi_meme");
    for mauvais in [
        json!({"role": "root"}),
        json!({"role": "valideur", "x": 1}),
        json!({}),
    ] {
        let r = b.appel("POST", &chemin, &admin, &corps(&mauvais)).await;
        assert_eq!(r.statut, StatusCode::BAD_REQUEST, "{mauvais}");
    }
    // Audit : champ avant/après, aucune donnée personnelle.
    let audit = b.audit().await.join("\n");
    assert!(audit.contains("ModifierDroits"), "{audit}");
    assert!(audit.contains("valideur"), "{audit}");
    assert!(!audit.contains("@exemple.invalid"), "{audit}");
    b.chaine_verifiee(&admin).await;
}

#[tokio::test]
async fn une_action_sensible_exige_une_session_fraiche() {
    let b = banc().await;
    let admin = b.session(b.c.administrateur).await;
    b.horloge.avancer(Duration::minutes(6));
    let chemin = b.chemin(&format!("/membres/{}/role", b.c.collaborateur));
    let r = b
        .appel(
            "POST",
            &chemin,
            &admin,
            &corps(&json!({"role": "valideur"})),
        )
        .await;
    assert_eq!(r.statut, StatusCode::FORBIDDEN, "{}", r.texte());
    // Une lecture ne l'exige pas.
    let r = b.appel("GET", &b.chemin("/membres"), &admin, "").await;
    assert_eq!(r.statut, StatusCode::OK);
}

#[tokio::test]
async fn la_suspension_coupe_l_acces_et_se_retablit() {
    let b = banc().await;
    let admin = b.session(b.c.administrateur).await;
    let cible = b.c.collaborateur;
    let s = b.session(cible).await;
    let chemin = b.chemin(&format!("/membres/{cible}/suspension"));
    let r = b.appel("POST", &chemin, &admin, "").await;
    assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
    assert_eq!(r.json()["actif"], false);
    assert_eq!(
        b.appel("GET", "/api/moi/sessions", &s, "").await.statut,
        StatusCode::UNAUTHORIZED
    );
    // Suspendu, il n'agit plus dans le cabinet, même avec une session neuve.
    let s2 = b.session(cible).await;
    let r = b.appel("GET", &b.chemin("/dossiers"), &s2, "").await;
    assert!(
        r.statut == StatusCode::FORBIDDEN || r.statut == StatusCode::NOT_FOUND,
        "{}",
        r.statut
    );
    let r = b.appel("POST", &chemin, &admin, "").await;
    assert_eq!(r.json()["erreur"], "aucun_changement");
    let r = b.appel("DELETE", &chemin, &admin, "").await;
    assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
    assert_eq!(r.json()["actif"], true);
    let r = b.appel("GET", &b.chemin("/dossiers"), &s2, "").await;
    assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
    // Pas de suspension de soi-même.
    let soi = b.chemin(&format!("/membres/{}/suspension", b.c.administrateur));
    let r = b.appel("POST", &soi, &admin, "").await;
    assert_eq!(r.json()["erreur"], "action_sur_soi_meme");
    b.chaine_verifiee(&admin).await;
}

#[tokio::test]
async fn deux_administrateurs_ne_se_retrogradent_pas_mutuellement() {
    let b = banc().await;
    let second = b.membre("administrateur").await;
    let s1 = b.session(b.c.administrateur).await;
    let s2 = b.session(second).await;
    let c1 = b.chemin(&format!("/membres/{second}/role"));
    let c2 = b.chemin(&format!("/membres/{}/role", b.c.administrateur));
    let (corps1, corps2) = (
        corps(&json!({"role": "auditeur"})),
        corps(&json!({"role": "auditeur"})),
    );
    let (r1, r2) = tokio::join!(
        b.appel("POST", &c1, &s1, &corps1),
        b.appel("POST", &c2, &s2, &corps2)
    );
    let reussies = [&r1, &r2]
        .iter()
        .filter(|r| r.statut == StatusCode::OK)
        .count();
    assert_eq!(reussies, 1, "{} / {}", r1.texte(), r2.texte());
    let admins: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM appartenance WHERE cabinet_id = $1 AND role = 'administrateur' AND actif",
    )
    .bind(b.c.cabinet.uuid())
    .fetch_one(&b.env.admin)
    .await
    .unwrap();
    assert_eq!(admins, 1);
}

// --- Affectations et sessions d'un membre ------------------------------------------------------

#[tokio::test]
async fn l_affectation_et_la_delegation_se_gerent_et_s_auditent() {
    let b = banc().await;
    let admin = b.session(b.c.administrateur).await;
    let cible = b.c.collaborateur;
    let s = b.session(cible).await;
    let acces = b.chemin(&format!("/membres/{cible}/acces"));
    // Le collaborateur ne voit que le premier dossier ; après affectation, les deux.
    let vus = |r: &Reponse| r.json().as_array().unwrap().len();
    assert_eq!(
        vus(&b.appel("GET", &b.chemin("/dossiers"), &s, "").await),
        1
    );
    let r = b
        .appel(
            "POST",
            &acces,
            &admin,
            &corps(&json!({"dossier": b.c.autre_dossier.to_string()})),
        )
        .await;
    assert_eq!(r.statut, StatusCode::CREATED, "{}", r.texte());
    assert_eq!(
        vus(&b.appel("GET", &b.chemin("/dossiers"), &s, "").await),
        2
    );
    // Délégation : passe de faux à vrai (200), puis rien ne change (409).
    let deleg =
        corps(&json!({"dossier": b.c.autre_dossier.to_string(), "validation_deleguee": true}));
    let r = b.appel("POST", &acces, &admin, &deleg).await;
    assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
    let r = b.appel("POST", &acces, &admin, &deleg).await;
    assert_eq!(r.statut, StatusCode::CONFLICT);
    // Retrait.
    let retrait = b.chemin(&format!("/membres/{cible}/acces/{}", b.c.autre_dossier));
    let r = b.appel("DELETE", &retrait, &admin, "").await;
    assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
    assert_eq!(
        vus(&b.appel("GET", &b.chemin("/dossiers"), &s, "").await),
        1
    );
    assert_eq!(
        b.appel("DELETE", &retrait, &admin, "").await.statut,
        StatusCode::NOT_FOUND
    );
    // Dossier d'un autre cabinet ou inexistant : 404, rien d'écrit.
    let autre = creer_cabinet(&b.env.admin, "Autre").await;
    for dossier in [autre.dossier.uuid(), Uuid::new_v4()] {
        let r = b
            .appel(
                "POST",
                &acces,
                &admin,
                &corps(&json!({"dossier": dossier.to_string()})),
            )
            .await;
        assert_eq!(r.statut, StatusCode::NOT_FOUND, "{}", r.texte());
    }
    let audit = b.audit().await.join("\n");
    assert!(audit.contains("validation_deleguee"), "{audit}");
    b.chaine_verifiee(&admin).await;
}

#[tokio::test]
async fn l_administrateur_voit_et_revoque_les_sessions_d_un_membre() {
    let b = banc().await;
    let admin = b.session(b.c.administrateur).await;
    let cible = b.c.collaborateur;
    let s1 = b.session(cible).await;
    let s2 = b.session(cible).await;
    let chemin = b.chemin(&format!("/membres/{cible}/sessions"));
    let r = b.appel("GET", &chemin, &admin, "").await;
    assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
    let vues = r.json();
    assert_eq!(vues.as_array().unwrap().len(), 2);
    // Ni jeton, ni empreinte, ni adresse.
    for v in vues.as_array().unwrap() {
        let mut cles: Vec<&String> = v.as_object().unwrap().keys().collect();
        cles.sort();
        assert_eq!(
            cles,
            [
                "appareil",
                "cree_le",
                "derniere_activite",
                "id",
                "methode",
                "nature"
            ]
        );
    }
    assert!(
        !r.texte()
            .contains(&s1.0.split_once('=').unwrap().1.to_owned())
    );
    let id = vues[0]["id"].as_str().unwrap().to_owned();
    let r = b
        .appel("DELETE", &format!("{chemin}/{id}"), &admin, "")
        .await;
    assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
    let vivantes = [&s1, &s2];
    let mut ouvertes = 0;
    for s in vivantes {
        if b.appel("GET", "/api/moi/sessions", s, "").await.statut == StatusCode::OK {
            ouvertes += 1;
        }
    }
    assert_eq!(ouvertes, 1);
    // Session révoquée une fois : 404 la seconde fois.
    let r = b
        .appel("DELETE", &format!("{chemin}/{id}"), &admin, "")
        .await;
    assert_eq!(r.statut, StatusCode::NOT_FOUND);
    // Un collaborateur n'y a pas accès.
    let r = b.appel("GET", &chemin, &s1, "").await;
    assert!(r.statut == StatusCode::FORBIDDEN || r.statut == StatusCode::UNAUTHORIZED);
    let motif: String = sqlx::query_scalar("SELECT motif_revocation FROM session WHERE id = $1")
        .bind(Uuid::parse_str(&id).unwrap())
        .fetch_one(&b.env.admin)
        .await
        .unwrap();
    assert_eq!(motif, "revocation_administrateur");
    b.chaine_verifiee(&admin).await;
}

// --- Séparation des tâches ---------------------------------------------------------------------

#[tokio::test]
async fn la_separation_des_taches_se_desactive_avec_reauthentification() {
    let b = banc().await;
    let admin = b.session(b.c.administrateur).await;
    let chemin = b.chemin("/separation-taches");
    let r = b.appel("POST", &chemin, &admin, "").await;
    assert_eq!(r.json()["erreur"], "aucun_changement", "active par défaut");
    b.horloge.avancer(Duration::minutes(6));
    let r = b.appel("DELETE", &chemin, &admin, "").await;
    assert_eq!(
        r.statut,
        StatusCode::FORBIDDEN,
        "session non fraîche : {}",
        r.texte()
    );
    let frais = b.session(b.c.administrateur).await;
    let r = b.appel("DELETE", &chemin, &frais, "").await;
    assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
    assert_eq!(r.json()["separation_taches"], false);
    let r = b.appel("POST", &chemin, &frais, "").await;
    assert_eq!(r.statut, StatusCode::OK);
    assert!(b.audit().await.join("\n").contains("separation_taches"));
    b.chaine_verifiee(&frais).await;
}

// --- Invitations -------------------------------------------------------------------------------

#[tokio::test]
async fn une_invitation_se_cree_se_liste_et_se_revoque() {
    let b = banc().await;
    let admin = b.session(b.c.administrateur).await;
    let chemin = b.chemin("/invitations");
    let corps_ok = corps(
        &json!({"courriel": "  Nouvelle.Personne@Cabinet.Example ", "nom": "Awa Diop", "role": "valideur"}),
    );
    let r = b.appel("POST", &chemin, &admin, &corps_ok).await;
    assert_eq!(r.statut, StatusCode::CREATED, "{}", r.texte());
    assert_eq!(r.json()["courriel"], "nouvelle.personne@cabinet.example");
    let id = r.json()["id"].as_str().unwrap().to_owned();
    // Doublon (même courriel, casse différente) : 409.
    let r = b
        .appel(
            "POST",
            &chemin,
            &admin,
            &corps(&json!({"courriel": "NOUVELLE.personne@cabinet.example", "nom": "X", "role": "auditeur"})),
        )
        .await;
    assert_eq!(r.json()["erreur"], "invitation_en_cours");
    // Courriel d'un membre : 409.
    let membre = sqlx::query_scalar::<_, String>("SELECT courriel FROM utilisateur WHERE id = $1")
        .bind(b.c.collaborateur.uuid())
        .fetch_one(&b.env.admin)
        .await
        .unwrap();
    let r = b
        .appel(
            "POST",
            &chemin,
            &admin,
            &corps(&json!({"courriel": membre, "nom": "X", "role": "auditeur"})),
        )
        .await;
    assert_eq!(r.json()["erreur"], "deja_membre");
    let r = b.appel("GET", &chemin, &admin, "").await;
    assert_eq!(r.json().as_array().unwrap().len(), 1);
    // Révocation : l'invitation sort de la liste et son lien meurt.
    let uuid = Uuid::parse_str(&id).unwrap();
    let lien = b
        .lien(NatureLien::Invitation, uuid, b.c.administrateur)
        .await;
    let r = b
        .appel("DELETE", &format!("{chemin}/{id}"), &admin, "")
        .await;
    assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
    assert_eq!(
        b.utiliser_lien(&lien).await.statut,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        b.appel("DELETE", &format!("{chemin}/{id}"), &admin, "")
            .await
            .json()["erreur"],
        "invitation_non_en_attente"
    );
    // Ni courriel ni nom dans l'audit (chaîne définitive).
    let audit = b.audit().await.join("\n");
    assert!(
        !audit.contains("cabinet.example") && !audit.contains("Awa"),
        "{audit}"
    );
    b.chaine_verifiee(&admin).await;
}

#[tokio::test]
async fn les_entrees_hostiles_d_une_invitation_sont_refusees() {
    let b = banc().await;
    let admin = b.session(b.c.administrateur).await;
    let chemin = b.chemin("/invitations");
    for (courriel, nom) in [
        ("a@cabinet.sn\r\nBcc: pirate@exemple.sn", "Nom"),
        ("a@cabinet.sn\nBcc: pirate@exemple.sn", "Nom"),
        ("a@cabinet.sn, b@cabinet.sn", "Nom"),
        ("\"a b\"@cabinet.sn", "Nom"),
        ("a@cabinet.sn", "Nom\r\nSubject: faux"),
        ("a@cabinet.sn", "Nom\u{202e}"),
        ("a@cabinet.sn", ""),
        ("é@cabinet.sn", "Nom"),
    ] {
        let r = b
            .appel(
                "POST",
                &chemin,
                &admin,
                &corps(&json!({"courriel": courriel, "nom": nom, "role": "auditeur"})),
            )
            .await;
        assert_eq!(r.statut, StatusCode::BAD_REQUEST, "{courriel:?} {nom:?}");
    }
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM invitation WHERE cabinet_id = $1")
        .bind(b.c.cabinet.uuid())
        .fetch_one(&b.env.admin)
        .await
        .unwrap();
    assert_eq!(n, 0);
}

#[tokio::test]
async fn l_invitation_ne_revele_pas_si_un_compte_existe() {
    let b = banc().await;
    let admin = b.session(b.c.administrateur).await;
    let autre = creer_cabinet(&b.env.admin, "Autre").await;
    let existant =
        sqlx::query_scalar::<_, String>("SELECT courriel FROM utilisateur WHERE id = $1")
            .bind(autre.administrateur.uuid())
            .fetch_one(&b.env.admin)
            .await
            .unwrap();
    let chemin = b.chemin("/invitations");
    let r1 = b
        .appel(
            "POST",
            &chemin,
            &admin,
            &corps(&json!({"courriel": existant, "nom": "N", "role": "auditeur"})),
        )
        .await;
    let r2 = b
        .appel(
            "POST",
            &chemin,
            &admin,
            &corps(&json!({"courriel": "inconnu@cabinet.example", "nom": "N", "role": "auditeur"})),
        )
        .await;
    assert_eq!(r1.statut, StatusCode::CREATED);
    assert_eq!(r2.statut, StatusCode::CREATED);
    let cles =
        |r: &Reponse| -> Vec<String> { r.json().as_object().unwrap().keys().cloned().collect() };
    assert_eq!(cles(&r1), cles(&r2));
}

// --- Usage des liens ---------------------------------------------------------------------------

#[tokio::test]
async fn un_lien_d_invitation_cree_le_compte_et_ouvre_une_session_d_enrolement() {
    let b = banc().await;
    let nouveau = format!("nouveau.{}@cabinet.example", Uuid::new_v4().simple());
    let admin = b.session(b.c.administrateur).await;
    let r = b
        .appel(
            "POST",
            &b.chemin("/invitations"),
            &admin,
            &corps(&json!({"courriel": nouveau, "nom": "Nouveau", "role": "collaborateur"})),
        )
        .await;
    let id = Uuid::parse_str(r.json()["id"].as_str().unwrap()).unwrap();
    let lien = b.lien(NatureLien::Invitation, id, b.c.administrateur).await;
    // Le lien n'est jamais en clair en base.
    let n: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM lien_acces WHERE encode(jeton_hache, 'escape') = $1",
    )
    .bind(&lien)
    .fetch_one(&b.env.admin)
    .await
    .unwrap();
    assert_eq!(n, 0);
    let r = b.utiliser_lien(&lien).await;
    assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
    assert_eq!(r.json()["nature"], "enrolement");
    assert_eq!(r.json()["methode"], "lien_enrolement");
    let cookie = r.entetes["set-cookie"]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_owned();
    let csrf = r.json()["csrf"].as_str().unwrap().to_owned();
    let session: Session = (cookie, csrf);
    // Compte et appartenance créés avec le rôle invité.
    let (role, actif): (String, bool) = sqlx::query_as(
        "SELECT a.role, a.actif FROM appartenance a JOIN utilisateur u ON u.id = a.utilisateur_id \
         WHERE a.cabinet_id = $1 AND u.courriel = $2",
    )
    .bind(b.c.cabinet.uuid())
    .bind(&nouveau)
    .fetch_one(&b.env.admin)
    .await
    .unwrap();
    assert_eq!((role.as_str(), actif), ("collaborateur", true));
    // La session d'enrôlement n'ouvre aucune route du cabinet ni aucune route métier.
    for chemin in [
        b.chemin("/dossiers"),
        b.chemin("/membres"),
        "/api/moi/cabinets".to_owned(),
    ] {
        assert_eq!(
            b.appel("GET", &chemin, &session, "").await.statut,
            StatusCode::UNAUTHORIZED,
            "{chemin}"
        );
    }
    // Elle sert à enrôler une passkey (défi accordé), et à rien d'autre.
    let r = b
        .appel("POST", "/api/moi/facteurs/passkeys/defi", &session, "")
        .await;
    assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
    let r = b
        .appel(
            "POST",
            "/api/moi/facteurs/secours/preparation",
            &session,
            "",
        )
        .await;
    assert_ne!(
        r.statut,
        StatusCode::OK,
        "le secours ne s'enrôle pas par un lien"
    );
    // Lien à usage unique : le second usage est refusé comme un lien inconnu.
    let second = b.utiliser_lien(&lien).await;
    let inconnu = b.utiliser_lien(&"A".repeat(43)).await;
    assert_eq!(second.statut, StatusCode::UNAUTHORIZED);
    assert_eq!(second.corps, inconnu.corps);
    // Invitation acceptée ; l'usage est dans l'audit du cabinet, chaîne intacte.
    let etat: String = sqlx::query_scalar("SELECT etat FROM invitation WHERE id = $1")
        .bind(id)
        .fetch_one(&b.env.admin)
        .await
        .unwrap();
    assert_eq!(etat, "acceptee");
    let audit = b.audit().await.join("\n");
    assert!(audit.contains("UtiliserLienInvitation"), "{audit}");
    assert!(audit.contains("OuvrirSession"), "{audit}");
    assert!(!audit.contains(&lien));
    b.chaine_verifiee(&admin).await;
}

#[tokio::test]
async fn un_lien_expire_ou_mal_forme_est_refuse_de_la_meme_facon() {
    let b = banc().await;
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO invitation (cabinet_id, id, courriel, nom, role, cree_le, etat) \
         VALUES ($1, $2, 'x@cabinet.example', 'X', 'auditeur', now(), 'en_attente')",
    )
    .bind(b.c.cabinet.uuid())
    .bind(id)
    .execute(&b.env.admin)
    .await
    .unwrap();
    let lien = b.lien(NatureLien::Invitation, id, b.c.administrateur).await;
    b.horloge.avancer(Duration::hours(25));
    let refus = b.utiliser_lien(&lien).await;
    assert_eq!(refus.statut, StatusCode::UNAUTHORIZED);
    for faux in [
        "",
        "court",
        &"!".repeat(43),
        &"é".repeat(43),
        &"A".repeat(44),
    ] {
        let r = b.utiliser_lien(faux).await;
        assert_eq!(r.statut, StatusCode::UNAUTHORIZED, "{faux:?}");
        assert_eq!(r.corps, refus.corps);
    }
    // Rien n'a été créé.
    let n: i64 =
        sqlx::query_scalar("SELECT count(*) FROM utilisateur WHERE courriel = 'x@cabinet.example'")
            .fetch_one(&b.env.admin)
            .await
            .unwrap();
    assert_eq!(n, 0);
}

#[tokio::test]
async fn un_utilisateur_existant_accepte_l_invitation_connecte() {
    let b = banc().await;
    let admin = b.session(b.c.administrateur).await;
    let autre = creer_cabinet(&b.env.admin, "Autre").await;
    let existant = autre.collaborateur;
    let courriel: String = sqlx::query_scalar("SELECT courriel FROM utilisateur WHERE id = $1")
        .bind(existant.uuid())
        .fetch_one(&b.env.admin)
        .await
        .unwrap();
    let r = b
        .appel(
            "POST",
            &b.chemin("/invitations"),
            &admin,
            &corps(&json!({"courriel": courriel, "nom": "Existant", "role": "auditeur"})),
        )
        .await;
    assert_eq!(r.statut, StatusCode::CREATED, "{}", r.texte());
    let id = Uuid::parse_str(r.json()["id"].as_str().unwrap()).unwrap();
    let lien = b.lien(NatureLien::Invitation, id, b.c.administrateur).await;
    // Hors session : un compte existe, le lien reste vivant.
    let r = b.utiliser_lien(&lien).await;
    assert_eq!(r.statut, StatusCode::CONFLICT, "{}", r.texte());
    assert_eq!(r.json()["erreur"], "compte_existant");
    // Un autre utilisateur connecté ne peut pas l'accepter (courriel différent) : même réponse
    // qu'un lien inconnu, et le lien n'est pas consommé.
    let intrus = b.session(autre.administrateur).await;
    let corps_lien = corps(&json!({"jeton": lien}));
    let r_intrus = b
        .appel(
            "POST",
            "/api/moi/invitations/acceptation",
            &intrus,
            &corps_lien,
        )
        .await;
    assert_eq!(r_intrus.statut, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(r_intrus.json()["erreur"], "lien_invalide");
    let inconnu = b
        .appel(
            "POST",
            "/api/moi/invitations/acceptation",
            &intrus,
            &corps(&json!({"jeton": "A".repeat(43)})),
        )
        .await;
    assert_eq!(r_intrus.corps, inconnu.corps);
    // Le titulaire l'accepte.
    let s = b.session(existant).await;
    let r = b
        .appel("POST", "/api/moi/invitations/acceptation", &s, &corps_lien)
        .await;
    assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
    assert_eq!(r.json()["cabinet"], b.c.cabinet.to_string());
    assert_eq!(r.json()["role"], "auditeur");
    let r = b.appel("GET", "/api/moi/cabinets", &s, "").await;
    assert_eq!(r.json().as_array().unwrap().len(), 2, "{}", r.texte());
    // Une seconde acceptation échoue.
    let r = b
        .appel("POST", "/api/moi/invitations/acceptation", &s, &corps_lien)
        .await;
    assert_eq!(r.statut, StatusCode::UNPROCESSABLE_ENTITY);
    b.chaine_verifiee(&admin).await;
}

// --- Réinitialisation d'accès ------------------------------------------------------------------

async fn demander_reinitialisation(b: &Banc, admin: &Session, cible: IdUtilisateur) -> Reponse {
    b.appel(
        "POST",
        &b.chemin(&format!("/membres/{cible}/reinitialisation")),
        admin,
        &corps(&json!({"motif_code": "appareil_perdu", "motif": "Appel de vérification au titulaire", "verification_hors_bande": true})),
    )
    .await
}

#[tokio::test]
async fn la_reinitialisation_respecte_le_delai_et_retire_les_facteurs() {
    let b = banc().await;
    let admin = b.session(b.c.administrateur).await;
    let cible = b.c.collaborateur;
    installer_secours(&b.env.admin, cible, "cheval agrafe pile correcte").await;
    installer_codes(&b.env.admin, cible).await;
    sqlx::query(
        "INSERT INTO facteur_passkey (id, utilisateur_id, identifiant_credential, cle, nom, cree_le) \
         VALUES ($1, $2, $3, '{}', 'ancienne', now())",
    )
    .bind(Uuid::new_v4())
    .bind(cible.uuid())
    .bind(Uuid::new_v4().as_bytes().to_vec())
    .execute(&b.env.admin)
    .await
    .unwrap();
    let ancienne = b.session(cible).await;
    // Demande : 201, échéance à 24 h.
    let r = demander_reinitialisation(&b, &admin, cible).await;
    assert_eq!(r.statut, StatusCode::CREATED, "{}", r.texte());
    let id = Uuid::parse_str(r.json()["id"].as_str().unwrap()).unwrap();
    // Doublon : 409.
    assert_eq!(
        demander_reinitialisation(&b, &admin, cible).await.json()["erreur"],
        "reinitialisation_en_cours"
    );
    // Avant l'échéance, même avec un lien émis, rien ne s'ouvre ; le compte est intact.
    let trop_tot = b
        .lien(NatureLien::Reinitialisation, id, b.c.administrateur)
        .await;
    assert_eq!(
        b.utiliser_lien(&trop_tot).await.statut,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        b.appel("GET", "/api/moi/sessions", &ancienne, "")
            .await
            .statut,
        StatusCode::OK
    );
    let secours: i64 = b
        .compter(
            "SELECT count(*) FROM facteur_secours WHERE utilisateur_id = $1",
            cible.uuid(),
        )
        .await;
    assert_eq!(secours, 1);
    // Après 24 h, un lien neuf (1 h) ouvre une session d'enrôlement et retire tout.
    b.horloge
        .avancer(Duration::hours(24) + Duration::minutes(1));
    let lien = b
        .lien(NatureLien::Reinitialisation, id, b.c.administrateur)
        .await;
    let r = b.utiliser_lien(&lien).await;
    assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
    assert_eq!(r.json()["nature"], "enrolement");
    for (table, sql) in [
        (
            "facteur_secours",
            "SELECT count(*) FROM facteur_secours WHERE utilisateur_id = $1",
        ),
        (
            "code_recuperation",
            "SELECT count(*) FROM code_recuperation WHERE utilisateur_id = $1",
        ),
        (
            "facteur_passkey",
            "SELECT count(*) FROM facteur_passkey WHERE utilisateur_id = $1",
        ),
    ] {
        assert_eq!(b.compter(sql, cible.uuid()).await, 0, "{table}");
    }
    assert_eq!(
        b.appel("GET", "/api/moi/sessions", &ancienne, "")
            .await
            .statut,
        StatusCode::UNAUTHORIZED
    );
    let etat: String = sqlx::query_scalar("SELECT etat FROM reinitialisation WHERE id = $1")
        .bind(id)
        .fetch_one(&b.env.admin)
        .await
        .unwrap();
    assert_eq!(etat, "aboutie");
    assert_eq!(
        b.utiliser_lien(&lien).await.statut,
        StatusCode::UNAUTHORIZED
    );
    // Chaque étape est à l'audit du cabinet, avec le code du motif ; la chaîne est intacte.
    let audit = b.audit().await.join("\n");
    for attendu in [
        "ReinitialiserAcces",
        "UtiliserLienReinitialisation",
        "appareil_perdu",
    ] {
        assert!(audit.contains(attendu), "{attendu} : {audit}");
    }
    // Le texte libre du motif reste dans la table (effaçable) : jamais dans la chaîne définitive.
    assert!(!audit.contains("Appel de vérification"), "{audit}");
    assert!(!audit.contains(&lien) && !audit.contains(&trop_tot));
    b.chaine_verifiee(&b.session(b.c.administrateur).await)
        .await;
}

#[tokio::test]
async fn la_reinitialisation_est_encadree() {
    let b = banc().await;
    let admin = b.session(b.c.administrateur).await;
    // Jamais son propre compte.
    let r = demander_reinitialisation(&b, &admin, b.c.administrateur).await;
    assert_eq!(r.json()["erreur"], "action_sur_soi_meme");
    // Sans vérification hors bande déclarée, ou sans motif : 400.
    let chemin = b.chemin(&format!("/membres/{}/reinitialisation", b.c.collaborateur));
    for mauvais in [
        json!({"motif_code": "autre", "motif": "ok", "verification_hors_bande": false}),
        json!({"motif_code": "autre", "motif": "  ", "verification_hors_bande": true}),
        json!({"motif_code": "autre", "motif": "a\nb", "verification_hors_bande": true}),
        json!({"motif_code": "autre", "verification_hors_bande": true}),
        json!({"motif_code": "inconnu", "motif": "ok", "verification_hors_bande": true}),
        json!({"motif": "ok", "verification_hors_bande": true}),
    ] {
        let r = b.appel("POST", &chemin, &admin, &corps(&mauvais)).await;
        assert_eq!(r.statut, StatusCode::BAD_REQUEST, "{mauvais}");
    }
    // Ré-authentification exigée.
    b.horloge.avancer(Duration::minutes(6));
    let r = demander_reinitialisation(&b, &admin, b.c.collaborateur).await;
    assert_eq!(r.statut, StatusCode::FORBIDDEN, "{}", r.texte());
    // Un non-administrateur n'y a pas accès.
    let valideur = b.membre("valideur").await;
    let sv = b.session(valideur).await;
    let r = demander_reinitialisation(&b, &sv, b.c.collaborateur).await;
    assert_eq!(r.statut, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn un_autre_administrateur_peut_annuler_pendant_le_delai() {
    let b = banc().await;
    let admin = b.session(b.c.administrateur).await;
    let second = b.membre("administrateur").await;
    let s2 = b.session(second).await;
    let cible = b.c.collaborateur;
    let r = demander_reinitialisation(&b, &admin, cible).await;
    let id = Uuid::parse_str(r.json()["id"].as_str().unwrap()).unwrap();
    let liste = b
        .appel("GET", &b.chemin("/reinitialisations"), &s2, "")
        .await;
    assert_eq!(liste.json().as_array().unwrap().len(), 1);
    let lien = b
        .lien(NatureLien::Reinitialisation, id, b.c.administrateur)
        .await;
    let r = b
        .appel(
            "DELETE",
            &b.chemin(&format!("/reinitialisations/{id}")),
            &s2,
            "",
        )
        .await;
    assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
    b.horloge
        .avancer(Duration::hours(24) + Duration::minutes(1));
    let s2 = b.session(second).await;
    assert_eq!(
        b.utiliser_lien(&lien).await.statut,
        StatusCode::UNAUTHORIZED
    );
    let etat: String = sqlx::query_scalar("SELECT etat FROM reinitialisation WHERE id = $1")
        .bind(id)
        .fetch_one(&b.env.admin)
        .await
        .unwrap();
    assert_eq!(etat, "annulee");
    // Une annulation ne s'annule pas ; une nouvelle demande est de nouveau possible.
    let r = b
        .appel(
            "DELETE",
            &b.chemin(&format!("/reinitialisations/{id}")),
            &s2,
            "",
        )
        .await;
    assert_eq!(r.json()["erreur"], "reinitialisation_non_en_attente");
    let s1 = b.session(b.c.administrateur).await;
    assert_eq!(
        demander_reinitialisation(&b, &s1, cible).await.statut,
        StatusCode::CREATED
    );
}

// --- RLS des liens -----------------------------------------------------------------------------

#[tokio::test]
async fn la_rls_des_liens_ne_montre_que_le_lien_presente() {
    let b = banc().await;
    let (i1, i2) = (Uuid::new_v4(), Uuid::new_v4());
    let l1 = b.lien(NatureLien::Invitation, i1, b.c.administrateur).await;
    let _l2 = b.lien(NatureLien::Invitation, i2, b.c.administrateur).await;
    // Sans contexte : rien.
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM lien_acces")
        .fetch_one(&b.env.application)
        .await
        .unwrap();
    assert_eq!(n, 0);
    // Avec l'empreinte du premier : lui seul.
    let emp = api::base::liens::JetonLien::depuis_texte(&l1)
        .unwrap()
        .empreinte();
    let mut tx = b.env.application.begin().await.unwrap();
    sqlx::query("SELECT set_config('sigma.jeton_lien', encode($1::bytea, 'hex'), true)")
        .bind(emp.octets().as_slice())
        .execute(&mut *tx)
        .await
        .unwrap();
    let cibles: Vec<Uuid> = sqlx::query_scalar("SELECT cible FROM lien_acces")
        .fetch_all(&mut *tx)
        .await
        .unwrap();
    assert_eq!(cibles, vec![i1]);
    // Jamais de suppression, et la ligne ne se réécrit pas hors des colonnes d'état.
    assert!(
        sqlx::query("DELETE FROM lien_acces")
            .execute(&mut *tx)
            .await
            .is_err()
    );
}

// --- Commandes d'administration de l'instance --------------------------------------------------

#[tokio::test]
async fn la_commande_cree_un_cabinet_et_invite_son_premier_administrateur() {
    use api::instance::{Commande, executer};
    let b = banc().await;
    let courriel = format!("premier.{}@cabinet.example", Uuid::new_v4().simple());
    let commande = Commande::CreerCabinet {
        nom: "Cabinet créé par commande".to_owned(),
        courriel: courriel.clone(),
        nom_administrateur: "Premier Administrateur".to_owned(),
    };
    let sortie = executer(
        &b.env.application,
        &commande,
        ORIGINE,
        b.horloge.maintenant(),
        false,
    )
    .await
    .unwrap();
    let cabinet: Uuid = sortie.lignes[0]
        .strip_prefix("cabinet : ")
        .unwrap()
        .parse()
        .unwrap();
    let lien = sortie.lignes.last().unwrap();
    let jeton = lien.split_once("/lien#").unwrap().1;
    // Le lien n'est écrit qu'à l'écran : ni en base, ni à l'audit.
    let audit: Vec<(String,)> =
        sqlx::query_as("SELECT contenu FROM audit_cabinet WHERE cabinet_id = $1")
            .bind(cabinet)
            .fetch_all(&b.env.admin)
            .await
            .unwrap();
    assert_eq!(audit.len(), 1);
    assert!(audit[0].0.contains("CreerCabinet") && !audit[0].0.contains(jeton));
    assert!(!audit[0].0.contains(&courriel));
    // Le lien crée l'administrateur du cabinet.
    let r = b.utiliser_lien(jeton).await;
    assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
    let role: String = sqlx::query_scalar(
        "SELECT a.role FROM appartenance a JOIN utilisateur u ON u.id = a.utilisateur_id \
         WHERE a.cabinet_id = $1 AND u.courriel = $2",
    )
    .bind(cabinet)
    .bind(&courriel)
    .fetch_one(&b.env.admin)
    .await
    .unwrap();
    assert_eq!(role, "administrateur");
}

#[tokio::test]
async fn la_commande_de_reinitialisation_suit_le_meme_delai_et_s_audite() {
    use api::instance::{Commande, executer};
    let b = banc().await;
    let courriel: String = sqlx::query_scalar("SELECT courriel FROM utilisateur WHERE id = $1")
        .bind(b.c.administrateur.uuid())
        .fetch_one(&b.env.admin)
        .await
        .unwrap();
    let commande = Commande::ReinitialiserAcces {
        cabinet: b.c.cabinet,
        courriel: courriel.clone(),
        motif_code: "compte_compromis".to_owned(),
        motif: "Identité vérifiée par visioconférence".to_owned(),
    };
    executer(
        &b.env.application,
        &commande,
        ORIGINE,
        b.horloge.maintenant(),
        false,
    )
    .await
    .unwrap();
    let (demandeur, delai): (Option<Uuid>, bool) = sqlx::query_as(
        "SELECT demandee_par, echeance - cree_le = interval '24 hours' FROM reinitialisation \
         WHERE cabinet_id = $1 AND utilisateur_id = $2",
    )
    .bind(b.c.cabinet.uuid())
    .bind(b.c.administrateur.uuid())
    .fetch_one(&b.env.admin)
    .await
    .unwrap();
    assert_eq!(demandeur, None);
    assert!(delai);
    // Une seconde demande est refusée ; un membre inconnu aussi.
    assert!(
        executer(
            &b.env.application,
            &commande,
            ORIGINE,
            b.horloge.maintenant(),
            false,
        )
        .await
        .is_err()
    );
    let inconnu = Commande::ReinitialiserAcces {
        cabinet: b.c.cabinet,
        courriel: "inconnu@cabinet.example".to_owned(),
        motif_code: "autre".to_owned(),
        motif: "m".to_owned(),
    };
    assert!(
        executer(
            &b.env.application,
            &inconnu,
            ORIGINE,
            b.horloge.maintenant(),
            false,
        )
        .await
        .is_err()
    );
    assert!(b.audit().await.join("\n").contains("ReinitialiserAcces"));
    let _ = courriel;
}

#[tokio::test]
async fn un_membre_de_plusieurs_cabinets_ne_se_reinitialise_pas() {
    let b = banc().await;
    let admin = b.session(b.c.administrateur).await;
    // Le collaborateur de A est aussi membre d'un autre cabinet : ses facteurs et sessions sont
    // ceux du compte, l'administrateur de A ne les retire pas.
    let autre = creer_cabinet(&b.env.admin, "Autre").await;
    let mut tx = b.env.admin.begin().await.unwrap();
    commun::ajouter_membre(&mut tx, autre.cabinet, b.c.collaborateur, "auditeur").await;
    tx.commit().await.unwrap();
    let r = demander_reinitialisation(&b, &admin, b.c.collaborateur).await;
    assert_eq!(r.statut, StatusCode::CONFLICT, "{}", r.texte());
    assert_eq!(r.json()["erreur"], "membre_d_un_autre_cabinet");
    let n: i64 = b
        .compter(
            "SELECT count(*) FROM reinitialisation WHERE utilisateur_id = $1",
            b.c.collaborateur.uuid(),
        )
        .await;
    assert_eq!(n, 0);
    // Le contexte du cabinet est rétabli : l'action suivante, dans la même requête, n'a pas bougé.
    let r = b.appel("GET", &b.chemin("/membres"), &admin, "").await;
    assert_eq!(r.statut, StatusCode::OK);
    // Un membre propre au cabinet reste réinitialisable.
    let propre = b.membre("auditeur").await;
    assert_eq!(
        demander_reinitialisation(&b, &admin, propre).await.statut,
        StatusCode::CREATED
    );
    // L'administrateur de l'autre cabinet n'apprend rien par la route de son cabinet (404).
    let sa = b.session(autre.administrateur).await;
    let r = b
        .appel(
            "POST",
            &format!(
                "/api/cabinets/{}/membres/{}/reinitialisation",
                b.c.cabinet, propre
            ),
            &sa,
            "",
        )
        .await;
    assert_eq!(r.statut, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn un_compte_qui_rejoint_un_autre_cabinet_pendant_le_delai_n_est_plus_reinitialise() {
    let b = banc().await;
    let admin = b.session(b.c.administrateur).await;
    let cible = b.c.collaborateur;
    installer_secours(&b.env.admin, cible, "cheval agrafe pile correcte").await;
    let r = demander_reinitialisation(&b, &admin, cible).await;
    let id = Uuid::parse_str(r.json()["id"].as_str().unwrap()).unwrap();
    let autre = creer_cabinet(&b.env.admin, "Autre").await;
    let mut tx = b.env.admin.begin().await.unwrap();
    commun::ajouter_membre(&mut tx, autre.cabinet, cible, "auditeur").await;
    tx.commit().await.unwrap();
    b.horloge
        .avancer(Duration::hours(24) + Duration::minutes(1));
    let lien = b
        .lien(NatureLien::Reinitialisation, id, b.c.administrateur)
        .await;
    assert_eq!(
        b.utiliser_lien(&lien).await.statut,
        StatusCode::UNAUTHORIZED
    );
    let secours: i64 = b
        .compter(
            "SELECT count(*) FROM facteur_secours WHERE utilisateur_id = $1",
            cible.uuid(),
        )
        .await;
    assert_eq!(secours, 1, "facteurs intacts");
}

#[tokio::test]
async fn deux_usages_simultanes_du_meme_lien_n_en_laissent_passer_qu_un() {
    let b = banc().await;
    let admin = b.session(b.c.administrateur).await;
    let courriel = format!("simultane.{}@cabinet.example", Uuid::new_v4().simple());
    let r = b
        .appel(
            "POST",
            &b.chemin("/invitations"),
            &admin,
            &corps(&json!({"courriel": courriel, "nom": "Simultané", "role": "auditeur"})),
        )
        .await;
    let id = Uuid::parse_str(r.json()["id"].as_str().unwrap()).unwrap();
    let lien = b.lien(NatureLien::Invitation, id, b.c.administrateur).await;
    let (r1, r2, r3) = tokio::join!(
        b.utiliser_lien(&lien),
        b.utiliser_lien(&lien),
        b.utiliser_lien(&lien)
    );
    let statuts = [r1.statut, r2.statut, r3.statut];
    assert_eq!(
        statuts.iter().filter(|s| **s == StatusCode::OK).count(),
        1,
        "{statuts:?}"
    );
    assert_eq!(
        statuts
            .iter()
            .filter(|s| **s == StatusCode::UNAUTHORIZED)
            .count(),
        2,
        "{statuts:?}"
    );
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM utilisateur WHERE courriel = $1")
        .bind(&courriel)
        .fetch_one(&b.env.admin)
        .await
        .unwrap();
    assert_eq!(n, 1);
}
