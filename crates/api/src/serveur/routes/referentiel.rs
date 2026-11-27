//! Routes du dossier et du référentiel (PR 11a de `docs/lots/L2.md`, L2-22) : plan de comptes,
//! journaux, exercices (les tiers ont leur module, [`super::tiers`], depuis le lot L3) ;
//! transitions des périodes et exercice suivant (PR 11b, L2-23, sous-module `periodes`) ; et le
//! référentiel d'un nouveau dossier, inscrit par la création du dossier ([`lire_creation`],
//! [`instancier`], appelés par `organisation`).
//!
//! Persistance et règles : [`crate::base::referentiel`]. Toute mutation passe par
//! [`muter`] (verrou du référentiel, copie, mutation pure, contrôle croisé, enregistrement des
//! seules différences) et rend une trace d'audit par changement enregistré.
//!
//! Défense en profondeur (ADR 0011) : même si Cedar se trompait, chaque cas d'usage relit le
//! dossier du chemin **visible** par l'utilisateur (affecté, sauf administrateur), et une
//! mutation (`ParametrerDossier`) exige en plus une appartenance active de rôle
//! `administrateur` ou `valideur`, lue en base. Une sous-ressource (`{compte}`, `{periode}`) est
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
use syscohada::PackPays;
use time::{Date, Month, OffsetDateTime};
use uuid::Uuid;

use super::{ActionRoute, CasUsage, Methode, Module, Portee, Requete, Resultat, Route};
use crate::base::audit::PorteeAudit;
use crate::base::organisation::{lire_appartenance, lire_dossier_visible};
use crate::base::referentiel::{
    Changement, ErreurMutation, Instanciation, Modele, Mutation, Ouverture, RefusReferentiel,
    charger, muter, nom_genre_compte, nom_sens, nom_statut_exercice, nom_statut_periode,
    nom_type_journal,
};
use crate::base::{IdDossier, TransactionTenant};
use crate::contexte_dossier::{
    ContexteDossier, DemandeContexte, IdentifiantLegal, RefusContexte, controler_identifiants,
    resoudre,
};
use crate::serveur::audit::Trace;
use crate::serveur::erreur::{CategorieErreur, ErreurApi, MotifValidation};
use crate::serveur::pipeline::{DossierAutorise, Garde, id_chemin};

mod periodes;

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
    /// Exercices, périodes et échéances de clôture.
    ListerExercices,
    /// Verrouiller une période (PR 11b).
    VerrouillerPeriode,
    /// Déverrouiller une période, avec un motif (PR 11b).
    DeverrouillerPeriode,
    /// Clôture informatique d'une période, irréversible (PR 11b).
    CloturerPeriode,
    /// Ouvrir l'exercice suivant (PR 11b).
    OuvrirExerciceSuivant,
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
        "/api/cabinets/{cabinet}/dossiers/{dossier}/exercices",
        Action::AccederDossier,
        Cas::ListerExercices,
    ),
    // Périodes et exercice suivant (PR 11b), dans l'ordre d'un parcours : verrouiller,
    // déverrouiller, clôturer, puis ouvrir l'exercice suivant.
    route(
        Methode::Post,
        "/api/cabinets/{cabinet}/dossiers/{dossier}/periodes/{periode}/verrouillage",
        Action::VerrouillerPeriode,
        Cas::VerrouillerPeriode,
    ),
    route(
        Methode::Post,
        "/api/cabinets/{cabinet}/dossiers/{dossier}/periodes/{periode}/deverrouillage",
        Action::DeverrouillerPeriode,
        Cas::DeverrouillerPeriode,
    ),
    route(
        Methode::Post,
        "/api/cabinets/{cabinet}/dossiers/{dossier}/periodes/{periode}/cloture",
        Action::CloturerPeriode,
        Cas::CloturerPeriode,
    ),
    route(
        Methode::Post,
        "/api/cabinets/{cabinet}/dossiers/{dossier}/exercices/suivant",
        Action::OuvrirExerciceSuivant,
        Cas::OuvrirExerciceSuivant,
    ),
];

// --- Création d'un dossier ------------------------------------------------------------------

/// Corps de la création d'un dossier (`POST /api/cabinets/{cabinet}/dossiers`) :
/// - entité nouvelle : `{"nom", "pays", "nature":"creation", "debut_activite":"AAAA-MM-JJ",
///   "fin_premier_exercice":"annee_de_debut"|"annee_suivante"}` ;
/// - dossier repris : `{"nom", "pays", "nature":"reprise", "annee":AAAA}` ;
/// - dans les deux cas, facultatifs : `"devise"` (ISO 4217), `"fuseau"` (IANA), `"langue"`
///   (BCP 47), `"forme_juridique"`, `"identifiants_legaux":[{"type", "valeur"}]`. Le **pays**
///   est obligatoire ; devise, fuseau et langue valent par défaut ceux de son pack (ADR 0027).
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CorpsCreation {
    nom: String,
    pays: String,
    devise: Option<String>,
    fuseau: Option<String>,
    langue: Option<String>,
    forme_juridique: Option<String>,
    #[serde(default)]
    identifiants_legaux: Vec<CorpsIdentifiant>,
    nature: String,
    debut_activite: Option<String>,
    fin_premier_exercice: Option<String>,
    annee: Option<i32>,
}

/// Un identifiant légal du corps de la création.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CorpsIdentifiant {
    #[serde(rename = "type")]
    pub(crate) type_identifiant: String,
    pub(crate) valeur: String,
}

/// Demande de création d'un dossier : son nom, son contexte (pays, devise de tenue, fuseau,
/// langue, forme juridique, identifiants légaux) et son référentiel, calculé en pur.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DemandeCreation {
    nom: String,
    contexte: ContexteDossier,
    identifiants: Vec<IdentifiantLegal>,
    instanciation: Instanciation,
}

impl DemandeCreation {
    /// Nom demandé (validé par `organisation`).
    pub(crate) fn nom(&self) -> &str {
        &self.nom
    }

    /// Contexte du dossier à créer (valeurs par défaut du pays complétées, contrôlées).
    pub(crate) fn contexte(&self) -> &ContexteDossier {
        &self.contexte
    }

    /// Identifiants légaux demandés (types du pays, valeurs bornées).
    pub(crate) fn identifiants(&self) -> &[IdentifiantLegal] {
        &self.identifiants
    }
}

/// Refus de contexte → réponse 422 d'un code stable.
pub(crate) fn refus_contexte(refus: RefusContexte) -> ErreurApi {
    let motif = match refus {
        RefusContexte::PaysInconnu => MotifValidation::PaysInconnu,
        RefusContexte::DeviseInconnue => MotifValidation::DeviseTenueInconnue,
        RefusContexte::FuseauInconnu => MotifValidation::FuseauInconnu,
        RefusContexte::Invalide => MotifValidation::ContexteDossierInvalide,
    };
    ErreurApi::Validation { motif, ligne: None }
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

/// Lit le corps de la création d'un dossier, résout son contexte et prépare **en pur** tout son
/// référentiel (plan, journaux, premier exercice) : toute erreur survient avant la première
/// écriture. La date du jour (règles d'ouverture) est celle du **fuseau du dossier**.
///
/// # Errors
/// [`ErreurApi::RequeteInvalide`] : corps mal formé, combinaison de champs invalide, refus des
/// règles d'ouverture ; 422 `pays_inconnu`, `devise_tenue_inconnue`, `fuseau_inconnu` ou
/// `contexte_dossier_invalide` ; erreur interne si le référentiel officiel est invalide.
pub(crate) fn lire_creation(
    corps: &[u8],
    maintenant: OffsetDateTime,
) -> Result<DemandeCreation, ErreurApi> {
    let Json(c) =
        Json::<CorpsCreation>::from_bytes(corps).map_err(|_| ErreurApi::RequeteInvalide)?;
    let pack = PackPays::officiel().map_err(|_| ErreurApi::Interne(CategorieErreur::Donnee))?;
    let contexte = resoudre(
        pack,
        &DemandeContexte {
            pays: &c.pays,
            devise: c.devise.as_deref(),
            fuseau: c.fuseau.as_deref(),
            langue: c.langue.as_deref(),
            forme_juridique: c.forme_juridique.as_deref(),
        },
    )
    .map_err(refus_contexte)?;
    let identifiants: Vec<IdentifiantLegal> = c
        .identifiants_legaux
        .iter()
        .map(|i| IdentifiantLegal {
            type_identifiant: i.type_identifiant.clone(),
            valeur: i.valeur.clone(),
        })
        .collect();
    controler_identifiants(pack, &contexte.pays, &identifiants).map_err(refus_contexte)?;
    let aujourd_hui = contexte
        .date_du_jour(maintenant)
        .map_err(|_| ErreurApi::Interne(CategorieErreur::Donnee))?;
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
        contexte,
        identifiants,
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

fn vue_periode(
    id: Uuid,
    exercice: &Exercice,
    p: &compta_core::Periode,
) -> Result<VuePeriode, ErreurApi> {
    let echeance = match p.statut() {
        compta_core::StatutPeriode::Cloturee => None,
        _ => Some(
            echeance_de_cloture(exercice, p, crate::base::referentiel::delai_cloture())
                .map_err(|_| ErreurApi::Interne(CategorieErreur::Donnee))?
                .to_string(),
        ),
    };
    Ok(VuePeriode {
        id: id.to_string(),
        debut: p.debut().to_string(),
        fin: p.fin().to_string(),
        statut: nom_statut_periode(p.statut()),
        echeance_cloture: echeance,
    })
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
        .map(|p| vue_periode(id_periode(p.debut()).ok_or(interne)?, exercice, p))
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

/// Exécute un cas d'usage du module dans la transaction du pipeline.
pub(crate) async fn executer(
    cas: Cas,
    tx: &mut TransactionTenant,
    garde: &Garde,
    requete: &Requete<'_>,
) -> Result<Resultat, ErreurApi> {
    let dossier = dossier_visible(tx, garde).await?;
    match cas {
        Cas::ListerComptes | Cas::ListerJournaux | Cas::ListerExercices => lire(cas, tx).await,
        Cas::CreerCompte
        | Cas::RenommerCompte
        | Cas::DesactiverCompte
        | Cas::ReactiverCompte
        | Cas::CreerJournal => {
            exiger_parametreur(tx).await?;
            parametrer(cas, tx, dossier.id(), requete).await
        }
        Cas::VerrouillerPeriode
        | Cas::DeverrouillerPeriode
        | Cas::CloturerPeriode
        | Cas::OuvrirExerciceSuivant => {
            periodes::executer(cas, tx, garde, dossier.id(), requete).await
        }
    }
}

/// Lectures du référentiel du dossier du chemin.
async fn lire(cas: Cas, tx: &mut TransactionTenant) -> Result<Resultat, ErreurApi> {
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

/// Ressource à rendre après une mutation.
enum Rendu {
    Compte(NumeroCompte),
    Journal(CodeJournal),
}

/// Mutation pure d'une route de paramétrage, appliquée par [`muter`] à une copie de l'état.
async fn parametrer(
    cas: Cas,
    tx: &mut TransactionTenant,
    dossier: IdDossier,
    requete: &Requete<'_>,
) -> Result<Resultat, ErreurApi> {
    let statut = match cas {
        Cas::CreerCompte | Cas::CreerJournal => StatusCode::CREATED,
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
            let id = id_chemin(requete.chemin, "compte")?;
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
        Cas::ListerComptes
        | Cas::ListerJournaux
        | Cas::ListerExercices
        | Cas::VerrouillerPeriode
        | Cas::DeverrouillerPeriode
        | Cas::CloturerPeriode
        | Cas::OuvrirExerciceSuivant => {
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
    };
    Ok(Resultat { reponse, traces })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn jour() -> OffsetDateTime {
        Date::from_calendar_date(2026, Month::November, 19)
            .unwrap()
            .midnight()
            .assume_utc()
    }

    #[test]
    fn corps_de_creation() {
        for valide in [
            r#"{"nom":"A","pays":"SN","nature":"reprise","annee":2026}"#,
            r#"{"nom":"A","pays":"SN","nature":"creation","debut_activite":"2026-09-01","fin_premier_exercice":"annee_suivante"}"#,
            r#"{"nom":"A","pays":"SN","nature":"creation","debut_activite":"2027-01-01","fin_premier_exercice":"annee_de_debut"}"#,
        ] {
            assert!(lire_creation(valide.as_bytes(), jour()).is_ok(), "{valide}");
        }
        for invalide in [
            r#"{"nom":"A","pays":"SN"}"#,
            r#"{"nom":"A","pays":"SN","nature":"reprise","annee":2027}"#,
            r#"{"nom":"A","pays":"SN","nature":"reprise"}"#,
            r#"{"nom":"A","pays":"SN","nature":"reprise","annee":2026,"debut_activite":"2026-01-01"}"#,
            r#"{"nom":"A","pays":"SN","nature":"creation","debut_activite":"2026-03-01","fin_premier_exercice":"annee_suivante"}"#,
            r#"{"nom":"A","pays":"SN","nature":"creation","debut_activite":"2026-3-01","fin_premier_exercice":"annee_de_debut"}"#,
            r#"{"nom":"A","pays":"SN","nature":"creation","debut_activite":"2026-02-30","fin_premier_exercice":"annee_de_debut"}"#,
            r#"{"nom":"A","pays":"SN","nature":"creation","debut_activite":"2026-02-01","fin_premier_exercice":"autre"}"#,
            r#"{"nom":"A","pays":"SN","nature":"creation","debut_activite":"2026-02-01"}"#,
            r#"{"nom":"A","pays":"SN","nature":"autre","annee":2026}"#,
            r#"{"nom":"A","pays":"SN","nature":"reprise","annee":2026,"autre":1}"#,
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
    fn contexte_de_creation() {
        let ok = |corps: &str| lire_creation(corps.as_bytes(), jour()).unwrap();
        // Valeurs par défaut du pays (RDC : CDF, deux décimales) et identifiants du pays.
        let d = ok(
            r#"{"nom":"A","pays":"CD","nature":"reprise","annee":2026,"forme_juridique":"SARL","identifiants_legaux":[{"type":"RCCM","valeur":"CD/KIN/RCCM/20-B-1"}]}"#,
        );
        assert_eq!(d.contexte().devise, "CDF");
        assert_eq!(d.contexte().decimales, 2);
        assert_eq!(d.contexte().fuseau, "Africa/Kinshasa");
        assert_eq!(d.identifiants().len(), 1);
        // Valeurs fournies.
        let d = ok(
            r#"{"nom":"A","pays":"SN","devise":"EUR","fuseau":"Africa/Douala","langue":"fr-SN","nature":"reprise","annee":2026}"#,
        );
        assert_eq!(
            (d.contexte().devise.as_str(), d.contexte().decimales),
            ("EUR", 2)
        );
        // Pays obligatoire (400), inconnu, devise, fuseau, forme, identifiant : 422 stables.
        let refus = |corps: &str| lire_creation(corps.as_bytes(), jour()).unwrap_err();
        assert_eq!(
            refus(r#"{"nom":"A","nature":"reprise","annee":2026}"#),
            ErreurApi::RequeteInvalide
        );
        for (corps, motif) in [
            (
                r#"{"nom":"A","pays":"FR","nature":"reprise","annee":2026}"#,
                MotifValidation::PaysInconnu,
            ),
            (
                r#"{"nom":"A","pays":"SN","devise":"ZZZ","nature":"reprise","annee":2026}"#,
                MotifValidation::DeviseTenueInconnue,
            ),
            (
                r#"{"nom":"A","pays":"SN","fuseau":"Europe/Paris","nature":"reprise","annee":2026}"#,
                MotifValidation::FuseauInconnu,
            ),
            (
                r#"{"nom":"A","pays":"SN","langue":"FR","nature":"reprise","annee":2026}"#,
                MotifValidation::ContexteDossierInvalide,
            ),
            (
                r#"{"nom":"A","pays":"SN","forme_juridique":"","nature":"reprise","annee":2026}"#,
                MotifValidation::ContexteDossierInvalide,
            ),
            (
                r#"{"nom":"A","pays":"CM","identifiants_legaux":[{"type":"NINEA","valeur":"1"}],"nature":"reprise","annee":2026}"#,
                MotifValidation::ContexteDossierInvalide,
            ),
        ] {
            assert_eq!(
                refus(corps),
                ErreurApi::Validation { motif, ligne: None },
                "{corps}"
            );
        }
        // Un champ inconnu reste refusé (400).
        assert_eq!(
            refus(r#"{"nom":"A","pays":"SN","colonne_ninea":"1","nature":"reprise","annee":2026}"#),
            ErreurApi::RequeteInvalide
        );
    }

    #[test]
    fn date_d_ouverture_dans_le_fuseau_du_pays() {
        // 23 h 30 UTC le 31 décembre 2026 : déjà 2027 au Cameroun (UTC+1), encore 2026 au Sénégal.
        // Reprise de l'année 2027 : seulement acceptée si « aujourd'hui » est en 2027 ou après ?
        // On ne vérifie ici que la date retenue par la demande : début d'activité du 1er janvier
        // 2027, admis dès le 1er janvier 2027 au Cameroun et pas encore au Sénégal.
        let instant = Date::from_calendar_date(2026, Month::December, 31)
            .unwrap()
            .with_hms(23, 30, 0)
            .unwrap()
            .assume_utc();
        let corps = |pays: &str| {
            format!(r#"{{"nom":"A","pays":"{pays}","nature":"reprise","annee":2027}}"#)
        };
        assert!(lire_creation(corps("CM").as_bytes(), instant).is_ok());
        assert_eq!(
            lire_creation(corps("SN").as_bytes(), instant),
            Err(ErreurApi::RequeteInvalide)
        );
    }
}
