//! Comptes de trésorerie, avertissements de solde, 585, arrêté de caisse (L3-21, PR 11 de
//! `docs/lots/L3.md` ; ADR 0034 points 1 à 3 ; spec §9.1, §9.3, §15.1).
//!
//! # Compte de trésorerie (ADR 0034 point 1)
//! Un compte de trésorerie = un compte du plan + un journal de trésorerie. **Création assistée**
//! (consensus Odoo, `account_journal.py:918-952`, qui crée le compte par défaut d'un journal de
//! banque ou de caisse au premier code libre sous un préfixe) : un sous-compte du **compte par
//! défaut du type** (marque `type_tresorerie` de `crates/syscohada/data/comptes.toml` : aucun
//! numéro dans le code, règle absolue 9), au premier segment libre `01` à `99` (ou celui fourni),
//! et un journal de trésorerie au premier code libre `BQ1`, `CA1`, `MM1`… (préfixes de la spec
//! §5.2 ; ou celui fourni), par la **seule voie de mutation du référentiel** (`muter` : verrou,
//! contrôles de `compta-core`, audit de chaque changement). Un journal de trésorerie existant (le
//! journal CA par défaut) peut aussi être repris tel quel. Compte, journal, type et devise ne
//! changent plus ; nom, plafond, comptes d'écart et archivage sont modifiables et journalisés.
//! En L3, la devise est celle de tenue (422 `tresorerie_devise_etrangere` sinon : L6).
//!
//! # Avertissements de solde (ADR 0034 point 2, décision D2)
//! **Jamais un refus** (aucun moteur ne bloque ; AUDCIF p. PDF 428 parle de « présomption
//! d'irrégularité ») : `caisse_solde_crediteur`, `portefeuille_solde_crediteur`,
//! `caisse_plafond_depasse` (plafond facultatif, aucun par défaut), calculés par
//! `compta_core::rapprochement::avertissements_solde` sur les écritures **validées**. Le plan L3
//! ne prévoit qu'une modification du chemin de validation (la copie de l'échéance, PR 8) : la
//! validation (`brouillard/validation.rs`) n'est **pas** modifiée ; les avertissements sont
//! **calculés à la demande** (liste, fiche du compte, arrêté de caisse), à une date facultative.
//! `virement_interne_non_solde` : compte marqué `virement_interne` (585) au solde non nul
//! (spec §9.3, §15.1), à la demande aussi ; l'alerte périodique est de la PR 15.
//!
//! # Arrêté de caisse (ADR 0034 point 3)
//! Le comptage est figé en **pièce interne** : une note d'opérations diverses de la forme figée
//! `piece-interne-v1` (nature `note-od`, format inchangé : le plan ne réserve à `docs/integrite/`
//! que `etat-rapprochement-v1`), qui cite le compte, le journal, le montant compté, le solde
//! comptable et l'écart. L'écart non nul est **proposé en brouillon** (journal de la caisse, mêmes
//! contrôles que la saisie, note en pièce principale) sur le compte d'écart **désigné** par le
//! dossier pour cette caisse (excédent ou manquant ; Odoo : comptes de gain et de perte de caisse
//! par journal, `account_journal.py:234-243`) ; sans compte désigné, l'arrêté est enregistré avec
//! l'avertissement `compte_ecart_non_designe`, sans brouillon. Rien n'est validé (règle
//! absolue 6).
//!
//! # Défense en profondeur (ADR 0011)
//! Même si Cedar se trompait : chaque cas relit le dossier **visible** (affecté, sauf
//! administrateur) et, pour une mutation, le rôle de l'appartenance active en base
//! (paramétrage : administrateur, valideur ; arrêté : plus collaborateur). Un compte de
//! trésorerie d'un autre dossier est absent (404), comme un identifiant inexistant.

use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

use authz::{Action, Role};
use axum::Json;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use compta_core::rapprochement::{self, AvertissementSolde};
use compta_core::{
    CodeJournal, Devise, DeviseTenue, Journal, Montant, NumeroCompte, TexteCourt, TypeJournal,
};
use serde::{Deserialize, Deserializer, Serialize};
use syscohada::TypeTresorerie;
use time::{Date, OffsetDateTime};
use uuid::Uuid;

use super::{ActionRoute, CasUsage, Methode, Module, Portee, Requete, Resultat, Route};
use crate::base::TransactionTenant;
use crate::base::audit::PorteeAudit;
use crate::base::organisation::{lire_appartenance, lire_dossier_visible};
use crate::base::referentiel::{
    ErreurMutation, ReferentielDossier, RefusReferentiel, charger, muter, verrouiller,
};
use crate::base::tresorerie::{
    self as base, ArreteLu, CompteTresorerieLu, Configuration, NouveauCompteTresorerie,
    NouvelArrete,
};
use crate::pieces::interne::{self, NoteOd};
use crate::serveur::audit::Trace;
use crate::serveur::erreur::{CategorieErreur, ErreurApi, MotifConflit, MotifValidation};
use crate::serveur::pipeline::{DossierAutorise, Garde, id_chemin};

/// Cas d'usage du module.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cas {
    /// Comptes de trésorerie du dossier, avec solde et avertissements.
    Lister,
    /// Créer un compte de trésorerie (création assistée, ou reprise d'un journal existant).
    Creer,
    /// Un compte de trésorerie, avec solde et avertissements.
    Lire,
    /// Modifier nom, plafond, comptes d'écart, archivage.
    Modifier,
    /// Soldes des comptes de virements internes (585) et avertissements.
    VirementsInternes,
    /// Arrêté de caisse : comptage figé en pièce interne, écart proposé en brouillon.
    ArreterCaisse,
    /// Arrêtés d'une caisse.
    ListerArretes,
}

/// Tranche du module.
pub const MODULE: Module = Module {
    nom: "tresorerie",
    routes: ROUTES,
    champs_journal: &[],
};

const fn route(methode: Methode, chemin: &'static str, action: Action, cas: Cas) -> Route {
    Route {
        methode,
        chemin,
        action: ActionRoute::Cabinet(action),
        portee: Portee::Dossier,
        cas_usage: CasUsage::Tresorerie(cas),
    }
}

/// Routes du module. Lecture : `ConsulterTresorerie` (quatre rôles, plan C6) ; comptes :
/// `ParametrerTresorerie` (administrateur, valideur) ; arrêté : `ArreterCaisse` (plus
/// collaborateur).
pub const ROUTES: &[Route] = &[
    route(
        Methode::Get,
        "/api/cabinets/{cabinet}/dossiers/{dossier}/tresorerie/comptes",
        Action::ConsulterTresorerie,
        Cas::Lister,
    ),
    route(
        Methode::Post,
        "/api/cabinets/{cabinet}/dossiers/{dossier}/tresorerie/comptes",
        Action::ParametrerTresorerie,
        Cas::Creer,
    ),
    route(
        Methode::Get,
        "/api/cabinets/{cabinet}/dossiers/{dossier}/tresorerie/comptes/{compte_tresorerie}",
        Action::ConsulterTresorerie,
        Cas::Lire,
    ),
    route(
        Methode::Post,
        "/api/cabinets/{cabinet}/dossiers/{dossier}/tresorerie/comptes/{compte_tresorerie}/modification",
        Action::ParametrerTresorerie,
        Cas::Modifier,
    ),
    route(
        Methode::Get,
        "/api/cabinets/{cabinet}/dossiers/{dossier}/tresorerie/virements-internes",
        Action::ConsulterTresorerie,
        Cas::VirementsInternes,
    ),
    route(
        Methode::Post,
        "/api/cabinets/{cabinet}/dossiers/{dossier}/tresorerie/comptes/{compte_tresorerie}/arretes",
        Action::ArreterCaisse,
        Cas::ArreterCaisse,
    ),
    route(
        Methode::Get,
        "/api/cabinets/{cabinet}/dossiers/{dossier}/tresorerie/comptes/{compte_tresorerie}/arretes",
        Action::ConsulterTresorerie,
        Cas::ListerArretes,
    ),
];

/// Nombre d'arrêtés rendus par la liste d'une caisse (les plus récents).
const ARRETES_MAX: i64 = 100;
/// Lignes de détail du comptage, au plus (la note en admet 200, dont celles du serveur).
const DETAIL_MAX: usize = 190;
/// Dernier rang de la numérotation assistée (segment `01` à `99`, code `BQ1` à `BQ99`).
const RANG_MAX: u8 = 99;

/// Exécute un cas d'usage du module dans la transaction du pipeline.
pub(crate) async fn executer(
    cas: Cas,
    tx: &mut TransactionTenant,
    garde: &Garde,
    requete: &Requete<'_>,
) -> Result<Resultat, ErreurApi> {
    let dossier = dossier_visible(tx, garde).await?;
    match cas {
        Cas::Lister => lister(tx, &dossier, requete).await,
        Cas::Lire => lire(tx, &dossier, requete).await,
        Cas::VirementsInternes => virements_internes(tx, requete).await,
        Cas::ListerArretes => lister_arretes(tx, requete).await,
        Cas::Creer => {
            exiger_role(tx, &[Role::Administrateur, Role::Valideur]).await?;
            creer(tx, &dossier, requete).await
        }
        Cas::Modifier => {
            exiger_role(tx, &[Role::Administrateur, Role::Valideur]).await?;
            modifier(tx, &dossier, requete).await
        }
        Cas::ArreterCaisse => {
            exiger_role(
                tx,
                &[Role::Administrateur, Role::Valideur, Role::Collaborateur],
            )
            .await?;
            arreter(tx, &dossier, requete).await
        }
    }
}

// --- Socle ------------------------------------------------------------------------------------

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

/// Appartenance **active** lue en base, d'un des rôles admis : sinon 404 (absente ou suspendue)
/// ou 403.
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

fn donnee() -> ErreurApi {
    ErreurApi::Interne(CategorieErreur::Donnee)
}

fn validation(motif: MotifValidation) -> ErreurApi {
    ErreurApi::Validation { motif, ligne: None }
}

fn conflit(motif: MotifConflit) -> ErreurApi {
    ErreurApi::Conflit {
        motif,
        nombre: None,
    }
}

fn corps<T: serde::de::DeserializeOwned>(octets: &[u8]) -> Result<T, ErreurApi> {
    Json::<T>::from_bytes(octets)
        .map(|Json(t)| t)
        .map_err(|_| ErreurApi::RequeteInvalide)
}

/// Référentiel officiel embarqué (`syscohada`), chargé une fois : comptes par défaut des types
/// et marques `virement_interne`.
fn officiel() -> Result<&'static syscohada::Referentiel, ErreurApi> {
    static REFERENTIEL: OnceLock<Option<syscohada::Referentiel>> = OnceLock::new();
    REFERENTIEL
        .get_or_init(|| syscohada::Referentiel::officiel().ok())
        .as_ref()
        .ok_or_else(donnee)
}

/// Type de `compta-core` d'un type de trésorerie.
const fn type_pur(t: TypeTresorerie) -> rapprochement::TypeTresorerie {
    match t {
        TypeTresorerie::Banque => rapprochement::TypeTresorerie::Banque,
        TypeTresorerie::Caisse => rapprochement::TypeTresorerie::Caisse,
        TypeTresorerie::Portefeuille => rapprochement::TypeTresorerie::Portefeuille,
    }
}

/// Préfixe du code du journal d'un type (spec §5.2 : `BQx`, `MMx`, `CA`).
const fn prefixe_journal(t: TypeTresorerie) -> &'static str {
    match t {
        TypeTresorerie::Banque => "BQ",
        TypeTresorerie::Caisse => "CA",
        TypeTresorerie::Portefeuille => "MM",
    }
}

/// Seul paramètre admis des lectures : `au=AAAA-MM-JJ` (date du solde ; toutes les écritures
/// validées sans elle). Tout autre paramètre, doublon ou date invalide : 400.
fn date_du_solde(parametres: Option<&str>) -> Result<Option<Date>, ErreurApi> {
    let mut au = None;
    for paire in parametres
        .unwrap_or("")
        .split('&')
        .filter(|p| !p.is_empty())
    {
        match paire.split_once('=') {
            Some(("au", valeur)) if au.is_none() => {
                au = Some(interne::lire_date(valeur).map_err(|_| ErreurApi::RequeteInvalide)?);
            }
            _ => return Err(ErreurApi::RequeteInvalide),
        }
    }
    Ok(au)
}

fn sans_parametres(requete: &Requete<'_>) -> Result<(), ErreurApi> {
    match requete.parametres {
        None | Some("") => Ok(()),
        Some(_) => Err(ErreurApi::RequeteInvalide),
    }
}

/// Texte décimal exact d'un montant, sans zéro final.
fn texte(m: &Montant) -> String {
    m.valeur().normalize().to_string()
}

/// Solde d'un compte (écritures validées, au plus `au`), zéro sans mouvement.
async fn solde(
    tx: &mut TransactionTenant,
    numero: &str,
    devise: Devise,
    au: Option<Date>,
) -> Result<Montant, ErreurApi> {
    let lus = base::soldes(tx, &[numero.to_owned()], au).await?;
    match lus.first() {
        None => Ok(Montant::zero(devise)),
        Some((_, s)) => Montant::parse(s, devise).map_err(|_| donnee()),
    }
}

// --- Vues -------------------------------------------------------------------------------------

#[derive(Debug, Serialize)]
struct VueReference {
    id: String,
    numero: String,
}

#[derive(Debug, Serialize)]
struct VueJournalLie {
    id: String,
    code: String,
}

/// Avertissement (jamais un refus) : code stable, solde et plafond s'il y a lieu.
#[derive(Debug, Serialize)]
struct VueAvertissement {
    code: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    compte: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    solde: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    plafond: Option<String>,
}

fn vue_avertissement(a: &AvertissementSolde) -> VueAvertissement {
    let (solde, plafond) = match a {
        AvertissementSolde::CaisseSoldeCrediteur { solde }
        | AvertissementSolde::PortefeuilleSoldeCrediteur { solde } => (*solde, None),
        AvertissementSolde::CaissePlafondDepasse { solde, plafond } => (*solde, Some(*plafond)),
    };
    VueAvertissement {
        code: a.code(),
        compte: None,
        solde: Some(solde.normalize().to_string()),
        plafond: plafond.map(|p| p.normalize().to_string()),
    }
}

#[derive(Debug, Serialize)]
struct VueCompteTresorerie {
    id: String,
    #[serde(rename = "type")]
    type_tresorerie: String,
    nom: String,
    compte: VueReference,
    journal: VueJournalLie,
    devise: String,
    plafond: Option<String>,
    compte_excedent: Option<VueReference>,
    compte_manquant: Option<VueReference>,
    archive: bool,
    /// Solde des écritures validées (débit positif), à la date `au` si elle est donnée.
    solde: String,
    au: Option<String>,
    avertissements: Vec<VueAvertissement>,
}

fn reference((id, numero): &(Uuid, String)) -> VueReference {
    VueReference {
        id: id.to_string(),
        numero: numero.clone(),
    }
}

/// Fiche d'un compte, avec son solde et ses avertissements à la date `au`.
async fn vue_compte(
    tx: &mut TransactionTenant,
    c: &CompteTresorerieLu,
    au: Option<Date>,
) -> Result<VueCompteTresorerie, ErreurApi> {
    let devise = Devise::parse(&c.devise).map_err(|_| donnee())?;
    let type_tresorerie = TypeTresorerie::depuis_code(&c.type_tresorerie).ok_or_else(donnee)?;
    let solde = solde(tx, &c.compte, devise, au).await?;
    let plafond = c
        .plafond
        .as_deref()
        .map(|p| Montant::parse(p, devise).map(|m| m.valeur()))
        .transpose()
        .map_err(|_| donnee())?;
    let avertissements =
        rapprochement::avertissements_solde(type_pur(type_tresorerie), solde.valeur(), plafond)
            .iter()
            .map(vue_avertissement)
            .collect();
    Ok(VueCompteTresorerie {
        id: c.id.to_string(),
        type_tresorerie: c.type_tresorerie.clone(),
        nom: c.nom.clone(),
        compte: reference(&(c.compte_id, c.compte.clone())),
        journal: VueJournalLie {
            id: c.journal_id.to_string(),
            code: c.journal.clone(),
        },
        devise: c.devise.clone(),
        plafond: c.plafond.clone(),
        compte_excedent: c.compte_excedent.as_ref().map(reference),
        compte_manquant: c.compte_manquant.as_ref().map(reference),
        archive: c.archive,
        solde: texte(&solde),
        au: au.map(interne::date),
        avertissements,
    })
}

#[derive(Debug, Serialize)]
struct ListeComptes {
    comptes: Vec<VueCompteTresorerie>,
}

// --- Lectures ---------------------------------------------------------------------------------

async fn lister(
    tx: &mut TransactionTenant,
    _dossier: &DossierAutorise,
    requete: &Requete<'_>,
) -> Result<Resultat, ErreurApi> {
    let au = date_du_solde(requete.parametres)?;
    let mut comptes = Vec::new();
    for c in base::lister(tx).await? {
        comptes.push(vue_compte(tx, &c, au).await?);
    }
    Ok(Resultat::lecture(
        Json(ListeComptes { comptes }).into_response(),
    ))
}

/// Compte de trésorerie du chemin (`{compte_tresorerie}`), dans le dossier du contexte : celui
/// d'un autre dossier est absent (404).
async fn compte_du_chemin(
    tx: &mut TransactionTenant,
    requete: &Requete<'_>,
    verrou: bool,
) -> Result<CompteTresorerieLu, ErreurApi> {
    let id = id_chemin(requete.chemin, "compte_tresorerie")?;
    base::lire(tx, id, verrou)
        .await?
        .ok_or(ErreurApi::Introuvable)
}

async fn lire(
    tx: &mut TransactionTenant,
    _dossier: &DossierAutorise,
    requete: &Requete<'_>,
) -> Result<Resultat, ErreurApi> {
    let au = date_du_solde(requete.parametres)?;
    let c = compte_du_chemin(tx, requete, false).await?;
    let vue = vue_compte(tx, &c, au).await?;
    Ok(Resultat::lecture(Json(vue).into_response()))
}

#[derive(Debug, Serialize)]
struct VueVirement {
    numero: String,
    intitule: String,
    solde: String,
}

#[derive(Debug, Serialize)]
struct ListeVirements {
    au: Option<String>,
    comptes: Vec<VueVirement>,
    avertissements: Vec<VueAvertissement>,
}

/// Comptes de virements internes du plan du dossier (marque `virement_interne`, héritée par les
/// sous-comptes) et leur solde ; `virement_interne_non_solde` pour chaque solde non nul (spec
/// §9.3 : « alerte si 585 ne revient pas à zéro » ; jamais un refus).
async fn virements_internes(
    tx: &mut TransactionTenant,
    requete: &Requete<'_>,
) -> Result<Resultat, ErreurApi> {
    let au = date_du_solde(requete.parametres)?;
    let officiel = officiel()?;
    let referentiel = charger(tx).await?.ok_or(ErreurApi::Introuvable)?;
    let marques: Vec<(String, String)> = referentiel
        .etat()
        .plan
        .comptes()
        .filter(|c| officiel.est_virement_interne(c.numero()))
        .map(|c| (c.numero().to_string(), c.intitule().to_string()))
        .collect();
    let numeros: Vec<String> = marques.iter().map(|(n, _)| n.clone()).collect();
    let soldes = base::soldes(tx, &numeros, au).await?;
    let mut comptes = Vec::with_capacity(marques.len());
    let mut avertissements = Vec::new();
    for (numero, intitule) in marques {
        let solde = soldes
            .iter()
            .find(|(n, _)| *n == numero)
            .map_or_else(|| "0".to_owned(), |(_, s)| s.clone());
        let normalise = crate::base::brouillard::decimal_canonique(&solde);
        if normalise != "0" && normalise != "-0" {
            avertissements.push(VueAvertissement {
                code: "virement_interne_non_solde",
                compte: Some(numero.clone()),
                solde: Some(normalise.clone()),
                plafond: None,
            });
        }
        comptes.push(VueVirement {
            numero,
            intitule,
            solde: normalise,
        });
    }
    Ok(Resultat::lecture(
        Json(ListeVirements {
            au: au.map(interne::date),
            comptes,
            avertissements,
        })
        .into_response(),
    ))
}

#[derive(Debug, Serialize)]
struct VueArrete {
    id: String,
    compte_tresorerie: String,
    date: String,
    montant_compte: String,
    solde_comptable: String,
    ecart: String,
    piece: String,
    brouillon: Option<String>,
    arrete_par: String,
    arrete_le: String,
}

fn vue_arrete(compte: Uuid, a: &ArreteLu) -> VueArrete {
    let canon = crate::base::brouillard::decimal_canonique;
    VueArrete {
        id: a.id.to_string(),
        compte_tresorerie: compte.to_string(),
        date: interne::date(a.date),
        montant_compte: canon(&a.montant_compte),
        solde_comptable: canon(&a.solde_comptable),
        ecart: canon(&a.ecart),
        piece: a.piece.to_string(),
        brouillon: a.brouillon.map(|b| b.to_string()),
        arrete_par: a.arrete_par.to_string(),
        arrete_le: interne::instant(a.arrete_le),
    }
}

#[derive(Debug, Serialize)]
struct ListeArretes {
    arretes: Vec<VueArrete>,
}

async fn lister_arretes(
    tx: &mut TransactionTenant,
    requete: &Requete<'_>,
) -> Result<Resultat, ErreurApi> {
    sans_parametres(requete)?;
    let c = compte_du_chemin(tx, requete, false).await?;
    let arretes = base::arretes(tx, c.id, ARRETES_MAX)
        .await?
        .iter()
        .map(|a| vue_arrete(c.id, a))
        .collect();
    Ok(Resultat::lecture(
        Json(ListeArretes { arretes }).into_response(),
    ))
}

// --- Création ---------------------------------------------------------------------------------

/// Corps de la création : `type` (`banque`, `caisse`, `portefeuille`) et `nom` (intitulé du
/// sous-compte et du journal) ; facultatifs : `segment` (`A-Z0-9`, 1 à 12) et `code_journal`
/// de la création assistée, **ou** `journal_existant` (code d'un journal de trésorerie à
/// reprendre), `plafond` (caisse), `devise` (celle de tenue en L3).
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CorpsCreation {
    #[serde(rename = "type")]
    type_tresorerie: String,
    nom: String,
    segment: Option<String>,
    code_journal: Option<String>,
    journal_existant: Option<String>,
    plafond: Option<String>,
    devise: Option<String>,
}

/// Plafond de caisse lu dans la devise de tenue : strictement positif, échelle de la devise.
fn plafond_lu(texte: &str, tenue: DeviseTenue) -> Result<Montant, ErreurApi> {
    let m = Montant::parse_tenue(texte, &tenue).map_err(|e| match e {
        compta_core::ErreurMontant::EchelleExcessive { .. } => {
            validation(MotifValidation::TresorerieParametreInvalide)
        }
        _ => ErreurApi::RequeteInvalide,
    })?;
    if m.est_strictement_positif() {
        Ok(m)
    } else {
        Err(validation(MotifValidation::TresorerieParametreInvalide))
    }
}

/// Refus d'une mutation du référentiel, en réponse : ressource inconnue 404, sinon 400 (comme
/// le module `referentiel`).
fn refus_mutation(e: ErreurMutation) -> ErreurApi {
    match e {
        ErreurMutation::Refus(RefusReferentiel::Inconnu) | ErreurMutation::Absent => {
            ErreurApi::Introuvable
        }
        ErreurMutation::Refus(RefusReferentiel::CompteTresorerieDejaUtilise(..)) => {
            conflit(MotifConflit::JournalDejaTresorerie)
        }
        ErreurMutation::Refus(_) => ErreurApi::RequeteInvalide,
        ErreurMutation::Base(e) => ErreurApi::from(e),
    }
}

/// Premier numéro libre `<défaut>-01` à `<défaut>-99` du plan.
fn segment_libre(
    etat: &crate::base::referentiel::EtatReferentiel,
    defaut: &NumeroCompte,
) -> Option<NumeroCompte> {
    (1..=RANG_MAX)
        .filter_map(|n| NumeroCompte::parse(&format!("{defaut}-{n:02}")).ok())
        .find(|n| etat.plan.compte(n).is_none())
}

/// Premier code libre `<préfixe>1` à `<préfixe>99` des journaux.
fn code_libre(
    etat: &crate::base::referentiel::EtatReferentiel,
    prefixe: &str,
) -> Option<CodeJournal> {
    (1..=RANG_MAX)
        .filter_map(|n| CodeJournal::parse(&format!("{prefixe}{n}")).ok())
        .find(|c| etat.journaux.journal(c).is_none())
}

#[allow(clippy::too_many_lines)]
async fn creer(
    tx: &mut TransactionTenant,
    dossier: &DossierAutorise,
    requete: &Requete<'_>,
) -> Result<Resultat, ErreurApi> {
    sans_parametres(requete)?;
    let c: CorpsCreation = corps(requete.corps)?;
    let type_tresorerie =
        TypeTresorerie::depuis_code(&c.type_tresorerie).ok_or(ErreurApi::RequeteInvalide)?;
    let nom = TexteCourt::parse(&c.nom).map_err(|_| ErreurApi::RequeteInvalide)?;
    let contexte = dossier.contexte();
    if c.devise.as_deref().is_some_and(|d| d != contexte.devise) {
        return Err(validation(MotifValidation::TresorerieDeviseEtrangere));
    }
    let tenue = contexte.tenue()?;
    let plafond = match c.plafond.as_deref() {
        None => None,
        Some(_) if type_tresorerie != TypeTresorerie::Caisse => {
            return Err(validation(MotifValidation::TresorerieParametreInvalide));
        }
        Some(p) => Some(texte(&plafond_lu(p, tenue)?)),
    };
    let mut traces = Vec::new();
    let (compte_id, journal_id) = if let Some(code) = &c.journal_existant {
        if c.segment.is_some() || c.code_journal.is_some() {
            return Err(ErreurApi::RequeteInvalide);
        }
        reprendre_journal(tx, code).await?
    } else {
        let (compte_id, journal_id, t) =
            creation_assistee(tx, dossier, type_tresorerie, &nom, &c).await?;
        traces.extend(t);
        (compte_id, journal_id)
    };
    let id = Uuid::new_v4();
    let type_code = type_tresorerie.code();
    base::inserer(
        tx,
        &NouveauCompteTresorerie {
            id,
            type_tresorerie: type_code,
            nom: nom.as_str(),
            compte_id,
            journal_id,
            devise: &contexte.devise,
            plafond: plafond.as_deref(),
            instant: requete.maintenant,
        },
    )
    .await?;
    let mut trace = Trace::new(PorteeAudit::Dossier(dossier.id()), "compte_tresorerie", id);
    trace.donnees.push(("compte", compte_id.to_string()));
    trace.donnees.push(("journal", journal_id.to_string()));
    let mut champ = |nom: &'static str, valeur: Option<String>| {
        if valeur.is_some() {
            trace.champs.push((nom, None, valeur));
        }
    };
    champ("type", Some(type_code.to_owned()));
    champ("nom", Some(nom.as_str().to_owned()));
    champ("devise", Some(contexte.devise.clone()));
    champ("plafond", plafond.clone());
    traces.push(trace);
    let lu = base::lire(tx, id, false).await?.ok_or_else(donnee)?;
    let vue = vue_compte(tx, &lu, None).await?;
    Ok(Resultat {
        reponse: (StatusCode::CREATED, Json(vue)).into_response(),
        traces,
    })
}

/// Reprise d'un journal de trésorerie existant (journal CA par défaut…) : son compte devient
/// celui du compte de trésorerie. Journal inconnu : 400 ; d'un autre type ou déjà lié : 409.
async fn reprendre_journal(
    tx: &mut TransactionTenant,
    code: &str,
) -> Result<(Uuid, Uuid), ErreurApi> {
    // Verrou du référentiel : les créations de comptes de trésorerie d'un dossier se suivent.
    if !verrouiller(tx).await? {
        return Err(ErreurApi::Introuvable);
    }
    let r = charger(tx).await?.ok_or(ErreurApi::Introuvable)?;
    let code = CodeJournal::parse(code).map_err(|_| ErreurApi::RequeteInvalide)?;
    let journal = r
        .etat()
        .journaux
        .journal(&code)
        .ok_or(ErreurApi::RequeteInvalide)?;
    let numero = journal
        .compte_tresorerie()
        .ok_or(conflit(MotifConflit::JournalDejaTresorerie))?;
    let journal_id = r.id_journal(&code).ok_or_else(donnee)?;
    let compte_id = r.id_compte(numero).ok_or_else(donnee)?;
    let deja = base::lister(tx)
        .await?
        .iter()
        .any(|t| t.journal_id == journal_id || t.compte_id == compte_id);
    if deja {
        return Err(conflit(MotifConflit::JournalDejaTresorerie));
    }
    Ok((compte_id, journal_id))
}

/// Création assistée : sous-compte du compte par défaut du type et journal de trésorerie, par
/// [`muter`] (verrou, contrôles de `compta-core`, une trace par création).
async fn creation_assistee(
    tx: &mut TransactionTenant,
    dossier: &DossierAutorise,
    type_tresorerie: TypeTresorerie,
    nom: &TexteCourt,
    c: &CorpsCreation,
) -> Result<(Uuid, Uuid, Vec<Trace>), ErreurApi> {
    let defaut = officiel()?
        .compte_par_defaut_tresorerie(type_tresorerie)
        .ok_or_else(donnee)?
        .clone();
    let segment = c
        .segment
        .as_deref()
        .map(|s| NumeroCompte::parse(&format!("{defaut}-{s}")))
        .transpose()
        .map_err(|_| ErreurApi::RequeteInvalide)?;
    let code = c
        .code_journal
        .as_deref()
        .map(CodeJournal::parse)
        .transpose()
        .map_err(|_| ErreurApi::RequeteInvalide)?;
    let epuise = AtomicBool::new(false);
    let nom = nom.clone();
    let mutation = muter(tx, |_: &ReferentielDossier, etat| {
        let numero = match segment {
            Some(n) => n,
            None => segment_libre(etat, &defaut).ok_or_else(|| {
                epuise.store(true, Ordering::Relaxed);
                RefusReferentiel::Valeur
            })?,
        };
        let code = match code {
            Some(c) => c,
            None => code_libre(etat, prefixe_journal(type_tresorerie)).ok_or_else(|| {
                epuise.store(true, Ordering::Relaxed);
                RefusReferentiel::Valeur
            })?,
        };
        etat.plan.ajouter_sous_compte(numero.clone(), nom.clone())?;
        let journal = Journal::nouveau(
            code.clone(),
            nom,
            TypeJournal::Tresorerie,
            Some(numero.clone()),
            &etat.plan,
        )?;
        etat.journaux.ajouter(journal)?;
        Ok((numero, code))
    })
    .await
    .map_err(|e| {
        if epuise.load(Ordering::Relaxed) {
            conflit(MotifConflit::NumerotationTresorerieEpuisee)
        } else {
            refus_mutation(e)
        }
    })?;
    let id_de = |genre: &str| {
        mutation
            .changements
            .iter()
            .find(|ch| ch.genre == genre && ch.cree)
            .map(|ch| ch.id)
            .ok_or_else(donnee)
    };
    let (compte_id, journal_id) = (id_de("compte")?, id_de("journal")?);
    let traces = mutation
        .changements
        .iter()
        .map(|ch| {
            let mut trace = Trace::new(PorteeAudit::Dossier(dossier.id()), ch.genre, ch.id);
            trace.donnees.clone_from(&ch.donnees);
            trace.champs.clone_from(&ch.champs);
            trace
        })
        .collect();
    Ok((compte_id, journal_id, traces))
}

// --- Modification -----------------------------------------------------------------------------

/// Changement demandé d'un champ effaçable : absent (rien ne change), `null` (effacé) ou valeur.
#[derive(Debug, Default)]
enum Changement<T> {
    #[default]
    Inchange,
    Efface,
    Valeur(T),
}

impl<T> Changement<T> {
    const fn est_inchange(&self) -> bool {
        matches!(self, Self::Inchange)
    }

    const fn est_valeur(&self) -> bool {
        matches!(self, Self::Valeur(_))
    }
}

/// Champ présent dans le corps : `null` efface, une valeur remplace (absent : [`Default`]).
fn changement<'de, D, T>(d: D) -> Result<Changement<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Ok(match Option::<T>::deserialize(d)? {
        None => Changement::Efface,
        Some(v) => Changement::Valeur(v),
    })
}

/// Corps de la modification : au moins un champ. `plafond`, `compte_excedent` et
/// `compte_manquant` (numéros de comptes du dossier) s'effacent par `null`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CorpsModification {
    nom: Option<String>,
    #[serde(default, deserialize_with = "changement")]
    plafond: Changement<String>,
    #[serde(default, deserialize_with = "changement")]
    compte_excedent: Changement<String>,
    #[serde(default, deserialize_with = "changement")]
    compte_manquant: Changement<String>,
    archive: Option<bool>,
}

/// Compte d'écart désigné : un compte de mouvement actif du plan du dossier (contrôle de la
/// saisie, `compta-core`), sinon 422 avec le motif du plan.
fn compte_d_ecart(r: &ReferentielDossier, numero: &str) -> Result<(Uuid, String), ErreurApi> {
    let numero = NumeroCompte::parse(numero).map_err(|_| ErreurApi::RequeteInvalide)?;
    r.etat()
        .plan
        .compte_de_mouvement_actif(&numero)
        .map_err(|e| {
            validation(match e {
                compta_core::ErreurPlan::Inactif(_) => MotifValidation::CompteInactif,
                compta_core::ErreurPlan::Regroupement(_) => MotifValidation::CompteDeRegroupement,
                _ => MotifValidation::CompteInconnu,
            })
        })?;
    let id = r.id_compte(&numero).ok_or_else(donnee)?;
    Ok((id, numero.to_string()))
}

#[allow(clippy::too_many_lines)]
async fn modifier(
    tx: &mut TransactionTenant,
    dossier: &DossierAutorise,
    requete: &Requete<'_>,
) -> Result<Resultat, ErreurApi> {
    sans_parametres(requete)?;
    let m: CorpsModification = corps(requete.corps)?;
    if m.nom.is_none()
        && m.plafond.est_inchange()
        && m.compte_excedent.est_inchange()
        && m.compte_manquant.est_inchange()
        && m.archive.is_none()
    {
        return Err(ErreurApi::RequeteInvalide);
    }
    let avant = compte_du_chemin(tx, requete, true).await?;
    let caisse = avant.type_tresorerie == TypeTresorerie::Caisse.code();
    let propre_a_la_caisse =
        m.plafond.est_valeur() || m.compte_excedent.est_valeur() || m.compte_manquant.est_valeur();
    if propre_a_la_caisse && !caisse {
        return Err(validation(MotifValidation::TresorerieParametreInvalide));
    }
    let nom = match &m.nom {
        Some(n) => TexteCourt::parse(n)
            .map_err(|_| ErreurApi::RequeteInvalide)?
            .as_str()
            .to_owned(),
        None => avant.nom.clone(),
    };
    let plafond = match &m.plafond {
        Changement::Inchange => avant.plafond.clone(),
        Changement::Efface => None,
        Changement::Valeur(p) => Some(texte(&plafond_lu(p, dossier.contexte().tenue()?)?)),
    };
    let r = charger(tx).await?.ok_or(ErreurApi::Introuvable)?;
    let designer = |champ: &Changement<String>,
                    actuel: &Option<(Uuid, String)>|
     -> Result<Option<(Uuid, String)>, ErreurApi> {
        match champ {
            Changement::Inchange => Ok(actuel.clone()),
            Changement::Efface => Ok(None),
            Changement::Valeur(n) => compte_d_ecart(&r, n).map(Some),
        }
    };
    let excedent = designer(&m.compte_excedent, &avant.compte_excedent)?;
    let manquant = designer(&m.compte_manquant, &avant.compte_manquant)?;
    let archive = m.archive.unwrap_or(avant.archive);
    let numero = |c: &Option<(Uuid, String)>| c.as_ref().map(|(_, n)| n.clone());
    let plafond_canon =
        |p: &Option<String>| p.as_deref().map(crate::base::brouillard::decimal_canonique);
    let mut trace = Trace::new(
        PorteeAudit::Dossier(dossier.id()),
        "compte_tresorerie",
        avant.id,
    );
    for (champ, av, ap) in [
        ("nom", Some(avant.nom.clone()), Some(nom.clone())),
        (
            "plafond",
            plafond_canon(&avant.plafond),
            plafond_canon(&plafond),
        ),
        (
            "compte_excedent",
            numero(&avant.compte_excedent),
            numero(&excedent),
        ),
        (
            "compte_manquant",
            numero(&avant.compte_manquant),
            numero(&manquant),
        ),
        (
            "archive",
            Some(avant.archive.to_string()),
            Some(archive.to_string()),
        ),
    ] {
        if av != ap {
            trace.champs.push((champ, av, ap));
        }
    }
    if trace.champs.is_empty() {
        return Err(conflit(MotifConflit::AucunChangement));
    }
    base::configurer(
        tx,
        avant.id,
        &Configuration {
            nom,
            plafond,
            compte_excedent_id: excedent.as_ref().map(|(id, _)| *id),
            compte_manquant_id: manquant.as_ref().map(|(id, _)| *id),
            archive,
        },
    )
    .await?;
    let apres = base::lire(tx, avant.id, false).await?.ok_or_else(donnee)?;
    let vue = vue_compte(tx, &apres, None).await?;
    Ok(Resultat {
        reponse: Json(vue).into_response(),
        traces: vec![trace],
    })
}

// --- Arrêté de caisse -------------------------------------------------------------------------

/// Corps de l'arrêté : `date` (`AAAA-MM-JJ`), `montant_compte` (espèces comptées, dans la
/// devise de tenue, positif ou nul), `detail` facultatif (lignes libres du procès-verbal de
/// comptage : billets, pièces…), repris dans la pièce interne.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CorpsArrete {
    date: String,
    montant_compte: String,
    #[serde(default)]
    detail: Vec<String>,
}

#[derive(Debug, Serialize)]
struct VueArreteCree {
    #[serde(flatten)]
    arrete: VueArrete,
    /// Avertissements, jamais des refus : écart sans compte désigné, solde de la caisse.
    avertissements: Vec<VueAvertissement>,
}

/// Instant de l'horloge, à la microseconde (précision de la forme canonique de la note).
fn maintenant() -> OffsetDateTime {
    let i = OffsetDateTime::now_utc();
    i.replace_nanosecond(i.nanosecond() / 1000 * 1000)
        .unwrap_or(i)
}

/// Corps de création du brouillon d'écart (celui de `POST …/brouillons`) : journal de la caisse,
/// date de l'arrêté ; excédent : débit de la caisse, crédit du compte d'excédent ; manquant :
/// débit du compte de manquant, crédit de la caisse.
fn corps_ecart(
    caisse: &CompteTresorerieLu,
    date: Date,
    ecart: &Montant,
    contrepartie: &str,
) -> Vec<u8> {
    let montant = ecart.valeur().abs().normalize().to_string();
    let (debit, credit) = if ecart.est_strictement_positif() {
        (caisse.compte.as_str(), contrepartie)
    } else {
        (contrepartie, caisse.compte.as_str())
    };
    serde_json::json!({
        "journal": caisse.journal,
        "date": interne::date(date),
        "libelle": "Arrêté de caisse : écart de comptage",
        "lignes": [
            {"compte": debit, "sens": "debit", "montant": montant},
            {"compte": credit, "sens": "credit", "montant": montant},
        ],
    })
    .to_string()
    .into_bytes()
}

#[allow(clippy::too_many_lines)]
async fn arreter(
    tx: &mut TransactionTenant,
    dossier: &DossierAutorise,
    requete: &Requete<'_>,
) -> Result<Resultat, ErreurApi> {
    sans_parametres(requete)?;
    let a: CorpsArrete = corps(requete.corps)?;
    if a.detail.len() > DETAIL_MAX {
        return Err(ErreurApi::RequeteInvalide);
    }
    let date = interne::lire_date(&a.date).map_err(|_| ErreurApi::RequeteInvalide)?;
    // Verrou de la caisse : ses arrêtés se suivent.
    let caisse = compte_du_chemin(tx, requete, true).await?;
    if caisse.type_tresorerie != TypeTresorerie::Caisse.code() {
        return Err(validation(MotifValidation::TresoreriePasUneCaisse));
    }
    if caisse.archive {
        return Err(conflit(MotifConflit::CompteTresorerieArchive));
    }
    let tenue = dossier.contexte().tenue()?;
    let compte = Montant::parse_tenue(&a.montant_compte, &tenue).map_err(|e| match e {
        compta_core::ErreurMontant::EchelleExcessive { .. } => {
            validation(MotifValidation::MontantEchelleTenue)
        }
        _ => ErreurApi::RequeteInvalide,
    })?;
    if compte.valeur().is_sign_negative() && !compte.est_nul() {
        return Err(ErreurApi::RequeteInvalide);
    }
    let solde = solde(tx, &caisse.compte, tenue.devise(), Some(date)).await?;
    let ecart = compte.soustraire(&solde).map_err(|_| donnee())?;
    let mut traces = Vec::new();
    let mut avertissements = Vec::new();

    // 1. Brouillon d'écart (mêmes contrôles que la saisie), avant tout dépôt de la pièce.
    let contrepartie = if ecart.est_nul() {
        None
    } else if ecart.est_strictement_positif() {
        caisse.compte_excedent.as_ref()
    } else {
        caisse.compte_manquant.as_ref()
    };
    if !ecart.est_nul() && contrepartie.is_none() {
        avertissements.push(VueAvertissement {
            code: "compte_ecart_non_designe",
            compte: None,
            solde: None,
            plafond: None,
        });
    }
    let brouillon = match contrepartie {
        None => None,
        Some((_, numero)) => {
            let octets = corps_ecart(&caisse, date, &ecart, numero);
            let (id, version, t) =
                super::brouillard::proposer(tx, dossier, &octets, requete.registre_fiscal()?)
                    .await?;
            traces.extend(t);
            Some((id, version))
        }
    };

    // 2. Comptage figé en pièce interne (note d'OD, forme `piece-interne-v1`).
    let contexte = *tx.contexte();
    let mut lignes = vec![
        format!("compte_tresorerie={}", caisse.id),
        format!("compte={}", caisse.compte),
        format!("journal={}", caisse.journal),
        format!("devise={}", caisse.devise),
        format!("montant_compte={}", texte(&compte)),
        format!("solde_comptable={}", texte(&solde)),
        format!("ecart={}", texte(&ecart)),
    ];
    lignes.extend(a.detail.iter().map(|d| format!("detail={d}")));
    let note = NoteOd::new(
        contexte.id_cabinet().uuid(),
        dossier.id().uuid(),
        contexte.id_utilisateur().uuid(),
        maintenant(),
        date,
        "Arrêté de caisse".to_owned(),
        lignes,
    )
    .map_err(|_| ErreurApi::RequeteInvalide)?;
    let (piece, trace_piece) = super::pieces::creer_note(tx, dossier, requete, &note).await?;
    traces.push(trace_piece);
    if let Some((id, version)) = brouillon {
        traces.push(super::brouillard::lier_principale(tx, dossier, id, version, piece).await?);
    }

    // 3. Arrêté, en ajout seul.
    let id = Uuid::new_v4();
    let (t_compte, t_solde, t_ecart) = (texte(&compte), texte(&solde), texte(&ecart));
    base::inserer_arrete(
        tx,
        &NouvelArrete {
            id,
            compte_tresorerie: caisse.id,
            date,
            montant_compte: &t_compte,
            solde_comptable: &t_solde,
            ecart: &t_ecart,
            piece,
            brouillon: brouillon.map(|(b, _)| b),
            instant: note.instant(),
        },
    )
    .await?;
    let plafond = caisse
        .plafond
        .as_deref()
        .map(|p| Montant::parse(p, tenue.devise()).map(|m| m.valeur()))
        .transpose()
        .map_err(|_| donnee())?;
    avertissements.extend(
        rapprochement::avertissements_solde(
            rapprochement::TypeTresorerie::Caisse,
            solde.valeur(),
            plafond,
        )
        .iter()
        .map(vue_avertissement),
    );
    let mut trace = Trace::new(PorteeAudit::Dossier(dossier.id()), "arrete_caisse", id);
    trace
        .donnees
        .push(("compte_tresorerie", caisse.id.to_string()));
    trace.donnees.push(("piece", piece.to_string()));
    if let Some((b, _)) = brouillon {
        trace.donnees.push(("brouillon", b.to_string()));
    }
    if !avertissements.is_empty() {
        let codes: Vec<&str> = avertissements.iter().map(|a| a.code).collect();
        trace.donnees.push(("avertissements", codes.join(",")));
    }
    for (champ, valeur) in [
        ("date", interne::date(date)),
        ("montant_compte", t_compte.clone()),
        ("solde_comptable", t_solde.clone()),
        ("ecart", t_ecart.clone()),
    ] {
        trace.champs.push((champ, None, Some(valeur)));
    }
    traces.push(trace);
    let lu = ArreteLu {
        id,
        date,
        montant_compte: t_compte,
        solde_comptable: t_solde,
        ecart: t_ecart,
        piece,
        brouillon: brouillon.map(|(b, _)| b),
        arrete_par: contexte.id_utilisateur().uuid(),
        arrete_le: note.instant(),
    };
    Ok(Resultat {
        reponse: (
            StatusCode::CREATED,
            Json(VueArreteCree {
                arrete: vue_arrete(caisse.id, &lu),
                avertissements,
            }),
        )
            .into_response(),
        traces,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn date_du_solde_stricte() {
        assert_eq!(date_du_solde(None), Ok(None));
        assert_eq!(date_du_solde(Some("")), Ok(None));
        assert_eq!(
            date_du_solde(Some("au=2026-11-30")),
            Ok(Some(
                Date::from_calendar_date(2026, time::Month::November, 30).unwrap()
            ))
        );
        for invalide in [
            "au=2026-02-30",
            "au=30/11/2026",
            "au=2026-11-30&au=2026-11-30",
            "autre=1",
            "au",
        ] {
            assert_eq!(
                date_du_solde(Some(invalide)),
                Err(ErreurApi::RequeteInvalide),
                "{invalide}"
            );
        }
    }

    #[test]
    fn chaque_type_a_son_prefixe_de_journal_et_son_type_pur() {
        for t in TypeTresorerie::TOUS {
            assert!(CodeJournal::parse(&format!("{}{RANG_MAX}", prefixe_journal(t))).is_ok());
            let _ = type_pur(t);
        }
        // Le compte par défaut de chaque type vient des données (règle absolue 9).
        let r = officiel().unwrap();
        for t in TypeTresorerie::TOUS {
            assert!(r.compte_par_defaut_tresorerie(t).is_some(), "{t}");
        }
    }

    #[test]
    fn corps_de_l_ecart_selon_le_sens() {
        let caisse = CompteTresorerieLu {
            id: Uuid::nil(),
            type_tresorerie: "caisse".to_owned(),
            nom: "Caisse".to_owned(),
            compte_id: Uuid::nil(),
            compte: "571-01".to_owned(),
            journal_id: Uuid::nil(),
            journal: "CA1".to_owned(),
            devise: "XOF".to_owned(),
            plafond: None,
            compte_excedent: None,
            compte_manquant: None,
            archive: false,
            cree_le: OffsetDateTime::UNIX_EPOCH,
        };
        let date = Date::from_calendar_date(2026, time::Month::November, 30).unwrap();
        let xof = Devise::parse("XOF").unwrap();
        let lire = |octets: Vec<u8>| serde_json::from_slice::<serde_json::Value>(&octets).unwrap();
        let excedent = lire(corps_ecart(
            &caisse,
            date,
            &Montant::parse("1500", xof).unwrap(),
            "7XX",
        ));
        assert_eq!(excedent["lignes"][0]["compte"], "571-01");
        assert_eq!(excedent["lignes"][0]["sens"], "debit");
        assert_eq!(excedent["lignes"][1]["compte"], "7XX");
        assert_eq!(excedent["lignes"][1]["montant"], "1500");
        let manquant = lire(corps_ecart(
            &caisse,
            date,
            &Montant::parse("-250.50", xof).unwrap(),
            "6XX",
        ));
        assert_eq!(manquant["lignes"][0]["compte"], "6XX");
        assert_eq!(manquant["lignes"][1]["compte"], "571-01");
        assert_eq!(manquant["lignes"][1]["montant"], "250.5");
        assert_eq!(manquant["journal"], "CA1");
        assert_eq!(manquant["date"], "2026-11-30");
    }
}
