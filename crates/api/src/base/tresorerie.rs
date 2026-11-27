//! Comptes de trésorerie et arrêtés de caisse en base (L3-21, PR 11 de `docs/lots/L3.md` ;
//! migrations `0540_compte_tresorerie.sql` et `0541_arrete_caisse.sql` ; ADR 0034 points 1 à 3).
//!
//! Niveau « requête » de l'isolation (ADR 0011) : chaque requête filtre par le cabinet et le
//! dossier du contexte, en plus de la RLS ; un identifiant d'un autre dossier ne rend rien.
//! Les montants passent en texte décimal exact (`NUMERIC` en base, jamais de flottant) ; les
//! soldes sont calculés par PostgreSQL sur les seules écritures **validées**.

use time::{Date, OffsetDateTime};
use uuid::Uuid;

use super::{ErreurBase, TransactionTenant};

fn contexte(tx: &TransactionTenant) -> Result<(Uuid, Uuid), ErreurBase> {
    let dossier = tx.contexte().id_dossier().ok_or(ErreurBase::Donnee(
        "trésorerie hors du contexte d'un dossier",
    ))?;
    Ok((tx.contexte().id_cabinet().uuid(), dossier.uuid()))
}

/// Compte de trésorerie relu, avec le numéro de son compte et le code de son journal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompteTresorerieLu {
    /// Identifiant.
    pub id: Uuid,
    /// `banque`, `caisse` ou `portefeuille`.
    pub type_tresorerie: String,
    /// Nom.
    pub nom: String,
    /// Compte du plan.
    pub compte_id: Uuid,
    /// Numéro de ce compte.
    pub compte: String,
    /// Journal de trésorerie.
    pub journal_id: Uuid,
    /// Code de ce journal.
    pub journal: String,
    /// Devise (celle de tenue du dossier en L3).
    pub devise: String,
    /// Plafond de caisse facultatif, décimal exact en texte.
    pub plafond: Option<String>,
    /// Compte d'écart d'excédent désigné : identifiant et numéro.
    pub compte_excedent: Option<(Uuid, String)>,
    /// Compte d'écart de manquant désigné : identifiant et numéro.
    pub compte_manquant: Option<(Uuid, String)>,
    /// Archivé.
    pub archive: bool,
    /// Création.
    pub cree_le: OffsetDateTime,
}

fn paire(id: Option<Uuid>, numero: Option<String>) -> Option<(Uuid, String)> {
    id.zip(numero)
}

macro_rules! lu {
    ($l:expr) => {
        CompteTresorerieLu {
            id: $l.id,
            type_tresorerie: $l.type_tresorerie,
            nom: $l.nom,
            compte_id: $l.compte_id,
            compte: $l.compte,
            journal_id: $l.journal_id,
            journal: $l.journal,
            devise: $l.devise,
            plafond: $l.plafond,
            compte_excedent: paire($l.compte_excedent_id, $l.excedent),
            compte_manquant: paire($l.compte_manquant_id, $l.manquant),
            archive: $l.archive,
            cree_le: $l.cree_le,
        }
    };
}

/// Comptes de trésorerie du dossier du contexte, par nom puis identifiant.
///
/// # Errors
/// Contexte sans dossier, erreur de PostgreSQL.
pub async fn lister(tx: &mut TransactionTenant) -> Result<Vec<CompteTresorerieLu>, ErreurBase> {
    let (cabinet, dossier) = contexte(tx)?;
    let lignes = sqlx::query!(
        r#"
        SELECT t.id, t.type AS type_tresorerie, t.nom, t.compte_id, c.numero AS compte,
               t.journal_id, j.code AS journal, t.devise, t.plafond::text AS plafond,
               t.compte_excedent_id, e.numero AS "excedent?", t.compte_manquant_id,
               m.numero AS "manquant?", t.archive, t.cree_le
        FROM compte_tresorerie t
        JOIN compte c ON c.cabinet_id = t.cabinet_id AND c.dossier_id = t.dossier_id
                     AND c.id = t.compte_id
        JOIN journal j ON j.cabinet_id = t.cabinet_id AND j.dossier_id = t.dossier_id
                      AND j.id = t.journal_id
        LEFT JOIN compte e ON e.cabinet_id = t.cabinet_id AND e.dossier_id = t.dossier_id
                         AND e.id = t.compte_excedent_id
        LEFT JOIN compte m ON m.cabinet_id = t.cabinet_id AND m.dossier_id = t.dossier_id
                         AND m.id = t.compte_manquant_id
        WHERE t.cabinet_id = $1 AND t.dossier_id = $2
        ORDER BY t.nom, t.id
        "#,
        cabinet,
        dossier,
    )
    .fetch_all(tx.connexion())
    .await?;
    Ok(lignes.into_iter().map(|l| lu!(l)).collect())
}

/// Compte de trésorerie `id` du dossier du contexte ; avec `verrou`, sa ligne est prise
/// `FOR NO KEY UPDATE` (modification, arrêté de caisse : les arrêtés d'une caisse se suivent).
///
/// # Errors
/// Contexte sans dossier, erreur de PostgreSQL.
pub async fn lire(
    tx: &mut TransactionTenant,
    id: Uuid,
    verrou: bool,
) -> Result<Option<CompteTresorerieLu>, ErreurBase> {
    let (cabinet, dossier) = contexte(tx)?;
    if verrou {
        let present = sqlx::query_scalar!(
            "SELECT id FROM compte_tresorerie \
             WHERE cabinet_id = $1 AND dossier_id = $2 AND id = $3 FOR NO KEY UPDATE",
            cabinet,
            dossier,
            id,
        )
        .fetch_optional(tx.connexion())
        .await?;
        if present.is_none() {
            return Ok(None);
        }
    }
    let ligne = sqlx::query!(
        r#"
        SELECT t.id, t.type AS type_tresorerie, t.nom, t.compte_id, c.numero AS compte,
               t.journal_id, j.code AS journal, t.devise, t.plafond::text AS plafond,
               t.compte_excedent_id, e.numero AS "excedent?", t.compte_manquant_id,
               m.numero AS "manquant?", t.archive, t.cree_le
        FROM compte_tresorerie t
        JOIN compte c ON c.cabinet_id = t.cabinet_id AND c.dossier_id = t.dossier_id
                     AND c.id = t.compte_id
        JOIN journal j ON j.cabinet_id = t.cabinet_id AND j.dossier_id = t.dossier_id
                      AND j.id = t.journal_id
        LEFT JOIN compte e ON e.cabinet_id = t.cabinet_id AND e.dossier_id = t.dossier_id
                         AND e.id = t.compte_excedent_id
        LEFT JOIN compte m ON m.cabinet_id = t.cabinet_id AND m.dossier_id = t.dossier_id
                         AND m.id = t.compte_manquant_id
        WHERE t.cabinet_id = $1 AND t.dossier_id = $2 AND t.id = $3
        "#,
        cabinet,
        dossier,
        id,
    )
    .fetch_optional(tx.connexion())
    .await?;
    Ok(ligne.map(|l| lu!(l)))
}

/// Nouveau compte de trésorerie.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NouveauCompteTresorerie<'a> {
    /// Identifiant.
    pub id: Uuid,
    /// `banque`, `caisse` ou `portefeuille`.
    pub type_tresorerie: &'a str,
    /// Nom.
    pub nom: &'a str,
    /// Compte du plan.
    pub compte_id: Uuid,
    /// Journal de trésorerie.
    pub journal_id: Uuid,
    /// Devise.
    pub devise: &'a str,
    /// Plafond de caisse, décimal exact en texte.
    pub plafond: Option<&'a str>,
    /// Instant de création.
    pub instant: OffsetDateTime,
}

/// Inscrit un compte de trésorerie, créé par l'utilisateur du contexte.
///
/// # Errors
/// Contexte sans dossier, contrainte refusée par PostgreSQL (compte ou journal déjà liés,
/// devise autre que celle de tenue).
pub async fn inserer(
    tx: &mut TransactionTenant,
    n: &NouveauCompteTresorerie<'_>,
) -> Result<(), ErreurBase> {
    let (cabinet, dossier) = contexte(tx)?;
    let auteur = tx.contexte().id_utilisateur().uuid();
    sqlx::query!(
        "INSERT INTO compte_tresorerie \
             (cabinet_id, dossier_id, id, type, nom, compte_id, journal_id, devise, plafond, \
              cree_par, cree_le) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9::text::numeric, $10, $11)",
        cabinet,
        dossier,
        n.id,
        n.type_tresorerie,
        n.nom,
        n.compte_id,
        n.journal_id,
        n.devise,
        n.plafond,
        auteur,
        n.instant,
    )
    .execute(tx.connexion())
    .await?;
    Ok(())
}

/// Configuration modifiable d'un compte de trésorerie.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Configuration {
    /// Nom.
    pub nom: String,
    /// Plafond de caisse.
    pub plafond: Option<String>,
    /// Compte d'écart d'excédent.
    pub compte_excedent_id: Option<Uuid>,
    /// Compte d'écart de manquant.
    pub compte_manquant_id: Option<Uuid>,
    /// Archivé.
    pub archive: bool,
}

/// Remplace la configuration d'un compte de trésorerie **verrouillé** ([`lire`] avec verrou).
///
/// # Errors
/// Contexte sans dossier, contrainte refusée par PostgreSQL.
pub async fn configurer(
    tx: &mut TransactionTenant,
    id: Uuid,
    c: &Configuration,
) -> Result<(), ErreurBase> {
    let (cabinet, dossier) = contexte(tx)?;
    sqlx::query!(
        "UPDATE compte_tresorerie \
         SET nom = $4, plafond = $5::text::numeric, compte_excedent_id = $6, \
             compte_manquant_id = $7, archive = $8 \
         WHERE cabinet_id = $1 AND dossier_id = $2 AND id = $3",
        cabinet,
        dossier,
        id,
        c.nom,
        c.plafond.as_deref(),
        c.compte_excedent_id,
        c.compte_manquant_id,
        c.archive,
    )
    .execute(tx.connexion())
    .await?;
    Ok(())
}

/// Soldes (débit positif, décimal exact en texte) des comptes `numeros`, sur les écritures
/// **validées** de date comptable au plus `au` (toutes sans date). Un compte sans mouvement
/// n'est pas rendu.
///
/// # Errors
/// Contexte sans dossier, erreur de PostgreSQL.
pub async fn soldes(
    tx: &mut TransactionTenant,
    numeros: &[String],
    au: Option<Date>,
) -> Result<Vec<(String, String)>, ErreurBase> {
    let (cabinet, dossier) = contexte(tx)?;
    let lignes = sqlx::query!(
        r#"
        SELECT l.compte_numero,
               SUM(CASE WHEN l.sens = 'debit' THEN l.montant ELSE -l.montant END)::text
                   AS "solde!"
        FROM ecriture_validee_ligne l
        JOIN ecriture_validee e ON e.cabinet_id = l.cabinet_id AND e.dossier_id = l.dossier_id
                               AND e.id = l.ecriture_id
        WHERE l.cabinet_id = $1 AND l.dossier_id = $2 AND l.compte_numero = ANY($3)
          AND ($4::date IS NULL OR e.date_comptable <= $4)
        GROUP BY l.compte_numero
        ORDER BY l.compte_numero
        "#,
        cabinet,
        dossier,
        numeros,
        au,
    )
    .fetch_all(tx.connexion())
    .await?;
    Ok(lignes
        .into_iter()
        .map(|l| (l.compte_numero, l.solde))
        .collect())
}

/// Arrêté de caisse à inscrire.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NouvelArrete<'a> {
    /// Identifiant.
    pub id: Uuid,
    /// Caisse.
    pub compte_tresorerie: Uuid,
    /// Date de l'arrêté.
    pub date: Date,
    /// Montant compté.
    pub montant_compte: &'a str,
    /// Solde comptable à la date.
    pub solde_comptable: &'a str,
    /// Écart (compté moins solde).
    pub ecart: &'a str,
    /// Pièce interne du comptage.
    pub piece: Uuid,
    /// Brouillon d'écart proposé.
    pub brouillon: Option<Uuid>,
    /// Instant.
    pub instant: OffsetDateTime,
}

/// Inscrit un arrêté de caisse (ajout seul), par l'utilisateur du contexte.
///
/// # Errors
/// Contexte sans dossier, contrainte refusée par PostgreSQL.
pub async fn inserer_arrete(
    tx: &mut TransactionTenant,
    a: &NouvelArrete<'_>,
) -> Result<(), ErreurBase> {
    let (cabinet, dossier) = contexte(tx)?;
    let auteur = tx.contexte().id_utilisateur().uuid();
    sqlx::query!(
        "INSERT INTO arrete_caisse \
             (cabinet_id, dossier_id, id, compte_tresorerie_id, date_arrete, montant_compte, \
              solde_comptable, ecart, piece_id, brouillon_id, arrete_par, arrete_le) \
         VALUES ($1, $2, $3, $4, $5, $6::text::numeric, $7::text::numeric, $8::text::numeric, \
                 $9, $10, $11, $12)",
        cabinet,
        dossier,
        a.id,
        a.compte_tresorerie,
        a.date,
        a.montant_compte,
        a.solde_comptable,
        a.ecart,
        a.piece,
        a.brouillon,
        auteur,
        a.instant,
    )
    .execute(tx.connexion())
    .await?;
    Ok(())
}

/// Arrêté de caisse relu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArreteLu {
    /// Identifiant.
    pub id: Uuid,
    /// Date de l'arrêté.
    pub date: Date,
    /// Montant compté.
    pub montant_compte: String,
    /// Solde comptable.
    pub solde_comptable: String,
    /// Écart.
    pub ecart: String,
    /// Pièce interne.
    pub piece: Uuid,
    /// Brouillon d'écart proposé.
    pub brouillon: Option<Uuid>,
    /// Auteur.
    pub arrete_par: Uuid,
    /// Instant.
    pub arrete_le: OffsetDateTime,
}

/// Arrêtés d'une caisse, du plus récent au plus ancien, au plus `limite`.
///
/// # Errors
/// Contexte sans dossier, erreur de PostgreSQL.
pub async fn arretes(
    tx: &mut TransactionTenant,
    compte_tresorerie: Uuid,
    limite: i64,
) -> Result<Vec<ArreteLu>, ErreurBase> {
    let (cabinet, dossier) = contexte(tx)?;
    let lignes = sqlx::query!(
        r#"
        SELECT id, date_arrete, montant_compte::text AS "montant_compte!",
               solde_comptable::text AS "solde_comptable!", ecart::text AS "ecart!", piece_id,
               brouillon_id, arrete_par, arrete_le
        FROM arrete_caisse
        WHERE cabinet_id = $1 AND dossier_id = $2 AND compte_tresorerie_id = $3
        ORDER BY date_arrete DESC, arrete_le DESC, id
        LIMIT $4
        "#,
        cabinet,
        dossier,
        compte_tresorerie,
        limite,
    )
    .fetch_all(tx.connexion())
    .await?;
    Ok(lignes
        .into_iter()
        .map(|l| ArreteLu {
            id: l.id,
            date: l.date_arrete,
            montant_compte: l.montant_compte,
            solde_comptable: l.solde_comptable,
            ecart: l.ecart,
            piece: l.piece_id,
            brouillon: l.brouillon_id,
            arrete_par: l.arrete_par,
            arrete_le: l.arrete_le,
        })
        .collect())
}

/// Vrai si le dossier du contexte a au moins un arrêté de caisse (mouvement qui fige la devise de
/// tenue, migration 0540).
///
/// # Errors
/// Contexte sans dossier, erreur de PostgreSQL.
pub async fn a_des_arretes(tx: &mut TransactionTenant) -> Result<bool, ErreurBase> {
    let (cabinet, dossier) = contexte(tx)?;
    Ok(sqlx::query_scalar!(
        r#"SELECT EXISTS (SELECT 1 FROM arrete_caisse WHERE cabinet_id = $1 AND dossier_id = $2)
           AS "existe!""#,
        cabinet,
        dossier,
    )
    .fetch_one(tx.connexion())
    .await?)
}
