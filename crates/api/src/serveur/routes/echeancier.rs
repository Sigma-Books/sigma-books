//! Échéancier : conditions de règlement, assistant d'échéances, échéances des lignes validées,
//! révisions (L3-17, PR 8 de `docs/lots/L3.md` ; ADR 0033).
//!
//! Persistance : [`crate::base::echeancier`] ; calcul : `compta_core::echeance` (pur), jamais
//! dans ce module (P6). Migrations `0510` à `0514`.
//!
//! # Conditions de règlement (ADR 0033 point 3)
//! Configuration du dossier, modifiable et journalisée : code stable (unique, immuable), libellé,
//! activité, termes (pourcentage du total, montant fixe ou solde ; délai parmi les quatre types
//! d'Odoo). Le dernier terme est le solde, la somme des pourcentages reste sous 100 : contrôlé par
//! `compta-core` (Tryton `payment_term.py:40-47`, Odoo `account_payment_term.py:156-162`,
//! `:226-230`). Les **modèles d'usage** ([`MODELES_D_USAGE`]) sont créés avec le dossier
//! ([`creer_modeles_d_usage`]) et restent éditables ; aucune valeur légale de délai n'est livrée.
//!
//! # Assistant à la saisie
//! `GET …/conditions-reglement/{condition}/echeances?date=&montant=` rend les termes calculés par
//! `ConditionReglement::repartir` (une ligne de tiers par terme, Tryton `invoice.py:1342-1354`),
//! sans rien écrire. L'interface n'en calcule aucun.
//!
//! # Échéances des lignes validées (ADR 0033 points 1 et 2)
//! L'échéance initiale est copiée du brouillon à la validation (`base::validation::inserer`) ;
//! elle se **révise en ajout seul** (`echeance_report`, motif obligatoire), tant que le résiduel de
//! la ligne n'est pas nul (409 `echeance_soldee`). Le résiduel vient du lettrage
//! ([`crate::base::echeancier::residuel`], interface que la PR 9a branche). Le statut (ouverte,
//! partiellement réglée, soldée) et la situation (à échoir, échue, soldée) sont **calculés**,
//! jamais stockés (Odoo `account_move.py:49-57`, Tryton `invoice.py:883-895`).
//!
//! # Audit sans valeur personnelle
//! Condition : opération, code, noms des champs modifiés, nombre de termes. Révision : écriture,
//! position, rang, dates ; **jamais le motif** (texte libre).
//!
//! # Défense en profondeur (ADR 0011)
//! Dossier du chemin relu **visible**, rôle de l'appartenance active relu en base (rôles des
//! politiques Cedar de l'action), condition et écriture cherchées dans le seul dossier du contexte.

use authz::{Action, Role};
use axum::Json;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use compta_core::echeance::{
    ConditionReglement, Delai, EcheanceLigne, ErreurEcheance, JourDuMois, MontantFixe,
    MotifRevision, Pourcentage, Residuel, Revision, Situation, StatutReglement, Terme, ValeurTerme,
};
use compta_core::{DeviseTenue, ErreurMontant, Montant};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use time::Date;
use uuid::Uuid;

use super::brouillard::date_lue;
use super::{ActionRoute, CasUsage, Methode, Module, Portee, Requete, Resultat, Route};
use crate::base::TransactionTenant;
use crate::base::audit::PorteeAudit;
use crate::base::brouillard::{RefusSaisie, decimal_canonique};
use crate::base::echeancier::{
    self as base, ConditionLue, EcritureEcheances, LigneTiersValidee, NouvelleRevision,
    RevisionLue, TermeStocke,
};
use crate::base::organisation::{lire_appartenance, lire_dossier_visible};
use crate::contexte_dossier::texte_borne;
use crate::pieces::interne;
use crate::serveur::audit::Trace;
use crate::serveur::erreur::{CategorieErreur, ErreurApi, MotifConflit, MotifValidation};
use crate::serveur::pipeline::{DossierAutorise, Garde, id_chemin};

/// Cas d'usage du module.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cas {
    /// Conditions de règlement du dossier.
    ListerConditions,
    /// Créer une condition de règlement.
    CreerCondition,
    /// Une condition de règlement.
    LireCondition,
    /// Modifier une condition (libellé, activité, termes).
    ModifierCondition,
    /// Assistant à la saisie : termes calculés pour une date et un montant, sans écriture.
    CalculerEcheances,
    /// Échéances des lignes de tiers d'une écriture validée, statut et situation calculés.
    LireEcheances,
    /// Réviser l'échéance d'une ligne de tiers validée (ajout seul).
    ReviserEcheance,
}

/// Tranche du module.
pub const MODULE: Module = Module {
    nom: "echeancier",
    routes: ROUTES,
    champs_journal: &[],
};

const fn route(methode: Methode, chemin: &'static str, action: Action, cas: Cas) -> Route {
    Route {
        methode,
        chemin,
        action: ActionRoute::Cabinet(action),
        portee: Portee::Dossier,
        cas_usage: CasUsage::Echeancier(cas),
    }
}

/// Routes du module (actions de `crates/authz/politiques/roles/tiers.cedar` ; l'assistant relève
/// de la saisie, `SaisirBrouillard`).
pub const ROUTES: &[Route] = &[
    route(
        Methode::Get,
        "/api/cabinets/{cabinet}/dossiers/{dossier}/conditions-reglement",
        Action::ConsulterTiers,
        Cas::ListerConditions,
    ),
    route(
        Methode::Post,
        "/api/cabinets/{cabinet}/dossiers/{dossier}/conditions-reglement",
        Action::ParametrerConditionsReglement,
        Cas::CreerCondition,
    ),
    route(
        Methode::Get,
        "/api/cabinets/{cabinet}/dossiers/{dossier}/conditions-reglement/{condition}",
        Action::ConsulterTiers,
        Cas::LireCondition,
    ),
    route(
        Methode::Post,
        "/api/cabinets/{cabinet}/dossiers/{dossier}/conditions-reglement/{condition}/modification",
        Action::ParametrerConditionsReglement,
        Cas::ModifierCondition,
    ),
    route(
        Methode::Get,
        "/api/cabinets/{cabinet}/dossiers/{dossier}/conditions-reglement/{condition}/echeances",
        Action::SaisirBrouillard,
        Cas::CalculerEcheances,
    ),
    route(
        Methode::Get,
        "/api/cabinets/{cabinet}/dossiers/{dossier}/echeances/{ecriture}",
        Action::ConsulterEditionsTiers,
        Cas::LireEcheances,
    ),
    route(
        Methode::Post,
        "/api/cabinets/{cabinet}/dossiers/{dossier}/echeances/{ecriture}/revisions",
        Action::ReviserEcheance,
        Cas::ReviserEcheance,
    ),
];

// --- Rôles et bornes -------------------------------------------------------------------------

/// Rôles qui lisent conditions et échéances (Cedar `l3-c6-consulter-tiers`,
/// `l3-c6-consulter-editions-tiers`).
const ROLES_LECTURE: &[Role] = &[
    Role::Administrateur,
    Role::Valideur,
    Role::Collaborateur,
    Role::Auditeur,
];
/// Rôles de la saisie et de la révision (Cedar `s20-saisir-brouillard`,
/// `l3-c6-tenir-fiche-tiers-et-echeances`).
const ROLES_SAISIE: &[Role] = &[Role::Administrateur, Role::Valideur, Role::Collaborateur];
/// Rôles du paramétrage des conditions (Cedar `l3-c6-parametrer-tiers`).
const ROLES_PARAMETRAGE: &[Role] = &[Role::Administrateur, Role::Valideur];

/// Nombre maximal de termes d'une condition (limite de sécurité contre un corps démesuré,
/// F2.16 ; contrainte de la migration `0510`). Tryton et Odoo n'en fixent pas.
pub const TERMES_MAX: usize = 50;
/// Longueur maximale du libellé d'une condition (migration `0510`).
const LIBELLE_MAX: usize = 100;
/// Chiffres entiers au plus d'un montant fixe (`numeric` < 10^20, comme les lignes).
const CHIFFRES_ENTIERS_MAX: usize = 20;

// --- Modèles d'usage -------------------------------------------------------------------------

/// Terme d'un modèle d'usage : valeur (`pourcentage`, `solde`), pourcentage, délai, jours, jour.
type TermeModele = (
    &'static str,
    Option<&'static str>,
    &'static str,
    i32,
    Option<u8>,
);

/// **Modèles d'usage** créés avec chaque dossier (ADR 0033 point 3 : « modèles d'usage fournis à
/// la création du dossier, éditables ; aucune valeur légale de délai n'est livrée »). Ce ne sont
/// pas des données légales d'un pays : constante documentée, pas une donnée de pack. Repris des
/// conditions livrées par Odoo (`addons/account/data/account_data.xml:34-100`, commit `fa175fb` :
/// paiement immédiat, 15, 30 et 45 jours, fin du mois suivant, « 30 % maintenant, solde à
/// 60 jours »), plus « 30 jours fin de mois », condition du critère de fin du lot L3
/// (`docs/lots/L3.md`), au sens du délai d'Odoo `days_end_of_month_on_the` avec un jour nul
/// (`compta_core::echeance::Delai::JoursPuisJourDuMoisSuivant`).
pub const MODELES_D_USAGE: &[(&str, &str, &[TermeModele])] = &[
    ("COMPTANT", "Comptant", &[("solde", None, "jours", 0, None)]),
    ("15J", "15 jours", &[("solde", None, "jours", 15, None)]),
    ("30J", "30 jours", &[("solde", None, "jours", 30, None)]),
    ("45J", "45 jours", &[("solde", None, "jours", 45, None)]),
    (
        "30JFM",
        "30 jours fin de mois",
        &[("solde", None, "jour_du_mois_suivant", 30, Some(0))],
    ),
    (
        "FMS",
        "Fin du mois suivant",
        &[("solde", None, "fin_du_mois_suivant", 0, None)],
    ),
    (
        "30C-60J",
        "30 % comptant, solde à 60 jours",
        &[
            ("pourcentage", Some("30"), "jours", 0, None),
            ("solde", None, "jours", 60, None),
        ],
    ),
];

fn termes_du_modele(termes: &[TermeModele]) -> Vec<TermeStocke> {
    termes
        .iter()
        .map(|(valeur, montant, delai, jours, jour)| TermeStocke {
            valeur: (*valeur).to_owned(),
            montant: montant.map(str::to_owned),
            delai: (*delai).to_owned(),
            jours: *jours,
            jour_du_mois: *jour,
        })
        .collect()
}

/// Crée les modèles d'usage dans le dossier du contexte (création d'un dossier, après
/// l'instanciation du référentiel, qui restreint la transaction au dossier). Rend leur nombre,
/// cité par la trace de la création du dossier.
///
/// # Errors
/// Erreur de PostgreSQL ; modèle invalide (500 : défaut du code, couvert par les tests).
pub(crate) async fn creer_modeles_d_usage(
    tx: &mut TransactionTenant,
    instant: time::OffsetDateTime,
) -> Result<usize, ErreurApi> {
    for (code, libelle, termes) in MODELES_D_USAGE {
        let condition = ConditionLue {
            id: Uuid::new_v4(),
            code: (*code).to_owned(),
            libelle: (*libelle).to_owned(),
            actif: true,
            termes: termes_du_modele(termes),
        };
        condition_pure(&condition.termes)
            .map_err(|_| ErreurApi::Interne(CategorieErreur::Donnee))?;
        if !base::creer_condition(tx, &condition, instant).await? {
            return Err(ErreurApi::Interne(CategorieErreur::Donnee));
        }
    }
    Ok(MODELES_D_USAGE.len())
}

// --- Conversions pures -------------------------------------------------------------------------

fn invalide() -> ErreurApi {
    ErreurApi::Validation {
        motif: MotifValidation::ConditionReglementInvalide,
        ligne: None,
    }
}

/// Décimal strictement positif en texte (`30`, `12.5`), sans signe ni exposant, au plus
/// [`CHIFFRES_ENTIERS_MAX`] chiffres entiers ; rendu sous sa forme canonique.
fn decimal_positif(texte: &str) -> Option<Decimal> {
    let (entier, fraction) = texte.split_once('.').unwrap_or((texte, ""));
    let forme = !entier.is_empty()
        && entier.len() <= CHIFFRES_ENTIERS_MAX
        && entier.bytes().all(|b| b.is_ascii_digit())
        && (!texte.contains('.') || !fraction.is_empty())
        && fraction.bytes().all(|b| b.is_ascii_digit());
    if !forme {
        return None;
    }
    Decimal::from_str_exact(texte)
        .ok()
        .filter(|d| *d > Decimal::ZERO)
}

/// Terme de `compta-core` d'un terme stocké ou demandé.
fn terme_pur(t: &TermeStocke) -> Result<Terme, ErreurEcheance> {
    let montant = || {
        t.montant
            .as_deref()
            .and_then(decimal_positif)
            .ok_or(ErreurEcheance::Depassement)
    };
    let valeur = match (t.valeur.as_str(), &t.montant) {
        ("solde", None) => ValeurTerme::Solde,
        ("pourcentage", Some(_)) => ValeurTerme::Pourcentage(Pourcentage::nouveau(montant()?)?),
        ("fixe", Some(_)) => ValeurTerme::Fixe(MontantFixe::nouveau(montant()?)?),
        _ => return Err(ErreurEcheance::Depassement),
    };
    let delai = match (t.delai.as_str(), t.jour_du_mois) {
        ("jours", None) => Delai::Jours(t.jours),
        ("fin_de_mois", None) => Delai::JoursApresFinDeMois(t.jours),
        ("fin_du_mois_suivant", None) => Delai::JoursApresFinDuMoisSuivant(t.jours),
        ("jour_du_mois_suivant", Some(jour)) => Delai::JoursPuisJourDuMoisSuivant {
            jours: t.jours,
            jour: JourDuMois::nouveau(jour)?,
        },
        _ => return Err(ErreurEcheance::Depassement),
    };
    Ok(Terme::nouveau(valeur, delai))
}

/// Condition de `compta-core` (contrôles de l'ADR 0033 point 3) de termes stockés ou demandés.
fn condition_pure(termes: &[TermeStocke]) -> Result<ConditionReglement, ErreurEcheance> {
    if termes.len() > TERMES_MAX {
        return Err(ErreurEcheance::Depassement);
    }
    ConditionReglement::nouvelle(termes.iter().map(terme_pur).collect::<Result<_, _>>()?)
}

/// Condition relue en base : un terme que `compta-core` refuse est une donnée incohérente (500),
/// jamais une condition tronquée.
fn condition_relue(c: &ConditionLue) -> Result<ConditionReglement, ErreurApi> {
    condition_pure(&c.termes).map_err(|_| ErreurApi::Interne(CategorieErreur::Donnee))
}

// --- Corps et vues -----------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct CorpsTerme {
    /// `pourcentage`, `fixe` ou `solde`.
    valeur: String,
    /// Pourcentage du total ou montant fixe (texte décimal) ; absent pour le solde.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    montant: Option<String>,
    /// `jours`, `fin_de_mois`, `fin_du_mois_suivant`, `jour_du_mois_suivant`.
    delai: String,
    /// Nombre de jours (0 par défaut).
    #[serde(default)]
    jours: i32,
    /// Jour du mois suivant (0 : fin du mois), délai `jour_du_mois_suivant` seulement.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    jour: Option<u8>,
}

impl CorpsTerme {
    fn stocke(&self) -> Result<TermeStocke, ErreurApi> {
        let montant = match &self.montant {
            None => None,
            Some(m) => Some(
                decimal_positif(m)
                    .ok_or_else(invalide)?
                    .normalize()
                    .to_string(),
            ),
        };
        Ok(TermeStocke {
            valeur: self.valeur.clone(),
            montant,
            delai: self.delai.clone(),
            jours: self.jours,
            jour_du_mois: self.jour,
        })
    }

    fn vue(t: &TermeStocke) -> Self {
        Self {
            valeur: t.valeur.clone(),
            montant: t.montant.clone(),
            delai: t.delai.clone(),
            jours: t.jours,
            jour: t.jour_du_mois,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CorpsCreation {
    code: String,
    libelle: String,
    termes: Vec<CorpsTerme>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CorpsModification {
    libelle: Option<String>,
    actif: Option<bool>,
    termes: Option<Vec<CorpsTerme>>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CorpsRevision {
    position: i16,
    nouvelle_echeance: String,
    motif: String,
}

#[derive(Debug, Serialize)]
struct VueCondition {
    id: String,
    code: String,
    libelle: String,
    actif: bool,
    termes: Vec<CorpsTerme>,
}

#[derive(Debug, Serialize)]
struct ListeConditions {
    conditions: Vec<VueCondition>,
}

#[derive(Debug, Serialize)]
struct VueTermeCalcule {
    date: String,
    montant: String,
}

#[derive(Debug, Serialize)]
struct VueCalcul {
    /// Condition appliquée.
    id: String,
    date: String,
    montant: String,
    termes: Vec<VueTermeCalcule>,
}

#[derive(Debug, Serialize)]
struct VueRevision {
    rang: i32,
    nouvelle_echeance: String,
    date_operation: String,
    motif: String,
    auteur: String,
    cree_le: String,
}

#[derive(Debug, Serialize)]
struct VueLigneEcheance {
    position: i16,
    compte: String,
    tiers: String,
    sens: String,
    montant: String,
    /// Échéance copiée à la validation (absente : repli sur la date comptable).
    echeance_initiale: Option<String>,
    /// Échéance effective à la date `au` (ADR 0033 point 2).
    echeance: String,
    residuel: String,
    /// `ouverte`, `partiellement_reglee`, `soldee`.
    statut: &'static str,
    /// `a_echoir`, `echue`, `soldee`.
    situation: &'static str,
    /// Retard en jours d'une échéance échue.
    #[serde(skip_serializing_if = "Option::is_none")]
    retard_jours: Option<i64>,
    revisions: Vec<VueRevision>,
}

#[derive(Debug, Serialize)]
struct VueEcheances {
    /// Écriture validée.
    id: String,
    date_comptable: String,
    au: String,
    lignes: Vec<VueLigneEcheance>,
}

fn vue_condition(c: &ConditionLue) -> VueCondition {
    VueCondition {
        id: c.id.to_string(),
        code: c.code.clone(),
        libelle: c.libelle.clone(),
        actif: c.actif,
        termes: c.termes.iter().map(CorpsTerme::vue).collect(),
    }
}

const fn nom_statut(s: StatutReglement) -> &'static str {
    match s {
        StatutReglement::Ouverte => "ouverte",
        StatutReglement::PartiellementReglee => "partiellement_reglee",
        StatutReglement::Soldee => "soldee",
    }
}

fn texte_decimal(d: Decimal) -> String {
    decimal_canonique(&d.normalize().to_string())
}

/// Échéance d'une ligne validée reconstituée pour `compta-core` (révisions dans l'ordre du rang).
fn echeance_de(
    date_comptable: Date,
    ligne: &LigneTiersValidee,
    revisions: &[RevisionLue],
) -> Result<EcheanceLigne, ErreurApi> {
    let donnee = || ErreurApi::Interne(CategorieErreur::Donnee);
    let lues = revisions
        .iter()
        .filter(|r| r.position == ligne.position)
        .map(|r| {
            Revision::nouvelle(
                u32::try_from(r.rang).map_err(|_| donnee())?,
                r.nouvelle_echeance,
                r.date_operation,
                MotifRevision::parse(&r.motif).map_err(|_| donnee())?,
            )
            .map_err(|_| donnee())
        })
        .collect::<Result<Vec<_>, _>>()?;
    EcheanceLigne::reconstituer(date_comptable, ligne.initiale, lues).map_err(|_| donnee())
}

/// Vue d'une ligne à la date `au`, d'après son résiduel (fourni par le lettrage).
fn vue_ligne(
    au: Date,
    ligne: &LigneTiersValidee,
    echeance: &EcheanceLigne,
    residuel: &Residuel,
    revisions: &[RevisionLue],
) -> VueLigneEcheance {
    let (situation, retard_jours) = match echeance.situation(au, residuel) {
        Situation::Soldee => ("soldee", None),
        Situation::AEchoir => ("a_echoir", None),
        Situation::Echue { retard_jours } => ("echue", Some(retard_jours)),
    };
    VueLigneEcheance {
        position: ligne.position,
        compte: ligne.compte.clone(),
        tiers: ligne.tiers.to_string(),
        sens: ligne.sens.clone(),
        montant: texte_decimal(ligne.montant),
        echeance_initiale: ligne.initiale.map(interne::date),
        echeance: interne::date(echeance.effective(au)),
        residuel: texte_decimal(residuel.residuel()),
        statut: nom_statut(residuel.statut()),
        situation,
        retard_jours,
        revisions: revisions
            .iter()
            .filter(|r| r.position == ligne.position)
            .map(|r| VueRevision {
                rang: r.rang,
                nouvelle_echeance: interne::date(r.nouvelle_echeance),
                date_operation: interne::date(r.date_operation),
                motif: r.motif.clone(),
                auteur: r.auteur.to_string(),
                cree_le: interne::instant(r.cree_le),
            })
            .collect(),
    }
}

/// Erreur d'une révision refusée par `compta-core`.
fn erreur_revision(e: &ErreurEcheance) -> ErreurApi {
    match e {
        ErreurEcheance::EcheanceSoldee => ErreurApi::Conflit {
            motif: MotifConflit::EcheanceSoldee,
            nombre: None,
        },
        // Une révision plus récente que le jour de l'opération : révision concurrente (ou fuseau
        // du dossier changé à minuit) ; relire puis réessayer.
        ErreurEcheance::RevisionHorsOrdre(_) => ErreurApi::Conflit {
            motif: MotifConflit::EcheanceRevisionConcurrente,
            nombre: None,
        },
        _ => ErreurApi::Interne(CategorieErreur::Donnee),
    }
}

fn corps<T: serde::de::DeserializeOwned>(octets: &[u8]) -> Result<T, ErreurApi> {
    Json::<T>::from_bytes(octets)
        .map(|Json(t)| t)
        .map_err(|_| ErreurApi::RequeteInvalide)
}

/// Paramètres d'URL nommés, chacun au plus une fois ; tout autre : 400.
fn parametres<'a>(
    texte: Option<&'a str>,
    admis: &[&str],
) -> Result<Vec<(&'a str, &'a str)>, ErreurApi> {
    let mut lus: Vec<(&str, &str)> = Vec::new();
    for paire in texte.unwrap_or("").split('&').filter(|p| !p.is_empty()) {
        let (cle, valeur) = paire.split_once('=').ok_or(ErreurApi::RequeteInvalide)?;
        if !admis.contains(&cle) || lus.iter().any(|(c, _)| *c == cle) {
            return Err(ErreurApi::RequeteInvalide);
        }
        lus.push((cle, valeur));
    }
    Ok(lus)
}

fn parametre<'a>(lus: &[(&str, &'a str)], cle: &str) -> Option<&'a str> {
    lus.iter().find(|(c, _)| *c == cle).map(|(_, v)| *v)
}

// --- Exécution ---------------------------------------------------------------------------------

/// Exécute un cas d'usage du module dans la transaction du pipeline.
pub(crate) async fn executer(
    cas: Cas,
    tx: &mut TransactionTenant,
    garde: &Garde,
    requete: &Requete<'_>,
) -> Result<Resultat, ErreurApi> {
    let dossier = dossier_visible(tx, garde).await?;
    match cas {
        Cas::ListerConditions => {
            exiger_role(tx, ROLES_LECTURE).await?;
            parametres(requete.parametres, &[])?;
            let conditions = base::lister_conditions(tx).await?;
            Ok(Resultat::lecture(
                Json(ListeConditions {
                    conditions: conditions.iter().map(vue_condition).collect(),
                })
                .into_response(),
            ))
        }
        Cas::LireCondition => {
            exiger_role(tx, ROLES_LECTURE).await?;
            parametres(requete.parametres, &[])?;
            let c = condition_du_chemin(tx, requete).await?;
            Ok(Resultat::lecture(Json(vue_condition(&c)).into_response()))
        }
        Cas::CreerCondition => {
            exiger_role(tx, ROLES_PARAMETRAGE).await?;
            creer_condition(tx, &dossier, requete).await
        }
        Cas::ModifierCondition => {
            exiger_role(tx, ROLES_PARAMETRAGE).await?;
            modifier_condition(tx, &dossier, requete).await
        }
        Cas::CalculerEcheances => {
            exiger_role(tx, ROLES_SAISIE).await?;
            calculer(tx, &dossier, requete).await
        }
        Cas::LireEcheances => {
            exiger_role(tx, ROLES_LECTURE).await?;
            lire_echeances(tx, &dossier, requete).await
        }
        Cas::ReviserEcheance => {
            exiger_role(tx, ROLES_SAISIE).await?;
            reviser(tx, &dossier, requete).await
        }
    }
}

/// Le dossier du chemin, **visible** par l'utilisateur (relu en base) et décidé par le pipeline.
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

async fn condition_du_chemin(
    tx: &mut TransactionTenant,
    requete: &Requete<'_>,
) -> Result<ConditionLue, ErreurApi> {
    let id = id_chemin(requete.chemin, "condition")?;
    base::lire_condition(tx, id)
        .await?
        .ok_or(ErreurApi::Introuvable)
}

/// Libellé d'une condition : 1 à [`LIBELLE_MAX`] caractères, sans caractère de contrôle ni de
/// format.
fn libelle(texte: &str) -> Result<String, ErreurApi> {
    if texte_borne(texte, LIBELLE_MAX) {
        Ok(texte.to_owned())
    } else {
        Err(invalide())
    }
}

/// Termes demandés, contrôlés par `compta-core`.
fn termes(demandes: &[CorpsTerme]) -> Result<Vec<TermeStocke>, ErreurApi> {
    if demandes.len() > TERMES_MAX {
        return Err(invalide());
    }
    let termes = demandes
        .iter()
        .map(CorpsTerme::stocke)
        .collect::<Result<Vec<_>, _>>()?;
    condition_pure(&termes).map_err(|_| invalide())?;
    Ok(termes)
}

fn trace_condition(
    dossier: &DossierAutorise,
    c: &ConditionLue,
    operation: &str,
    champs: &[&str],
) -> Trace {
    let mut t = Trace::new(
        PorteeAudit::Dossier(dossier.id()),
        "condition_reglement",
        c.id,
    );
    t.donnees.push(("operation", operation.to_owned()));
    t.donnees.push(("code", c.code.clone()));
    t.donnees.push(("champs", champs.join(",")));
    t.donnees.push(("termes", c.termes.len().to_string()));
    t
}

async fn creer_condition(
    tx: &mut TransactionTenant,
    dossier: &DossierAutorise,
    requete: &Requete<'_>,
) -> Result<Resultat, ErreurApi> {
    let c: CorpsCreation = corps(requete.corps)?;
    let code_valide = (1..=20).contains(&c.code.len())
        && c.code
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b"._-".contains(&b));
    if !code_valide {
        return Err(invalide());
    }
    let condition = ConditionLue {
        id: Uuid::new_v4(),
        code: c.code,
        libelle: libelle(&c.libelle)?,
        actif: true,
        termes: termes(&c.termes)?,
    };
    if !base::creer_condition(tx, &condition, requete.horloge.maintenant()).await? {
        return Err(ErreurApi::Conflit {
            motif: MotifConflit::ConditionReglementCodeDejaUtilise,
            nombre: None,
        });
    }
    let trace = trace_condition(dossier, &condition, "creation", &["libelle", "termes"]);
    Ok(Resultat {
        reponse: (StatusCode::CREATED, Json(vue_condition(&condition))).into_response(),
        traces: vec![trace],
    })
}

async fn modifier_condition(
    tx: &mut TransactionTenant,
    dossier: &DossierAutorise,
    requete: &Requete<'_>,
) -> Result<Resultat, ErreurApi> {
    let c: CorpsModification = corps(requete.corps)?;
    if c.libelle.is_none() && c.actif.is_none() && c.termes.is_none() {
        return Err(ErreurApi::RequeteInvalide);
    }
    let libelle_demande = c.libelle.as_deref().map(libelle).transpose()?;
    let termes_demandes = c.termes.as_deref().map(termes).transpose()?;
    let id = id_chemin(requete.chemin, "condition")?;
    if !base::verrouiller_condition(tx, id).await? {
        return Err(ErreurApi::Introuvable);
    }
    let avant = base::lire_condition(tx, id)
        .await?
        .ok_or(ErreurApi::Introuvable)?;
    let mut apres = avant.clone();
    if let Some(l) = libelle_demande {
        apres.libelle = l;
    }
    if let Some(a) = c.actif {
        apres.actif = a;
    }
    if let Some(t) = termes_demandes {
        apres.termes = t;
    }
    let champs: Vec<&str> = [
        ("libelle", apres.libelle != avant.libelle),
        ("actif", apres.actif != avant.actif),
        ("termes", apres.termes != avant.termes),
    ]
    .into_iter()
    .filter_map(|(nom, change)| change.then_some(nom))
    .collect();
    if champs.is_empty() {
        return Err(ErreurApi::Conflit {
            motif: MotifConflit::AucunChangement,
            nombre: None,
        });
    }
    base::modifier_condition(tx, &apres, apres.termes != avant.termes).await?;
    let trace = trace_condition(dossier, &apres, "modification", &champs);
    Ok(Resultat {
        reponse: Json(vue_condition(&apres)).into_response(),
        traces: vec![trace],
    })
}

/// Total à répartir : montant de la ligne de tiers dans la devise de tenue, signé (un avoir est
/// négatif), à l'échelle de la devise (comme la saisie : 422 `montant_echelle_devise_tenue`).
fn montant_demande(texte: &str, tenue: DeviseTenue) -> Result<Montant, ErreurApi> {
    Montant::parse_tenue(texte, &tenue).map_err(|e| match e {
        ErreurMontant::EchelleExcessive { .. } => ErreurApi::Saisie {
            refus: RefusSaisie::MontantEchelleTenue,
            ligne: None,
        },
        _ => ErreurApi::RequeteInvalide,
    })
}

/// Assistant à la saisie : termes de la condition pour une date de référence (la date comptable
/// en L3, ADR 0033 point 3) et un total ; aucune écriture.
async fn calculer(
    tx: &mut TransactionTenant,
    dossier: &DossierAutorise,
    requete: &Requete<'_>,
) -> Result<Resultat, ErreurApi> {
    // La condition d'abord : celle d'un autre dossier est absente (404), quels que soient les
    // paramètres.
    let c = condition_du_chemin(tx, requete).await?;
    let condition = condition_relue(&c)?;
    let lus = parametres(requete.parametres, &["date", "montant"])?;
    let date = parametre(&lus, "date")
        .and_then(date_lue)
        .ok_or(ErreurApi::RequeteInvalide)?;
    let tenue = dossier.contexte().tenue()?;
    let total = montant_demande(
        parametre(&lus, "montant").ok_or(ErreurApi::RequeteInvalide)?,
        tenue,
    )?;
    let calcules = condition
        .repartir(total, date, &tenue)
        .map_err(|e| match e {
            // Date d'échéance hors du calendrier : référence ou délai démesurés.
            ErreurEcheance::DateHorsLimites | ErreurEcheance::Depassement => {
                ErreurApi::RequeteInvalide
            }
            _ => ErreurApi::Interne(CategorieErreur::Donnee),
        })?;
    Ok(Resultat::lecture(
        Json(VueCalcul {
            id: c.id.to_string(),
            date: interne::date(date),
            montant: texte_decimal(total.valeur()),
            termes: calcules
                .iter()
                .map(|t| VueTermeCalcule {
                    date: interne::date(t.date()),
                    montant: texte_decimal(t.montant().valeur()),
                })
                .collect(),
        })
        .into_response(),
    ))
}

/// Écriture validée du chemin, relue dans le dossier du contexte (absente : 404).
async fn ecriture_du_chemin(
    tx: &mut TransactionTenant,
    requete: &Requete<'_>,
) -> Result<(Uuid, EcritureEcheances), ErreurApi> {
    let id = id_chemin(requete.chemin, "ecriture")?;
    let ecriture = base::lire_ecriture(tx, id)
        .await?
        .ok_or(ErreurApi::Introuvable)?;
    Ok((id, ecriture))
}

/// Résiduel d'une ligne, lu par l'interface du lettrage ([`base::residuel`]).
async fn residuel_de(
    tx: &mut TransactionTenant,
    ecriture: Uuid,
    ligne: &LigneTiersValidee,
) -> Result<Residuel, ErreurApi> {
    let residuel = base::residuel(tx, ecriture, ligne.position, ligne.montant).await?;
    Residuel::nouveau(ligne.montant.abs(), residuel)
        .map_err(|_| ErreurApi::Interne(CategorieErreur::Donnee))
}

async fn lire_echeances(
    tx: &mut TransactionTenant,
    dossier: &DossierAutorise,
    requete: &Requete<'_>,
) -> Result<Resultat, ErreurApi> {
    let lus = parametres(requete.parametres, &["au"])?;
    let au = match parametre(&lus, "au") {
        Some(texte) => date_lue(texte).ok_or(ErreurApi::RequeteInvalide)?,
        None => dossier.contexte().date_du_jour(requete.maintenant)?,
    };
    let (id, ecriture) = ecriture_du_chemin(tx, requete).await?;
    let mut lignes = Vec::with_capacity(ecriture.lignes.len());
    for ligne in &ecriture.lignes {
        let echeance = echeance_de(ecriture.date_comptable, ligne, &ecriture.revisions)?;
        let residuel = residuel_de(tx, id, ligne).await?;
        lignes.push(vue_ligne(
            au,
            ligne,
            &echeance,
            &residuel,
            &ecriture.revisions,
        ));
    }
    Ok(Resultat::lecture(
        Json(VueEcheances {
            id: id.to_string(),
            date_comptable: interne::date(ecriture.date_comptable),
            au: interne::date(au),
            lignes,
        })
        .into_response(),
    ))
}

/// Révision d'échéance (ADR 0033 point 2) : ajout seul, refusée quand le résiduel est nul.
async fn reviser(
    tx: &mut TransactionTenant,
    dossier: &DossierAutorise,
    requete: &Requete<'_>,
) -> Result<Resultat, ErreurApi> {
    let c: CorpsRevision = corps(requete.corps)?;
    let nouvelle = date_lue(&c.nouvelle_echeance).ok_or(ErreurApi::RequeteInvalide)?;
    let motif = MotifRevision::parse(&c.motif).map_err(|_| ErreurApi::Validation {
        motif: MotifValidation::EcheanceMotifInvalide,
        ligne: None,
    })?;
    let (id, ecriture) = ecriture_du_chemin(tx, requete).await?;
    // Une échéance est portée par une ligne de tiers (ADR 0033 point 1) : une ligne sans tiers,
    // ou absente de l'écriture, n'en a pas.
    let ligne = ecriture
        .lignes
        .iter()
        .find(|l| l.position == c.position)
        .ok_or_else(|| ErreurApi::Saisie {
            refus: RefusSaisie::EcheanceSansTiers,
            ligne: u32::try_from(c.position).ok(),
        })?;
    // Résiduel relu par l'interface du lettrage, qui prendra le verrou du compte (PR 9a).
    let residuel = residuel_de(tx, id, ligne).await?;
    let mut echeance = echeance_de(ecriture.date_comptable, ligne, &ecriture.revisions)?;
    let instant = requete.horloge.maintenant();
    let jour = dossier.contexte().date_du_jour(instant)?;
    let revision = echeance
        .reviser(nouvelle, jour, motif, &residuel)
        .map_err(|e| erreur_revision(&e))?
        .clone();
    let nouvelle_revision = NouvelleRevision {
        ecriture: id,
        position: ligne.position,
        rang: i32::try_from(revision.rang())
            .map_err(|_| ErreurApi::Interne(CategorieErreur::Donnee))?,
        nouvelle_echeance: revision.nouvelle_echeance(),
        date_operation: revision.date_operation(),
        motif: revision.motif().as_str().to_owned(),
        instant,
    };
    if !base::ajouter_revision(tx, &nouvelle_revision).await? {
        return Err(ErreurApi::Conflit {
            motif: MotifConflit::EcheanceRevisionConcurrente,
            nombre: None,
        });
    }
    let mut trace = Trace::new(PorteeAudit::Dossier(dossier.id()), "echeance", id);
    trace.donnees.push(("operation", "revision".to_owned()));
    trace.donnees.push(("position", ligne.position.to_string()));
    trace
        .donnees
        .push(("rang", nouvelle_revision.rang.to_string()));
    trace.donnees.push((
        "nouvelle_echeance",
        interne::date(nouvelle_revision.nouvelle_echeance),
    ));
    trace.donnees.push((
        "date_operation",
        interne::date(nouvelle_revision.date_operation),
    ));
    // Vue de la ligne après la révision, à la date du jour du dossier.
    let mut revisions = ecriture.revisions.clone();
    revisions.push(RevisionLue {
        position: ligne.position,
        rang: nouvelle_revision.rang,
        nouvelle_echeance: nouvelle_revision.nouvelle_echeance,
        date_operation: nouvelle_revision.date_operation,
        motif: nouvelle_revision.motif.clone(),
        auteur: tx.contexte().id_utilisateur().uuid(),
        cree_le: instant,
    });
    let vue = VueEcheances {
        id: id.to_string(),
        date_comptable: interne::date(ecriture.date_comptable),
        au: interne::date(jour),
        lignes: vec![vue_ligne(jour, ligne, &echeance, &residuel, &revisions)],
    };
    Ok(Resultat {
        reponse: (StatusCode::CREATED, Json(vue)).into_response(),
        traces: vec![trace],
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::date;

    fn tenue() -> DeviseTenue {
        DeviseTenue::nouvelle(compta_core::Devise::parse("XOF").unwrap(), 0).unwrap()
    }

    fn ligne(montant: &str, initiale: Option<Date>) -> LigneTiersValidee {
        LigneTiersValidee {
            position: 1,
            compte: "4111".to_owned(),
            sens: "debit".to_owned(),
            montant: Decimal::from_str_exact(montant).unwrap(),
            tiers: Uuid::nil(),
            initiale,
        }
    }

    #[test]
    fn les_modeles_d_usage_sont_des_conditions_valides_aux_codes_uniques() {
        let mut codes = std::collections::BTreeSet::new();
        for (code, libelle, termes) in MODELES_D_USAGE {
            assert!(codes.insert(*code), "{code} en double");
            assert!(texte_borne(libelle, LIBELLE_MAX), "{libelle}");
            assert!(
                code.len() <= 20
                    && code.bytes().all(|b| b.is_ascii_uppercase()
                        || b.is_ascii_digit()
                        || b"._-".contains(&b))
            );
            condition_pure(&termes_du_modele(termes)).unwrap();
        }
    }

    #[test]
    fn trente_jours_fin_de_mois_et_trente_soixante() {
        let modele = |code: &str| {
            let (_, _, termes) = MODELES_D_USAGE.iter().find(|(c, _, _)| *c == code).unwrap();
            condition_pure(&termes_du_modele(termes)).unwrap()
        };
        let xof = |v: &str| Montant::parse(v, compta_core::Devise::parse("XOF").unwrap()).unwrap();
        // 15 janvier + 30 jours = 14 février, puis fin du mois.
        let t = modele("30JFM")
            .repartir(xof("1000"), date!(2026 - 01 - 15), &tenue())
            .unwrap();
        assert_eq!(t.len(), 1);
        assert_eq!(t[0].date(), date!(2026 - 02 - 28));
        let t = modele("30C-60J")
            .repartir(xof("1001"), date!(2026 - 01 - 15), &tenue())
            .unwrap();
        assert_eq!(t.len(), 2);
        assert_eq!(t[0].date(), date!(2026 - 01 - 15));
        assert_eq!(texte_decimal(t[0].montant().valeur()), "300");
        assert_eq!(t[1].date(), date!(2026 - 03 - 16));
        assert_eq!(texte_decimal(t[1].montant().valeur()), "701");
    }

    #[test]
    fn termes_demandes_controles_par_compta_core() {
        let terme = |valeur: &str, montant: Option<&str>| CorpsTerme {
            valeur: valeur.to_owned(),
            montant: montant.map(str::to_owned),
            delai: "jours".to_owned(),
            jours: 30,
            jour: None,
        };
        assert!(termes(&[terme("solde", None)]).is_ok());
        assert!(termes(&[terme("pourcentage", Some("40")), terme("solde", None)]).is_ok());
        assert!(termes(&[terme("fixe", Some("1000.5")), terme("solde", None)]).is_ok());
        for refuse in [
            vec![],
            vec![terme("pourcentage", Some("40"))],
            vec![terme("solde", None), terme("solde", None)],
            vec![terme("pourcentage", Some("100")), terme("solde", None)],
            vec![terme("pourcentage", Some("-5")), terme("solde", None)],
            vec![terme("pourcentage", Some("1e2")), terme("solde", None)],
            vec![terme("fixe", Some("0")), terme("solde", None)],
            vec![terme("fixe", None), terme("solde", None)],
            vec![terme("solde", Some("5"))],
            vec![terme("autre", None)],
        ] {
            assert_eq!(termes(&refuse), Err(invalide()), "{refuse:?}");
        }
        let mut jour = terme("solde", None);
        jour.delai = "jour_du_mois_suivant".to_owned();
        assert_eq!(termes(&[jour.clone()]), Err(invalide()));
        jour.jour = Some(10);
        assert!(termes(&[jour.clone()]).is_ok());
        jour.jour = Some(32);
        assert_eq!(termes(&[jour]), Err(invalide()));
        let trop = vec![terme("fixe", Some("1")); TERMES_MAX];
        let mut trop = trop;
        trop.push(terme("solde", None));
        assert_eq!(termes(&trop), Err(invalide()));
    }

    #[test]
    fn statut_et_situation_calcules_du_residuel_fourni() {
        let l = ligne("1000", Some(date!(2026 - 03 - 31)));
        let e = echeance_de(date!(2026 - 03 - 01), &l, &[]).unwrap();
        let residuel =
            |r: &str| Residuel::nouveau(l.montant, Decimal::from_str_exact(r).unwrap()).unwrap();
        let ouverte = vue_ligne(date!(2026 - 03 - 31), &l, &e, &residuel("1000"), &[]);
        assert_eq!((ouverte.statut, ouverte.situation), ("ouverte", "a_echoir"));
        let echue = vue_ligne(date!(2026 - 04 - 10), &l, &e, &residuel("400"), &[]);
        assert_eq!(
            (echue.statut, echue.situation, echue.retard_jours),
            ("partiellement_reglee", "echue", Some(10))
        );
        let soldee = vue_ligne(date!(2026 - 04 - 10), &l, &e, &residuel("0"), &[]);
        assert_eq!((soldee.statut, soldee.situation), ("soldee", "soldee"));
        // Sans échéance initiale : repli sur la date comptable (Tryton `move.py:776`).
        let sans = ligne("1000", None);
        let e = echeance_de(date!(2026 - 03 - 01), &sans, &[]).unwrap();
        assert_eq!(
            vue_ligne(date!(2026 - 03 - 02), &sans, &e, &residuel("1000"), &[]).retard_jours,
            Some(1)
        );
    }

    #[test]
    fn revision_refusee_quand_le_residuel_est_nul() {
        let l = ligne("1000", Some(date!(2026 - 03 - 31)));
        let mut e = echeance_de(date!(2026 - 03 - 01), &l, &[]).unwrap();
        let motif = || MotifRevision::parse("Report accordé").unwrap();
        let nul = Residuel::nouveau(l.montant, Decimal::ZERO).unwrap();
        let err = e
            .reviser(date!(2026 - 04 - 30), date!(2026 - 03 - 20), motif(), &nul)
            .unwrap_err();
        assert_eq!(
            erreur_revision(&err),
            ErreurApi::Conflit {
                motif: MotifConflit::EcheanceSoldee,
                nombre: None
            }
        );
        let partiel = Residuel::nouveau(l.montant, Decimal::ONE).unwrap();
        let r = e
            .reviser(
                date!(2026 - 04 - 30),
                date!(2026 - 03 - 20),
                motif(),
                &partiel,
            )
            .unwrap()
            .clone();
        assert_eq!(r.rang(), 1);
        assert_eq!(e.effective(date!(2026 - 03 - 20)), date!(2026 - 04 - 30));
        // L'échéance effective à une date antérieure à la révision reste l'initiale.
        assert_eq!(e.effective(date!(2026 - 03 - 19)), date!(2026 - 03 - 31));
        let err = e
            .reviser(
                date!(2026 - 05 - 31),
                date!(2026 - 03 - 19),
                motif(),
                &partiel,
            )
            .unwrap_err();
        assert_eq!(
            erreur_revision(&err),
            ErreurApi::Conflit {
                motif: MotifConflit::EcheanceRevisionConcurrente,
                nombre: None
            }
        );
    }

    #[test]
    fn revisions_relues_par_rang_et_position() {
        let l = ligne("1000", None);
        let r = |position, rang, jour| RevisionLue {
            position,
            rang,
            nouvelle_echeance: jour,
            date_operation: date!(2026 - 03 - 10),
            motif: "Report".to_owned(),
            auteur: Uuid::nil(),
            cree_le: time::OffsetDateTime::UNIX_EPOCH,
        };
        let revisions = [
            r(1, 2, date!(2026 - 05 - 31)),
            r(2, 1, date!(2027 - 01 - 01)),
            r(1, 1, date!(2026 - 04 - 30)),
        ];
        let e = echeance_de(date!(2026 - 03 - 01), &l, &revisions).unwrap();
        assert_eq!(e.revisions().len(), 2);
        assert_eq!(e.effective(date!(2026 - 03 - 10)), date!(2026 - 05 - 31));
        // Une révision stockée illisible (motif hors grammaire) : donnée incohérente.
        let mut illisible = r(1, 1, date!(2026 - 04 - 30));
        illisible.motif = "\u{202E}".to_owned();
        assert_eq!(
            echeance_de(date!(2026 - 03 - 01), &l, &[illisible]).unwrap_err(),
            ErreurApi::Interne(CategorieErreur::Donnee)
        );
    }

    #[test]
    fn decimaux_stricts() {
        assert!(decimal_positif("30").is_some());
        assert!(decimal_positif("12.5").is_some());
        for refuse in ["", "0", "0.0", "-1", "+1", "1.", ".5", "1e2", "1 0", "１"] {
            assert!(decimal_positif(refuse).is_none(), "{refuse}");
        }
        assert!(decimal_positif(&"9".repeat(20)).is_some());
        assert!(decimal_positif(&"9".repeat(21)).is_none());
    }

    #[test]
    fn parametres_nommes_uniques() {
        assert!(parametres(Some("date=2026-01-01&montant=10"), &["date", "montant"]).is_ok());
        assert!(parametres(Some("date=1&date=2"), &["date"]).is_err());
        assert!(parametres(Some("autre=1"), &["date"]).is_err());
        assert!(parametres(Some("date"), &["date"]).is_err());
        assert_eq!(parametres(None, &[]).unwrap(), Vec::<(&str, &str)>::new());
    }
}
