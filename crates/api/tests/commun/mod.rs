//! Environnement des tests de base : PostgreSQL réel, approvisionné et migré (crates/api/README.md).
//!
//! Variables :
//! - `SIGMA_TEST_ADMIN_URL` : super-utilisateur de la base de test. Il prépare les données des
//!   deux cabinets (il contourne la RLS), crée le rôle de test qui contourne la RLS et la base
//!   jetable du test de catalogue. Jamais utilisé par l'application ;
//! - `SIGMA_TEST_APPLICATION_URL` : rôle applicatif (`sigma_application`), même base.
//!
//! Sans ces variables, ou sans base migrée, chaque test échoue avec un message clair : il ne
//! s'ignore jamais en silence.

// Aide de test : les `expect` y sont voulus (un environnement invalide doit faire échouer le test).
#![allow(dead_code, clippy::expect_used)]

use api::base::{IdCabinet, IdDossier, IdUtilisateur};
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgPool, PgPoolOptions};

/// Migrations du crate, embarquées : celles que la base de test doit avoir appliquées.
pub static MIGRATEUR: Migrator = sqlx::migrate!("./migrations");

/// Rôle de test qui contourne la RLS, créé par les seuls tests (jamais par une migration).
pub const ROLE_CONTOURNEMENT: &str = "sigma_test_contournement";

const AIDE: &str = "les tests `tests-postgres` exigent un PostgreSQL approvisionné et migré \
                    (voir crates/api/README.md) ; sans base, lancez `cargo test` sans cette fonctionnalité";

/// Lit une variable d'environnement obligatoire.
pub fn variable(nom: &str) -> String {
    match std::env::var(nom) {
        Ok(valeur) if !valeur.is_empty() => valeur,
        _ => panic!("variable d'environnement {nom} absente ou vide : {AIDE}"),
    }
}

/// Pools des tests.
pub struct Env {
    /// Super-utilisateur : préparation des données et contrôles.
    pub admin: PgPool,
    /// Rôle applicatif.
    pub application: PgPool,
}

/// Ouvre les pools et vérifie que la base est migrée exactement comme le crate l'attend.
pub async fn env() -> Env {
    let admin = PgPoolOptions::new()
        .max_connections(4)
        .connect(&variable("SIGMA_TEST_ADMIN_URL"))
        .await
        .unwrap_or_else(|e| panic!("connexion SIGMA_TEST_ADMIN_URL impossible ({e}) : {AIDE}"));
    let application = PgPoolOptions::new()
        .max_connections(4)
        .connect(&variable("SIGMA_TEST_APPLICATION_URL"))
        .await
        .unwrap_or_else(|e| {
            panic!("connexion SIGMA_TEST_APPLICATION_URL impossible ({e}) : {AIDE}")
        });
    verifier_migrations(&admin).await;
    Env { admin, application }
}

/// La base porte toutes les migrations du crate, avec les mêmes sommes de contrôle.
pub async fn verifier_migrations(admin: &PgPool) {
    let appliquees: Vec<(i64, Vec<u8>)> = sqlx::query_as(
        "SELECT version, checksum FROM _sqlx_migrations WHERE success ORDER BY version",
    )
    .fetch_all(admin)
    .await
    .unwrap_or_else(|e| panic!("base non migrée ({e}) : lancez `sqlx migrate run` ; {AIDE}"));
    let attendues: Vec<(i64, Vec<u8>)> = MIGRATEUR
        .iter()
        .map(|m| (m.version, m.checksum.to_vec()))
        .collect();
    assert_eq!(
        appliquees, attendues,
        "migrations appliquées différentes de crates/api/migrations : lancez `sqlx migrate run` \
         sur une base neuve ; {AIDE}"
    );
}

/// Un cabinet de test complet : un administrateur, un collaborateur, deux dossiers et
/// l'affectation du collaborateur au premier dossier. Créé par le super-utilisateur.
pub struct CabinetTest {
    pub cabinet: IdCabinet,
    pub administrateur: IdUtilisateur,
    pub collaborateur: IdUtilisateur,
    pub dossier: IdDossier,
    pub autre_dossier: IdDossier,
}

/// Crée un cabinet de test (identifiants neufs : les tests parallèles ne se gênent pas).
pub async fn creer_cabinet(admin: &PgPool, nom: &str) -> CabinetTest {
    let c = CabinetTest {
        cabinet: IdCabinet::nouveau(),
        administrateur: IdUtilisateur::nouveau(),
        collaborateur: IdUtilisateur::nouveau(),
        dossier: IdDossier::nouveau(),
        autre_dossier: IdDossier::nouveau(),
    };
    let mut tx = admin.begin().await.expect("transaction admin");
    sqlx::query("INSERT INTO cabinet (id, nom) VALUES ($1, $2)")
        .bind(c.cabinet.uuid())
        .bind(nom)
        .execute(&mut *tx)
        .await
        .expect("cabinet");
    for (utilisateur, role) in [
        (c.administrateur, "administrateur"),
        (c.collaborateur, "collaborateur"),
    ] {
        sqlx::query("INSERT INTO utilisateur (id, courriel, nom) VALUES ($1, $2, $3)")
            .bind(utilisateur.uuid())
            .bind(format!("{utilisateur}@exemple.invalid"))
            .bind(format!("{role} de {nom}"))
            .execute(&mut *tx)
            .await
            .expect("utilisateur");
        sqlx::query(
            "INSERT INTO appartenance (cabinet_id, utilisateur_id, role) VALUES ($1, $2, $3)",
        )
        .bind(c.cabinet.uuid())
        .bind(utilisateur.uuid())
        .bind(role)
        .execute(&mut *tx)
        .await
        .expect("appartenance");
    }
    for (dossier, suffixe) in [(c.dossier, "1"), (c.autre_dossier, "2")] {
        sqlx::query("INSERT INTO dossier (cabinet_id, id, nom) VALUES ($1, $2, $3)")
            .bind(c.cabinet.uuid())
            .bind(dossier.uuid())
            .bind(format!("{nom} dossier {suffixe}"))
            .execute(&mut *tx)
            .await
            .expect("dossier");
    }
    sqlx::query(
        "INSERT INTO acces_dossier (cabinet_id, dossier_id, utilisateur_id) VALUES ($1, $2, $3)",
    )
    .bind(c.cabinet.uuid())
    .bind(c.dossier.uuid())
    .bind(c.collaborateur.uuid())
    .execute(&mut *tx)
    .await
    .expect("acces_dossier");
    tx.commit().await.expect("commit admin");
    c
}

/// Code SQLSTATE d'une erreur PostgreSQL.
pub fn code_sql(erreur: &sqlx::Error) -> Option<String> {
    erreur
        .as_database_error()
        .and_then(sqlx::error::DatabaseError::code)
        .map(std::borrow::Cow::into_owned)
}
