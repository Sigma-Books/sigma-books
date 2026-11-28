//! Tiers : fiche, identifiants, contacts, adresses, coordonnées bancaires, archivage (L3-15, PR 7a
//! de `docs/lots/L3.md` ; ADR 0031). La fusion et l'anonymisation (L3-16) viendront avec la PR 7b.
//!
//! Persistance : [`crate::base::tiers`]. Les routes de tiers de L2 (`referentiel` : liste,
//! création, modification) sont reprises ici, aux mêmes chemins, sous les actions Cedar de L3.
//!
//! # Fiche
//! - **En-tête** : nom ; type et compte collectif **modifiables tant qu'aucune ligne ne cite le
//!   tiers**, figés ensuite (ADR 0031 point 1, amendement du 2026-11-27 : Tryton et Odoo laissent
//!   changer le compte du tiers ; contrôle croisé du référentiel comme à la création) ;
//!   code auxiliaire unique dans le dossier et immuable (tiré d'office s'il n'est pas fourni),
//!   devise par défaut, indicateurs `assujetti_tva` et `soumis_retenue` sans effet comptable,
//!   condition de règlement par défaut (`condition_reglement`, PR 8 ; ADR 0033) : une condition
//!   du dossier, sinon 404.
//! - **Identifiants légaux** : types du pack du pays **du dossier** (ADR 0027 ; aucune règle de
//!   pays dans le code), au plus un par type, valeur libre bornée : aucun format n'est sourcé, donc
//!   aucun avertissement de forme (ADR 0031 point 2).
//! - **Adresses, contacts** : facultatifs, textes bornés sans caractère de contrôle ni de format
//!   (F11.8). Forme du courriel : **avertissement** (Tryton refuse, non repris).
//! - **Coordonnées bancaires** : type du pack (RIB, IBAN, portefeuille, autre), aucune règle de
//!   forme ; lues par une route à part, refusée à l'auditeur (`ConsulterCoordonneesBancairesTiers`,
//!   VP17) ; toute modification alerte les administrateurs (F1.26).
//! - **Doublons** d'identifiant (même type, même valeur) ou de numéro bancaire chez un autre tiers
//!   du dossier : **signalés** (`avertissements`), jamais refusés (Tryton
//!   `party/party.py:1061-1078`, Odoo `res_partner_bank.py:68-92`). Nom en double : toléré.
//! - **Archivage** libre, avec motif ; désarchivage possible (contrôle croisé du référentiel :
//!   le compte collectif doit rester utilisable). Aucune suppression (ADR 0031 point 5).
//!
//! Une liste fournie (`identifiants`, `adresses`, `contacts`, `comptes`) **remplace** la liste
//! entière : une ligne citée par son `id` est conservée (modifiée si elle change), une ligne sans
//! `id` est ajoutée, une ligne existante absente est retirée. Une liste absente est conservée.
//!
//! # Audit sans valeur personnelle (ADR 0031 point 8)
//! Chaque changement est une trace au dossier : identifiant du tiers ou de la ligne, opération,
//! **noms** des champs modifiés, type d'identifiant, type et compte collectif du tiers ; jamais
//! un nom, une adresse, un numéro, ni le motif d'archivage (texte libre).
//!
//! # Défense en profondeur (ADR 0011)
//! Même si Cedar se trompait : le dossier du chemin est relu **visible** par l'utilisateur, le
//! rôle de l'appartenance active est relu en base, et le tiers est cherché dans le seul dossier du
//! contexte (celui d'un autre dossier est absent : 404).

use std::collections::{BTreeMap, BTreeSet};

use authz::{Action, Role};
use axum::Json;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use compta_core::{NumeroCompte, TexteCourt};
use serde::{Deserialize, Deserializer, Serialize};
use syscohada::PackPays;
use uuid::Uuid;

use super::{ActionRoute, CasUsage, Methode, Module, Portee, Requete, Resultat, Route};
use crate::base::TransactionTenant;
use crate::base::audit::PorteeAudit;
use crate::base::organisation::{lire_appartenance, lire_dossier_visible};
use crate::base::referentiel::TypeTiers;
use crate::base::taches::{SujetNotification, notifier};
use crate::base::tiers::{
    self, Adresse, CompteBancaire, Contact, EnteteTiers, ErreurTiers, FicheTiers, NouveauTiers,
    Operation,
};
use crate::contexte_dossier::{
    IDENTIFIANTS_MAX, VALEUR_IDENTIFIANT_MAX, a_un_caractere_interdit, texte_borne,
};
use crate::serveur::audit::Trace;
use crate::serveur::erreur::{CategorieErreur, ErreurApi, MotifConflit, MotifValidation};
use crate::serveur::pipeline::{DossierAutorise, Garde, id_chemin};

/// Cas d'usage du module.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cas {
    /// Tiers du dossier, paginés.
    Lister,
    /// Créer un tiers.
    Creer,
    /// Fiche d'un tiers, sans ses coordonnées bancaires.
    Lire,
    /// Modifier la fiche d'un tiers (en-tête, identifiants, adresses, contacts).
    Modifier,
    /// Archiver un tiers, avec un motif.
    Archiver,
    /// Désarchiver un tiers.
    Desarchiver,
    /// Coordonnées bancaires d'un tiers (auditeur exclu).
    LireComptesBancaires,
    /// Remplacer les coordonnées bancaires d'un tiers (alerte aux administrateurs).
    ModifierComptesBancaires,
}

/// Tranche du module.
pub const MODULE: Module = Module {
    nom: "tiers",
    routes: ROUTES,
    champs_journal: &[],
};

const fn route(methode: Methode, chemin: &'static str, action: Action, cas: Cas) -> Route {
    Route {
        methode,
        chemin,
        action: ActionRoute::Cabinet(action),
        portee: Portee::Dossier,
        cas_usage: CasUsage::Tiers(cas),
    }
}

/// Routes du module (actions de `crates/authz/politiques/roles/tiers.cedar`).
pub const ROUTES: &[Route] = &[
    route(
        Methode::Get,
        "/api/cabinets/{cabinet}/dossiers/{dossier}/tiers",
        Action::ConsulterTiers,
        Cas::Lister,
    ),
    route(
        Methode::Post,
        "/api/cabinets/{cabinet}/dossiers/{dossier}/tiers",
        Action::CreerTiers,
        Cas::Creer,
    ),
    route(
        Methode::Get,
        "/api/cabinets/{cabinet}/dossiers/{dossier}/tiers/{tiers}",
        Action::ConsulterTiers,
        Cas::Lire,
    ),
    route(
        Methode::Post,
        "/api/cabinets/{cabinet}/dossiers/{dossier}/tiers/{tiers}/modification",
        Action::ModifierTiers,
        Cas::Modifier,
    ),
    route(
        Methode::Post,
        "/api/cabinets/{cabinet}/dossiers/{dossier}/tiers/{tiers}/archivage",
        Action::ArchiverTiers,
        Cas::Archiver,
    ),
    route(
        Methode::Post,
        "/api/cabinets/{cabinet}/dossiers/{dossier}/tiers/{tiers}/desarchivage",
        Action::ArchiverTiers,
        Cas::Desarchiver,
    ),
    route(
        Methode::Get,
        "/api/cabinets/{cabinet}/dossiers/{dossier}/tiers/{tiers}/comptes-bancaires",
        Action::ConsulterCoordonneesBancairesTiers,
        Cas::LireComptesBancaires,
    ),
    route(
        Methode::Post,
        "/api/cabinets/{cabinet}/dossiers/{dossier}/tiers/{tiers}/comptes-bancaires",
        Action::ModifierTiers,
        Cas::ModifierComptesBancaires,
    ),
];

// --- Bornes ----------------------------------------------------------------------------------

/// Taille d'une page de tiers par défaut.
const PAGE_PAR_DEFAUT: usize = 50;
/// Taille maximale d'une page de tiers.
const PAGE_MAX: usize = 200;
/// Longueur maximale du code auxiliaire (ADR 0031 point 1).
const CODE_MAX: usize = 20;
/// Bornes de taille de la fiche (limites de sécurité contre un corps démesuré, F2.16) : au plus
/// tant d'adresses, de contacts et de coordonnées bancaires par tiers. Les identifiants légaux
/// suivent la borne du dossier ([`IDENTIFIANTS_MAX`], un par type).
const ADRESSES_MAX: usize = 20;
const CONTACTS_MAX: usize = 50;
const COMPTES_MAX: usize = 20;
/// Longueurs maximales des champs (celles des contraintes des migrations `0500` à `0504`).
const NOM_MAX: usize = 200;
const MOTIF_MAX: usize = 200;
const LIBELLE_MAX: usize = 100;
const RUE_MAX: usize = 200;
const CODE_POSTAL_MAX: usize = 20;
const VILLE_MAX: usize = 100;
const VALEUR_CONTACT_MAX: usize = 200;
const NUMERO_MAX: usize = 64;
const ETABLISSEMENT_MAX: usize = 200;
/// Types de moyens de contact (Tryton `party/contact_mechanism.py`, `_TYPES`).
const TYPES_CONTACT: [&str; 6] = [
    "telephone",
    "mobile",
    "fax",
    "courriel",
    "site_web",
    "autre",
];

/// Rôles qui lisent la fiche (Cedar `l3-c6-consulter-tiers`).
const ROLES_LECTURE: &[Role] = &[
    Role::Administrateur,
    Role::Valideur,
    Role::Collaborateur,
    Role::Auditeur,
];
/// Rôles qui tiennent la fiche, coordonnées bancaires comprises, et les lisent
/// (Cedar `l3-c6-tenir-fiche-tiers-et-echeances`, `l3-c6-consulter-coordonnees-bancaires-tiers`).
const ROLES_SAISIE: &[Role] = &[Role::Administrateur, Role::Valideur, Role::Collaborateur];
/// Rôles qui créent un tiers (Cedar `l3-c6-parametrer-tiers`).
const ROLES_CREATION: &[Role] = &[Role::Administrateur, Role::Valideur];

// --- Vues ------------------------------------------------------------------------------------

#[derive(Debug, Serialize)]
struct VueEntete {
    id: String,
    code: String,
    nom: String,
    #[serde(rename = "type")]
    genre: &'static str,
    compte_collectif: String,
    actif: bool,
    devise: Option<String>,
    assujetti_tva: bool,
    soumis_retenue: bool,
    /// Condition de règlement par défaut de saisie (ADR 0033).
    condition_reglement: Option<String>,
}

#[derive(Debug, Serialize)]
struct ListeTiers {
    tiers: Vec<VueEntete>,
    /// Identifiant du dernier tiers de la page, à passer en `apres` ; absent en fin de liste.
    suite: Option<String>,
}

#[derive(Debug, Serialize)]
struct VueIdentifiant {
    id: String,
    #[serde(rename = "type")]
    genre: String,
    valeur: String,
}

#[derive(Debug, Serialize)]
struct VueAdresse {
    id: String,
    libelle: Option<String>,
    rue: Option<String>,
    code_postal: Option<String>,
    ville: Option<String>,
    region: Option<String>,
    pays: Option<String>,
}

#[derive(Debug, Serialize)]
struct VueContact {
    id: String,
    #[serde(rename = "type")]
    genre: String,
    valeur: String,
    nom: Option<String>,
}

#[derive(Debug, Serialize)]
struct VueCompteBancaire {
    id: String,
    #[serde(rename = "type")]
    genre: String,
    numero: String,
    etablissement: Option<String>,
    devise: Option<String>,
}

/// Avertissement non bloquant : doublon (identifiant, numéro bancaire) ou forme d'un courriel.
#[derive(Debug, Serialize, PartialEq, Eq)]
struct Avertissement {
    code: &'static str,
    /// Type d'identifiant en double.
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    genre: Option<String>,
    /// Ligne concernée (contact, coordonnée bancaire).
    #[serde(skip_serializing_if = "Option::is_none")]
    ligne: Option<String>,
    /// Autres tiers du dossier qui portent la même valeur.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tiers: Vec<String>,
}

#[derive(Debug, Serialize)]
struct VueFiche {
    #[serde(flatten)]
    entete: VueEntete,
    motif_archivage: Option<String>,
    archive_le: Option<String>,
    identifiants: Vec<VueIdentifiant>,
    adresses: Vec<VueAdresse>,
    contacts: Vec<VueContact>,
    avertissements: Vec<Avertissement>,
}

#[derive(Debug, Serialize)]
struct VueComptesBancaires {
    /// Identifiant du tiers.
    id: String,
    comptes: Vec<VueCompteBancaire>,
    avertissements: Vec<Avertissement>,
}

fn vue_entete(e: &EnteteTiers) -> VueEntete {
    VueEntete {
        id: e.id.to_string(),
        code: e.code.clone(),
        nom: e.nom.clone(),
        genre: e.genre.nom(),
        compte_collectif: e.compte_collectif.clone(),
        actif: e.actif,
        devise: e.devise.clone(),
        assujetti_tva: e.assujetti_tva,
        soumis_retenue: e.soumis_retenue,
        condition_reglement: e.condition_reglement.map(|c| c.to_string()),
    }
}

/// Forme d'un courriel, pour un **avertissement** seulement : une seule `@`, une partie locale,
/// un domaine qui contient un point et ne commence ni ne finit par lui, aucun blanc.
fn courriel_bien_forme(valeur: &str) -> bool {
    let mut parties = valeur.split('@');
    let (Some(local), Some(domaine), None) = (parties.next(), parties.next(), parties.next())
    else {
        return false;
    };
    !local.is_empty()
        && domaine.contains('.')
        && !domaine.starts_with('.')
        && !domaine.ends_with('.')
        && !valeur.chars().any(char::is_whitespace)
}

fn vue_fiche(fiche: &FicheTiers, doublons: Vec<(String, Vec<Uuid>)>) -> VueFiche {
    let mut avertissements: Vec<Avertissement> = doublons
        .into_iter()
        .map(|(genre, autres)| Avertissement {
            code: "identifiant_doublon",
            genre: Some(genre),
            ligne: None,
            tiers: autres.iter().map(Uuid::to_string).collect(),
        })
        .collect();
    avertissements.extend(
        fiche
            .contacts
            .iter()
            .filter(|(_, c)| c.type_contact == "courriel" && !courriel_bien_forme(&c.valeur))
            .map(|(id, _)| Avertissement {
                code: "courriel_forme",
                genre: None,
                ligne: Some(id.to_string()),
                tiers: Vec::new(),
            }),
    );
    VueFiche {
        entete: vue_entete(&fiche.entete),
        motif_archivage: fiche.entete.motif_archivage.clone(),
        archive_le: fiche.entete.archive_le.map(crate::pieces::interne::instant),
        identifiants: fiche
            .identifiants
            .iter()
            .map(|i| VueIdentifiant {
                id: i.id.to_string(),
                genre: i.type_identifiant.clone(),
                valeur: i.valeur.clone(),
            })
            .collect(),
        adresses: fiche
            .adresses
            .iter()
            .map(|(id, a)| VueAdresse {
                id: id.to_string(),
                libelle: a.libelle.clone(),
                rue: a.rue.clone(),
                code_postal: a.code_postal.clone(),
                ville: a.ville.clone(),
                region: a.region.clone(),
                pays: a.pays.clone(),
            })
            .collect(),
        contacts: fiche
            .contacts
            .iter()
            .map(|(id, c)| VueContact {
                id: id.to_string(),
                genre: c.type_contact.clone(),
                valeur: c.valeur.clone(),
                nom: c.nom.clone(),
            })
            .collect(),
        avertissements,
    }
}

// --- Corps -----------------------------------------------------------------------------------

/// Champ texte d'une modification : absent (conservé), `null` (effacé) ou une valeur.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
enum ChampTexte {
    #[default]
    Absent,
    Efface,
    Valeur(String),
}

impl<'de> Deserialize<'de> for ChampTexte {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Ok(match Option::<String>::deserialize(d)? {
            None => Self::Efface,
            Some(v) => Self::Valeur(v),
        })
    }
}

/// Corps de la création : `nom`, `type`, `compte_collectif` obligatoires (comme en L2) ; `code`,
/// `devise`, `assujetti_tva`, `soumis_retenue` facultatifs.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CorpsCreation {
    nom: String,
    #[serde(rename = "type")]
    genre: String,
    compte_collectif: String,
    code: Option<String>,
    devise: Option<String>,
    #[serde(default)]
    assujetti_tva: bool,
    #[serde(default)]
    soumis_retenue: bool,
    condition_reglement: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CorpsIdentifiant {
    #[serde(rename = "type")]
    genre: String,
    valeur: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CorpsAdresse {
    id: Option<String>,
    libelle: Option<String>,
    rue: Option<String>,
    code_postal: Option<String>,
    ville: Option<String>,
    region: Option<String>,
    pays: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CorpsContact {
    id: Option<String>,
    #[serde(rename = "type")]
    genre: String,
    valeur: String,
    nom: Option<String>,
}

/// Corps de la modification : chaque champ absent est conservé ; `"devise": null` l'efface ;
/// une liste présente remplace la liste entière.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CorpsModification {
    nom: Option<String>,
    /// Type et compte collectif : modifiables tant qu'aucune ligne ne cite le tiers (ADR 0031
    /// point 1, amendement du 2026-11-27).
    #[serde(rename = "type")]
    genre: Option<String>,
    compte_collectif: Option<String>,
    #[serde(default)]
    devise: ChampTexte,
    assujetti_tva: Option<bool>,
    soumis_retenue: Option<bool>,
    /// Condition de règlement par défaut : `null` l'efface.
    #[serde(default)]
    condition_reglement: ChampTexte,
    identifiants: Option<Vec<CorpsIdentifiant>>,
    adresses: Option<Vec<CorpsAdresse>>,
    contacts: Option<Vec<CorpsContact>>,
}

impl CorpsModification {
    fn est_vide(&self) -> bool {
        self.nom.is_none()
            && self.genre.is_none()
            && self.compte_collectif.is_none()
            && self.devise == ChampTexte::Absent
            && self.assujetti_tva.is_none()
            && self.soumis_retenue.is_none()
            && self.condition_reglement == ChampTexte::Absent
            && self.identifiants.is_none()
            && self.adresses.is_none()
            && self.contacts.is_none()
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CorpsArchivage {
    motif: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CorpsVide {}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CorpsCompteBancaire {
    id: Option<String>,
    #[serde(rename = "type")]
    genre: String,
    numero: String,
    etablissement: Option<String>,
    devise: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CorpsComptesBancaires {
    comptes: Vec<CorpsCompteBancaire>,
}

fn corps<T: serde::de::DeserializeOwned>(octets: &[u8]) -> Result<T, ErreurApi> {
    Json::<T>::from_bytes(octets)
        .map(|Json(t)| t)
        .map_err(|_| ErreurApi::RequeteInvalide)
}

// --- Contrôles de forme (purs) ---------------------------------------------------------------

fn invalide() -> ErreurApi {
    ErreurApi::Validation {
        motif: MotifValidation::TiersValeurInvalide,
        ligne: None,
    }
}

fn limite(motif: MotifValidation) -> ErreurApi {
    ErreurApi::Validation { motif, ligne: None }
}

/// Texte borné, non vide, sans caractère de contrôle ni de format Unicode (catégorie `Cf` : F11.8 ;
/// 422 `tiers_caractere_invalide`, qui ne cite jamais le caractère).
fn texte(valeur: &str, max: usize) -> Result<String, ErreurApi> {
    if a_un_caractere_interdit(valeur) {
        return Err(limite(MotifValidation::TiersCaractereInvalide));
    }
    if texte_borne(valeur, max) {
        Ok(valeur.to_owned())
    } else {
        Err(invalide())
    }
}

fn texte_facultatif(valeur: Option<&str>, max: usize) -> Result<Option<String>, ErreurApi> {
    valeur.map(|v| texte(v, max)).transpose()
}

/// Nom d'un tiers : la grammaire de `TexteCourt` (L2), plus le refus des caractères de format
/// bidirectionnels (F11.8).
fn nom(valeur: &str) -> Result<TexteCourt, ErreurApi> {
    texte(valeur, NOM_MAX)?;
    TexteCourt::parse(valeur).map_err(|_| invalide())
}

/// Code auxiliaire : alphabet `A-Z 0-9 . _ -`, 1 à 20 caractères (ADR 0031 point 1).
fn code(valeur: &str) -> Result<String, ErreurApi> {
    let forme = (1..=CODE_MAX).contains(&valeur.len())
        && valeur
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b"._-".contains(&b));
    if forme {
        Ok(valeur.to_owned())
    } else {
        Err(invalide())
    }
}

/// Devise : forme ISO 4217 (trois lettres majuscules). Un tiers étranger peut avoir une devise
/// hors du pack des devises de tenue.
fn devise(valeur: &str) -> Result<String, ErreurApi> {
    if valeur.len() == 3 && valeur.bytes().all(|b| b.is_ascii_uppercase()) {
        Ok(valeur.to_owned())
    } else {
        Err(invalide())
    }
}

/// Pays d'une adresse : forme ISO 3166-1 alpha-2 (un tiers peut être hors de l'OHADA).
fn pays(valeur: &str) -> Result<String, ErreurApi> {
    if valeur.len() == 2 && valeur.bytes().all(|b| b.is_ascii_uppercase()) {
        Ok(valeur.to_owned())
    } else {
        Err(invalide())
    }
}

/// Condition de règlement citée : un UUID (sinon 400) d'une condition **du dossier du contexte**
/// (sinon 404 : celle d'un autre dossier est absente).
async fn condition_citee(tx: &mut TransactionTenant, valeur: &str) -> Result<Uuid, ErreurApi> {
    let id = Uuid::try_parse(valeur).map_err(|_| ErreurApi::RequeteInvalide)?;
    if crate::base::echeancier::condition_existe(tx, id).await? {
        Ok(id)
    } else {
        Err(ErreurApi::Introuvable)
    }
}

/// Identifiant d'une ligne citée dans une liste : UUID, sinon 400.
fn id_ligne(valeur: Option<&str>) -> Result<Option<Uuid>, ErreurApi> {
    valeur
        .map(|v| Uuid::try_parse(v).map_err(|_| ErreurApi::RequeteInvalide))
        .transpose()
}

/// Identifiants légaux demandés : types du pack du pays du dossier (ADR 0027), un par type, au
/// plus [`IDENTIFIANTS_MAX`], valeur de 1 à [`VALEUR_IDENTIFIANT_MAX`] caractères.
fn identifiants(
    pack: &PackPays,
    pays_du_dossier: &str,
    demandes: &[CorpsIdentifiant],
) -> Result<BTreeMap<String, String>, ErreurApi> {
    let inconnu = ErreurApi::Validation {
        motif: MotifValidation::TiersIdentifiantTypeInconnu,
        ligne: None,
    };
    let pays = pack
        .pays_par_code(pays_du_dossier)
        .ok_or(ErreurApi::Interne(CategorieErreur::Donnee))?;
    if demandes.len() > IDENTIFIANTS_MAX {
        return Err(limite(MotifValidation::TiersTropDIdentifiants));
    }
    let mut lus = BTreeMap::new();
    for d in demandes {
        if pays.type_identifiant(&d.genre).is_none() {
            return Err(inconnu);
        }
        let valeur = texte(&d.valeur, VALEUR_IDENTIFIANT_MAX)?;
        if lus.insert(d.genre.clone(), valeur).is_some() {
            return Err(inconnu);
        }
    }
    Ok(lus)
}

fn adresse(c: &CorpsAdresse) -> Result<(Option<Uuid>, Adresse), ErreurApi> {
    Ok((
        id_ligne(c.id.as_deref())?,
        Adresse {
            libelle: texte_facultatif(c.libelle.as_deref(), LIBELLE_MAX)?,
            rue: texte_facultatif(c.rue.as_deref(), RUE_MAX)?,
            code_postal: texte_facultatif(c.code_postal.as_deref(), CODE_POSTAL_MAX)?,
            ville: texte_facultatif(c.ville.as_deref(), VILLE_MAX)?,
            region: texte_facultatif(c.region.as_deref(), VILLE_MAX)?,
            pays: c.pays.as_deref().map(pays).transpose()?,
        },
    ))
}

fn contact(c: &CorpsContact) -> Result<(Option<Uuid>, Contact), ErreurApi> {
    if !TYPES_CONTACT.contains(&c.genre.as_str()) {
        return Err(invalide());
    }
    Ok((
        id_ligne(c.id.as_deref())?,
        Contact {
            type_contact: c.genre.clone(),
            valeur: texte(&c.valeur, VALEUR_CONTACT_MAX)?,
            nom: texte_facultatif(c.nom.as_deref(), NOM_MAX)?,
        },
    ))
}

fn compte_bancaire(
    pack: &PackPays,
    c: &CorpsCompteBancaire,
) -> Result<(Option<Uuid>, CompteBancaire), ErreurApi> {
    if pack.type_identifiant_bancaire(&c.genre).is_none() {
        return Err(ErreurApi::Validation {
            motif: MotifValidation::TiersCompteBancaireTypeInconnu,
            ligne: None,
        });
    }
    Ok((
        id_ligne(c.id.as_deref())?,
        CompteBancaire {
            type_identifiant: c.genre.clone(),
            numero: texte(&c.numero, NUMERO_MAX)?,
            etablissement: texte_facultatif(c.etablissement.as_deref(), ETABLISSEMENT_MAX)?,
            devise: c.devise.as_deref().map(devise).transpose()?,
        },
    ))
}

// --- Différences (pures) ---------------------------------------------------------------------

/// Différence d'une liste de lignes identifiées : la liste demandée remplace l'existante. Une
/// ligne demandée avec un `id` absent de l'existant est **absente** (404 : elle peut être celle
/// d'un autre tiers ou d'un autre dossier) ; un `id` cité deux fois : 400.
fn differences<T: Clone + PartialEq>(
    existant: &[(Uuid, T)],
    demande: Vec<(Option<Uuid>, T)>,
    nouvel_id: &mut impl FnMut() -> Uuid,
) -> Result<Vec<Operation<T>>, ErreurApi> {
    let avant: BTreeMap<Uuid, &T> = existant.iter().map(|(id, t)| (*id, t)).collect();
    let mut cites = BTreeSet::new();
    let mut operations = Vec::new();
    for (id, valeur) in demande {
        match id {
            None => operations.push(Operation::Ajout(nouvel_id(), valeur)),
            Some(id) => {
                let ancien = avant.get(&id).ok_or(ErreurApi::Introuvable)?;
                if !cites.insert(id) {
                    return Err(ErreurApi::RequeteInvalide);
                }
                if **ancien != valeur {
                    operations.push(Operation::Modification(id, valeur));
                }
            }
        }
    }
    operations.extend(
        existant
            .iter()
            .filter(|(id, _)| !cites.contains(id))
            .map(|(id, _)| Operation::Retrait(*id)),
    );
    Ok(operations)
}

/// Différence des identifiants légaux, repérés par leur type (un par type).
fn differences_identifiants(
    existant: &[tiers::Identifiant],
    demande: &BTreeMap<String, String>,
    nouvel_id: &mut impl FnMut() -> Uuid,
) -> Vec<(String, Operation<String>)> {
    let mut operations = Vec::new();
    for (genre, valeur) in demande {
        match existant.iter().find(|i| &i.type_identifiant == genre) {
            None => operations.push((genre.clone(), Operation::Ajout(nouvel_id(), valeur.clone()))),
            Some(i) if &i.valeur != valeur => {
                operations.push((genre.clone(), Operation::Modification(i.id, valeur.clone())));
            }
            Some(_) => {}
        }
    }
    operations.extend(
        existant
            .iter()
            .filter(|i| !demande.contains_key(&i.type_identifiant))
            .map(|i| (i.type_identifiant.clone(), Operation::Retrait(i.id))),
    );
    operations
}

/// Noms des champs d'une adresse qui diffèrent.
fn champs_adresse(a: &Adresse, b: &Adresse) -> Vec<&'static str> {
    [
        ("libelle", a.libelle != b.libelle),
        ("rue", a.rue != b.rue),
        ("code_postal", a.code_postal != b.code_postal),
        ("ville", a.ville != b.ville),
        ("region", a.region != b.region),
        ("pays", a.pays != b.pays),
    ]
    .into_iter()
    .filter_map(|(nom, change)| change.then_some(nom))
    .collect()
}

fn champs_contact(a: &Contact, b: &Contact) -> Vec<&'static str> {
    [
        ("type", a.type_contact != b.type_contact),
        ("valeur", a.valeur != b.valeur),
        ("nom", a.nom != b.nom),
    ]
    .into_iter()
    .filter_map(|(nom, change)| change.then_some(nom))
    .collect()
}

fn champs_compte(a: &CompteBancaire, b: &CompteBancaire) -> Vec<&'static str> {
    [
        ("type", a.type_identifiant != b.type_identifiant),
        ("numero", a.numero != b.numero),
        ("etablissement", a.etablissement != b.etablissement),
        ("devise", a.devise != b.devise),
    ]
    .into_iter()
    .filter_map(|(nom, change)| change.then_some(nom))
    .collect()
}

/// Champs renseignés d'une ligne ajoutée (noms seulement).
fn champs_renseignes(champs: &[(&'static str, bool)]) -> Vec<&'static str> {
    champs
        .iter()
        .filter_map(|(nom, present)| present.then_some(*nom))
        .collect()
}

// --- Traces (sans valeur personnelle) --------------------------------------------------------

fn trace(dossier: crate::base::IdDossier, genre: &'static str, id: Uuid) -> Trace {
    Trace::new(PorteeAudit::Dossier(dossier), genre, id)
}

/// Trace d'une ligne de fiche : tiers, opération, champs (noms), et données non personnelles.
fn trace_ligne(
    dossier: crate::base::IdDossier,
    genre: &'static str,
    tiers: Uuid,
    operation: &'static str,
    ligne: Uuid,
    champs: &[&'static str],
    donnees: Vec<(&'static str, String)>,
) -> Trace {
    let mut t = trace(dossier, genre, ligne);
    t.donnees.push(("tiers", tiers.to_string()));
    t.donnees.push(("operation", operation.to_owned()));
    t.donnees.extend(donnees);
    if !champs.is_empty() {
        t.donnees.push(("champs", champs.join(",")));
    }
    t
}

// --- Exécution -------------------------------------------------------------------------------

/// Exécute un cas d'usage du module dans la transaction du pipeline.
pub(crate) async fn executer(
    cas: Cas,
    tx: &mut TransactionTenant,
    garde: &Garde,
    requete: &Requete<'_>,
) -> Result<Resultat, ErreurApi> {
    let dossier = dossier_visible(tx, garde).await?;
    match cas {
        Cas::Lister => {
            exiger_role(tx, ROLES_LECTURE).await?;
            lister(tx, requete).await
        }
        Cas::Lire => {
            exiger_role(tx, ROLES_LECTURE).await?;
            let id = id_chemin(requete.chemin, "tiers")?;
            Ok(Resultat::lecture(
                Json(fiche(tx, id).await?).into_response(),
            ))
        }
        Cas::LireComptesBancaires => {
            exiger_role(tx, ROLES_SAISIE).await?;
            let id = id_chemin(requete.chemin, "tiers")?;
            tiers::lire_entete(tx, id)
                .await?
                .ok_or(ErreurApi::Introuvable)?;
            Ok(Resultat::lecture(
                Json(comptes_bancaires(tx, id).await?).into_response(),
            ))
        }
        Cas::Creer => {
            exiger_role(tx, ROLES_CREATION).await?;
            creer(tx, &dossier, requete).await
        }
        Cas::Modifier => {
            exiger_role(tx, ROLES_SAISIE).await?;
            modifier(tx, &dossier, requete).await
        }
        Cas::Archiver | Cas::Desarchiver => {
            exiger_role(tx, ROLES_SAISIE).await?;
            archiver(cas, tx, garde, &dossier, requete).await
        }
        Cas::ModifierComptesBancaires => {
            exiger_role(tx, ROLES_SAISIE).await?;
            modifier_comptes_bancaires(tx, garde, &dossier, requete).await
        }
    }
}

/// Le dossier du chemin, **visible** par l'utilisateur (relu en base : affecté, sauf
/// administrateur) et décidé par le pipeline. Sinon : ressource d'autrui (404).
async fn dossier_visible(
    tx: &mut TransactionTenant,
    garde: &Garde,
) -> Result<DossierAutorise, ErreurApi> {
    let id = tx.contexte().id_dossier().ok_or(ErreurApi::Introuvable)?;
    let dossier = lire_dossier_visible(tx, id)
        .await?
        .ok_or(ErreurApi::Introuvable)?;
    garde.dossier_du_chemin(dossier)
}

/// Défense en profondeur : appartenance **active** lue en base, d'un des rôles admis (ceux des
/// politiques Cedar de l'action). Absente ou suspendue : 404 ; autre rôle : 403.
async fn exiger_role(tx: &mut TransactionTenant, admis: &[Role]) -> Result<(), ErreurApi> {
    let appartenance = lire_appartenance(tx)
        .await?
        .filter(|a| a.actif)
        .ok_or(ErreurApi::Introuvable)?;
    if admis.iter().any(|r| r.nom() == appartenance.role) {
        Ok(())
    } else {
        Err(ErreurApi::Interdit)
    }
}

/// Pagination : `apres` (UUID du dernier tiers reçu) et `limite` (1 à [`PAGE_MAX`]). Tout autre
/// paramètre, doublon ou valeur invalide : 400.
fn pagination(parametres: Option<&str>) -> Result<(Option<Uuid>, usize), ErreurApi> {
    let (mut apres, mut limite) = (None, None);
    for paire in parametres
        .unwrap_or("")
        .split('&')
        .filter(|p| !p.is_empty())
    {
        let (cle, valeur) = paire.split_once('=').ok_or(ErreurApi::RequeteInvalide)?;
        let deja = match cle {
            "apres" => apres
                .replace(Uuid::try_parse(valeur).map_err(|_| ErreurApi::RequeteInvalide)?)
                .is_some(),
            "limite" => {
                let n = (!valeur.is_empty() && valeur.bytes().all(|b| b.is_ascii_digit()))
                    .then(|| valeur.parse::<usize>().ok())
                    .flatten()
                    .filter(|n| (1..=PAGE_MAX).contains(n))
                    .ok_or(ErreurApi::RequeteInvalide)?;
                limite.replace(n).is_some()
            }
            _ => true,
        };
        if deja {
            return Err(ErreurApi::RequeteInvalide);
        }
    }
    Ok((apres, limite.unwrap_or(PAGE_PAR_DEFAUT)))
}

async fn lister(tx: &mut TransactionTenant, requete: &Requete<'_>) -> Result<Resultat, ErreurApi> {
    let (apres, limite) = pagination(requete.parametres)?;
    // Un de plus que la page : dit s'il reste une suite.
    let lus = i64::try_from(limite + 1).map_err(|_| ErreurApi::RequeteInvalide)?;
    let mut tous = tiers::lister(tx, apres, lus)
        .await?
        .ok_or(ErreurApi::RequeteInvalide)?;
    let reste = tous.len() > limite;
    tous.truncate(limite);
    let page: Vec<VueEntete> = tous.iter().map(vue_entete).collect();
    let suite = reste.then(|| page.last().map(|t| t.id.clone())).flatten();
    Ok(Resultat::lecture(
        Json(ListeTiers { tiers: page, suite }).into_response(),
    ))
}

/// Vue de la fiche du tiers `id`, avec ses avertissements (doublons, forme des courriels).
async fn fiche(tx: &mut TransactionTenant, id: Uuid) -> Result<VueFiche, ErreurApi> {
    let fiche = tiers::lire_fiche(tx, id)
        .await?
        .ok_or(ErreurApi::Introuvable)?;
    let doublons = tiers::doublons_identifiants(tx, id).await?;
    Ok(vue_fiche(&fiche, doublons))
}

/// Vue des coordonnées bancaires du tiers `id`, avec les doublons de numéro.
async fn comptes_bancaires(
    tx: &mut TransactionTenant,
    id: Uuid,
) -> Result<VueComptesBancaires, ErreurApi> {
    let comptes = tiers::lire_comptes_bancaires(tx, id).await?;
    let avertissements = tiers::doublons_comptes_bancaires(tx, id)
        .await?
        .into_iter()
        .map(|(ligne, autres)| Avertissement {
            code: "compte_bancaire_doublon",
            genre: None,
            ligne: Some(ligne.to_string()),
            tiers: autres.iter().map(Uuid::to_string).collect(),
        })
        .collect();
    Ok(VueComptesBancaires {
        id: id.to_string(),
        comptes: comptes
            .into_iter()
            .map(|(id, c)| VueCompteBancaire {
                id: id.to_string(),
                genre: c.type_identifiant,
                numero: c.numero,
                etablissement: c.etablissement,
                devise: c.devise,
            })
            .collect(),
        avertissements,
    })
}

fn erreur_tiers(e: ErreurTiers) -> ErreurApi {
    match e {
        ErreurTiers::Absent => ErreurApi::Introuvable,
        // Contrôle croisé du référentiel (compte collectif, type) : comme en L2, 400.
        ErreurTiers::Refus(_) => ErreurApi::RequeteInvalide,
        ErreurTiers::CodeDejaUtilise => ErreurApi::Conflit {
            motif: MotifConflit::TiersCodeDejaUtilise,
            nombre: None,
        },
        ErreurTiers::TirageImpossible => ErreurApi::Conflit {
            motif: MotifConflit::TiersCodeTirageImpossible,
            nombre: None,
        },
        ErreurTiers::DejaMouvemente => ErreurApi::Conflit {
            motif: MotifConflit::TiersDejaMouvemente,
            nombre: None,
        },
        ErreurTiers::Base(e) => ErreurApi::from(e),
    }
}

fn aucun_changement() -> ErreurApi {
    ErreurApi::Conflit {
        motif: MotifConflit::AucunChangement,
        nombre: None,
    }
}

async fn creer(
    tx: &mut TransactionTenant,
    dossier: &DossierAutorise,
    requete: &Requete<'_>,
) -> Result<Resultat, ErreurApi> {
    let c: CorpsCreation = corps(requete.corps)?;
    let genre = TypeTiers::parse(&c.genre).ok_or(ErreurApi::RequeteInvalide)?;
    let compte_collectif =
        NumeroCompte::parse(&c.compte_collectif).map_err(|_| ErreurApi::RequeteInvalide)?;
    let nouveau = NouveauTiers {
        id: Uuid::new_v4(),
        code: c.code.as_deref().map(code).transpose()?,
        nom: nom(&c.nom)?,
        genre,
        compte_collectif,
        devise: c.devise.as_deref().map(devise).transpose()?,
        assujetti_tva: c.assujetti_tva,
        soumis_retenue: c.soumis_retenue,
    };
    let condition = match c.condition_reglement.as_deref() {
        Some(v) => Some(condition_citee(tx, v).await?),
        None => None,
    };
    tiers::creer(tx, &nouveau).await.map_err(erreur_tiers)?;
    if condition.is_some() {
        tiers::changer_condition_reglement(tx, nouveau.id, condition).await?;
    }
    let mut t = trace(dossier.id(), "tiers", nouveau.id);
    t.donnees.push(("operation", "creation".to_owned()));
    t.donnees.push(("type", genre.nom().to_owned()));
    t.donnees
        .push(("compte_collectif", nouveau.compte_collectif.to_string()));
    let renseignes = champs_renseignes(&[
        ("code", c.code.is_some()),
        ("nom", true),
        ("devise", c.devise.is_some()),
        ("assujetti_tva", c.assujetti_tva),
        ("soumis_retenue", c.soumis_retenue),
        ("condition_reglement", condition.is_some()),
    ]);
    t.donnees.push(("champs", renseignes.join(",")));
    let vue = fiche(tx, nouveau.id).await?;
    Ok(Resultat {
        reponse: (StatusCode::CREATED, Json(vue)).into_response(),
        traces: vec![t],
    })
}

/// Verrouille le tiers du chemin et relit sa fiche. Absent du dossier : 404.
async fn fiche_verrouillee(
    tx: &mut TransactionTenant,
    requete: &Requete<'_>,
) -> Result<FicheTiers, ErreurApi> {
    let id = id_chemin(requete.chemin, "tiers")?;
    if !tiers::verrouiller(tx, id).await? {
        return Err(ErreurApi::Introuvable);
    }
    tiers::lire_fiche(tx, id)
        .await?
        .ok_or(ErreurApi::Introuvable)
}

#[allow(clippy::too_many_lines)] // En-tête puis trois listes, dans l'ordre : une suite, pas une logique.
async fn modifier(
    tx: &mut TransactionTenant,
    dossier: &DossierAutorise,
    requete: &Requete<'_>,
) -> Result<Resultat, ErreurApi> {
    let c: CorpsModification = corps(requete.corps)?;
    if c.est_vide() {
        return Err(ErreurApi::RequeteInvalide);
    }
    let pack = PackPays::officiel().map_err(|_| ErreurApi::Interne(CategorieErreur::Donnee))?;
    // Contrôles de forme avant tout verrou.
    let nom_demande = c.nom.as_deref().map(nom).transpose()?;
    let devise_demandee = match &c.devise {
        ChampTexte::Absent => None,
        ChampTexte::Efface => Some(None),
        ChampTexte::Valeur(v) => Some(Some(devise(v)?)),
    };
    let condition_demandee = match &c.condition_reglement {
        ChampTexte::Absent => None,
        ChampTexte::Efface => Some(None),
        ChampTexte::Valeur(v) => Some(Some(condition_citee(tx, v).await?)),
    };
    let identifiants_demandes = c
        .identifiants
        .as_deref()
        .map(|l| identifiants(pack, &dossier.contexte().pays, l))
        .transpose()?;
    let adresses_demandees = c
        .adresses
        .as_deref()
        .map(|l| {
            if l.len() > ADRESSES_MAX {
                return Err(limite(MotifValidation::TiersTropDAdresses));
            }
            l.iter().map(adresse).collect::<Result<Vec<_>, _>>()
        })
        .transpose()?;
    let contacts_demandes = c
        .contacts
        .as_deref()
        .map(|l| {
            if l.len() > CONTACTS_MAX {
                return Err(limite(MotifValidation::TiersTropDeContacts));
            }
            l.iter().map(contact).collect::<Result<Vec<_>, _>>()
        })
        .transpose()?;

    let genre_demande = c
        .genre
        .as_deref()
        .map(|g| TypeTiers::parse(g).ok_or(ErreurApi::RequeteInvalide))
        .transpose()?;
    let compte_demande = c
        .compte_collectif
        .as_deref()
        .map(|n| NumeroCompte::parse(n).map_err(|_| ErreurApi::RequeteInvalide))
        .transpose()?;
    // Ordre des verrous : référentiel, puis tiers (celui de `changer_classement`).
    if (genre_demande.is_some() || compte_demande.is_some())
        && !crate::base::referentiel::verrouiller(tx).await?
    {
        return Err(ErreurApi::Introuvable);
    }
    let avant = fiche_verrouillee(tx, requete).await?;
    let id = avant.entete.id;
    let mut traces = Vec::new();

    // Type et compte collectif : changés seulement avant le premier mouvement.
    let genre = genre_demande.unwrap_or(avant.entete.genre);
    let compte =
        compte_demande.map_or_else(|| avant.entete.compte_collectif.clone(), |n| n.to_string());
    let classement: Vec<&'static str> = [
        ("type", genre != avant.entete.genre),
        ("compte_collectif", compte != avant.entete.compte_collectif),
    ]
    .into_iter()
    .filter_map(|(n, change)| change.then_some(n))
    .collect();
    if !classement.is_empty() {
        let numero = NumeroCompte::parse(&compte)
            .map_err(|_| ErreurApi::Interne(CategorieErreur::Donnee))?;
        tiers::changer_classement(tx, &avant.entete, genre, &numero)
            .await
            .map_err(erreur_tiers)?;
        let mut t = trace(dossier.id(), "tiers", id);
        t.donnees.push(("operation", "classement".to_owned()));
        t.donnees.push(("champs", classement.join(",")));
        traces.push(t);
    }

    // En-tête.
    let mut entete = avant.entete.clone();
    if let Some(n) = nom_demande {
        n.as_str().clone_into(&mut entete.nom);
    }
    if let Some(d) = devise_demandee {
        entete.devise = d;
    }
    if let Some(a) = c.assujetti_tva {
        entete.assujetti_tva = a;
    }
    if let Some(s) = c.soumis_retenue {
        entete.soumis_retenue = s;
    }
    if let Some(condition) = condition_demandee {
        entete.condition_reglement = condition;
    }
    let mut champs: Vec<&'static str> = [
        ("nom", entete.nom != avant.entete.nom),
        ("devise", entete.devise != avant.entete.devise),
        (
            "assujetti_tva",
            entete.assujetti_tva != avant.entete.assujetti_tva,
        ),
        (
            "soumis_retenue",
            entete.soumis_retenue != avant.entete.soumis_retenue,
        ),
    ]
    .into_iter()
    .filter_map(|(n, change)| change.then_some(n))
    .collect();
    if entete.condition_reglement != avant.entete.condition_reglement {
        tiers::changer_condition_reglement(tx, id, entete.condition_reglement).await?;
        champs.push("condition_reglement");
    }
    if !champs.is_empty() {
        tiers::modifier_entete(tx, &entete).await?;
        let mut t = trace(dossier.id(), "tiers", id);
        t.donnees.push(("operation", "modification".to_owned()));
        t.donnees.push(("champs", champs.join(",")));
        traces.push(t);
    }

    let mut nouvel_id = Uuid::new_v4;
    // Identifiants légaux : le changement d'un identifiant est journalisé (ADR 0031 point 2),
    // par son type, jamais par sa valeur.
    if let Some(demandes) = &identifiants_demandes {
        for (genre, operation) in
            differences_identifiants(&avant.identifiants, demandes, &mut nouvel_id)
        {
            let ligne = operation.ligne();
            let champs: &[&str] = match &operation {
                Operation::Ajout(..) | Operation::Modification(..) => &["valeur"],
                Operation::Retrait(_) => &[],
            };
            let nom_op = operation.nom();
            let operation = match &operation {
                Operation::Ajout(l, v) => Operation::Ajout(*l, (genre.as_str(), v.as_str())),
                Operation::Modification(l, v) => {
                    Operation::Modification(*l, (genre.as_str(), v.as_str()))
                }
                Operation::Retrait(l) => Operation::Retrait(*l),
            };
            tiers::ecrire_identifiant(tx, id, operation).await?;
            traces.push(trace_ligne(
                dossier.id(),
                "tiers_identifiant",
                id,
                nom_op,
                ligne,
                champs,
                vec![("type", genre.clone())],
            ));
        }
    }
    if let Some(demandes) = adresses_demandees {
        for operation in differences(&avant.adresses, demandes, &mut nouvel_id)? {
            let (ligne, champs) = match &operation {
                Operation::Ajout(l, a) => (*l, champs_adresse(&Adresse::default(), a)),
                Operation::Modification(l, a) => {
                    let ancien = avant
                        .adresses
                        .iter()
                        .find(|(i, _)| i == l)
                        .map(|(_, x)| x)
                        .ok_or(ErreurApi::Interne(CategorieErreur::Donnee))?;
                    (*l, champs_adresse(ancien, a))
                }
                Operation::Retrait(l) => (*l, Vec::new()),
            };
            let nom_op = operation.nom();
            tiers::ecrire_adresse(tx, id, operation.as_ref()).await?;
            traces.push(trace_ligne(
                dossier.id(),
                "tiers_adresse",
                id,
                nom_op,
                ligne,
                &champs,
                Vec::new(),
            ));
        }
    }
    if let Some(demandes) = contacts_demandes {
        for operation in differences(&avant.contacts, demandes, &mut nouvel_id)? {
            let (ligne, champs, genre) = match &operation {
                Operation::Ajout(l, c) => (
                    *l,
                    champs_renseignes(&[
                        ("type", true),
                        ("valeur", true),
                        ("nom", c.nom.is_some()),
                    ]),
                    Some(c.type_contact.clone()),
                ),
                Operation::Modification(l, c) => {
                    let ancien = avant
                        .contacts
                        .iter()
                        .find(|(i, _)| i == l)
                        .map(|(_, x)| x)
                        .ok_or(ErreurApi::Interne(CategorieErreur::Donnee))?;
                    (*l, champs_contact(ancien, c), Some(c.type_contact.clone()))
                }
                Operation::Retrait(l) => (*l, Vec::new(), None),
            };
            let nom_op = operation.nom();
            tiers::ecrire_contact(tx, id, operation.as_ref()).await?;
            traces.push(trace_ligne(
                dossier.id(),
                "tiers_contact",
                id,
                nom_op,
                ligne,
                &champs,
                genre.map(|g| ("type", g)).into_iter().collect(),
            ));
        }
    }
    if traces.is_empty() {
        return Err(aucun_changement());
    }
    let vue = fiche(tx, id).await?;
    Ok(Resultat {
        reponse: Json(vue).into_response(),
        traces,
    })
}

async fn archiver(
    cas: Cas,
    tx: &mut TransactionTenant,
    garde: &Garde,
    dossier: &DossierAutorise,
    requete: &Requete<'_>,
) -> Result<Resultat, ErreurApi> {
    let motif = if cas == Cas::Archiver {
        Some(texte(
            &corps::<CorpsArchivage>(requete.corps)?.motif,
            MOTIF_MAX,
        )?)
    } else {
        corps::<CorpsVide>(requete.corps)?;
        None
    };
    let id = id_chemin(requete.chemin, "tiers")?;
    let operation = if let Some(motif) = motif {
        let avant = fiche_verrouillee(tx, requete).await?;
        if !avant.entete.actif {
            return Err(aucun_changement());
        }
        tiers::archiver(tx, id, &motif, garde.utilisateur().uuid()).await?;
        "archivage"
    } else {
        // Le verrou du référentiel (contrôle croisé du désarchivage) précède celui du tiers.
        if !crate::base::referentiel::verrouiller(tx).await? {
            return Err(ErreurApi::Introuvable);
        }
        let avant = fiche_verrouillee(tx, requete).await?;
        if avant.entete.actif {
            return Err(aucun_changement());
        }
        tiers::desarchiver(tx, &avant.entete)
            .await
            .map_err(erreur_tiers)?;
        "desarchivage"
    };
    // Le motif est un texte libre : stocké sur le tiers, jamais dans l'audit.
    let mut t = trace(dossier.id(), "tiers", id);
    t.donnees.push(("operation", operation.to_owned()));
    let vue = fiche(tx, id).await?;
    Ok(Resultat {
        reponse: Json(vue).into_response(),
        traces: vec![t],
    })
}

async fn modifier_comptes_bancaires(
    tx: &mut TransactionTenant,
    garde: &Garde,
    dossier: &DossierAutorise,
    requete: &Requete<'_>,
) -> Result<Resultat, ErreurApi> {
    let c: CorpsComptesBancaires = corps(requete.corps)?;
    if c.comptes.len() > COMPTES_MAX {
        return Err(limite(MotifValidation::TiersTropDeComptesBancaires));
    }
    let pack = PackPays::officiel().map_err(|_| ErreurApi::Interne(CategorieErreur::Donnee))?;
    let demandes = c
        .comptes
        .iter()
        .map(|c| compte_bancaire(pack, c))
        .collect::<Result<Vec<_>, _>>()?;
    let avant = fiche_verrouillee(tx, requete).await?;
    let id = avant.entete.id;
    let existants = tiers::lire_comptes_bancaires(tx, id).await?;
    let mut traces = Vec::new();
    for operation in differences(&existants, demandes, &mut Uuid::new_v4)? {
        let (ligne, champs, genre) = match &operation {
            Operation::Ajout(l, c) => (
                *l,
                champs_renseignes(&[
                    ("type", true),
                    ("numero", true),
                    ("etablissement", c.etablissement.is_some()),
                    ("devise", c.devise.is_some()),
                ]),
                Some(c.type_identifiant.clone()),
            ),
            Operation::Modification(l, c) => {
                let ancien = existants
                    .iter()
                    .find(|(i, _)| i == l)
                    .map(|(_, x)| x)
                    .ok_or(ErreurApi::Interne(CategorieErreur::Donnee))?;
                (
                    *l,
                    champs_compte(ancien, c),
                    Some(c.type_identifiant.clone()),
                )
            }
            Operation::Retrait(l) => (*l, Vec::new(), None),
        };
        let nom_op = operation.nom();
        tiers::ecrire_compte_bancaire(tx, id, operation.as_ref()).await?;
        traces.push(trace_ligne(
            dossier.id(),
            "tiers_compte_bancaire",
            id,
            nom_op,
            ligne,
            &champs,
            genre.map(|g| ("type", g)).into_iter().collect(),
        ));
    }
    if traces.is_empty() {
        return Err(aucun_changement());
    }
    // Alerte aux administrateurs du cabinet (ADR 0031 point 3, F1.26), dans la transaction de
    // la modification : aucune donnée du tiers, seul le dossier.
    notifier(
        tx,
        SujetNotification::CoordonneesBancairesTiers,
        Some(garde.utilisateur().uuid()),
    )
    .await?;
    let vue = comptes_bancaires(tx, id).await?;
    Ok(Resultat {
        reponse: Json(vue).into_response(),
        traces,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pack() -> &'static PackPays {
        PackPays::officiel().unwrap()
    }

    fn identifiant(genre: &str, valeur: &str) -> CorpsIdentifiant {
        CorpsIdentifiant {
            genre: genre.to_owned(),
            valeur: valeur.to_owned(),
        }
    }

    fn validation(motif: MotifValidation) -> ErreurApi {
        ErreurApi::Validation { motif, ligne: None }
    }

    #[test]
    fn identifiants_types_par_le_pays_du_dossier() {
        // Sénégal : RCCM et AUTRE (communs), NINEA (propre).
        let lus = identifiants(
            pack(),
            "SN",
            &[
                identifiant("NINEA", "0012345 2G3"),
                identifiant("RCCM", "SN-DKR-1"),
            ],
        )
        .unwrap();
        assert_eq!(lus.len(), 2);
        // Cameroun : NINEA n'est pas un type du pack de ce pays.
        assert_eq!(
            identifiants(pack(), "CM", &[identifiant("NINEA", "1")]),
            Err(validation(MotifValidation::TiersIdentifiantTypeInconnu))
        );
        // Un tiers étranger : AUTRE admis partout.
        assert!(identifiants(pack(), "CM", &[identifiant("AUTRE", "FR123")]).is_ok());
        // Un seul identifiant par type.
        assert_eq!(
            identifiants(
                pack(),
                "SN",
                &[identifiant("RCCM", "1"), identifiant("RCCM", "2")]
            ),
            Err(validation(MotifValidation::TiersIdentifiantTypeInconnu))
        );
        // Valeur vide ou trop longue : refusée.
        for valeur in ["", " ", &"1".repeat(65)] {
            assert_eq!(
                identifiants(pack(), "SN", &[identifiant("NINEA", valeur)]),
                Err(invalide()),
                "{valeur:?}"
            );
        }
        // Caractère de contrôle (C0, C1) ou de format Unicode (`Cf`) : code propre (F11.8).
        for c in [
            '\n',
            '\u{7}',
            '\u{85}',
            '\u{202E}',
            '\u{200B}',
            '\u{200D}',
            '\u{2060}',
            '\u{FEFF}',
            '\u{AD}',
            '\u{61C}',
            '\u{E0041}',
        ] {
            let valeur = format!("12{c}3");
            assert_eq!(
                identifiants(pack(), "SN", &[identifiant("NINEA", &valeur)]),
                Err(validation(MotifValidation::TiersCaractereInvalide)),
                "{valeur:?}"
            );
        }
        // L'espace insécable n'est pas un caractère de format : admis.
        assert!(identifiants(pack(), "SN", &[identifiant("NINEA", "12\u{A0}3")]).is_ok());
        // Aucun format imposé : une valeur quelconque, bornée, est admise (aucun n'est sourcé).
        assert!(identifiants(pack(), "SN", &[identifiant("NINEA", "n'importe quoi")]).is_ok());
        let trop: Vec<CorpsIdentifiant> = (0..=IDENTIFIANTS_MAX)
            .map(|i| identifiant("AUTRE", &i.to_string()))
            .collect();
        assert_eq!(
            identifiants(pack(), "SN", &trop),
            Err(validation(MotifValidation::TiersTropDIdentifiants))
        );
    }

    #[test]
    fn formes_du_code_de_la_devise_et_du_pays() {
        for valide in ["T000001", "C-01", "A.B_C", "0", &"Z".repeat(20)] {
            assert!(code(valide).is_ok(), "{valide}");
        }
        for invalide_ in ["", "t1", "A B", "É1", &"Z".repeat(21), "A/B"] {
            assert!(code(invalide_).is_err(), "{invalide_}");
        }
        assert!(devise("EUR").is_ok() && devise("GBP").is_ok());
        assert!(devise("eur").is_err() && devise("EURO").is_err());
        assert!(pays("FR").is_ok() && pays("SN").is_ok());
        assert!(pays("fra").is_err() && pays("F").is_err());
        // Nom : grammaire de `TexteCourt` et aucun caractère de format (F11.8).
        assert!(nom("Société Diop & fils").is_ok());
        assert!(nom("abc\u{202E}def").is_err() && nom("a\tb").is_err() && nom(" ").is_err());
    }

    #[test]
    fn types_de_coordonnees_bancaires_du_pack() {
        let corps = |genre: &str| CorpsCompteBancaire {
            id: None,
            genre: genre.to_owned(),
            numero: "SN012 01001 012345678901 23".to_owned(),
            etablissement: None,
            devise: Some("XOF".to_owned()),
        };
        for genre in ["RIB", "IBAN", "PORTEFEUILLE", "AUTRE"] {
            assert!(compte_bancaire(pack(), &corps(genre)).is_ok(), "{genre}");
        }
        assert_eq!(
            compte_bancaire(pack(), &corps("SWIFT")).map(|_| ()),
            Err(validation(MotifValidation::TiersCompteBancaireTypeInconnu))
        );
        // Aucune règle de forme : un IBAN « invalide » est admis (aucune n'est sourcée).
        let mut iban = corps("IBAN");
        iban.numero = "XX00".to_owned();
        assert!(compte_bancaire(pack(), &iban).is_ok());
    }

    #[test]
    fn courriel_mal_forme_signale_seulement() {
        for bien in ["a@b.sn", "prenom.nom@exemple.co.sn"] {
            assert!(courriel_bien_forme(bien), "{bien}");
        }
        for mal in [
            "a", "a@b", "@b.sn", "a@@b.sn", "a@b.sn.", "a b@c.sn", "a@.sn",
        ] {
            assert!(!courriel_bien_forme(mal), "{mal}");
        }
        // La saisie reste admise : la forme n'est qu'un avertissement de la vue.
        let c = CorpsContact {
            id: None,
            genre: "courriel".to_owned(),
            valeur: "pas un courriel".to_owned(),
            nom: None,
        };
        assert!(contact(&c).is_ok());
        let mut autre = c;
        autre.genre = "pigeon".to_owned();
        assert_eq!(contact(&autre).map(|_| ()), Err(invalide()));
    }

    #[test]
    fn une_liste_remplace_la_liste_entiere() {
        let (a, b, c) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
        let existant = vec![(a, 1), (b, 2)];
        let mut n = 0;
        let mut nouvel_id = || {
            n += 1;
            c
        };
        // `a` inchangé, `b` modifié, une ligne ajoutée ; rien d'autre.
        let ops = differences(
            &existant,
            vec![(Some(a), 1), (Some(b), 3), (None, 4)],
            &mut nouvel_id,
        )
        .unwrap();
        assert_eq!(
            ops,
            vec![Operation::Modification(b, 3), Operation::Ajout(c, 4)]
        );
        // Liste vide : tout est retiré.
        let ops = differences(&existant, Vec::new(), &mut nouvel_id).unwrap();
        assert_eq!(ops, vec![Operation::Retrait(a), Operation::Retrait(b)]);
        // Ligne inconnue (d'un autre tiers ou d'un autre dossier) : absente.
        assert_eq!(
            differences(&existant, vec![(Some(Uuid::new_v4()), 1)], &mut nouvel_id),
            Err(ErreurApi::Introuvable)
        );
        // Ligne citée deux fois : 400.
        assert_eq!(
            differences(&existant, vec![(Some(a), 1), (Some(a), 1)], &mut nouvel_id),
            Err(ErreurApi::RequeteInvalide)
        );
        // Même liste : aucune opération.
        assert_eq!(
            differences(&existant, vec![(Some(a), 1), (Some(b), 2)], &mut nouvel_id),
            Ok(Vec::new())
        );
    }

    #[test]
    fn identifiants_reperes_par_leur_type() {
        let (r, n) = (Uuid::new_v4(), Uuid::new_v4());
        let existant = vec![
            tiers::Identifiant {
                id: r,
                type_identifiant: "RCCM".to_owned(),
                valeur: "1".to_owned(),
            },
            tiers::Identifiant {
                id: n,
                type_identifiant: "NINEA".to_owned(),
                valeur: "2".to_owned(),
            },
        ];
        let nouveau = Uuid::new_v4();
        let demande: BTreeMap<String, String> = [
            ("RCCM".to_owned(), "1".to_owned()),
            ("AUTRE".to_owned(), "3".to_owned()),
        ]
        .into_iter()
        .collect();
        let ops = differences_identifiants(&existant, &demande, &mut || nouveau);
        assert_eq!(
            ops,
            vec![
                (
                    "AUTRE".to_owned(),
                    Operation::Ajout(nouveau, "3".to_owned())
                ),
                ("NINEA".to_owned(), Operation::Retrait(n)),
            ]
        );
    }

    #[test]
    fn champs_modifies_par_leur_nom() {
        let a = Adresse {
            ville: Some("Dakar".to_owned()),
            pays: Some("SN".to_owned()),
            ..Adresse::default()
        };
        let b = Adresse {
            ville: Some("Thiès".to_owned()),
            pays: Some("SN".to_owned()),
            rue: Some("Rue 1".to_owned()),
            ..Adresse::default()
        };
        assert_eq!(champs_adresse(&a, &b), ["rue", "ville"]);
        // Le changement du pays d'une adresse est journalisé (ADR 0031 point 2).
        let c = Adresse {
            pays: Some("ML".to_owned()),
            ..a.clone()
        };
        assert_eq!(champs_adresse(&a, &c), ["pays"]);
    }

    #[test]
    fn corps_de_la_modification() {
        let c: CorpsModification = corps(br#"{"devise":null}"#).unwrap();
        assert_eq!(c.devise, ChampTexte::Efface);
        assert!(!c.est_vide());
        let c: CorpsModification = corps(b"{}").unwrap();
        assert!(c.est_vide());
        // L'archivage passe par sa route : `actif` n'est plus un champ de la modification.
        assert!(corps::<CorpsModification>(br#"{"actif":false}"#).is_err());
        // Code immuable : refusé (champ inconnu). Type et compte collectif : admis par le corps,
        // contrôlés ensuite (premier mouvement, contrôle croisé du référentiel).
        assert!(corps::<CorpsModification>(br#"{"code":"X"}"#).is_err());
        let c: CorpsModification =
            corps(br#"{"type":"fournisseur","compte_collectif":"401"}"#).unwrap();
        assert!(!c.est_vide());
    }

    #[test]
    fn pagination_des_tiers() {
        let id = Uuid::new_v4();
        assert_eq!(pagination(None), Ok((None, PAGE_PAR_DEFAUT)));
        assert_eq!(
            pagination(Some(&format!("apres={id}&limite=3"))),
            Ok((Some(id), 3))
        );
        for invalide_ in [
            "limite=0",
            "limite=201",
            "apres=x",
            "limite=1&limite=2",
            "autre=1",
            "limite",
        ] {
            assert_eq!(
                pagination(Some(invalide_)),
                Err(ErreurApi::RequeteInvalide),
                "{invalide_}"
            );
        }
    }
}
