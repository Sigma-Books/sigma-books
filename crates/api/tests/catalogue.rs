//! Test de catalogue (ADR 0011, « Tests ») : le schéma, lu dans les catalogues de PostgreSQL,
//! respecte les règles de l'isolation. Une table oubliée fait échouer la CI (F2.7).
//!
//! Règles vérifiées :
//! - toute table hors `tables-instance.toml` a la RLS activée et forcée, une politique permissive
//!   pour chaque commande, des expressions `USING` et `WITH CHECK` qui lisent `sigma.cabinet` ;
//! - une table d'instance existe, et ne porte ni `cabinet_id` ni `dossier_id` ;
//! - aucune vue sans `security_invoker`, aucune vue matérialisée ni table étrangère ;
//! - les rôles applicatif et de lecture d'audit ne possèdent rien (même par appartenance à un
//!   rôle), n'ont ni `CREATE` sur le schéma ou la base, ni `TEMPORARY`, ni `TRUNCATE`, `TRIGGER`,
//!   `MAINTAIN` sur une table (ces droits échappent à la RLS) ;
//! - les trois rôles existent, sans `SUPERUSER`, `BYPASSRLS`, `CREATEROLE`, `CREATEDB` ni
//!   `REPLICATION` ; aucun rôle hors super-utilisateurs n'a `BYPASSRLS`, sauf le rôle de test ;
//! - aucune colonne `real`, `double precision` (règle absolue 2), `money`, ni `timestamp` sans
//!   fuseau (instants en `timestamptz`).
//!
//! Le second test ajoute des écarts dans une base jetable et vérifie que chacun est détecté.
#![cfg(feature = "tests-postgres")]
// Fichier de test : les `expect`/`unwrap` y sont voulus.
#![allow(clippy::expect_used, clippy::unwrap_used)]

mod commun;

use std::str::FromStr;

use commun::{MIGRATEUR, env, variable};
use serde::Deserialize;
use sqlx::postgres::{PgConnectOptions, PgConnection};
use sqlx::{Connection, Executor};

/// Base jetable du test de non-régression du catalogue.
const BASE_JETABLE: &str = "sigma_test_catalogue_jetable";

#[derive(Deserialize)]
struct ListeTablesInstance {
    table: Vec<TableInstance>,
}

#[derive(Deserialize)]
struct TableInstance {
    nom: String,
    justification: String,
}

/// Noms des tables d'instance, lus dans la liste versionnée.
fn tables_instance() -> Vec<String> {
    let liste: ListeTablesInstance = toml::from_str(include_str!("../tables-instance.toml"))
        .expect("tables-instance.toml illisible");
    for t in &liste.table {
        assert!(
            !t.justification.trim().is_empty(),
            "table d'instance {} sans justification",
            t.nom
        );
    }
    liste.table.into_iter().map(|t| t.nom).collect()
}

/// Schémas de l'application (hors schémas système).
macro_rules! schemas {
    () => {
        "n.nspname NOT IN ('pg_catalog', 'information_schema') \
         AND n.nspname NOT LIKE 'pg\\_toast%' AND n.nspname NOT LIKE 'pg\\_temp%'"
    };
}

/// Rôles d'exécution, qui ne doivent rien posséder.
macro_rules! roles_app {
    () => {
        "(VALUES ('sigma_application'), ('sigma_lecture_audit')) AS r (nom)"
    };
}

/// Les trois rôles de Sigma Books.
macro_rules! roles_tous {
    () => {
        "(VALUES ('sigma_migration'), ('sigma_application'), ('sigma_lecture_audit')) AS r (nom)"
    };
}

/// Requêtes de contrôle : chacune renvoie une ligne de texte par écart. `$1` : tables d'instance.
/// Textes constants, assemblés à la compilation.
const CONTROLES: &[&str] = &[
    // RLS activée et forcée.
    concat!(
        "SELECT format('%I.%I : RLS %s', n.nspname, c.relname, \
                    CASE WHEN c.relrowsecurity THEN 'non forcée' ELSE 'non activée' END) \
             FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace \
             WHERE ",
        schemas!(),
        " AND c.relkind IN ('r', 'p') \
               AND NOT (n.nspname = 'public' AND c.relname = ANY($1)) \
               AND NOT (c.relrowsecurity AND c.relforcerowsecurity)"
    ),
    // Une politique permissive pour chaque commande.
    concat!(
        "SELECT format('%I.%I : aucune politique pour %s', n.nspname, c.relname, cmd.nom) \
             FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace \
             CROSS JOIN (VALUES ('r', 'SELECT'), ('a', 'INSERT'), ('w', 'UPDATE'), ('d', 'DELETE')) \
                 AS cmd (code, nom) \
             WHERE ",
        schemas!(),
        " AND c.relkind IN ('r', 'p') \
               AND NOT (n.nspname = 'public' AND c.relname = ANY($1)) \
               AND NOT EXISTS (SELECT 1 FROM pg_policy p WHERE p.polrelid = c.oid \
                               AND p.polpermissive AND p.polcmd::text IN (cmd.code, '*')) \
             ORDER BY 1"
    ),
    // USING et WITH CHECK présents, et lisant le cabinet du contexte.
    concat!(
        "SELECT format('%I.%I : politique %I %s', n.nspname, c.relname, p.polname, defaut) \
             FROM pg_policy p JOIN pg_class c ON c.oid = p.polrelid \
             JOIN pg_namespace n ON n.oid = c.relnamespace \
             CROSS JOIN LATERAL (VALUES \
                 (p.polcmd::text IN ('*', 'r', 'w', 'd') AND p.polqual IS NULL, 'sans USING'), \
                 (p.polcmd::text IN ('*', 'a', 'w') AND p.polwithcheck IS NULL, 'sans WITH CHECK'), \
                 (p.polqual IS NOT NULL AND pg_get_expr(p.polqual, p.polrelid) \
                     NOT LIKE '%current_setting(''sigma.cabinet''::text, true)%', \
                  'dont USING ne lit pas sigma.cabinet'), \
                 (p.polwithcheck IS NOT NULL AND pg_get_expr(p.polwithcheck, p.polrelid) \
                     NOT LIKE '%current_setting(''sigma.cabinet''::text, true)%', \
                  'dont WITH CHECK ne lit pas sigma.cabinet') \
             ) AS e (ecart, defaut) \
             WHERE ",
        schemas!(),
        " AND e.ecart ORDER BY 1"
    ),
    // Tables d'instance : existantes, sans clé de tenant.
    "SELECT format('table d''instance %s absente du schéma', e) \
         FROM unnest($1::text[]) AS e \
         WHERE NOT EXISTS (SELECT 1 FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace \
                           WHERE n.nspname = 'public' AND c.relname = e AND c.relkind IN ('r', 'p'))",
    "SELECT format('table d''instance %I avec la colonne %I', c.relname, a.attname) \
         FROM pg_attribute a JOIN pg_class c ON c.oid = a.attrelid \
         JOIN pg_namespace n ON n.oid = c.relnamespace \
         WHERE n.nspname = 'public' AND c.relname = ANY($1) AND a.attnum > 0 \
           AND NOT a.attisdropped AND a.attname IN ('cabinet_id', 'dossier_id')",
    // Vues, vues matérialisées, tables étrangères.
    concat!(
        "SELECT format('%I.%I : %s', n.nspname, c.relname, \
                    CASE c.relkind WHEN 'v' THEN 'vue sans security_invoker' \
                                   WHEN 'm' THEN 'vue matérialisée (sans RLS)' \
                                   ELSE 'table étrangère' END) \
             FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace \
             WHERE ",
        schemas!(),
        " AND (c.relkind IN ('m', 'f') OR (c.relkind = 'v' \
                   AND NOT coalesce('security_invoker=true' = ANY(c.reloptions) \
                                    OR 'security_invoker=on' = ANY(c.reloptions), false)))"
    ),
    // Propriété : ni directe, ni par appartenance à un rôle propriétaire.
    concat!(
        "SELECT format('%s possède %s', r.nom, o.objet) FROM ",
        roles_app!(),
        " \
             JOIN ( \
                 SELECT format('la relation %I.%I', n.nspname, c.relname) AS objet, c.relowner AS proprio \
                 FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace WHERE ",
        schemas!(),
        " \
                 UNION ALL SELECT format('le schéma %I', n.nspname), n.nspowner \
                 FROM pg_namespace n WHERE ",
        schemas!(),
        " \
                 UNION ALL SELECT format('la fonction %I.%I', n.nspname, p.proname), p.proowner \
                 FROM pg_proc p JOIN pg_namespace n ON n.oid = p.pronamespace WHERE ",
        schemas!(),
        " \
                 UNION ALL SELECT format('le type %I.%I', n.nspname, t.typname), t.typowner \
                 FROM pg_type t JOIN pg_namespace n ON n.oid = t.typnamespace WHERE ",
        schemas!(),
        " \
                 UNION ALL SELECT format('la base %I', d.datname), d.datdba \
                 FROM pg_database d WHERE d.datname = current_database() \
             ) AS o ON pg_has_role(r.nom, o.proprio, 'MEMBER') \
             ORDER BY 1"
    ),
    // Droits sur le schéma et la base.
    concat!(
        "SELECT format('%s a CREATE sur le schéma %I', r.nom, n.nspname) \
             FROM ",
        roles_app!(),
        " CROSS JOIN pg_namespace n \
             WHERE ",
        schemas!(),
        " AND has_schema_privilege(r.nom, n.oid, 'CREATE') \
             UNION ALL \
             SELECT format('%s a %s sur la base', r.nom, d.droit) \
             FROM ",
        roles_app!(),
        " CROSS JOIN (VALUES ('CREATE'), ('TEMPORARY')) AS d (droit) \
             WHERE has_database_privilege(r.nom, current_database(), d.droit)"
    ),
    // Droits de table qui échappent à la RLS.
    concat!(
        "SELECT format('%s a %s sur %I.%I', r.nom, d.droit, n.nspname, c.relname) \
             FROM ",
        roles_app!(),
        " \
             CROSS JOIN (VALUES ('TRUNCATE'), ('TRIGGER'), ('MAINTAIN'), ('REFERENCES')) AS d (droit) \
             CROSS JOIN pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace \
             WHERE ",
        schemas!(),
        " AND c.relkind IN ('r', 'p') \
               AND has_table_privilege(r.nom, c.oid, d.droit) ORDER BY 1"
    ),
    // Attributs des rôles.
    concat!(
        "SELECT format('rôle %s absent', r.nom) FROM ",
        roles_tous!(),
        " \
             WHERE NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = r.nom) \
             UNION ALL \
             SELECT format('rôle %s : %s', g.rolname, a.attribut) \
             FROM pg_roles g CROSS JOIN LATERAL (VALUES \
                 ('SUPERUSER', g.rolsuper), ('BYPASSRLS', g.rolbypassrls), \
                 ('CREATEROLE', g.rolcreaterole), ('CREATEDB', g.rolcreatedb), \
                 ('REPLICATION', g.rolreplication)) AS a (attribut, actif) \
             WHERE g.rolname IN ('sigma_migration', 'sigma_application', 'sigma_lecture_audit') \
               AND a.actif \
             UNION ALL \
             SELECT format('rôle %s : BYPASSRLS sans être super-utilisateur', g.rolname) \
             FROM pg_roles g \
             WHERE g.rolbypassrls AND NOT g.rolsuper AND g.rolname <> 'sigma_test_contournement' \
             UNION ALL \
             SELECT format('%s est membre de sigma_migration', r.nom) FROM ",
        roles_app!(),
        " \
             WHERE pg_has_role(r.nom, 'sigma_migration', 'MEMBER')"
    ),
    // Types interdits, y compris en tableau ou en domaine.
    concat!(
        "SELECT format('%I.%I.%I : type %s interdit', n.nspname, c.relname, a.attname, \
                           format_type(a.atttypid, a.atttypmod)) \
             FROM pg_attribute a JOIN pg_class c ON c.oid = a.attrelid \
             JOIN pg_namespace n ON n.oid = c.relnamespace \
             JOIN pg_type t ON t.oid = a.atttypid \
             LEFT JOIN pg_type e ON e.oid = t.typelem AND t.typcategory = 'A' \
             WHERE ",
        schemas!(),
        " AND c.relkind IN ('r', 'p', 'v', 'm', 'f') \
               AND a.attnum > 0 AND NOT a.attisdropped \
               AND ARRAY[a.atttypid, NULLIF(t.typbasetype, 0), e.oid, NULLIF(e.typbasetype, 0)] \
                   && ARRAY['real'::regtype, 'double precision'::regtype, 'money'::regtype, \
                            'timestamp without time zone'::regtype]::oid[] \
             ORDER BY 1"
    ),
];

/// Écarts du catalogue de la base de la connexion.
async fn ecarts(connexion: &mut PgConnection, exceptions: &[String]) -> Vec<String> {
    let mut ecarts = Vec::new();
    for requete in CONTROLES {
        let mut q = sqlx::query_scalar(*requete);
        if requete.contains("$1") {
            q = q.bind(exceptions);
        }
        let lignes: Vec<String> = q
            .fetch_all(&mut *connexion)
            .await
            .unwrap_or_else(|e| panic!("contrôle de catalogue en échec ({e}) :\n{requete}"));
        ecarts.extend(lignes);
    }
    ecarts
}

#[tokio::test]
async fn le_catalogue_respecte_les_regles_d_isolation() {
    let env = env().await;
    let mut connexion = env.admin.acquire().await.unwrap();
    let ecarts = ecarts(&mut connexion, &tables_instance()).await;
    assert!(
        ecarts.is_empty(),
        "écarts du catalogue :\n{}",
        ecarts.join("\n")
    );
}

#[tokio::test]
async fn une_table_oubliee_fait_echouer_le_controle() {
    let url_admin = variable("SIGMA_TEST_ADMIN_URL");
    // Vérifie aussi l'environnement (base principale migrée).
    let env = env().await;

    // Base jetable, possédée par le rôle de migration (comme la vraie).
    let mut admin = env.admin.acquire().await.unwrap();
    admin
        .execute("DROP DATABASE IF EXISTS sigma_test_catalogue_jetable WITH (FORCE)")
        .await
        .unwrap();
    admin
        .execute("CREATE DATABASE sigma_test_catalogue_jetable OWNER sigma_migration")
        .await
        .unwrap();
    // Droits sur la base, comme crates/api/sql/approvisionnement.sql.
    admin
        .execute(
            "REVOKE ALL ON DATABASE sigma_test_catalogue_jetable FROM PUBLIC; \
             GRANT CONNECT ON DATABASE sigma_test_catalogue_jetable \
                 TO sigma_migration, sigma_application, sigma_lecture_audit",
        )
        .await
        .unwrap();

    let options = PgConnectOptions::from_str(&url_admin)
        .unwrap()
        .database(BASE_JETABLE);
    let mut jetable = PgConnection::connect_with(&options).await.unwrap();
    // Migrations exécutées sous le rôle de migration, comme `sqlx migrate run`.
    jetable.execute("SET ROLE sigma_migration").await.unwrap();
    MIGRATEUR.run(&mut jetable).await.unwrap();

    let exceptions = tables_instance();
    let initiaux = ecarts(&mut jetable, &exceptions).await;
    assert!(initiaux.is_empty(), "base neuve : {initiaux:?}");

    // Écarts introduits par le propriétaire, puis par le super-utilisateur.
    jetable
        .execute(
            "CREATE TABLE table_oubliee (id uuid PRIMARY KEY, cabinet_id uuid NOT NULL); \
             CREATE TABLE rls_non_forcee (cabinet_id uuid NOT NULL); \
             ALTER TABLE rls_non_forcee ENABLE ROW LEVEL SECURITY; \
             CREATE TABLE politique_partielle (cabinet_id uuid NOT NULL); \
             ALTER TABLE politique_partielle ENABLE ROW LEVEL SECURITY; \
             ALTER TABLE politique_partielle FORCE ROW LEVEL SECURITY; \
             CREATE POLICY lecture ON politique_partielle FOR SELECT USING (true); \
             ALTER TABLE utilisateur ADD COLUMN solde double precision; \
             ALTER TABLE utilisateur ADD COLUMN cabinet_id uuid; \
             ALTER TABLE dossier ADD COLUMN ouvert_le timestamp; \
             CREATE VIEW vue_dossiers AS SELECT * FROM dossier; \
             GRANT TRUNCATE ON dossier TO sigma_application;",
        )
        .await
        .unwrap();
    jetable.execute("RESET ROLE").await.unwrap();
    jetable
        .execute(
            "CREATE TABLE possedee (cabinet_id uuid NOT NULL); \
             ALTER TABLE possedee OWNER TO sigma_application; \
             GRANT CREATE ON SCHEMA public TO sigma_application;",
        )
        .await
        .unwrap();

    let trouves = ecarts(&mut jetable, &exceptions).await;
    let tables_instance_plus_absente: Vec<String> = exceptions
        .iter()
        .cloned()
        .chain(["table_disparue".to_owned()])
        .collect();
    let trouves_absente = ecarts(&mut jetable, &tables_instance_plus_absente).await;
    jetable.close().await.unwrap();
    admin
        .execute("DROP DATABASE sigma_test_catalogue_jetable WITH (FORCE)")
        .await
        .unwrap();

    for attendu in [
        "public.table_oubliee : RLS non activée",
        "public.table_oubliee : aucune politique pour SELECT",
        "public.rls_non_forcee : RLS non forcée",
        "public.politique_partielle : aucune politique pour INSERT",
        "public.politique_partielle : aucune politique pour DELETE",
        "public.politique_partielle : politique lecture dont USING ne lit pas sigma.cabinet",
        "public.utilisateur.solde : type double precision interdit",
        "table d'instance utilisateur avec la colonne cabinet_id",
        "public.dossier.ouvert_le : type timestamp without time zone interdit",
        "public.vue_dossiers : vue sans security_invoker",
        "sigma_application a TRUNCATE sur public.dossier",
        "sigma_application possède la relation public.possedee",
        "sigma_application a CREATE sur le schéma public",
    ] {
        assert!(
            trouves.iter().any(|e| e == attendu),
            "écart non détecté : {attendu}\nécarts trouvés :\n{}",
            trouves.join("\n")
        );
    }
    assert!(
        trouves_absente
            .iter()
            .any(|e| e == "table d'instance table_disparue absente du schéma"),
        "{trouves_absente:?}"
    );
}
