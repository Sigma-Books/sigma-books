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
//! Le nombre d'écritures lues est borné (`borne`, par défaut [`ECRITURES_MAX`], réglage
//! `editions_ecritures_max`) : une édition plus grande se restreint par la période. Chaque
//! lecture demande une écriture de plus que la borne pour savoir s'il y en avait davantage.
//!
//! Seules les écritures **de l'exercice** sont lues : l'appelant passe `du` = premier jour de
//! l'exercice. Une écriture antérieure à l'exercice n'entre jamais dans ses éditions ; ses
//! à-nouveaux sont les écritures d'un journal d'à-nouveaux **de l'exercice** (en pratique celles
//! du premier jour).

use std::collections::HashMap;

use time::Date;
use uuid::Uuid;

use super::brouillard::{
    ContenuStocke, DeviseStockee, EtiquetteStockee, LigneStockee, decimal_canonique,
};
use super::validation::hex;
use super::{ErreurBase, IdCabinet, IdDossier, TransactionTenant};

/// Nombre maximal d'écritures (validées, puis brouillons) lues pour une édition, par défaut.
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
    /// Numéro de l'écriture contre-passée, si celle-ci est une contre-passation.
    pub origine_numero: Option<String>,
    /// Lignes, dans l'ordre.
    pub lignes: Vec<LigneStockee>,
    /// Empreintes (hexadécimales minuscules) des pièces liées, dans l'ordre de la justification :
    /// `compta-core` ne reconstitue pas une écriture validée sans justification.
    pub empreintes: Vec<String>,
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
        date_echeance: None,
    })
}

/// Écritures validées du dossier du contexte, datées dans `[du, au]`, dans l'ordre (date
/// comptable, numéro).
///
/// # Errors
/// Contexte sans dossier, ligne relue incohérente, erreur de PostgreSQL.
#[allow(clippy::too_many_lines)]
pub async fn validees_entre(
    tx: &mut TransactionTenant,
    du: Date,
    au: Date,
    borne: i64,
) -> Result<Lecture<ValideeLue>, ErreurBase> {
    let (cabinet, dossier) = tenant(tx)?;
    let entetes = sqlx::query!(
        r#"
        SELECT e.id, e.numero, j.code AS journal, e.date_comptable, e.date_valeur_comptable,
               e.libelle, e.version_parametres, e.date_validation,
               o.numero AS "origine_numero?"
        FROM ecriture_validee e
        JOIN journal j ON j.cabinet_id = e.cabinet_id AND j.dossier_id = e.dossier_id
                      AND j.id = e.journal_id
        LEFT JOIN ecriture_validee o ON o.cabinet_id = e.cabinet_id
                                    AND o.dossier_id = e.dossier_id AND o.id = e.origine_id
        WHERE e.cabinet_id = $1 AND e.dossier_id = $2 AND e.date_comptable BETWEEN $3 AND $4
        ORDER BY e.date_comptable, e.numero
        LIMIT $5
        "#,
        cabinet.uuid(),
        dossier.uuid(),
        du,
        au,
        borne.saturating_add(1),
    )
    .fetch_all(tx.connexion())
    .await?;
    let borne = usize::try_from(borne).unwrap_or(usize::MAX);
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
    let pieces = sqlx::query!(
        r#"
        SELECT v.ecriture_id, p.empreinte
        FROM ecriture_validee_piece v
        JOIN piece p ON p.cabinet_id = v.cabinet_id AND p.dossier_id = v.dossier_id
                    AND p.id = v.piece_id
        WHERE v.cabinet_id = $1 AND v.dossier_id = $2 AND v.ecriture_id = ANY($3)
        ORDER BY v.ecriture_id, v.position
        "#,
        cabinet.uuid(),
        dossier.uuid(),
        &ids,
    )
    .fetch_all(tx.connexion())
    .await?;
    let mut empreintes: HashMap<Uuid, Vec<String>> = HashMap::new();
    for p in pieces {
        empreintes
            .entry(p.ecriture_id)
            .or_default()
            .push(hex(&p.empreinte));
    }
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
            empreintes: empreintes.remove(&e.id).unwrap_or_default(),
            numero: e.numero,
            journal: e.journal,
            date: e.date_comptable,
            date_valeur_comptable: e.date_valeur_comptable,
            libelle: e.libelle,
            version_parametres: e.version_parametres,
            date_validation: e.date_validation,
            origine_numero: e.origine_numero,
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
    borne: i64,
) -> Result<Lecture<ContenuStocke>, ErreurBase> {
    let (cabinet, dossier) = tenant(tx)?;
    let entetes = sqlx::query!(
        r#"
        SELECT b.id, b.journal_id, j.code AS journal, b.date_comptable, b.date_valeur_comptable,
               b.libelle, b.devise_tenue, b.decimales_tenue, b.version_parametres
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
        borne.saturating_add(1),
    )
    .fetch_all(tx.connexion())
    .await?;
    let borne = usize::try_from(borne).unwrap_or(usize::MAX);
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
        .map(|b| {
            Ok(ContenuStocke {
                lignes: par_brouillon.remove(&b.id).unwrap_or_default(),
                journal_id: b.journal_id,
                journal: b.journal,
                date: b.date_comptable,
                date_valeur_comptable: b.date_valeur_comptable,
                libelle: b.libelle,
                devise_tenue: b.devise_tenue,
                decimales_tenue: u8::try_from(b.decimales_tenue)
                    .map_err(|_| ErreurBase::Donnee("échelle de brouillon invalide"))?,
                version_parametres: b.version_parametres,
            })
        })
        .collect::<Result<Vec<_>, ErreurBase>>()?;
    Ok(Lecture { elements, depasse })
}

/// Brouillons non validés regroupés par (journal, date comptable) : le code du journal, la date
/// et le nombre de brouillons, pour l'alerte de brouillard non validé (équilibrés ou non). Le
/// regroupement est fait par PostgreSQL : le volume lu ne dépend pas du nombre de brouillons
/// mais du nombre de couples (journal, jour), borné par `borne` (`depasse` si davantage).
///
/// # Errors
/// Contexte sans dossier, erreur de PostgreSQL.
pub async fn brouillons_par_journal_et_date(
    tx: &mut TransactionTenant,
    borne: i64,
) -> Result<Lecture<(String, Date, usize)>, ErreurBase> {
    let (cabinet, dossier) = tenant(tx)?;
    let lignes = sqlx::query!(
        r#"
        SELECT j.code AS journal, b.date_comptable, count(*) AS "nombre!"
        FROM brouillon b
        JOIN journal j ON j.cabinet_id = b.cabinet_id AND j.dossier_id = b.dossier_id
                      AND j.id = b.journal_id
        WHERE b.cabinet_id = $1 AND b.dossier_id = $2
        GROUP BY j.code, b.date_comptable
        ORDER BY b.date_comptable, j.code
        LIMIT $3
        "#,
        cabinet.uuid(),
        dossier.uuid(),
        borne.saturating_add(1),
    )
    .fetch_all(tx.connexion())
    .await?;
    let borne = usize::try_from(borne).unwrap_or(usize::MAX);
    let depasse = lignes.len() > borne;
    let elements = lignes
        .into_iter()
        .take(borne)
        .map(|l| {
            Ok((
                l.journal,
                l.date_comptable,
                usize::try_from(l.nombre)
                    .map_err(|_| ErreurBase::Donnee("nombre de brouillons négatif"))?,
            ))
        })
        .collect::<Result<_, ErreurBase>>()?;
    Ok(Lecture { elements, depasse })
}
