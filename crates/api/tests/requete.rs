//! Isolation entre tenants, niveau « requête » (ADR 0011, « Tests ») : les fonctions d'accès,
//! exécutées par un rôle qui **contourne la RLS**, ne renvoient rien d'un autre cabinet ni,
//! dans le contexte d'un dossier, d'un autre dossier. Le filtre de la requête suffit seul.
//!
//! Le rôle `sigma_test_contournement` (`BYPASSRLS`, `NOLOGIN`, membre de `sigma_application`) est
//! créé ici, par le super-utilisateur de test : jamais par une migration.
#![cfg(feature = "tests-postgres")]
// Fichier de test : les `expect`/`unwrap` y sont voulus.
#![allow(clippy::expect_used, clippy::unwrap_used)]

mod commun;

use api::base::organisation::{creer_dossier, lire_cabinet, lire_dossier, lister_dossiers};
use api::base::{ContexteTenant, ErreurBase, IdDossier, ouvrir_transaction};
use commun::{ROLE_CONTOURNEMENT, creer_cabinet, env, variable};
use sqlx::Executor;
use sqlx::postgres::{PgPool, PgPoolOptions};

/// Crée le rôle de test qui contourne la RLS (idempotent, sûr en parallèle).
async fn creer_role_contournement(admin: &PgPool) {
    admin
        .execute(
            "DO $$ BEGIN \
                 CREATE ROLE sigma_test_contournement NOLOGIN BYPASSRLS; \
             EXCEPTION WHEN duplicate_object OR unique_violation THEN NULL; \
             END $$",
        )
        .await
        .expect("création du rôle de contournement");
    admin
        .execute("GRANT sigma_application TO sigma_test_contournement")
        .await
        .expect("droits du rôle de contournement");
}

/// Pool du super-utilisateur, chaque connexion passée sous le rôle de contournement.
async fn pool_contournement() -> PgPool {
    PgPoolOptions::new()
        .max_connections(2)
        .after_connect(|connexion, _| {
            Box::pin(async move {
                connexion
                    .execute("SET ROLE sigma_test_contournement")
                    .await?;
                Ok(())
            })
        })
        .connect(&variable("SIGMA_TEST_ADMIN_URL"))
        .await
        .expect("pool de contournement")
}

#[tokio::test]
async fn les_fonctions_d_acces_filtrent_seules_par_tenant() {
    let env = env().await;
    creer_role_contournement(&env.admin).await;
    let a = creer_cabinet(&env.admin, "A").await;
    let b = creer_cabinet(&env.admin, "B").await;
    let pool = pool_contournement().await;

    // Le rôle contourne bien la RLS : sans le filtre de la requête, B serait visible.
    let mut tx = ouvrir_transaction(&pool, ContexteTenant::cabinet(a.cabinet, a.administrateur))
        .await
        .unwrap();
    let (role, visibles_de_b): (String, i64) = sqlx::query_as(
        "SELECT current_user::text, (SELECT count(*) FROM dossier WHERE cabinet_id = $1)",
    )
    .bind(b.cabinet.uuid())
    .fetch_one(tx.connexion())
    .await
    .unwrap();
    assert_eq!(role, ROLE_CONTOURNEMENT);
    assert_eq!(visibles_de_b, 2, "le rôle de test ne contourne pas la RLS");

    // Contexte du cabinet A.
    let cabinet = lire_cabinet(&mut tx).await.unwrap().unwrap();
    assert_eq!(cabinet.id, a.cabinet);
    assert!(
        cabinet.separation_taches,
        "séparation des tâches activée par défaut"
    );
    let dossiers = lister_dossiers(&mut tx).await.unwrap();
    assert_eq!(dossiers.len(), 2);
    assert!(dossiers.iter().all(|d| d.cabinet == a.cabinet));
    assert!(lire_dossier(&mut tx, a.dossier).await.unwrap().is_some());
    for dossier in [b.dossier, b.autre_dossier, IdDossier::nouveau()] {
        // Ressource d'autrui = ressource absente, comme un identifiant inexistant.
        assert_eq!(lire_dossier(&mut tx, dossier).await.unwrap(), None);
    }
    tx.annuler().await.unwrap();

    // Contexte du dossier 1 de A : ni le dossier 2 de A, ni ceux de B.
    let mut tx = ouvrir_transaction(
        &pool,
        ContexteTenant::dossier(a.cabinet, a.dossier, a.collaborateur),
    )
    .await
    .unwrap();
    let dossiers = lister_dossiers(&mut tx).await.unwrap();
    assert_eq!(
        dossiers.iter().map(|d| d.id).collect::<Vec<_>>(),
        vec![a.dossier]
    );
    assert_eq!(lire_dossier(&mut tx, a.autre_dossier).await.unwrap(), None);
    assert_eq!(lire_dossier(&mut tx, b.dossier).await.unwrap(), None);
    // Créer un dossier est une action du cabinet.
    assert!(matches!(
        creer_dossier(&mut tx, "refusé").await,
        Err(ErreurBase::ContexteDeDossier)
    ));
    tx.annuler().await.unwrap();

    // Un contexte qui cite le dossier de B sous le cabinet A ne livre rien non plus.
    let mut tx = ouvrir_transaction(
        &pool,
        ContexteTenant::dossier(a.cabinet, b.dossier, a.administrateur),
    )
    .await
    .unwrap();
    assert_eq!(lister_dossiers(&mut tx).await.unwrap(), Vec::new());
    assert_eq!(lire_dossier(&mut tx, b.dossier).await.unwrap(), None);
    tx.annuler().await.unwrap();
}

#[tokio::test]
async fn les_fonctions_d_acces_sous_le_role_applicatif() {
    let env = env().await;
    let a = creer_cabinet(&env.admin, "A").await;
    let b = creer_cabinet(&env.admin, "B").await;

    let mut tx = ouvrir_transaction(
        &env.application,
        ContexteTenant::cabinet(a.cabinet, a.administrateur),
    )
    .await
    .unwrap();
    let cree = creer_dossier(&mut tx, "Nouveau dossier").await.unwrap();
    assert_eq!(cree.cabinet, a.cabinet);
    assert_eq!(cree.id.uuid().get_version_num(), 4, "UUID v4");
    assert_eq!(
        lire_dossier(&mut tx, cree.id).await.unwrap(),
        Some(cree.clone())
    );
    assert_eq!(lister_dossiers(&mut tx).await.unwrap().len(), 3);
    assert_eq!(lire_dossier(&mut tx, b.dossier).await.unwrap(), None);
    tx.confirmer().await.unwrap();

    // Le dossier créé n'est pas visible depuis B.
    let mut tx = ouvrir_transaction(
        &env.application,
        ContexteTenant::cabinet(b.cabinet, b.administrateur),
    )
    .await
    .unwrap();
    assert_eq!(lire_dossier(&mut tx, cree.id).await.unwrap(), None);
    assert_eq!(lister_dossiers(&mut tx).await.unwrap().len(), 2);
}
