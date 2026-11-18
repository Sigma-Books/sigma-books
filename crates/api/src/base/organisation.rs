//! Fonctions d'accès aux tables d'organisation (cabinet, dossier).
//!
//! Niveau « requête » de l'isolation (ADR 0011) : chaque requête filtre par le contexte de la
//! [`TransactionTenant`], en plus de la RLS. Les tests l'exécutent sous un rôle qui contourne la
//! RLS : le filtre suffit seul. Une ressource d'un autre tenant est une ressource absente (`None`).

use time::OffsetDateTime;

use super::{ErreurBase, IdCabinet, IdDossier, TransactionTenant};

/// Un cabinet (tenant).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cabinet {
    /// Identifiant.
    pub id: IdCabinet,
    /// Nom.
    pub nom: String,
    /// Séparation des tâches (spec §20, Q5.3) : activée par défaut.
    pub separation_taches: bool,
    /// Instant de création.
    pub cree_le: OffsetDateTime,
}

/// Un dossier : une entité juridique tenue par le cabinet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dossier {
    /// Identifiant.
    pub id: IdDossier,
    /// Cabinet qui tient le dossier.
    pub cabinet: IdCabinet,
    /// Nom (raison sociale).
    pub nom: String,
    /// Instant de création.
    pub cree_le: OffsetDateTime,
}

/// Lit le cabinet du contexte.
///
/// # Errors
/// Erreur de PostgreSQL.
pub async fn lire_cabinet(tx: &mut TransactionTenant) -> Result<Option<Cabinet>, ErreurBase> {
    let cabinet = tx.contexte().id_cabinet().uuid();
    let ligne = sqlx::query!(
        "SELECT id, nom, separation_taches, cree_le FROM cabinet WHERE id = $1",
        cabinet,
    )
    .fetch_optional(tx.connexion())
    .await?;
    Ok(ligne.map(|l| Cabinet {
        id: IdCabinet::depuis_uuid(l.id),
        nom: l.nom,
        separation_taches: l.separation_taches,
        cree_le: l.cree_le,
    }))
}

/// Liste les dossiers du cabinet du contexte, par nom ; dans le contexte d'un dossier, ce seul
/// dossier (même règle que la RLS).
///
/// L'affectation de l'utilisateur aux dossiers (Q5.1) est vérifiée par Cedar, pas ici.
///
/// # Errors
/// Erreur de PostgreSQL.
pub async fn lister_dossiers(tx: &mut TransactionTenant) -> Result<Vec<Dossier>, ErreurBase> {
    let cabinet = tx.contexte().id_cabinet().uuid();
    let dossier = tx.contexte().id_dossier().map(IdDossier::uuid);
    let lignes = sqlx::query!(
        "SELECT cabinet_id, id, nom, cree_le FROM dossier \
         WHERE cabinet_id = $1 AND ($2::uuid IS NULL OR id = $2) \
         ORDER BY nom, id",
        cabinet,
        dossier,
    )
    .fetch_all(tx.connexion())
    .await?;
    Ok(lignes
        .into_iter()
        .map(|l| Dossier {
            id: IdDossier::depuis_uuid(l.id),
            cabinet: IdCabinet::depuis_uuid(l.cabinet_id),
            nom: l.nom,
            cree_le: l.cree_le,
        })
        .collect())
}

/// Lit un dossier du cabinet du contexte. Un dossier d'un autre cabinet, ou autre que celui du
/// contexte, est absent (`None`), comme un identifiant inexistant.
///
/// # Errors
/// Erreur de PostgreSQL.
pub async fn lire_dossier(
    tx: &mut TransactionTenant,
    id: IdDossier,
) -> Result<Option<Dossier>, ErreurBase> {
    let cabinet = tx.contexte().id_cabinet().uuid();
    let dossier = tx.contexte().id_dossier().map(IdDossier::uuid);
    let ligne = sqlx::query!(
        "SELECT cabinet_id, id, nom, cree_le FROM dossier \
         WHERE cabinet_id = $1 AND id = $2 AND ($3::uuid IS NULL OR id = $3)",
        cabinet,
        id.uuid(),
        dossier,
    )
    .fetch_optional(tx.connexion())
    .await?;
    Ok(ligne.map(|l| Dossier {
        id: IdDossier::depuis_uuid(l.id),
        cabinet: IdCabinet::depuis_uuid(l.cabinet_id),
        nom: l.nom,
        cree_le: l.cree_le,
    }))
}

/// Crée un dossier dans le cabinet du contexte, avec un nouvel identifiant (UUID v4).
///
/// Action du cabinet : refusée dans le contexte d'un dossier.
///
/// # Errors
/// [`ErreurBase::ContexteDeDossier`], ou erreur de PostgreSQL (nom invalide, RLS).
pub async fn creer_dossier(tx: &mut TransactionTenant, nom: &str) -> Result<Dossier, ErreurBase> {
    if tx.contexte().id_dossier().is_some() {
        return Err(ErreurBase::ContexteDeDossier);
    }
    let cabinet = tx.contexte().id_cabinet().uuid();
    let id = IdDossier::nouveau();
    let ligne = sqlx::query!(
        "INSERT INTO dossier (cabinet_id, id, nom) VALUES ($1, $2, $3) RETURNING cree_le",
        cabinet,
        id.uuid(),
        nom,
    )
    .fetch_one(tx.connexion())
    .await?;
    Ok(Dossier {
        id,
        cabinet: IdCabinet::depuis_uuid(cabinet),
        nom: nom.to_owned(),
        cree_le: ligne.cree_le,
    })
}
