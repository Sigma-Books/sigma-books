//! Routes du dossier et du référentiel (PR 11a de `docs/lots/L2.md`, L2-22) : plan de comptes,
//! journaux, tiers minimal, exercices ; et le référentiel d'un nouveau dossier, inscrit par la
//! création du dossier ([`lire_creation`], [`instancier`], appelés par `organisation`).
//!
//! Persistance et règles : [`crate::base::referentiel`]. Toute mutation passe par
//! [`muter`] (verrou du référentiel, copie, mutation pure, contrôle croisé, enregistrement des
//! seules différences) et rend une trace d'audit par changement enregistré.
//!
//! Défense en profondeur (ADR 0011) : même si Cedar se trompait, chaque cas d'usage relit le
//! dossier du chemin **visible** par l'utilisateur (affecté, sauf administrateur), et une
//! mutation (`ParametrerDossier`) exige en plus une appartenance active de rôle
//! `administrateur` ou `valideur`, lue en base. Une sous-ressource (`{compte}`, `{tiers}`) est
//! cherchée dans le seul dossier du contexte : celle d'un autre dossier est absente (404).

use authz::Action;
use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use compta_core::{
    CodeJournal, Compte, Exercice, FinPremierExercice, Journal, NumeroCompte, TexteCourt,
    TypeJournal, echeance_de_cloture,
};
use serde::{Deserialize, Serialize};
use time::{Date, Month, OffsetDateTime};
use uuid::Uuid;

use super::{ActionRoute, CasUsage, Methode, Module, Portee, Requete, Resultat, Route};
use crate::base::audit::PorteeAudit;
use crate::base::organisation::{lire_appartenance, lire_dossier_visible};
use crate::base::referentiel::{
    Changement, ErreurMutation, Instanciation, Modele, Mutation, Ouverture, RefusReferentiel,
    Tiers, TypeTiers, charger, muter, nom_genre_compte, nom_sens, nom_statut_exercice,
    nom_statut_periode, nom_type_journal,
};
use crate::base::{IdDossier, TransactionTenant};
use crate::serveur::audit::Trace;
use crate::serveur::erreur::{CategorieErreur, ErreurApi};
use crate::serveur::pipeline::{DossierAutorise, Garde};

/// Cas d'usage du module.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cas {
    /// Plan de comptes du dossier.
    ListerComptes,
    /// Créer un sous-compte ou un compte auxiliaire.
    CreerCompte,
    /// Renommer un compte créé par le dossier.
    RenommerCompte,
    /// Désactiver un compte.
    DesactiverCompte,
    /// Réactiver un compte.
    ReactiverCompte,
    /// Journaux du dossier.
    ListerJournaux,
    /// Créer un journal (`BQx`, `MMx`…).
    CreerJournal,
    /// Tiers du dossier, paginés.
    ListerTiers,
    /// Créer un tiers.
    CreerTiers,
    /// Renommer, désactiver ou réactiver un tiers.
    ModifierTiers,
    /// Exercices, périodes et échéances de clôture.
    ListerExercices,
}

/// Tranche du module.
pub const MODULE: Module = Module {
    nom: "referentiel",
    routes: ROUTES,
    champs_journal: &[],
};

const fn route(methode: Methode, chemin: &'static str, action: Action, cas: Cas) -> Route {
    Route {
        methode,
        chemin,
        action: ActionRoute::Cabinet(action),
        portee: Portee::Dossier,
        cas_usage: CasUsage::Referentiel(cas),
    }
}

/// Routes du module. Lectures : `AccederDossier` (les quatre rôles, nécessaires à la saisie) ;
/// mutations : `ParametrerDossier` (§20 « Paramétrer plan, journaux, fiscalité »).
pub const ROUTES: &[Route] = &[
    route(
        Methode::Get,
        "/api/cabinets/{cabinet}/dossiers/{dossier}/comptes",
        Action::AccederDossier,
        Cas::ListerComptes,
    ),
    route(
        Methode::Post,
        "/api/cabinets/{cabinet}/dossiers/{dossier}/comptes",
        Action::ParametrerDossier,
        Cas::CreerCompte,
    ),
    route(
        Methode::Post,
        "/api/cabinets/{cabinet}/dossiers/{dossier}/comptes/{compte}/intitule",
        Action::ParametrerDossier,
        Cas::RenommerCompte,
    ),
    route(
        Methode::Post,
        "/api/cabinets/{cabinet}/dossiers/{dossier}/comptes/{compte}/desactivation",
        Action::ParametrerDossier,
        Cas::DesactiverCompte,
    ),
    route(
        Methode::Post,
        "/api/cabinets/{cabinet}/dossiers/{dossier}/comptes/{compte}/reactivation",
        Action::ParametrerDossier,
        Cas::ReactiverCompte,
    ),
    route(
        Methode::Get,
        "/api/cabinets/{cabinet}/dossiers/{dossier}/journaux",
        Action::AccederDossier,
        Cas::ListerJournaux,
    ),
    route(
        Methode::Post,
        "/api/cabinets/{cabinet}/dossiers/{dossier}/journaux",
        Action::ParametrerDossier,
        Cas::CreerJournal,
    ),
    route(
        Methode::Get,
        "/api/cabinets/{cabinet}/dossiers/{dossier}/tiers",
        Action::AccederDossier,
        Cas::ListerTiers,
    ),
    route(
        Methode::Post,
        "/api/cabinets/{cabinet}/dossiers/{dossier}/tiers",
        Action::ParametrerDossier,
        Cas::CreerTiers,
    ),
    route(
        Methode::Post,
        "/api/cabinets/{cabinet}/dossiers/{dossier}/tiers/{tiers}/modification",
        Action::ParametrerDossier,
        Cas::ModifierTiers,
    ),
    route(
        Methode::Get,
        "/api/cabinets/{cabinet}/dossiers/{dossier}/exercices",
        Action::AccederDossier,
        Cas::ListerExercices,
    ),
];

// --- Création d'un dossier ------------------------------------------------------------------

/// Corps de la création d'un dossier (`POST /api/cabinets/{cabinet}/dossiers`) :
/// - entité nouvelle : `{"nom", "nature":"creation", "debut_activite":"AAAA-MM-JJ",
///   "fin_premier_exercice":"annee_de_debut"|"annee_suivante"}` ;
/// - dossier repris : `{"nom", "nature":"reprise", "annee":AAAA}`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CorpsCreation {
    nom: String,
    nature: String,
    debut_activite: Option<String>,
    fin_premier_exercice: Option<String>,
    annee: Option<i32>,
}

/// Demande de création d'un dossier : son nom et son référentiel, calculé en pur.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DemandeCreation {
    nom: String,
    instanciation: Instanciation,
}

impl DemandeCreation {
    /// Nom demandé (validé par `organisation`).
    pub(crate) fn nom(&self) -> &str {
        &self.nom
    }
}

/// Date `AAAA-MM-JJ` stricte.
fn date_lue(texte: &str) -> Option<Date> {
    let octets = texte.as_bytes();
    let forme = octets.len() == 10
        && octets[4] == b'-'
        && octets[7] == b'-'
        && octets
            .iter()
            .enumerate()
            .all(|(i, b)| i == 4 || i == 7 || b.is_ascii_digit());
    if !forme {
        return None;
    }
    let annee = texte.get(0..4)?.parse().ok()?;
    let mois = Month::try_from(texte.get(5..7)?.parse::<u8>().ok()?).ok()?;
    let jour = texte.get(8..10)?.parse().ok()?;
    Date::from_calendar_date(annee, mois, jour).ok()
}

/// Date du jour de l'API (UTC), pour refuser un dossier repris dans une année future.
pub(crate) fn date_du_jour() -> Date {
    OffsetDateTime::now_utc().date()
}

/// Lit le corps de la création d'un dossier et prépare **en pur** tout son référentiel (plan,
/// journaux, premier exercice) : toute erreur survient avant la première écriture.
///
/// # Errors
/// [`ErreurApi::RequeteInvalide`] : corps mal formé, combinaison de champs invalide, refus des
/// règles d'ouverture ; erreur interne si le référentiel officiel est invalide.
pub(crate) fn lire_creation(corps: &[u8], aujourd_hui: Date) -> Result<DemandeCreation, ErreurApi> {
    let Json(c) =
        Json::<CorpsCreation>::from_bytes(corps).map_err(|_| ErreurApi::RequeteInvalide)?;
    let ouverture = match (
        c.nature.as_str(),
        c.debut_activite.as_deref(),
        c.fin_premier_exercice.as_deref(),
        c.annee,
    ) {
        ("creation", Some(debut), Some(fin), None) => Ouverture::Creation {
            debut_activite: date_lue(debut).ok_or(ErreurApi::RequeteInvalide)?,
            fin: match fin {
                "annee_de_debut" => FinPremierExercice::AnneeDeDebut,
                "annee_suivante" => FinPremierExercice::AnneeSuivante,
                _ => return Err(ErreurApi::RequeteInvalide),
            },
        },
        ("reprise", None, None, Some(annee)) => Ouverture::Reprise { annee },
        _ => return Err(ErreurApi::RequeteInvalide),
    };
    let modele = Modele::officiel()?;
    let instanciation = Instanciation::preparer(modele, ouverture, aujourd_hui)
        .map_err(|_| ErreurApi::RequeteInvalide)?;
    Ok(DemandeCreation {
        nom: c.nom,
        instanciation,
    })
}

/// Inscrit le référentiel préparé dans le dossier créé et rend les données citées par la
/// première entrée de l'audit du dossier : référentiel (version et empreinte), nombre de
/// comptes, codes des journaux, premier exercice, nature et début d'activité. Pas une entrée
/// par compte : la copie est citée par son empreinte (ADR 0014, minimisation).
///
/// # Errors
/// Erreur de base : la transaction de la création est abandonnée.
pub(crate) async fn instancier(
    tx: &mut TransactionTenant,
    dossier: IdDossier,
    demande: &DemandeCreation,
) -> Result<Vec<(&'static str, String)>, ErreurApi> {
    let i = &demande.instanciation;
    crate::base::referentiel::instancier(tx, dossier, i).await?;
    let journaux: Vec<String> = i
        .etat()
        .journaux
        .journaux()
        .map(|j| j.code().to_string())
        .collect();
    let exercice = i
        .exercices()
        .exercices()
        .first()
        .ok_or(ErreurApi::Interne(CategorieErreur::Donnee))?;
    let mut donnees = vec![
        (
            "referentiel",
            format!("{} ; {}", i.version(), i.empreinte()),
        ),
        ("comptes", i.etat().plan.comptes().count().to_string()),
        ("journaux", journaux.join(",")),
        (
            "exercice",
            format!(
                "{} ; {} ; {}",
                exercice.code(),
                exercice.debut(),
                exercice.fin()
            ),
        ),
        ("nature", i.ouverture().nature().to_owned()),
    ];
    if let Ouverture::Creation { debut_activite, .. } = i.ouverture() {
        donnees.push(("debut_activite", debut_activite.to_string()));
    }
    Ok(donnees)
}

// --- Vues ---------------------------------------------------------------------------------

// Vue JSON : chaque booléen est un attribut distinct du compte.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Serialize)]
struct VueCompte {
    id: String,
    numero: String,
    intitule: String,
    genre: &'static str,
    sens: &'static str,
    collectif: bool,
    tresorerie: bool,
    actif: bool,
    /// Compte de mouvement (sans enfant) : seul un tel compte reçoit des écritures.
    mouvement: bool,
}

#[derive(Debug, Serialize)]
struct VueJournal {
    id: String,
    code: String,
    intitule: String,
    #[serde(rename = "type")]
    genre: &'static str,
    admis_en_cloture: bool,
    compte_tresorerie: Option<String>,
}

#[derive(Debug, Serialize)]
struct VueTiers {
    id: String,
    nom: String,
    #[serde(rename = "type")]
    genre: &'static str,
    compte_collectif: String,
    actif: bool,
}

#[derive(Debug, Serialize)]
struct VuePeriode {
    id: String,
    debut: String,
    fin: String,
    statut: &'static str,
    /// Échéance de la clôture informatique (AUDCIF art. 22 3°), si la période est ouverte ou
    /// verrouillée.
    echeance_cloture: Option<String>,
}

#[derive(Debug, Serialize)]
struct VueExercice {
    id: String,
    code: String,
    debut: String,
    fin: String,
    statut: &'static str,
    periodes: Vec<VuePeriode>,
}

#[derive(Debug, Serialize)]
struct ListeComptes {
    comptes: Vec<VueCompte>,
}

#[derive(Debug, Serialize)]
struct ListeJournaux {
    journaux: Vec<VueJournal>,
}

#[derive(Debug, Serialize)]
struct ListeTiers {
    tiers: Vec<VueTiers>,
    /// Identifiant du dernier tiers de la page, à passer en `apres` ; absent en fin de liste.
    suite: Option<String>,
}

#[derive(Debug, Serialize)]
struct ListeExercices {
    exercices: Vec<VueExercice>,
}

fn vue_compte(id: Uuid, compte: &Compte, plan: &compta_core::PlanComptes) -> VueCompte {
    VueCompte {
        id: id.to_string(),
        numero: compte.numero().to_string(),
        intitule: compte.intitule().to_string(),
        genre: nom_genre_compte(compte.genre()),
        sens: nom_sens(compte.sens()),
        collectif: compte.est_collectif(),
        tresorerie: compte.est_admis_en_journal_de_tresorerie(),
        actif: compte.est_actif(),
        mouvement: !plan.a_des_enfants(compte.numero()),
    }
}

fn vue_journal(id: Uuid, journal: &Journal) -> VueJournal {
    VueJournal {
        id: id.to_string(),
        code: journal.code().to_string(),
        intitule: journal.intitule().to_string(),
        genre: nom_type_journal(journal.genre()),
        admis_en_cloture: journal.admis_en_cloture(),
        compte_tresorerie: journal.compte_tresorerie().map(ToString::to_string),
    }
}

fn vue_tiers(id: Uuid, tiers: &Tiers) -> VueTiers {
    VueTiers {
        id: id.to_string(),
        nom: tiers.nom().to_string(),
        genre: tiers.genre().nom(),
        compte_collectif: tiers.compte_collectif().to_string(),
        actif: tiers.est_actif(),
    }
}

fn vue_exercice(
    id: Uuid,
    exercice: &Exercice,
    id_periode: impl Fn(Date) -> Option<Uuid>,
) -> Result<VueExercice, ErreurApi> {
    let interne = ErreurApi::Interne(CategorieErreur::Donnee);
    let periodes = exercice
        .periodes()
        .iter()
        .map(|p| {
            let echeance = match p.statut() {
                compta_core::StatutPeriode::Cloturee => None,
                _ => Some(
                    echeance_de_cloture(exercice, p)
                        .map_err(|_| interne)?
                        .to_string(),
                ),
            };
            Ok(VuePeriode {
                id: id_periode(p.debut()).ok_or(interne)?.to_string(),
                debut: p.debut().to_string(),
                fin: p.fin().to_string(),
                statut: nom_statut_periode(p.statut()),
                echeance_cloture: echeance,
            })
        })
        .collect::<Result<Vec<_>, ErreurApi>>()?;
    Ok(VueExercice {
        id: id.to_string(),
        code: exercice.code().to_string(),
        debut: exercice.debut().to_string(),
        fin: exercice.fin().to_string(),
        statut: nom_statut_exercice(exercice.statut()),
        periodes,
    })
}

// --- Exécution ----------------------------------------------------------------------------

/// Taille d'une page de tiers par défaut.
const PAGE_TIERS_PAR_DEFAUT: usize = 50;
/// Taille maximale d'une page de tiers.
const PAGE_TIERS_MAX: usize = 200;

/// Exécute un cas d'usage du module dans la transaction du pipeline.
pub(crate) async fn executer(
    cas: Cas,
    tx: &mut TransactionTenant,
    garde: &Garde,
    requete: &Requete<'_>,
) -> Result<Resultat, ErreurApi> {
    let dossier = dossier_visible(tx, garde).await?;
    match cas {
        Cas::ListerComptes | Cas::ListerJournaux | Cas::ListerTiers | Cas::ListerExercices => {
            lire(cas, tx, requete).await
        }
        Cas::CreerCompte
        | Cas::RenommerCompte
        | Cas::DesactiverCompte
        | Cas::ReactiverCompte
        | Cas::CreerJournal
        | Cas::CreerTiers
        | Cas::ModifierTiers => {
            exiger_parametreur(tx).await?;
            parametrer(cas, tx, dossier.id(), requete).await
        }
    }
}

/// Lectures du référentiel du dossier du chemin.
async fn lire(
    cas: Cas,
    tx: &mut TransactionTenant,
    requete: &Requete<'_>,
) -> Result<Resultat, ErreurApi> {
    match cas {
        Cas::ListerComptes => {
            let r = charger(tx).await?.ok_or(ErreurApi::Introuvable)?;
            let plan = &r.etat().plan;
            let comptes = plan
                .comptes()
                .map(|c| {
                    let id = r
                        .id_compte(c.numero())
                        .ok_or(ErreurApi::Interne(CategorieErreur::Donnee))?;
                    Ok(vue_compte(id, c, plan))
                })
                .collect::<Result<Vec<_>, ErreurApi>>()?;
            Ok(Resultat::lecture(
                Json(ListeComptes { comptes }).into_response(),
            ))
        }
        Cas::ListerJournaux => {
            let r = charger(tx).await?.ok_or(ErreurApi::Introuvable)?;
            let journaux = r
                .etat()
                .journaux
                .journaux()
                .map(|j| {
                    let id = r
                        .id_journal(j.code())
                        .ok_or(ErreurApi::Interne(CategorieErreur::Donnee))?;
                    Ok(vue_journal(id, j))
                })
                .collect::<Result<Vec<_>, ErreurApi>>()?;
            Ok(Resultat::lecture(
                Json(ListeJournaux { journaux }).into_response(),
            ))
        }
        Cas::ListerTiers => {
            let (apres, limite) = pagination(requete.parametres)?;
            let r = charger(tx).await?.ok_or(ErreurApi::Introuvable)?;
            let mut tous: Vec<(&Uuid, &Tiers)> = r.etat().tiers.iter().collect();
            tous.sort_by(|a, b| (a.1.nom(), a.0).cmp(&(b.1.nom(), b.0)));
            let debut = match apres {
                None => 0,
                Some(id) => {
                    tous.iter()
                        .position(|(i, _)| **i == id)
                        .ok_or(ErreurApi::RequeteInvalide)?
                        + 1
                }
            };
            let page: Vec<VueTiers> = tous
                .iter()
                .skip(debut)
                .take(limite)
                .map(|(id, t)| vue_tiers(**id, t))
                .collect();
            let suite = (debut + page.len() < tous.len())
                .then(|| page.last().map(|t| t.id.clone()))
                .flatten();
            Ok(Resultat::lecture(
                Json(ListeTiers { tiers: page, suite }).into_response(),
            ))
        }
        Cas::ListerExercices => {
            let r = charger(tx).await?.ok_or(ErreurApi::Introuvable)?;
            let exercices = r
                .exercices()
                .exercices()
                .iter()
                .map(|e| {
                    let id = r
                        .id_exercice(e.code())
                        .ok_or(ErreurApi::Interne(CategorieErreur::Donnee))?;
                    vue_exercice(id, e, |d| r.id_periode(d))
                })
                .collect::<Result<Vec<_>, ErreurApi>>()?;
            Ok(Resultat::lecture(
                Json(ListeExercices { exercices }).into_response(),
            ))
        }
        _ => Err(ErreurApi::Interne(CategorieErreur::Donnee)),
    }
}

/// Pagination des tiers : `apres` (UUID du dernier tiers reçu) et `limite` (1 à
/// [`PAGE_TIERS_MAX`]). Tout autre paramètre, doublon ou valeur invalide : 400.
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
                    .filter(|n| (1..=PAGE_TIERS_MAX).contains(n))
                    .ok_or(ErreurApi::RequeteInvalide)?;
                limite.replace(n).is_some()
            }
            _ => true,
        };
        if deja {
            return Err(ErreurApi::RequeteInvalide);
        }
    }
    Ok((apres, limite.unwrap_or(PAGE_TIERS_PAR_DEFAUT)))
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

/// Défense en profondeur de `ParametrerDossier` : appartenance **active** lue en base, de rôle
/// `administrateur` ou `valideur` (§20). Appartenance absente ou suspendue : 404 ; autre rôle :
/// 403.
async fn exiger_parametreur(tx: &mut TransactionTenant) -> Result<(), ErreurApi> {
    let appartenance = lire_appartenance(tx)
        .await?
        .filter(|a| a.actif)
        .ok_or(ErreurApi::Introuvable)?;
    let autorises = [authz::Role::Administrateur, authz::Role::Valideur];
    if autorises.iter().any(|r| r.nom() == appartenance.role) {
        Ok(())
    } else {
        Err(ErreurApi::Interdit)
    }
}

fn corps<T: serde::de::DeserializeOwned>(octets: &[u8]) -> Result<T, ErreurApi> {
    Json::<T>::from_bytes(octets)
        .map(|Json(t)| t)
        .map_err(|_| ErreurApi::RequeteInvalide)
}

fn texte(valeur: &str) -> Result<TexteCourt, RefusReferentiel> {
    TexteCourt::parse(valeur).map_err(|_| RefusReferentiel::Valeur)
}

fn numero(valeur: &str) -> Result<NumeroCompte, RefusReferentiel> {
    NumeroCompte::parse(valeur).map_err(|_| RefusReferentiel::Valeur)
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CorpsCompte {
    numero: String,
    intitule: String,
    genre: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CorpsIntitule {
    intitule: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CorpsVide {}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CorpsJournal {
    code: String,
    intitule: String,
    #[serde(rename = "type")]
    genre: String,
    compte_tresorerie: Option<String>,
    admis_en_cloture: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CorpsTiers {
    nom: String,
    #[serde(rename = "type")]
    genre: String,
    compte_collectif: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CorpsModificationTiers {
    nom: Option<String>,
    actif: Option<bool>,
}

/// Ressource à rendre après une mutation.
enum Rendu {
    Compte(NumeroCompte),
    Journal(CodeJournal),
    Tiers(Uuid),
}

/// Mutation pure d'une route de paramétrage, appliquée par [`muter`] à une copie de l'état.
async fn parametrer(
    cas: Cas,
    tx: &mut TransactionTenant,
    dossier: IdDossier,
    requete: &Requete<'_>,
) -> Result<Resultat, ErreurApi> {
    let sous = requete.sous_ressource;
    let statut = match cas {
        Cas::CreerCompte | Cas::CreerJournal | Cas::CreerTiers => StatusCode::CREATED,
        _ => StatusCode::OK,
    };
    let resultat = match cas {
        Cas::CreerCompte => {
            let c: CorpsCompte = corps(requete.corps)?;
            muter(tx, move |_, etat| {
                let n = numero(&c.numero)?;
                let intitule = texte(&c.intitule)?;
                match c.genre.as_str() {
                    "sous_compte" => etat.plan.ajouter_sous_compte(n.clone(), intitule)?,
                    "auxiliaire" => etat.plan.ajouter_auxiliaire(n.clone(), intitule)?,
                    _ => return Err(RefusReferentiel::Valeur),
                };
                Ok(Rendu::Compte(n))
            })
            .await
        }
        Cas::RenommerCompte | Cas::DesactiverCompte | Cas::ReactiverCompte => {
            let id = sous.ok_or(ErreurApi::Introuvable)?;
            let intitule = if cas == Cas::RenommerCompte {
                Some(corps::<CorpsIntitule>(requete.corps)?.intitule)
            } else {
                corps::<CorpsVide>(requete.corps)?;
                None
            };
            muter(tx, move |relu, etat| {
                let n = relu
                    .compte_par_id(id)
                    .ok_or(RefusReferentiel::Inconnu)?
                    .numero()
                    .clone();
                match (cas, intitule) {
                    (Cas::RenommerCompte, Some(i)) => etat.plan.renommer(&n, texte(&i)?)?,
                    (Cas::DesactiverCompte, _) => etat.plan.desactiver(&n)?,
                    _ => etat.plan.reactiver(&n)?,
                }
                Ok(Rendu::Compte(n))
            })
            .await
        }
        Cas::CreerJournal => {
            let c: CorpsJournal = corps(requete.corps)?;
            muter(tx, move |_, etat| ajouter_journal(&c, etat)).await
        }
        Cas::CreerTiers => {
            let c: CorpsTiers = corps(requete.corps)?;
            let id = Uuid::new_v4();
            muter(tx, move |_, etat| {
                let genre = TypeTiers::parse(&c.genre).ok_or(RefusReferentiel::Valeur)?;
                let tiers = Tiers::nouveau(texte(&c.nom)?, genre, numero(&c.compte_collectif)?);
                etat.tiers.insert(id, tiers);
                Ok(Rendu::Tiers(id))
            })
            .await
        }
        Cas::ModifierTiers => {
            let id = sous.ok_or(ErreurApi::Introuvable)?;
            let c: CorpsModificationTiers = corps(requete.corps)?;
            if c.nom.is_none() && c.actif.is_none() {
                return Err(ErreurApi::RequeteInvalide);
            }
            muter(tx, move |_, etat| {
                let tiers = etat.tiers.get_mut(&id).ok_or(RefusReferentiel::Inconnu)?;
                if let Some(nom) = &c.nom {
                    tiers.renommer(texte(nom)?);
                }
                if let Some(actif) = c.actif {
                    tiers.definir_actif(actif);
                }
                Ok(Rendu::Tiers(id))
            })
            .await
        }
        Cas::ListerComptes | Cas::ListerJournaux | Cas::ListerTiers | Cas::ListerExercices => {
            return Err(ErreurApi::Interne(CategorieErreur::Donnee));
        }
    };
    let mutation = resultat.map_err(|e| match e {
        // Sous-ressource inconnue dans ce dossier, peut-être d'un autre : absente.
        ErreurMutation::Refus(RefusReferentiel::Inconnu) | ErreurMutation::Absent => {
            ErreurApi::Introuvable
        }
        ErreurMutation::Refus(_) => ErreurApi::RequeteInvalide,
        ErreurMutation::Base(e) => ErreurApi::from(e),
    })?;
    rendre(dossier, statut, &mutation)
}

/// Mutation pure de la création d'un journal.
fn ajouter_journal(
    c: &CorpsJournal,
    etat: &mut crate::base::referentiel::EtatReferentiel,
) -> Result<Rendu, RefusReferentiel> {
    let genre = TypeJournal::parse(&c.genre).map_err(|_| RefusReferentiel::Valeur)?;
    // Les à-nouveaux sont générés, jamais saisis : le journal AN par défaut suffit.
    if genre == TypeJournal::ANouveaux {
        return Err(RefusReferentiel::Valeur);
    }
    let code = CodeJournal::parse(&c.code).map_err(|_| RefusReferentiel::Valeur)?;
    let compte = c.compte_tresorerie.as_deref().map(numero).transpose()?;
    let journal = Journal::nouveau(code.clone(), texte(&c.intitule)?, genre, compte, &etat.plan)?;
    let journal = if c.admis_en_cloture == Some(true) {
        journal.admettre_en_cloture()?
    } else {
        journal
    };
    etat.journaux.ajouter(journal)?;
    Ok(Rendu::Journal(code))
}

/// Réponse et traces d'une mutation enregistrée : une trace au dossier par changement.
fn rendre(
    dossier: IdDossier,
    statut: StatusCode,
    mutation: &Mutation<Rendu>,
) -> Result<Resultat, ErreurApi> {
    let traces = mutation
        .changements
        .iter()
        .map(|c: &Changement| {
            let mut trace = Trace::new(PorteeAudit::Dossier(dossier), c.genre, c.id);
            trace.donnees.clone_from(&c.donnees);
            trace.champs.clone_from(&c.champs);
            trace
        })
        .collect();
    let interne = ErreurApi::Interne(CategorieErreur::Donnee);
    let id_de = |genre: &str| {
        mutation
            .changements
            .iter()
            .find(|c| c.genre == genre)
            .map(|c| c.id)
            .ok_or(interne)
    };
    let etat = &mutation.etat;
    let reponse: Response = match &mutation.resultat {
        Rendu::Compte(n) => {
            let compte = etat.plan.compte(n).ok_or(interne)?;
            (
                statut,
                Json(vue_compte(id_de("compte")?, compte, &etat.plan)),
            )
                .into_response()
        }
        Rendu::Journal(code) => {
            let journal = etat.journaux.journal(code).ok_or(interne)?;
            (statut, Json(vue_journal(id_de("journal")?, journal))).into_response()
        }
        Rendu::Tiers(id) => {
            let tiers = etat.tiers.get(id).ok_or(interne)?;
            (statut, Json(vue_tiers(*id, tiers))).into_response()
        }
    };
    Ok(Resultat { reponse, traces })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn jour() -> Date {
        Date::from_calendar_date(2026, Month::November, 19).unwrap()
    }

    #[test]
    fn corps_de_creation() {
        for valide in [
            r#"{"nom":"A","nature":"reprise","annee":2026}"#,
            r#"{"nom":"A","nature":"creation","debut_activite":"2026-09-01","fin_premier_exercice":"annee_suivante"}"#,
            r#"{"nom":"A","nature":"creation","debut_activite":"2027-01-01","fin_premier_exercice":"annee_de_debut"}"#,
        ] {
            assert!(lire_creation(valide.as_bytes(), jour()).is_ok(), "{valide}");
        }
        for invalide in [
            r#"{"nom":"A"}"#,
            r#"{"nom":"A","nature":"reprise","annee":2027}"#,
            r#"{"nom":"A","nature":"reprise"}"#,
            r#"{"nom":"A","nature":"reprise","annee":2026,"debut_activite":"2026-01-01"}"#,
            r#"{"nom":"A","nature":"creation","debut_activite":"2026-03-01","fin_premier_exercice":"annee_suivante"}"#,
            r#"{"nom":"A","nature":"creation","debut_activite":"2026-3-01","fin_premier_exercice":"annee_de_debut"}"#,
            r#"{"nom":"A","nature":"creation","debut_activite":"2026-02-30","fin_premier_exercice":"annee_de_debut"}"#,
            r#"{"nom":"A","nature":"creation","debut_activite":"2026-02-01","fin_premier_exercice":"autre"}"#,
            r#"{"nom":"A","nature":"creation","debut_activite":"2026-02-01"}"#,
            r#"{"nom":"A","nature":"autre","annee":2026}"#,
            r#"{"nom":"A","nature":"reprise","annee":2026,"autre":1}"#,
            "pas du json",
        ] {
            assert_eq!(
                lire_creation(invalide.as_bytes(), jour()),
                Err(ErreurApi::RequeteInvalide),
                "{invalide}"
            );
        }
    }

    #[test]
    fn pagination_des_tiers() {
        let id = Uuid::new_v4();
        assert_eq!(pagination(None), Ok((None, PAGE_TIERS_PAR_DEFAUT)));
        assert_eq!(
            pagination(Some(&format!("apres={id}&limite=3"))),
            Ok((Some(id), 3))
        );
        for invalide in [
            "limite=0",
            "limite=201",
            "apres=x",
            "limite=1&limite=2",
            "autre=1",
            "limite",
        ] {
            assert_eq!(
                pagination(Some(invalide)),
                Err(ErreurApi::RequeteInvalide),
                "{invalide}"
            );
        }
    }
}
