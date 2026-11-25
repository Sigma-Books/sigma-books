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

pub mod administration;
pub mod banc;
pub mod brouillard;
pub mod facteurs;
pub mod pieces;
pub mod referentiel;
pub mod serveur;

use api::base::{IdCabinet, IdDossier, IdUtilisateur};
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgPool, PgPoolOptions};
use uuid::Uuid;

/// Migrations du crate, embarquées : celles que la base de test doit avoir appliquées.
pub static MIGRATEUR: Migrator = sqlx::migrate!("./migrations");

/// Rôle de lecture d'audit (approvisionnement, ADR 0014).
pub const ROLE_LECTURE_AUDIT: &str = "sigma_lecture_audit";

/// Rôle de migration, propriétaire des tables (approvisionnement, ADR 0011).
pub const ROLE_MIGRATION: &str = "sigma_migration";

/// Rôle de test qui contourne la RLS, créé par les seuls tests (jamais par une migration).
pub const ROLE_CONTOURNEMENT: &str = "sigma_test_contournement";

const AIDE: &str = "les tests `tests-postgres` exigent un PostgreSQL approvisionné et migré \
                    (voir crates/api/README.md) ; sans base, lancez `cargo test` sans cette fonctionnalité";

/// Chemin concret d'un modèle de route : cabinet, dossier, puis chaque autre paramètre
/// (`{compte}`, `{tiers}`, `{piece}`…) donné par `sous_ressource`. Mécanisme unique des voies
/// référentiel et pièces (tests d'audit et d'isolation).
pub fn chemin_concret(
    modele: &str,
    cabinet: Uuid,
    dossier: Uuid,
    sous_ressource: impl Fn(&str) -> Uuid,
) -> String {
    let mut chemin = String::new();
    let mut reste = modele;
    while let Some(debut) = reste.find('{') {
        let fin = reste[debut..].find('}').expect("accolade fermante") + debut;
        chemin.push_str(&reste[..debut]);
        let valeur = match &reste[debut + 1..fin] {
            "cabinet" => cabinet,
            "dossier" => dossier,
            autre => sous_ressource(autre),
        };
        chemin.push_str(&valeur.to_string());
        reste = &reste[fin + 1..];
    }
    chemin.push_str(reste);
    chemin
}

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
    /// Rôle de lecture d'audit : connexions du super-utilisateur qui prennent aussitôt le rôle
    /// `sigma_lecture_audit` (`SET ROLE`), dont seuls les droits s'appliquent alors (RLS
    /// comprise). Aucun mot de passe de plus n'est exigé de l'environnement de test.
    pub lecture_audit: PgPool,
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
    let lecture_audit = pool_sous_role(ROLE_LECTURE_AUDIT).await;
    Env {
        admin,
        application,
        lecture_audit,
    }
}

/// Pool du super-utilisateur dont chaque connexion prend aussitôt `role` (`SET ROLE`).
pub async fn pool_sous_role(role: &'static str) -> PgPool {
    PgPoolOptions::new()
        .max_connections(4)
        .after_connect(move |connexion, _| {
            Box::pin(async move {
                sqlx::query(match role {
                    ROLE_LECTURE_AUDIT => "SET ROLE sigma_lecture_audit",
                    ROLE_MIGRATION => "SET ROLE sigma_migration",
                    _ => "SELECT 1 / 0",
                })
                .execute(connexion)
                .await?;
                Ok(())
            })
        })
        .connect(&variable("SIGMA_TEST_ADMIN_URL"))
        .await
        .unwrap_or_else(|e| panic!("connexion sous le rôle {role} impossible ({e}) : {AIDE}"))
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
#[derive(Clone, Copy)]
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
        creer_utilisateur(&mut tx, utilisateur, &format!("{role} de {nom}")).await;
        ajouter_membre(&mut tx, c.cabinet, utilisateur, role).await;
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
    affecter(&mut tx, c.cabinet, c.dossier, c.collaborateur).await;
    tx.commit().await.expect("commit admin");
    c
}

/// Crée un utilisateur (super-utilisateur, hors RLS).
pub async fn creer_utilisateur(tx: &mut sqlx::PgConnection, utilisateur: IdUtilisateur, nom: &str) {
    sqlx::query("INSERT INTO utilisateur (id, courriel, nom) VALUES ($1, $2, $3)")
        .bind(utilisateur.uuid())
        .bind(format!("{utilisateur}@exemple.invalid"))
        .bind(nom)
        .execute(&mut *tx)
        .await
        .expect("utilisateur");
}

/// Ajoute un utilisateur existant à un cabinet, avec un rôle (appartenance active).
pub async fn ajouter_membre(
    tx: &mut sqlx::PgConnection,
    cabinet: IdCabinet,
    utilisateur: IdUtilisateur,
    role: &str,
) {
    sqlx::query(
        "INSERT INTO appartenance (cabinet_id, utilisateur_id, role, id) VALUES ($1, $2, $3, $4)",
    )
    .bind(cabinet.uuid())
    .bind(utilisateur.uuid())
    .bind(role)
    .bind(Uuid::new_v4())
    .execute(&mut *tx)
    .await
    .expect("appartenance");
}

/// Affecte un membre du cabinet à un dossier (Q5.1), sans délégation.
pub async fn affecter(
    tx: &mut sqlx::PgConnection,
    cabinet: IdCabinet,
    dossier: IdDossier,
    utilisateur: IdUtilisateur,
) {
    sqlx::query(
        "INSERT INTO acces_dossier (cabinet_id, dossier_id, utilisateur_id, id) \
         VALUES ($1, $2, $3, $4)",
    )
    .bind(cabinet.uuid())
    .bind(dossier.uuid())
    .bind(utilisateur.uuid())
    .bind(Uuid::new_v4())
    .execute(&mut *tx)
    .await
    .expect("acces_dossier");
}

/// Ajoute au cabinet un membre de rôle `role`, affecté au premier dossier ; renvoie son
/// identifiant.
pub async fn ajouter_affecte(admin: &PgPool, c: &CabinetTest, role: &str) -> IdUtilisateur {
    let utilisateur = IdUtilisateur::nouveau();
    let mut tx = admin.begin().await.expect("transaction admin");
    creer_utilisateur(&mut tx, utilisateur, &format!("{role} affecté")).await;
    ajouter_membre(&mut tx, c.cabinet, utilisateur, role).await;
    affecter(&mut tx, c.cabinet, c.dossier, utilisateur).await;
    tx.commit().await.expect("commit admin");
    utilisateur
}

/// Code SQLSTATE d'une erreur PostgreSQL.
pub fn code_sql(erreur: &sqlx::Error) -> Option<String> {
    erreur
        .as_database_error()
        .and_then(sqlx::error::DatabaseError::code)
        .map(std::borrow::Cow::into_owned)
}
