//! Jetons d'horodatage d'un dossier et tête de chaîne qu'ils couvrent (L2-30, PR 15a ; migration
//! `0400_horodatage.sql` ; ADR 0010). Lecture et ajout seuls : un jeton est une preuve.

use integrite::Empreinte;
use time::OffsetDateTime;
use uuid::Uuid;

use super::{ErreurBase, TransactionTenant};

fn dossier(tx: &TransactionTenant) -> Result<(Uuid, Uuid), ErreurBase> {
    let dossier = tx
        .contexte()
        .id_dossier()
        .ok_or(ErreurBase::Donnee("jeton d'horodatage hors d'un dossier"))?;
    Ok((tx.contexte().id_cabinet().uuid(), dossier.uuid()))
}

/// Tête de la chaîne d'un dossier : nombre d'écritures et dernière empreinte.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TeteChaine {
    /// Nombre d'écritures chaînées.
    pub rang: i64,
    /// Empreinte de tête.
    pub empreinte: Empreinte,
}

/// Lit la tête de chaîne du dossier du contexte (sans verrou) ; absente tant qu'aucune écriture
/// n'a été validée.
///
/// # Errors
/// Contexte sans dossier, empreinte invalide, erreur de PostgreSQL.
pub async fn lire_tete(tx: &mut TransactionTenant) -> Result<Option<TeteChaine>, ErreurBase> {
    let (cabinet, dossier) = dossier(tx)?;
    let Some(l) = sqlx::query!(
        "SELECT rang, empreinte FROM chaine_tete WHERE cabinet_id = $1 AND dossier_id = $2",
        cabinet,
        dossier,
    )
    .fetch_optional(tx.connexion())
    .await?
    else {
        return Ok(None);
    };
    let octets: [u8; 32] = l
        .empreinte
        .as_slice()
        .try_into()
        .map_err(|_| ErreurBase::Donnee("empreinte de tête de longueur inattendue"))?;
    Ok(Some(TeteChaine {
        rang: l.rang,
        empreinte: Empreinte::depuis_octets(octets),
    }))
}

/// Ce que le dernier jeton obtenu pour un dossier couvre.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DernierJeton {
    /// Plus grand nombre d'écritures couvert par un jeton.
    pub nombre: i64,
    /// Instant de la plus récente obtention.
    pub obtenu_le: OffsetDateTime,
}

/// Dernier jeton du dossier du contexte.
///
/// # Errors
/// Contexte sans dossier, erreur de PostgreSQL.
pub async fn dernier_jeton(tx: &mut TransactionTenant) -> Result<Option<DernierJeton>, ErreurBase> {
    let (cabinet, dossier) = dossier(tx)?;
    let l = sqlx::query!(
        "SELECT max(nombre) AS nombre, max(obtenu_le) AS obtenu_le FROM jeton_horodatage \
         WHERE cabinet_id = $1 AND dossier_id = $2",
        cabinet,
        dossier,
    )
    .fetch_one(tx.connexion())
    .await?;
    Ok(l.nombre
        .zip(l.obtenu_le)
        .map(|(nombre, obtenu_le)| DernierJeton { nombre, obtenu_le }))
}

/// Jeton à enregistrer.
#[derive(Debug, Clone)]
pub struct NouveauJeton<'a> {
    /// Écritures couvertes.
    pub nombre: i64,
    /// Tête de chaîne horodatée.
    pub tete: Empreinte,
    /// Autorité qui l'a délivré.
    pub autorite: &'a str,
    /// Date attestée.
    pub date_jeton: OffsetDateTime,
    /// Instant d'obtention.
    pub obtenu_le: OffsetDateTime,
    /// Déclencheur (nom du sujet de la tâche).
    pub declencheur: &'a str,
    /// Jeton RFC 3161 (DER).
    pub der: &'a [u8],
}

/// Enregistre un jeton (déjà vérifié par `integrite`) et rend son identifiant.
///
/// # Errors
/// Contexte sans dossier, erreur de PostgreSQL.
pub async fn inserer_jeton(
    tx: &mut TransactionTenant,
    jeton: &NouveauJeton<'_>,
) -> Result<Uuid, ErreurBase> {
    let (cabinet, dossier) = dossier(tx)?;
    let id = Uuid::new_v4();
    sqlx::query!(
        "INSERT INTO jeton_horodatage (cabinet_id, dossier_id, id, nombre, tete, autorite, \
                                       date_jeton, obtenu_le, declencheur, jeton) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
        cabinet,
        dossier,
        id,
        jeton.nombre,
        jeton.tete.octets().as_slice(),
        jeton.autorite,
        jeton.date_jeton,
        jeton.obtenu_le,
        jeton.declencheur,
        jeton.der,
    )
    .execute(tx.connexion())
    .await?;
    Ok(id)
}
