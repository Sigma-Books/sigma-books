//! Périodes et exercice suivant contre un PostgreSQL réel (L2-23, PR 11b) : transitions par les
//! routes et leur audit, refus (règles, rôles, ré-authentification, corps), irréversibilité en
//! base, et concurrence (attente au lieu d'un refus à tort, validation simulée qui retarde le
//! verrouillage, aucun interblocage).
#![cfg(feature = "tests-postgres")]
#![allow(clippy::expect_used, clippy::unwrap_used)]

mod commun;

use std::time::Duration;

use api::base::referentiel::{ErreurPeriode, Transition, changer_statut};
use api::base::{ContexteTenant, ErreurBase, IdUtilisateur, ouvrir_transaction};
use api::serveur::{Reglages, routeur};
use axum::Router;
use axum::http::StatusCode;
use commun::referentiel::doter;
use commun::serveur::{ORIGINE, Reponse, application_base, envoyer, lire};
use commun::{CabinetTest, Env, ajouter_affecte, code_sql, creer_cabinet, env, variable};
use compta_core::TexteCourt;
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

/// Cabinet de test dont le premier dossier a son référentiel (exercice civil en cours).
struct Banc {
    env: Env,
    c: CabinetTest,
    routeur: Router,
    /// Périodes du premier exercice, dans l'ordre.
    periodes: Vec<Uuid>,
}

async fn banc() -> Banc {
    let env = env().await;
    let c = creer_cabinet(&env.admin, "Cabinet").await;
    doter(&env.application, c.cabinet, c.dossier, c.administrateur).await;
    let periodes =
        sqlx::query_scalar("SELECT id FROM periode WHERE dossier_id = $1 ORDER BY debut")
            .bind(c.dossier.uuid())
            .fetch_all(&env.admin)
            .await
            .unwrap();
    let routeur = routeur(&application_base(&env, Reglages::new(ORIGINE.to_owned())));
    Banc {
        env,
        c,
        routeur,
        periodes,
    }
}

impl Banc {
    fn chemin(&self, periode: Uuid, action: &str) -> String {
        format!(
            "/api/cabinets/{}/dossiers/{}/periodes/{periode}/{action}",
            self.c.cabinet, self.c.dossier
        )
    }

    async fn poster(
        &self,
        periode: Uuid,
        action: &str,
        utilisateur: IdUtilisateur,
        corps: &str,
    ) -> Reponse {
        envoyer(
            &self.routeur,
            &self.chemin(periode, action),
            Some(utilisateur),
            corps,
            None,
        )
        .await
    }

    async fn verrouiller(&self, periode: Uuid, u: IdUtilisateur) -> Reponse {
        self.poster(periode, "verrouillage", u, "{}").await
    }

    async fn deverrouiller(&self, periode: Uuid, u: IdUtilisateur, corps: &str) -> Reponse {
        self.poster(periode, "deverrouillage", u, corps).await
    }

    async fn cloturer(&self, periode: Uuid, u: IdUtilisateur) -> Reponse {
        self.poster(periode, "cloture", u, CLOTURE).await
    }

    async fn ouvrir_suivant(&self, u: IdUtilisateur, anciennete: Option<i64>) -> Reponse {
        envoyer(
            &self.routeur,
            &format!(
                "/api/cabinets/{}/dossiers/{}/exercices/suivant",
                self.c.cabinet, self.c.dossier
            ),
            Some(u),
            "{}",
            anciennete,
        )
        .await
    }

    async fn statut(&self, periode: Uuid) -> String {
        sqlx::query_scalar("SELECT statut FROM periode WHERE id = $1")
            .bind(periode)
            .fetch_one(&self.env.admin)
            .await
            .unwrap()
    }

    async fn entrees_audit(&self) -> i64 {
        sqlx::query_scalar("SELECT count(*) FROM audit_dossier WHERE dossier_id = $1")
            .bind(self.c.dossier.uuid())
            .fetch_one(&self.env.admin)
            .await
            .unwrap()
    }

    async fn derniere_entree(&self) -> String {
        sqlx::query_scalar(
            "SELECT contenu FROM audit_dossier WHERE dossier_id = $1 ORDER BY rang DESC LIMIT 1",
        )
        .bind(self.c.dossier.uuid())
        .fetch_one(&self.env.admin)
        .await
        .unwrap()
    }

    fn contexte(&self) -> ContexteTenant {
        ContexteTenant::dossier(self.c.cabinet, self.c.dossier, self.c.administrateur)
    }
}

const CLOTURE: &str = r#"{"confirmation":"irreversible"}"#;
const MOTIF: &str = r#"{"motif":"Facture fournisseur reçue en retard"}"#;

fn ok(r: &Reponse) {
    assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
}

fn refus(r: &Reponse, statut: StatusCode) {
    assert_eq!(r.statut, statut, "{}", r.texte());
}

#[tokio::test]
async fn transitions_par_les_routes_et_leur_audit() {
    let b = banc().await;
    let (admin, p0) = (b.c.administrateur, b.periodes[0]);

    let r = b.verrouiller(p0, admin).await;
    ok(&r);
    assert_eq!(r.json()["id"], p0.to_string());
    assert_eq!(r.json()["statut"], "verrouillee");
    assert!(r.json()["echeance_cloture"].is_string());
    let entree = b.derniere_entree().await;
    assert!(
        entree.contains("periode:") && entree.contains("verrouillee"),
        "{entree}"
    );

    let r = b.deverrouiller(p0, admin, MOTIF).await;
    ok(&r);
    assert_eq!(r.json()["statut"], "ouverte");
    // Motif journalisé, dans l'audit et dans la ligne de la période.
    let entree = b.derniere_entree().await;
    assert!(
        entree.contains("Facture fournisseur reçue en retard"),
        "{entree}"
    );
    let (motif, par): (Option<String>, Option<Uuid>) =
        sqlx::query_as("SELECT motif, statut_par FROM periode WHERE id = $1")
            .bind(p0)
            .fetch_one(&b.env.admin)
            .await
            .unwrap();
    assert_eq!(
        motif.as_deref(),
        Some("Facture fournisseur reçue en retard")
    );
    assert_eq!(par, Some(admin.uuid()));

    // Clôture directe d'une période ouverte (AUDCIF art. 22, aucun verrouillage préalable).
    let r = b.cloturer(p0, admin).await;
    ok(&r);
    assert_eq!(r.json()["statut"], "cloturee");
    assert!(r.json()["echeance_cloture"].is_null());
    assert!(b.derniere_entree().await.contains("cloturee"));
    let motif: Option<String> = sqlx::query_scalar("SELECT motif FROM periode WHERE id = $1")
        .bind(p0)
        .fetch_one(&b.env.admin)
        .await
        .unwrap();
    assert_eq!(
        motif, None,
        "le motif est celui du dernier changement seulement"
    );

    // Exercice suivant : l'année civile qui suit, douze périodes, inscrit à l'audit.
    let r = b.ouvrir_suivant(admin, None).await;
    assert_eq!(r.statut, StatusCode::CREATED, "{}", r.texte());
    let annee = time::OffsetDateTime::now_utc().year() + 1;
    assert_eq!(r.json()["code"], annee.to_string());
    assert_eq!(r.json()["debut"], format!("{annee}-01-01"));
    assert_eq!(r.json()["periodes"].as_array().unwrap().len(), 12);
    let entree = b.derniere_entree().await;
    assert!(
        entree.contains("exercice:") && entree.contains(&annee.to_string()),
        "{entree}"
    );
    let liste = lire(
        &b.routeur,
        &format!(
            "/api/cabinets/{}/dossiers/{}/exercices",
            b.c.cabinet, b.c.dossier
        ),
        Some(admin),
    )
    .await;
    assert_eq!(liste.json()["exercices"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn refus_des_regles_sans_effet() {
    let b = banc().await;
    let admin = b.c.administrateur;
    let p = &b.periodes;
    let audit = b.entrees_audit().await;

    // Période antérieure non clôturée.
    refus(&b.cloturer(p[1], admin).await, StatusCode::BAD_REQUEST);
    // Dernière période : elle se clôture avec l'exercice.
    refus(&b.cloturer(p[11], admin).await, StatusCode::BAD_REQUEST);
    // Déverrouiller une période ouverte, verrouiller deux fois.
    refus(
        &b.deverrouiller(p[2], admin, MOTIF).await,
        StatusCode::BAD_REQUEST,
    );
    ok(&b.verrouiller(p[2], admin).await);
    refus(&b.verrouiller(p[2], admin).await, StatusCode::BAD_REQUEST);
    // Déverrouillage sans motif, motif blanc ou trop long, champ inconnu.
    for corps in [
        "{}",
        r#"{"motif":"   "}"#,
        &format!(r#"{{"motif":"{}"}}"#, "x".repeat(201)),
        r#"{"motif":"Erreur","autre":1}"#,
    ] {
        refus(
            &b.deverrouiller(p[2], admin, corps).await,
            StatusCode::BAD_REQUEST,
        );
    }
    // Clôture sans la confirmation exacte.
    for corps in ["{}", r#"{"confirmation":"oui"}"#] {
        refus(
            &b.poster(p[0], "cloture", admin, corps).await,
            StatusCode::BAD_REQUEST,
        );
    }
    assert_eq!(b.statut(p[2]).await, "verrouillee");
    assert_eq!(b.statut(p[0]).await, "ouverte");

    // Période déjà clôturée : ni reclôturée, ni verrouillée, ni déverrouillée.
    ok(&b.cloturer(p[0], admin).await);
    let audit_apres = b.entrees_audit().await;
    assert_eq!(
        audit_apres,
        audit + 2,
        "seuls le verrouillage et la clôture sont tracés"
    );
    refus(&b.cloturer(p[0], admin).await, StatusCode::BAD_REQUEST);
    refus(&b.verrouiller(p[0], admin).await, StatusCode::BAD_REQUEST);
    refus(
        &b.deverrouiller(p[0], admin, MOTIF).await,
        StatusCode::BAD_REQUEST,
    );
    assert_eq!(b.statut(p[0]).await, "cloturee");

    // Au plus deux exercices non clôturés.
    assert_eq!(
        b.ouvrir_suivant(admin, None).await.statut,
        StatusCode::CREATED
    );
    refus(
        &b.ouvrir_suivant(admin, None).await,
        StatusCode::BAD_REQUEST,
    );
    assert_eq!(b.entrees_audit().await, audit_apres + 1);

    // Période inexistante : absente.
    refus(
        &b.verrouiller(Uuid::new_v4(), admin).await,
        StatusCode::NOT_FOUND,
    );
}

#[tokio::test]
async fn roles_et_reauthentification() {
    let b = banc().await;
    let valideur = ajouter_affecte(&b.env.admin, &b.c, "valideur").await;
    let auditeur = ajouter_affecte(&b.env.admin, &b.c, "auditeur").await;
    let collaborateur = b.c.collaborateur;
    let p0 = b.periodes[0];
    let audit = b.entrees_audit().await;

    for u in [collaborateur, auditeur] {
        refus(&b.verrouiller(p0, u).await, StatusCode::FORBIDDEN);
        refus(&b.cloturer(p0, u).await, StatusCode::FORBIDDEN);
        refus(&b.ouvrir_suivant(u, None).await, StatusCode::FORBIDDEN);
    }
    ok(&b.verrouiller(p0, valideur).await);
    for u in [collaborateur, auditeur, valideur] {
        refus(&b.deverrouiller(p0, u, MOTIF).await, StatusCode::FORBIDDEN);
    }
    assert_eq!(b.statut(p0).await, "verrouillee");

    // Session vérifiée il y a dix minutes : non fraîche, refusée pour les actions
    // ré-authentifiées ; le verrouillage, lui, ne l'exige pas.
    let ancienne = |action: &'static str, corps: &'static str| {
        let chemin = b.chemin(p0, action);
        let routeur = b.routeur.clone();
        let admin = b.c.administrateur;
        async move { envoyer(&routeur, &chemin, Some(admin), corps, Some(600)).await }
    };
    refus(
        &ancienne("deverrouillage", MOTIF).await,
        StatusCode::FORBIDDEN,
    );
    refus(&ancienne("cloture", CLOTURE).await, StatusCode::FORBIDDEN);
    refus(
        &b.ouvrir_suivant(b.c.administrateur, Some(600)).await,
        StatusCode::FORBIDDEN,
    );
    assert_eq!(b.statut(p0).await, "verrouillee");
    assert_eq!(b.entrees_audit().await, audit + 1);
    // Le valideur clôture et ouvre l'exercice suivant.
    ok(&b.cloturer(p0, valideur).await);
    assert_eq!(
        b.ouvrir_suivant(valideur, None).await.statut,
        StatusCode::CREATED
    );
}

#[tokio::test]
async fn une_periode_cloturee_ne_se_modifie_pas_en_base() {
    // Seconde barrière : même une requête directe du rôle applicatif ne rouvre rien.
    let b = banc().await;
    ok(&b.cloturer(b.periodes[0], b.c.administrateur).await);
    let mut tx = ouvrir_transaction(&b.env.application, b.contexte())
        .await
        .unwrap();
    let ecrites = sqlx::query("UPDATE periode SET statut = 'ouverte' WHERE id = $1")
        .bind(b.periodes[0])
        .execute(tx.connexion())
        .await
        .unwrap()
        .rows_affected();
    assert_eq!(ecrites, 0);
    // Une ligne clôturée n'est pas rendue par un verrou de ligne : elle ne change plus.
    let verrouillees: Vec<Uuid> =
        sqlx::query_scalar("SELECT id FROM periode WHERE id = $1 FOR UPDATE")
            .bind(b.periodes[0])
            .fetch_all(tx.connexion())
            .await
            .unwrap();
    assert_eq!(verrouillees, Vec::<Uuid>::new());
    // Une période ouverte se modifie (le droit par colonne existe).
    let ecrites = sqlx::query("UPDATE periode SET statut = statut WHERE id = $1")
        .bind(b.periodes[1])
        .execute(tx.connexion())
        .await
        .unwrap()
        .rows_affected();
    assert_eq!(ecrites, 1);
    drop(tx);
    assert_eq!(b.statut(b.periodes[0]).await, "cloturee");
}

async fn transition(
    pool: &PgPool,
    contexte: ContexteTenant,
    periode: Uuid,
    t: Transition,
) -> Result<(), ErreurPeriode> {
    let mut tx = ouvrir_transaction(pool, contexte).await.unwrap();
    sqlx::query("SET LOCAL lock_timeout = '10s'")
        .execute(tx.connexion())
        .await
        .unwrap();
    let auteur = contexte.id_utilisateur().uuid();
    changer_statut(&mut tx, periode, t, auteur, time::OffsetDateTime::now_utc()).await?;
    tx.confirmer().await.unwrap();
    Ok(())
}

#[tokio::test]
async fn la_seconde_cloture_attend_la_premiere_au_lieu_d_etre_refusee() {
    let b = banc().await;
    let contexte = b.contexte();
    let auteur = b.c.administrateur.uuid();
    let instant = time::OffsetDateTime::now_utc();
    let mut tx1 = ouvrir_transaction(&b.env.application, contexte)
        .await
        .unwrap();
    changer_statut(
        &mut tx1,
        b.periodes[0],
        Transition::Cloturer,
        auteur,
        instant,
    )
    .await
    .unwrap();

    // Clôture de la deuxième période pendant que la première se clôture : elle attend.
    let pool = b.env.application.clone();
    let seconde = b.periodes[1];
    let tache =
        tokio::spawn(
            async move { transition(&pool, contexte, seconde, Transition::Cloturer).await },
        );
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(
        !tache.is_finished(),
        "la seconde clôture n'attend pas la première"
    );
    tx1.confirmer().await.unwrap();
    tache.await.unwrap().unwrap();
    assert_eq!(b.statut(b.periodes[1]).await, "cloturee");
}

#[tokio::test]
async fn une_validation_en_cours_retarde_le_verrouillage() {
    let b = banc().await;
    let contexte = b.contexte();
    // Validation simulée (PR 13) : la période de l'écriture tenue `FOR SHARE`.
    let mut validation = ouvrir_transaction(&b.env.application, contexte)
        .await
        .unwrap();
    let tenues: Vec<Uuid> = sqlx::query_scalar("SELECT id FROM periode WHERE id = $1 FOR SHARE")
        .bind(b.periodes[0])
        .fetch_all(validation.connexion())
        .await
        .unwrap();
    assert_eq!(tenues, vec![b.periodes[0]]);

    for t in [Transition::Verrouiller, Transition::Cloturer] {
        let mut tx = ouvrir_transaction(&b.env.application, contexte)
            .await
            .unwrap();
        sqlx::query("SET LOCAL lock_timeout = '100ms'")
            .execute(tx.connexion())
            .await
            .unwrap();
        let erreur = changer_statut(
            &mut tx,
            b.periodes[0],
            t,
            b.c.administrateur.uuid(),
            time::OffsetDateTime::now_utc(),
        )
        .await
        .unwrap_err();
        assert!(
            matches!(erreur, ErreurPeriode::Base(ErreurBase::Sqlx(ref e)) if code_sql(e).as_deref() == Some("55P03")),
            "{erreur:?}"
        );
    }
    // Une fois la validation terminée, le verrouillage passe.
    drop(validation);
    transition(
        &b.env.application,
        contexte,
        b.periodes[0],
        Transition::Verrouiller,
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn aucun_interblocage_entre_transitions_et_validations() {
    let pool = PgPoolOptions::new()
        .max_connections(16)
        .connect(&variable("SIGMA_TEST_APPLICATION_URL"))
        .await
        .unwrap();
    let motif = TexteCourt::parse("Concurrence").unwrap();
    for _ in 0..4 {
        let b = banc().await;
        let contexte = b.contexte();
        for periode in &b.periodes[6..8] {
            transition(&pool, contexte, *periode, Transition::Verrouiller)
                .await
                .unwrap();
        }
        let mut taches = Vec::new();
        // Clôtures des quatre premières périodes, verrouillages et déverrouillages des
        // suivantes, en même temps que des validations simulées sur chacune (ordre inverse).
        for (i, periode) in b.periodes.iter().copied().take(8).enumerate() {
            let t = match i {
                0..=3 => Transition::Cloturer,
                4 | 5 => Transition::Verrouiller,
                _ => Transition::Deverrouiller(motif.clone()),
            };
            let pool = pool.clone();
            taches.push(tokio::spawn(async move {
                transition(&pool, contexte, periode, t).await.err()
            }));
        }
        for periode in b.periodes.iter().copied().take(8).rev() {
            let pool = pool.clone();
            taches.push(tokio::spawn(async move {
                let mut tx = ouvrir_transaction(&pool, contexte).await.unwrap();
                sqlx::query("SELECT id FROM periode WHERE id = $1 FOR SHARE")
                    .bind(periode)
                    .fetch_all(tx.connexion())
                    .await
                    .unwrap();
                tokio::time::sleep(Duration::from_millis(20)).await;
                tx.confirmer().await.unwrap();
                None
            }));
        }
        for tache in taches {
            // Un refus des règles est admis (une clôture qui a vu la précédente encore
            // ouverte) ; toute erreur de base (interblocage 40P01, délai) fait échouer.
            match tache.await.unwrap() {
                None | Some(ErreurPeriode::Refus(_)) => {}
                Some(autre) => panic!("échec inattendu (interblocage ?) : {autre:?}"),
            }
        }
        // La première clôture aboutit toujours ; verrouillages et déverrouillages aussi ;
        // l'état relu reste atteignable (clôtures en préfixe).
        assert_eq!(b.statut(b.periodes[0]).await, "cloturee");
        for (i, attendu) in [
            (4, "verrouillee"),
            (5, "verrouillee"),
            (6, "ouverte"),
            (7, "ouverte"),
        ] {
            assert_eq!(b.statut(b.periodes[i]).await, attendu);
        }
        let mut tx = ouvrir_transaction(&pool, contexte).await.unwrap();
        assert!(
            api::base::referentiel::charger(&mut tx)
                .await
                .unwrap()
                .is_some()
        );
    }
}
