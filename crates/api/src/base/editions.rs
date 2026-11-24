//! Lectures en bloc des éditions (PR 14 de `docs/lots/L2.md`, L2-28) : les écritures validées
//! et les brouillons d'un dossier datés dans une plage, lus à plat (une requête pour les en-têtes,
//! une pour les lignes), que `compta-core` agrège ensuite (balance, grand livre, journaux,
//! centralisateur).
//!
//! L'appelant ouvre une transaction à instantané unique
//! ([`super::ouvrir_transaction_lecture`]) : les écritures validées, les brouillons et le
//! référentiel viennent du même état. Toute requête filtre par le cabinet et le dossier du
//! contexte, sous RLS.
//!
//! Le nombre d'écritures lues est borné ([`ECRITURES_MAX`]) : une édition plus grande se
//! restreint par la période. Chaque lecture demande une écriture de plus que la borne pour savoir
//! s'il y en avait davantage.

use std::collections::HashMap;

use time::Date;
use uuid::Uuid;

use super::brouillard::{
    ContenuStocke, DeviseStockee, EtiquetteStockee, LigneStockee, decimal_canonique,
};
use super::{ErreurBase, IdCabinet, IdDossier, TransactionTenant};

/// Nombre maximal d'écritures (validées, puis brouillons) lues pour une édition.
pub const ECRITURES_MAX: i64 = 20_000;

/// Écriture validée relue pour une édition : colonnes et lignes, sans chaîne ni pièces.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValideeLue {
    /// Numéro `AC-2026-000231`.
    pub numero: String,
    /// Code du journal.
    pub journal: String,
    /// Date comptable.
    pub date: Date,
    /// Date de valeur comptable.
    pub date_valeur_comptable: Option<Date>,
    /// Libellé.
    pub libelle: String,
    /// Version des paramètres fiscaux.
    pub version_parametres: Option<String>,
    /// Date de validation.
    pub date_validation: Date,
    /// Lignes, dans l'ordre.
    pub lignes: Vec<LigneStockee>,
}

/// Lecture bornée : les éléments, et si la borne était dépassée (les éléments sont alors
/// tronqués à la borne et l'édition doit être refusée).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lecture<T> {
    /// Éléments lus.
    pub elements: Vec<T>,
    /// Vrai si le dossier en contient davantage sur la plage.
    pub depasse: bool,
}

fn tenant(tx: &TransactionTenant) -> Result<(IdCabinet, IdDossier), ErreurBase> {
    let contexte = tx.contexte();
    let dossier = contexte
        .id_dossier()
        .ok_or(ErreurBase::Donnee("éditions hors du contexte d'un dossier"))?;
    Ok((contexte.id_cabinet(), dossier))
}

/// Colonnes d'une ligne telle que lue (validée ou brouillon).
struct Colonnes {
    compte: String,
    sens: String,
    montant: String,
    devise: Option<String>,
    montant_devise: Option<String>,
    cours: Option<String>,
    tiers: Option<Uuid>,
    nature: Option<String>,
    sens_fiscal: Option<String>,
    regime: Option<String>,
    taux: Option<String>,
    exigibilite: Option<String>,
    motif: Option<String>,
}

fn ligne_lue(c: Colonnes) -> Result<LigneStockee, ErreurBase> {
    let incoherent = ErreurBase::Donnee("ligne lue pour une édition incohérente");
    let devise = match (c.devise, c.montant_devise, c.cours) {
        (None, None, None) => None,
        (Some(code), Some(montant), Some(cours)) => Some(DeviseStockee {
            code,
            montant: decimal_canonique(&montant),
            cours: decimal_canonique(&cours),
        }),
        _ => return Err(incoherent),
    };
    let etiquette = match (c.nature, c.sens_fiscal, c.regime) {
        (None, None, None) => None,
        (Some(nature), Some(sens), Some(regime)) => Some(EtiquetteStockee {
            nature,
            sens,
            regime,
            taux: c.taux.map(|t| decimal_canonique(&t)),
            exigibilite: c.exigibilite,
            motif: c.motif,
        }),
        _ => return Err(incoherent),
    };
    let sens = match c.sens.as_str() {
        "debit" => "debit",
        "credit" => "credit",
        _ => return Err(incoherent),
    };
    Ok(LigneStockee {
        compte: c.compte,
        sens,
        montant: decimal_canonique(&c.montant),
        devise,
        tiers: c.tiers,
        etiquette,
    })
}

/// Écritures validées du dossier du contexte, datées dans `[du, au]`, dans l'ordre (date
/// comptable, numéro).
///
/// # Errors
/// Contexte sans dossier, ligne relue incohérente, erreur de PostgreSQL.
pub async fn validees_entre(
    tx: &mut TransactionTenant,
    du: Date,
    au: Date,
) -> Result<Lecture<ValideeLue>, ErreurBase> {
    let (cabinet, dossier) = tenant(tx)?;
    let entetes = sqlx::query!(
        r#"
        SELECT e.id, e.numero, j.code AS journal, e.date_comptable, e.date_valeur_comptable,
               e.libelle, e.version_parametres, e.date_validation
        FROM ecriture_validee e
        JOIN journal j ON j.cabinet_id = e.cabinet_id AND j.dossier_id = e.dossier_id
                      AND j.id = e.journal_id
        WHERE e.cabinet_id = $1 AND e.dossier_id = $2 AND e.date_comptable BETWEEN $3 AND $4
        ORDER BY e.date_comptable, e.numero
        LIMIT $5
        "#,
        cabinet.uuid(),
        dossier.uuid(),
        du,
        au,
        ECRITURES_MAX + 1,
    )
    .fetch_all(tx.connexion())
    .await?;
    let borne = usize::try_from(ECRITURES_MAX).unwrap_or(usize::MAX);
    let depasse = entetes.len() > borne;
    let entetes: Vec<_> = entetes.into_iter().take(borne).collect();
    let ids: Vec<Uuid> = entetes.iter().map(|e| e.id).collect();
    let lignes = sqlx::query!(
        r#"
        SELECT l.ecriture_id, l.compte_numero, l.sens, l.montant::text AS "montant!", l.devise,
               l.montant_devise::text AS montant_devise, l.cours::text AS cours, l.tiers_id,
               l.fiscal_nature, l.fiscal_sens, l.fiscal_regime,
               l.fiscal_taux::text AS fiscal_taux, l.fiscal_exigibilite, l.fiscal_motif
        FROM ecriture_validee_ligne l
        WHERE l.cabinet_id = $1 AND l.dossier_id = $2 AND l.ecriture_id = ANY($3)
        ORDER BY l.ecriture_id, l.position
        "#,
        cabinet.uuid(),
        dossier.uuid(),
        &ids,
    )
    .fetch_all(tx.connexion())
    .await?;
    let mut par_ecriture: HashMap<Uuid, Vec<LigneStockee>> = HashMap::new();
    for l in lignes {
        let ligne = ligne_lue(Colonnes {
            compte: l.compte_numero,
            sens: l.sens,
            montant: l.montant,
            devise: l.devise,
            montant_devise: l.montant_devise,
            cours: l.cours,
            tiers: l.tiers_id,
            nature: l.fiscal_nature,
            sens_fiscal: l.fiscal_sens,
            regime: l.fiscal_regime,
            taux: l.fiscal_taux,
            exigibilite: l.fiscal_exigibilite,
            motif: l.fiscal_motif,
        })?;
        par_ecriture.entry(l.ecriture_id).or_default().push(ligne);
    }
    let elements = entetes
        .into_iter()
        .map(|e| ValideeLue {
            lignes: par_ecriture.remove(&e.id).unwrap_or_default(),
            numero: e.numero,
            journal: e.journal,
            date: e.date_comptable,
            date_valeur_comptable: e.date_valeur_comptable,
            libelle: e.libelle,
            version_parametres: e.version_parametres,
            date_validation: e.date_validation,
        })
        .collect();
    Ok(Lecture { elements, depasse })
}

/// Brouillons du dossier du contexte, datés dans `[du, au]`, avec leurs lignes, dans l'ordre
/// (date comptable, identifiant).
///
/// # Errors
/// Contexte sans dossier, ligne relue incohérente, erreur de PostgreSQL.
pub async fn brouillons_entre(
    tx: &mut TransactionTenant,
    du: Date,
    au: Date,
) -> Result<Lecture<ContenuStocke>, ErreurBase> {
    let (cabinet, dossier) = tenant(tx)?;
    let entetes = sqlx::query!(
        r#"
        SELECT b.id, b.journal_id, j.code AS journal, b.date_comptable, b.date_valeur_comptable,
               b.libelle
        FROM brouillon b
        JOIN journal j ON j.cabinet_id = b.cabinet_id AND j.dossier_id = b.dossier_id
                      AND j.id = b.journal_id
        WHERE b.cabinet_id = $1 AND b.dossier_id = $2 AND b.date_comptable BETWEEN $3 AND $4
        ORDER BY b.date_comptable, b.id
        LIMIT $5
        "#,
        cabinet.uuid(),
        dossier.uuid(),
        du,
        au,
        ECRITURES_MAX + 1,
    )
    .fetch_all(tx.connexion())
    .await?;
    let borne = usize::try_from(ECRITURES_MAX).unwrap_or(usize::MAX);
    let depasse = entetes.len() > borne;
    let entetes: Vec<_> = entetes.into_iter().take(borne).collect();
    let ids: Vec<Uuid> = entetes.iter().map(|e| e.id).collect();
    let lignes = sqlx::query!(
        r#"
        SELECT l.brouillon_id, l.compte_numero, l.sens, l.montant::text AS "montant!", l.devise,
               l.montant_devise::text AS montant_devise, l.cours::text AS cours, l.tiers_id,
               l.fiscal_nature, l.fiscal_sens, l.fiscal_regime,
               l.fiscal_taux::text AS fiscal_taux, l.fiscal_exigibilite, l.fiscal_motif
        FROM brouillon_ligne l
        WHERE l.cabinet_id = $1 AND l.dossier_id = $2 AND l.brouillon_id = ANY($3)
        ORDER BY l.brouillon_id, l.position
        "#,
        cabinet.uuid(),
        dossier.uuid(),
        &ids,
    )
    .fetch_all(tx.connexion())
    .await?;
    let mut par_brouillon: HashMap<Uuid, Vec<LigneStockee>> = HashMap::new();
    for l in lignes {
        let ligne = ligne_lue(Colonnes {
            compte: l.compte_numero,
            sens: l.sens,
            montant: l.montant,
            devise: l.devise,
            montant_devise: l.montant_devise,
            cours: l.cours,
            tiers: l.tiers_id,
            nature: l.fiscal_nature,
            sens_fiscal: l.fiscal_sens,
            regime: l.fiscal_regime,
            taux: l.fiscal_taux,
            exigibilite: l.fiscal_exigibilite,
            motif: l.fiscal_motif,
        })?;
        par_brouillon.entry(l.brouillon_id).or_default().push(ligne);
    }
    let elements = entetes
        .into_iter()
        .map(|b| ContenuStocke {
            lignes: par_brouillon.remove(&b.id).unwrap_or_default(),
            journal_id: b.journal_id,
            journal: b.journal,
            date: b.date_comptable,
            date_valeur_comptable: b.date_valeur_comptable,
            libelle: b.libelle,
        })
        .collect();
    Ok(Lecture { elements, depasse })
}

/// Code du journal et date comptable de **chaque** brouillon du dossier du contexte (équilibré
/// ou non), pour l'alerte de brouillard non validé. Pas de borne : deux colonnes par brouillon.
///
/// # Errors
/// Contexte sans dossier, erreur de PostgreSQL.
pub async fn brouillons_datees(
    tx: &mut TransactionTenant,
) -> Result<Vec<(String, Date)>, ErreurBase> {
    let (cabinet, dossier) = tenant(tx)?;
    let lignes = sqlx::query!(
        r#"
        SELECT j.code AS journal, b.date_comptable
        FROM brouillon b
        JOIN journal j ON j.cabinet_id = b.cabinet_id AND j.dossier_id = b.dossier_id
                      AND j.id = b.journal_id
        WHERE b.cabinet_id = $1 AND b.dossier_id = $2
        ORDER BY b.date_comptable, b.id
        "#,
        cabinet.uuid(),
        dossier.uuid(),
    )
    .fetch_all(tx.connexion())
    .await?;
    Ok(lignes
        .into_iter()
        .map(|l| (l.journal, l.date_comptable))
        .collect())
}
