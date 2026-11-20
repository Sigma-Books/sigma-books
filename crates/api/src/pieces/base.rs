//! Accès à la table `piece` (migration `0300_pieces.sql`).
//!
//! Niveau « requête » de l'isolation (ADR 0011) : chaque requête filtre par le cabinet **et** le
//! dossier du contexte de la [`TransactionTenant`], en plus de la RLS. Sans dossier en contexte,
//! aucune fonction ne lit ni n'écrit rien ([`ErreurBase::Donnee`]). Une pièce d'un autre dossier
//! ou d'un autre cabinet est une pièce absente (`None`). La recherche des doublons ne regarde
//! que le dossier du contexte (spec §2 principe 12, §9.3 ; F3.5).

use integrite::Empreinte;
use time::OffsetDateTime;
use uuid::Uuid;

use super::{CleObjet, Nature, TypePiece};
use crate::base::{ErreurBase, IdCabinet, IdDossier, IdUtilisateur, TransactionTenant};

/// Une pièce, telle que relue en base.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Piece {
    /// Identifiant.
    pub id: Uuid,
    /// Nature.
    pub nature: Nature,
    /// Type reconnu.
    pub type_piece: TypePiece,
    /// Taille, en octets.
    pub taille: u64,
    /// SHA-256 du contenu, calculée par le serveur au dépôt.
    pub empreinte: Empreinte,
    /// Nom de fichier d'origine (métadonnée).
    pub nom_origine: Option<String>,
    /// Clé de l'objet.
    pub cle: CleObjet,
    /// Version de l'objet écrite au dépôt, seule lue ensuite.
    pub version: String,
    /// Fin de la rétention posée sur l'objet.
    pub retenue_jusqu_au: OffsetDateTime,
    /// Déposant.
    pub deposee_par: IdUtilisateur,
    /// Instant du dépôt.
    pub deposee_le: OffsetDateTime,
    /// Écart (dépôt erroné) : instant, auteur et motif.
    pub ecart: Option<Ecart>,
}

/// Écart d'une pièce déposée par erreur.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ecart {
    /// Instant.
    pub le: OffsetDateTime,
    /// Auteur.
    pub par: IdUtilisateur,
    /// Motif.
    pub motif: String,
}

/// Pièce à inscrire, une fois son objet écrit.
#[derive(Debug, Clone)]
pub struct NouvellePiece {
    /// Identifiant (UUID v4 tiré par `api`).
    pub id: Uuid,
    /// Nature.
    pub nature: Nature,
    /// Type reconnu.
    pub type_piece: TypePiece,
    /// Taille, en octets.
    pub taille: u64,
    /// SHA-256 du contenu.
    pub empreinte: Empreinte,
    /// Nom de fichier d'origine.
    pub nom_origine: Option<String>,
    /// Clé de l'objet.
    pub cle: CleObjet,
    /// Version écrite.
    pub version: String,
    /// Fin de la rétention.
    pub retenue_jusqu_au: OffsetDateTime,
    /// Instant du dépôt.
    pub deposee_le: OffsetDateTime,
}

/// Cabinet et dossier du contexte ; sans dossier, rien n'est lu.
fn tenant(tx: &TransactionTenant) -> Result<(IdCabinet, IdDossier), ErreurBase> {
    let contexte = tx.contexte();
    let dossier = contexte
        .id_dossier()
        .ok_or(ErreurBase::Donnee("pièce hors du contexte d'un dossier"))?;
    Ok((contexte.id_cabinet(), dossier))
}

/// Ligne brute de la table.
struct Ligne {
    id: Uuid,
    nature: String,
    type_contenu: String,
    taille: i64,
    empreinte: Vec<u8>,
    nom_origine: Option<String>,
    cle_objet: String,
    version_objet: String,
    retenue_jusqu_au: OffsetDateTime,
    deposee_par: Uuid,
    deposee_le: OffsetDateTime,
    ecartee_le: Option<OffsetDateTime>,
    ecartee_par: Option<Uuid>,
    motif_ecart: Option<String>,
}

impl Ligne {
    fn piece(self, cabinet: IdCabinet, dossier: IdDossier) -> Result<Piece, ErreurBase> {
        let incoherente = ErreurBase::Donnee("pièce incohérente");
        let empreinte: [u8; 32] = self
            .empreinte
            .try_into()
            .map_err(|_| ErreurBase::Donnee("empreinte de pièce"))?;
        let ecart = match (self.ecartee_le, self.ecartee_par, self.motif_ecart) {
            (None, None, None) => None,
            (Some(le), Some(par), Some(motif)) => Some(Ecart {
                le,
                par: IdUtilisateur::depuis_uuid(par),
                motif,
            }),
            _ => return Err(incoherente),
        };
        Ok(Piece {
            id: self.id,
            nature: Nature::depuis_nom(&self.nature).ok_or(ErreurBase::Donnee("nature"))?,
            type_piece: TypePiece::depuis_mime(&self.type_contenu)
                .ok_or(ErreurBase::Donnee("type de pièce"))?,
            taille: u64::try_from(self.taille).map_err(|_| ErreurBase::Donnee("taille"))?,
            empreinte: Empreinte::depuis_octets(empreinte),
            nom_origine: self.nom_origine,
            cle: CleObjet::relue(&self.cle_objet, cabinet, dossier)
                .ok_or(ErreurBase::Donnee("clé d'objet"))?,
            version: self.version_objet,
            retenue_jusqu_au: self.retenue_jusqu_au,
            deposee_par: IdUtilisateur::depuis_uuid(self.deposee_par),
            deposee_le: self.deposee_le,
            ecart,
        })
    }
}

/// Inscrit une pièce dans le dossier du contexte, déposée par l'utilisateur du contexte.
///
/// # Errors
/// Hors du contexte d'un dossier, ou erreur de PostgreSQL (contrainte refusée).
pub async fn inserer_piece(
    tx: &mut TransactionTenant,
    piece: &NouvellePiece,
) -> Result<(), ErreurBase> {
    let (cabinet, dossier) = tenant(tx)?;
    let deposant = tx.contexte().id_utilisateur().uuid();
    let taille = i64::try_from(piece.taille).map_err(|_| ErreurBase::Donnee("taille"))?;
    sqlx::query!(
        r#"
        INSERT INTO piece (cabinet_id, dossier_id, id, nature, type_contenu, taille, empreinte,
                           nom_origine, cle_objet, version_objet, retenue_jusqu_au,
                           deposee_par, deposee_le)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
        "#,
        cabinet.uuid(),
        dossier.uuid(),
        piece.id,
        piece.nature.nom(),
        piece.type_piece.mime(),
        taille,
        piece.empreinte.octets().as_slice(),
        piece.nom_origine.as_deref(),
        piece.cle.as_str(),
        piece.version,
        piece.retenue_jusqu_au,
        deposant,
        piece.deposee_le,
    )
    .execute(tx.connexion())
    .await?;
    Ok(())
}

/// Une pièce du dossier du contexte, écartée ou non.
///
/// # Errors
/// Hors du contexte d'un dossier, ou erreur de PostgreSQL.
pub async fn lire_piece(tx: &mut TransactionTenant, id: Uuid) -> Result<Option<Piece>, ErreurBase> {
    let (cabinet, dossier) = tenant(tx)?;
    let ligne = sqlx::query_as!(
        Ligne,
        r#"
        SELECT id, nature, type_contenu, taille, empreinte, nom_origine, cle_objet,
               version_objet, retenue_jusqu_au, deposee_par, deposee_le, ecartee_le,
               ecartee_par, motif_ecart
        FROM piece
        WHERE cabinet_id = $1 AND dossier_id = $2 AND id = $3
        "#,
        cabinet.uuid(),
        dossier.uuid(),
        id,
    )
    .fetch_optional(tx.connexion())
    .await?;
    ligne.map(|l| l.piece(cabinet, dossier)).transpose()
}

/// Pièces non écartées du dossier du contexte, plus récentes d'abord, après la pièce `apres`
/// (pagination par curseur), au plus `limite`.
///
/// # Errors
/// Hors du contexte d'un dossier, ou erreur de PostgreSQL.
pub async fn lister_pieces(
    tx: &mut TransactionTenant,
    apres: Option<Uuid>,
    limite: u32,
) -> Result<Vec<Piece>, ErreurBase> {
    let (cabinet, dossier) = tenant(tx)?;
    let lignes = sqlx::query_as!(
        Ligne,
        r#"
        SELECT p.id, p.nature, p.type_contenu, p.taille, p.empreinte, p.nom_origine,
               p.cle_objet, p.version_objet, p.retenue_jusqu_au, p.deposee_par, p.deposee_le,
               p.ecartee_le, p.ecartee_par, p.motif_ecart
        FROM piece p
        WHERE p.cabinet_id = $1 AND p.dossier_id = $2 AND p.ecartee_le IS NULL
          AND ($3::uuid IS NULL OR (p.deposee_le, p.id) < (
                SELECT c.deposee_le, c.id FROM piece c
                WHERE c.cabinet_id = $1 AND c.dossier_id = $2 AND c.id = $3))
        ORDER BY p.deposee_le DESC, p.id DESC
        LIMIT $4
        "#,
        cabinet.uuid(),
        dossier.uuid(),
        apres,
        i64::from(limite),
    )
    .fetch_all(tx.connexion())
    .await?;
    lignes
        .into_iter()
        .map(|l| l.piece(cabinet, dossier))
        .collect()
}

/// Verrouille la ligne du dossier du contexte (`FOR NO KEY UPDATE`) jusqu'à la fin de la
/// transaction : les dépôts d'un même dossier se succèdent sur la recherche des doublons, si
/// bien que de deux dépôts concurrents du même fichier, un seul passe sans avertissement
/// (spec §9.3). Pris **avant** la recherche et donc avant l'écriture de l'objet : aucun objet
/// WORM n'est écrit pour un dépôt qui serait ensuite signalé en doublon. Verrou de ligne sous
/// RLS, borné au dossier : aucun espace de clés partagé entre tenants (F3.5). Ordre des verrous
/// (ADR 0014) : celui de l'action, puis les têtes d'audit. `false` si le dossier est absent.
///
/// # Errors
/// Hors du contexte d'un dossier, ou erreur de PostgreSQL.
pub async fn verrouiller_dossier(tx: &mut TransactionTenant) -> Result<bool, ErreurBase> {
    let (cabinet, dossier) = tenant(tx)?;
    let ligne = sqlx::query_scalar!(
        "SELECT id FROM dossier WHERE cabinet_id = $1 AND id = $2 FOR NO KEY UPDATE",
        cabinet.uuid(),
        dossier.uuid(),
    )
    .fetch_optional(tx.connexion())
    .await?;
    Ok(ligne.is_some())
}

/// Pièces non écartées **du dossier du contexte** de même empreinte (doublons, spec §9.3).
/// Jamais d'un autre dossier ni d'un autre cabinet (F3.5).
///
/// # Errors
/// Hors du contexte d'un dossier, ou erreur de PostgreSQL.
pub async fn pieces_de_meme_empreinte(
    tx: &mut TransactionTenant,
    empreinte: &Empreinte,
) -> Result<Vec<Piece>, ErreurBase> {
    let (cabinet, dossier) = tenant(tx)?;
    let lignes = sqlx::query_as!(
        Ligne,
        r#"
        SELECT id, nature, type_contenu, taille, empreinte, nom_origine, cle_objet,
               version_objet, retenue_jusqu_au, deposee_par, deposee_le, ecartee_le,
               ecartee_par, motif_ecart
        FROM piece
        WHERE cabinet_id = $1 AND dossier_id = $2 AND empreinte = $3 AND ecartee_le IS NULL
        ORDER BY deposee_le, id
        LIMIT 20
        "#,
        cabinet.uuid(),
        dossier.uuid(),
        empreinte.octets().as_slice(),
    )
    .fetch_all(tx.connexion())
    .await?;
    lignes
        .into_iter()
        .map(|l| l.piece(cabinet, dossier))
        .collect()
}

/// Issue d'un écart.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IssueEcart {
    /// Pièce écartée.
    Ecartee(Box<Piece>),
    /// Pièce absente du dossier, ou déjà écartée.
    Absente,
    /// Pièce liée à un brouillon : elle ne s'écarte pas (une pièce liée à une écriture validée
    /// non plus, PR 13b).
    Liee,
}

/// Écarte une pièce non écartée du dossier du contexte (dépôt erroné), au nom de l'utilisateur
/// du contexte. La pièce reste stockée sous verrou jusqu'au terme de sa rétention.
///
/// La pièce est d'abord verrouillée `FOR UPDATE` : un lien concurrent (qui la tient `FOR SHARE`)
/// est attendu, puis vu ; les liens sont comptés **après** ce verrou. Une pièce liée à un
/// brouillon n'est pas écartée ([`IssueEcart::Liee`]).
///
/// # Errors
/// Hors du contexte d'un dossier, ou erreur de PostgreSQL.
pub async fn ecarter_piece(
    tx: &mut TransactionTenant,
    id: Uuid,
    motif: &str,
    instant: OffsetDateTime,
) -> Result<IssueEcart, ErreurBase> {
    let (cabinet, dossier) = tenant(tx)?;
    let auteur = tx.contexte().id_utilisateur().uuid();
    let verrou = sqlx::query_scalar!(
        "SELECT id FROM piece WHERE cabinet_id = $1 AND dossier_id = $2 AND id = $3 \
         AND ecartee_le IS NULL FOR UPDATE",
        cabinet.uuid(),
        dossier.uuid(),
        id,
    )
    .fetch_optional(tx.connexion())
    .await?;
    if verrou.is_none() {
        return Ok(IssueEcart::Absente);
    }
    if crate::base::brouillard::brouillons_lies_a_la_piece(tx, id).await? > 0 {
        return Ok(IssueEcart::Liee);
    }
    let ligne = sqlx::query_as!(
        Ligne,
        r#"
        UPDATE piece
        SET ecartee_le = $4, ecartee_par = $5, motif_ecart = $6
        WHERE cabinet_id = $1 AND dossier_id = $2 AND id = $3 AND ecartee_le IS NULL
        RETURNING id, nature, type_contenu, taille, empreinte, nom_origine, cle_objet,
                  version_objet, retenue_jusqu_au, deposee_par, deposee_le, ecartee_le,
                  ecartee_par, motif_ecart
        "#,
        cabinet.uuid(),
        dossier.uuid(),
        id,
        instant,
        auteur,
        motif,
    )
    .fetch_optional(tx.connexion())
    .await?;
    Ok(match ligne {
        Some(l) => IssueEcart::Ecartee(Box::new(l.piece(cabinet, dossier)?)),
        None => IssueEcart::Absente,
    })
}
