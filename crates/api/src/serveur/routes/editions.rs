//! Routes des éditions et des alertes (PR 14 de `docs/lots/L2.md`, L2-28 et L2-29) : balance
//! générale (6 colonnes, AUDCIF art. 19) et balances de travail (2, 4 et 8 colonnes), grand
//! livre, journaux, centralisateur, alertes. **Lectures seules.**
//!
//! Le calcul est celui de `compta_core::editions` : l'API relit les écritures validées (et, en
//! mode `provisoire`, le brouillard équilibré) de l'exercice jusqu'à la fin de la période, dans
//! **un seul instantané** (`REPEATABLE READ`, lecture seule), et ne calcule rien elle-même.
//!
//! # Paramètres
//! `exercice` (code ; par défaut celui qui contient `au` ou `du`, sinon le dernier), `du` et
//! `au` (`AAAA-MM-JJ`, dans l'exercice ; par défaut ses bornes), `mode` (`definitif`, par
//! défaut, ou `provisoire`), `comptes` et `journaux` (listes séparées par des virgules, 50 au
//! plus), et pour la balance `format` (2, 4, 6 par défaut, 8) et `regroupement` (`compte`, par
//! défaut, ou `tronc`). Tout autre paramètre, un paramètre répété ou une valeur invalide : 400.
//! Plus de `editions_ecritures_max` écritures (par défaut [`crate::base::editions::ECRITURES_MAX`]) sur la plage : 409
//! `edition_trop_volumineuse`. Seules les écritures de l'**exercice** comptent : une écriture
//! antérieure à son premier jour n'entre ni dans ses mouvements ni dans son ouverture ; les
//! à-nouveaux sont les écritures d'un journal d'à-nouveaux de l'exercice.
//!
//! # Brouillard non pris en compte (mode provisoire)
//! Un brouillon déséquilibré ou illisible (donnée stockée que les règles actuelles ne relisent
//! plus, par exemple un régime fiscal retiré d'une version du référentiel) n'entre pas dans une
//! édition provisoire : il ne la fait **jamais** échouer, et la réponse le **signale**
//! (`brouillons_exclus`).
//!
//! # Défense en profondeur (ADR 0011)
//! Le dossier est relu **visible** (affectation, sauf administrateur) ; toute lecture filtre par
//! le cabinet et le dossier du contexte, sous RLS. Les quatre rôles du cabinet consultent les
//! éditions (§20) : l'auditeur est autorisé.
//!
//! # Alertes
//! Brouillard non validé : une alerte par (période, journal) dont la période est terminée
//! (`compta_core::brouillard_en_retard`, AUDCIF art. 22 2° : une écriture est en retard dès le
//! lendemain de la fin de sa période). Échéance de clôture : une alerte par période terminée et
//! non clôturée dont l'échéance (`Exercices::echeances_de_cloture`) tombe dans les
//! [`PREAVIS_ECHEANCE`] jours, ou est dépassée (point 15).

use authz::Action;
use axum::Json;
use axum::response::IntoResponse;
use compta_core::{
    Balance, Centralisateur, CodeExercice, CodeJournal, EcritureEquilibree, EcritureValidee,
    ErreurEdition, Exercice, Exercices, Filtre, FormatBalance, Mode, NumeroCompte, Regroupement,
    StatutPeriode, brouillard_en_retard, grand_livre, journaux,
};
use serde::Serialize;
use time::Date;

use super::brouillard::date_lue;
use super::brouillard::validation::{
    ColonnesValidee, equilibree_du_brouillon, reconstituer_colonnes,
};
use super::{ActionRoute, CasUsage, Methode, Module, Portee, Requete, Resultat, Route};
use crate::base::brouillard::decimal_canonique;
use crate::base::editions::{brouillons_entre, brouillons_par_journal_et_date, validees_entre};
use crate::base::organisation::{lire_contexte_du_dossier, lire_dossier_visible};
use crate::base::referentiel::charger;
use crate::base::{TransactionTenant, ouvrir_transaction_lecture};
use crate::parametres_fiscaux::{ParametresDuDossier, lire_propres};
use crate::pieces::interne;
use crate::serveur::erreur::{CategorieErreur, ErreurApi, MotifConflit};
use crate::serveur::pipeline::Garde;

/// Cas d'usage du module.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cas {
    /// Balance générale ou de travail.
    Balance,
    /// Grand livre général.
    GrandLivre,
    /// Journaux de la période.
    Journaux,
    /// Centralisateur des journaux (AUDCIF art. 19).
    Centralisateur,
    /// Alertes du dossier.
    Alertes,
}

/// Tranche du module.
pub const MODULE: Module = Module {
    nom: "editions",
    routes: ROUTES,
    champs_journal: &[],
};

const fn route(chemin: &'static str, cas: Cas) -> Route {
    Route {
        methode: Methode::Get,
        chemin,
        // §20 « Consulter les éditions » : les quatre rôles du cabinet.
        action: ActionRoute::Cabinet(Action::ConsulterEditions),
        portee: Portee::Dossier,
        cas_usage: CasUsage::Editions(cas),
    }
}

/// Routes du module.
pub const ROUTES: &[Route] = &[
    route(
        "/api/cabinets/{cabinet}/dossiers/{dossier}/editions/balance",
        Cas::Balance,
    ),
    route(
        "/api/cabinets/{cabinet}/dossiers/{dossier}/editions/grand-livre",
        Cas::GrandLivre,
    ),
    route(
        "/api/cabinets/{cabinet}/dossiers/{dossier}/editions/journaux",
        Cas::Journaux,
    ),
    route(
        "/api/cabinets/{cabinet}/dossiers/{dossier}/editions/centralisateur",
        Cas::Centralisateur,
    ),
    route(
        "/api/cabinets/{cabinet}/dossiers/{dossier}/alertes",
        Cas::Alertes,
    ),
];

/// Jours avant l'échéance de clôture à partir desquels l'alerte est émise.
pub const PREAVIS_ECHEANCE: i64 = 30;

/// Nombre maximal d'éléments d'une liste de filtre.
const LISTE_MAX: usize = 50;

/// Exécute un cas d'usage du module dans la transaction du pipeline.
pub(crate) async fn executer(
    cas: Cas,
    tx: &mut TransactionTenant,
    garde: &Garde,
    requete: &Requete<'_>,
) -> Result<Resultat, ErreurApi> {
    // Le dossier du chemin, relu visible (affectation, sauf administrateur).
    let id = tx.contexte().id_dossier().ok_or(ErreurApi::Introuvable)?;
    let dossier = lire_dossier_visible(tx, id)
        .await?
        .ok_or(ErreurApi::Introuvable)?;
    garde.dossier_du_chemin(dossier)?;
    match cas {
        Cas::Alertes => alertes(tx, requete).await,
        _ => edition(cas, tx, requete).await,
    }
}

// --- Paramètres -------------------------------------------------------------------------

#[derive(Debug, Default)]
struct Parametres {
    exercice: Option<CodeExercice>,
    du: Option<Date>,
    au: Option<Date>,
    provisoire: bool,
    format: Option<FormatBalance>,
    regroupement: Option<Regroupement>,
    comptes: Vec<NumeroCompte>,
    journaux: Vec<CodeJournal>,
}

fn liste<T>(valeur: &str, lire: impl Fn(&str) -> Option<T>) -> Result<Vec<T>, ErreurApi> {
    let elements: Vec<T> = valeur
        .split(',')
        .map(lire)
        .collect::<Option<_>>()
        .ok_or(ErreurApi::RequeteInvalide)?;
    if elements.is_empty() || elements.len() > LISTE_MAX {
        return Err(ErreurApi::RequeteInvalide);
    }
    Ok(elements)
}

/// Lit les paramètres, en n'admettant que `admis` ; une clé répétée ou inconnue est refusée.
fn lire_parametres(texte: Option<&str>, admis: &[&str]) -> Result<Parametres, ErreurApi> {
    let invalide = ErreurApi::RequeteInvalide;
    let mut p = Parametres::default();
    let mut vues: Vec<&str> = Vec::new();
    for paire in texte.unwrap_or("").split('&').filter(|x| !x.is_empty()) {
        let (cle, valeur) = paire.split_once('=').ok_or(invalide)?;
        if !admis.contains(&cle) || vues.contains(&cle) {
            return Err(invalide);
        }
        vues.push(cle);
        match cle {
            "exercice" => {
                p.exercice = Some(CodeExercice::parse(valeur).map_err(|_| invalide)?);
            }
            "du" => p.du = Some(date_lue(valeur).ok_or(invalide)?),
            "au" => p.au = Some(date_lue(valeur).ok_or(invalide)?),
            "mode" => {
                p.provisoire = match valeur {
                    "definitif" => false,
                    "provisoire" => true,
                    _ => return Err(invalide),
                };
            }
            "format" => {
                p.format = Some(match valeur {
                    "2" => FormatBalance::Deux,
                    "4" => FormatBalance::Quatre,
                    "6" => FormatBalance::Six,
                    "8" => FormatBalance::Huit,
                    _ => return Err(invalide),
                });
            }
            "regroupement" => {
                p.regroupement = Some(match valeur {
                    "compte" => Regroupement::ParCompte,
                    "tronc" => Regroupement::ParTronc,
                    _ => return Err(invalide),
                });
            }
            "comptes" => p.comptes = liste(valeur, |c| NumeroCompte::parse(c).ok())?,
            "journaux" => p.journaux = liste(valeur, |j| CodeJournal::parse(j).ok())?,
            _ => return Err(invalide),
        }
    }
    Ok(p)
}

fn parametres_du_cas(cas: Cas, texte: Option<&str>) -> Result<Parametres, ErreurApi> {
    let communs = ["exercice", "du", "au", "mode", "comptes", "journaux"];
    match cas {
        Cas::Balance => {
            lire_parametres(texte, &[&communs[..], &["format", "regroupement"]].concat())
        }
        // Une écriture figure toujours en entier dans son journal : pas de filtre de comptes.
        Cas::Journaux => lire_parametres(texte, &["exercice", "du", "au", "mode", "journaux"]),
        _ => lire_parametres(texte, &communs),
    }
}

// --- Lecture des données ------------------------------------------------------------------

struct Donnees {
    exercice: CodeExercice,
    du: Date,
    au: Date,
    mode: Mode,
    validees: Vec<EcritureValidee>,
    brouillard: Vec<EcritureEquilibree>,
    /// Brouillons écartés du mode provisoire : déséquilibrés, illisibles.
    exclus: Exclus,
    filtre: Filtre,
}

/// Brouillons que le mode provisoire n'a pas pu prendre en compte.
#[derive(Debug, Default, Clone, Copy, Serialize)]
struct Exclus {
    desequilibres: usize,
    illisibles: usize,
}

fn donnee() -> ErreurApi {
    ErreurApi::Interne(CategorieErreur::Donnee)
}

/// Exercice demandé, ou celui de `au`/`du`, ou le dernier.
fn choisir_exercice<'a>(
    exercices: &'a Exercices,
    p: &Parametres,
) -> Result<&'a Exercice, ErreurApi> {
    if let Some(code) = &p.exercice {
        return exercices.exercice(code).ok_or(ErreurApi::RequeteInvalide);
    }
    if let Some(date) = p.au.or(p.du) {
        return exercices
            .exercice_contenant(date)
            .ok_or(ErreurApi::RequeteInvalide);
    }
    exercices
        .exercices()
        .last()
        .ok_or(ErreurApi::RequeteInvalide)
}

fn erreur_edition(e: &ErreurEdition) -> ErreurApi {
    match e {
        ErreurEdition::BornesInversees { .. } | ErreurEdition::HorsExercice(_) => {
            ErreurApi::RequeteInvalide
        }
        ErreurEdition::Depassement => donnee(),
    }
}

async fn lire_donnees(
    tx: &TransactionTenant,
    requete: &Requete<'_>,
    p: &Parametres,
) -> Result<Donnees, ErreurApi> {
    let registre = requete.registre_fiscal()?;
    let mut instantane = ouvrir_transaction_lecture(requete.pool, *tx.contexte()).await?;
    let lu = lire_dans_instantane(&mut instantane, p, registre, requete.editions_max).await;
    instantane.annuler().await?;
    lu
}

async fn lire_dans_instantane(
    instantane: &mut TransactionTenant,
    p: &Parametres,
    registre: &fiscal::Registre,
    borne: i64,
) -> Result<Donnees, ErreurApi> {
    let referentiel = charger(instantane).await?.ok_or(ErreurApi::Introuvable)?;
    let contexte = lire_contexte_du_dossier(instantane).await?;
    let tenue = contexte.tenue()?;
    let propres = lire_propres(instantane, registre).await?;
    let fiscal = ParametresDuDossier::nouveau(registre, &contexte.pays, &propres);
    let exercice = choisir_exercice(referentiel.exercices(), p)?;
    let du = p.du.unwrap_or_else(|| exercice.debut());
    let au = p.au.unwrap_or_else(|| exercice.fin());
    let mut filtre = Filtre::nouveau(exercice, &referentiel.etat().journaux, du, au)
        .map_err(|e| erreur_edition(&e))?;
    if !p.comptes.is_empty() {
        filtre = filtre.comptes(p.comptes.clone());
    }
    if !p.journaux.is_empty() {
        filtre = filtre.journaux(p.journaux.clone());
    }
    // L'exercice jusqu'à `au` : les écritures antérieures à la période alimentent l'ouverture.
    let lues = validees_entre(instantane, exercice.debut(), au, borne).await?;
    if lues.depasse {
        return Err(trop_volumineuse(borne));
    }
    let mut validees = Vec::with_capacity(lues.elements.len());
    for e in &lues.elements {
        validees.push(
            reconstituer_colonnes(
                &ColonnesValidee {
                    numero: &e.numero,
                    journal: &e.journal,
                    date: e.date,
                    date_valeur_comptable: e.date_valeur_comptable,
                    libelle: &e.libelle,
                    version_parametres: e.version_parametres.as_deref(),
                    date_validation: e.date_validation,
                    origine_numero: e.origine_numero.as_deref(),
                    lignes: &e.lignes,
                },
                &e.empreintes,
                &fiscal,
                &tenue,
            )
            .map_err(|_| donnee())?,
        );
    }
    let mut brouillard = Vec::new();
    let mut exclus = Exclus::default();
    if p.provisoire {
        let brouillons = brouillons_entre(instantane, exercice.debut(), au, borne).await?;
        if brouillons.depasse {
            return Err(trop_volumineuse(borne));
        }
        for b in &brouillons.elements {
            // Le brouillard déséquilibré ou illisible n'entre pas dans une édition provisoire : il
            // est compté et signalé, jamais cause d'échec (on ne peut pas l'atteindre par la
            // saisie, mais une version du référentiel peut avoir retiré une valeur stockée).
            match equilibree_du_brouillon(b, &fiscal, &tenue) {
                Ok(Some(e)) => brouillard.push(e),
                Ok(None) => exclus.desequilibres += 1,
                Err(_) => exclus.illisibles += 1,
            }
        }
    }
    Ok(Donnees {
        exercice: exercice.code().clone(),
        du,
        au,
        mode: if p.provisoire {
            Mode::Provisoire
        } else {
            Mode::Definitif
        },
        validees,
        brouillard,
        exclus,
        filtre,
    })
}

fn trop_volumineuse(borne: i64) -> ErreurApi {
    ErreurApi::Conflit {
        motif: MotifConflit::EditionTropVolumineuse,
        nombre: u32::try_from(borne).ok(),
    }
}

// --- Vues -------------------------------------------------------------------------------

#[allow(clippy::needless_pass_by_value)] // Passé tel quel à `map` sur des valeurs décimales.
fn dec(d: impl ToString) -> String {
    decimal_canonique(&d.to_string())
}

fn jour(d: Date) -> String {
    interne::date(d)
}

#[derive(Debug, Serialize)]
struct Entete {
    exercice: String,
    du: String,
    au: String,
    mode: &'static str,
    /// Brouillons que le mode provisoire n'a pas pu prendre en compte (zéros en définitif).
    brouillons_exclus: Exclus,
}

fn entete(d: &Donnees) -> Entete {
    Entete {
        exercice: d.exercice.as_str().to_owned(),
        du: jour(d.du),
        au: jour(d.au),
        mode: match d.mode {
            Mode::Definitif => "definitif",
            Mode::Provisoire => "provisoire",
        },
        brouillons_exclus: d.exclus,
    }
}

#[allow(clippy::unnecessary_wraps)] // Même forme de retour que les autres cas d'usage.
fn reponse<T: Serialize>(vue: T) -> Result<Resultat, ErreurApi> {
    Ok(Resultat::lecture(Json(vue).into_response()))
}

const fn noms_colonnes(format: FormatBalance) -> &'static [&'static str] {
    match format {
        FormatBalance::Deux => &["solde_debiteur", "solde_crediteur"],
        FormatBalance::Quatre => &[
            "total_debit",
            "total_credit",
            "solde_debiteur",
            "solde_crediteur",
        ],
        FormatBalance::Six => &[
            "ouverture_debit",
            "ouverture_credit",
            "mouvements_debit",
            "mouvements_credit",
            "solde_debiteur",
            "solde_crediteur",
        ],
        FormatBalance::Huit => &[
            "ouverture_debit",
            "ouverture_credit",
            "mouvements_debit",
            "mouvements_credit",
            "total_debit",
            "total_credit",
            "solde_debiteur",
            "solde_crediteur",
        ],
    }
}

#[derive(Debug, Serialize)]
struct VueLigneBalance {
    cle: String,
    valeurs: Vec<String>,
}

#[derive(Debug, Serialize)]
struct VueBalance {
    #[serde(flatten)]
    entete: Entete,
    format: u8,
    regroupement: &'static str,
    colonnes: &'static [&'static str],
    lignes: Vec<VueLigneBalance>,
    totaux: Vec<String>,
    /// Totaux des colonnes débit égaux aux totaux des colonnes crédit (deux à deux).
    equilibree: bool,
}

fn balance(d: &Donnees, p: &Parametres) -> Result<Resultat, ErreurApi> {
    let format = p.format.unwrap_or(FormatBalance::Six);
    let regroupement = p.regroupement.unwrap_or(Regroupement::ParCompte);
    let b = Balance::calculer(&d.validees, &d.brouillard, d.mode, &d.filtre, regroupement)
        .map_err(|e| erreur_edition(&e))?;
    let totaux = b.totaux(format).map_err(|e| erreur_edition(&e))?;
    let equilibree = totaux.chunks(2).all(|c| c.first() == c.get(1));
    let mut lignes = Vec::with_capacity(b.lignes().len());
    for l in b.lignes() {
        lignes.push(VueLigneBalance {
            cle: l.cle().to_owned(),
            valeurs: l
                .colonnes(format)
                .map_err(|e| erreur_edition(&e))?
                .into_iter()
                .map(dec)
                .collect(),
        });
    }
    reponse(VueBalance {
        entete: entete(d),
        format: match format {
            FormatBalance::Deux => 2,
            FormatBalance::Quatre => 4,
            FormatBalance::Six => 6,
            FormatBalance::Huit => 8,
        },
        regroupement: match regroupement {
            Regroupement::ParCompte => "compte",
            Regroupement::ParTronc => "tronc",
        },
        colonnes: noms_colonnes(format),
        lignes,
        totaux: totaux.into_iter().map(dec).collect(),
        equilibree,
    })
}

#[derive(Debug, Serialize)]
struct VueLigneGrandLivre {
    date: String,
    journal: String,
    numero: Option<String>,
    date_valeur_comptable: Option<String>,
    libelle: String,
    tiers: Option<String>,
    debit: String,
    credit: String,
    solde: String,
}

#[derive(Debug, Serialize)]
struct VueCompteGrandLivre {
    compte: String,
    solde_ouverture: String,
    lignes: Vec<VueLigneGrandLivre>,
    total_debit: String,
    total_credit: String,
    solde_final: String,
}

#[derive(Debug, Serialize)]
struct VueGrandLivre {
    #[serde(flatten)]
    entete: Entete,
    comptes: Vec<VueCompteGrandLivre>,
}

fn grand_livre_vue(d: &Donnees) -> Result<Resultat, ErreurApi> {
    let comptes = grand_livre(&d.validees, &d.brouillard, d.mode, &d.filtre)
        .map_err(|e| erreur_edition(&e))?;
    reponse(VueGrandLivre {
        entete: entete(d),
        comptes: comptes
            .into_iter()
            .map(|c| VueCompteGrandLivre {
                compte: c.compte.as_str().to_owned(),
                solde_ouverture: dec(c.solde_ouverture),
                lignes: c
                    .lignes
                    .into_iter()
                    .map(|l| VueLigneGrandLivre {
                        date: jour(l.date),
                        journal: l.journal.as_str().to_owned(),
                        numero: l.numero.map(|n| n.to_string()),
                        date_valeur_comptable: l.date_valeur_comptable.map(jour),
                        libelle: l.libelle.as_str().to_owned(),
                        tiers: l.tiers.map(|t| t.as_str().to_owned()),
                        debit: dec(l.debit),
                        credit: dec(l.credit),
                        solde: dec(l.solde),
                    })
                    .collect(),
                total_debit: dec(c.total_debit),
                total_credit: dec(c.total_credit),
                solde_final: dec(c.solde_final),
            })
            .collect(),
    })
}

#[derive(Debug, Serialize)]
struct VueLigneJournal {
    compte: String,
    debit: String,
    credit: String,
    tiers: Option<String>,
}

#[derive(Debug, Serialize)]
struct VueEcritureJournal {
    numero: Option<String>,
    date: String,
    date_valeur_comptable: Option<String>,
    libelle: String,
    lignes: Vec<VueLigneJournal>,
}

#[derive(Debug, Serialize)]
struct VueJournal {
    journal: String,
    ecritures: Vec<VueEcritureJournal>,
    total_debit: String,
    total_credit: String,
}

#[derive(Debug, Serialize)]
struct VueJournaux {
    #[serde(flatten)]
    entete: Entete,
    journaux: Vec<VueJournal>,
}

fn journaux_vue(d: &Donnees) -> Result<Resultat, ErreurApi> {
    let blocs =
        journaux(&d.validees, &d.brouillard, d.mode, &d.filtre).map_err(|e| erreur_edition(&e))?;
    reponse(VueJournaux {
        entete: entete(d),
        journaux: blocs
            .into_iter()
            .map(|b| VueJournal {
                journal: b.journal.as_str().to_owned(),
                ecritures: b
                    .ecritures
                    .into_iter()
                    .map(|e| VueEcritureJournal {
                        numero: e.numero.map(|n| n.to_string()),
                        date: jour(e.date),
                        date_valeur_comptable: e.date_valeur_comptable.map(jour),
                        libelle: e.libelle.as_str().to_owned(),
                        lignes: e
                            .lignes
                            .iter()
                            .map(|l| VueLigneJournal {
                                compte: l.compte().as_str().to_owned(),
                                debit: dec(l.debit()),
                                credit: dec(l.credit()),
                                tiers: l.tiers().map(|t| t.as_str().to_owned()),
                            })
                            .collect(),
                    })
                    .collect(),
                total_debit: dec(b.total_debit),
                total_credit: dec(b.total_credit),
            })
            .collect(),
    })
}

#[derive(Debug, Serialize)]
struct VueMois {
    annee: i32,
    mois: u8,
    journal: Option<String>,
    compte: Option<String>,
    debit: String,
    credit: String,
}

#[derive(Debug, Serialize)]
struct VueRubrique {
    /// Une ligne par (mois, journal).
    par_journal: Vec<VueMois>,
    /// Détail par (mois, journal, compte).
    par_compte: Vec<VueMois>,
    /// Sous-totaux par mois.
    par_mois: Vec<VueMois>,
    total_debit: String,
    total_credit: String,
}

#[derive(Debug, Serialize)]
struct VueCentralisateur {
    #[serde(flatten)]
    entete: Entete,
    mouvements: VueRubrique,
    a_nouveaux: VueRubrique,
}

fn rubrique(r: &compta_core::RubriqueCentralisateur) -> VueRubrique {
    let (total_debit, total_credit) = r.total();
    VueRubrique {
        par_journal: r
            .lignes()
            .iter()
            .map(|l| VueMois {
                annee: l.mois.annee(),
                mois: u8::from(l.mois.mois()),
                journal: Some(l.journal.as_str().to_owned()),
                compte: None,
                debit: dec(l.debit),
                credit: dec(l.credit),
            })
            .collect(),
        par_compte: r
            .lignes_par_compte()
            .iter()
            .map(|l| VueMois {
                annee: l.mois.annee(),
                mois: u8::from(l.mois.mois()),
                journal: Some(l.journal.as_str().to_owned()),
                compte: Some(l.compte.as_str().to_owned()),
                debit: dec(l.debit),
                credit: dec(l.credit),
            })
            .collect(),
        par_mois: r
            .totaux_par_mois()
            .iter()
            .map(|t| VueMois {
                annee: t.mois.annee(),
                mois: u8::from(t.mois.mois()),
                journal: None,
                compte: None,
                debit: dec(t.debit),
                credit: dec(t.credit),
            })
            .collect(),
        total_debit: dec(total_debit),
        total_credit: dec(total_credit),
    }
}

fn centralisateur(d: &Donnees) -> Result<Resultat, ErreurApi> {
    let c = Centralisateur::calculer(&d.validees, &d.brouillard, d.mode, &d.filtre)
        .map_err(|e| erreur_edition(&e))?;
    reponse(VueCentralisateur {
        entete: entete(d),
        mouvements: rubrique(c.mouvements()),
        a_nouveaux: rubrique(c.a_nouveaux()),
    })
}

async fn edition(
    cas: Cas,
    tx: &mut TransactionTenant,
    requete: &Requete<'_>,
) -> Result<Resultat, ErreurApi> {
    let p = parametres_du_cas(cas, requete.parametres)?;
    let d = lire_donnees(tx, requete, &p).await?;
    match cas {
        Cas::Balance => balance(&d, &p),
        Cas::GrandLivre => grand_livre_vue(&d),
        Cas::Journaux => journaux_vue(&d),
        _ => centralisateur(&d),
    }
}

// --- Alertes ----------------------------------------------------------------------------

#[derive(Debug, Serialize)]
struct VueAlerteBrouillard {
    journal: String,
    debut_periode: String,
    fin_periode: String,
    nombre: usize,
    plus_ancienne: String,
    jours_de_retard: i64,
}

#[derive(Debug, Serialize)]
struct VueAlerteEcheance {
    exercice: String,
    debut: String,
    fin: String,
    echeance: String,
    /// Négatif : l'échéance est dépassée.
    jours_restants: i64,
    depassee: bool,
    statut: &'static str,
}

#[derive(Debug, Serialize)]
struct VueAlertes {
    date_du_jour: String,
    brouillard_non_valide: Vec<VueAlerteBrouillard>,
    echeances_de_cloture: Vec<VueAlerteEcheance>,
}

async fn alertes(tx: &mut TransactionTenant, requete: &Requete<'_>) -> Result<Resultat, ErreurApi> {
    if requete.parametres.is_some_and(|p| !p.is_empty()) {
        return Err(ErreurApi::RequeteInvalide);
    }
    // Date du jour : l'horloge de la requête (jamais dans `compta-core`), dans le fuseau du
    // dossier (ADR 0027).
    let aujourdhui = lire_contexte_du_dossier(tx)
        .await?
        .date_du_jour(requete.maintenant)?;
    let mut instantane = ouvrir_transaction_lecture(requete.pool, *tx.contexte()).await?;
    let lu = async {
        let referentiel = charger(&mut instantane)
            .await?
            .ok_or(ErreurApi::Introuvable)?;
        // Regroupés par (journal, jour) par PostgreSQL : le volume lu ne dépend pas du nombre
        // de brouillons.
        let groupes = brouillons_par_journal_et_date(&mut instantane, requete.editions_max).await?;
        Ok::<_, ErreurApi>((referentiel, groupes))
    }
    .await;
    instantane.annuler().await?;
    let (referentiel, groupes) = lu?;
    if groupes.depasse {
        return Err(trop_volumineuse(requete.editions_max));
    }
    let exercices = referentiel.exercices();
    let codes: Vec<(CodeJournal, Date, usize)> = groupes
        .elements
        .into_iter()
        .map(|(j, d, n)| Ok((CodeJournal::parse(&j).map_err(|_| donnee())?, d, n)))
        .collect::<Result<_, ErreurApi>>()?;
    let brouillard_non_valide = brouillard_en_retard(
        codes
            .iter()
            .flat_map(|(j, d, n)| std::iter::repeat_n((j, *d), *n)),
        exercices,
        aujourdhui,
    )
    .into_iter()
    .map(|a| VueAlerteBrouillard {
        journal: a.journal.as_str().to_owned(),
        debut_periode: jour(a.debut_periode),
        fin_periode: jour(a.fin_periode),
        nombre: a.nombre,
        plus_ancienne: jour(a.plus_ancienne),
        jours_de_retard: a.jours_de_retard,
    })
    .collect();
    let echeances = exercices
        .echeances_de_cloture(crate::base::referentiel::delai_cloture())
        .map_err(|_| donnee())?;
    let echeances_de_cloture = echeances
        .into_iter()
        // Période terminée, non clôturée, échéance proche ou dépassée.
        .filter(|e| e.fin < aujourdhui)
        .map(|e| {
            let restants = (e.echeance - aujourdhui).whole_days();
            (e, restants)
        })
        .filter(|(_, restants)| *restants <= PREAVIS_ECHEANCE)
        .map(|(e, restants)| VueAlerteEcheance {
            exercice: e.exercice.as_str().to_owned(),
            debut: jour(e.debut),
            fin: jour(e.fin),
            echeance: jour(e.echeance),
            jours_restants: restants,
            depassee: restants < 0,
            statut: match e.statut {
                StatutPeriode::Ouverte => "ouverte",
                StatutPeriode::Verrouillee => "verrouillee",
                StatutPeriode::Cloturee => "cloturee",
            },
        })
        .collect();
    reponse(VueAlertes {
        date_du_jour: jour(aujourdhui),
        brouillard_non_valide,
        echeances_de_cloture,
    })
}
