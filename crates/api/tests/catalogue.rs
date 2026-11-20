//! Test de catalogue (ADR 0011, « Tests ») : le schéma, lu dans les catalogues de PostgreSQL,
//! respecte les règles de l'isolation. Une table oubliée fait échouer la CI (F2.7).
//!
//! Règles vérifiées (listes d'exceptions : `crates/api/catalogue/*.toml`, un fichier par module ;
//! une table ou une fonction n'y est déclarée qu'une fois, tous fichiers confondus, sauf dans la
//! liste `ajout_seul`, qui se combine avec les autres) :
//! - toute table, hors tables d'instance sans RLS, a la RLS activée et forcée et une politique
//!   permissive pour chaque commande, avec `USING` et `WITH CHECK` selon la commande ;
//! - **forme exacte** des politiques d'une table de cabinet, telle que PostgreSQL l'affiche :
//!   `cabinet_id = NULLIF(current_setting('sigma.cabinet', true), '')::uuid` (`id` pour `cabinet`) ;
//!   pour une table qui porte `dossier_id` (et pour `dossier`, par son `id`), la même condition
//!   **et** `dossier_id = NULLIF(current_setting('sigma.dossier', true), '')::uuid` ; la variante
//!   « dossier absent ⇒ tout le cabinet » n'est admise que pour la liste `dossier_facultatif` ;
//! - lecture de ses propres cabinets sans cabinet en contexte : seules les tables de la liste
//!   `lecture_membre` admettent, pour une politique `FOR SELECT` **seulement**, la forme exacte
//!   « `sigma.cabinet` absent ET appartenance de l'utilisateur du contexte » ;
//! - tables d'instance : existantes, sans `cabinet_id` ni `dossier_id` ; sous RLS si `rls = true`,
//!   avec des politiques qui lisent le contexte ;
//! - aucune vue sans `security_invoker`, aucune vue matérialisée ni table étrangère ;
//! - les rôles applicatif et de lecture d'audit ne possèdent rien (même par appartenance), n'ont
//!   ni `CREATE` sur le schéma ou la base, ni `TEMPORARY`, ni `TRUNCATE`, `TRIGGER`, `MAINTAIN`,
//!   `REFERENCES` sur une table, ni aucun droit sur une séquence (identifiants UUID : aucune n'est
//!   nécessaire) ; ni `UPDATE` ni `DELETE` sur une table en ajout seul ;
//! - aucune fonction `SECURITY DEFINER` hors liste, aucun `EXECUTE` accordé à `PUBLIC` sur nos
//!   fonctions, aucun droit par défaut (`pg_default_acl`) ;
//! - les trois rôles existent, sans `SUPERUSER`, `BYPASSRLS`, `CREATEROLE`, `CREATEDB` ni
//!   `REPLICATION` ; hors super-utilisateurs, seul le rôle de test `sigma_test_contournement`,
//!   `NOLOGIN`, a `BYPASSRLS` ;
//! - aucune colonne `real`, `double precision` (règle absolue 2), `money`, ni `timestamp` sans
//!   fuseau (instants en `timestamptz`) ;
//! - **politiques restrictives** : seulement celles de la liste `irreversible`, `FOR UPDATE`, de
//!   `USING` exact et `WITH CHECK (true)` ; chacune de la liste existe ;
//! - **brouillard** (fil-piège, PR 11b) : les tables `brouillon%`, la liste `brouillard` et
//!   `api::base::referentiel::TABLES_BROUILLARD_LUES_A_LA_CLOTURE` sont le même ensemble : une
//!   table de brouillard ne s'ajoute pas sans être lue à la clôture d'une période.
//!
//! Le second test introduit des écarts dans une base jetable et vérifie que chacun est détecté.
#![cfg(feature = "tests-postgres")]
// Fichier de test : les `expect`/`unwrap` y sont voulus.
#![allow(clippy::expect_used, clippy::unwrap_used)]

mod commun;

use std::collections::{BTreeMap, BTreeSet};
use std::str::FromStr;

use commun::{MIGRATEUR, env, variable};
use serde::Deserialize;
use sqlx::postgres::{PgConnectOptions, PgConnection};
use sqlx::{Connection, Executor};
use tokio::sync::Mutex;

/// Les deux tests lisent les rôles du cluster ; le second en modifie un temporairement.
static VERROU_ROLES: Mutex<()> = Mutex::const_new(());

// --- Listes versionnées -------------------------------------------------------------------

/// Un fichier du catalogue (`catalogue/<module>.toml`) : une liste absente y est vide ; une clé
/// inconnue (liste mal nommée) est refusée.
#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct FichierCatalogue {
    ajout_seul: Vec<Entree>,
    security_definer: Vec<Entree>,
    instance: Vec<Instance>,
    dossier_facultatif: Vec<Entree>,
    lecture_membre: Vec<Entree>,
    irreversible: Vec<Irreversible>,
    brouillard: Vec<Entree>,
}

impl FichierCatalogue {
    /// Noms déclarés (tables et fonctions), chacun avec sa catégorie : une table n'est
    /// déclarée qu'une fois par catégorie. `ajout_seul` est une catégorie à part, qui se
    /// combine avec les autres (journal d'instance en ajout seul, ADR 0017) ; les autres listes
    /// forment une seule catégorie (une table d'instance n'est pas une table de dossier).
    fn noms(&self) -> impl Iterator<Item = (&'static str, &String)> {
        self.ajout_seul
            .iter()
            .map(|e| ("ajout_seul", &e.nom))
            .chain(
                self.security_definer
                    .iter()
                    .chain(&self.dossier_facultatif)
                    .chain(&self.lecture_membre)
                    .map(|e| ("liste", &e.nom)),
            )
            .chain(self.instance.iter().map(|i| ("liste", &i.nom)))
    }

    fn etendre(&mut self, autre: Self) {
        self.ajout_seul.extend(autre.ajout_seul);
        self.security_definer.extend(autre.security_definer);
        self.instance.extend(autre.instance);
        self.dossier_facultatif.extend(autre.dossier_facultatif);
        self.lecture_membre.extend(autre.lecture_membre);
        self.irreversible.extend(autre.irreversible);
        self.brouillard.extend(autre.brouillard);
    }
}

/// Les fichiers du catalogue, triés par nom, et leur réunion. Tout fichier du répertoire doit
/// être un `.toml` lisible.
fn lire_catalogue() -> (Vec<String>, FichierCatalogue) {
    let repertoire = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("catalogue");
    let mut chemins: Vec<_> = std::fs::read_dir(&repertoire)
        .expect("crates/api/catalogue/ illisible")
        .map(|e| e.expect("entrée du catalogue").path())
        .collect();
    chemins.sort();
    let mut noms = Vec::new();
    let mut declarees: BTreeMap<(&'static str, String), String> = BTreeMap::new();
    let mut reunion = FichierCatalogue::default();
    for chemin in chemins {
        let nom = chemin.file_name().unwrap().to_string_lossy().into_owned();
        assert!(
            chemin.extension().is_some_and(|e| e == "toml"),
            "catalogue/{nom} : seuls des fichiers .toml"
        );
        let texte = std::fs::read_to_string(&chemin).unwrap();
        let fichier: FichierCatalogue =
            toml::from_str(&texte).unwrap_or_else(|e| panic!("catalogue/{nom} illisible : {e}"));
        for (categorie, table) in fichier.noms() {
            if let Some(autre) = declarees.insert((categorie, table.clone()), nom.clone()) {
                panic!("{table} déclarée deux fois : catalogue/{autre} et catalogue/{nom}");
            }
        }
        noms.push(nom);
        reunion.etendre(fichier);
    }
    assert!(!noms.is_empty(), "catalogue vide");
    (noms, reunion)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Entree {
    nom: String,
    justification: String,
}

/// Politique restrictive d'irréversibilité admise : table et `USING` exact (`pg_get_expr`).
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Irreversible {
    nom: String,
    using: String,
    justification: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Instance {
    nom: String,
    rls: bool,
    justification: String,
}

/// Listes du catalogue (tables et fonctions du schéma `public`).
#[derive(Clone)]
struct Regles {
    instances: Vec<String>,
    instances_sans_rls: Vec<String>,
    instances_avec_rls: Vec<String>,
    dossier_facultatif: Vec<String>,
    lecture_membre: Vec<String>,
    ajout_seul: Vec<String>,
    security_definer: Vec<String>,
    /// Table et `USING` exact de chaque politique restrictive admise.
    irreversible: Vec<(String, String)>,
    /// Tables du brouillard déclarées.
    brouillard: Vec<String>,
}

fn regles() -> Regles {
    let (fichiers, f) = lire_catalogue();
    let mut noms = BTreeSet::new();
    let justifiees = f
        .ajout_seul
        .iter()
        .map(|e| ("ajout_seul", &e.nom, &e.justification))
        .chain(
            f.security_definer
                .iter()
                .chain(&f.dossier_facultatif)
                .chain(&f.lecture_membre)
                .map(|e| ("liste", &e.nom, &e.justification)),
        )
        .chain(
            f.instance
                .iter()
                .map(|e| ("liste", &e.nom, &e.justification)),
        );
    for (categorie, nom, justification) in justifiees {
        assert!(!justification.trim().is_empty(), "{nom} sans justification");
        assert!(
            noms.insert((categorie, nom.clone())),
            "{nom} figure deux fois dans le catalogue ({fichiers:?})"
        );
    }
    // Listes transverses (une table de dossier ordinaire peut y figurer) : uniques chacune.
    let mut transverses = BTreeSet::new();
    let declarees = f
        .irreversible
        .iter()
        .map(|e| ("irreversible", &e.nom, &e.justification))
        .chain(
            f.brouillard
                .iter()
                .map(|e| ("brouillard", &e.nom, &e.justification)),
        );
    for (liste, nom, justification) in declarees {
        assert!(!justification.trim().is_empty(), "{nom} sans justification");
        assert!(
            transverses.insert((liste, nom.clone())),
            "{nom} figure deux fois dans la liste {liste} ({fichiers:?})"
        );
    }
    let noms_de = |v: &[Entree]| v.iter().map(|e| e.nom.clone()).collect::<Vec<_>>();
    Regles {
        instances: f.instance.iter().map(|i| i.nom.clone()).collect(),
        instances_sans_rls: f
            .instance
            .iter()
            .filter(|i| !i.rls)
            .map(|i| i.nom.clone())
            .collect(),
        instances_avec_rls: f
            .instance
            .iter()
            .filter(|i| i.rls)
            .map(|i| i.nom.clone())
            .collect(),
        dossier_facultatif: noms_de(&f.dossier_facultatif),
        lecture_membre: noms_de(&f.lecture_membre),
        ajout_seul: noms_de(&f.ajout_seul),
        security_definer: noms_de(&f.security_definer),
        irreversible: f
            .irreversible
            .iter()
            .map(|e| (e.nom.clone(), e.using.clone()))
            .collect(),
        brouillard: noms_de(&f.brouillard),
    }
}

// --- Contrôles SQL ------------------------------------------------------------------------

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

/// Liste passée en paramètre (`$1`) d'un contrôle.
#[derive(Clone, Copy)]
enum Liste {
    Aucune,
    InstancesSansRls,
    Instances,
    AjoutSeul,
    SecurityDefiner,
}

/// Contrôles : chacun renvoie une ligne de texte par écart. Textes constants, assemblés à la
/// compilation.
const CONTROLES: &[(Liste, &str)] = &[
    // RLS activée et forcée.
    (
        Liste::InstancesSansRls,
        concat!(
            "SELECT format('%I.%I : RLS %s', n.nspname, c.relname, \
                 CASE WHEN c.relrowsecurity THEN 'non forcée' ELSE 'non activée' END) \
             FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace WHERE ",
            schemas!(),
            " AND c.relkind IN ('r', 'p') \
              AND NOT (n.nspname = 'public' AND c.relname = ANY($1)) \
              AND NOT (c.relrowsecurity AND c.relforcerowsecurity)"
        ),
    ),
    // Une politique permissive pour chaque commande.
    (
        Liste::InstancesSansRls,
        concat!(
            "SELECT format('%I.%I : aucune politique pour %s', n.nspname, c.relname, cmd.nom) \
             FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace \
             CROSS JOIN (VALUES ('r', 'SELECT'), ('a', 'INSERT'), ('w', 'UPDATE'), ('d', 'DELETE')) \
                 AS cmd (code, nom) WHERE ",
            schemas!(),
            " AND c.relkind IN ('r', 'p') \
              AND NOT (n.nspname = 'public' AND c.relname = ANY($1)) \
              AND NOT EXISTS (SELECT 1 FROM pg_policy p WHERE p.polrelid = c.oid \
                              AND p.polpermissive AND p.polcmd::text IN (cmd.code, '*')) \
             ORDER BY 1"
        ),
    ),
    // Tables d'instance : existantes, sans clé de tenant.
    (
        Liste::Instances,
        "SELECT format('table d''instance %s absente du schéma', e) \
         FROM unnest($1::text[]) AS e \
         WHERE NOT EXISTS (SELECT 1 FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace \
                           WHERE n.nspname = 'public' AND c.relname = e AND c.relkind IN ('r', 'p'))",
    ),
    (
        Liste::Instances,
        "SELECT format('table d''instance %I avec la colonne %I', c.relname, a.attname) \
         FROM pg_attribute a JOIN pg_class c ON c.oid = a.attrelid \
         JOIN pg_namespace n ON n.oid = c.relnamespace \
         WHERE n.nspname = 'public' AND c.relname = ANY($1) AND a.attnum > 0 \
           AND NOT a.attisdropped AND a.attname IN ('cabinet_id', 'dossier_id')",
    ),
    // Vues, vues matérialisées, tables étrangères.
    (
        Liste::Aucune,
        concat!(
            "SELECT format('%I.%I : %s', n.nspname, c.relname, \
                 CASE c.relkind WHEN 'v' THEN 'vue sans security_invoker' \
                                WHEN 'm' THEN 'vue matérialisée (sans RLS)' \
                                ELSE 'table étrangère' END) \
             FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace WHERE ",
            schemas!(),
            " AND (c.relkind IN ('m', 'f') OR (c.relkind = 'v' \
                   AND NOT coalesce('security_invoker=true' = ANY(c.reloptions) \
                                    OR 'security_invoker=on' = ANY(c.reloptions), false)))"
        ),
    ),
    // Propriété : ni directe, ni par appartenance à un rôle propriétaire.
    (
        Liste::Aucune,
        concat!(
            "SELECT format('%s possède %s', r.nom, o.objet) FROM ",
            roles_app!(),
            " JOIN ( \
                 SELECT format('la relation %I.%I', n.nspname, c.relname) AS objet, c.relowner AS proprio \
                 FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace WHERE ",
            schemas!(),
            " UNION ALL SELECT format('le schéma %I', n.nspname), n.nspowner \
                 FROM pg_namespace n WHERE ",
            schemas!(),
            " UNION ALL SELECT format('la fonction %I.%I', n.nspname, p.proname), p.proowner \
                 FROM pg_proc p JOIN pg_namespace n ON n.oid = p.pronamespace WHERE ",
            schemas!(),
            " UNION ALL SELECT format('le type %I.%I', n.nspname, t.typname), t.typowner \
                 FROM pg_type t JOIN pg_namespace n ON n.oid = t.typnamespace WHERE ",
            schemas!(),
            " UNION ALL SELECT format('la base %I', d.datname), d.datdba \
                 FROM pg_database d WHERE d.datname = current_database() \
             ) AS o ON pg_has_role(r.nom, o.proprio, 'MEMBER') \
             ORDER BY 1"
        ),
    ),
    // Droits sur le schéma et la base.
    (
        Liste::Aucune,
        concat!(
            "SELECT format('%s a CREATE sur le schéma %I', r.nom, n.nspname) FROM ",
            roles_app!(),
            " CROSS JOIN pg_namespace n WHERE ",
            schemas!(),
            " AND has_schema_privilege(r.nom, n.oid, 'CREATE') \
             UNION ALL \
             SELECT format('%s a %s sur la base', r.nom, d.droit) FROM ",
            roles_app!(),
            " CROSS JOIN (VALUES ('CREATE'), ('TEMPORARY')) AS d (droit) \
             WHERE has_database_privilege(r.nom, current_database(), d.droit)"
        ),
    ),
    // Droits de table qui échappent à la RLS.
    (
        Liste::Aucune,
        concat!(
            "SELECT format('%s a %s sur %I.%I', r.nom, d.droit, n.nspname, c.relname) FROM ",
            roles_app!(),
            " CROSS JOIN (VALUES ('TRUNCATE'), ('TRIGGER'), ('MAINTAIN'), ('REFERENCES')) AS d (droit) \
              CROSS JOIN pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace WHERE ",
            schemas!(),
            " AND c.relkind IN ('r', 'p') \
              AND has_table_privilege(r.nom, c.oid, d.droit) ORDER BY 1"
        ),
    ),
    // Séquences : aucun droit (identifiants UUID tirés par `api`).
    (
        Liste::Aucune,
        concat!(
            "SELECT format('%s a %s sur la séquence %I.%I', r.nom, d.droit, n.nspname, c.relname) \
             FROM ",
            roles_app!(),
            " CROSS JOIN (VALUES ('USAGE'), ('SELECT'), ('UPDATE')) AS d (droit) \
              CROSS JOIN pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace WHERE ",
            schemas!(),
            " AND c.relkind = 'S' AND has_sequence_privilege(r.nom, c.oid, d.droit) ORDER BY 1"
        ),
    ),
    // Tables en ajout seul : existantes, ni UPDATE (même sur une colonne) ni DELETE.
    (
        Liste::AjoutSeul,
        "SELECT format('table en ajout seul %s absente du schéma', e) \
         FROM unnest($1::text[]) AS e \
         WHERE NOT EXISTS (SELECT 1 FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace \
                           WHERE n.nspname = 'public' AND c.relname = e AND c.relkind IN ('r', 'p'))",
    ),
    (
        Liste::AjoutSeul,
        concat!(
            "SELECT format('%s a %s sur %I.%I (ajout seul)', r.nom, d.droit, n.nspname, c.relname) \
             FROM ",
            roles_app!(),
            " CROSS JOIN (VALUES ('UPDATE'), ('DELETE')) AS d (droit) \
              CROSS JOIN pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace \
             WHERE n.nspname = 'public' AND c.relname = ANY($1) AND c.relkind IN ('r', 'p') \
               AND (has_table_privilege(r.nom, c.oid, d.droit) \
                    OR (d.droit = 'UPDATE' AND has_any_column_privilege(r.nom, c.oid, 'UPDATE'))) \
             ORDER BY 1"
        ),
    ),
    // Fonctions SECURITY DEFINER hors liste.
    (
        Liste::SecurityDefiner,
        concat!(
            "SELECT format('fonction %I.%I : SECURITY DEFINER hors liste', n.nspname, p.proname) \
             FROM pg_proc p JOIN pg_namespace n ON n.oid = p.pronamespace WHERE ",
            schemas!(),
            " AND p.prosecdef AND NOT (n.nspname = 'public' AND p.proname = ANY($1)) ORDER BY 1"
        ),
    ),
    // EXECUTE accordé à PUBLIC (accordé par défaut à la création d'une fonction).
    (
        Liste::Aucune,
        concat!(
            "SELECT format('fonction %I.%I : EXECUTE accordé à PUBLIC', n.nspname, p.proname) \
             FROM pg_proc p JOIN pg_namespace n ON n.oid = p.pronamespace WHERE ",
            schemas!(),
            " AND EXISTS (SELECT 1 FROM aclexplode(coalesce(p.proacl, acldefault('f', p.proowner))) a \
                          WHERE a.grantee = 0 AND a.privilege_type = 'EXECUTE') ORDER BY 1"
        ),
    ),
    // Droits par défaut : toute attribution doit être explicite, table par table.
    (
        Liste::Aucune,
        "SELECT format('droits par défaut de %s (%s) : %s', pg_get_userbyid(d.defaclrole), \
                       coalesce(n.nspname, 'toute la base'), d.defaclacl::text) \
         FROM pg_default_acl d LEFT JOIN pg_namespace n ON n.oid = d.defaclnamespace ORDER BY 1",
    ),
    // Attributs des rôles.
    (
        Liste::Aucune,
        concat!(
            "SELECT format('rôle %s absent', r.nom) FROM ",
            roles_tous!(),
            " WHERE NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = r.nom) \
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
             WHERE g.rolbypassrls AND NOT g.rolsuper \
               AND NOT (g.rolname = 'sigma_test_contournement' AND NOT g.rolcanlogin) \
             UNION ALL \
             SELECT format('%s est membre de sigma_migration', r.nom) FROM ",
            roles_app!(),
            " WHERE pg_has_role(r.nom, 'sigma_migration', 'MEMBER')"
        ),
    ),
    // Types interdits, y compris en tableau ou en domaine.
    (
        Liste::Aucune,
        concat!(
            "SELECT format('%I.%I.%I : type %s interdit', n.nspname, c.relname, a.attname, \
                           format_type(a.atttypid, a.atttypmod)) \
             FROM pg_attribute a JOIN pg_class c ON c.oid = a.attrelid \
             JOIN pg_namespace n ON n.oid = c.relnamespace \
             JOIN pg_type t ON t.oid = a.atttypid \
             LEFT JOIN pg_type e ON e.oid = t.typelem AND t.typcategory = 'A' WHERE ",
            schemas!(),
            " AND c.relkind IN ('r', 'p', 'v', 'm', 'f') \
              AND a.attnum > 0 AND NOT a.attisdropped \
              AND ARRAY[a.atttypid, NULLIF(t.typbasetype, 0), e.oid, NULLIF(e.typbasetype, 0)] \
                  && ARRAY['real'::regtype, 'double precision'::regtype, 'money'::regtype, \
                           'timestamp without time zone'::regtype]::oid[] \
             ORDER BY 1"
        ),
    ),
];

// --- Forme des politiques -----------------------------------------------------------------

/// `<cle> = NULLIF(current_setting('sigma.<parametre>', true), '')::uuid`, tel qu'affiché par
/// PostgreSQL (`pg_get_expr`, `pg_policies`).
fn egalite_contexte(cle: &str, parametre: &str) -> String {
    format!("({cle} = (NULLIF(current_setting('sigma.{parametre}'::text, true), ''::text))::uuid)")
}

/// Formes admises pour les expressions des politiques d'une table de tenant.
fn formes_admises(table: &str, dossier_id: bool, regles: &Regles) -> Vec<String> {
    let cle_cabinet = if table == "cabinet" {
        "id"
    } else {
        "cabinet_id"
    };
    let cabinet = egalite_contexte(cle_cabinet, "cabinet");
    let cle_dossier = if table == "dossier" {
        Some("id")
    } else if dossier_id {
        Some("dossier_id")
    } else {
        None
    };
    let Some(cle_dossier) = cle_dossier else {
        return vec![cabinet];
    };
    let dossier = egalite_contexte(cle_dossier, "dossier");
    let mut formes = vec![format!("({cabinet} AND {dossier})")];
    if regles.dossier_facultatif.iter().any(|t| t == table) {
        formes.push(format!(
            "({cabinet} AND ((NULLIF(current_setting('sigma.dossier'::text, true), ''::text) \
             IS NULL) OR {dossier}))"
        ));
    }
    formes
}

/// Forme admise, pour une politique `FOR SELECT` d'une table de la liste `lecture_membre`, de
/// la lecture de ses propres lignes sans cabinet en contexte : `sigma.cabinet` absent **et**
/// appartenance de l'utilisateur du contexte (par `utilisateur_id`, ou, pour `cabinet`, par une
/// appartenance **active** à ce cabinet). Espaces compactés (`pg_get_expr` coupe les sous-requêtes).
fn forme_lecture_membre(table: &str) -> String {
    let sans_cabinet = "(NULLIF(current_setting('sigma.cabinet'::text, true), ''::text) IS NULL)";
    let utilisateur = "(NULLIF(current_setting('sigma.utilisateur'::text, true), ''::text))::uuid";
    if table == "cabinet" {
        format!(
            "({sans_cabinet} AND (EXISTS ( SELECT 1 FROM appartenance a \
             WHERE ((a.cabinet_id = cabinet.id) AND (a.utilisateur_id = {utilisateur}) \
             AND a.actif))))"
        )
    } else {
        format!("({sans_cabinet} AND (utilisateur_id = {utilisateur}))")
    }
}

/// Expression aux espaces compactés.
fn compacter(expression: &str) -> String {
    expression.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Politique lue au catalogue : schéma, table, nom, commande, permissive, `USING`, `WITH CHECK`.
type Politique = (
    String,
    String,
    String,
    String,
    bool,
    Option<String>,
    Option<String>,
);

/// Écarts de forme des politiques, et tables de tenant sans clé de cabinet.
async fn ecarts_politiques(connexion: &mut PgConnection, regles: &Regles) -> Vec<String> {
    // Tables des schémas de l'application, avec leurs clés de tenant.
    let tables: Vec<(String, String, bool, bool)> = sqlx::query_as(concat!(
        "SELECT n.nspname::text, c.relname::text, \
                EXISTS (SELECT 1 FROM pg_attribute a WHERE a.attrelid = c.oid \
                        AND a.attname = 'cabinet_id' AND NOT a.attisdropped), \
                EXISTS (SELECT 1 FROM pg_attribute a WHERE a.attrelid = c.oid \
                        AND a.attname = 'dossier_id' AND NOT a.attisdropped) \
         FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace WHERE ",
        schemas!(),
        " AND c.relkind IN ('r', 'p') ORDER BY 1, 2"
    ))
    .fetch_all(&mut *connexion)
    .await
    .unwrap();
    let politiques: Vec<Politique> = sqlx::query_as(concat!(
        "SELECT n.nspname::text, c.relname::text, p.polname::text, p.polcmd::text, \
                    p.polpermissive, pg_get_expr(p.polqual, p.polrelid), pg_get_expr(p.polwithcheck, p.polrelid) \
             FROM pg_policy p JOIN pg_class c ON c.oid = p.polrelid \
             JOIN pg_namespace n ON n.oid = c.relnamespace WHERE ",
        schemas!(),
        " ORDER BY 1, 2, 3"
    ))
    .fetch_all(&mut *connexion)
    .await
    .unwrap();

    let dans = |liste: &[String], schema: &str, table: &str| {
        schema == "public" && liste.iter().any(|t| t == table)
    };
    let mut ecarts = Vec::new();
    let mut formes: BTreeMap<(String, String), Vec<String>> = BTreeMap::new();
    for (schema, table, cabinet_id, dossier_id) in tables {
        if dans(&regles.instances, &schema, &table) {
            continue;
        }
        if table != "cabinet" && !cabinet_id {
            ecarts.push(format!(
                "{schema}.{table} : table de tenant sans colonne cabinet_id"
            ));
        }
        let admises = formes_admises(&table, dossier_id, regles);
        formes.insert((schema, table), admises);
    }

    let mut irreversibles_vues = BTreeSet::new();
    for (schema, table, nom, commande, permissive, using, check) in politiques {
        if dans(&regles.instances_sans_rls, &schema, &table) {
            continue;
        }
        let prefixe = format!("{schema}.{table} : politique {nom}");
        if !permissive {
            // Restrictive : seulement l'irréversibilité déclarée, de forme exacte.
            let admise = schema == "public"
                && commande == "w"
                && check.as_deref() == Some("true")
                && regles
                    .irreversible
                    .iter()
                    .any(|(t, u)| *t == table && using.as_deref() == Some(u.as_str()));
            if admise {
                irreversibles_vues.insert(table);
            } else {
                ecarts.push(format!(
                    "{prefixe} : politique restrictive hors liste (« {using:?} », « {check:?} »)"
                ));
            }
            continue;
        }
        if matches!(commande.as_str(), "*" | "r" | "w" | "d") && using.is_none() {
            ecarts.push(format!("{prefixe} sans USING"));
        }
        if matches!(commande.as_str(), "*" | "a" | "w") && check.is_none() {
            ecarts.push(format!("{prefixe} sans WITH CHECK"));
        }
        for (clause, expression) in [("USING", &using), ("WITH CHECK", &check)] {
            let Some(expression) = expression else {
                continue;
            };
            if dans(&regles.instances_avec_rls, &schema, &table) {
                if !expression.contains("current_setting('sigma.") {
                    ecarts.push(format!(
                        "{prefixe} : {clause} ne lit aucun contexte (« {expression} »)"
                    ));
                }
            } else if let Some(admises) = formes.get(&(schema.clone(), table.clone()))
                && !admises.iter().any(|f| f == expression)
                && !(commande == "r"
                    && dans(&regles.lecture_membre, &schema, &table)
                    && compacter(expression) == forme_lecture_membre(&table))
            {
                ecarts.push(format!(
                    "{prefixe} : {clause} hors forme admise (« {expression} »)"
                ));
            }
        }
    }
    for (table, _) in &regles.irreversible {
        if !irreversibles_vues.contains(table) {
            ecarts.push(format!(
                "public.{table} : politique d'irréversibilité déclarée absente"
            ));
        }
    }
    ecarts
}

/// Fil-piège du brouillard : tables `brouillon%` du schéma `public`, liste `brouillard` du
/// catalogue et tables lues à la clôture d'une période sont le même ensemble.
async fn ecarts_brouillard(
    connexion: &mut PgConnection,
    regles: &Regles,
    lues: &[&str],
) -> Vec<String> {
    let en_base: BTreeSet<String> = sqlx::query_scalar(
        "SELECT c.relname::text FROM pg_class c \
         WHERE c.relnamespace = 'public'::regnamespace AND c.relkind IN ('r', 'p') \
           AND c.relname LIKE 'brouillon%'",
    )
    .fetch_all(&mut *connexion)
    .await
    .unwrap()
    .into_iter()
    .collect();
    let declarees: BTreeSet<String> = regles.brouillard.iter().cloned().collect();
    let lues: BTreeSet<String> = lues.iter().map(|t| (*t).to_owned()).collect();
    let mut ecarts = Vec::new();
    for table in en_base.union(&declarees).chain(&lues) {
        if !(en_base.contains(table) && declarees.contains(table) && lues.contains(table)) {
            ecarts.push(format!(
                "table de brouillard {table} : en base {}, au catalogue {}, lue à la clôture {}",
                en_base.contains(table),
                declarees.contains(table),
                lues.contains(table)
            ));
        }
    }
    ecarts.sort();
    ecarts.dedup();
    ecarts
}

/// Écarts du catalogue de la base de la connexion.
async fn ecarts(connexion: &mut PgConnection, regles: &Regles) -> Vec<String> {
    let mut ecarts = Vec::new();
    for (liste, requete) in CONTROLES {
        let valeurs = match liste {
            Liste::Aucune => None,
            Liste::InstancesSansRls => Some(&regles.instances_sans_rls),
            Liste::Instances => Some(&regles.instances),
            Liste::AjoutSeul => Some(&regles.ajout_seul),
            Liste::SecurityDefiner => Some(&regles.security_definer),
        };
        let mut q = sqlx::query_scalar(*requete);
        if let Some(valeurs) = valeurs {
            q = q.bind(valeurs);
        }
        let lignes: Vec<String> = q
            .fetch_all(&mut *connexion)
            .await
            .unwrap_or_else(|e| panic!("contrôle de catalogue en échec ({e}) :\n{requete}"));
        ecarts.extend(lignes);
    }
    ecarts.extend(ecarts_politiques(connexion, regles).await);
    ecarts.extend(
        ecarts_brouillard(
            connexion,
            regles,
            api::base::referentiel::TABLES_BROUILLARD_LUES_A_LA_CLOTURE,
        )
        .await,
    );
    ecarts
}

/// Écarts introduits dans la base jetable par le propriétaire (droits par défaut en dernier).
const ECARTS_PROPRIETAIRE: &str = "CREATE TABLE table_oubliee (id uuid PRIMARY KEY, cabinet_id uuid NOT NULL); \
             CREATE TABLE rls_non_forcee (cabinet_id uuid NOT NULL); \
             ALTER TABLE rls_non_forcee ENABLE ROW LEVEL SECURITY; \
             CREATE TABLE politique_partielle (cabinet_id uuid NOT NULL); \
             ALTER TABLE politique_partielle ENABLE ROW LEVEL SECURITY; \
             ALTER TABLE politique_partielle FORCE ROW LEVEL SECURITY; \
             CREATE POLICY lecture ON politique_partielle FOR SELECT USING (true); \
             CREATE TABLE contexte_non_nul (cabinet_id uuid NOT NULL); \
             ALTER TABLE contexte_non_nul ENABLE ROW LEVEL SECURITY; \
             ALTER TABLE contexte_non_nul FORCE ROW LEVEL SECURITY; \
             CREATE POLICY tenant ON contexte_non_nul FOR ALL \
                 USING (current_setting('sigma.cabinet', true) IS NOT NULL) \
                 WITH CHECK (current_setting('sigma.cabinet', true) IS NOT NULL); \
             CREATE TABLE piece_laxiste (cabinet_id uuid NOT NULL, dossier_id uuid NOT NULL); \
             ALTER TABLE piece_laxiste ENABLE ROW LEVEL SECURITY; \
             ALTER TABLE piece_laxiste FORCE ROW LEVEL SECURITY; \
             CREATE POLICY tenant ON piece_laxiste FOR ALL \
                 USING (cabinet_id = NULLIF(current_setting('sigma.cabinet', true), '')::uuid) \
                 WITH CHECK (cabinet_id = NULLIF(current_setting('sigma.cabinet', true), '')::uuid); \
             CREATE TABLE piece_stricte (cabinet_id uuid NOT NULL, dossier_id uuid NOT NULL); \
             ALTER TABLE piece_stricte ENABLE ROW LEVEL SECURITY; \
             ALTER TABLE piece_stricte FORCE ROW LEVEL SECURITY; \
             CREATE POLICY tenant ON piece_stricte FOR ALL \
                 USING (cabinet_id = NULLIF(current_setting('sigma.cabinet', true), '')::uuid \
                        AND dossier_id = NULLIF(current_setting('sigma.dossier', true), '')::uuid) \
                 WITH CHECK (cabinet_id = NULLIF(current_setting('sigma.cabinet', true), '')::uuid \
                        AND dossier_id = NULLIF(current_setting('sigma.dossier', true), '')::uuid); \
             CREATE POLICY membre_hors_liste ON acces_dossier FOR SELECT \
                 USING (NULLIF(current_setting('sigma.cabinet', true), '') IS NULL \
                        AND utilisateur_id = NULLIF(current_setting('sigma.utilisateur', true), '')::uuid); \
             CREATE POLICY membre_toutes_commandes ON appartenance FOR ALL \
                 USING (NULLIF(current_setting('sigma.cabinet', true), '') IS NULL \
                        AND utilisateur_id = NULLIF(current_setting('sigma.utilisateur', true), '')::uuid) \
                 WITH CHECK (NULLIF(current_setting('sigma.cabinet', true), '') IS NULL \
                        AND utilisateur_id = NULLIF(current_setting('sigma.utilisateur', true), '')::uuid); \
             CREATE POLICY membre_meme_avec_cabinet ON appartenance FOR SELECT \
                 USING (utilisateur_id = NULLIF(current_setting('sigma.utilisateur', true), '')::uuid); \
             CREATE POLICY restrictive_hors_liste ON tiers AS RESTRICTIVE FOR UPDATE \
                 USING (actif) WITH CHECK (true); \
             CREATE POLICY irreversible_laxiste ON periode AS RESTRICTIVE FOR ALL \
                 USING (statut <> 'cloturee') WITH CHECK (true); \
             DROP POLICY exercice_irreversible ON exercice; \
             CREATE TABLE brouillon_oublie (cabinet_id uuid NOT NULL); \
             ALTER TABLE utilisateur ADD COLUMN solde double precision; \
             ALTER TABLE utilisateur ADD COLUMN cabinet_id uuid; \
             ALTER TABLE dossier ADD COLUMN ouvert_le timestamp; \
             CREATE VIEW vue_dossiers AS SELECT * FROM dossier; \
             GRANT TRUNCATE ON dossier TO sigma_application; \
             CREATE FUNCTION f_definer() RETURNS int LANGUAGE sql SECURITY DEFINER AS 'SELECT 1'; \
             REVOKE EXECUTE ON FUNCTION f_definer() FROM PUBLIC; \
             CREATE FUNCTION f_publique() RETURNS int LANGUAGE sql AS 'SELECT 1'; \
             CREATE SEQUENCE seq_test; \
             GRANT USAGE ON SEQUENCE seq_test TO sigma_application; \
             ALTER DEFAULT PRIVILEGES IN SCHEMA public GRANT SELECT ON TABLES TO sigma_application;";

/// Écarts introduits par le super-utilisateur, dont un rôle du cluster (rétabli aussitôt après
/// la lecture des écarts).
const ECARTS_SUPERUTILISATEUR: &str = "CREATE TABLE possedee (cabinet_id uuid NOT NULL); \
             ALTER TABLE possedee OWNER TO sigma_application; \
             GRANT CREATE ON SCHEMA public TO sigma_application; \
             DO $$ BEGIN \
                 CREATE ROLE sigma_test_contournement NOLOGIN BYPASSRLS; \
             EXCEPTION WHEN duplicate_object OR unique_violation THEN NULL; \
             END $$; \
             ALTER ROLE sigma_test_contournement LOGIN;";

/// Écarts que le contrôle doit trouver après [`ECARTS_PROPRIETAIRE`] et
/// [`ECARTS_SUPERUTILISATEUR`] (début de la ligne d'écart).
const ECARTS_ATTENDUS: &[&str] = &[
    "public.table_oubliee : RLS non activée",
    "public.table_oubliee : aucune politique pour SELECT",
    "public.rls_non_forcee : RLS non forcée",
    "public.politique_partielle : aucune politique pour INSERT",
    "public.politique_partielle : aucune politique pour DELETE",
    "public.politique_partielle : politique lecture : USING hors forme admise",
    "public.contexte_non_nul : politique tenant : USING hors forme admise",
    "public.contexte_non_nul : politique tenant : WITH CHECK hors forme admise",
    "public.piece_laxiste : politique tenant : USING hors forme admise",
    "public.piece_laxiste : politique tenant : WITH CHECK hors forme admise",
    "public.acces_dossier : politique membre_hors_liste : USING hors forme admise",
    "public.appartenance : politique membre_toutes_commandes : USING hors forme admise",
    "public.appartenance : politique membre_toutes_commandes : WITH CHECK hors forme admise",
    "public.appartenance : politique membre_meme_avec_cabinet : USING hors forme admise",
    "public.utilisateur.solde : type double precision interdit",
    "table d'instance utilisateur avec la colonne cabinet_id",
    "public.dossier.ouvert_le : type timestamp without time zone interdit",
    "public.vue_dossiers : vue sans security_invoker",
    "sigma_application a TRUNCATE sur public.dossier",
    "sigma_application possède la relation public.possedee",
    "sigma_application a CREATE sur le schéma public",
    "fonction public.f_definer : SECURITY DEFINER hors liste",
    "fonction public.f_publique : EXECUTE accordé à PUBLIC",
    "sigma_application a USAGE sur la séquence public.seq_test",
    "droits par défaut de sigma_migration (public)",
    "rôle sigma_test_contournement : BYPASSRLS sans être super-utilisateur",
    "public.tiers : politique restrictive_hors_liste : politique restrictive hors liste",
    "public.periode : politique irreversible_laxiste : politique restrictive hors liste",
    "public.exercice : politique d'irréversibilité déclarée absente",
    "table de brouillard brouillon_oublie : en base true, au catalogue false",
];

#[tokio::test]
async fn le_catalogue_respecte_les_regles_d_isolation() {
    let _verrou = VERROU_ROLES.lock().await;
    let env = env().await;
    let mut connexion = env.admin.acquire().await.unwrap();
    let ecarts = ecarts(&mut connexion, &regles()).await;
    assert!(
        ecarts.is_empty(),
        "écarts du catalogue :\n{}",
        ecarts.join("\n")
    );
}

#[tokio::test]
async fn une_table_oubliee_fait_echouer_le_controle() {
    let _verrou = VERROU_ROLES.lock().await;
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
        .database("sigma_test_catalogue_jetable");
    let mut jetable = PgConnection::connect_with(&options).await.unwrap();
    // Migrations exécutées sous le rôle de migration, comme `sqlx migrate run`.
    jetable.execute("SET ROLE sigma_migration").await.unwrap();
    MIGRATEUR.run(&mut jetable).await.unwrap();

    let regles = regles();
    let initiaux = ecarts(&mut jetable, &regles).await;
    assert!(initiaux.is_empty(), "base neuve : {initiaux:?}");

    // Écarts introduits par le propriétaire, puis par le super-utilisateur.
    jetable.execute(ECARTS_PROPRIETAIRE).await.unwrap();
    jetable.execute("RESET ROLE").await.unwrap();
    jetable.execute(ECARTS_SUPERUTILISATEUR).await.unwrap();

    let trouves = ecarts(&mut jetable, &regles).await;
    jetable
        .execute("ALTER ROLE sigma_test_contournement NOLOGIN")
        .await
        .unwrap();
    // Listes augmentées : table d'instance absente ; tables en ajout seul (l'une absente,
    // l'autre avec UPDATE et DELETE accordés).
    let mut regles_augmentees = regles.clone();
    regles_augmentees
        .instances
        .push("table_disparue".to_owned());
    regles_augmentees.ajout_seul = vec!["acces_dossier".to_owned(), "journal_absent".to_owned()];
    let trouves_listes = ecarts(&mut jetable, &regles_augmentees).await;
    jetable.close().await.unwrap();
    admin
        .execute("DROP DATABASE sigma_test_catalogue_jetable WITH (FORCE)")
        .await
        .unwrap();

    let detecte = |trouves: &[String], attendu: &str| {
        assert!(
            trouves.iter().any(|e| e.starts_with(attendu)),
            "écart non détecté : {attendu}\nécarts trouvés :\n{}",
            trouves.join("\n")
        );
    };
    for attendu in ECARTS_ATTENDUS {
        detecte(&trouves, attendu);
    }
    // La table de dossier stricte est conforme ; la fonction SECURITY DEFINER n'ouvre pas
    // EXECUTE à PUBLIC.
    for conforme in [
        "public.piece_stricte",
        "public.appartenance : politique appartenance_membre",
        "public.cabinet : politique cabinet_membre",
        "public.periode : politique periode_irreversible",
        "fonction public.f_definer : EXECUTE",
    ] {
        assert!(
            !trouves.iter().any(|e| e.starts_with(conforme)),
            "faux positif : {conforme}\n{}",
            trouves.join("\n")
        );
    }
    for attendu in [
        "table d'instance table_disparue absente du schéma",
        "table en ajout seul journal_absent absente du schéma",
        "sigma_application a UPDATE sur public.acces_dossier (ajout seul)",
        "sigma_application a DELETE sur public.acces_dossier (ajout seul)",
    ] {
        detecte(&trouves_listes, attendu);
    }
}
