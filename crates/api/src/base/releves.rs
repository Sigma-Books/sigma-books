//! Profils d'import et dépôts de relevés en base (L3-22, PR 12a de `docs/lots/L3.md` ;
//! migrations `0561_profil_releve.sql` et `0562_depot_releve.sql` ; ADR 0034 points 4 à 6).
//!
//! Niveau « requête » de l'isolation (ADR 0011) : chaque requête filtre par le cabinet et le
//! dossier du contexte, en plus de la RLS ; un identifiant d'un autre dossier ne rend rien. Les
//! deux tables sont en **ajout seul** : aucune fonction ne modifie ni ne supprime une ligne.

use time::OffsetDateTime;
use uuid::Uuid;

use super::{ErreurBase, TransactionTenant};

fn contexte(tx: &TransactionTenant) -> Result<(Uuid, Uuid), ErreurBase> {
    let dossier = tx
        .contexte()
        .id_dossier()
        .ok_or(ErreurBase::Donnee("relevé hors du contexte d'un dossier"))?;
    Ok((tx.contexte().id_cabinet().uuid(), dossier.uuid()))
}

/// Version d'un profil d'import, relue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfilLu {
    /// Identifiant du profil (commun à ses versions).
    pub id: Uuid,
    /// Version, à partir de 1.
    pub version: i32,
    /// Nom.
    pub nom: String,
    /// Forme JSON canonique du profil validé.
    pub definition: String,
    /// SHA-256 de la définition.
    pub empreinte: Vec<u8>,
    /// Auteur de la version.
    pub cree_par: Uuid,
    /// Instant de la version.
    pub cree_le: OffsetDateTime,
}

/// Nouvelle version d'un profil (la première, ou la suivante).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NouvelleVersion<'a> {
    /// Identifiant du profil.
    pub id: Uuid,
    /// Version.
    pub version: i32,
    /// Nom.
    pub nom: &'a str,
    /// Forme JSON canonique.
    pub definition: &'a str,
    /// SHA-256 de la définition.
    pub empreinte: &'a [u8; 32],
    /// Instant.
    pub cree_le: OffsetDateTime,
}

/// Nombre de profils (distincts, toutes versions confondues) du dossier du contexte.
///
/// # Errors
/// Contexte sans dossier, erreur de PostgreSQL.
pub async fn nombre_de_profils(tx: &mut TransactionTenant) -> Result<i64, ErreurBase> {
    let (cabinet, dossier) = contexte(tx)?;
    let n = sqlx::query_scalar!(
        r#"SELECT count(DISTINCT id) AS "n!" FROM profil_releve
           WHERE cabinet_id = $1 AND dossier_id = $2"#,
        cabinet,
        dossier,
    )
    .fetch_one(tx.connexion())
    .await?;
    Ok(n)
}

/// Inscrit une version de profil, créée par l'utilisateur du contexte.
///
/// # Errors
/// Contexte sans dossier, erreur de PostgreSQL (version déjà prise, version précédente absente).
pub async fn inserer_version(
    tx: &mut TransactionTenant,
    v: &NouvelleVersion<'_>,
) -> Result<(), ErreurBase> {
    let (cabinet, dossier) = contexte(tx)?;
    let auteur = tx.contexte().id_utilisateur().uuid();
    sqlx::query!(
        r#"
        INSERT INTO profil_releve (cabinet_id, dossier_id, id, version, nom, definition,
                                   empreinte, cree_par, cree_le)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
        "#,
        cabinet,
        dossier,
        v.id,
        v.version,
        v.nom,
        v.definition,
        v.empreinte.as_slice(),
        auteur,
        v.cree_le,
    )
    .execute(tx.connexion())
    .await?;
    Ok(())
}

/// Version `version` du profil `id` du dossier du contexte ; sans version, la dernière.
///
/// # Errors
/// Contexte sans dossier, erreur de PostgreSQL.
pub async fn lire_profil(
    tx: &mut TransactionTenant,
    id: Uuid,
    version: Option<i32>,
) -> Result<Option<ProfilLu>, ErreurBase> {
    let (cabinet, dossier) = contexte(tx)?;
    let ligne = sqlx::query_as!(
        ProfilLu,
        r#"
        SELECT id, version, nom, definition, empreinte, cree_par, cree_le
        FROM profil_releve
        WHERE cabinet_id = $1 AND dossier_id = $2 AND id = $3
          AND ($4::integer IS NULL OR version = $4)
        ORDER BY version DESC
        LIMIT 1
        "#,
        cabinet,
        dossier,
        id,
        version,
    )
    .fetch_optional(tx.connexion())
    .await?;
    Ok(ligne)
}

/// Toutes les versions du profil `id`, de la première à la dernière.
///
/// # Errors
/// Contexte sans dossier, erreur de PostgreSQL.
pub async fn versions(tx: &mut TransactionTenant, id: Uuid) -> Result<Vec<ProfilLu>, ErreurBase> {
    let (cabinet, dossier) = contexte(tx)?;
    Ok(sqlx::query_as!(
        ProfilLu,
        r#"
        SELECT id, version, nom, definition, empreinte, cree_par, cree_le
        FROM profil_releve
        WHERE cabinet_id = $1 AND dossier_id = $2 AND id = $3
        ORDER BY version
        "#,
        cabinet,
        dossier,
        id,
    )
    .fetch_all(tx.connexion())
    .await?)
}

/// Dernière version de chaque profil du dossier du contexte, par nom puis identifiant, `limite`
/// au plus, après le profil `apres` (curseur : son nom et son identifiant, dernière version).
///
/// # Errors
/// Contexte sans dossier, erreur de PostgreSQL.
pub async fn lister_profils(
    tx: &mut TransactionTenant,
    apres: Option<Uuid>,
    limite: i64,
) -> Result<Vec<ProfilLu>, ErreurBase> {
    let (cabinet, dossier) = contexte(tx)?;
    Ok(sqlx::query_as!(
        ProfilLu,
        r#"
        WITH derniere AS (
            SELECT DISTINCT ON (id) id, version, nom, definition, empreinte, cree_par, cree_le
            FROM profil_releve
            WHERE cabinet_id = $1 AND dossier_id = $2
            ORDER BY id, version DESC
        )
        SELECT d.id AS "id!", d.version AS "version!", d.nom AS "nom!",
               d.definition AS "definition!", d.empreinte AS "empreinte!",
               d.cree_par AS "cree_par!", d.cree_le AS "cree_le!"
        FROM derniere d
        WHERE $3::uuid IS NULL
           OR (d.nom, d.id) > (SELECT c.nom, c.id FROM derniere c WHERE c.id = $3)
        ORDER BY d.nom, d.id
        LIMIT $4
        "#,
        cabinet,
        dossier,
        apres,
        limite,
    )
    .fetch_all(tx.connexion())
    .await?)
}

/// Dépôt de relevé relu, avec l'empreinte, la taille et le nom de sa pièce.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DepotLu {
    /// Identifiant.
    pub id: Uuid,
    /// Compte de trésorerie visé.
    pub compte_tresorerie_id: Uuid,
    /// Pièce du fichier.
    pub piece_id: Uuid,
    /// Statut probant (`importe_non_certifie`).
    pub statut_probant: String,
    /// SHA-256 du fichier (celle de la pièce).
    pub empreinte: Vec<u8>,
    /// Taille du fichier, en octets.
    pub taille: i64,
    /// Nom d'origine du fichier (métadonnée).
    pub nom_origine: Option<String>,
    /// Pièce écartée (dépôt erroné, ADR 0015).
    pub ecartee: bool,
    /// Auteur du dépôt.
    pub depose_par: Uuid,
    /// Instant du dépôt.
    pub depose_le: OffsetDateTime,
}

/// Nouveau dépôt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NouveauDepot {
    /// Identifiant.
    pub id: Uuid,
    /// Compte de trésorerie visé.
    pub compte_tresorerie_id: Uuid,
    /// Pièce du fichier, déjà inscrite.
    pub piece_id: Uuid,
    /// Instant.
    pub depose_le: OffsetDateTime,
}

/// Inscrit un dépôt, par l'utilisateur du contexte.
///
/// # Errors
/// Contexte sans dossier, erreur de PostgreSQL (pièce qui n'est pas un texte externe, compte de
/// trésorerie absent).
pub async fn inserer_depot(tx: &mut TransactionTenant, d: &NouveauDepot) -> Result<(), ErreurBase> {
    let (cabinet, dossier) = contexte(tx)?;
    let auteur = tx.contexte().id_utilisateur().uuid();
    sqlx::query!(
        r#"
        INSERT INTO depot_releve (cabinet_id, dossier_id, id, compte_tresorerie_id, piece_id,
                                  depose_par, depose_le)
        VALUES ($1, $2, $3, $4, $5, $6, $7)
        "#,
        cabinet,
        dossier,
        d.id,
        d.compte_tresorerie_id,
        d.piece_id,
        auteur,
        d.depose_le,
    )
    .execute(tx.connexion())
    .await?;
    Ok(())
}

/// Dépôt `id` du dossier du contexte.
///
/// # Errors
/// Contexte sans dossier, erreur de PostgreSQL.
pub async fn lire_depot(
    tx: &mut TransactionTenant,
    id: Uuid,
) -> Result<Option<DepotLu>, ErreurBase> {
    let (cabinet, dossier) = contexte(tx)?;
    Ok(sqlx::query_as!(
        DepotLu,
        r#"
        SELECT r.id, r.compte_tresorerie_id, r.piece_id, r.statut_probant, p.empreinte,
               p.taille, p.nom_origine, (p.ecartee_le IS NOT NULL) AS "ecartee!",
               r.depose_par, r.depose_le
        FROM depot_releve r
        JOIN piece p ON p.cabinet_id = r.cabinet_id AND p.dossier_id = r.dossier_id
                    AND p.id = r.piece_id
        WHERE r.cabinet_id = $1 AND r.dossier_id = $2 AND r.id = $3
        "#,
        cabinet,
        dossier,
        id,
    )
    .fetch_optional(tx.connexion())
    .await?)
}

/// Dépôts du dossier du contexte, plus récents d'abord, `limite` au plus ; d'un seul compte de
/// trésorerie si `compte` est donné.
///
/// # Errors
/// Contexte sans dossier, erreur de PostgreSQL.
pub async fn lister_depots(
    tx: &mut TransactionTenant,
    compte: Option<Uuid>,
    limite: i64,
) -> Result<Vec<DepotLu>, ErreurBase> {
    let (cabinet, dossier) = contexte(tx)?;
    Ok(sqlx::query_as!(
        DepotLu,
        r#"
        SELECT r.id, r.compte_tresorerie_id, r.piece_id, r.statut_probant, p.empreinte,
               p.taille, p.nom_origine, (p.ecartee_le IS NOT NULL) AS "ecartee!",
               r.depose_par, r.depose_le
        FROM depot_releve r
        JOIN piece p ON p.cabinet_id = r.cabinet_id AND p.dossier_id = r.dossier_id
                    AND p.id = r.piece_id
        WHERE r.cabinet_id = $1 AND r.dossier_id = $2
          AND ($3::uuid IS NULL OR r.compte_tresorerie_id = $3)
        ORDER BY r.depose_le DESC, r.id DESC
        LIMIT $4
        "#,
        cabinet,
        dossier,
        compte,
        limite,
    )
    .fetch_all(tx.connexion())
    .await?)
}

/// Dépôts **du dossier du contexte** dont la pièce, non écartée, a cette empreinte (même fichier
/// à l'octet). Jamais d'un autre dossier ni d'un autre cabinet (F3.5).
///
/// # Errors
/// Contexte sans dossier, erreur de PostgreSQL.
pub async fn depots_de_meme_empreinte(
    tx: &mut TransactionTenant,
    empreinte: &[u8; 32],
) -> Result<Vec<DepotLu>, ErreurBase> {
    let (cabinet, dossier) = contexte(tx)?;
    Ok(sqlx::query_as!(
        DepotLu,
        r#"
        SELECT r.id, r.compte_tresorerie_id, r.piece_id, r.statut_probant, p.empreinte,
               p.taille, p.nom_origine, (p.ecartee_le IS NOT NULL) AS "ecartee!",
               r.depose_par, r.depose_le
        FROM depot_releve r
        JOIN piece p ON p.cabinet_id = r.cabinet_id AND p.dossier_id = r.dossier_id
                    AND p.id = r.piece_id
        WHERE r.cabinet_id = $1 AND r.dossier_id = $2 AND p.empreinte = $3
          AND p.ecartee_le IS NULL
        ORDER BY r.depose_le, r.id
        "#,
        cabinet,
        dossier,
        empreinte.as_slice(),
    )
    .fetch_all(tx.connexion())
    .await?)
}
