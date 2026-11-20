//! Routes du brouillard (PR 13a de `docs/lots/L2.md`, L2-25) : saisie, modification, suppression,
//! drapeau « à revoir », liens de pièces. **Aucune route ne valide** (PR 13b) et aucune ne touche
//! une écriture validée.
//!
//! Persistance : [`crate::base::brouillard`] ; règles de saisie : [`saisie`].
//!
//! # Défense en profondeur (ADR 0011)
//! Même si Cedar se trompait, chaque cas d'usage relit le dossier **visible** (affectation, sauf
//! administrateur) et, pour une mutation, exige un rôle qui saisit (§20 : administrateur,
//! valideur, collaborateur) lu en base. Toute lecture filtre par le cabinet et le dossier du
//! contexte, sous RLS : le brouillon d'un autre dossier donne le même 404 qu'un brouillon
//! inexistant. Les routes d'un brouillon (portée [`Portee::Brouillon`]) sont décidées par Cedar
//! sur l'**écriture** (créateur et contributeurs lus en base), non sur le dossier.
//!
//! # Journal des modifications
//! Pas de table propre : le journal d'audit chaîné du dossier (ADR 0014) porte l'avant/après.
//! Une entrée pour l'en-tête (donnée `operation`, champs modifiés, `version`), plus une entrée
//! **par ligne changée** (champ `ligne`, donnée `ligne` = position) : les champs d'une entrée
//! ont des noms uniques et statiques. Les montants y figurent en clair (la chaîne est le
//! journal métier du dossier), le tiers par son identifiant seulement.
//!
//! # Verrou optimiste
//! `version` : un changement de contenu (en-tête, lignes, pièces) la porte, et est refusé (409
//! `brouillon_perime`) si elle a changé. Le drapeau n'est pas un contenu : il ne la change pas.

use authz::Action;
use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uuid::Uuid;

use super::{ActionRoute, CasUsage, Methode, Module, Portee, Requete, Resultat, Route};
use crate::base::TransactionTenant;
use crate::base::audit::PorteeAudit;
use crate::base::brouillard::{
    self, BrouillonStocke, ContenuStocke, FiltreBrouillard, LienPiece, LigneStockee, PieceALier,
    RefusSaisie, ResumeBrouillon, RolePiece, StatutVise,
};
use crate::base::organisation::{lire_appartenance, lire_dossier_visible};
use crate::base::referentiel::charger;
use crate::pieces::interne;
use crate::serveur::audit::Trace;
use crate::serveur::erreur::{CategorieErreur, ErreurApi, MotifConflit};
use crate::serveur::pipeline::{DossierAutorise, Garde, id_chemin};

mod saisie;

pub use saisie::referentiel_tva;

/// Cas d'usage du module.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cas {
    /// Créer un brouillon.
    Creer,
    /// Brouillons du dossier, paginés.
    Lister,
    /// Un brouillon, avec ses lignes, pièces et contributeurs.
    Lire,
    /// Remplacer le contenu d'un brouillon (avec sa version).
    Modifier,
    /// Supprimer un brouillon (avec sa version).
    Supprimer,
    /// Poser le drapeau « à revoir ».
    PoserARevoir,
    /// Lever le drapeau « à revoir ».
    LeverARevoir,
    /// Lier une pièce.
    LierPiece,
    /// Retirer le lien d'une pièce.
    RetirerPiece,
}

/// Tranche du module.
pub const MODULE: Module = Module {
    nom: "brouillard",
    routes: ROUTES,
    champs_journal: &[],
};

const BASE: &str = "/api/cabinets/{cabinet}/dossiers/{dossier}/brouillard";

const fn route(
    methode: Methode,
    chemin: &'static str,
    action: Action,
    portee: Portee,
    cas: Cas,
) -> Route {
    Route {
        methode,
        chemin,
        action: ActionRoute::Cabinet(action),
        portee,
        cas_usage: CasUsage::Brouillard(cas),
    }
}

/// Routes du module. Lectures : `AccederDossier` (les quatre rôles) ; création :
/// `SaisirBrouillard` (sur le dossier) ; le reste, `CorrigerBrouillard` sur l'**écriture**
/// (§20 « Saisir, corriger dans le brouillard » : administrateur, valideur, collaborateur).
pub const ROUTES: &[Route] = &[
    route(
        Methode::Post,
        BASE,
        Action::SaisirBrouillard,
        Portee::Dossier,
        Cas::Creer,
    ),
    route(
        Methode::Get,
        BASE,
        Action::AccederDossier,
        Portee::Dossier,
        Cas::Lister,
    ),
    route(
        Methode::Get,
        "/api/cabinets/{cabinet}/dossiers/{dossier}/brouillard/{brouillon}",
        Action::AccederDossier,
        Portee::Dossier,
        Cas::Lire,
    ),
    route(
        Methode::Post,
        "/api/cabinets/{cabinet}/dossiers/{dossier}/brouillard/{brouillon}/modification",
        Action::CorrigerBrouillard,
        Portee::Brouillon,
        Cas::Modifier,
    ),
    route(
        Methode::Post,
        "/api/cabinets/{cabinet}/dossiers/{dossier}/brouillard/{brouillon}/suppression",
        Action::CorrigerBrouillard,
        Portee::Brouillon,
        Cas::Supprimer,
    ),
    route(
        Methode::Post,
        "/api/cabinets/{cabinet}/dossiers/{dossier}/brouillard/{brouillon}/a-revoir",
        Action::CorrigerBrouillard,
        Portee::Brouillon,
        Cas::PoserARevoir,
    ),
    route(
        Methode::Post,
        "/api/cabinets/{cabinet}/dossiers/{dossier}/brouillard/{brouillon}/revue-faite",
        Action::CorrigerBrouillard,
        Portee::Brouillon,
        Cas::LeverARevoir,
    ),
    route(
        Methode::Post,
        "/api/cabinets/{cabinet}/dossiers/{dossier}/brouillard/{brouillon}/pieces",
        Action::CorrigerBrouillard,
        Portee::Brouillon,
        Cas::LierPiece,
    ),
    route(
        Methode::Post,
        "/api/cabinets/{cabinet}/dossiers/{dossier}/brouillard/{brouillon}/pieces/{piece}/retrait",
        Action::CorrigerBrouillard,
        Portee::Brouillon,
        Cas::RetirerPiece,
    ),
];

const PAGE_PAR_DEFAUT: u32 = 50;
const PAGE_MAX: u32 = 100;

/// Exécute un cas d'usage du module dans la transaction du pipeline.
pub(crate) async fn executer(
    cas: Cas,
    tx: &mut TransactionTenant,
    garde: &Garde,
    requete: &Requete<'_>,
) -> Result<Resultat, ErreurApi> {
    let dossier = dossier_autorise(tx, garde).await?;
    match cas {
        Cas::Lister => lister(tx, requete).await,
        Cas::Lire => lire(tx, requete).await,
        Cas::Creer => {
            exiger_saisisseur(tx).await?;
            creer(tx, &dossier, requete).await
        }
        _ => {
            exiger_saisisseur(tx).await?;
            let id = id_chemin(requete.chemin, "brouillon")?;
            // La décision Cedar a porté sur ce brouillon, et sur lui seul.
            if garde.brouillon_du_chemin() != Some(id) {
                return Err(ErreurApi::Introuvable);
            }
            match cas {
                Cas::Modifier => modifier(tx, &dossier, id, requete).await,
                Cas::Supprimer => supprimer(tx, &dossier, id, requete).await,
                Cas::PoserARevoir => drapeau(tx, &dossier, id, requete, true).await,
                Cas::LeverARevoir => drapeau(tx, &dossier, id, requete, false).await,
                Cas::LierPiece => lier(tx, &dossier, id, requete).await,
                _ => retirer(tx, &dossier, id, requete).await,
            }
        }
    }
}

/// Le dossier du chemin, relu **visible** (affectation, sauf administrateur).
async fn dossier_autorise(
    tx: &mut TransactionTenant,
    garde: &Garde,
) -> Result<DossierAutorise, ErreurApi> {
    let id = tx.contexte().id_dossier().ok_or(ErreurApi::Introuvable)?;
    let dossier = lire_dossier_visible(tx, id)
        .await?
        .ok_or(ErreurApi::Introuvable)?;
    garde.dossier_du_chemin(dossier)
}

/// Appartenance **active**, lue en base, d'un rôle qui saisit (§20). Sinon 403.
async fn exiger_saisisseur(tx: &mut TransactionTenant) -> Result<(), ErreurApi> {
    let appartenance = lire_appartenance(tx)
        .await?
        .filter(|a| a.actif)
        .ok_or(ErreurApi::Introuvable)?;
    let autorises = [
        authz::Role::Administrateur,
        authz::Role::Valideur,
        authz::Role::Collaborateur,
    ];
    if autorises.iter().any(|r| r.nom() == appartenance.role) {
        Ok(())
    } else {
        Err(ErreurApi::Interdit)
    }
}

/// Instant de l'horloge, à la microseconde (précision de PostgreSQL).
fn maintenant() -> OffsetDateTime {
    let i = OffsetDateTime::now_utc();
    i.replace_nanosecond(i.nanosecond() / 1000 * 1000)
        .unwrap_or(i)
}

fn corps<T: serde::de::DeserializeOwned>(octets: &[u8]) -> Result<T, ErreurApi> {
    Json::<T>::from_bytes(octets)
        .map(|Json(v)| v)
        .map_err(|_| ErreurApi::RequeteInvalide)
}

fn conflit(motif: MotifConflit) -> ErreurApi {
    ErreurApi::Conflit {
        motif,
        nombre: None,
    }
}

fn sans_parametres(requete: &Requete<'_>) -> Result<(), ErreurApi> {
    if requete.parametres.is_some_and(|p| !p.is_empty()) {
        Err(ErreurApi::RequeteInvalide)
    } else {
        Ok(())
    }
}

// --- Vues -------------------------------------------------------------------------------

#[derive(Debug, Serialize)]
struct VueLigne {
    position: usize,
    compte: String,
    sens: &'static str,
    montant: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    devise: Option<VueDevise>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tiers: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    etiquette: Option<VueEtiquette>,
}

#[derive(Debug, Serialize)]
struct VueDevise {
    code: String,
    montant: String,
    cours: String,
}

#[derive(Debug, Serialize)]
struct VueEtiquette {
    nature: String,
    sens: String,
    regime: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    taux: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    exigibilite: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    motif: Option<String>,
}

#[derive(Debug, Serialize)]
struct VuePieceLiee {
    piece: String,
    role: &'static str,
    lie_par: String,
    lie_le: String,
}

#[derive(Debug, Serialize)]
struct VueBrouillon {
    id: String,
    journal: String,
    date: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    date_valeur_comptable: Option<String>,
    libelle: String,
    a_revoir: bool,
    version: i32,
    cree_par: String,
    cree_le: String,
    modifie_par: String,
    modifie_le: String,
    contributeurs: Vec<String>,
    lignes: Vec<VueLigne>,
    pieces: Vec<VuePieceLiee>,
}

fn vue(b: &BrouillonStocke) -> VueBrouillon {
    VueBrouillon {
        id: b.id.to_string(),
        journal: b.contenu.journal.clone(),
        date: interne::date(b.contenu.date),
        date_valeur_comptable: b.contenu.date_valeur_comptable.map(interne::date),
        libelle: b.contenu.libelle.clone(),
        a_revoir: b.a_revoir,
        version: b.version,
        cree_par: b.cree_par.to_string(),
        cree_le: interne::instant(b.cree_le),
        modifie_par: b.modifie_par.to_string(),
        modifie_le: interne::instant(b.modifie_le),
        contributeurs: b.contributeurs.iter().map(ToString::to_string).collect(),
        lignes: b
            .contenu
            .lignes
            .iter()
            .enumerate()
            .map(|(i, l)| VueLigne {
                position: i + 1,
                compte: l.compte.clone(),
                sens: l.sens,
                montant: l.montant.clone(),
                devise: l.devise.as_ref().map(|d| VueDevise {
                    code: d.code.clone(),
                    montant: d.montant.clone(),
                    cours: d.cours.clone(),
                }),
                tiers: l.tiers.map(|t| t.to_string()),
                etiquette: l.etiquette.as_ref().map(|e| VueEtiquette {
                    nature: e.nature.clone(),
                    sens: e.sens.clone(),
                    regime: e.regime.clone(),
                    taux: e.taux.clone(),
                    exigibilite: e.exigibilite.clone(),
                    motif: e.motif.clone(),
                }),
            })
            .collect(),
        pieces: b.pieces.iter().map(vue_lien).collect(),
    }
}

fn vue_lien(l: &LienPiece) -> VuePieceLiee {
    VuePieceLiee {
        piece: l.piece.to_string(),
        role: l.role.nom(),
        lie_par: l.lie_par.to_string(),
        lie_le: interne::instant(l.lie_le),
    }
}

#[derive(Debug, Serialize)]
struct VueResume {
    id: String,
    journal: String,
    date: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    date_valeur_comptable: Option<String>,
    libelle: String,
    a_revoir: bool,
    version: i32,
    lignes: i64,
    debit: String,
    credit: String,
    cree_par: String,
    cree_le: String,
    modifie_par: String,
    modifie_le: String,
}

fn vue_resume(r: &ResumeBrouillon) -> VueResume {
    VueResume {
        id: r.id.to_string(),
        journal: r.journal.clone(),
        date: interne::date(r.date),
        date_valeur_comptable: r.date_valeur_comptable.map(interne::date),
        libelle: r.libelle.clone(),
        a_revoir: r.a_revoir,
        version: r.version,
        lignes: r.lignes,
        debit: r.debit.clone(),
        credit: r.credit.clone(),
        cree_par: r.cree_par.to_string(),
        cree_le: interne::instant(r.cree_le),
        modifie_par: r.modifie_par.to_string(),
        modifie_le: interne::instant(r.modifie_le),
    }
}

#[derive(Debug, Serialize)]
struct VuePage {
    brouillons: Vec<VueResume>,
    suivant: Option<String>,
}

// --- Lecture ----------------------------------------------------------------------------

fn lister_filtre(texte: Option<&str>) -> Result<(FiltreBrouillard, u32), ErreurApi> {
    let invalide = ErreurApi::RequeteInvalide;
    let mut filtre = FiltreBrouillard::default();
    let mut limite = None;
    for paire in texte.unwrap_or("").split('&').filter(|p| !p.is_empty()) {
        let (cle, valeur) = paire.split_once('=').ok_or(invalide)?;
        let uuid = || Uuid::try_parse(valeur).map_err(|_| invalide);
        let deja = match cle {
            "journal" => {
                let code = !valeur.is_empty()
                    && valeur.len() <= 8
                    && valeur
                        .bytes()
                        .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit());
                if !code {
                    return Err(invalide);
                }
                filtre.journal.replace(valeur.to_owned()).is_some()
            }
            "periode" => filtre.periode.replace(uuid()?).is_some(),
            "apres" => filtre.apres.replace(uuid()?).is_some(),
            "limite" => {
                let n = (!valeur.is_empty() && valeur.bytes().all(|b| b.is_ascii_digit()))
                    .then(|| valeur.parse::<u32>().ok())
                    .flatten()
                    .filter(|l| (1..=PAGE_MAX).contains(l))
                    .ok_or(invalide)?;
                limite.replace(n).is_some()
            }
            _ => return Err(invalide),
        };
        if deja {
            return Err(invalide);
        }
    }
    Ok((filtre, limite.unwrap_or(PAGE_PAR_DEFAUT)))
}

async fn lister(tx: &mut TransactionTenant, requete: &Requete<'_>) -> Result<Resultat, ErreurApi> {
    let (filtre, limite) = lister_filtre(requete.parametres)?;
    let page = brouillard::lister(tx, &filtre, limite).await?;
    let suivant = (page.len() == limite as usize)
        .then(|| page.last().map(|p| p.id.to_string()))
        .flatten();
    let vue = VuePage {
        brouillons: page.iter().map(vue_resume).collect(),
        suivant,
    };
    Ok(Resultat::lecture(Json(vue).into_response()))
}

async fn lire(tx: &mut TransactionTenant, requete: &Requete<'_>) -> Result<Resultat, ErreurApi> {
    sans_parametres(requete)?;
    let id = id_chemin(requete.chemin, "brouillon")?;
    let b = brouillard::lire(tx, id)
        .await?
        .ok_or(ErreurApi::Introuvable)?;
    Ok(Resultat::lecture(Json(vue(&b)).into_response()))
}

// --- Traces -----------------------------------------------------------------------------

fn trace_de(dossier: &DossierAutorise, id: Uuid, operation: &str) -> Trace {
    let mut t = Trace::new(PorteeAudit::Dossier(dossier.id()), "brouillon", id);
    t.donnees.push(("operation", operation.to_owned()));
    t
}

/// Traces d'un changement de contenu : l'en-tête (champs changés, `version`), puis une trace par
/// ligne changée. `avant` absent : création ; `apres` absent : suppression.
fn traces_contenu(
    dossier: &DossierAutorise,
    id: Uuid,
    operation: &str,
    avant: Option<&BrouillonStocke>,
    apres: Option<(&ContenuStocke, i32)>,
) -> Vec<Trace> {
    let ancien = avant.map(|b| &b.contenu);
    let nouveau = apres.map(|(c, _)| c);
    let mut en_tete = trace_de(dossier, id, operation);
    let mut champ = |nom: &'static str, av: Option<String>, ap: Option<String>| {
        if av != ap {
            en_tete.champs.push((nom, av, ap));
        }
    };
    champ(
        "journal",
        ancien.map(|c| c.journal.clone()),
        nouveau.map(|c| c.journal.clone()),
    );
    champ(
        "date",
        ancien.map(|c| interne::date(c.date)),
        nouveau.map(|c| interne::date(c.date)),
    );
    champ(
        "date_valeur_comptable",
        ancien.and_then(|c| c.date_valeur_comptable.map(interne::date)),
        nouveau.and_then(|c| c.date_valeur_comptable.map(interne::date)),
    );
    champ(
        "libelle",
        ancien.map(|c| c.libelle.clone()),
        nouveau.map(|c| c.libelle.clone()),
    );
    champ(
        "version",
        avant.map(|b| b.version.to_string()),
        apres.map(|(_, v)| v.to_string()),
    );
    let mut traces = vec![en_tete];
    let (avant_l, apres_l) = (
        ancien.map_or(&[][..], |c| c.lignes.as_slice()),
        nouveau.map_or(&[][..], |c| c.lignes.as_slice()),
    );
    for i in 0..avant_l.len().max(apres_l.len()) {
        let (av, ap) = (avant_l.get(i), apres_l.get(i));
        if av == ap {
            continue;
        }
        let mut t = trace_de(dossier, id, operation);
        t.donnees.push(("ligne", (i + 1).to_string()));
        t.champs.push((
            "ligne",
            av.map(LigneStockee::texte_audit),
            ap.map(LigneStockee::texte_audit),
        ));
        traces.push(t);
    }
    traces
}

// --- Contrôles communs --------------------------------------------------------------------

/// Contenu contrôlé contre le référentiel relu, **puis** période de la date verrouillée
/// `FOR SHARE` et son statut relu : une période ou un exercice clôturé refuse (409).
async fn contenu_controle(
    tx: &mut TransactionTenant,
    c: &saisie::CorpsContenu,
) -> Result<ContenuStocke, ErreurApi> {
    let referentiel = charger(tx).await?.ok_or(ErreurApi::Introuvable)?;
    let tva = referentiel_tva()?;
    let controle = saisie::controler(c, &referentiel, tva)?;
    let contenu = controle.contenu;
    let visee = brouillard::verrouiller_date(tx, contenu.date)
        .await?
        .ok_or(ErreurApi::Saisie {
            refus: RefusSaisie::DateHorsExercice,
            ligne: None,
        })?;
    if visee.periode == StatutVise::Clos || visee.exercice == StatutVise::Clos {
        return Err(conflit(MotifConflit::DateDansPeriodeCloturee));
    }
    if visee.exercice == StatutVise::Intermediaire {
        let admis = referentiel
            .etat()
            .journaux
            .journaux()
            .find(|j| j.code().as_str() == contenu.journal)
            .is_some_and(compta_core::Journal::admis_en_cloture);
        if !admis {
            return Err(conflit(MotifConflit::JournalNonAdmisEnCloture));
        }
    }
    Ok(contenu)
}

/// Verrouille le brouillon et vérifie sa version (409 `brouillon_perime`).
async fn verrouiller_a_la_version(
    tx: &mut TransactionTenant,
    id: Uuid,
    version: i32,
) -> Result<(), ErreurApi> {
    let courante = brouillard::verrouiller(tx, id)
        .await?
        .ok_or(ErreurApi::Introuvable)?;
    if courante == version {
        Ok(())
    } else {
        Err(conflit(MotifConflit::BrouillonPerime))
    }
}

fn reponse_brouillon(statut: StatusCode, b: &BrouillonStocke) -> Response {
    (statut, Json(vue(b))).into_response()
}

async fn relire(tx: &mut TransactionTenant, id: Uuid) -> Result<BrouillonStocke, ErreurApi> {
    brouillard::lire(tx, id)
        .await?
        .ok_or(ErreurApi::Interne(CategorieErreur::Donnee))
}

// --- Création -----------------------------------------------------------------------------

async fn creer(
    tx: &mut TransactionTenant,
    dossier: &DossierAutorise,
    requete: &Requete<'_>,
) -> Result<Resultat, ErreurApi> {
    sans_parametres(requete)?;
    let c: saisie::CorpsContenu = corps(requete.corps)?;
    if c.version.is_some() {
        return Err(ErreurApi::RequeteInvalide);
    }
    let contenu = contenu_controle(tx, &c).await?;
    let id = Uuid::new_v4();
    brouillard::creer(tx, id, &contenu, maintenant()).await?;
    let b = relire(tx, id).await?;
    let traces = traces_contenu(dossier, id, "creation", None, Some((&contenu, b.version)));
    Ok(Resultat {
        reponse: reponse_brouillon(StatusCode::CREATED, &b),
        traces,
    })
}

// --- Modification -------------------------------------------------------------------------

async fn modifier(
    tx: &mut TransactionTenant,
    dossier: &DossierAutorise,
    id: Uuid,
    requete: &Requete<'_>,
) -> Result<Resultat, ErreurApi> {
    sans_parametres(requete)?;
    let c: saisie::CorpsContenu = corps(requete.corps)?;
    let version = c.version.ok_or(ErreurApi::RequeteInvalide)?;
    let contenu = contenu_controle(tx, &c).await?;
    verrouiller_a_la_version(tx, id, version).await?;
    let avant = relire(tx, id).await?;
    if avant.contenu == contenu {
        return Err(conflit(MotifConflit::AucunChangement));
    }
    let nouvelle = brouillard::remplacer(tx, id, &contenu, maintenant()).await?;
    let apres = relire(tx, id).await?;
    let traces = traces_contenu(
        dossier,
        id,
        "modification",
        Some(&avant),
        Some((&contenu, nouvelle)),
    );
    Ok(Resultat {
        reponse: reponse_brouillon(StatusCode::OK, &apres),
        traces,
    })
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CorpsVersion {
    version: i32,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CorpsVide {}

// --- Suppression --------------------------------------------------------------------------

async fn supprimer(
    tx: &mut TransactionTenant,
    dossier: &DossierAutorise,
    id: Uuid,
    requete: &Requete<'_>,
) -> Result<Resultat, ErreurApi> {
    sans_parametres(requete)?;
    let CorpsVersion { version } = corps(requete.corps)?;
    verrouiller_a_la_version(tx, id, version).await?;
    let avant = relire(tx, id).await?;
    brouillard::supprimer(tx, id).await?;
    let traces = traces_contenu(dossier, id, "suppression", Some(&avant), None);
    Ok(Resultat {
        reponse: (
            StatusCode::OK,
            Json(SupprimeVue {
                id: id.to_string(),
                supprime: true,
            }),
        )
            .into_response(),
        traces,
    })
}

#[derive(Debug, Serialize)]
struct SupprimeVue {
    id: String,
    supprime: bool,
}

// --- Drapeau ------------------------------------------------------------------------------

async fn drapeau(
    tx: &mut TransactionTenant,
    dossier: &DossierAutorise,
    id: Uuid,
    requete: &Requete<'_>,
    pose: bool,
) -> Result<Resultat, ErreurApi> {
    sans_parametres(requete)?;
    if !requete.corps.is_empty() {
        corps::<CorpsVide>(requete.corps)?;
    }
    brouillard::verrouiller(tx, id)
        .await?
        .ok_or(ErreurApi::Introuvable)?;
    let avant = relire(tx, id).await?;
    if avant.a_revoir == pose {
        return Err(conflit(MotifConflit::DrapeauInchange));
    }
    brouillard::poser_drapeau(tx, id, pose).await?;
    let apres = relire(tx, id).await?;
    let oui_non = |b: bool| if b { "oui" } else { "non" }.to_owned();
    let mut trace = trace_de(dossier, id, if pose { "a_revoir" } else { "revue_faite" });
    trace.champs.push((
        "a_revoir",
        Some(oui_non(avant.a_revoir)),
        Some(oui_non(apres.a_revoir)),
    ));
    Ok(Resultat {
        reponse: reponse_brouillon(StatusCode::OK, &apres),
        traces: vec![trace],
    })
}

// --- Pièces -------------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CorpsLien {
    piece: String,
    role: String,
    version: i32,
}

async fn lier(
    tx: &mut TransactionTenant,
    dossier: &DossierAutorise,
    id: Uuid,
    requete: &Requete<'_>,
) -> Result<Resultat, ErreurApi> {
    sans_parametres(requete)?;
    let c: CorpsLien = corps(requete.corps)?;
    let piece = Uuid::try_parse(&c.piece).map_err(|_| ErreurApi::RequeteInvalide)?;
    let role = RolePiece::depuis_nom(&c.role).ok_or(ErreurApi::RequeteInvalide)?;
    // Ordre des verrous : le brouillon, puis la pièce.
    verrouiller_a_la_version(tx, id, c.version).await?;
    match brouillard::verrouiller_piece(tx, piece).await? {
        PieceALier::Absente => return Err(ErreurApi::Introuvable),
        PieceALier::Ecartee => return Err(conflit(MotifConflit::PieceEcartee)),
        PieceALier::Liable => {}
    }
    let liens = brouillard::liens_du_brouillon(tx, id).await?;
    if liens.iter().any(|l| l.piece == piece) {
        return Err(conflit(MotifConflit::PieceDejaLiee));
    }
    if role == RolePiece::Principale && liens.iter().any(|l| l.role == RolePiece::Principale) {
        return Err(conflit(MotifConflit::PrincipaleExistante));
    }
    let avant = c.version;
    let nouvelle = brouillard::lier_piece(tx, id, piece, role, maintenant()).await?;
    let b = relire(tx, id).await?;
    let mut trace = trace_de(dossier, id, "lien_piece");
    trace.donnees.push(("piece", piece.to_string()));
    trace.donnees.push(("role", role.nom().to_owned()));
    trace.champs.push((
        "version",
        Some(avant.to_string()),
        Some(nouvelle.to_string()),
    ));
    Ok(Resultat {
        reponse: reponse_brouillon(StatusCode::OK, &b),
        traces: vec![trace],
    })
}

async fn retirer(
    tx: &mut TransactionTenant,
    dossier: &DossierAutorise,
    id: Uuid,
    requete: &Requete<'_>,
) -> Result<Resultat, ErreurApi> {
    sans_parametres(requete)?;
    let CorpsVersion { version } = corps(requete.corps)?;
    let piece = id_chemin(requete.chemin, "piece")?;
    verrouiller_a_la_version(tx, id, version).await?;
    let nouvelle = brouillard::retirer_piece(tx, id, piece, maintenant())
        .await?
        .ok_or(ErreurApi::Introuvable)?;
    let b = relire(tx, id).await?;
    let mut trace = trace_de(dossier, id, "retrait_piece");
    trace.donnees.push(("piece", piece.to_string()));
    trace.champs.push((
        "version",
        Some(version.to_string()),
        Some(nouvelle.to_string()),
    ));
    Ok(Resultat {
        reponse: reponse_brouillon(StatusCode::OK, &b),
        traces: vec![trace],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filtres_stricts() {
        let (f, l) = lister_filtre(Some("journal=ACH&limite=10")).unwrap();
        assert_eq!(f.journal.as_deref(), Some("ACH"));
        assert_eq!(l, 10);
        assert_eq!(lister_filtre(None).unwrap().1, PAGE_PAR_DEFAUT);
        for refuse in [
            "journal=ach",
            "journal=A&journal=B",
            "limite=0",
            "limite=101",
            "periode=x",
            "autre=1",
            "journal",
        ] {
            assert!(lister_filtre(Some(refuse)).is_err(), "{refuse}");
        }
    }
}
