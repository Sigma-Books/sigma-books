//! Accès à PostgreSQL : pool du rôle applicatif et transaction au contexte de tenant (ADR 0011).
//!
//! L'isolation entre tenants tient à trois niveaux (Fondations §3.2) ; ce module porte les deux
//! derniers :
//! - **RLS** : chaque transaction pose le contexte (cabinet, dossier facultatif, utilisateur) par
//!   `set_config('sigma.…', $1, true)`, l'équivalent paramétré de `SET LOCAL`. Le contexte meurt
//!   avec la transaction : il ne fuit jamais d'une connexion du pool à la suivante ;
//! - **requête** : les fonctions d'accès ([`organisation`]) filtrent aussi par le contexte de la
//!   [`TransactionTenant`] qu'elles reçoivent, si bien qu'un rôle qui contournerait la RLS ne verrait
//!   toujours rien d'un autre tenant.

pub mod organisation;

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

/// Contexte de tenant d'une transaction : cabinet, dossier (absent pour une action du cabinet)
/// et utilisateur. Il vient du chemin de la requête HTTP, revérifié par Cedar (ADR 0011).
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

async fn verifier_role_application(pool: &PgPool) -> Result<(), ErreurBase> {
    let role = sqlx::query!(
        r#"
        SELECT
            r.rolsuper AS "superutilisateur!",
            r.rolbypassrls AS "contourne_rls!",
            EXISTS (
                SELECT 1
                FROM pg_catalog.pg_class c
                JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace
                WHERE n.nspname = 'public' AND pg_has_role(current_user, c.relowner, 'USAGE')
            ) AS "proprietaire!"
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
    if role.proprietaire {
        return Err(ErreurBase::RoleTropPrivilegie(
            "propriétaire d'objets du schéma",
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
    let mut tx = pool.begin().await?;
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
