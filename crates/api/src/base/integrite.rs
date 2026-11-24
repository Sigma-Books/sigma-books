//! Lectures de l'intégrité d'un dossier : écritures validées pour la vérification et l'export, jetons
//! d'horodatage, plus ancienne tâche d'horodatage en attente (L2-31, PR 15b). Lecture seule :
//! les écritures validées et les jetons sont en ajout seul.
//!
//! Les fonctions reçoivent la transaction de **lecture** à instantané unique
//! ([`super::ouvrir_transaction_lecture`]) de l'appelant : tête, écritures et jetons y sont
//! d'un même état.

use integrite::Empreinte;
use time::OffsetDateTime;
use uuid::Uuid;

use super::brouillard::decimal_canonique;
use super::{ErreurBase, TransactionTenant};
use crate::verification::{JetonStocke, LigneChaine};

/// Nombre maximal de jetons lus pour un dossier (celui d'un export : `integrite::export`).
pub const JETONS_MAX: i64 = 10_000;

fn dossier(tx: &TransactionTenant) -> Result<(Uuid, Uuid), ErreurBase> {
    let dossier = tx.contexte().id_dossier().ok_or(ErreurBase::Donnee(
        "intégrité hors du contexte d'un dossier",
    ))?;
    Ok((tx.contexte().id_cabinet().uuid(), dossier.uuid()))
}

fn empreinte(octets: &[u8]) -> Result<Empreinte, ErreurBase> {
    let tableau: [u8; 32] = octets
        .try_into()
        .map_err(|_| ErreurBase::Donnee("empreinte de longueur inattendue"))?;
    Ok(Empreinte::depuis_octets(tableau))
}

/// Écritures du dossier de rang supérieur à `apres`, dans l'ordre des rangs, au plus `limite`.
///
/// # Errors
/// Contexte sans dossier, donnée incohérente, erreur de PostgreSQL.
pub async fn page_chaine(
    tx: &mut TransactionTenant,
    apres: i64,
    limite: i64,
) -> Result<Vec<LigneChaine>, ErreurBase> {
    let (cabinet, dossier) = dossier(tx)?;
    let lignes = sqlx::query!(
        "SELECT id, rang, numero, date_comptable, date_validation, date_valeur_comptable, \
                libelle, format, contenu_canonique, empreinte, empreinte_precedente \
         FROM ecriture_validee \
         WHERE cabinet_id = $1 AND dossier_id = $2 AND rang > $3 \
         ORDER BY rang LIMIT $4",
        cabinet,
        dossier,
        apres,
        limite,
    )
    .fetch_all(tx.connexion())
    .await?;
    let ids: Vec<Uuid> = lignes.iter().map(|l| l.id).collect();
    let details = sqlx::query!(
        r#"SELECT ecriture_id, compte_numero, sens, montant::text AS "montant!"
           FROM ecriture_validee_ligne
           WHERE cabinet_id = $1 AND dossier_id = $2 AND ecriture_id = ANY($3)"#,
        cabinet,
        dossier,
        &ids,
    )
    .fetch_all(tx.connexion())
    .await?;
    lignes
        .into_iter()
        .map(|l| {
            let propres = details
                .iter()
                .filter(|d| d.ecriture_id == l.id)
                .map(|d| {
                    let sens = if d.sens == "debit" { "D" } else { "C" };
                    format!(
                        "{}:{sens}:{}",
                        d.compte_numero,
                        decimal_canonique(&d.montant)
                    )
                })
                .collect();
            Ok(LigneChaine {
                rang: l.rang,
                numero: l.numero,
                date: l.date_comptable,
                date_validation: l.date_validation,
                date_valeur_comptable: l.date_valeur_comptable,
                libelle: l.libelle,
                format: l.format,
                contenu: l.contenu_canonique,
                empreinte: empreinte(&l.empreinte)?,
                precedente: empreinte(&l.empreinte_precedente)?,
                lignes: propres,
            })
        })
        .collect()
}

/// Vrai si une écriture du dossier est au format canonique v3 (étiquette fiscale) : l'export
/// est alors en v4.
///
/// # Errors
/// Contexte sans dossier, erreur de PostgreSQL.
pub async fn a_une_etiquette(tx: &mut TransactionTenant) -> Result<bool, ErreurBase> {
    let (cabinet, dossier) = dossier(tx)?;
    Ok(sqlx::query_scalar!(
        r#"SELECT EXISTS (SELECT 1 FROM ecriture_validee
                          WHERE cabinet_id = $1 AND dossier_id = $2 AND format = 3) AS "oui!""#,
        cabinet,
        dossier,
    )
    .fetch_one(tx.connexion())
    .await?)
}

/// Jetons du dossier, dans l'ordre de la chaîne couverte puis de leur obtention (au plus
/// [`JETONS_MAX`] : au-delà, les plus récents).
///
/// # Errors
/// Contexte sans dossier, donnée incohérente, erreur de PostgreSQL.
pub async fn jetons(tx: &mut TransactionTenant) -> Result<Vec<JetonStocke>, ErreurBase> {
    let (cabinet, dossier) = dossier(tx)?;
    let lignes = sqlx::query!(
        "SELECT id, nombre, tete, autorite, date_jeton, obtenu_le, declencheur, jeton \
         FROM (SELECT * FROM jeton_horodatage WHERE cabinet_id = $1 AND dossier_id = $2 \
               ORDER BY nombre DESC, obtenu_le DESC, id DESC LIMIT $3) t \
         ORDER BY nombre, obtenu_le, id",
        cabinet,
        dossier,
        JETONS_MAX,
    )
    .fetch_all(tx.connexion())
    .await?;
    lignes
        .into_iter()
        .map(|l| {
            Ok(JetonStocke {
                id: l.id,
                nombre: l.nombre,
                tete: empreinte(&l.tete)?,
                autorite: l.autorite,
                date_jeton: l.date_jeton,
                obtenu_le: l.obtenu_le,
                declencheur: l.declencheur,
                der: l.jeton,
            })
        })
        .collect()
}

/// Jeton d'horodatage sans son contenu DER : ce que montrent l'état et la liste (le DER, jusqu'à
/// 64 Kio par jeton, n'est lu que pour l'export et la vérification).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResumeJeton {
    /// Identifiant.
    pub id: Uuid,
    /// Nombre d'écritures couvertes.
    pub nombre: i64,
    /// Tête de chaîne horodatée.
    pub tete: Empreinte,
    /// Autorité.
    pub autorite: String,
    /// Date attestée.
    pub date_jeton: OffsetDateTime,
    /// Instant d'obtention.
    pub obtenu_le: OffsetDateTime,
    /// Déclencheur.
    pub declencheur: String,
}

/// Jetons du dossier **sans leur DER**, dans l'ordre de la chaîne couverte puis de leur
/// obtention (au plus [`JETONS_MAX`] : au-delà, les plus récents).
///
/// # Errors
/// Contexte sans dossier, donnée incohérente, erreur de PostgreSQL.
pub async fn resumes_jetons(tx: &mut TransactionTenant) -> Result<Vec<ResumeJeton>, ErreurBase> {
    let (cabinet, dossier) = dossier(tx)?;
    let lignes = sqlx::query!(
        "SELECT id, nombre, tete, autorite, date_jeton, obtenu_le, declencheur \
         FROM (SELECT id, nombre, tete, autorite, date_jeton, obtenu_le, declencheur \
               FROM jeton_horodatage WHERE cabinet_id = $1 AND dossier_id = $2 \
               ORDER BY nombre DESC, obtenu_le DESC, id DESC LIMIT $3) t \
         ORDER BY nombre, obtenu_le, id",
        cabinet,
        dossier,
        JETONS_MAX,
    )
    .fetch_all(tx.connexion())
    .await?;
    lignes
        .into_iter()
        .map(|l| {
            Ok(ResumeJeton {
                id: l.id,
                nombre: l.nombre,
                tete: empreinte(&l.tete)?,
                autorite: l.autorite,
                date_jeton: l.date_jeton,
                obtenu_le: l.obtenu_le,
                declencheur: l.declencheur,
            })
        })
        .collect()
}

/// Dernier jeton du dossier (le plus de chaîne couverte, puis le plus récent), sans son DER.
///
/// # Errors
/// Contexte sans dossier, donnée incohérente, erreur de PostgreSQL.
pub async fn dernier_resume(tx: &mut TransactionTenant) -> Result<Option<ResumeJeton>, ErreurBase> {
    let (cabinet, dossier) = dossier(tx)?;
    let Some(l) = sqlx::query!(
        "SELECT id, nombre, tete, autorite, date_jeton, obtenu_le, declencheur \
         FROM jeton_horodatage WHERE cabinet_id = $1 AND dossier_id = $2 \
         ORDER BY nombre DESC, obtenu_le DESC, id DESC LIMIT 1",
        cabinet,
        dossier,
    )
    .fetch_optional(tx.connexion())
    .await?
    else {
        return Ok(None);
    };
    Ok(Some(ResumeJeton {
        id: l.id,
        nombre: l.nombre,
        tete: empreinte(&l.tete)?,
        autorite: l.autorite,
        date_jeton: l.date_jeton,
        obtenu_le: l.obtenu_le,
        declencheur: l.declencheur,
    }))
}

/// Nombre de jetons du dossier.
///
/// # Errors
/// Contexte sans dossier, erreur de PostgreSQL.
pub async fn compter_jetons(tx: &mut TransactionTenant) -> Result<i64, ErreurBase> {
    let (cabinet, dossier) = dossier(tx)?;
    Ok(sqlx::query_scalar!(
        r#"SELECT count(*) AS "n!" FROM jeton_horodatage WHERE cabinet_id = $1 AND dossier_id = $2"#,
        cabinet,
        dossier,
    )
    .fetch_one(tx.connexion())
    .await?)
}

/// Création de la plus ancienne tâche d'horodatage non terminée du dossier : une écriture non
/// couverte par un jeton existe depuis au moins cet instant.
///
/// # Errors
/// Contexte sans dossier, erreur de PostgreSQL.
pub async fn horodatage_en_attente_depuis(
    tx: &mut TransactionTenant,
) -> Result<Option<OffsetDateTime>, ErreurBase> {
    let (cabinet, dossier) = dossier(tx)?;
    Ok(sqlx::query_scalar!(
        "SELECT min(cree_le) FROM tache \
         WHERE nature = 'horodatage' AND etat <> 'terminee' \
           AND cabinet_cible = $1 AND dossier_cible = $2",
        cabinet,
        dossier,
    )
    .fetch_one(tx.connexion())
    .await?)
}
