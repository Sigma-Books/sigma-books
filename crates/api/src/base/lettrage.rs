//! Lettrage en base (L3-18, PR 9a de `docs/lots/L3.md` ; migration `0520_lettrage.sql` ;
//! ADR 0032).
//!
//! Ce module ne fait que lire et écrire : les règles (conditions, imputation, résiduels, rupture)
//! sont celles de `compta_core::lettrage`, appliquées par [`crate::serveur::routes::lettrage`].
//! Niveau « requête » de l'isolation (ADR 0011) : chaque requête filtre par le cabinet et le
//! dossier du contexte, en plus de la RLS ; un identifiant d'un autre dossier ne rend rien. Les
//! montants passent en texte décimal exact (`NUMERIC` en base, jamais de flottant).
//!
//! Aucune requête de ce module ne modifie une écriture validée (règle absolue 5) : les tables du
//! lettrage sont en ajout seul ; seul le compteur du compte (état dérivé) reçoit un `UPDATE`.

use std::collections::BTreeMap;

use time::{Date, OffsetDateTime};
use uuid::Uuid;

use super::brouillard::{DeviseStockee, decimal_canonique};
use super::{ErreurBase, TransactionTenant};

fn contexte(tx: &TransactionTenant) -> Result<(Uuid, Uuid), ErreurBase> {
    let dossier = tx
        .contexte()
        .id_dossier()
        .ok_or(ErreurBase::Donnee("lettrage hors du contexte d'un dossier"))?;
    Ok((tx.contexte().id_cabinet().uuid(), dossier.uuid()))
}

/// Référence d'une ligne d'écriture validée : (écriture, position à partir de 1).
pub type RefLigne = (Uuid, i16);

fn colonnes(refs: &[RefLigne]) -> (Vec<Uuid>, Vec<i16>) {
    refs.iter().copied().unzip()
}

// --- Lignes validées ----------------------------------------------------------------------------

/// Ligne d'écriture validée relue pour le lettrage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LigneLue {
    /// Écriture validée.
    pub ecriture_id: Uuid,
    /// Position de la ligne (à partir de 1).
    pub position: i16,
    /// Rang de chaîne de l'écriture (unique par dossier, ADR 0020).
    pub rang: i64,
    /// Numéro de l'écriture.
    pub numero: String,
    /// Date comptable.
    pub date: Date,
    /// Compte : identifiant et numéro.
    pub compte_id: Uuid,
    /// Numéro du compte.
    pub compte: String,
    /// `debit` ou `credit`.
    pub sens: String,
    /// Montant en devise de tenue, décimal canonique.
    pub montant: String,
    /// Montant en devise étrangère.
    pub devise: Option<DeviseStockee>,
    /// Tiers de la ligne (non résolu).
    pub tiers: Option<Uuid>,
}

impl LigneLue {
    /// Référence de la ligne.
    #[must_use]
    pub fn reference(&self) -> RefLigne {
        (self.ecriture_id, self.position)
    }
}

macro_rules! ligne_lue {
    ($l:expr) => {
        LigneLue {
            ecriture_id: $l.ecriture_id,
            position: $l.position,
            rang: $l.rang,
            numero: $l.numero,
            date: $l.date_comptable,
            compte_id: $l.compte_id,
            compte: $l.compte_numero,
            sens: $l.sens,
            montant: decimal_canonique(&$l.montant),
            devise: match ($l.devise, $l.montant_devise, $l.cours) {
                (Some(code), Some(montant), Some(cours)) => Some(DeviseStockee {
                    code,
                    montant,
                    cours,
                }),
                _ => None,
            },
            tiers: $l.tiers_id,
        }
    };
}

/// Lignes validées désignées par `refs`, dans le dossier du contexte (une référence d'un autre
/// dossier ou inexistante ne rend rien), par rang de chaîne puis position.
///
/// # Errors
/// Contexte sans dossier, erreur de PostgreSQL.
pub async fn lignes(
    tx: &mut TransactionTenant,
    refs: &[RefLigne],
) -> Result<Vec<LigneLue>, ErreurBase> {
    let (cabinet, dossier) = contexte(tx)?;
    let (ecritures, positions) = colonnes(refs);
    let lues = sqlx::query!(
        r#"SELECT l.ecriture_id, l.position, e.rang, e.numero, e.date_comptable,
                  c.id AS compte_id, l.compte_numero, l.sens, l.montant::text AS "montant!",
                  l.devise, l.montant_devise::text AS montant_devise, l.cours::text AS cours,
                  l.tiers_id
             FROM (SELECT DISTINCT * FROM unnest($3::uuid[], $4::int2[])) AS r (ecriture_id, position)
             JOIN ecriture_validee_ligne l
               ON l.cabinet_id = $1 AND l.dossier_id = $2
              AND l.ecriture_id = r.ecriture_id AND l.position = r.position
             JOIN ecriture_validee e
               ON e.cabinet_id = l.cabinet_id AND e.dossier_id = l.dossier_id
              AND e.id = l.ecriture_id
             JOIN compte c
               ON c.cabinet_id = l.cabinet_id AND c.dossier_id = l.dossier_id
              AND c.numero = l.compte_numero
            ORDER BY e.rang, l.position"#,
        cabinet,
        dossier,
        &ecritures,
        &positions,
    )
    .fetch_all(tx.connexion())
    .await?;
    Ok(lues.into_iter().map(|l| ligne_lue!(l)).collect())
}

/// Lignes validées dont le tiers est l'un de `tiers`, par rang de chaîne puis position, au plus
/// `limite`.
///
/// # Errors
/// Contexte sans dossier, erreur de PostgreSQL.
pub async fn lignes_des_tiers(
    tx: &mut TransactionTenant,
    tiers: &[Uuid],
    limite: i64,
) -> Result<Vec<LigneLue>, ErreurBase> {
    let (cabinet, dossier) = contexte(tx)?;
    let lues = sqlx::query!(
        r#"SELECT l.ecriture_id, l.position, e.rang, e.numero, e.date_comptable,
                  c.id AS compte_id, l.compte_numero, l.sens, l.montant::text AS "montant!",
                  l.devise, l.montant_devise::text AS montant_devise, l.cours::text AS cours,
                  l.tiers_id
             FROM ecriture_validee_ligne l
             JOIN ecriture_validee e
               ON e.cabinet_id = l.cabinet_id AND e.dossier_id = l.dossier_id
              AND e.id = l.ecriture_id
             JOIN compte c
               ON c.cabinet_id = l.cabinet_id AND c.dossier_id = l.dossier_id
              AND c.numero = l.compte_numero
            WHERE l.cabinet_id = $1 AND l.dossier_id = $2 AND l.tiers_id = ANY($3::uuid[])
            ORDER BY e.rang, l.position
            LIMIT $4"#,
        cabinet,
        dossier,
        tiers,
        limite,
    )
    .fetch_all(tx.connexion())
    .await?;
    Ok(lues.into_iter().map(|l| ligne_lue!(l)).collect())
}

/// Lignes validées du compte `compte_id`, par rang de chaîne puis position, au plus `limite`.
///
/// # Errors
/// Contexte sans dossier, erreur de PostgreSQL.
pub async fn lignes_du_compte(
    tx: &mut TransactionTenant,
    compte_id: Uuid,
    limite: i64,
) -> Result<Vec<LigneLue>, ErreurBase> {
    let (cabinet, dossier) = contexte(tx)?;
    let lues = sqlx::query!(
        r#"SELECT l.ecriture_id, l.position, e.rang, e.numero, e.date_comptable,
                  c.id AS compte_id, l.compte_numero, l.sens, l.montant::text AS "montant!",
                  l.devise, l.montant_devise::text AS montant_devise, l.cours::text AS cours,
                  l.tiers_id
             FROM compte c
             JOIN ecriture_validee_ligne l
               ON l.cabinet_id = c.cabinet_id AND l.dossier_id = c.dossier_id
              AND l.compte_numero = c.numero
             JOIN ecriture_validee e
               ON e.cabinet_id = l.cabinet_id AND e.dossier_id = l.dossier_id
              AND e.id = l.ecriture_id
            WHERE c.cabinet_id = $1 AND c.dossier_id = $2 AND c.id = $3
            ORDER BY e.rang, l.position
            LIMIT $4"#,
        cabinet,
        dossier,
        compte_id,
        limite,
    )
    .fetch_all(tx.connexion())
    .await?;
    Ok(lues.into_iter().map(|l| ligne_lue!(l)).collect())
}

/// Numéro du compte `id` du dossier du contexte, s'il existe.
///
/// # Errors
/// Contexte sans dossier, erreur de PostgreSQL.
pub async fn numero_du_compte(
    tx: &mut TransactionTenant,
    id: Uuid,
) -> Result<Option<String>, ErreurBase> {
    let (cabinet, dossier) = contexte(tx)?;
    Ok(sqlx::query_scalar!(
        "SELECT numero FROM compte WHERE cabinet_id = $1 AND dossier_id = $2 AND id = $3",
        cabinet,
        dossier,
        id,
    )
    .fetch_optional(tx.connexion())
    .await?)
}

// --- Tiers résolu (ADR 0031 point 4) ---------------------------------------------------------------

/// Tiers **résolu** de chaque tiers de `tiers` : la cible d'une fusion non rompue dont il est la
/// source, sinon lui-même (ADR 0031 point 4 : la résolution est faite **en un seul endroit**).
///
/// Aucune fusion n'existe encore (`tiers_fusion` arrive avec la PR 7b) : chaque tiers est son
/// propre tiers résolu. La PR 7b branche ici sa table, et [`tiers_rattaches`] avec elle ; aucun
/// autre code du lettrage ne lit le tiers d'une ligne sans passer par ces deux fonctions.
///
/// # Errors
/// Contexte sans dossier.
#[allow(clippy::unused_async)] // La PR 7b y lit `tiers_fusion` : signature asynchrone d'avance.
pub async fn tiers_resolus(
    tx: &mut TransactionTenant,
    tiers: &[Uuid],
) -> Result<BTreeMap<Uuid, Uuid>, ErreurBase> {
    contexte(tx)?;
    Ok(tiers.iter().map(|t| (*t, *t)).collect())
}

/// Tiers dont le tiers résolu est `tiers` : lui-même et les sources des fusions non rompues
/// vers lui (aucune avant la PR 7b ; voir [`tiers_resolus`]).
///
/// # Errors
/// Contexte sans dossier.
#[allow(clippy::unused_async)] // La PR 7b y lit `tiers_fusion` : signature asynchrone d'avance.
pub async fn tiers_rattaches(
    tx: &mut TransactionTenant,
    tiers: Uuid,
) -> Result<Vec<Uuid>, ErreurBase> {
    contexte(tx)?;
    Ok(vec![tiers])
}

// --- Lettrages ----------------------------------------------------------------------------------

/// Lien relu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LienLu {
    /// Ligne au débit.
    pub debit: RefLigne,
    /// Ligne au crédit.
    pub credit: RefLigne,
    /// Montant imputé, décimal canonique.
    pub montant: String,
}

/// Rupture relue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuptureLue {
    /// Identifiant.
    pub id: Uuid,
    /// Rang d'opération.
    pub rang: i64,
    /// Date de l'opération.
    pub date: Date,
    /// Motif.
    pub motif: String,
    /// Auteur.
    pub auteur: Uuid,
}

/// Lettrage relu, avec ses liens (dans l'ordre) et sa rupture éventuelle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LettrageLu {
    /// Identifiant.
    pub id: Uuid,
    /// Compte.
    pub compte_id: Uuid,
    /// Lettre.
    pub lettre: String,
    /// Rang d'opération du compte.
    pub rang: i64,
    /// Date de l'opération.
    pub date: Date,
    /// `manuelle`, `automatique` ou `proposition_validee`.
    pub origine: String,
    /// Auteur (aucun pour l'application automatique).
    pub auteur: Option<Uuid>,
    /// Liens.
    pub liens: Vec<LienLu>,
    /// Rupture.
    pub rupture: Option<RuptureLue>,
}

/// Lettrages `ids` du dossier du contexte (ceux d'un autre dossier sont absents), par compte puis
/// rang d'opération.
///
/// # Errors
/// Contexte sans dossier, erreur de PostgreSQL.
pub async fn lettrages(
    tx: &mut TransactionTenant,
    ids: &[Uuid],
) -> Result<Vec<LettrageLu>, ErreurBase> {
    let (cabinet, dossier) = contexte(tx)?;
    let entetes = sqlx::query!(
        "SELECT id, compte_id, lettre, rang, date_operation, origine, auteur FROM lettrage \
         WHERE cabinet_id = $1 AND dossier_id = $2 AND id = ANY($3::uuid[]) \
         ORDER BY compte_id, rang",
        cabinet,
        dossier,
        ids,
    )
    .fetch_all(tx.connexion())
    .await?;
    let ids: Vec<Uuid> = entetes.iter().map(|e| e.id).collect();
    let liens = sqlx::query!(
        r#"SELECT lettrage_id, debit_ecriture_id, debit_position, credit_ecriture_id,
                  credit_position, montant::text AS "montant!"
             FROM lettrage_lien
            WHERE cabinet_id = $1 AND dossier_id = $2 AND lettrage_id = ANY($3::uuid[])
            ORDER BY lettrage_id, position"#,
        cabinet,
        dossier,
        &ids,
    )
    .fetch_all(tx.connexion())
    .await?;
    let ruptures = sqlx::query!(
        "SELECT id, lettrage_id, rang, date_operation, motif, auteur FROM lettrage_rupture \
         WHERE cabinet_id = $1 AND dossier_id = $2 AND lettrage_id = ANY($3::uuid[])",
        cabinet,
        dossier,
        &ids,
    )
    .fetch_all(tx.connexion())
    .await?;
    let mut par_lettrage: BTreeMap<Uuid, Vec<LienLu>> = BTreeMap::new();
    for l in liens {
        par_lettrage.entry(l.lettrage_id).or_default().push(LienLu {
            debit: (l.debit_ecriture_id, l.debit_position),
            credit: (l.credit_ecriture_id, l.credit_position),
            montant: decimal_canonique(&l.montant),
        });
    }
    let mut rompus: BTreeMap<Uuid, RuptureLue> = ruptures
        .into_iter()
        .map(|r| {
            (
                r.lettrage_id,
                RuptureLue {
                    id: r.id,
                    rang: r.rang,
                    date: r.date_operation,
                    motif: r.motif,
                    auteur: r.auteur,
                },
            )
        })
        .collect();
    Ok(entetes
        .into_iter()
        .map(|e| LettrageLu {
            id: e.id,
            compte_id: e.compte_id,
            lettre: e.lettre,
            rang: e.rang,
            date: e.date_operation,
            origine: e.origine,
            auteur: e.auteur,
            liens: par_lettrage.remove(&e.id).unwrap_or_default(),
            rupture: rompus.remove(&e.id),
        })
        .collect())
}

/// Identifiants des lettrages (rompus compris) dont un lien cite l'une des lignes `refs`.
///
/// # Errors
/// Contexte sans dossier, erreur de PostgreSQL.
pub async fn lettrages_citant(
    tx: &mut TransactionTenant,
    refs: &[RefLigne],
) -> Result<Vec<Uuid>, ErreurBase> {
    let (cabinet, dossier) = contexte(tx)?;
    let (ecritures, positions) = colonnes(refs);
    Ok(sqlx::query_scalar!(
        r#"SELECT k.lettrage_id AS "id!"
             FROM unnest($3::uuid[], $4::int2[]) AS r (ecriture_id, position)
             JOIN lettrage_lien k
               ON k.cabinet_id = $1 AND k.dossier_id = $2
              AND k.debit_ecriture_id = r.ecriture_id AND k.debit_position = r.position
           UNION
           SELECT k.lettrage_id
             FROM unnest($3::uuid[], $4::int2[]) AS r (ecriture_id, position)
             JOIN lettrage_lien k
               ON k.cabinet_id = $1 AND k.dossier_id = $2
              AND k.credit_ecriture_id = r.ecriture_id AND k.credit_position = r.position"#,
        cabinet,
        dossier,
        &ecritures,
        &positions,
    )
    .fetch_all(tx.connexion())
    .await?)
}

// --- Compteur du compte (rang 7 de l'ordre des verrous, ADR 0032 point 8) -------------------------

/// Compteur d'un compte, relu sous verrou.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Compteur {
    /// Dernière lettre tirée (aucune avant le premier lettrage).
    pub derniere_lettre: Option<String>,
    /// Dernier rang d'opération (0 : aucune).
    pub dernier_rang: i64,
}

/// Verrouille `FOR UPDATE` le compteur de lettrage du compte `compte_id`, créé à la demande
/// (comme la tête de chaîne) : deux mutations de lettrage du même compte se suivent.
///
/// # Errors
/// Contexte sans dossier, erreur de PostgreSQL.
pub async fn verrouiller_compteur(
    tx: &mut TransactionTenant,
    compte_id: Uuid,
) -> Result<Compteur, ErreurBase> {
    let (cabinet, dossier) = contexte(tx)?;
    sqlx::query!(
        "INSERT INTO compteur_lettrage (cabinet_id, dossier_id, compte_id, dernier_rang) \
         VALUES ($1, $2, $3, 0) ON CONFLICT DO NOTHING",
        cabinet,
        dossier,
        compte_id,
    )
    .execute(tx.connexion())
    .await?;
    let l = sqlx::query!(
        "SELECT derniere_lettre, dernier_rang FROM compteur_lettrage \
         WHERE cabinet_id = $1 AND dossier_id = $2 AND compte_id = $3 FOR UPDATE",
        cabinet,
        dossier,
        compte_id,
    )
    .fetch_one(tx.connexion())
    .await?;
    Ok(Compteur {
        derniere_lettre: l.derniere_lettre,
        dernier_rang: l.dernier_rang,
    })
}

/// Avance le compteur (sous le verrou de [`verrouiller_compteur`]).
///
/// # Errors
/// Contexte sans dossier, erreur de PostgreSQL.
pub async fn avancer_compteur(
    tx: &mut TransactionTenant,
    compte_id: Uuid,
    compteur: &Compteur,
) -> Result<(), ErreurBase> {
    let (cabinet, dossier) = contexte(tx)?;
    sqlx::query!(
        "UPDATE compteur_lettrage SET derniere_lettre = $4, dernier_rang = $5 \
         WHERE cabinet_id = $1 AND dossier_id = $2 AND compte_id = $3",
        cabinet,
        dossier,
        compte_id,
        compteur.derniere_lettre.as_deref(),
        compteur.dernier_rang,
    )
    .execute(tx.connexion())
    .await?;
    Ok(())
}

// --- Inscriptions (ajout seul) --------------------------------------------------------------------

/// Lettrage à inscrire.
#[derive(Debug, Clone)]
pub struct NouveauLettrage<'a> {
    /// Identifiant.
    pub id: Uuid,
    /// Compte.
    pub compte_id: Uuid,
    /// Lettre.
    pub lettre: &'a str,
    /// Rang d'opération.
    pub rang: i64,
    /// Date de l'opération.
    pub date: Date,
    /// Origine.
    pub origine: &'static str,
    /// Instant de l'inscription.
    pub instant: OffsetDateTime,
    /// Liens, dans l'ordre de l'imputation : (débit, crédit, montant en décimal exact).
    pub liens: &'a [(RefLigne, RefLigne, String)],
}

/// Inscrit un lettrage et ses liens ; l'auteur est l'utilisateur du contexte. Chaque lien passe
/// par le déclencheur `lettrage_lien_controle` (seconde barrière).
///
/// # Errors
/// Contexte sans dossier, erreur de PostgreSQL (contrainte, déclencheur).
pub async fn inserer_lettrage(
    tx: &mut TransactionTenant,
    n: &NouveauLettrage<'_>,
) -> Result<(), ErreurBase> {
    let (cabinet, dossier) = contexte(tx)?;
    let auteur = tx.contexte().id_utilisateur().uuid();
    sqlx::query!(
        "INSERT INTO lettrage (cabinet_id, dossier_id, id, compte_id, lettre, rang, \
         date_operation, origine, auteur, cree_le) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
        cabinet,
        dossier,
        n.id,
        n.compte_id,
        n.lettre,
        n.rang,
        n.date,
        n.origine,
        auteur,
        n.instant,
    )
    .execute(tx.connexion())
    .await?;
    for (rang, (debit, credit, montant)) in (1_i16..).zip(n.liens) {
        sqlx::query!(
            "INSERT INTO lettrage_lien (cabinet_id, dossier_id, lettrage_id, position, \
             debit_ecriture_id, debit_position, credit_ecriture_id, credit_position, montant) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9::text::numeric)",
            cabinet,
            dossier,
            n.id,
            rang,
            debit.0,
            debit.1,
            credit.0,
            credit.1,
            montant,
        )
        .execute(tx.connexion())
        .await?;
    }
    Ok(())
}

/// Rupture à inscrire.
#[derive(Debug, Clone)]
pub struct NouvelleRupture<'a> {
    /// Identifiant.
    pub id: Uuid,
    /// Lettrage rompu.
    pub lettrage_id: Uuid,
    /// Compte du lettrage.
    pub compte_id: Uuid,
    /// Rang d'opération.
    pub rang: i64,
    /// Date de l'opération.
    pub date: Date,
    /// Motif (déjà contrôlé par `compta_core::MotifRupture`).
    pub motif: &'a str,
    /// Instant de l'inscription.
    pub instant: OffsetDateTime,
}

/// Inscrit une rupture ; l'auteur est l'utilisateur du contexte. Aucun lien n'est supprimé ni
/// modifié.
///
/// # Errors
/// Contexte sans dossier, erreur de PostgreSQL (dont l'unicité d'une rupture par lettrage).
pub async fn inserer_rupture(
    tx: &mut TransactionTenant,
    n: &NouvelleRupture<'_>,
) -> Result<(), ErreurBase> {
    let (cabinet, dossier) = contexte(tx)?;
    let auteur = tx.contexte().id_utilisateur().uuid();
    sqlx::query!(
        "INSERT INTO lettrage_rupture (cabinet_id, dossier_id, id, lettrage_id, compte_id, rang, \
         date_operation, motif, auteur, cree_le) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
        cabinet,
        dossier,
        n.id,
        n.lettrage_id,
        n.compte_id,
        n.rang,
        n.date,
        n.motif,
        auteur,
        n.instant,
    )
    .execute(tx.connexion())
    .await?;
    Ok(())
}

/// Vrai si `date` tombe dans un exercice clos du dossier du contexte (lecture simple, pour un
/// avertissement).
///
/// # Errors
/// Contexte sans dossier, erreur de PostgreSQL.
pub async fn dans_un_exercice_clos(
    tx: &mut TransactionTenant,
    date: Date,
) -> Result<bool, ErreurBase> {
    let (cabinet, dossier) = contexte(tx)?;
    Ok(sqlx::query_scalar!(
        r#"SELECT EXISTS (SELECT 1 FROM exercice
                           WHERE cabinet_id = $1 AND dossier_id = $2
                             AND debut <= $3 AND fin >= $3 AND statut = 'cloture') AS "clos!""#,
        cabinet,
        dossier,
        date,
    )
    .fetch_one(tx.connexion())
    .await?)
}

// --- Interface de l'échéancier (ADR 0033 point 2 ; PR 8) -----------------------------------------

/// Résiduel de chaque ligne de `refs` : montant moins la somme des liens des lettrages **non
/// rompus** qui la citent (ADR 0032 point 1), en décimal canonique, calculé par agrégat (jamais
/// stocké). Une référence d'un autre dossier ou inexistante est absente du résultat. Lecture
/// simple, sans verrou : pour une décision (révision d'échéance), prendre d'abord
/// [`verrouiller_compte_de_ligne`]. Même résultat que `compta_core::RegistreLettrage::residuel`
/// sur un registre rejoué (test `residuels_par_agregat_egaux_a_ceux_du_registre`).
///
/// # Errors
/// Contexte sans dossier, erreur de PostgreSQL.
pub async fn residuels(
    tx: &mut TransactionTenant,
    refs: &[RefLigne],
) -> Result<BTreeMap<RefLigne, String>, ErreurBase> {
    let (cabinet, dossier) = contexte(tx)?;
    let (ecritures, positions) = colonnes(refs);
    let lus = sqlx::query!(
        r#"SELECT l.ecriture_id, l.position,
                  (l.montant - COALESCE((
                      SELECT sum(k.montant) FROM lettrage_lien k
                       WHERE k.cabinet_id = l.cabinet_id AND k.dossier_id = l.dossier_id
                         AND ((k.debit_ecriture_id = l.ecriture_id
                               AND k.debit_position = l.position)
                              OR (k.credit_ecriture_id = l.ecriture_id
                                  AND k.credit_position = l.position))
                         AND NOT EXISTS (
                             SELECT 1 FROM lettrage_rupture x
                              WHERE x.cabinet_id = k.cabinet_id AND x.dossier_id = k.dossier_id
                                AND x.lettrage_id = k.lettrage_id)), 0))::text AS "residuel!"
             FROM (SELECT DISTINCT * FROM unnest($3::uuid[], $4::int2[])) AS r (ecriture_id, position)
             JOIN ecriture_validee_ligne l
               ON l.cabinet_id = $1 AND l.dossier_id = $2
              AND l.ecriture_id = r.ecriture_id AND l.position = r.position"#,
        cabinet,
        dossier,
        &ecritures,
        &positions,
    )
    .fetch_all(tx.connexion())
    .await?;
    Ok(lus
        .into_iter()
        .map(|l| ((l.ecriture_id, l.position), decimal_canonique(&l.residuel)))
        .collect())
}

/// Verrou du compte de la ligne `(ecriture, position)` : son compteur de lettrage `FOR UPDATE`
/// (rang 7 de l'ordre des verrous, ADR 0032 point 8), créé à la demande. Pris par la **révision
/// d'une échéance** (ADR 0033 point 2) avant de lire le résiduel ([`residuels`]) : un lettrage ou
/// une rupture concurrents du même compte attendent la fin de la révision, et inversement.
/// `None` si la ligne n'existe pas dans le dossier du contexte.
///
/// # Errors
/// Contexte sans dossier, erreur de PostgreSQL.
pub async fn verrouiller_compte_de_ligne(
    tx: &mut TransactionTenant,
    ecriture: Uuid,
    position: i16,
) -> Result<Option<Compteur>, ErreurBase> {
    let (cabinet, dossier) = contexte(tx)?;
    let compte = sqlx::query_scalar!(
        "SELECT c.id FROM ecriture_validee_ligne l \
         JOIN compte c ON c.cabinet_id = l.cabinet_id AND c.dossier_id = l.dossier_id \
                      AND c.numero = l.compte_numero \
         WHERE l.cabinet_id = $1 AND l.dossier_id = $2 AND l.ecriture_id = $3 \
           AND l.position = $4",
        cabinet,
        dossier,
        ecriture,
        position,
    )
    .fetch_optional(tx.connexion())
    .await?;
    match compte {
        None => Ok(None),
        Some(id) => Ok(Some(verrouiller_compteur(tx, id).await?)),
    }
}
