//! Isolation entre tenants, niveau RLS (ADR 0011, « Tests ») : rôle applicatif, contexte du
//! cabinet A, requêtes SQL brutes visant les lignes du cabinet B. Aucune n'aboutit.
#![cfg(feature = "tests-postgres")]
// Fichier de test : les `expect`/`unwrap` y sont voulus.
#![allow(clippy::expect_used, clippy::unwrap_used)]

mod commun;

use api::base::{
    ContexteTenant, ErreurBase, IdUtilisateur, ReglagesPool, TransactionTenant,
    ouvrir_pool_application, ouvrir_transaction, verifier_role_application,
};
use commun::{CabinetTest, code_sql, creer_cabinet, env, variable};
use sqlx::postgres::{PgPool, PgPoolOptions};
use uuid::Uuid;

/// SQLSTATE `insufficient_privilege` : droit refusé, ou ligne refusée par `WITH CHECK`.
const REFUS: &str = "42501";

async fn transaction_de(pool: &PgPool, c: &CabinetTest) -> TransactionTenant {
    ouvrir_transaction(pool, ContexteTenant::cabinet(c.cabinet, c.administrateur))
        .await
        .expect("transaction au contexte du cabinet")
}

/// Nombre de lignes de B visibles, table par table.
async fn lignes_visibles_de(tx: &mut TransactionTenant, b: &CabinetTest) -> [i64; 4] {
    let id = b.cabinet.uuid();
    let mut n = [0; 4];
    for (i, requete) in [
        "SELECT count(*) FROM cabinet WHERE id = $1",
        "SELECT count(*) FROM appartenance WHERE cabinet_id = $1",
        "SELECT count(*) FROM dossier WHERE cabinet_id = $1",
        "SELECT count(*) FROM acces_dossier WHERE cabinet_id = $1",
    ]
    .into_iter()
    .enumerate()
    {
        n[i] = sqlx::query_scalar(requete)
            .bind(id)
            .fetch_one(tx.connexion())
            .await
            .unwrap();
    }
    n
}

#[tokio::test]
async fn select_ne_voit_que_le_cabinet_du_contexte() {
    let env = env().await;
    let a = creer_cabinet(&env.admin, "A").await;
    let b = creer_cabinet(&env.admin, "B").await;

    let mut tx = transaction_de(&env.application, &a).await;
    assert_eq!(lignes_visibles_de(&mut tx, &b).await, [0; 4]);
    // Sans filtre : seules les lignes de A remontent.
    let cabinets: Vec<Uuid> = sqlx::query_scalar("SELECT id FROM cabinet")
        .fetch_all(tx.connexion())
        .await
        .unwrap();
    assert_eq!(cabinets, vec![a.cabinet.uuid()]);
    let dossiers: Vec<Uuid> = sqlx::query_scalar("SELECT cabinet_id FROM dossier")
        .fetch_all(tx.connexion())
        .await
        .unwrap();
    assert_eq!(dossiers, vec![a.cabinet.uuid(); 2]);
    // Les lignes de A sont bien visibles : le test n'est pas vide de sens.
    assert_eq!(lignes_visibles_de(&mut tx, &a).await, [1, 2, 2, 1]);
}

#[tokio::test]
async fn insert_dans_un_autre_cabinet_est_refuse() {
    let env = env().await;
    let a = creer_cabinet(&env.admin, "A").await;
    let b = creer_cabinet(&env.admin, "B").await;
    let nouveau = Uuid::new_v4();

    for (requete, valeurs) in [
        (
            "INSERT INTO cabinet (id, nom) VALUES ($1, 'intrus')",
            vec![nouveau],
        ),
        (
            "INSERT INTO dossier (cabinet_id, id, nom) VALUES ($1, $2, 'intrus')",
            vec![b.cabinet.uuid(), nouveau],
        ),
        (
            "INSERT INTO appartenance (cabinet_id, utilisateur_id, role) \
             VALUES ($1, $2, 'administrateur')",
            vec![b.cabinet.uuid(), a.administrateur.uuid()],
        ),
        (
            "INSERT INTO acces_dossier (cabinet_id, dossier_id, utilisateur_id) VALUES ($1, $2, $3)",
            vec![b.cabinet.uuid(), b.dossier.uuid(), b.administrateur.uuid()],
        ),
    ] {
        let mut tx = transaction_de(&env.application, &a).await;
        let mut q = sqlx::query(requete);
        for v in valeurs {
            q = q.bind(v);
        }
        let erreur = q.execute(tx.connexion()).await.expect_err(requete);
        assert_eq!(
            code_sql(&erreur).as_deref(),
            Some(REFUS),
            "{requete} : {erreur}"
        );
    }
}

#[tokio::test]
async fn cle_composite_interdit_de_citer_un_dossier_d_un_autre_cabinet() {
    let env = env().await;
    let a = creer_cabinet(&env.admin, "A").await;
    let b = creer_cabinet(&env.admin, "B").await;

    // Ligne au nom de A (WITH CHECK satisfait) qui cite le dossier de B : la clé
    // étrangère (cabinet_id, dossier_id) n'existe pas.
    let mut tx = transaction_de(&env.application, &a).await;
    let erreur = sqlx::query(
        "INSERT INTO acces_dossier (cabinet_id, dossier_id, utilisateur_id) VALUES ($1, $2, $3)",
    )
    .bind(a.cabinet.uuid())
    .bind(b.dossier.uuid())
    .bind(a.administrateur.uuid())
    .execute(tx.connexion())
    .await
    .expect_err("dossier d'un autre cabinet");
    assert_eq!(code_sql(&erreur).as_deref(), Some("23503"), "{erreur}");

    // Même chose avec un utilisateur qui n'est pas membre du cabinet A.
    let mut tx = transaction_de(&env.application, &a).await;
    let erreur = sqlx::query(
        "INSERT INTO acces_dossier (cabinet_id, dossier_id, utilisateur_id) VALUES ($1, $2, $3)",
    )
    .bind(a.cabinet.uuid())
    .bind(a.dossier.uuid())
    .bind(b.collaborateur.uuid())
    .execute(tx.connexion())
    .await
    .expect_err("membre d'un autre cabinet");
    assert_eq!(code_sql(&erreur).as_deref(), Some("23503"), "{erreur}");
}

#[tokio::test]
async fn update_et_delete_sur_un_autre_cabinet_n_affectent_rien() {
    let env = env().await;
    let a = creer_cabinet(&env.admin, "A").await;
    let b = creer_cabinet(&env.admin, "B").await;

    let mut tx = transaction_de(&env.application, &a).await;
    for requete in [
        "UPDATE cabinet SET nom = 'piraté', separation_taches = false WHERE id = $1",
        "UPDATE dossier SET nom = 'piraté' WHERE cabinet_id = $1",
        "UPDATE appartenance SET role = 'administrateur' WHERE cabinet_id = $1",
        "UPDATE acces_dossier SET validation_deleguee = true WHERE cabinet_id = $1",
        "DELETE FROM appartenance WHERE cabinet_id = $1",
        "DELETE FROM acces_dossier WHERE cabinet_id = $1",
        // Sans clause : seules les lignes de A sont touchées (vérifié plus bas sur B).
        "UPDATE dossier SET nom = nom WHERE $1::uuid IS NOT NULL",
    ] {
        let resultat = sqlx::query(requete)
            .bind(b.cabinet.uuid())
            .execute(tx.connexion())
            .await
            .expect(requete);
        if requete.contains("$1::uuid IS NOT NULL") {
            assert_eq!(resultat.rows_affected(), 2, "{requete}");
        } else {
            assert_eq!(resultat.rows_affected(), 0, "{requete}");
        }
    }
    tx.confirmer().await.unwrap();

    // Le rôle applicatif n'a ni DELETE sur cabinet et dossier, ni UPDATE de leurs clés.
    // WITH CHECK sur UPDATE (« ligne de A déplacée vers B ») : aucune colonne de tenant
    // (`id`, `cabinet_id`, `dossier_id`, `utilisateur_id`) n'est modifiable aujourd'hui, l'UPDATE
    // est refusé dès le droit (ci-dessous). Le cas sera testé dès qu'une telle colonne le deviendra.
    for requete in [
        "DELETE FROM cabinet WHERE id = $1",
        "DELETE FROM dossier WHERE cabinet_id = $1",
        "UPDATE dossier SET cabinet_id = $1",
        "UPDATE cabinet SET id = $1",
        "TRUNCATE dossier",
        "UPDATE utilisateur SET nom = 'piraté' WHERE id = $1",
        "DELETE FROM utilisateur WHERE id = $1",
    ] {
        let mut tx = transaction_de(&env.application, &a).await;
        let mut q = sqlx::query(requete);
        if requete.contains("$1") {
            q = q.bind(b.cabinet.uuid());
        }
        let erreur = q.execute(tx.connexion()).await.expect_err(requete);
        assert_eq!(
            code_sql(&erreur).as_deref(),
            Some(REFUS),
            "{requete} : {erreur}"
        );
    }

    // Les lignes de B sont intactes.
    let (nom, separation): (String, bool) =
        sqlx::query_as("SELECT nom, separation_taches FROM cabinet WHERE id = $1")
            .bind(b.cabinet.uuid())
            .fetch_one(&env.admin)
            .await
            .unwrap();
    assert_eq!((nom.as_str(), separation), ("B", true));
    let comptes: (i64, i64, i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM appartenance WHERE cabinet_id = $1 AND role <> 'administrateur'), \
                (SELECT count(*) FROM acces_dossier WHERE cabinet_id = $1 AND NOT validation_deleguee), \
                (SELECT count(*) FROM dossier WHERE cabinet_id = $1 AND nom LIKE 'B dossier %'), \
                (SELECT count(*) FROM appartenance WHERE cabinet_id = $1)",
    )
    .bind(b.cabinet.uuid())
    .fetch_one(&env.admin)
    .await
    .unwrap();
    assert_eq!(comptes, (1, 1, 2, 2));
}

#[tokio::test]
async fn sans_contexte_aucune_ligne() {
    let env = env().await;
    creer_cabinet(&env.admin, "A").await;

    // Transaction ouverte sans `ouvrir_transaction`, et requête hors transaction.
    let mut tx = env.application.begin().await.unwrap();
    for (table, requete) in [
        ("cabinet", "SELECT count(*) FROM cabinet"),
        ("appartenance", "SELECT count(*) FROM appartenance"),
        ("dossier", "SELECT count(*) FROM dossier"),
        ("acces_dossier", "SELECT count(*) FROM acces_dossier"),
        ("utilisateur", "SELECT count(*) FROM utilisateur"),
    ] {
        let n: i64 = sqlx::query_scalar(requete)
            .fetch_one(&mut *tx)
            .await
            .unwrap();
        assert_eq!(n, 0, "{table}");
        let n: i64 = sqlx::query_scalar(requete)
            .fetch_one(&env.application)
            .await
            .unwrap();
        assert_eq!(n, 0, "{table} hors transaction");
    }
    // Un contexte vide (chaîne vide) ne vaut pas mieux qu'un contexte absent.
    sqlx::query("SELECT set_config('sigma.cabinet', '', true)")
        .execute(&mut *tx)
        .await
        .unwrap();
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM cabinet")
        .fetch_one(&mut *tx)
        .await
        .unwrap();
    assert_eq!(n, 0);
}

#[tokio::test]
async fn le_contexte_ne_fuit_pas_d_une_transaction_a_la_suivante() {
    let env = env().await;
    let a = creer_cabinet(&env.admin, "A").await;
    // Une seule connexion : la transaction suivante réutilise forcément la même.
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect(&variable("SIGMA_TEST_APPLICATION_URL"))
        .await
        .unwrap();
    let pid_initial: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&pool)
        .await
        .unwrap();

    for confirmer in [true, false] {
        let mut tx = transaction_de(&pool, &a).await;
        let n: i64 = sqlx::query_scalar("SELECT count(*) FROM dossier")
            .fetch_one(tx.connexion())
            .await
            .unwrap();
        assert_eq!(n, 2);
        if confirmer {
            tx.confirmer().await.unwrap();
        } else {
            tx.annuler().await.unwrap();
        }

        let mut suivante = pool.begin().await.unwrap();
        let (pid, cabinet, n): (i32, Option<String>, i64) = sqlx::query_as(
            "SELECT pg_backend_pid(), current_setting('sigma.cabinet', true), \
                    (SELECT count(*) FROM dossier)",
        )
        .fetch_one(&mut *suivante)
        .await
        .unwrap();
        assert_eq!(pid, pid_initial, "même connexion du pool");
        assert_eq!(cabinet.as_deref().unwrap_or(""), "", "contexte resté posé");
        assert_eq!(
            n, 0,
            "lignes visibles après la transaction (confirmée : {confirmer})"
        );
    }

    // Une transaction abandonnée (drop) est annulée et ne laisse rien non plus.
    {
        let _tx = transaction_de(&pool, &a).await;
    }
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM dossier")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n, 0);
}

#[tokio::test]
async fn contexte_de_dossier_restreint_au_dossier() {
    let env = env().await;
    let a = creer_cabinet(&env.admin, "A").await;
    let mut tx = ouvrir_transaction(
        &env.application,
        ContexteTenant::dossier(a.cabinet, a.autre_dossier, a.administrateur),
    )
    .await
    .unwrap();
    let dossiers: Vec<Uuid> = sqlx::query_scalar("SELECT id FROM dossier")
        .fetch_all(tx.connexion())
        .await
        .unwrap();
    assert_eq!(dossiers, vec![a.autre_dossier.uuid()]);
    // L'accès du collaborateur porte sur l'autre dossier : invisible ici.
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM acces_dossier")
        .fetch_one(tx.connexion())
        .await
        .unwrap();
    assert_eq!(n, 0);
    let resultat = sqlx::query("UPDATE dossier SET nom = 'x' WHERE id = $1")
        .bind(a.dossier.uuid())
        .execute(tx.connexion())
        .await
        .unwrap();
    assert_eq!(resultat.rows_affected(), 0);
}

#[tokio::test]
async fn le_pool_applicatif_refuse_un_role_qui_contourne_la_rls() {
    let erreur =
        ouvrir_pool_application(&variable("SIGMA_TEST_ADMIN_URL"), ReglagesPool::default())
            .await
            .expect_err("super-utilisateur accepté");
    assert!(
        matches!(erreur, ErreurBase::RoleTropPrivilegie("SUPERUSER")),
        "{erreur}"
    );
    ouvrir_pool_application(
        &variable("SIGMA_TEST_APPLICATION_URL"),
        ReglagesPool::default(),
    )
    .await
    .expect("rôle applicatif refusé");
}

#[tokio::test]
async fn utilisateur_visible_seulement_depuis_son_cabinet_ou_par_lui_meme() {
    let env = env().await;
    let a = creer_cabinet(&env.admin, "A").await;
    let b = creer_cabinet(&env.admin, "B").await;

    // Contexte du cabinet A : ses deux membres, aucun de B.
    let mut tx = transaction_de(&env.application, &a).await;
    let mut visibles: Vec<Uuid> = sqlx::query_scalar("SELECT id FROM utilisateur ORDER BY id")
        .fetch_all(tx.connexion())
        .await
        .unwrap();
    let mut attendus = vec![a.administrateur.uuid(), a.collaborateur.uuid()];
    attendus.sort();
    visibles.sort();
    assert_eq!(visibles, attendus);
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM utilisateur WHERE id = ANY($1)")
        .bind(vec![b.administrateur.uuid(), b.collaborateur.uuid()])
        .fetch_one(tx.connexion())
        .await
        .unwrap();
    assert_eq!(n, 0, "membres de B visibles depuis A");

    // Création dans le contexte d'un cabinet : admise, puis visible une fois membre.
    let nouveau = IdUtilisateur::nouveau();
    sqlx::query("INSERT INTO utilisateur (id, courriel, nom) VALUES ($1, $2, 'Invité')")
        .bind(nouveau.uuid())
        .bind(format!("{nouveau}@exemple.invalid"))
        .execute(tx.connexion())
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO appartenance (cabinet_id, utilisateur_id, role) VALUES ($1, $2, 'auditeur')",
    )
    .bind(a.cabinet.uuid())
    .bind(nouveau.uuid())
    .execute(tx.connexion())
    .await
    .unwrap();
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM utilisateur WHERE id = $1")
        .bind(nouveau.uuid())
        .fetch_one(tx.connexion())
        .await
        .unwrap();
    assert_eq!(n, 1);
    tx.annuler().await.unwrap();

    // Sans contexte : ni lecture, ni création.
    let mut tx = env.application.begin().await.unwrap();
    let erreur = sqlx::query("INSERT INTO utilisateur (id, courriel, nom) VALUES ($1, $2, 'x')")
        .bind(Uuid::new_v4())
        .bind("intrus@exemple.invalid")
        .execute(&mut *tx)
        .await
        .expect_err("création sans contexte");
    assert_eq!(code_sql(&erreur).as_deref(), Some(REFUS), "{erreur}");
}

/// Rôle de test `NOINHERIT` membre de `sigma_migration` : il n'hérite pas des droits, mais peut
/// devenir propriétaire par `SET ROLE`. Le pool doit le refuser.
#[tokio::test]
async fn le_pool_refuse_un_membre_noinherit_du_role_de_migration() {
    use sqlx::Executor;

    let env = env().await;
    env.admin
        .execute(
            "DO $$ BEGIN \
                 CREATE ROLE sigma_test_noinherit NOLOGIN NOINHERIT; \
             EXCEPTION WHEN duplicate_object OR unique_violation THEN NULL; \
             END $$",
        )
        .await
        .unwrap();
    env.admin
        .execute("GRANT sigma_migration TO sigma_test_noinherit")
        .await
        .unwrap();
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .after_connect(|connexion, _| {
            Box::pin(async move {
                connexion.execute("SET ROLE sigma_test_noinherit").await?;
                Ok(())
            })
        })
        .connect(&variable("SIGMA_TEST_ADMIN_URL"))
        .await
        .unwrap();
    let resultat = verifier_role_application(&pool).await;
    pool.close().await;
    env.admin
        .execute("DROP ROLE sigma_test_noinherit")
        .await
        .unwrap();
    assert!(
        matches!(
            resultat,
            Err(ErreurBase::RoleTropPrivilegie("membre de sigma_migration"))
        ),
        "{resultat:?}"
    );
}
