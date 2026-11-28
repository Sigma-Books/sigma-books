//! Fiche des tiers en base (lot L3, PR 7a, L3-15 ; ADR 0031 ; migrations `0500` à `0504`).
//!
//! Seul module qui écrit dans `tiers` et ses tables de fiche (`tiers_identifiant`,
//! `tiers_adresse`, `tiers_contact`, `tiers_compte_bancaire`). Les contrôles de forme et de pays
//! sont faits par la route (`serveur::routes::tiers`) ; ici : persistance, verrous, doublons.
//!
//! # Verrous
//! - **Création, désarchivage, changement de type ou de compte collectif** : verrou du référentiel (ligne `dossier` en `FOR NO KEY
//!   UPDATE`, [`referentiel::verrouiller`]), puis contrôle croisé du référentiel
//!   ([`referentiel::verifier`] : compte collectif existant, collectif, actif, qui admet le type du
//!   tiers), comme toute mutation du référentiel de L2. Le code tiré d'office l'est sous ce verrou.
//! - **Toute autre mutation d'un tiers** : la ligne du tiers en `FOR NO KEY UPDATE`
//!   ([`verrouiller`]) : deux modifications d'une même fiche se suivent, sans bloquer la saisie
//!   (le brouillard ne prend que `FOR KEY SHARE` par sa clé étrangère, et la validation
//!   `FOR SHARE`, qui attend seulement la fin de la modification).
//!
//! Niveau « requête » de l'isolation (ADR 0011) : chaque requête filtre par le cabinet et le
//! dossier du contexte, en plus de la RLS.

use time::OffsetDateTime;
use uuid::Uuid;

use super::referentiel::{self, RefusReferentiel, Tiers, TypeTiers};
use super::{ErreurBase, TransactionTenant};

/// Contexte (cabinet, dossier) de la transaction.
fn contexte(tx: &TransactionTenant) -> Result<(Uuid, Uuid), ErreurBase> {
    let dossier = tx
        .contexte()
        .id_dossier()
        .ok_or(ErreurBase::Donnee("tiers hors du contexte d'un dossier"))?;
    Ok((tx.contexte().id_cabinet().uuid(), dossier.uuid()))
}

/// En-tête d'un tiers : la ligne de `tiers`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnteteTiers {
    /// Identifiant.
    pub id: Uuid,
    /// Code auxiliaire, unique dans le dossier, immuable.
    pub code: String,
    /// Nom.
    pub nom: String,
    /// Type (figé).
    pub genre: TypeTiers,
    /// Compte collectif (figé).
    pub compte_collectif: String,
    /// Faux pour un tiers archivé.
    pub actif: bool,
    /// Devise par défaut de saisie.
    pub devise: Option<String>,
    /// Assujetti à la TVA (indicateur sans effet comptable en L3).
    pub assujetti_tva: bool,
    /// Soumis à retenue (indicateur sans effet comptable en L3).
    pub soumis_retenue: bool,
    /// Motif de l'archivage en cours.
    pub motif_archivage: Option<String>,
    /// Instant de l'archivage en cours.
    pub archive_le: Option<OffsetDateTime>,
    /// Condition de règlement par défaut de saisie (ADR 0031 point 4, ADR 0033 ; migration
    /// `0511`), modifiée par [`changer_condition_reglement`].
    pub condition_reglement: Option<Uuid>,
}

/// Identifiant légal d'un tiers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identifiant {
    /// Identifiant de la ligne.
    pub id: Uuid,
    /// Type (code du pack du pays du dossier).
    pub type_identifiant: String,
    /// Valeur.
    pub valeur: String,
}

/// Adresse d'un tiers : tous les champs sont facultatifs.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Adresse {
    /// Libellé (« Siège », « Entrepôt »…).
    pub libelle: Option<String>,
    /// Rue.
    pub rue: Option<String>,
    /// Code postal.
    pub code_postal: Option<String>,
    /// Ville.
    pub ville: Option<String>,
    /// Région, subdivision.
    pub region: Option<String>,
    /// Pays (ISO 3166-1 alpha-2, forme seule).
    pub pays: Option<String>,
}

/// Moyen de contact d'un tiers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Contact {
    /// Type (`telephone`, `mobile`, `fax`, `courriel`, `site_web`, `autre`).
    pub type_contact: String,
    /// Valeur.
    pub valeur: String,
    /// Personne à joindre.
    pub nom: Option<String>,
}

/// Coordonnée bancaire d'un tiers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompteBancaire {
    /// Type (code d'identifiant bancaire du pack).
    pub type_identifiant: String,
    /// Numéro.
    pub numero: String,
    /// Établissement.
    pub etablissement: Option<String>,
    /// Devise (ISO 4217, forme seule).
    pub devise: Option<String>,
}

/// Fiche complète d'un tiers, **sans** ses coordonnées bancaires (lues à part : Cedar les
/// refuse à l'auditeur).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FicheTiers {
    /// En-tête.
    pub entete: EnteteTiers,
    /// Identifiants légaux, par type.
    pub identifiants: Vec<Identifiant>,
    /// Adresses, par ordre de création.
    pub adresses: Vec<(Uuid, Adresse)>,
    /// Moyens de contact, par ordre de création.
    pub contacts: Vec<(Uuid, Contact)>,
}

/// Nouveau tiers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NouveauTiers {
    /// Identifiant tiré par l'appelant.
    pub id: Uuid,
    /// Code demandé ; tiré d'office (`T` et six chiffres au moins) s'il est absent.
    pub code: Option<String>,
    /// Nom.
    pub nom: compta_core::TexteCourt,
    /// Type.
    pub genre: TypeTiers,
    /// Compte collectif.
    pub compte_collectif: compta_core::NumeroCompte,
    /// Devise par défaut.
    pub devise: Option<String>,
    /// Assujetti à la TVA.
    pub assujetti_tva: bool,
    /// Soumis à retenue.
    pub soumis_retenue: bool,
}

/// Échec d'une mutation d'un tiers.
#[derive(Debug, thiserror::Error)]
pub enum ErreurTiers {
    /// Le dossier du contexte n'a pas de référentiel, ou le tiers n'y est pas.
    #[error("tiers absent")]
    Absent,
    /// Contrôle croisé du référentiel refusé (compte collectif, type).
    #[error(transparent)]
    Refus(#[from] RefusReferentiel),
    /// Code déjà pris par un autre tiers du dossier.
    #[error("code de tiers déjà utilisé")]
    CodeDejaUtilise,
    /// Aucun code ne peut être tiré d'office : le code est à saisir.
    #[error("aucun code de tiers ne peut être tiré d'office")]
    TirageImpossible,
    /// Type ou compte collectif demandé pour un tiers déjà cité par une ligne.
    #[error("tiers déjà mouvementé")]
    DejaMouvemente,
    /// Erreur d'accès à la base.
    #[error(transparent)]
    Base(#[from] ErreurBase),
}

impl From<sqlx::Error> for ErreurTiers {
    fn from(erreur: sqlx::Error) -> Self {
        Self::Base(ErreurBase::Sqlx(erreur))
    }
}

fn genre_lu(texte: &str) -> Result<TypeTiers, ErreurBase> {
    TypeTiers::parse(texte).ok_or(ErreurBase::Donnee("type de tiers"))
}

// --- Lectures --------------------------------------------------------------------------------

/// Une page de tiers, triés par (nom, identifiant), après le tiers `apres`. `None` si `apres`
/// n'est pas un tiers du dossier.
///
/// # Errors
/// Contexte sans dossier, erreur de PostgreSQL.
pub async fn lister(
    tx: &mut TransactionTenant,
    apres: Option<Uuid>,
    limite: i64,
) -> Result<Option<Vec<EnteteTiers>>, ErreurBase> {
    let (cabinet, dossier) = contexte(tx)?;
    let curseur = match apres {
        None => None,
        Some(id) => {
            let Some(ligne) = sqlx::query!(
                "SELECT nom FROM tiers WHERE cabinet_id = $1 AND dossier_id = $2 AND id = $3",
                cabinet,
                dossier,
                id,
            )
            .fetch_optional(tx.connexion())
            .await?
            else {
                return Ok(None);
            };
            Some((ligne.nom, id))
        }
    };
    let (nom, id) = curseur.unzip();
    let lignes = sqlx::query!(
        "SELECT id, code, nom, type, compte_collectif, actif, devise, assujetti_tva, \
                soumis_retenue, motif_archivage, archive_le, condition_reglement_id \
         FROM tiers \
         WHERE cabinet_id = $1 AND dossier_id = $2 \
           AND ($3::text IS NULL OR (nom, id) > ($3, $4::uuid)) \
         ORDER BY nom, id \
         LIMIT $5",
        cabinet,
        dossier,
        nom,
        id,
        limite,
    )
    .fetch_all(tx.connexion())
    .await?;
    lignes
        .into_iter()
        .map(|l| {
            Ok(EnteteTiers {
                id: l.id,
                code: l.code,
                nom: l.nom,
                genre: genre_lu(&l.r#type)?,
                compte_collectif: l.compte_collectif,
                actif: l.actif,
                devise: l.devise,
                assujetti_tva: l.assujetti_tva,
                soumis_retenue: l.soumis_retenue,
                motif_archivage: l.motif_archivage,
                archive_le: l.archive_le,
                condition_reglement: l.condition_reglement_id,
            })
        })
        .collect::<Result<Vec<_>, ErreurBase>>()
        .map(Some)
}

/// En-tête du tiers `id` du dossier du contexte.
///
/// # Errors
/// Contexte sans dossier, erreur de PostgreSQL.
pub async fn lire_entete(
    tx: &mut TransactionTenant,
    id: Uuid,
) -> Result<Option<EnteteTiers>, ErreurBase> {
    let (cabinet, dossier) = contexte(tx)?;
    let ligne = sqlx::query!(
        "SELECT id, code, nom, type, compte_collectif, actif, devise, assujetti_tva, \
                soumis_retenue, motif_archivage, archive_le, condition_reglement_id \
         FROM tiers WHERE cabinet_id = $1 AND dossier_id = $2 AND id = $3",
        cabinet,
        dossier,
        id,
    )
    .fetch_optional(tx.connexion())
    .await?;
    ligne
        .map(|l| {
            Ok(EnteteTiers {
                id: l.id,
                code: l.code,
                nom: l.nom,
                genre: genre_lu(&l.r#type)?,
                compte_collectif: l.compte_collectif,
                actif: l.actif,
                devise: l.devise,
                assujetti_tva: l.assujetti_tva,
                soumis_retenue: l.soumis_retenue,
                motif_archivage: l.motif_archivage,
                archive_le: l.archive_le,
                condition_reglement: l.condition_reglement_id,
            })
        })
        .transpose()
}

/// Fiche du tiers `id` (sans coordonnées bancaires).
///
/// # Errors
/// Contexte sans dossier, erreur de PostgreSQL.
pub async fn lire_fiche(
    tx: &mut TransactionTenant,
    id: Uuid,
) -> Result<Option<FicheTiers>, ErreurBase> {
    let (cabinet, dossier) = contexte(tx)?;
    let Some(entete) = lire_entete(tx, id).await? else {
        return Ok(None);
    };
    let identifiants = sqlx::query!(
        "SELECT id, type, valeur FROM tiers_identifiant \
         WHERE cabinet_id = $1 AND dossier_id = $2 AND tiers_id = $3 ORDER BY type",
        cabinet,
        dossier,
        id,
    )
    .fetch_all(tx.connexion())
    .await?
    .into_iter()
    .map(|l| Identifiant {
        id: l.id,
        type_identifiant: l.r#type,
        valeur: l.valeur,
    })
    .collect();
    let adresses = sqlx::query!(
        "SELECT id, libelle, rue, code_postal, ville, region, pays FROM tiers_adresse \
         WHERE cabinet_id = $1 AND dossier_id = $2 AND tiers_id = $3 ORDER BY cree_le, id",
        cabinet,
        dossier,
        id,
    )
    .fetch_all(tx.connexion())
    .await?
    .into_iter()
    .map(|l| {
        (
            l.id,
            Adresse {
                libelle: l.libelle,
                rue: l.rue,
                code_postal: l.code_postal,
                ville: l.ville,
                region: l.region,
                pays: l.pays,
            },
        )
    })
    .collect();
    let contacts = sqlx::query!(
        "SELECT id, type, valeur, nom FROM tiers_contact \
         WHERE cabinet_id = $1 AND dossier_id = $2 AND tiers_id = $3 ORDER BY cree_le, id",
        cabinet,
        dossier,
        id,
    )
    .fetch_all(tx.connexion())
    .await?
    .into_iter()
    .map(|l| {
        (
            l.id,
            Contact {
                type_contact: l.r#type,
                valeur: l.valeur,
                nom: l.nom,
            },
        )
    })
    .collect();
    Ok(Some(FicheTiers {
        entete,
        identifiants,
        adresses,
        contacts,
    }))
}

/// Coordonnées bancaires du tiers `tiers`, par ordre de création.
///
/// # Errors
/// Contexte sans dossier, erreur de PostgreSQL.
pub async fn lire_comptes_bancaires(
    tx: &mut TransactionTenant,
    tiers: Uuid,
) -> Result<Vec<(Uuid, CompteBancaire)>, ErreurBase> {
    let (cabinet, dossier) = contexte(tx)?;
    Ok(sqlx::query!(
        "SELECT id, type, numero, etablissement, devise FROM tiers_compte_bancaire \
         WHERE cabinet_id = $1 AND dossier_id = $2 AND tiers_id = $3 ORDER BY cree_le, id",
        cabinet,
        dossier,
        tiers,
    )
    .fetch_all(tx.connexion())
    .await?
    .into_iter()
    .map(|l| {
        (
            l.id,
            CompteBancaire {
                type_identifiant: l.r#type,
                numero: l.numero,
                etablissement: l.etablissement,
                devise: l.devise,
            },
        )
    })
    .collect())
}

/// Doublons des identifiants légaux du tiers : pour chacun de ses types, les **autres** tiers du
/// dossier qui portent le même type et la même valeur (majuscules, sans blanc). Signalés,
/// jamais refusés (Tryton `party/party.py:1061-1078`).
///
/// # Errors
/// Contexte sans dossier, erreur de PostgreSQL.
pub async fn doublons_identifiants(
    tx: &mut TransactionTenant,
    tiers: Uuid,
) -> Result<Vec<(String, Vec<Uuid>)>, ErreurBase> {
    let (cabinet, dossier) = contexte(tx)?;
    let lignes = sqlx::query!(
        r#"SELECT i.type AS "type!", array_agg(DISTINCT a.tiers_id ORDER BY a.tiers_id) AS "autres!"
           FROM tiers_identifiant i
           JOIN tiers_identifiant a
             ON a.cabinet_id = i.cabinet_id AND a.dossier_id = i.dossier_id
            AND a.type = i.type AND a.valeur_normalisee = i.valeur_normalisee
            AND a.tiers_id <> i.tiers_id
           WHERE i.cabinet_id = $1 AND i.dossier_id = $2 AND i.tiers_id = $3
           GROUP BY i.type ORDER BY i.type"#,
        cabinet,
        dossier,
        tiers,
    )
    .fetch_all(tx.connexion())
    .await?;
    Ok(lignes.into_iter().map(|l| (l.r#type, l.autres)).collect())
}

/// Doublons des coordonnées bancaires du tiers : pour chacune, les **autres** tiers du dossier
/// qui ont le même numéro (majuscules, sans blanc). Signalés, jamais refusés (Odoo
/// `res_partner_bank.py:68-92`).
///
/// # Errors
/// Contexte sans dossier, erreur de PostgreSQL.
pub async fn doublons_comptes_bancaires(
    tx: &mut TransactionTenant,
    tiers: Uuid,
) -> Result<Vec<(Uuid, Vec<Uuid>)>, ErreurBase> {
    let (cabinet, dossier) = contexte(tx)?;
    let lignes = sqlx::query!(
        r#"SELECT c.id AS "id!", array_agg(DISTINCT a.tiers_id ORDER BY a.tiers_id) AS "autres!"
           FROM tiers_compte_bancaire c
           JOIN tiers_compte_bancaire a
             ON a.cabinet_id = c.cabinet_id AND a.dossier_id = c.dossier_id
            AND a.numero_normalise = c.numero_normalise AND a.tiers_id <> c.tiers_id
           WHERE c.cabinet_id = $1 AND c.dossier_id = $2 AND c.tiers_id = $3
           GROUP BY c.id ORDER BY c.id"#,
        cabinet,
        dossier,
        tiers,
    )
    .fetch_all(tx.connexion())
    .await?;
    Ok(lignes.into_iter().map(|l| (l.id, l.autres)).collect())
}

// --- Mutations -------------------------------------------------------------------------------

/// Verrouille la ligne du tiers `id` (`FOR NO KEY UPDATE`). Faux s'il n'est pas dans le dossier.
///
/// # Errors
/// Contexte sans dossier, erreur de PostgreSQL.
pub async fn verrouiller(tx: &mut TransactionTenant, id: Uuid) -> Result<bool, ErreurBase> {
    let (cabinet, dossier) = contexte(tx)?;
    Ok(sqlx::query!(
        "SELECT id FROM tiers WHERE cabinet_id = $1 AND dossier_id = $2 AND id = $3 \
         FOR NO KEY UPDATE",
        cabinet,
        dossier,
        id,
    )
    .fetch_optional(tx.connexion())
    .await?
    .is_some())
}

/// Contrôle croisé du référentiel avec le tiers `id` actif, de type `genre` sur `compte`
/// (création, désarchivage). Le verrou du référentiel est pris d'abord.
async fn controler_referentiel(
    tx: &mut TransactionTenant,
    id: Uuid,
    tiers: Tiers,
) -> Result<(), ErreurTiers> {
    if !referentiel::verrouiller(tx).await? {
        return Err(ErreurTiers::Absent);
    }
    let relu = referentiel::charger(tx).await?.ok_or(ErreurTiers::Absent)?;
    let mut candidat = relu.etat().clone();
    candidat.tiers.insert(id, tiers);
    referentiel::verifier(&candidat)?;
    Ok(())
}

/// Crée un tiers actif ; rend son code. Sous le verrou du référentiel : contrôle croisé (compte
/// collectif, type), code tiré d'office s'il n'est pas fourni, unicité du code (409).
///
/// # Errors
/// [`ErreurTiers`].
pub async fn creer(tx: &mut TransactionTenant, n: &NouveauTiers) -> Result<String, ErreurTiers> {
    let (cabinet, dossier) = contexte(tx)?;
    controler_referentiel(
        tx,
        n.id,
        Tiers::nouveau(n.nom.clone(), n.genre, n.compte_collectif.clone()),
    )
    .await?;
    let code = match &n.code {
        Some(code) => {
            let pris = sqlx::query!(
                "SELECT id FROM tiers WHERE cabinet_id = $1 AND dossier_id = $2 AND code = $3",
                cabinet,
                dossier,
                code,
            )
            .fetch_optional(tx.connexion())
            .await?
            .is_some();
            if pris {
                return Err(ErreurTiers::CodeDejaUtilise);
            }
            code.clone()
        }
        None => code_suivant(tx).await?,
    };
    sqlx::query!(
        "INSERT INTO tiers (cabinet_id, dossier_id, id, code, nom, type, compte_collectif, \
                            actif, devise, assujetti_tva, soumis_retenue) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, true, $8, $9, $10)",
        cabinet,
        dossier,
        n.id,
        code,
        n.nom.as_str(),
        n.genre.nom(),
        n.compte_collectif.as_str(),
        n.devise,
        n.assujetti_tva,
        n.soumis_retenue,
    )
    .execute(tx.connexion())
    .await?;
    Ok(code)
}

/// Plus grand rang tiré d'office : `T` suivi de 18 chiffres au plus (le rang tient dans un `i64`
/// et son code dans les 20 caractères admis).
const RANG_MAX: i64 = 999_999_999_999_999_999;

/// Prochain code tiré d'office : `T` suivi du plus grand rang déjà tiré plus un, sur six
/// chiffres au moins (forme des codes donnés aux tiers de L2 par la migration `0500`). Sous le
/// verrou du référentiel. Si un code saisi a épuisé la suite (`T` et 18 neuf), ou si le code
/// calculé est déjà pris, aucun code n'est tiré : [`ErreurTiers::TirageImpossible`] (le code est
/// alors à saisir), jamais une violation d'unicité.
async fn code_suivant(tx: &mut TransactionTenant) -> Result<String, ErreurTiers> {
    let (cabinet, dossier) = contexte(tx)?;
    let dernier: i64 = sqlx::query_scalar!(
        r#"SELECT coalesce(max(substring(code FROM 2)::bigint), 0) AS "dernier!"
           FROM tiers
           WHERE cabinet_id = $1 AND dossier_id = $2 AND code ~ '^T[0-9]{6,18}$'"#,
        cabinet,
        dossier,
    )
    .fetch_one(tx.connexion())
    .await?;
    let suivant = tirage_suivant(dernier).ok_or(ErreurTiers::TirageImpossible)?;
    let pris = sqlx::query!(
        "SELECT id FROM tiers WHERE cabinet_id = $1 AND dossier_id = $2 AND code = $3",
        cabinet,
        dossier,
        suivant,
    )
    .fetch_optional(tx.connexion())
    .await?
    .is_some();
    if pris {
        return Err(ErreurTiers::TirageImpossible);
    }
    Ok(suivant)
}

/// Code qui suit le rang `dernier`, s'il reste dans la suite ([`RANG_MAX`]).
fn tirage_suivant(dernier: i64) -> Option<String> {
    let suivant = dernier.checked_add(1).filter(|n| *n <= RANG_MAX)?;
    Some(format!("T{suivant:06}"))
}

/// Enregistre l'en-tête modifiable du tiers (nom, devise, indicateurs fiscaux). Le tiers est
/// déjà verrouillé.
///
/// # Errors
/// Contexte sans dossier, erreur de PostgreSQL.
pub async fn modifier_entete(
    tx: &mut TransactionTenant,
    e: &EnteteTiers,
) -> Result<(), ErreurBase> {
    let (cabinet, dossier) = contexte(tx)?;
    sqlx::query!(
        "UPDATE tiers SET nom = $4, devise = $5, assujetti_tva = $6, soumis_retenue = $7 \
         WHERE cabinet_id = $1 AND dossier_id = $2 AND id = $3",
        cabinet,
        dossier,
        e.id,
        e.nom,
        e.devise,
        e.assujetti_tva,
        e.soumis_retenue,
    )
    .execute(tx.connexion())
    .await?;
    Ok(())
}

/// Change la condition de règlement par défaut du tiers `id` (déjà verrouillé, ou créé dans
/// cette transaction). La condition appartient au dossier : clé étrangère composite (`0511`) ;
/// l'appelant l'a lue dans le dossier du contexte.
///
/// # Errors
/// Contexte sans dossier, erreur de PostgreSQL.
pub async fn changer_condition_reglement(
    tx: &mut TransactionTenant,
    id: Uuid,
    condition: Option<Uuid>,
) -> Result<(), ErreurBase> {
    let (cabinet, dossier) = contexte(tx)?;
    sqlx::query!(
        "UPDATE tiers SET condition_reglement_id = $4 \
         WHERE cabinet_id = $1 AND dossier_id = $2 AND id = $3",
        cabinet,
        dossier,
        id,
        condition,
    )
    .execute(tx.connexion())
    .await?;
    Ok(())
}

/// Archive le tiers (déjà verrouillé et actif) : il n'accepte plus de nouvelle ligne au
/// brouillon (ADR 0031 point 5).
///
/// # Errors
/// Contexte sans dossier, erreur de PostgreSQL.
pub async fn archiver(
    tx: &mut TransactionTenant,
    id: Uuid,
    motif: &str,
    utilisateur: Uuid,
) -> Result<(), ErreurBase> {
    let (cabinet, dossier) = contexte(tx)?;
    sqlx::query!(
        "UPDATE tiers SET actif = false, motif_archivage = $4, archive_le = now(), \
                          archive_par = $5 \
         WHERE cabinet_id = $1 AND dossier_id = $2 AND id = $3",
        cabinet,
        dossier,
        id,
        motif,
        utilisateur,
    )
    .execute(tx.connexion())
    .await?;
    Ok(())
}

/// Désarchive le tiers archivé `e` : contrôle croisé du référentiel d'abord (son compte collectif
/// a pu être désactivé pendant l'archivage), puis verrou du tiers.
///
/// # Errors
/// [`ErreurTiers`].
pub async fn desarchiver(tx: &mut TransactionTenant, e: &EnteteTiers) -> Result<(), ErreurTiers> {
    let (cabinet, dossier) = contexte(tx)?;
    let nom =
        compta_core::TexteCourt::parse(&e.nom).map_err(|_| ErreurBase::Donnee("nom d'un tiers"))?;
    let compte = compta_core::NumeroCompte::parse(&e.compte_collectif)
        .map_err(|_| ErreurBase::Donnee("compte collectif d'un tiers"))?;
    controler_referentiel(tx, e.id, Tiers::nouveau(nom, e.genre, compte)).await?;
    sqlx::query!(
        "UPDATE tiers SET actif = true, motif_archivage = NULL, archive_le = NULL, \
                          archive_par = NULL \
         WHERE cabinet_id = $1 AND dossier_id = $2 AND id = $3",
        cabinet,
        dossier,
        e.id,
    )
    .execute(tx.connexion())
    .await?;
    Ok(())
}

/// Vrai si une ligne cite le tiers : ligne de brouillon ou ligne d'écriture validée (les seules
/// tables qui le référencent, migrations `0202` et `0203`). **Seul contrôle** du figement du type
/// et du compte collectif (ADR 0031 point 1, amendement du 2026-11-27) : une table de L3 qui
/// citerait un tiers dans une ligne comptable s'ajoute ici, dans la même requête.
///
/// # Errors
/// Contexte sans dossier, erreur de PostgreSQL.
pub async fn est_mouvemente(tx: &mut TransactionTenant, id: Uuid) -> Result<bool, ErreurBase> {
    let (cabinet, dossier) = contexte(tx)?;
    Ok(sqlx::query_scalar!(
        r#"SELECT (EXISTS (SELECT 1 FROM brouillon_ligne
                           WHERE cabinet_id = $1 AND dossier_id = $2 AND tiers_id = $3)
                OR EXISTS (SELECT 1 FROM ecriture_validee_ligne
                           WHERE cabinet_id = $1 AND dossier_id = $2 AND tiers_id = $3))
           AS "mouvemente!""#,
        cabinet,
        dossier,
        id,
    )
    .fetch_one(tx.connexion())
    .await?)
}

/// Change le type ou le compte collectif du tiers `e` (Tryton et Odoo laissent changer le compte
/// du tiers ; ADR 0031 point 1, amendement du 2026-11-27) : seulement tant qu'aucune ligne ne le
/// cite ([`est_mouvemente`] ; sinon [`ErreurTiers::DejaMouvemente`]), et sous le contrôle croisé
/// du référentiel (le nouveau compte admet le type). Verrous : référentiel (qui attend les saisies
/// en cours, `FOR SHARE` sur le dossier), puis ligne du tiers en `FOR UPDATE` (qui attend toute
/// insertion d'une ligne qui le cite, `FOR KEY SHARE`).
///
/// # Errors
/// [`ErreurTiers`].
pub async fn changer_classement(
    tx: &mut TransactionTenant,
    e: &EnteteTiers,
    genre: TypeTiers,
    compte_collectif: &compta_core::NumeroCompte,
) -> Result<(), ErreurTiers> {
    let (cabinet, dossier) = contexte(tx)?;
    if !referentiel::verrouiller(tx).await? {
        return Err(ErreurTiers::Absent);
    }
    let present = sqlx::query!(
        "SELECT id FROM tiers WHERE cabinet_id = $1 AND dossier_id = $2 AND id = $3 FOR UPDATE",
        cabinet,
        dossier,
        e.id,
    )
    .fetch_optional(tx.connexion())
    .await?
    .is_some();
    if !present {
        return Err(ErreurTiers::Absent);
    }
    if est_mouvemente(tx, e.id).await? {
        return Err(ErreurTiers::DejaMouvemente);
    }
    let nom =
        compta_core::TexteCourt::parse(&e.nom).map_err(|_| ErreurBase::Donnee("nom d'un tiers"))?;
    controler_referentiel(
        tx,
        e.id,
        Tiers::nouveau(nom, genre, compte_collectif.clone()),
    )
    .await?;
    sqlx::query!(
        "UPDATE tiers SET type = $4, compte_collectif = $5 \
         WHERE cabinet_id = $1 AND dossier_id = $2 AND id = $3",
        cabinet,
        dossier,
        e.id,
        genre.nom(),
        compte_collectif.as_str(),
    )
    .execute(tx.connexion())
    .await?;
    Ok(())
}

/// Ajoute, modifie ou retire un identifiant légal du tiers (déjà verrouillé).
///
/// # Errors
/// Contexte sans dossier, erreur de PostgreSQL.
pub async fn ecrire_identifiant(
    tx: &mut TransactionTenant,
    tiers: Uuid,
    operation: Operation<(&str, &str)>,
) -> Result<(), ErreurBase> {
    let (cabinet, dossier) = contexte(tx)?;
    match operation {
        Operation::Ajout(id, (type_identifiant, valeur)) => {
            sqlx::query!(
                "INSERT INTO tiers_identifiant (cabinet_id, dossier_id, id, tiers_id, type, valeur) \
                 VALUES ($1, $2, $3, $4, $5, $6)",
                cabinet,
                dossier,
                id,
                tiers,
                type_identifiant,
                valeur,
            )
            .execute(tx.connexion())
            .await?;
        }
        Operation::Modification(id, (_, valeur)) => {
            sqlx::query!(
                "UPDATE tiers_identifiant SET valeur = $5 \
                 WHERE cabinet_id = $1 AND dossier_id = $2 AND tiers_id = $3 AND id = $4",
                cabinet,
                dossier,
                tiers,
                id,
                valeur,
            )
            .execute(tx.connexion())
            .await?;
        }
        Operation::Retrait(id) => {
            sqlx::query!(
                "DELETE FROM tiers_identifiant \
                 WHERE cabinet_id = $1 AND dossier_id = $2 AND tiers_id = $3 AND id = $4",
                cabinet,
                dossier,
                tiers,
                id,
            )
            .execute(tx.connexion())
            .await?;
        }
    }
    Ok(())
}

/// Ajout, modification ou retrait d'une ligne de fiche (identifiant de la ligne, valeur).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operation<T> {
    /// Nouvelle ligne.
    Ajout(Uuid, T),
    /// Ligne existante modifiée.
    Modification(Uuid, T),
    /// Ligne retirée.
    Retrait(Uuid),
}

impl<T> Operation<T> {
    /// La même opération, sur une référence à la valeur.
    #[must_use]
    pub fn as_ref(&self) -> Operation<&T> {
        match self {
            Self::Ajout(id, v) => Operation::Ajout(*id, v),
            Self::Modification(id, v) => Operation::Modification(*id, v),
            Self::Retrait(id) => Operation::Retrait(*id),
        }
    }

    /// Identifiant de la ligne.
    #[must_use]
    pub fn ligne(&self) -> Uuid {
        match self {
            Self::Ajout(id, _) | Self::Modification(id, _) | Self::Retrait(id) => *id,
        }
    }

    /// Nom de l'opération, cité par l'audit.
    #[must_use]
    pub fn nom(&self) -> &'static str {
        match self {
            Self::Ajout(..) => "ajout",
            Self::Modification(..) => "modification",
            Self::Retrait(_) => "retrait",
        }
    }
}

/// Ajoute, modifie ou retire une adresse du tiers (déjà verrouillé).
///
/// # Errors
/// Contexte sans dossier, erreur de PostgreSQL.
pub async fn ecrire_adresse(
    tx: &mut TransactionTenant,
    tiers: Uuid,
    operation: Operation<&Adresse>,
) -> Result<(), ErreurBase> {
    let (cabinet, dossier) = contexte(tx)?;
    match operation {
        Operation::Ajout(id, a) => {
            sqlx::query!(
                "INSERT INTO tiers_adresse (cabinet_id, dossier_id, id, tiers_id, libelle, rue, \
                                            code_postal, ville, region, pays) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
                cabinet,
                dossier,
                id,
                tiers,
                a.libelle,
                a.rue,
                a.code_postal,
                a.ville,
                a.region,
                a.pays,
            )
            .execute(tx.connexion())
            .await?;
        }
        Operation::Modification(id, a) => {
            sqlx::query!(
                "UPDATE tiers_adresse SET libelle = $5, rue = $6, code_postal = $7, ville = $8, \
                                          region = $9, pays = $10 \
                 WHERE cabinet_id = $1 AND dossier_id = $2 AND tiers_id = $3 AND id = $4",
                cabinet,
                dossier,
                tiers,
                id,
                a.libelle,
                a.rue,
                a.code_postal,
                a.ville,
                a.region,
                a.pays,
            )
            .execute(tx.connexion())
            .await?;
        }
        Operation::Retrait(id) => {
            sqlx::query!(
                "DELETE FROM tiers_adresse \
                 WHERE cabinet_id = $1 AND dossier_id = $2 AND tiers_id = $3 AND id = $4",
                cabinet,
                dossier,
                tiers,
                id,
            )
            .execute(tx.connexion())
            .await?;
        }
    }
    Ok(())
}

/// Ajoute, modifie ou retire un moyen de contact du tiers (déjà verrouillé).
///
/// # Errors
/// Contexte sans dossier, erreur de PostgreSQL.
pub async fn ecrire_contact(
    tx: &mut TransactionTenant,
    tiers: Uuid,
    operation: Operation<&Contact>,
) -> Result<(), ErreurBase> {
    let (cabinet, dossier) = contexte(tx)?;
    match operation {
        Operation::Ajout(id, c) => {
            sqlx::query!(
                "INSERT INTO tiers_contact (cabinet_id, dossier_id, id, tiers_id, type, valeur, nom) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7)",
                cabinet,
                dossier,
                id,
                tiers,
                c.type_contact,
                c.valeur,
                c.nom,
            )
            .execute(tx.connexion())
            .await?;
        }
        Operation::Modification(id, c) => {
            sqlx::query!(
                "UPDATE tiers_contact SET type = $5, valeur = $6, nom = $7 \
                 WHERE cabinet_id = $1 AND dossier_id = $2 AND tiers_id = $3 AND id = $4",
                cabinet,
                dossier,
                tiers,
                id,
                c.type_contact,
                c.valeur,
                c.nom,
            )
            .execute(tx.connexion())
            .await?;
        }
        Operation::Retrait(id) => {
            sqlx::query!(
                "DELETE FROM tiers_contact \
                 WHERE cabinet_id = $1 AND dossier_id = $2 AND tiers_id = $3 AND id = $4",
                cabinet,
                dossier,
                tiers,
                id,
            )
            .execute(tx.connexion())
            .await?;
        }
    }
    Ok(())
}

/// Ajoute, modifie ou retire une coordonnée bancaire du tiers (déjà verrouillé).
///
/// # Errors
/// Contexte sans dossier, erreur de PostgreSQL.
pub async fn ecrire_compte_bancaire(
    tx: &mut TransactionTenant,
    tiers: Uuid,
    operation: Operation<&CompteBancaire>,
) -> Result<(), ErreurBase> {
    let (cabinet, dossier) = contexte(tx)?;
    match operation {
        Operation::Ajout(id, c) => {
            sqlx::query!(
                "INSERT INTO tiers_compte_bancaire (cabinet_id, dossier_id, id, tiers_id, type, \
                                                    numero, etablissement, devise) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
                cabinet,
                dossier,
                id,
                tiers,
                c.type_identifiant,
                c.numero,
                c.etablissement,
                c.devise,
            )
            .execute(tx.connexion())
            .await?;
        }
        Operation::Modification(id, c) => {
            sqlx::query!(
                "UPDATE tiers_compte_bancaire SET type = $5, numero = $6, etablissement = $7, \
                                                  devise = $8 \
                 WHERE cabinet_id = $1 AND dossier_id = $2 AND tiers_id = $3 AND id = $4",
                cabinet,
                dossier,
                tiers,
                id,
                c.type_identifiant,
                c.numero,
                c.etablissement,
                c.devise,
            )
            .execute(tx.connexion())
            .await?;
        }
        Operation::Retrait(id) => {
            sqlx::query!(
                "DELETE FROM tiers_compte_bancaire \
                 WHERE cabinet_id = $1 AND dossier_id = $2 AND tiers_id = $3 AND id = $4",
                cabinet,
                dossier,
                tiers,
                id,
            )
            .execute(tx.connexion())
            .await?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn le_tirage_reste_dans_la_suite() {
        assert_eq!(tirage_suivant(0).as_deref(), Some("T000001"));
        assert_eq!(tirage_suivant(999_999).as_deref(), Some("T1000000"));
        assert_eq!(
            tirage_suivant(RANG_MAX - 1).as_deref(),
            Some("T999999999999999999")
        );
        // `T` et 18 neuf saisi à la main : plus aucun tirage, jamais un code hors de la suite
        // (que le tirage suivant ignorerait, puis rendrait en double).
        assert_eq!(tirage_suivant(RANG_MAX), None);
        assert_eq!(tirage_suivant(i64::MAX), None);
    }
}
