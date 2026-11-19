//! Fonctions d'accès aux tables d'organisation (cabinet, dossier).
//!
//! Niveau « requête » de l'isolation (ADR 0011) : chaque requête filtre par le contexte de la
//! [`TransactionTenant`], en plus de la RLS. Les tests l'exécutent sous un rôle qui contourne la
//! RLS : le filtre suffit seul. Une ressource d'un autre tenant est une ressource absente (`None`).
//!
//! Les fonctions `…_visible(s)` ajoutent l'affectation aux dossiers (Q5.1), que la RLS ne vérifie
//! pas (ADR 0011) : l'utilisateur du contexte ne voit que les dossiers qui lui sont affectés, sauf
//! s'il administre le cabinet. Elles reprennent la règle de la politique Cedar
//! `invariant-affectation-dossier` : si Cedar se trompait, la requête ne livrerait rien de plus.

use time::OffsetDateTime;

use super::{
    ErreurBase, IdAccesDossier, IdAppartenance, IdCabinet, IdDossier, IdUtilisateur,
    TransactionTenant,
};

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

/// Appartenance d'un utilisateur à un cabinet, avec son rôle (principal des décisions Cedar).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Appartenance {
    /// Identifiant.
    pub id: IdAppartenance,
    /// Cabinet.
    pub cabinet: IdCabinet,
    /// Utilisateur.
    pub utilisateur: IdUtilisateur,
    /// Rôle, tel qu'il est enregistré (`administrateur`, `valideur`, `collaborateur`, `auditeur`).
    pub role: String,
    /// Faux si l'appartenance est suspendue (ADR 0014).
    pub actif: bool,
}

/// Affectation d'un membre à un dossier (Q5.1), avec la délégation du droit de valider (Q5.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AccesDossier {
    /// Identifiant.
    pub id: IdAccesDossier,
    /// Dossier affecté.
    pub dossier: IdDossier,
    /// Délégation du droit de valider (Q5.2).
    pub validation_deleguee: bool,
}

/// Un dossier visible par l'utilisateur du contexte, avec son affectation éventuelle (absente
/// pour un administrateur non affecté).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DossierVisible {
    /// Le dossier.
    pub dossier: Dossier,
    /// L'affectation de l'utilisateur du contexte à ce dossier.
    pub acces: Option<AccesDossier>,
}

/// Lit l'appartenance de l'utilisateur du contexte au cabinet du contexte. Absente (`None`) si
/// l'utilisateur n'en est pas membre : le cabinet est alors une ressource d'autrui.
///
/// # Errors
/// Erreur de PostgreSQL.
pub async fn lire_appartenance(
    tx: &mut TransactionTenant,
) -> Result<Option<Appartenance>, ErreurBase> {
    let cabinet = tx.contexte().id_cabinet().uuid();
    let utilisateur = tx.contexte().id_utilisateur().uuid();
    let ligne = sqlx::query!(
        "SELECT id, cabinet_id, utilisateur_id, role, actif FROM appartenance \
         WHERE cabinet_id = $1 AND utilisateur_id = $2",
        cabinet,
        utilisateur,
    )
    .fetch_optional(tx.connexion())
    .await?;
    Ok(ligne.map(|l| Appartenance {
        id: IdAppartenance::depuis_uuid(l.id),
        cabinet: IdCabinet::depuis_uuid(l.cabinet_id),
        utilisateur: IdUtilisateur::depuis_uuid(l.utilisateur_id),
        role: l.role,
        actif: l.actif,
    }))
}

/// Lit l'affectation de l'utilisateur du contexte à un dossier du cabinet du contexte (et, dans
/// le contexte d'un dossier, à ce seul dossier).
///
/// # Errors
/// Erreur de PostgreSQL.
pub async fn lire_acces_dossier(
    tx: &mut TransactionTenant,
    dossier: IdDossier,
) -> Result<Option<AccesDossier>, ErreurBase> {
    let cabinet = tx.contexte().id_cabinet().uuid();
    let utilisateur = tx.contexte().id_utilisateur().uuid();
    let contexte = tx.contexte().id_dossier().map(IdDossier::uuid);
    let ligne = sqlx::query!(
        "SELECT id, dossier_id, validation_deleguee FROM acces_dossier \
         WHERE cabinet_id = $1 AND utilisateur_id = $2 AND dossier_id = $3 \
           AND ($4::uuid IS NULL OR dossier_id = $4)",
        cabinet,
        utilisateur,
        dossier.uuid(),
        contexte,
    )
    .fetch_optional(tx.connexion())
    .await?;
    Ok(ligne.map(|l| AccesDossier {
        id: IdAccesDossier::depuis_uuid(l.id),
        dossier: IdDossier::depuis_uuid(l.dossier_id),
        validation_deleguee: l.validation_deleguee,
    }))
}

/// Liste, par nom, les dossiers du cabinet du contexte **visibles** par l'utilisateur du
/// contexte : tous s'il administre le cabinet, sinon ceux qui lui sont affectés ; rien si son
/// appartenance est absente ou suspendue. Dans le contexte d'un dossier, ce seul dossier au plus.
///
/// # Errors
/// Erreur de PostgreSQL.
pub async fn lister_dossiers_visibles(
    tx: &mut TransactionTenant,
) -> Result<Vec<DossierVisible>, ErreurBase> {
    let cabinet = tx.contexte().id_cabinet().uuid();
    let utilisateur = tx.contexte().id_utilisateur().uuid();
    let contexte = tx.contexte().id_dossier().map(IdDossier::uuid);
    let lignes = sqlx::query!(
        r#"
        SELECT d.cabinet_id, d.id, d.nom, d.cree_le,
               ad.id AS "acces_id?", ad.validation_deleguee AS "validation_deleguee?"
        FROM dossier d
        JOIN appartenance a
          ON a.cabinet_id = d.cabinet_id AND a.utilisateur_id = $2 AND a.actif
        LEFT JOIN acces_dossier ad
          ON ad.cabinet_id = d.cabinet_id AND ad.dossier_id = d.id AND ad.utilisateur_id = $2
        WHERE d.cabinet_id = $1
          AND ($3::uuid IS NULL OR d.id = $3)
          AND (a.role = 'administrateur' OR ad.id IS NOT NULL)
        ORDER BY d.nom, d.id
        "#,
        cabinet,
        utilisateur,
        contexte,
    )
    .fetch_all(tx.connexion())
    .await?;
    Ok(lignes
        .into_iter()
        .map(|l| {
            let dossier = IdDossier::depuis_uuid(l.id);
            DossierVisible {
                acces: l.acces_id.map(|id| AccesDossier {
                    id: IdAccesDossier::depuis_uuid(id),
                    dossier,
                    validation_deleguee: l.validation_deleguee.unwrap_or(false),
                }),
                dossier: Dossier {
                    id: dossier,
                    cabinet: IdCabinet::depuis_uuid(l.cabinet_id),
                    nom: l.nom,
                    cree_le: l.cree_le,
                },
            }
        })
        .collect())
}

/// Lit un dossier du cabinet du contexte **visible** par l'utilisateur du contexte (même règle
/// que [`lister_dossiers_visibles`]). Un dossier d'autrui, non affecté ou inexistant est absent.
///
/// # Errors
/// Erreur de PostgreSQL.
pub async fn lire_dossier_visible(
    tx: &mut TransactionTenant,
    id: IdDossier,
) -> Result<Option<Dossier>, ErreurBase> {
    let cabinet = tx.contexte().id_cabinet().uuid();
    let utilisateur = tx.contexte().id_utilisateur().uuid();
    let contexte = tx.contexte().id_dossier().map(IdDossier::uuid);
    let ligne = sqlx::query!(
        r#"
        SELECT d.cabinet_id, d.id, d.nom, d.cree_le
        FROM dossier d
        JOIN appartenance a
          ON a.cabinet_id = d.cabinet_id AND a.utilisateur_id = $3 AND a.actif
        WHERE d.cabinet_id = $1 AND d.id = $2
          AND ($4::uuid IS NULL OR d.id = $4)
          AND (a.role = 'administrateur' OR EXISTS (
                SELECT 1 FROM acces_dossier ad
                WHERE ad.cabinet_id = d.cabinet_id AND ad.dossier_id = d.id
                  AND ad.utilisateur_id = $3))
        "#,
        cabinet,
        id.uuid(),
        utilisateur,
        contexte,
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

/// Lit le cabinet du contexte, s'il compte l'utilisateur du contexte parmi ses membres
/// **actifs** : sinon il est absent, comme celui d'autrui (appartenance suspendue, ADR 0014).
///
/// # Errors
/// Erreur de PostgreSQL.
pub async fn lire_cabinet(tx: &mut TransactionTenant) -> Result<Option<Cabinet>, ErreurBase> {
    let cabinet = tx.contexte().id_cabinet().uuid();
    let utilisateur = tx.contexte().id_utilisateur().uuid();
    let ligne = sqlx::query!(
        "SELECT c.id, c.nom, c.separation_taches, c.cree_le FROM cabinet c \
         WHERE c.id = $1 AND EXISTS ( \
             SELECT 1 FROM appartenance a \
             WHERE a.cabinet_id = c.id AND a.utilisateur_id = $2 AND a.actif)",
        cabinet,
        utilisateur,
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

/// Affecte l'utilisateur du contexte à un dossier du cabinet du contexte (Q5.1), sans
/// délégation : le créateur d'un dossier y est affecté (ADR 0014).
///
/// # Errors
/// Erreur de PostgreSQL (dossier ou membre d'un autre cabinet : clés composites, RLS).
pub async fn affecter_utilisateur_du_contexte(
    tx: &mut TransactionTenant,
    dossier: IdDossier,
) -> Result<IdAccesDossier, ErreurBase> {
    let cabinet = tx.contexte().id_cabinet().uuid();
    let utilisateur = tx.contexte().id_utilisateur().uuid();
    let id = IdAccesDossier::nouveau();
    sqlx::query!(
        "INSERT INTO acces_dossier (cabinet_id, dossier_id, utilisateur_id, id) \
         VALUES ($1, $2, $3, $4)",
        cabinet,
        dossier.uuid(),
        utilisateur,
        id.uuid(),
    )
    .execute(tx.connexion())
    .await?;
    Ok(id)
}

/// Noms des utilisateurs donnés qui sont membres du cabinet du contexte (actifs ou suspendus),
/// pour résoudre l'acteur d'une entrée d'audit à l'affichage et dans l'annexe d'un export
/// (ADR 0014). Un utilisateur qui n'est plus membre est absent.
///
/// # Errors
/// Erreur de PostgreSQL.
pub async fn lire_noms_membres(
    tx: &mut TransactionTenant,
    utilisateurs: &[IdUtilisateur],
) -> Result<Vec<(IdUtilisateur, String)>, ErreurBase> {
    let cabinet = tx.contexte().id_cabinet().uuid();
    let ids: Vec<uuid::Uuid> = utilisateurs.iter().map(|u| u.uuid()).collect();
    let lignes = sqlx::query!(
        "SELECT u.id, u.nom FROM utilisateur u \
         WHERE u.id = ANY($2) AND EXISTS ( \
             SELECT 1 FROM appartenance a WHERE a.cabinet_id = $1 AND a.utilisateur_id = u.id) \
         ORDER BY u.id",
        cabinet,
        &ids,
    )
    .fetch_all(tx.connexion())
    .await?;
    Ok(lignes
        .into_iter()
        .map(|l| (IdUtilisateur::depuis_uuid(l.id), l.nom))
        .collect())
}
