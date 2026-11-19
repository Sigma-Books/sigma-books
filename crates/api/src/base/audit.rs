//! Persistance du journal d'audit chaîné (ADR 0014 ; format `docs/integrite/audit-v1.md`).
//!
//! - **Écriture** ([`inscrire`]) : sous le rôle applicatif, dans la transaction de l'action. La
//!   tête de la portée est verrouillée (`SELECT … FOR UPDATE`), l'entrée sérialisée et chaînée
//!   par `integrite::audit` (pur), insérée, puis la tête avancée. Le rôle applicatif ne relit
//!   jamais les entrées : il n'en a pas le droit (F2.6).
//! - **Lecture** ([`lire_tete`], [`lire_page`]) : sous le rôle de lecture d'audit, dans une
//!   transaction au contexte du tenant (pool distinct).
//!
//! Chaque requête filtre par le contexte de la [`TransactionTenant`], en plus de la RLS : la
//! portée d'un dossier exige ce dossier en contexte.

use integrite::Empreinte;
use integrite::audit::{ChaineAudit, EntreeAudit, Evenement, Portee, Uuid};
use time::OffsetDateTime;

use super::{ErreurBase, IdDossier, TransactionTenant};

/// Portée d'une chaîne d'audit, dans le cabinet du contexte.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PorteeAudit {
    /// Chaîne du cabinet du contexte.
    Cabinet,
    /// Chaîne d'un dossier : il doit être le dossier du contexte.
    Dossier(IdDossier),
}

/// Une entrée enregistrée, telle que la lit le rôle de lecture d'audit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LigneAudit {
    /// Rang enregistré.
    pub rang: u64,
    /// Contenu canonique enregistré.
    pub contenu: String,
    /// Empreinte enregistrée.
    pub empreinte: Empreinte,
}

/// Résultat d'une inscription.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Inscription {
    /// Rang de l'entrée.
    pub rang: u64,
    /// Empreinte de l'entrée : nouvelle tête de la chaîne.
    pub empreinte: Empreinte,
}

fn uuid_audit(uuid: uuid::Uuid) -> Uuid {
    Uuid::depuis_octets(*uuid.as_bytes())
}

/// Portée au sens d'`integrite`, après contrôle du contexte : la portée d'un dossier exige ce
/// dossier en contexte.
///
/// # Errors
/// [`ErreurBase::Audit`] si le dossier n'est pas celui du contexte.
pub fn portee_integrite(tx: &TransactionTenant, portee: PorteeAudit) -> Result<Portee, ErreurBase> {
    let cabinet = uuid_audit(tx.contexte().id_cabinet().uuid());
    match portee {
        PorteeAudit::Cabinet => Ok(Portee::Cabinet { cabinet }),
        PorteeAudit::Dossier(dossier) if tx.contexte().id_dossier() == Some(dossier) => {
            Ok(Portee::Dossier {
                cabinet,
                dossier: uuid_audit(dossier.uuid()),
            })
        }
        PorteeAudit::Dossier(_) => Err(ErreurBase::Audit("dossier hors du contexte")),
    }
}

fn rang_base(rang: u64) -> Result<i64, ErreurBase> {
    i64::try_from(rang).map_err(|_| ErreurBase::Audit("rang hors limites"))
}

fn rang_lu(rang: i64) -> Result<u64, ErreurBase> {
    u64::try_from(rang).map_err(|_| ErreurBase::Audit("rang négatif"))
}

fn empreinte_lue(octets: &[u8]) -> Result<Empreinte, ErreurBase> {
    <[u8; 32]>::try_from(octets)
        .map(Empreinte::depuis_octets)
        .map_err(|_| ErreurBase::Audit("empreinte enregistrée invalide"))
}

/// Verrouille la tête de la chaîne de `portee` (`SELECT … FOR UPDATE`), en la créant au rang 0
/// sur l'empreinte de départ si la portée n'a encore aucune entrée. Les inscriptions concurrentes
/// d'une même portée attendent la fin de la transaction.
///
/// Ordre des verrous (ADR 0014) : verrouiller la tête du cabinet **avant** celle d'un dossier.
///
/// # Errors
/// Portée hors du contexte, erreur de PostgreSQL.
pub async fn verrouiller_tete(
    tx: &mut TransactionTenant,
    portee_audit: PorteeAudit,
) -> Result<ChaineAudit, ErreurBase> {
    let p = portee_integrite(tx, portee_audit)?;
    let depart = integrite::audit::genese(&p);
    let cabinet = tx.contexte().id_cabinet().uuid();
    let (rang, empreinte) = match portee_audit {
        PorteeAudit::Cabinet => {
            sqlx::query!(
                "INSERT INTO audit_tete_cabinet (cabinet_id, rang, empreinte) VALUES ($1, 0, $2) \
                 ON CONFLICT (cabinet_id) DO NOTHING",
                cabinet,
                depart.octets().as_slice(),
            )
            .execute(tx.connexion())
            .await?;
            let l = sqlx::query!(
                "SELECT rang, empreinte FROM audit_tete_cabinet WHERE cabinet_id = $1 FOR UPDATE",
                cabinet,
            )
            .fetch_one(tx.connexion())
            .await?;
            (l.rang, l.empreinte)
        }
        PorteeAudit::Dossier(dossier) => {
            sqlx::query!(
                "INSERT INTO audit_tete_dossier (cabinet_id, dossier_id, rang, empreinte) \
                 VALUES ($1, $2, 0, $3) ON CONFLICT (cabinet_id, dossier_id) DO NOTHING",
                cabinet,
                dossier.uuid(),
                depart.octets().as_slice(),
            )
            .execute(tx.connexion())
            .await?;
            let l = sqlx::query!(
                "SELECT rang, empreinte FROM audit_tete_dossier \
                 WHERE cabinet_id = $1 AND dossier_id = $2 FOR UPDATE",
                cabinet,
                dossier.uuid(),
            )
            .fetch_one(tx.connexion())
            .await?;
            (l.rang, l.empreinte)
        }
    };
    Ok(ChaineAudit::reprendre(
        p,
        empreinte_lue(&empreinte)?,
        rang_lu(rang)?,
    ))
}

/// Inscrit `evenement` à la chaîne de `portee`, dans la transaction de l'action, à `instant`
/// (UTC, tronqué à la microseconde). Si l'inscription échoue, l'appelant abandonne la
/// transaction : l'action est annulée (ADR 0014, « Même transaction »).
///
/// # Errors
/// Portée hors du contexte, entrée invalide, conflit de rang, erreur de PostgreSQL.
pub async fn inscrire(
    tx: &mut TransactionTenant,
    portee_audit: PorteeAudit,
    instant: OffsetDateTime,
    evenement: Evenement,
) -> Result<Inscription, ErreurBase> {
    let mut chaine = verrouiller_tete(tx, portee_audit).await?;
    let instant = instant
        .to_offset(time::UtcOffset::UTC)
        .replace_nanosecond(instant.nanosecond() / 1000 * 1000)
        .map_err(|_| ErreurBase::Audit("instant invalide"))?;
    let entree = EntreeAudit::nouvelle(*chaine.portee(), chaine.rang_suivant(), instant, evenement)
        .map_err(|_| ErreurBase::Audit("entrée invalide"))?;
    let (contenu, empreinte) = chaine
        .ajouter(&entree)
        .map_err(|_| ErreurBase::Audit("entrée hors de la chaîne"))?;
    let rang = rang_base(chaine.longueur())?;
    let cabinet = tx.contexte().id_cabinet().uuid();
    let octets = empreinte.octets().as_slice();
    match portee_audit {
        PorteeAudit::Cabinet => {
            sqlx::query!(
                "INSERT INTO audit_cabinet (cabinet_id, rang, contenu, empreinte) \
                 VALUES ($1, $2, $3, $4)",
                cabinet,
                rang,
                contenu,
                octets,
            )
            .execute(tx.connexion())
            .await?;
            sqlx::query!(
                "UPDATE audit_tete_cabinet SET rang = $2, empreinte = $3 WHERE cabinet_id = $1",
                cabinet,
                rang,
                octets,
            )
            .execute(tx.connexion())
            .await?;
        }
        PorteeAudit::Dossier(dossier) => {
            sqlx::query!(
                "INSERT INTO audit_dossier (cabinet_id, dossier_id, rang, contenu, empreinte) \
                 VALUES ($1, $2, $3, $4, $5)",
                cabinet,
                dossier.uuid(),
                rang,
                contenu,
                octets,
            )
            .execute(tx.connexion())
            .await?;
            sqlx::query!(
                "UPDATE audit_tete_dossier SET rang = $3, empreinte = $4 \
                 WHERE cabinet_id = $1 AND dossier_id = $2",
                cabinet,
                dossier.uuid(),
                rang,
                octets,
            )
            .execute(tx.connexion())
            .await?;
        }
    }
    Ok(Inscription {
        rang: chaine.longueur(),
        empreinte,
    })
}

/// Lit la tête enregistrée de la chaîne de `portee` (rôle de lecture d'audit) : rang et
/// empreinte ; absente si la portée n'a jamais eu d'entrée.
///
/// # Errors
/// Portée hors du contexte, donnée invalide, erreur de PostgreSQL.
pub async fn lire_tete(
    tx: &mut TransactionTenant,
    portee_audit: PorteeAudit,
) -> Result<Option<(u64, Empreinte)>, ErreurBase> {
    portee_integrite(tx, portee_audit)?;
    let cabinet = tx.contexte().id_cabinet().uuid();
    let ligne = match portee_audit {
        PorteeAudit::Cabinet => sqlx::query!(
            "SELECT rang, empreinte FROM audit_tete_cabinet WHERE cabinet_id = $1",
            cabinet,
        )
        .fetch_optional(tx.connexion())
        .await?
        .map(|l| (l.rang, l.empreinte)),
        PorteeAudit::Dossier(dossier) => sqlx::query!(
            "SELECT rang, empreinte FROM audit_tete_dossier \
             WHERE cabinet_id = $1 AND dossier_id = $2",
            cabinet,
            dossier.uuid(),
        )
        .fetch_optional(tx.connexion())
        .await?
        .map(|l| (l.rang, l.empreinte)),
    };
    ligne
        .map(|(rang, empreinte)| Ok((rang_lu(rang)?, empreinte_lue(&empreinte)?)))
        .transpose()
}

/// Lit au plus `limite` entrées de la chaîne de `portee` dont le rang dépasse `apres`, dans
/// l'ordre des rangs (rôle de lecture d'audit). Pagination imposée (F2.9).
///
/// # Errors
/// Portée hors du contexte, donnée invalide, erreur de PostgreSQL.
pub async fn lire_page(
    tx: &mut TransactionTenant,
    portee_audit: PorteeAudit,
    apres: u64,
    limite: u32,
) -> Result<Vec<LigneAudit>, ErreurBase> {
    portee_integrite(tx, portee_audit)?;
    let cabinet = tx.contexte().id_cabinet().uuid();
    let apres = rang_base(apres)?;
    let limite = i64::from(limite);
    let lignes: Vec<(i64, String, Vec<u8>)> = match portee_audit {
        PorteeAudit::Cabinet => sqlx::query!(
            "SELECT rang, contenu, empreinte FROM audit_cabinet \
             WHERE cabinet_id = $1 AND rang > $2 ORDER BY rang LIMIT $3",
            cabinet,
            apres,
            limite,
        )
        .fetch_all(tx.connexion())
        .await?
        .into_iter()
        .map(|l| (l.rang, l.contenu, l.empreinte))
        .collect(),
        PorteeAudit::Dossier(dossier) => sqlx::query!(
            "SELECT rang, contenu, empreinte FROM audit_dossier \
             WHERE cabinet_id = $1 AND dossier_id = $2 AND rang > $3 ORDER BY rang LIMIT $4",
            cabinet,
            dossier.uuid(),
            apres,
            limite,
        )
        .fetch_all(tx.connexion())
        .await?
        .into_iter()
        .map(|l| (l.rang, l.contenu, l.empreinte))
        .collect(),
    };
    lignes
        .into_iter()
        .map(|(rang, contenu, empreinte)| {
            Ok(LigneAudit {
                rang: rang_lu(rang)?,
                contenu,
                empreinte: empreinte_lue(&empreinte)?,
            })
        })
        .collect()
}
