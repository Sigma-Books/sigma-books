//! Accès à PostgreSQL : pool du rôle applicatif et transaction au contexte de tenant (ADR 0011).
//! Variables d'environnement : [`config`].
//!
//! L'isolation entre tenants tient à trois niveaux (Fondations §3.2) ; ce module porte les deux
//! derniers :
//! - **RLS** : chaque transaction pose le contexte (cabinet, dossier facultatif, utilisateur) par
//!   `set_config('sigma.…', $1, true)`, l'équivalent paramétré de `SET LOCAL`. Le contexte meurt
//!   avec la transaction : il ne fuit jamais d'une connexion du pool à la suivante ;
//! - **requête** : les fonctions d'accès ([`organisation`]) filtrent aussi par le contexte de la
//!   [`TransactionTenant`] qu'elles reçoivent, si bien qu'un rôle qui contournerait la RLS ne verrait
//!   toujours rien d'un autre tenant.
//!
//! Hors de tout cabinet, une [`TransactionUtilisateur`] ne pose que l'utilisateur : ses sessions
//! ([`session`]) et ses propres appartenances et cabinets (`GET /api/moi/cabinets`) lui sont
//! visibles, rien d'autre (migration `0004_sessions.sql`).
//!
//! # `sigma.utilisateur` : un utilisateur **authentifié**, jamais revendiqué
//! Le réglage `sigma.utilisateur` ouvre, sous RLS, tout ce qui appartient à cet utilisateur :
//! ses sessions (politique `session_titulaire` : les lire, les tourner, les révoquer), ses
//! appartenances et ses cabinets (`appartenance_membre`, `cabinet_membre`), sa ligne
//! `utilisateur`. Il ne doit **jamais** désigner un utilisateur seulement revendiqué, par exemple
//! le titulaire d'un courriel saisi à la connexion (PR 8b et 8c) : une telle transaction utilise
//! un autre réglage, propre à la connexion (`sigma.courriel_connexion`), et un autre type de
//! transaction ([`facteurs::TransactionConnexion`]). Seuls le posent :
//! - [`ouvrir_transaction_utilisateur`], **réservée au crate** (`pub(crate)`), appelée pour
//!   l'utilisateur d'une session authentifiée (pipeline, `serveur::moi` ; notation de
//!   l'activité) ou pour ouvrir la session d'un utilisateur dont un facteur vient d'être
//!   vérifié (`serveur::session::ouvrir_session`) ;
//! - [`facteurs::TransactionConnexion::authentifier`], sur présentation d'un
//!   [`facteurs::FacteurVerifie`] (pas TOTP ou code de récupération consommé), que seul le
//!   module [`facteurs`] fabrique ;
//! - [`ouvrir_transaction`] et [`ouvrir_transaction_lecture`], au contexte d'un cabinet, avec
//!   l'utilisateur authentifié du pipeline.
//!
//! Le journal d'audit ([`audit`]) est écrit par le rôle applicatif, qui ne peut pas le relire, et
//! lu par le rôle de lecture d'audit, sur un pool distinct ([`ouvrir_pool_lecture_audit`]),
//! toujours sous RLS et contexte (ADR 0014).

pub mod administration;
pub mod audit;
pub mod brouillard;
pub mod config;
pub mod editions;
pub mod evenement;
pub mod facteurs;
pub mod integrite;
pub mod jetons;
pub mod liens;
pub mod organisation;
pub mod parametres_fiscaux;
pub mod passkeys;
pub mod referentiel;
pub mod session;
pub mod taches;
pub mod tiers;
pub mod tresorerie;
pub mod validation;

use std::time::Duration;

use sqlx::postgres::{PgConnection, PgPool, PgPoolOptions};
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

/// Erreur d'accès à la base.
#[derive(Debug, thiserror::Error)]
pub enum ErreurBase {
    /// Erreur rapportée par `sqlx` ou par PostgreSQL.
    #[error("accès à la base : {0}")]
    Sqlx(#[from] sqlx::Error),
    /// Le rôle de connexion contourne la RLS ou possède des objets : il n'est pas le rôle applicatif.
    #[error("le rôle de connexion n'est pas un rôle applicatif : {0}")]
    RoleTropPrivilegie(&'static str),
    /// L'action relève du cabinet et ne peut pas s'exécuter dans le contexte d'un dossier.
    #[error("action du cabinet demandée dans le contexte d'un dossier")]
    ContexteDeDossier,
    /// Donnée lue en base incohérente (valeur hors de la liste attendue), ou demandée hors de
    /// son contexte.
    #[error("donnée incohérente : {0}")]
    Donnee(&'static str),
    /// Version de paramètres fiscaux d'un dossier illisible ou altérée (ADR 0030).
    #[error("version de paramètres fiscaux du dossier illisible")]
    VersionParametresIllisible,
    /// Le journal d'audit refuse l'entrée (portée hors du contexte, tête incohérente, entrée
    /// invalide) : l'action est annulée.
    #[error("inscription au journal d'audit impossible : {0}")]
    Audit(&'static str),
}

/// Identifiant d'un cabinet (UUID v4 tiré par `api`, ADR 0011).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct IdCabinet(Uuid);

/// Identifiant d'un dossier (UUID v4 tiré par `api`, ADR 0011).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct IdDossier(Uuid);

/// Identifiant d'un utilisateur (UUID v4 tiré par `api`, ADR 0011).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct IdUtilisateur(Uuid);

/// Identifiant d'une appartenance d'un utilisateur à un cabinet : principal des décisions Cedar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct IdAppartenance(Uuid);

/// Identifiant d'une affectation d'un membre à un dossier (Q5.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct IdAccesDossier(Uuid);

/// Identifiant **public** d'une session (ADR 0012), inscrit au journal d'audit : jamais son
/// jeton.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct IdSession(Uuid);

macro_rules! identifiant {
    ($type:ident) => {
        impl $type {
            /// Tire un nouvel identifiant (UUID v4).
            #[must_use]
            pub fn nouveau() -> Self {
                Self(Uuid::new_v4())
            }

            /// Reprend un identifiant existant (chemin d'une requête, ligne lue en base).
            #[must_use]
            pub const fn depuis_uuid(uuid: Uuid) -> Self {
                Self(uuid)
            }

            /// L'UUID sous-jacent.
            #[must_use]
            pub const fn uuid(self) -> Uuid {
                self.0
            }
        }

        impl std::fmt::Display for $type {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                // Forme canonique : minuscules avec tirets (format-v1.md §3).
                self.0.hyphenated().fmt(f)
            }
        }
    };
}

identifiant!(IdCabinet);
identifiant!(IdDossier);
identifiant!(IdUtilisateur);
identifiant!(IdAppartenance);
identifiant!(IdAccesDossier);
identifiant!(IdSession);

/// Contexte de tenant d'une transaction : cabinet, dossier (absent pour une action du cabinet)
/// et utilisateur. Il vient du chemin de la requête HTTP, revérifié par Cedar (ADR 0011).
/// L'utilisateur est celui de la session authentifiée, jamais un utilisateur revendiqué (doc
/// du module, « `sigma.utilisateur` »).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContexteTenant {
    cabinet: IdCabinet,
    dossier: Option<IdDossier>,
    utilisateur: IdUtilisateur,
}

impl ContexteTenant {
    /// Contexte d'une action du cabinet (aucun dossier).
    #[must_use]
    pub const fn cabinet(cabinet: IdCabinet, utilisateur: IdUtilisateur) -> Self {
        Self {
            cabinet,
            dossier: None,
            utilisateur,
        }
    }

    /// Contexte d'une action dans un dossier du cabinet.
    #[must_use]
    pub const fn dossier(
        cabinet: IdCabinet,
        dossier: IdDossier,
        utilisateur: IdUtilisateur,
    ) -> Self {
        Self {
            cabinet,
            dossier: Some(dossier),
            utilisateur,
        }
    }

    /// Contexte d'une tâche de la file (aucun utilisateur : l'identifiant nul ne désigne
    /// personne). Réservé au travailleur, qui le tire d'une ligne de la file.
    #[must_use]
    pub(crate) const fn systeme(cabinet: IdCabinet, dossier: IdDossier) -> Self {
        Self {
            cabinet,
            dossier: Some(dossier),
            utilisateur: IdUtilisateur(Uuid::nil()),
        }
    }

    /// Le cabinet du contexte.
    #[must_use]
    pub const fn id_cabinet(&self) -> IdCabinet {
        self.cabinet
    }

    /// Le dossier du contexte, absent pour une action du cabinet.
    #[must_use]
    pub const fn id_dossier(&self) -> Option<IdDossier> {
        self.dossier
    }

    /// L'utilisateur du contexte.
    #[must_use]
    pub const fn id_utilisateur(&self) -> IdUtilisateur {
        self.utilisateur
    }
}

/// Réglages du pool du rôle applicatif.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReglagesPool {
    /// Nombre maximal de connexions ouvertes.
    pub connexions_max: u32,
    /// Attente maximale d'une connexion libre.
    pub attente_max: Duration,
}

impl Default for ReglagesPool {
    fn default() -> Self {
        Self {
            connexions_max: 10,
            attente_max: Duration::from_secs(5),
        }
    }
}

/// Ouvre le pool de connexions du rôle applicatif.
///
/// Refuse un rôle qui contournerait la RLS (`SUPERUSER`, `BYPASSRLS`) ou qui posséderait un objet
/// du schéma (rôle de migration, ou membre de celui-ci) : l'application ne s'exécute jamais avec
/// ces droits (F2.10).
///
/// # Errors
/// Connexion impossible, ou rôle trop privilégié ([`ErreurBase::RoleTropPrivilegie`]).
pub async fn ouvrir_pool_application(
    url: &str,
    reglages: ReglagesPool,
) -> Result<PgPool, ErreurBase> {
    let pool = PgPoolOptions::new()
        .max_connections(reglages.connexions_max)
        .acquire_timeout(reglages.attente_max)
        .connect(url)
        .await?;
    verifier_role_application(&pool).await?;
    Ok(pool)
}

/// Vérifie que le rôle courant (`current_user`) des connexions du pool est un rôle applicatif :
/// ni `SUPERUSER`, ni `BYPASSRLS`, membre (`MEMBER`, que l'héritage soit actif ou non :
/// `NOINHERIT` n'empêche pas `SET ROLE`) d'aucun propriétaire d'objet du schéma ni de la base,
/// ni de `sigma_migration`.
///
/// # Errors
/// [`ErreurBase::RoleTropPrivilegie`], ou erreur de PostgreSQL.
pub async fn verifier_role_application(pool: &PgPool) -> Result<(), ErreurBase> {
    let role = sqlx::query!(
        r#"
        SELECT
            r.rolsuper AS "superutilisateur!",
            r.rolbypassrls AS "contourne_rls!",
            EXISTS (
                SELECT 1
                FROM pg_catalog.pg_class c
                JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace
                WHERE n.nspname = 'public' AND pg_has_role(current_user, c.relowner, 'MEMBER')
            ) AS "proprietaire_objet!",
            EXISTS (
                SELECT 1
                FROM pg_catalog.pg_database d
                WHERE d.datname = current_database() AND pg_has_role(current_user, d.datdba, 'MEMBER')
            ) AS "proprietaire_base!",
            EXISTS (
                SELECT 1
                FROM pg_catalog.pg_roles m
                WHERE m.rolname = 'sigma_migration' AND pg_has_role(current_user, m.oid, 'MEMBER')
            ) AS "membre_migration!"
        FROM pg_catalog.pg_roles r
        WHERE r.rolname = current_user
        "#
    )
    .fetch_one(pool)
    .await?;
    if role.superutilisateur {
        return Err(ErreurBase::RoleTropPrivilegie("SUPERUSER"));
    }
    if role.contourne_rls {
        return Err(ErreurBase::RoleTropPrivilegie("BYPASSRLS"));
    }
    if role.membre_migration {
        return Err(ErreurBase::RoleTropPrivilegie("membre de sigma_migration"));
    }
    if role.proprietaire_base {
        return Err(ErreurBase::RoleTropPrivilegie("propriétaire de la base"));
    }
    if role.proprietaire_objet {
        return Err(ErreurBase::RoleTropPrivilegie(
            "propriétaire d'objets du schéma",
        ));
    }
    Ok(())
}

/// Ouvre le pool de connexions du rôle de **lecture d'audit** (ADR 0014), distinct de celui du
/// rôle applicatif.
///
/// Refuse un rôle qui contournerait la RLS ou posséderait un objet (comme
/// [`verifier_role_application`]), qui serait membre du rôle applicatif, ou qui aurait un droit
/// d'écriture sur une table du schéma : ce pool ne fait que lire l'audit.
///
/// # Errors
/// Connexion impossible, ou rôle trop privilégié ([`ErreurBase::RoleTropPrivilegie`]).
pub async fn ouvrir_pool_lecture_audit(
    url: &str,
    reglages: ReglagesPool,
) -> Result<PgPool, ErreurBase> {
    let pool = PgPoolOptions::new()
        .max_connections(reglages.connexions_max)
        .acquire_timeout(reglages.attente_max)
        .connect(url)
        .await?;
    verifier_role_lecture_audit(&pool).await?;
    Ok(pool)
}

/// Vérifie que le rôle courant des connexions du pool est un rôle de lecture : les contrôles de
/// [`verifier_role_application`], ni membre de `sigma_application`, ni `INSERT`, `UPDATE`,
/// `DELETE` ou `TRUNCATE` sur une table du schéma `public`.
///
/// # Errors
/// [`ErreurBase::RoleTropPrivilegie`], ou erreur de PostgreSQL.
pub async fn verifier_role_lecture_audit(pool: &PgPool) -> Result<(), ErreurBase> {
    verifier_role_application(pool).await?;
    let role = sqlx::query!(
        r#"
        SELECT
            EXISTS (
                SELECT 1
                FROM pg_catalog.pg_roles m
                WHERE m.rolname = 'sigma_application' AND pg_has_role(current_user, m.oid, 'MEMBER')
            ) AS "membre_application!",
            EXISTS (
                SELECT 1
                FROM pg_catalog.pg_class c
                JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace
                CROSS JOIN (VALUES ('INSERT'), ('UPDATE'), ('DELETE'), ('TRUNCATE')) AS d (droit)
                WHERE n.nspname = 'public' AND c.relkind IN ('r', 'p')
                  AND (has_table_privilege(current_user, c.oid, d.droit)
                       OR (d.droit IN ('INSERT', 'UPDATE')
                           AND has_any_column_privilege(current_user, c.oid, d.droit)))
            ) AS "ecrit!"
        "#
    )
    .fetch_one(pool)
    .await?;
    if role.membre_application {
        return Err(ErreurBase::RoleTropPrivilegie(
            "membre de sigma_application",
        ));
    }
    if role.ecrit {
        return Err(ErreurBase::RoleTropPrivilegie(
            "droit d'écriture sur une table",
        ));
    }
    Ok(())
}

/// Transaction PostgreSQL dont le contexte de tenant est posé.
///
/// Les fonctions d'accès la reçoivent et filtrent par son contexte : les valeurs du filtre de la
/// requête sont donc celles que lit la RLS. Abandonnée sans [`TransactionTenant::confirmer`], elle
/// est annulée.
#[derive(Debug)]
pub struct TransactionTenant {
    tx: Transaction<'static, Postgres>,
    contexte: ContexteTenant,
}

impl TransactionTenant {
    /// Le contexte posé dans la transaction.
    #[must_use]
    pub const fn contexte(&self) -> &ContexteTenant {
        &self.contexte
    }

    /// La connexion de la transaction, pour les requêtes du crate. Toute requête filtre par
    /// [`TransactionTenant::contexte`] en plus de la RLS.
    pub fn connexion(&mut self) -> &mut PgConnection {
        &mut self.tx
    }

    /// Restreint le contexte de la transaction au dossier `dossier` du cabinet du contexte, pour
    /// le reste de la transaction (`set_config(…, true)`).
    ///
    /// Sert à une action du cabinet qui crée un dossier et inscrit la première entrée de sa
    /// chaîne d'audit (ADR 0014). Le contexte ne peut que se **restreindre** : d'un cabinet à
    /// l'un de ses dossiers, jamais d'un dossier à un autre ni vers un autre cabinet ; la RLS
    /// et les filtres des requêtes ne voient ensuite que ce dossier.
    ///
    /// # Errors
    /// [`ErreurBase::ContexteDeDossier`] si un autre dossier est déjà en contexte, ou erreur de
    /// PostgreSQL.
    pub async fn restreindre_au_dossier(&mut self, dossier: IdDossier) -> Result<(), ErreurBase> {
        match self.contexte.dossier {
            Some(courant) if courant == dossier => return Ok(()),
            Some(_) => return Err(ErreurBase::ContexteDeDossier),
            None => {}
        }
        let texte = dossier.to_string();
        sqlx::query!(
            "SELECT set_config('sigma.dossier', $1, true) AS dossier",
            texte
        )
        .fetch_one(&mut *self.tx)
        .await?;
        self.contexte.dossier = Some(dossier);
        Ok(())
    }

    /// Valide la transaction (`COMMIT`).
    ///
    /// # Errors
    /// Erreur de PostgreSQL à la validation.
    pub async fn confirmer(self) -> Result<(), ErreurBase> {
        self.tx.commit().await?;
        Ok(())
    }

    /// Annule la transaction (`ROLLBACK`).
    ///
    /// # Errors
    /// Erreur de PostgreSQL à l'annulation.
    pub async fn annuler(self) -> Result<(), ErreurBase> {
        self.tx.rollback().await?;
        Ok(())
    }
}

/// Ouvre une transaction (`READ COMMITTED`) et y pose le contexte de tenant.
///
/// Le contexte est posé par `set_config(…, true)` : local à la transaction, il disparaît au
/// `COMMIT` comme au `ROLLBACK`. Aucune valeur n'est concaténée au SQL (F2.1).
///
/// # Errors
/// Connexion indisponible ou erreur de PostgreSQL.
pub async fn ouvrir_transaction(
    pool: &PgPool,
    contexte: ContexteTenant,
) -> Result<TransactionTenant, ErreurBase> {
    let tx = pool.begin().await?;
    poser_contexte(tx, contexte).await
}

/// Ouvre une transaction de **lecture seule** à instantané unique (`REPEATABLE READ READ
/// ONLY`) et y pose le contexte de tenant : toutes ses lectures voient le même état validé.
///
/// Sert aux lectures du journal d'audit (consultation, vérification, export, ADR 0014) : une
/// action validée entre la lecture de la tête et celle des entrées ne peut pas faire croire à
/// une chaîne rompue. Le texte de `BEGIN` est constant.
///
/// # Errors
/// Connexion indisponible ou erreur de PostgreSQL.
pub async fn ouvrir_transaction_lecture(
    pool: &PgPool,
    contexte: ContexteTenant,
) -> Result<TransactionTenant, ErreurBase> {
    let tx = pool
        .begin_with("BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY")
        .await?;
    poser_contexte(tx, contexte).await
}

/// Transaction PostgreSQL **sans cabinet**, au contexte d'un seul utilisateur (ADR 0012).
///
/// Sous RLS, elle ne voit que les sessions de cet utilisateur, ses appartenances et les cabinets
/// où il en a une (politiques `appartenance_membre` et `cabinet_membre`, lecture seule) : aucun
/// dossier, aucune donnée d'un cabinet. Les fonctions d'accès filtrent aussi par
/// [`TransactionUtilisateur::utilisateur`]. Abandonnée sans [`TransactionUtilisateur::confirmer`],
/// elle est annulée.
#[derive(Debug)]
pub struct TransactionUtilisateur {
    tx: Transaction<'static, Postgres>,
    utilisateur: IdUtilisateur,
}

impl TransactionUtilisateur {
    /// L'utilisateur du contexte.
    #[must_use]
    pub const fn utilisateur(&self) -> IdUtilisateur {
        self.utilisateur
    }

    /// La connexion de la transaction, pour les requêtes du crate. Toute requête filtre par
    /// [`TransactionUtilisateur::utilisateur`] en plus de la RLS.
    pub fn connexion(&mut self) -> &mut PgConnection {
        &mut self.tx
    }

    /// Valide la transaction (`COMMIT`).
    ///
    /// # Errors
    /// Erreur de PostgreSQL à la validation.
    pub async fn confirmer(self) -> Result<(), ErreurBase> {
        self.tx.commit().await?;
        Ok(())
    }
}

/// Ouvre une transaction (`READ COMMITTED`) au contexte du seul utilisateur : cabinet, dossier
/// et jeton de session vides (lus comme NULL par les politiques).
///
/// **Réservée au crate** : `utilisateur` doit être celui d'une session authentifiée, ou celui
/// dont un facteur vient d'être vérifié, **jamais** un utilisateur seulement revendiqué (voir
/// la doc du module, « `sigma.utilisateur` »). Le test de structure
/// (`tests/structure.rs`) en liste les seuls appelants admis.
///
/// # Errors
/// Connexion indisponible ou erreur de PostgreSQL.
pub(crate) async fn ouvrir_transaction_utilisateur(
    pool: &PgPool,
    utilisateur: IdUtilisateur,
) -> Result<TransactionUtilisateur, ErreurBase> {
    let mut tx = pool.begin().await?;
    let texte = utilisateur.to_string();
    sqlx::query!(
        "SELECT set_config('sigma.cabinet', '', true) AS cabinet, \
                set_config('sigma.dossier', '', true) AS dossier, \
                set_config('sigma.jeton_session', '', true) AS jeton, \
                set_config('sigma.utilisateur', $1, true) AS utilisateur",
        texte,
    )
    .fetch_one(&mut *tx)
    .await?;
    Ok(TransactionUtilisateur { tx, utilisateur })
}

async fn poser_contexte(
    mut tx: Transaction<'static, Postgres>,
    contexte: ContexteTenant,
) -> Result<TransactionTenant, ErreurBase> {
    let cabinet = contexte.cabinet.to_string();
    // Dossier absent : chaîne vide, que les politiques lisent comme NULL (NULLIF).
    let dossier = contexte
        .dossier
        .map_or_else(String::new, |dossier| dossier.to_string());
    let utilisateur = contexte.utilisateur.to_string();
    sqlx::query!(
        "SELECT set_config('sigma.cabinet', $1, true) AS cabinet, \
                set_config('sigma.dossier', $2, true) AS dossier, \
                set_config('sigma.utilisateur', $3, true) AS utilisateur",
        cabinet,
        dossier,
        utilisateur,
    )
    .fetch_one(&mut *tx)
    .await?;
    Ok(TransactionTenant { tx, contexte })
}
