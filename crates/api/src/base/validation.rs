//! Écritures validées d'un dossier, chaîne de hachage et compteurs (L2-26, PR 13b ; migrations
//! `0203_validation.sql` et `0351_ecriture_validee_pieces.sql` ; spec §6.2 à §6.4 ; ADR 0007,
//! 0011, 0015).
//!
//! Ce module ne fait que lire et écrire. Les règles de validation sont celles de `compta-core`
//! ([`compta_core::valider`]) ; l'enchaînement est dans
//! [`crate::serveur::routes::brouillard`].
//!
//! # Ordre des verrous (ADR 0011, note d'étude de la PR 13 §3)
//! 1. exercices puis périodes du dossier, `FOR SHARE`, chronologiques ([`verrouiller_calendrier`]) ;
//! 2. brouillon `FOR UPDATE` ([`super::brouillard::verrouiller`]) ;
//! 3. comptes et tiers cités `FOR SHARE` ([`verrouiller_comptes_et_tiers`]) ;
//! 4. pièces liées `FOR SHARE`, par identifiant ([`verrouiller_pieces`]) ;
//! 5. tête de chaîne `FOR UPDATE` ([`verrouiller_tete`]) : elle sérialise les validations d'un
//!    dossier ;
//! 6. compteur `FOR UPDATE` ([`verrouiller_compteur`]) ;
//! 7. tête d'audit du dossier (prise par le pipeline à l'inscription des traces).
//!
//! Aucun verrou de ligne n'est possible sur les écritures validées (ajout seul : le rôle
//! applicatif n'a pas `UPDATE`) : la tête de chaîne en tient lieu.
//!
//! # Écriture
//! Seule une [`compta_core::Validation`] se persiste ([`Inscription::depuis`]) : les valeurs
//! écrites viennent d'elle. Les montants transitent en texte canonique (`::text::numeric`), comme
//! au brouillard.

use compta_core::{EcritureValidee, Validation};
use integrite::Empreinte;
use time::{Date, OffsetDateTime};
use uuid::Uuid;

use super::brouillard::{
    DeviseStockee, EtiquetteStockee, LigneStockee, RolePiece, decimal_canonique,
};
use super::{ErreurBase, IdCabinet, IdDossier, TransactionTenant};

fn tenant(tx: &TransactionTenant) -> Result<(IdCabinet, IdDossier), ErreurBase> {
    let contexte = tx.contexte();
    let dossier = contexte.id_dossier().ok_or(ErreurBase::Donnee(
        "écritures validées hors du contexte d'un dossier",
    ))?;
    Ok((contexte.id_cabinet(), dossier))
}

fn empreinte_lue(octets: &[u8]) -> Result<Empreinte, ErreurBase> {
    let tableau: [u8; 32] = octets
        .try_into()
        .map_err(|_| ErreurBase::Donnee("empreinte de longueur inattendue"))?;
    Ok(Empreinte::depuis_octets(tableau))
}

// --- Verrous ----------------------------------------------------------------------------

/// Verrouille `FOR SHARE` les exercices puis les périodes du dossier, dans l'ordre
/// chronologique : une clôture ou un verrouillage concurrent (verrou exclusif) attend la fin de
/// la validation ; une période clôturée reste visible (les déclencheurs d'irréversibilité ne
/// jouent que sur `UPDATE`), c'est `compta_core::valider` qui la refuse.
///
/// # Errors
/// Contexte sans dossier, erreur de PostgreSQL.
pub async fn verrouiller_calendrier(tx: &mut TransactionTenant) -> Result<(), ErreurBase> {
    let (cabinet, dossier) = tenant(tx)?;
    sqlx::query_scalar!(
        "SELECT id FROM exercice WHERE cabinet_id = $1 AND dossier_id = $2 ORDER BY debut \
         FOR SHARE",
        cabinet.uuid(),
        dossier.uuid(),
    )
    .fetch_all(tx.connexion())
    .await?;
    sqlx::query_scalar!(
        "SELECT id FROM periode WHERE cabinet_id = $1 AND dossier_id = $2 ORDER BY debut \
         FOR SHARE",
        cabinet.uuid(),
        dossier.uuid(),
    )
    .fetch_all(tx.connexion())
    .await?;
    Ok(())
}

/// Verrouille `FOR SHARE` les comptes et les tiers cités par les lignes (par ordre croissant,
/// sans doublon), après le brouillon et avant les pièces : une désactivation concurrente
/// (`UPDATE`) attend la fin de la validation. Ce verrou est possible parce que le rôle
/// applicatif a `UPDATE` sur `compte (intitule, actif)` et `tiers (nom, actif)`.
///
/// # Errors
/// Contexte sans dossier, erreur de PostgreSQL.
pub async fn verrouiller_comptes_et_tiers(
    tx: &mut TransactionTenant,
    comptes: &[String],
    tiers: &[Uuid],
) -> Result<(), ErreurBase> {
    let (cabinet, dossier) = tenant(tx)?;
    let mut comptes = comptes.to_vec();
    comptes.sort_unstable();
    comptes.dedup();
    for numero in comptes {
        sqlx::query_scalar!(
            "SELECT numero FROM compte WHERE cabinet_id = $1 AND dossier_id = $2 \
             AND numero = $3 FOR SHARE",
            cabinet.uuid(),
            dossier.uuid(),
            numero,
        )
        .fetch_optional(tx.connexion())
        .await?;
    }
    let mut tiers = tiers.to_vec();
    tiers.sort_unstable();
    tiers.dedup();
    for id in tiers {
        sqlx::query_scalar!(
            "SELECT id FROM tiers WHERE cabinet_id = $1 AND dossier_id = $2 AND id = $3 \
             FOR SHARE",
            cabinet.uuid(),
            dossier.uuid(),
            id,
        )
        .fetch_optional(tx.connexion())
        .await?;
    }
    Ok(())
}

/// Pièce liée, relue sous `FOR SHARE`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PieceVerrouillee {
    /// Identifiant.
    pub id: Uuid,
    /// Empreinte SHA-256 du contenu, en hexadécimal minuscule (référence de la justification).
    pub empreinte: String,
    /// Pièce écartée.
    pub ecartee: bool,
}

pub(super) fn hex(octets: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut s = String::with_capacity(octets.len() * 2);
    for o in octets {
        let _ = write!(s, "{o:02x}");
    }
    s
}

/// Verrouille `FOR SHARE` les pièces données (par identifiant croissant) et les relit. Une
/// pièce absente du dossier n'est pas rendue.
///
/// # Errors
/// Contexte sans dossier, erreur de PostgreSQL.
pub async fn verrouiller_pieces(
    tx: &mut TransactionTenant,
    ids: &[Uuid],
) -> Result<Vec<PieceVerrouillee>, ErreurBase> {
    let (cabinet, dossier) = tenant(tx)?;
    let mut tries = ids.to_vec();
    tries.sort_unstable();
    let mut rendues = Vec::with_capacity(tries.len());
    for id in tries {
        let l = sqlx::query!(
            "SELECT empreinte, ecartee_le IS NOT NULL AS \"ecartee!\" FROM piece \
             WHERE cabinet_id = $1 AND dossier_id = $2 AND id = $3 FOR SHARE",
            cabinet.uuid(),
            dossier.uuid(),
            id,
        )
        .fetch_optional(tx.connexion())
        .await?;
        if let Some(l) = l {
            rendues.push(PieceVerrouillee {
                id,
                empreinte: hex(&l.empreinte),
                ecartee: l.ecartee,
            });
        }
    }
    Ok(rendues)
}

/// Tête de chaîne du dossier : dernier rang et dernière empreinte.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tete {
    /// Rang de la dernière écriture chaînée (0 : chaîne vide).
    pub rang: i64,
    /// Empreinte de tête (`integrite::genese` pour une chaîne vide).
    pub empreinte: Empreinte,
}

/// Verrouille `FOR UPDATE` la tête de chaîne du dossier, créée à la demande : `INSERT … ON
/// CONFLICT DO NOTHING` (rang 0, empreinte de départ), puis `SELECT … FOR UPDATE`. Sans cela, deux
/// transactions partiraient de zéro.
///
/// # Errors
/// Contexte sans dossier, empreinte lue invalide, erreur de PostgreSQL.
pub async fn verrouiller_tete(
    tx: &mut TransactionTenant,
    genese: &Empreinte,
) -> Result<Tete, ErreurBase> {
    let (cabinet, dossier) = tenant(tx)?;
    sqlx::query!(
        "INSERT INTO chaine_tete (cabinet_id, dossier_id, rang, empreinte) \
         VALUES ($1, $2, 0, $3) ON CONFLICT DO NOTHING",
        cabinet.uuid(),
        dossier.uuid(),
        genese.octets().as_slice(),
    )
    .execute(tx.connexion())
    .await?;
    let l = sqlx::query!(
        "SELECT rang, empreinte FROM chaine_tete WHERE cabinet_id = $1 AND dossier_id = $2 \
         FOR UPDATE",
        cabinet.uuid(),
        dossier.uuid(),
    )
    .fetch_one(tx.connexion())
    .await?;
    Ok(Tete {
        rang: l.rang,
        empreinte: empreinte_lue(&l.empreinte)?,
    })
}

/// Verrouille `FOR UPDATE` le compteur d'un journal pour un exercice, créé à la demande ; rend la
/// dernière séquence attribuée.
///
/// # Errors
/// Contexte sans dossier, erreur de PostgreSQL.
pub async fn verrouiller_compteur(
    tx: &mut TransactionTenant,
    journal: Uuid,
    exercice: Uuid,
) -> Result<u32, ErreurBase> {
    let (cabinet, dossier) = tenant(tx)?;
    sqlx::query!(
        "INSERT INTO compteur_ecriture (cabinet_id, dossier_id, journal_id, exercice_id, dernier) \
         VALUES ($1, $2, $3, $4, 0) ON CONFLICT DO NOTHING",
        cabinet.uuid(),
        dossier.uuid(),
        journal,
        exercice,
    )
    .execute(tx.connexion())
    .await?;
    let dernier = sqlx::query_scalar!(
        "SELECT dernier FROM compteur_ecriture \
         WHERE cabinet_id = $1 AND dossier_id = $2 AND journal_id = $3 AND exercice_id = $4 \
         FOR UPDATE",
        cabinet.uuid(),
        dossier.uuid(),
        journal,
        exercice,
    )
    .fetch_one(tx.connexion())
    .await?;
    u32::try_from(dernier).map_err(|_| ErreurBase::Donnee("compteur négatif"))
}

// --- Inscription ------------------------------------------------------------------------

/// Ce qui s'inscrit à la validation : tiré d'une [`Validation`] (seule preuve de validation) et
/// des éléments de la transaction.
#[derive(Debug)]
pub struct Inscription {
    id: Uuid,
    journal_id: Uuid,
    exercice_id: Uuid,
    ecriture: EcritureValidee,
    lignes: Vec<LigneStockee>,
    format: i16,
    canonique: String,
    empreinte: Empreinte,
    precedente: Empreinte,
    rang: i64,
    valide_par: Uuid,
    valide_le: OffsetDateTime,
    pieces: Vec<(Uuid, RolePiece)>,
    origine: Option<Uuid>,
}

/// Éléments de la transaction qui accompagnent la validation.
#[derive(Debug)]
pub struct Circonstances {
    /// Identifiant du brouillon, repris par l'écriture validée.
    pub id: Uuid,
    /// Journal.
    pub journal_id: Uuid,
    /// Exercice de la date de l'écriture.
    pub exercice_id: Uuid,
    /// Contenu canonique haché.
    pub canonique: String,
    /// Version du format canonique (1, 2 ou 3).
    pub format: i16,
    /// Empreinte de l'écriture.
    pub empreinte: Empreinte,
    /// Empreinte de la tête lue sous verrou.
    pub precedente: Empreinte,
    /// Rang attribué.
    pub rang: i64,
    /// Validateur.
    pub valide_par: Uuid,
    /// Instant de la validation.
    pub valide_le: OffsetDateTime,
    /// Liens de pièces, dans l'ordre de la justification hachée.
    pub pieces: Vec<(Uuid, RolePiece)>,
    /// Pour une contre-passation : écriture d'origine (PR 13c) ; `None` pour une validation.
    pub origine: Option<Uuid>,
}

impl Inscription {
    /// Prépare l'inscription d'une validation.
    ///
    /// # Errors
    /// Tiers d'une ligne qui n'est pas un UUID.
    pub fn depuis(validation: &Validation, c: Circonstances) -> Result<Self, ErreurBase> {
        let ecriture = validation.ecriture().clone();
        let lignes = ecriture
            .lignes()
            .iter()
            .map(LigneStockee::depuis)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            id: c.id,
            journal_id: c.journal_id,
            exercice_id: c.exercice_id,
            ecriture,
            lignes,
            format: c.format,
            canonique: c.canonique,
            empreinte: c.empreinte,
            precedente: c.precedente,
            rang: c.rang,
            valide_par: c.valide_par,
            valide_le: c.valide_le,
            pieces: c.pieces,
            origine: c.origine,
        })
    }
}

/// Insère l'écriture validée, ses lignes et ses liens de pièces. Ni la tête ni le compteur :
/// voir [`mettre_a_jour_tete`] et [`mettre_a_jour_compteur`].
///
/// # Errors
/// Contexte sans dossier, contrainte refusée par PostgreSQL (unicité du rang ou du numéro :
/// filet contre un défaut de verrou).
pub async fn inserer(tx: &mut TransactionTenant, i: &Inscription) -> Result<(), ErreurBase> {
    let (cabinet, dossier) = tenant(tx)?;
    let e = &i.ecriture;
    let sequence = i32::try_from(e.numero().sequence())
        .map_err(|_| ErreurBase::Donnee("séquence hors borne"))?;
    sqlx::query!(
        r#"
        INSERT INTO ecriture_validee
            (cabinet_id, dossier_id, id, journal_id, exercice_id, sequence, numero,
             date_comptable, date_valeur_comptable, libelle, version_parametres, format,
             contenu_canonique, empreinte, empreinte_precedente, rang, valide_par,
             date_validation, valide_le, origine_id)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17,
                $18, $19, $20)
        "#,
        cabinet.uuid(),
        dossier.uuid(),
        i.id,
        i.journal_id,
        i.exercice_id,
        sequence,
        e.numero().to_string(),
        e.date(),
        e.date_valeur_comptable(),
        e.libelle().as_str(),
        e.version_parametres()
            .map(compta_core::VersionParametres::as_str),
        i.format,
        i.canonique,
        i.empreinte.octets().as_slice(),
        i.precedente.octets().as_slice(),
        i.rang,
        i.valide_par,
        e.date_validation(),
        i.valide_le,
        i.origine,
    )
    .execute(tx.connexion())
    .await?;
    for (indice, l) in i.lignes.iter().enumerate() {
        let position = i16::try_from(indice + 1).map_err(|_| ErreurBase::Donnee("position"))?;
        sqlx::query!(
            r#"
            INSERT INTO ecriture_validee_ligne
                (cabinet_id, dossier_id, ecriture_id, position, compte_numero, sens, montant,
                 devise, montant_devise, cours, tiers_id,
                 fiscal_nature, fiscal_sens, fiscal_regime, fiscal_taux, fiscal_exigibilite,
                 fiscal_motif)
            VALUES ($1, $2, $3, $4, $5, $6, $7::text::numeric,
                    $8, $9::text::numeric, $10::text::numeric, $11,
                    $12, $13, $14, $15::text::numeric, $16, $17)
            "#,
            cabinet.uuid(),
            dossier.uuid(),
            i.id,
            position,
            l.compte,
            l.sens,
            l.montant,
            l.devise.as_ref().map(|d| d.code.as_str()),
            l.devise.as_ref().map(|d| d.montant.as_str()),
            l.devise.as_ref().map(|d| d.cours.as_str()),
            l.tiers,
            l.etiquette.as_ref().map(|x| x.nature.as_str()),
            l.etiquette.as_ref().map(|x| x.sens.as_str()),
            l.etiquette.as_ref().map(|x| x.regime.as_str()),
            l.etiquette.as_ref().and_then(|x| x.taux.as_deref()),
            l.etiquette.as_ref().and_then(|x| x.exigibilite.as_deref()),
            l.etiquette.as_ref().and_then(|x| x.motif.as_deref()),
        )
        .execute(tx.connexion())
        .await?;
    }
    for (indice, (piece, role)) in i.pieces.iter().enumerate() {
        let position = i16::try_from(indice + 1).map_err(|_| ErreurBase::Donnee("position"))?;
        sqlx::query!(
            "INSERT INTO ecriture_validee_piece \
                 (cabinet_id, dossier_id, ecriture_id, piece_id, position, role) \
             VALUES ($1, $2, $3, $4, $5, $6)",
            cabinet.uuid(),
            dossier.uuid(),
            i.id,
            piece,
            position,
            role.nom(),
        )
        .execute(tx.connexion())
        .await?;
    }
    Ok(())
}

/// Écrit la nouvelle tête de chaîne (tête verrouillée par [`verrouiller_tete`]).
///
/// # Errors
/// Contexte sans dossier, tête absente, erreur de PostgreSQL.
pub async fn mettre_a_jour_tete(
    tx: &mut TransactionTenant,
    rang: i64,
    empreinte: &Empreinte,
) -> Result<(), ErreurBase> {
    let (cabinet, dossier) = tenant(tx)?;
    let n = sqlx::query!(
        "UPDATE chaine_tete SET rang = $3, empreinte = $4 \
         WHERE cabinet_id = $1 AND dossier_id = $2",
        cabinet.uuid(),
        dossier.uuid(),
        rang,
        empreinte.octets().as_slice(),
    )
    .execute(tx.connexion())
    .await?
    .rows_affected();
    if n != 1 {
        return Err(ErreurBase::Donnee("tête de chaîne absente"));
    }
    // Horodatage quotidien (ADR 0010) : la chaîne a changé, un jeton sera demandé au début du
    // jour suivant. Même transaction que la validation : la tâche n'existe que si elle aboutit.
    super::taches::planifier_horodatage(tx, super::taches::SujetHorodatage::Quotidien).await?;
    Ok(())
}

/// Écrit le nouveau compteur (verrouillé par [`verrouiller_compteur`]).
///
/// # Errors
/// Contexte sans dossier, compteur absent, erreur de PostgreSQL.
pub async fn mettre_a_jour_compteur(
    tx: &mut TransactionTenant,
    journal: Uuid,
    exercice: Uuid,
    dernier: u32,
) -> Result<(), ErreurBase> {
    let (cabinet, dossier) = tenant(tx)?;
    let dernier = i32::try_from(dernier).map_err(|_| ErreurBase::Donnee("compteur hors borne"))?;
    let n = sqlx::query!(
        "UPDATE compteur_ecriture SET dernier = $5 \
         WHERE cabinet_id = $1 AND dossier_id = $2 AND journal_id = $3 AND exercice_id = $4",
        cabinet.uuid(),
        dossier.uuid(),
        journal,
        exercice,
        dernier,
    )
    .execute(tx.connexion())
    .await?
    .rows_affected();
    if n == 1 {
        Ok(())
    } else {
        Err(ErreurBase::Donnee("compteur absent"))
    }
}

/// Supprime le brouillon validé et ses dépendances (lignes, liens de pièces, contributeurs) :
/// voir [`super::brouillard::supprimer`], qui prend le brouillon déjà verrouillé.
///
/// # Errors
/// Voir [`super::brouillard::supprimer`].
pub async fn supprimer_brouillon(tx: &mut TransactionTenant, id: Uuid) -> Result<(), ErreurBase> {
    super::brouillard::supprimer(tx, id).await
}

// --- Lecture ----------------------------------------------------------------------------

/// Lien de pièce d'une écriture validée.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LienValide {
    /// Pièce.
    pub piece: Uuid,
    /// Rôle.
    pub role: RolePiece,
    /// Empreinte de la pièce, hexadécimale minuscule.
    pub empreinte: String,
}

/// Écriture validée relue, colonnes comprises.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EcritureStockee {
    /// Identifiant (celui du brouillon d'origine).
    pub id: Uuid,
    /// Numéro `AC-2026-000231`.
    pub numero: String,
    /// Code du journal.
    pub journal: String,
    /// Code de l'exercice.
    pub exercice: String,
    /// Date comptable.
    pub date: Date,
    /// Date de valeur comptable.
    pub date_valeur_comptable: Option<Date>,
    /// Libellé.
    pub libelle: String,
    /// Version des paramètres fiscaux.
    pub version_parametres: Option<String>,
    /// Version du format canonique.
    pub format: i16,
    /// Contenu canonique haché.
    pub canonique: String,
    /// Empreinte.
    pub empreinte: Empreinte,
    /// Empreinte précédente.
    pub precedente: Empreinte,
    /// Rang dans la chaîne du dossier.
    pub rang: i64,
    /// Validateur.
    pub valide_par: Uuid,
    /// Date de validation.
    pub date_validation: Date,
    /// Instant de la validation.
    pub valide_le: OffsetDateTime,
    /// Origine (contre-passation, PR 13c).
    pub origine: Option<Uuid>,
    /// Numéro de l'écriture d'origine (contre-passation), cité par le contenu haché.
    pub origine_numero: Option<String>,
    /// Numéro de la contre-passation qui cite cette écriture, s'il y en a une.
    pub contre_passee_par: Option<String>,
    /// Lignes.
    pub lignes: Vec<LigneStockee>,
    /// Pièces, dans l'ordre de la justification hachée.
    pub pieces: Vec<LienValide>,
}

/// Résumé d'une écriture validée pour la liste.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResumeEcriture {
    /// Identifiant.
    pub id: Uuid,
    /// Numéro.
    pub numero: String,
    /// Date comptable.
    pub date: Date,
    /// Libellé.
    pub libelle: String,
    /// Rang de chaîne.
    pub rang: i64,
    /// Empreinte.
    pub empreinte: Empreinte,
    /// Total des débits, texte décimal canonique.
    pub total: String,
}

/// Lit une écriture validée du dossier du contexte.
///
/// # Errors
/// Contexte sans dossier, données incohérentes, erreur de PostgreSQL.
#[allow(clippy::too_many_lines)]
pub async fn lire(
    tx: &mut TransactionTenant,
    id: Uuid,
) -> Result<Option<EcritureStockee>, ErreurBase> {
    let (cabinet, dossier) = tenant(tx)?;
    let Some(e) = sqlx::query!(
        r#"
        SELECT e.numero, j.code AS journal, x.code AS exercice, e.date_comptable,
               e.date_valeur_comptable, e.libelle, e.version_parametres, e.format,
               e.contenu_canonique, e.empreinte, e.empreinte_precedente, e.rang, e.valide_par,
               e.date_validation, e.valide_le, e.origine_id, o.numero AS "origine_numero?",
               (SELECT c.numero FROM ecriture_validee c
                 WHERE c.cabinet_id = e.cabinet_id AND c.dossier_id = e.dossier_id
                   AND c.origine_id = e.id) AS "contre_passee_par?"
        FROM ecriture_validee e
        LEFT JOIN ecriture_validee o ON o.cabinet_id = e.cabinet_id AND o.dossier_id = e.dossier_id
                                    AND o.id = e.origine_id
        JOIN journal j ON j.cabinet_id = e.cabinet_id AND j.dossier_id = e.dossier_id
                      AND j.id = e.journal_id
        JOIN exercice x ON x.cabinet_id = e.cabinet_id AND x.dossier_id = e.dossier_id
                       AND x.id = e.exercice_id
        WHERE e.cabinet_id = $1 AND e.dossier_id = $2 AND e.id = $3
        "#,
        cabinet.uuid(),
        dossier.uuid(),
        id,
    )
    .fetch_optional(tx.connexion())
    .await?
    else {
        return Ok(None);
    };
    let lignes = sqlx::query!(
        r#"
        SELECT compte_numero, sens, montant::text AS "montant!", devise,
               montant_devise::text AS montant_devise, cours::text AS cours, tiers_id,
               fiscal_nature, fiscal_sens, fiscal_regime, fiscal_taux::text AS fiscal_taux,
               fiscal_exigibilite, fiscal_motif
        FROM ecriture_validee_ligne
        WHERE cabinet_id = $1 AND dossier_id = $2 AND ecriture_id = $3
        ORDER BY position
        "#,
        cabinet.uuid(),
        dossier.uuid(),
        id,
    )
    .fetch_all(tx.connexion())
    .await?;
    let incoherent = ErreurBase::Donnee("ligne d'écriture validée incohérente");
    let mut relues = Vec::with_capacity(lignes.len());
    for l in lignes {
        let devise = match (l.devise, l.montant_devise, l.cours) {
            (None, None, None) => None,
            (Some(code), Some(montant), Some(cours)) => Some(DeviseStockee {
                code,
                montant: decimal_canonique(&montant),
                cours: decimal_canonique(&cours),
            }),
            _ => return Err(incoherent),
        };
        let etiquette = match (l.fiscal_nature, l.fiscal_sens, l.fiscal_regime) {
            (None, None, None) => None,
            (Some(nature), Some(sens), Some(regime)) => Some(EtiquetteStockee {
                nature,
                sens,
                regime,
                taux: l.fiscal_taux.map(|t| decimal_canonique(&t)),
                exigibilite: l.fiscal_exigibilite,
                motif: l.fiscal_motif,
            }),
            _ => return Err(incoherent),
        };
        let sens = match l.sens.as_str() {
            "debit" => "debit",
            "credit" => "credit",
            _ => return Err(incoherent),
        };
        relues.push(LigneStockee {
            compte: l.compte_numero,
            sens,
            montant: decimal_canonique(&l.montant),
            devise,
            tiers: l.tiers_id,
            etiquette,
        });
    }
    let liens = sqlx::query!(
        "SELECT v.piece_id, v.role, p.empreinte FROM ecriture_validee_piece v \
         JOIN piece p ON p.cabinet_id = v.cabinet_id AND p.dossier_id = v.dossier_id \
                     AND p.id = v.piece_id \
         WHERE v.cabinet_id = $1 AND v.dossier_id = $2 AND v.ecriture_id = $3 \
         ORDER BY v.position",
        cabinet.uuid(),
        dossier.uuid(),
        id,
    )
    .fetch_all(tx.connexion())
    .await?;
    let pieces = liens
        .into_iter()
        .map(|l| {
            Ok(LienValide {
                piece: l.piece_id,
                role: RolePiece::depuis_nom(&l.role)
                    .ok_or(ErreurBase::Donnee("rôle d'une pièce liée"))?,
                empreinte: hex(&l.empreinte),
            })
        })
        .collect::<Result<Vec<_>, ErreurBase>>()?;
    Ok(Some(EcritureStockee {
        id,
        numero: e.numero,
        journal: e.journal,
        exercice: e.exercice,
        date: e.date_comptable,
        date_valeur_comptable: e.date_valeur_comptable,
        libelle: e.libelle,
        version_parametres: e.version_parametres,
        format: e.format,
        canonique: e.contenu_canonique,
        empreinte: empreinte_lue(&e.empreinte)?,
        precedente: empreinte_lue(&e.empreinte_precedente)?,
        rang: e.rang,
        valide_par: e.valide_par,
        date_validation: e.date_validation,
        valide_le: e.valide_le,
        origine: e.origine_id,
        origine_numero: e.origine_numero,
        contre_passee_par: e.contre_passee_par,
        lignes: relues,
        pieces,
    }))
}

/// Écritures du dossier du contexte, dans l'ordre (date comptable, numéro), après le curseur
/// strict, au plus `limite` (l'appelant demande `limite + 1`).
///
/// # Errors
/// Contexte sans dossier, empreinte invalide, erreur de PostgreSQL.
pub async fn lister(
    tx: &mut TransactionTenant,
    apres: Option<(Date, &str)>,
    limite: u32,
) -> Result<Vec<ResumeEcriture>, ErreurBase> {
    let (cabinet, dossier) = tenant(tx)?;
    let lignes = sqlx::query!(
        r#"
        SELECT e.id, e.numero, e.date_comptable, e.libelle, e.rang, e.empreinte,
               (SELECT COALESCE(sum(l.montant), 0)::text FROM ecriture_validee_ligne l
                 WHERE l.cabinet_id = e.cabinet_id AND l.dossier_id = e.dossier_id
                   AND l.ecriture_id = e.id AND l.sens = 'debit') AS "total!"
        FROM ecriture_validee e
        WHERE e.cabinet_id = $1 AND e.dossier_id = $2
          AND ($3::date IS NULL OR (e.date_comptable, e.numero) > ($3, $4))
        ORDER BY e.date_comptable, e.numero
        LIMIT $5
        "#,
        cabinet.uuid(),
        dossier.uuid(),
        apres.map(|(d, _)| d),
        apres.map(|(_, n)| n),
        i64::from(limite),
    )
    .fetch_all(tx.connexion())
    .await?;
    lignes
        .into_iter()
        .map(|l| {
            Ok(ResumeEcriture {
                id: l.id,
                numero: l.numero,
                date: l.date_comptable,
                libelle: l.libelle,
                rang: l.rang,
                empreinte: empreinte_lue(&l.empreinte)?,
                total: decimal_canonique(&l.total),
            })
        })
        .collect()
}

// --- Contre-passation (PR 13c) -----------------------------------------------------------

/// Vrai si l'écriture validée existe dans le dossier du contexte (sous RLS : celle d'un autre
/// dossier ou d'un autre cabinet est absente).
///
/// # Errors
/// Contexte sans dossier, erreur de PostgreSQL.
pub async fn existe(tx: &mut TransactionTenant, id: Uuid) -> Result<bool, ErreurBase> {
    let (cabinet, dossier) = tenant(tx)?;
    Ok(sqlx::query_scalar!(
        r#"SELECT EXISTS (SELECT 1 FROM ecriture_validee
                          WHERE cabinet_id = $1 AND dossier_id = $2 AND id = $3) AS "existe!""#,
        cabinet.uuid(),
        dossier.uuid(),
        id,
    )
    .fetch_one(tx.connexion())
    .await?)
}

/// Numéro de la contre-passation qui cite `origine`, s'il y en a une. À lire **après** le verrou
/// de la tête de chaîne : deux contre-passations de la même origine sont alors sérialisées.
///
/// # Errors
/// Contexte sans dossier, erreur de PostgreSQL.
pub async fn contre_passee_par(
    tx: &mut TransactionTenant,
    origine: Uuid,
) -> Result<Option<String>, ErreurBase> {
    let (cabinet, dossier) = tenant(tx)?;
    Ok(sqlx::query_scalar!(
        "SELECT numero FROM ecriture_validee \
         WHERE cabinet_id = $1 AND dossier_id = $2 AND origine_id = $3",
        cabinet.uuid(),
        dossier.uuid(),
        origine,
    )
    .fetch_optional(tx.connexion())
    .await?)
}

/// Pièces liées à une écriture validée (identifiants), pour les verrouiller avant la tête de
/// chaîne : les liens d'une écriture validée sont définitifs, leur lecture sans verrou est sûre.
///
/// # Errors
/// Contexte sans dossier, erreur de PostgreSQL.
pub async fn pieces_de(tx: &mut TransactionTenant, id: Uuid) -> Result<Vec<Uuid>, ErreurBase> {
    let (cabinet, dossier) = tenant(tx)?;
    Ok(sqlx::query_scalar!(
        "SELECT piece_id FROM ecriture_validee_piece \
         WHERE cabinet_id = $1 AND dossier_id = $2 AND ecriture_id = $3",
        cabinet.uuid(),
        dossier.uuid(),
        id,
    )
    .fetch_all(tx.connexion())
    .await?)
}

/// Vrai si l'erreur est la violation de l'unicité de l'origine : l'origine est déjà contre-passée
/// (filet de la sérialisation par la tête de chaîne, `0204_contre_passation.sql`).
#[must_use]
pub fn est_double_contre_passation(erreur: &ErreurBase) -> bool {
    matches!(
        erreur,
        ErreurBase::Sqlx(sqlx::Error::Database(d))
            if d.constraint() == Some("ecriture_validee_origine_unique")
    )
}
