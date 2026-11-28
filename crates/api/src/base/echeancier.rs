//! Conditions de règlement et échéances en base (L3-17, PR 8 de `docs/lots/L3.md` ; migrations
//! `0510` à `0514` ; ADR 0033).
//!
//! Niveau « requête » de l'isolation (ADR 0011) : chaque requête filtre par le cabinet et le
//! dossier du contexte, en plus de la RLS ; un identifiant d'un autre dossier ne rend rien.
//! Les montants et pourcentages passent en texte décimal exact (`NUMERIC` en base). Les
//! échéances sont **hors du contenu haché** : ce module ne lit les écritures validées qu'en
//! lecture et n'écrit que dans `echeance_report` (l'échéance initiale est copiée par
//! [`super::validation::inserer`]).

use rust_decimal::Decimal;
use time::{Date, OffsetDateTime};
use uuid::Uuid;

use super::brouillard::decimal_canonique;
use super::{ErreurBase, TransactionTenant};

fn contexte(tx: &TransactionTenant) -> Result<(Uuid, Uuid), ErreurBase> {
    let dossier = tx.contexte().id_dossier().ok_or(ErreurBase::Donnee(
        "échéancier hors du contexte d'un dossier",
    ))?;
    Ok((tx.contexte().id_cabinet().uuid(), dossier.uuid()))
}

// --- Conditions de règlement -----------------------------------------------------------------

/// Terme d'une condition, sous sa forme stockée (valeurs stables de la migration `0510`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TermeStocke {
    /// `pourcentage`, `fixe` ou `solde`.
    pub valeur: String,
    /// Pourcentage du total ou montant fixe, décimal canonique ; absent pour le solde.
    pub montant: Option<String>,
    /// `jours`, `fin_de_mois`, `fin_du_mois_suivant` ou `jour_du_mois_suivant`.
    pub delai: String,
    /// Nombre de jours (signé, comme chez Tryton et Odoo).
    pub jours: i32,
    /// Jour du mois suivant (0 : fin du mois), pour le seul délai `jour_du_mois_suivant`.
    pub jour_du_mois: Option<u8>,
}

/// Condition de règlement relue, avec ses termes dans l'ordre de leur rang.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConditionLue {
    /// Identifiant.
    pub id: Uuid,
    /// Code stable, unique dans le dossier.
    pub code: String,
    /// Libellé.
    pub libelle: String,
    /// Faux pour une condition archivée.
    pub actif: bool,
    /// Termes.
    pub termes: Vec<TermeStocke>,
}

/// Vrai si la condition `id` appartient au dossier du contexte.
///
/// # Errors
/// Contexte sans dossier, erreur de PostgreSQL.
pub async fn condition_existe(tx: &mut TransactionTenant, id: Uuid) -> Result<bool, ErreurBase> {
    let (cabinet, dossier) = contexte(tx)?;
    Ok(sqlx::query_scalar!(
        r#"SELECT EXISTS (SELECT 1 FROM condition_reglement
                          WHERE cabinet_id = $1 AND dossier_id = $2 AND id = $3) AS "existe!""#,
        cabinet,
        dossier,
        id,
    )
    .fetch_one(tx.connexion())
    .await?)
}

/// Termes des conditions `ids`, groupés par condition, dans l'ordre du rang.
async fn termes_de(
    tx: &mut TransactionTenant,
    ids: &[Uuid],
) -> Result<Vec<(Uuid, TermeStocke)>, ErreurBase> {
    let (cabinet, dossier) = contexte(tx)?;
    let lignes = sqlx::query!(
        r#"SELECT condition_id, valeur, montant::text AS montant, delai, jours, jour_du_mois
           FROM condition_reglement_terme
           WHERE cabinet_id = $1 AND dossier_id = $2 AND condition_id = ANY($3)
           ORDER BY condition_id, rang"#,
        cabinet,
        dossier,
        ids,
    )
    .fetch_all(tx.connexion())
    .await?;
    lignes
        .into_iter()
        .map(|l| {
            let jour_du_mois = l
                .jour_du_mois
                .map(|j| u8::try_from(j).map_err(|_| ErreurBase::Donnee("jour du mois")))
                .transpose()?;
            Ok((
                l.condition_id,
                TermeStocke {
                    valeur: l.valeur,
                    montant: l.montant.map(|m| decimal_canonique(&m)),
                    delai: l.delai,
                    jours: l.jours,
                    jour_du_mois,
                },
            ))
        })
        .collect()
}

fn assembler(
    entetes: Vec<(Uuid, String, String, bool)>,
    termes: &[(Uuid, TermeStocke)],
) -> Vec<ConditionLue> {
    entetes
        .into_iter()
        .map(|(id, code, libelle, actif)| ConditionLue {
            id,
            code,
            libelle,
            actif,
            termes: termes
                .iter()
                .filter(|(c, _)| *c == id)
                .map(|(_, t)| t.clone())
                .collect(),
        })
        .collect()
}

/// Conditions de règlement du dossier du contexte, par code.
///
/// # Errors
/// Contexte sans dossier, erreur de PostgreSQL.
pub async fn lister_conditions(
    tx: &mut TransactionTenant,
) -> Result<Vec<ConditionLue>, ErreurBase> {
    let (cabinet, dossier) = contexte(tx)?;
    let entetes: Vec<(Uuid, String, String, bool)> = sqlx::query!(
        "SELECT id, code, libelle, actif FROM condition_reglement \
         WHERE cabinet_id = $1 AND dossier_id = $2 ORDER BY code",
        cabinet,
        dossier,
    )
    .fetch_all(tx.connexion())
    .await?
    .into_iter()
    .map(|l| (l.id, l.code, l.libelle, l.actif))
    .collect();
    let ids: Vec<Uuid> = entetes.iter().map(|e| e.0).collect();
    let termes = termes_de(tx, &ids).await?;
    Ok(assembler(entetes, &termes))
}

/// La condition `id` du dossier du contexte.
///
/// # Errors
/// Contexte sans dossier, erreur de PostgreSQL.
pub async fn lire_condition(
    tx: &mut TransactionTenant,
    id: Uuid,
) -> Result<Option<ConditionLue>, ErreurBase> {
    let (cabinet, dossier) = contexte(tx)?;
    let Some(l) = sqlx::query!(
        "SELECT id, code, libelle, actif FROM condition_reglement \
         WHERE cabinet_id = $1 AND dossier_id = $2 AND id = $3",
        cabinet,
        dossier,
        id,
    )
    .fetch_optional(tx.connexion())
    .await?
    else {
        return Ok(None);
    };
    let termes = termes_de(tx, &[id]).await?;
    Ok(assembler(vec![(l.id, l.code, l.libelle, l.actif)], &termes).pop())
}

/// Verrouille la condition `id` (`FOR NO KEY UPDATE` : la clé étrangère d'un tiers qui la cite
/// ne prend que `KEY SHARE`, elle n'attend pas). Faux si elle est absente du dossier.
///
/// # Errors
/// Contexte sans dossier, erreur de PostgreSQL.
pub async fn verrouiller_condition(
    tx: &mut TransactionTenant,
    id: Uuid,
) -> Result<bool, ErreurBase> {
    let (cabinet, dossier) = contexte(tx)?;
    Ok(sqlx::query_scalar!(
        "SELECT id FROM condition_reglement \
         WHERE cabinet_id = $1 AND dossier_id = $2 AND id = $3 FOR NO KEY UPDATE",
        cabinet,
        dossier,
        id,
    )
    .fetch_optional(tx.connexion())
    .await?
    .is_some())
}

async fn inserer_termes(
    tx: &mut TransactionTenant,
    condition: Uuid,
    termes: &[TermeStocke],
) -> Result<(), ErreurBase> {
    let (cabinet, dossier) = contexte(tx)?;
    for (indice, t) in termes.iter().enumerate() {
        let rang = i16::try_from(indice + 1).map_err(|_| ErreurBase::Donnee("rang de terme"))?;
        sqlx::query!(
            "INSERT INTO condition_reglement_terme \
                 (cabinet_id, dossier_id, condition_id, rang, valeur, montant, delai, jours, \
                  jour_du_mois) \
             VALUES ($1, $2, $3, $4, $5, $6::text::numeric, $7, $8, $9)",
            cabinet,
            dossier,
            condition,
            rang,
            t.valeur,
            t.montant,
            t.delai,
            t.jours,
            t.jour_du_mois.map(i16::from),
        )
        .execute(tx.connexion())
        .await?;
    }
    Ok(())
}

/// Crée une condition et ses termes. Faux, sans rien écrire, si le code est déjà pris dans le
/// dossier (unicité tenue par la base, sans course possible).
///
/// # Errors
/// Contexte sans dossier, contrainte refusée par PostgreSQL.
pub async fn creer_condition(
    tx: &mut TransactionTenant,
    condition: &ConditionLue,
    instant: OffsetDateTime,
) -> Result<bool, ErreurBase> {
    let (cabinet, dossier) = contexte(tx)?;
    let creee = sqlx::query!(
        "INSERT INTO condition_reglement (cabinet_id, dossier_id, id, code, libelle, actif, \
                                          cree_le) \
         VALUES ($1, $2, $3, $4, $5, $6, $7) \
         ON CONFLICT (cabinet_id, dossier_id, code) DO NOTHING",
        cabinet,
        dossier,
        condition.id,
        condition.code,
        condition.libelle,
        condition.actif,
        instant,
    )
    .execute(tx.connexion())
    .await?
    .rows_affected()
        == 1;
    if creee {
        inserer_termes(tx, condition.id, &condition.termes).await?;
    }
    Ok(creee)
}

/// Enregistre le libellé et l'activité d'une condition **verrouillée**, et, si `termes` est
/// fourni, remplace ses termes (suppression puis insertion, comme les lignes d'un brouillon).
///
/// # Errors
/// Contexte sans dossier, contrainte refusée par PostgreSQL.
pub async fn modifier_condition(
    tx: &mut TransactionTenant,
    condition: &ConditionLue,
    remplacer_termes: bool,
) -> Result<(), ErreurBase> {
    let (cabinet, dossier) = contexte(tx)?;
    sqlx::query!(
        "UPDATE condition_reglement SET libelle = $4, actif = $5 \
         WHERE cabinet_id = $1 AND dossier_id = $2 AND id = $3",
        cabinet,
        dossier,
        condition.id,
        condition.libelle,
        condition.actif,
    )
    .execute(tx.connexion())
    .await?;
    if remplacer_termes {
        sqlx::query!(
            "DELETE FROM condition_reglement_terme \
             WHERE cabinet_id = $1 AND dossier_id = $2 AND condition_id = $3",
            cabinet,
            dossier,
            condition.id,
        )
        .execute(tx.connexion())
        .await?;
        inserer_termes(tx, condition.id, &condition.termes).await?;
    }
    Ok(())
}

// --- Échéances des lignes validées -----------------------------------------------------------

/// Copie les échéances des lignes du brouillon `brouillon` dans `echeance_ligne`, sous le même
/// identifiant (celui de l'écriture validée) et aux mêmes positions : **un `INSERT … SELECT`**,
/// appelé par [`super::validation::inserer`] dans la transaction de validation, avant la
/// suppression du brouillon. Rien n'entre dans le contenu haché ni dans la chaîne (ADR 0033
/// point 1). Une ligne sans échéance n'est pas copiée.
///
/// # Errors
/// Contexte sans dossier, contrainte refusée par PostgreSQL.
pub async fn copier_echeances(
    tx: &mut TransactionTenant,
    brouillon: Uuid,
) -> Result<(), ErreurBase> {
    let (cabinet, dossier) = contexte(tx)?;
    sqlx::query!(
        "INSERT INTO echeance_ligne \
             (cabinet_id, dossier_id, ecriture_id, position, date_echeance) \
         SELECT cabinet_id, dossier_id, brouillon_id, position, date_echeance \
         FROM brouillon_ligne \
         WHERE cabinet_id = $1 AND dossier_id = $2 AND brouillon_id = $3 \
           AND date_echeance IS NOT NULL",
        cabinet,
        dossier,
        brouillon,
    )
    .execute(tx.connexion())
    .await?;
    Ok(())
}

/// Ligne de tiers d'une écriture validée, avec son échéance initiale.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LigneTiersValidee {
    /// Position de la ligne (à partir de 1).
    pub position: i16,
    /// Numéro du compte.
    pub compte: String,
    /// `debit` ou `credit`.
    pub sens: String,
    /// Montant en devise de tenue (décimal exact, positif).
    pub montant: Decimal,
    /// Tiers.
    pub tiers: Uuid,
    /// Échéance initiale copiée à la validation (`echeance_ligne`).
    pub initiale: Option<Date>,
}

/// Révision relue (`echeance_report`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevisionLue {
    /// Position de la ligne révisée.
    pub position: i16,
    /// Rang de la révision sur la ligne.
    pub rang: i32,
    /// Nouvelle échéance.
    pub nouvelle_echeance: Date,
    /// Jour de l'opération (fuseau du dossier).
    pub date_operation: Date,
    /// Motif.
    pub motif: String,
    /// Auteur.
    pub auteur: Uuid,
    /// Instant de l'opération.
    pub cree_le: OffsetDateTime,
}

/// Écriture validée relue pour l'échéancier : sa date comptable (repli de l'échéance, ADR 0033
/// point 2) et ses lignes de tiers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EcritureEcheances {
    /// Date comptable.
    pub date_comptable: Date,
    /// Lignes de tiers, par position.
    pub lignes: Vec<LigneTiersValidee>,
    /// Révisions de ces lignes, par position puis rang.
    pub revisions: Vec<RevisionLue>,
}

/// L'écriture validée `ecriture` du dossier du contexte, ses lignes de tiers et leurs échéances.
///
/// # Errors
/// Contexte sans dossier, montant stocké illisible, erreur de PostgreSQL.
pub async fn lire_ecriture(
    tx: &mut TransactionTenant,
    ecriture: Uuid,
) -> Result<Option<EcritureEcheances>, ErreurBase> {
    let (cabinet, dossier) = contexte(tx)?;
    let Some(date_comptable) = sqlx::query_scalar!(
        "SELECT date_comptable FROM ecriture_validee \
         WHERE cabinet_id = $1 AND dossier_id = $2 AND id = $3",
        cabinet,
        dossier,
        ecriture,
    )
    .fetch_optional(tx.connexion())
    .await?
    else {
        return Ok(None);
    };
    let lignes = sqlx::query!(
        r#"SELECT l.position AS "position!", l.compte_numero AS "compte_numero!",
                  l.sens AS "sens!", l.montant::text AS "montant!",
                  l.tiers_id AS "tiers_id!", e.date_echeance AS "initiale?"
           FROM ecriture_validee_ligne l
           LEFT JOIN echeance_ligne e
             ON e.cabinet_id = l.cabinet_id AND e.dossier_id = l.dossier_id
            AND e.ecriture_id = l.ecriture_id AND e.position = l.position
           WHERE l.cabinet_id = $1 AND l.dossier_id = $2 AND l.ecriture_id = $3
             AND l.tiers_id IS NOT NULL
           ORDER BY l.position"#,
        cabinet,
        dossier,
        ecriture,
    )
    .fetch_all(tx.connexion())
    .await?
    .into_iter()
    .map(|l| {
        Ok(LigneTiersValidee {
            position: l.position,
            compte: l.compte_numero,
            sens: l.sens,
            montant: Decimal::from_str_exact(&l.montant)
                .map_err(|_| ErreurBase::Donnee("montant d'une ligne validée"))?,
            tiers: l.tiers_id,
            initiale: l.initiale,
        })
    })
    .collect::<Result<Vec<_>, ErreurBase>>()?;
    let revisions = sqlx::query!(
        "SELECT position, rang, nouvelle_echeance, date_operation, motif, auteur, cree_le \
         FROM echeance_report \
         WHERE cabinet_id = $1 AND dossier_id = $2 AND ecriture_id = $3 \
         ORDER BY position, rang",
        cabinet,
        dossier,
        ecriture,
    )
    .fetch_all(tx.connexion())
    .await?
    .into_iter()
    .map(|r| RevisionLue {
        position: r.position,
        rang: r.rang,
        nouvelle_echeance: r.nouvelle_echeance,
        date_operation: r.date_operation,
        motif: r.motif,
        auteur: r.auteur,
        cree_le: r.cree_le,
    })
    .collect();
    Ok(Some(EcritureEcheances {
        date_comptable,
        lignes,
        revisions,
    }))
}

/// **Interface du lettrage** (ADR 0032, PR 9a) : résiduel de la ligne `(ecriture, position)`, de
/// montant `montant`, en valeur absolue. Le lettrage n'est pas encore livré : aucune ligne n'a de
/// lien, le résiduel est le montant entier (ligne ouverte).
///
/// À brancher par la PR 9a : prendre d'abord le `compteur_lettrage` du compte `FOR UPDATE`
/// (rang 7 de l'ordre des verrous, ADR 0032 point 8 ; ADR 0033 point 2), puis rendre le montant
/// moins les liens des lettrages non rompus de la ligne. La révision appelle cette fonction
/// avant de décider ; la lecture de l'échéancier aussi.
///
/// # Errors
/// Contexte sans dossier (et, avec le lettrage, erreur de PostgreSQL).
#[allow(clippy::unused_async)] // Signature de l'interface : le lettrage (PR 9a) lira la base.
pub async fn residuel(
    tx: &mut TransactionTenant,
    ecriture: Uuid,
    position: i16,
    montant: Decimal,
) -> Result<Decimal, ErreurBase> {
    contexte(tx)?;
    let _ = (ecriture, position);
    Ok(montant.abs())
}

/// Nouvelle révision à inscrire.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NouvelleRevision {
    /// Écriture validée.
    pub ecriture: Uuid,
    /// Position de la ligne.
    pub position: i16,
    /// Rang calculé par `compta-core` (dernier rang plus un).
    pub rang: i32,
    /// Nouvelle échéance.
    pub nouvelle_echeance: Date,
    /// Jour de l'opération (fuseau du dossier).
    pub date_operation: Date,
    /// Motif contrôlé.
    pub motif: String,
    /// Instant de l'opération.
    pub instant: OffsetDateTime,
}

/// Inscrit une révision (ajout seul). Faux, sans rien écrire, si ce rang vient d'être pris par une
/// révision concurrente de la même ligne (clé primaire) : l'appelant rend 409, le client relit.
///
/// # Errors
/// Contexte sans dossier, contrainte refusée par PostgreSQL.
pub async fn ajouter_revision(
    tx: &mut TransactionTenant,
    r: &NouvelleRevision,
) -> Result<bool, ErreurBase> {
    let (cabinet, dossier) = contexte(tx)?;
    let auteur = tx.contexte().id_utilisateur().uuid();
    Ok(sqlx::query!(
        "INSERT INTO echeance_report (cabinet_id, dossier_id, ecriture_id, position, rang, \
                                      nouvelle_echeance, date_operation, motif, auteur, cree_le) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10) \
         ON CONFLICT (cabinet_id, dossier_id, ecriture_id, position, rang) DO NOTHING",
        cabinet,
        dossier,
        r.ecriture,
        r.position,
        r.rang,
        r.nouvelle_echeance,
        r.date_operation,
        r.motif,
        auteur,
        r.instant,
    )
    .execute(tx.connexion())
    .await?
    .rows_affected()
        == 1)
}
