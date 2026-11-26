//! Versions de paramètres fiscaux propres à un dossier (ADR 0030) : table en **ajout seul**
//! (`INSERT` et `SELECT` seulement, RLS forcée sur le cabinet et le dossier du contexte).

use time::{Date, OffsetDateTime};
use uuid::Uuid;

use super::{ErreurBase, TransactionTenant};

/// Nombre maximal de versions propres d'un dossier (contrainte de la table).
pub const VERSIONS_MAX: usize = 50;

/// Ligne de la table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionPropre {
    /// Identifiant de la ligne (cible de la trace d'audit).
    pub id: Uuid,
    /// Numéro dans le dossier (`+d<numero>`).
    pub numero: i32,
    /// Pays (ISO 3166-1 alpha-2).
    pub pays: String,
    /// Version de base du paquet (`AAAA.N`).
    pub version: String,
    /// Premier jour d'application.
    pub date_effet: Date,
    /// Dernier jour d'application.
    pub date_fin: Option<Date>,
    /// Contenu complet, au format des fichiers de données.
    pub contenu: String,
    /// SHA-256 du contenu.
    pub empreinte: Vec<u8>,
    /// Motif de la surcharge (texte libre).
    pub motif: String,
    /// Auteur (identifiant).
    pub cree_par: Uuid,
    /// Instant de création.
    pub cree_le: OffsetDateTime,
}

fn tenant(tx: &TransactionTenant) -> Result<(Uuid, Uuid), ErreurBase> {
    let contexte = tx.contexte();
    let dossier = contexte
        .id_dossier()
        .ok_or(ErreurBase::ContexteDeDossier)?
        .uuid();
    Ok((contexte.id_cabinet().uuid(), dossier))
}

/// Versions propres du dossier du contexte, par numéro croissant.
///
/// # Errors
/// Contexte sans dossier, erreur de PostgreSQL.
pub async fn lire_versions(tx: &mut TransactionTenant) -> Result<Vec<VersionPropre>, ErreurBase> {
    let (cabinet, dossier) = tenant(tx)?;
    let lignes = sqlx::query!(
        r#"
        SELECT id, numero, pays, version, date_effet, date_fin, contenu, empreinte, motif,
               cree_par, cree_le
        FROM parametres_fiscaux_dossier
        WHERE cabinet_id = $1 AND dossier_id = $2
        ORDER BY numero
        "#,
        cabinet,
        dossier,
    )
    .fetch_all(tx.connexion())
    .await?;
    Ok(lignes
        .into_iter()
        .map(|l| VersionPropre {
            id: l.id,
            numero: l.numero,
            pays: l.pays,
            version: l.version,
            date_effet: l.date_effet,
            date_fin: l.date_fin,
            contenu: l.contenu,
            empreinte: l.empreinte,
            motif: l.motif,
            cree_par: l.cree_par,
            cree_le: l.cree_le,
        })
        .collect())
}

/// Plus grand numéro de version propre du dossier du contexte (0 s'il n'y en a aucune) : relu
/// sous le verrou de la tête de chaîne, il détecte une version créée pendant une validation.
///
/// # Errors
/// Contexte sans dossier, erreur de PostgreSQL.
pub async fn numero_maximal(tx: &mut TransactionTenant) -> Result<i32, ErreurBase> {
    let (cabinet, dossier) = tenant(tx)?;
    let n = sqlx::query_scalar!(
        r#"SELECT COALESCE(max(numero), 0) AS "n!" FROM parametres_fiscaux_dossier
           WHERE cabinet_id = $1 AND dossier_id = $2"#,
        cabinet,
        dossier,
    )
    .fetch_one(tx.connexion())
    .await?;
    Ok(n)
}

/// Parmi `codes`, ceux qu'au moins une écriture **validée** du dossier utilise comme régime.
///
/// # Errors
/// Contexte sans dossier, erreur de PostgreSQL.
pub async fn regimes_utilises(
    tx: &mut TransactionTenant,
    codes: &[String],
) -> Result<Vec<String>, ErreurBase> {
    let (cabinet, dossier) = tenant(tx)?;
    let utilises = sqlx::query_scalar!(
        r#"SELECT DISTINCT fiscal_regime AS "regime!" FROM ecriture_validee_ligne
           WHERE cabinet_id = $1 AND dossier_id = $2 AND fiscal_regime = ANY($3)
           ORDER BY 1"#,
        cabinet,
        dossier,
        codes,
    )
    .fetch_all(tx.connexion())
    .await?;
    Ok(utilises)
}

/// Enregistre une version propre (ajout seul).
///
/// # Errors
/// Contexte sans dossier, contrainte refusée par PostgreSQL (date d'effet ou numéro en double,
/// plus de [`VERSIONS_MAX`] versions).
pub async fn inserer(tx: &mut TransactionTenant, v: &VersionPropre) -> Result<(), ErreurBase> {
    let (cabinet, dossier) = tenant(tx)?;
    sqlx::query!(
        r#"
        INSERT INTO parametres_fiscaux_dossier
            (cabinet_id, dossier_id, id, numero, pays, version, date_effet, date_fin, contenu,
             empreinte, motif, cree_par, cree_le)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
        "#,
        cabinet,
        dossier,
        v.id,
        v.numero,
        v.pays,
        v.version,
        v.date_effet,
        v.date_fin,
        v.contenu,
        v.empreinte,
        v.motif,
        v.cree_par,
        v.cree_le,
    )
    .execute(tx.connexion())
    .await?;
    Ok(())
}
