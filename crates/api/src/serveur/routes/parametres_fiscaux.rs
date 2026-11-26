//! Paramètres fiscaux d'un dossier (ADR 0030, point 3) : lecture des versions (paquet du pays et
//! versions propres), version applicable à une date, création d'une version propre. **Aucune
//! route de modification ni de suppression** : une surcharge est une nouvelle version datée,
//! jamais une retouche ; la table est en ajout seul (`0208`).
//!
//! - Lecture : tout rôle qui voit le dossier (`ConsulterParametresFiscaux`).
//! - Création : administrateur du cabinet seul (`CreerVersionParametresFiscaux`), session fraîche
//!   (invariant Cedar, recontrôlé ici), inscription à l'audit chaîné du dossier **sans le motif**
//!   (texte libre) : identifiants, dates, codes et empreinte seulement.
//!
//! Règles de la création, sous la tête de chaîne puis la ligne du dossier (le pays et les
//! validations ne bougent pas pendant l'écriture) : le pays a un paquet ; au plus
//! [`VERSIONS_MAX`] versions ; `date_effet` postérieure à celle de la dernière version propre, hors
//! période clôturée et exercice clos ; une version de base s'applique à cette date ; seuls les
//! taux (rôles du socle, ]0 ; 100], décimal canonique) et les régimes du socle offerts par la base
//! (jamais un régime exonéré ou à motifs, jamais un régime utilisé par une écriture validée)
//! peuvent changer ; motif de 1 à 400 octets.

use std::collections::BTreeMap;

use authz::Action;
use axum::Json;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use compta_core::{StatutExercice, StatutPeriode};
use fiscal::{ErreurFiscale, ParametresFiscaux, Portee as PorteeVersion, Surcharge};
use integrite::Empreinte;
use serde::{Deserialize, Serialize};
use time::Date;

use super::brouillard::date_lue;
use super::{ActionRoute, CasUsage, Methode, Module, Portee, Requete, Resultat, Route};
use crate::base::TransactionTenant;
use crate::base::audit::PorteeAudit;
use crate::base::organisation::{
    lire_appartenance, lire_contexte_du_dossier, verrouiller_dossier_pour_modification,
};
use crate::base::parametres_fiscaux::{self as base, VERSIONS_MAX, VersionPropre};
use crate::base::referentiel::charger;
use crate::base::validation::verrouiller_tete;
use crate::parametres_fiscaux::ParametresDuDossier;
use crate::serveur::audit::Trace;
use crate::serveur::erreur::{CategorieErreur, ErreurApi, MotifConflit, MotifValidation};
use crate::serveur::pipeline::Garde;

/// Cas d'usage du module.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cas {
    /// Versions du dossier : paquet du pays et versions propres.
    Lister,
    /// Version applicable à une date, avec ses taux, régimes et motifs.
    Applicable,
    /// Création d'une version propre.
    Creer,
}

/// Tranche du module.
pub const MODULE: Module = Module {
    nom: "parametres_fiscaux",
    routes: ROUTES,
    champs_journal: &[],
};

/// Routes du module.
pub const ROUTES: &[Route] = &[
    Route {
        methode: Methode::Get,
        chemin: "/api/cabinets/{cabinet}/dossiers/{dossier}/parametres-fiscaux",
        action: ActionRoute::Cabinet(Action::ConsulterParametresFiscaux),
        portee: Portee::Dossier,
        cas_usage: CasUsage::ParametresFiscaux(Cas::Lister),
    },
    Route {
        methode: Methode::Get,
        chemin: "/api/cabinets/{cabinet}/dossiers/{dossier}/parametres-fiscaux/applicable",
        action: ActionRoute::Cabinet(Action::ConsulterParametresFiscaux),
        portee: Portee::Dossier,
        cas_usage: CasUsage::ParametresFiscaux(Cas::Applicable),
    },
    Route {
        methode: Methode::Post,
        chemin: "/api/cabinets/{cabinet}/dossiers/{dossier}/parametres-fiscaux",
        action: ActionRoute::Cabinet(Action::CreerVersionParametresFiscaux),
        portee: Portee::Dossier,
        cas_usage: CasUsage::ParametresFiscaux(Cas::Creer),
    },
];

/// Au plus ce nombre de rôles de taux dans une demande.
const TAUX_MAX: usize = 8;
/// Au plus ce nombre de régimes retirés dans une demande.
const RETIRES_MAX: usize = 32;

/// Exécute un cas d'usage du module dans la transaction du pipeline.
pub(crate) async fn executer(
    cas: Cas,
    tx: &mut TransactionTenant,
    garde: &Garde,
    requete: &Requete<'_>,
) -> Result<Resultat, ErreurApi> {
    match cas {
        Cas::Lister => Ok(Resultat::lecture(lister(tx, requete).await?)),
        Cas::Applicable => Ok(Resultat::lecture(applicable(tx, requete).await?)),
        Cas::Creer => creer(tx, garde, requete).await,
    }
}

fn donnee() -> ErreurApi {
    ErreurApi::Interne(CategorieErreur::Donnee)
}

fn refus(motif: MotifValidation) -> ErreurApi {
    ErreurApi::Validation { motif, ligne: None }
}

fn conflit(motif: MotifConflit) -> ErreurApi {
    ErreurApi::Conflit {
        motif,
        nombre: None,
    }
}

fn hex(octets: &[u8]) -> String {
    use std::fmt::Write as _;
    octets.iter().fold(String::new(), |mut s, o| {
        let _ = write!(s, "{o:02x}");
        s
    })
}

fn sans_parametres(requete: &Requete<'_>) -> Result<(), ErreurApi> {
    if requete.parametres.is_some_and(|p| !p.is_empty()) {
        Err(ErreurApi::RequeteInvalide)
    } else {
        Ok(())
    }
}

// --- Lecture ------------------------------------------------------------------------------

#[derive(Debug, Serialize)]
struct VueVersion {
    identifiant: String,
    /// `pays` (paquet) ou `dossier` (version propre).
    portee: &'static str,
    /// Version du paquet (`AAAA.N`), celle de base pour une version propre.
    version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    numero: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<String>,
    date_effet: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    date_fin: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    motif: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cree_par: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cree_le: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    empreinte: Option<String>,
}

fn vue_du_pays(p: &ParametresFiscaux) -> VueVersion {
    VueVersion {
        identifiant: p.identifiant().to_owned(),
        portee: "pays",
        version: p.version().to_owned(),
        numero: None,
        id: None,
        date_effet: p.date_effet().to_string(),
        date_fin: p.date_fin().map(|d| d.to_string()),
        motif: None,
        cree_par: None,
        cree_le: None,
        empreinte: None,
    }
}

fn vue_propre(p: &ParametresFiscaux, l: &VersionPropre) -> VueVersion {
    VueVersion {
        identifiant: p.identifiant().to_owned(),
        portee: "dossier",
        version: l.version.clone(),
        numero: Some(l.numero),
        id: Some(l.id.to_string()),
        date_effet: l.date_effet.to_string(),
        date_fin: l.date_fin.map(|d| d.to_string()),
        motif: Some(l.motif.clone()),
        cree_par: Some(l.cree_par.to_string()),
        cree_le: Some(crate::pieces::interne::instant(l.cree_le)),
        empreinte: Some(hex(&l.empreinte)),
    }
}

#[derive(Debug, Serialize)]
struct VueListe {
    pays: String,
    versions: Vec<VueVersion>,
}

async fn lister(
    tx: &mut TransactionTenant,
    requete: &Requete<'_>,
) -> Result<axum::response::Response, ErreurApi> {
    sans_parametres(requete)?;
    let registre = requete.registre_fiscal()?;
    let pays = lire_contexte_du_dossier(tx).await?.pays;
    let lignes = base::lire_versions(tx).await?;
    let mut versions: Vec<VueVersion> = registre
        .versions_du_pays(&pays)
        .iter()
        .map(vue_du_pays)
        .collect();
    for l in &lignes {
        let p = crate::parametres_fiscaux::version_depuis_ligne(registre, l)?;
        versions.push(vue_propre(&p, l));
    }
    Ok((StatusCode::OK, Json(VueListe { pays, versions })).into_response())
}

#[derive(Debug, Serialize)]
struct VueTaux {
    role: String,
    valeur: String,
    source: String,
}

#[derive(Debug, Serialize)]
struct VueRegime {
    code: String,
    libelle: String,
    exonere: bool,
    ligne_taxe: bool,
    taux: Vec<String>,
    exigibilites: Vec<&'static str>,
    deduction: &'static str,
}

#[derive(Debug, Serialize)]
struct VueMotif {
    code: String,
    regime: String,
    libelle: String,
}

#[derive(Debug, Serialize)]
struct VueApplicable {
    date: String,
    version: Option<VueVersion>,
    taux: Vec<VueTaux>,
    regimes: Vec<VueRegime>,
    motifs: Vec<VueMotif>,
    retention_annees: Option<u16>,
}

/// Date demandée (`?date=AAAA-MM-JJ`) ; sans paramètre, le jour dans le fuseau du dossier.
fn date_du_parametre(
    requete: &Requete<'_>,
    contexte: &crate::contexte_dossier::ContexteDossier,
) -> Result<Date, ErreurApi> {
    match requete.parametres.filter(|p| !p.is_empty()) {
        None => Ok(contexte.date_du_jour(requete.horloge.maintenant())?),
        Some(p) => {
            let texte = p.strip_prefix("date=").ok_or(ErreurApi::RequeteInvalide)?;
            date_lue(texte).ok_or(ErreurApi::RequeteInvalide)
        }
    }
}

async fn applicable(
    tx: &mut TransactionTenant,
    requete: &Requete<'_>,
) -> Result<axum::response::Response, ErreurApi> {
    let contexte = lire_contexte_du_dossier(tx).await?;
    let jour = date_du_parametre(requete, &contexte)?;
    let registre = requete.registre_fiscal()?;
    let pays = contexte.pays;
    let lignes = base::lire_versions(tx).await?;
    let propres = lignes
        .iter()
        .map(|l| crate::parametres_fiscaux::version_depuis_ligne(registre, l))
        .collect::<Result<Vec<_>, _>>()?;
    let versions = ParametresDuDossier::nouveau(registre, &pays, &propres);
    let Some(p) = versions.applicable(jour) else {
        let vide = VueApplicable {
            date: jour.to_string(),
            version: None,
            taux: Vec::new(),
            regimes: Vec::new(),
            motifs: Vec::new(),
            retention_annees: None,
        };
        return Ok((StatusCode::OK, Json(vide)).into_response());
    };
    let vue = p.a_la_date(jour);
    let version = match p.portee() {
        PorteeVersion::Pack => vue_du_pays(p),
        PorteeVersion::Dossier(n) => {
            let ligne = lignes
                .iter()
                .find(|l| u32::try_from(l.numero) == Ok(n))
                .ok_or_else(donnee)?;
            vue_propre(p, ligne)
        }
    };
    let corps = VueApplicable {
        date: jour.to_string(),
        version: Some(version),
        taux: p
            .taux()
            .iter()
            .map(|t| VueTaux {
                role: t.role.as_str().to_owned(),
                valeur: t.valeur.to_string(),
                source: t.source.clone(),
            })
            .collect(),
        regimes: vue
            .regimes()
            .map(|r| VueRegime {
                code: r.code().to_string(),
                libelle: r.libelle().to_owned(),
                exonere: r.est_exonere(),
                ligne_taxe: r.porte_une_ligne_de_taxe(),
                taux: r.taux().iter().map(ToString::to_string).collect(),
                exigibilites: r
                    .exigibilites_etiquette()
                    .iter()
                    .map(|e| e.code())
                    .collect(),
                deduction: r.deduction().nom(),
            })
            .collect(),
        motifs: vue
            .motifs()
            .map(|m| VueMotif {
                code: m.code().to_string(),
                regime: m.regime().to_string(),
                libelle: m.libelle().to_owned(),
            })
            .collect(),
        retention_annees: Some(p.retention().annees_min),
    };
    Ok((StatusCode::OK, Json(corps)).into_response())
}

// --- Création -----------------------------------------------------------------------------

/// Corps de la création : une version propre dérivée de celle qui s'applique à `date_effet`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CorpsCreation {
    date_effet: String,
    date_fin: Option<String>,
    motif: String,
    /// Taux remplacés, par rôle (`normal`, `reduit`) : décimal canonique dans ]0 ; 100].
    #[serde(default)]
    taux: BTreeMap<String, String>,
    /// Codes des régimes retirés de l'offre du dossier à partir de `date_effet`.
    #[serde(default)]
    regimes_retires: Vec<String>,
}

fn erreur_de_derivation(e: &ErreurFiscale) -> ErreurApi {
    match e {
        ErreurFiscale::MotifInvalide => refus(MotifValidation::ParametresMotifInvalide),
        ErreurFiscale::SurchargeVide => refus(MotifValidation::ParametresAucunChangement),
        ErreurFiscale::Taux { .. } => refus(MotifValidation::ParametresTauxInvalide),
        ErreurFiscale::RoleInconnu(_) => refus(MotifValidation::ParametresRoleInconnu),
        ErreurFiscale::RegimeNonOffert(_) => refus(MotifValidation::ParametresRegimeInconnu),
        ErreurFiscale::RegimeNonRetirable(_) => {
            refus(MotifValidation::ParametresRegimeNonRetirable)
        }
        ErreurFiscale::Periode(_) => refus(MotifValidation::ParametresPeriodeInvalide),
        // Le contenu dérivé d'une version valide ne peut pas être invalide autrement.
        _ => donnee(),
    }
}

/// Vrai si la date tombe dans un exercice clos ou une période clôturée du dossier.
fn date_cloturee(exercices: &compta_core::Exercices, jour: Date) -> bool {
    exercices.exercice_contenant(jour).is_some_and(|e| {
        e.statut() == StatutExercice::Cloture
            || e.periode_contenant(jour)
                .is_some_and(|p| p.statut() == StatutPeriode::Cloturee)
    })
}

#[allow(clippy::too_many_lines)]
async fn creer(
    tx: &mut TransactionTenant,
    garde: &Garde,
    requete: &Requete<'_>,
) -> Result<Resultat, ErreurApi> {
    sans_parametres(requete)?;
    // Défense en profondeur : appartenance active de rôle administrateur, relue en base (Cedar
    // décide aussi) ; absente ou suspendue : 404, autre rôle : 403.
    let appartenance = lire_appartenance(tx)
        .await?
        .filter(|a| a.actif)
        .ok_or(ErreurApi::Introuvable)?;
    if appartenance.role != authz::Role::Administrateur.nom() {
        return Err(ErreurApi::Interdit);
    }
    let id_dossier = tx.contexte().id_dossier().ok_or(ErreurApi::Introuvable)?;
    let Json(c) =
        Json::<CorpsCreation>::from_bytes(requete.corps).map_err(|_| ErreurApi::RequeteInvalide)?;
    if c.taux.len() > TAUX_MAX
        || c.regimes_retires.len() > RETIRES_MAX
        || c.regimes_retires.iter().any(|r| r.len() > 32)
        || c.taux.iter().any(|(r, v)| r.len() > 32 || v.len() > 32)
    {
        return Err(ErreurApi::RequeteInvalide);
    }
    let date_effet = date_lue(&c.date_effet).ok_or(ErreurApi::RequeteInvalide)?;
    let date_fin = c
        .date_fin
        .as_deref()
        .map(|d| date_lue(d).ok_or(ErreurApi::RequeteInvalide))
        .transpose()?;
    // Ré-authentification : invariant Cedar, recontrôlé par le cas d'usage.
    if !garde.session_fraiche() {
        return Err(ErreurApi::Interdit);
    }
    let registre = requete.registre_fiscal()?;

    // Verrous : tête de chaîne (sérialise les validations), puis ligne du dossier (pays).
    let genese = integrite::genese(
        &integrite::IdDossier::parse(&id_dossier.uuid().to_string()).map_err(|_| donnee())?,
    );
    verrouiller_tete(tx, &genese).await?;
    let contexte = verrouiller_dossier_pour_modification(tx).await?;
    if registre.versions_du_pays(&contexte.pays).is_empty() {
        return Err(refus(MotifValidation::ParametresPaysSansPaquet));
    }
    let lignes = base::lire_versions(tx).await?;
    if lignes.len() >= VERSIONS_MAX {
        return Err(conflit(MotifConflit::ParametresTropDeVersions));
    }
    let propres = lignes
        .iter()
        .map(|l| crate::parametres_fiscaux::version_depuis_ligne(registre, l))
        .collect::<Result<Vec<_>, _>>()?;
    if propres.iter().any(|p| p.date_effet() >= date_effet) {
        return Err(conflit(MotifConflit::ParametresDateEffetAnterieure));
    }
    let referentiel = charger(tx).await?.ok_or(ErreurApi::Introuvable)?;
    if date_cloturee(referentiel.exercices(), date_effet) {
        return Err(conflit(MotifConflit::ParametresDateEffetCloturee));
    }
    let versions = ParametresDuDossier::nouveau(registre, &contexte.pays, &propres);
    let base_applicable = versions
        .applicable(date_effet)
        .ok_or_else(|| refus(MotifValidation::ParametresAucuneBase))?;
    let numero = lignes.last().map_or(1, |l| l.numero + 1);
    let taux: Vec<(String, String)> = c.taux.into_iter().collect();
    let nouvelle = base_applicable
        .deriver(
            registre.socle(),
            &Surcharge {
                numero: u32::try_from(numero).map_err(|_| donnee())?,
                date_effet,
                date_fin,
                motif: &c.motif,
                taux: &taux,
                regimes_retires: &c.regimes_retires,
            },
        )
        .map_err(|e| erreur_de_derivation(&e))?;
    registre
        .verifier_version_de_dossier(&propres, &nouvelle)
        .map_err(|_| donnee())?;
    // Jamais un régime utilisé par une écriture validée du dossier.
    if !base::regimes_utilises(tx, &c.regimes_retires)
        .await?
        .is_empty()
    {
        return Err(conflit(MotifConflit::ParametresRegimeUtilise));
    }

    let contenu = nouvelle.vers_texte();
    let empreinte = Empreinte::calculer(contenu.as_bytes());
    let ligne = VersionPropre {
        id: uuid::Uuid::new_v4(),
        numero,
        pays: nouvelle.pays().to_owned(),
        version: nouvelle.version().to_owned(),
        date_effet,
        date_fin,
        contenu,
        empreinte: empreinte.octets().to_vec(),
        motif: c.motif,
        cree_par: tx.contexte().id_utilisateur().uuid(),
        cree_le: requete.horloge.maintenant(),
    };
    base::inserer(tx, &ligne).await?;

    // Audit : identifiants, dates, codes et empreinte seulement (le motif est un texte libre).
    let mut trace = Trace::new(
        PorteeAudit::Dossier(id_dossier),
        "parametres_fiscaux",
        ligne.id,
    );
    trace
        .donnees
        .push(("version", nouvelle.identifiant().to_owned()));
    trace
        .donnees
        .push(("base", base_applicable.identifiant().to_owned()));
    trace.donnees.push(("date_effet", date_effet.to_string()));
    if let Some(f) = date_fin {
        trace.donnees.push(("date_fin", f.to_string()));
    }
    let roles: Vec<&str> = taux.iter().map(|(r, _)| r.as_str()).collect();
    if !roles.is_empty() {
        trace.donnees.push(("taux_modifies", roles.join(",")));
    }
    if !c.regimes_retires.is_empty() {
        trace
            .donnees
            .push(("regimes_retires", c.regimes_retires.join(",")));
    }
    trace.donnees.push(("empreinte", hex(empreinte.octets())));
    let vue = vue_propre(&nouvelle, &ligne);
    Ok(Resultat {
        reponse: (StatusCode::CREATED, Json(vue)).into_response(),
        traces: vec![trace],
    })
}
