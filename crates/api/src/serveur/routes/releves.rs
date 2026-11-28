//! Relevés importés, partie 1/2 (L3-22, PR 12a de `docs/lots/L3.md` ; ADR 0034 points 4 à 6 ;
//! spec §9.2) : profils d'import du dossier, versionnés ; dépôt du fichier CSV comme pièce ;
//! analyse (aperçu, **sans aucune écriture**). L'import des lignes, les doublons de lignes, le
//! contrôle des soldes, les règles et les propositions de brouillon sont de la PR 12b.
//!
//! # Profils (ADR 0034 point 6)
//! Un profil est une **donnée** validée par `formats` (Tryton `StatementImportCSV`,
//! `account_statement/statement.py:1449-1576` ; OCA `account_statement_import_sheet_mapping.py`).
//! Il est **versionné en ajout seul** : la création publie la version 1 ; tout changement publie la
//! version suivante du même profil, et une version publiée ne change jamais (comme les versions de
//! paramètres fiscaux d'un dossier, ADR 0030). Au plus `profils_max` profils par dossier (20,
//! valeur initiale de l'ADR) : 409 `releve_profils_max`. Un format de date sans année est refusé
//! (OCA complète l'année par l'horloge, non repris).
//!
//! # Dépôt (ADR 0034 point 4, amendement de l'ADR 0015 ; décision D3)
//! Le corps est le fichier, brut, borné **pendant la réception** à `limites.taille_fichier`
//! (5 Mio : 413 au-delà, avant toute lecture complète), sur un **compte de trésorerie** du dossier
//! (404 sinon ; 409 `compte_tresorerie_archive` s'il est archivé, comme l'arrêté de caisse). Type
//! « texte » reconnu par son **contenu** : aucune signature de PDF, JPEG ou PNG (elles priment),
//! décodable sans octet nul ni caractère de contrôle dans un encodage de la liste fermée de
//! `formats`, lignes bornées (415 `type_non_admis`, ou 422 `releve_ligne_trop_longue` avec la
//! ligne). Le fichier devient une **pièce externe** (WORM, empreinte SHA-256 du serveur, rétention
//! du paquet du pays), de statut probant « importé, non certifié ». Un fichier identique à l'octet
//! déjà déposé dans le dossier (pièce non écartée) n'est pas stocké une seconde fois : 200
//! `{"doublon":true,"depots":[…]}` avec renvoi aux dépôts précédents (ADR 0034 point 7, couche 1 :
//! un fichier identique ne contient aucune ligne nouvelle ; aucun objet WORM inutile, gardé dix
//! ans). Le corps n'est jamais journalisé ; le journal d'audit ne reçoit que des identifiants, des
//! noms de champs, l'empreinte et la taille (jamais le nom du fichier, un libellé ni un
//! identifiant bancaire : ADR 0034 point 11, F2.11).
//!
//! # Analyse (aperçu)
//! `GET …/releves/depots/{depot}/analyse/{profil}[?version=&lignes=]` : relit la pièce (empreinte
//! recalculée, F3.1), l'analyse par `formats::releve::analyser` avec les limites de l'instance
//! (lignes, colonnes, longueur de ligne et de cellule : 422 avec la ligne fautive), hors de la
//! boucle asynchrone, et rend : colonnes du profil, premières lignes lues (20 par défaut, 100 au
//! plus), alertes (`libelle_formule`, `caractere_bidirectionnel` : un texte du fichier n'est
//! jamais rendu sans l'alerte que `formats` lui attache), totaux calculés en décimal exact,
//! premier et dernier soldes lus. **Rien n'est écrit**, pas même au journal d'audit (lecture).
//!
//! # Défense en profondeur (ADR 0011)
//! Même si Cedar se trompait : chaque cas relit le dossier **visible** et, pour une mutation ou
//! l'analyse, le rôle de l'appartenance active en base (profils : administrateur, valideur ;
//! dépôt et analyse : plus collaborateur). Un dépôt, un profil ou un compte de trésorerie d'un
//! autre dossier est absent (404), comme un identifiant inexistant.

use std::time::Duration;

use authz::{Action, Role};
use axum::Json;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use formats::releve::{
    Colonne, Encodage, ErreurFormatDate, ErreurProfil, ErreurReleve, Limites, Montants, ProfilBrut,
    ProfilReleve, Releve, TAILLE_MAX_PROFIL,
};
use integrite::Empreinte;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uuid::Uuid;

use super::pieces::{deposer_releve, nom_origine, parametres, relire, valeur};
use super::{ActionRoute, CasUsage, Methode, Module, Portee, Requete, Resultat, Route};
use crate::base::TransactionTenant;
use crate::base::audit::PorteeAudit;
use crate::base::organisation::{lire_appartenance, lire_dossier_visible};
use crate::base::releves::{self as base, DepotLu, NouveauDepot, NouvelleVersion, ProfilLu};
use crate::base::tresorerie;
use crate::pieces::TypePiece;
use crate::pieces::base::{lire_piece, verrouiller_dossier};
use crate::pieces::interne;
use crate::serveur::audit::Trace;
use crate::serveur::erreur::{CategorieErreur, ErreurApi, MotifConflit, MotifReleve};
use crate::serveur::pipeline::{DossierAutorise, Garde, id_chemin};

/// Cas d'usage du module.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cas {
    /// Profils du dossier (dernière version de chacun).
    ListerProfils,
    /// Créer un profil (version 1).
    CreerProfil,
    /// Un profil : une version (la dernière par défaut) et la liste de ses versions.
    LireProfil,
    /// Publier la version suivante d'un profil.
    VersionnerProfil,
    /// Déposer un fichier de relevé sur un compte de trésorerie.
    Deposer,
    /// Dépôts du dossier (les plus récents).
    ListerDepots,
    /// Un dépôt.
    LireDepot,
    /// Analyse d'un dépôt avec un profil : aperçu, sans écriture.
    Analyser,
}

/// Tranche du module.
pub const MODULE: Module = Module {
    nom: "releves",
    routes: ROUTES,
    champs_journal: &[],
};

const fn route(methode: Methode, chemin: &'static str, action: Action, cas: Cas) -> Route {
    Route {
        methode,
        chemin,
        action: ActionRoute::Cabinet(action),
        portee: Portee::Dossier,
        cas_usage: CasUsage::Releves(cas),
    }
}

/// Routes du module. Lecture : `ConsulterTresorerie` (quatre rôles, plan C6) ; profils :
/// `ParametrerReleves` (administrateur, valideur) ; dépôt : `DeposerReleve` ; analyse, premier
/// temps de l'import : `ImporterReleve` (administrateur, valideur, collaborateur).
pub const ROUTES: &[Route] = &[
    route(
        Methode::Get,
        "/api/cabinets/{cabinet}/dossiers/{dossier}/releves/profils",
        Action::ConsulterTresorerie,
        Cas::ListerProfils,
    ),
    route(
        Methode::Post,
        "/api/cabinets/{cabinet}/dossiers/{dossier}/releves/profils",
        Action::ParametrerReleves,
        Cas::CreerProfil,
    ),
    route(
        Methode::Get,
        "/api/cabinets/{cabinet}/dossiers/{dossier}/releves/profils/{profil}",
        Action::ConsulterTresorerie,
        Cas::LireProfil,
    ),
    route(
        Methode::Post,
        "/api/cabinets/{cabinet}/dossiers/{dossier}/releves/profils/{profil}/versions",
        Action::ParametrerReleves,
        Cas::VersionnerProfil,
    ),
    route(
        Methode::Post,
        "/api/cabinets/{cabinet}/dossiers/{dossier}/releves/comptes/{compte_tresorerie}/depots",
        Action::DeposerReleve,
        Cas::Deposer,
    ),
    route(
        Methode::Get,
        "/api/cabinets/{cabinet}/dossiers/{dossier}/releves/depots",
        Action::ConsulterTresorerie,
        Cas::ListerDepots,
    ),
    route(
        Methode::Get,
        "/api/cabinets/{cabinet}/dossiers/{dossier}/releves/depots/{depot}",
        Action::ConsulterTresorerie,
        Cas::LireDepot,
    ),
    route(
        Methode::Get,
        "/api/cabinets/{cabinet}/dossiers/{dossier}/releves/depots/{depot}/analyse/{profil}",
        Action::ImporterReleve,
        Cas::Analyser,
    ),
];

/// Réglages d'instance des relevés (ADR 0034 point 6 : valeurs initiales, **lues par `api` et
/// passées à `formats`**, jamais codées dans `formats`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReglagesReleves {
    /// Limites d'un fichier : taille (appliquée aussi pendant la réception du dépôt), lignes,
    /// colonnes, longueur de ligne et de cellule.
    pub limites: Limites,
    /// Profils d'import par dossier, au plus.
    pub profils_max: i64,
    /// Délai de traitement d'un dépôt (réception du corps comprise).
    pub delai_depot: Duration,
}

impl Default for ReglagesReleves {
    /// Valeurs initiales de l'ADR 0034 point 6 : 5 Mio, 20 000 lignes, 50 colonnes, 64 Kio par
    /// ligne, 1 000 caractères par cellule, 20 profils par dossier ; délai du dépôt des pièces.
    fn default() -> Self {
        Self {
            limites: Limites {
                taille_fichier: 5 * 1024 * 1024,
                lignes: 20_000,
                colonnes: 50,
                longueur_ligne: 64 * 1024,
                longueur_cellule: 1000,
            },
            profils_max: 20,
            delai_depot: crate::pieces::DELAI_DEPOT_PAR_DEFAUT,
        }
    }
}

/// Limites propres d'une route du module : le dépôt reçoit un fichier entier, borné **pendant
/// la réception du flux** à la taille maximale d'un relevé.
#[must_use]
pub(crate) fn limites(cas: Cas, reglages: &ReglagesReleves) -> Option<(usize, Duration)> {
    match cas {
        Cas::Deposer => Some((reglages.limites.taille_fichier, reglages.delai_depot)),
        _ => None,
    }
}

/// Lignes de l'aperçu par défaut et au plus ; alertes rendues au plus (leur nombre total est
/// toujours donné).
const APERCU_PAR_DEFAUT: usize = 20;
const APERCU_MAX: usize = 100;
const ALERTES_MAX: usize = 200;
/// Dépôts rendus par la liste (les plus récents).
const DEPOTS_MAX: i64 = 100;
/// Longueur maximale d'un nom de profil, en caractères (colonne `nom`).
const NOM_MAX: usize = 200;
/// Statut probant d'un relevé déposé (décision D3).
const STATUT_PROBANT: &str = "importe_non_certifie";

/// Exécute un cas d'usage du module dans la transaction du pipeline.
pub(crate) async fn executer(
    cas: Cas,
    tx: &mut TransactionTenant,
    garde: &Garde,
    requete: &Requete<'_>,
) -> Result<Resultat, ErreurApi> {
    let dossier = dossier_visible(tx, garde).await?;
    let saisie = [Role::Administrateur, Role::Valideur, Role::Collaborateur];
    let parametrage = [Role::Administrateur, Role::Valideur];
    match cas {
        Cas::ListerProfils => lister_profils(tx, requete).await,
        Cas::LireProfil => lire_profil(tx, requete).await,
        Cas::ListerDepots => lister_depots(tx, requete).await,
        Cas::LireDepot => lire_depot(tx, requete).await,
        Cas::CreerProfil => {
            exiger_role(tx, &parametrage).await?;
            creer_profil(tx, &dossier, requete).await
        }
        Cas::VersionnerProfil => {
            exiger_role(tx, &parametrage).await?;
            versionner_profil(tx, &dossier, requete).await
        }
        Cas::Deposer => {
            exiger_role(tx, &saisie).await?;
            deposer(tx, &dossier, requete).await
        }
        Cas::Analyser => {
            exiger_role(tx, &saisie).await?;
            analyser(tx, &dossier, requete).await
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

fn refus(motif: MotifReleve, ligne: Option<usize>) -> ErreurApi {
    ErreurApi::Releve {
        motif,
        ligne: ligne.and_then(|l| u32::try_from(l).ok()),
    }
}

/// Instant de l'horloge de la requête, à la microseconde (précision de PostgreSQL).
fn instant(requete: &Requete<'_>) -> OffsetDateTime {
    let i = requete.horloge.maintenant();
    i.replace_nanosecond(i.nanosecond() / 1000 * 1000)
        .unwrap_or(i)
}

fn empreinte_lue(octets: &[u8]) -> Result<Empreinte, ErreurApi> {
    let tableau: [u8; 32] = octets.try_into().map_err(|_| donnee())?;
    Ok(Empreinte::depuis_octets(tableau))
}

/// Entier décimal strict (chiffres seuls) dans `bornes`, sinon 400.
fn entier<T: TryFrom<u64>>(
    texte: &str,
    bornes: std::ops::RangeInclusive<u64>,
) -> Result<T, ErreurApi> {
    if texte.is_empty() || texte.len() > 10 || !texte.bytes().all(|o| o.is_ascii_digit()) {
        return Err(ErreurApi::RequeteInvalide);
    }
    texte
        .parse::<u64>()
        .ok()
        .filter(|n| bornes.contains(n))
        .and_then(|n| T::try_from(n).ok())
        .ok_or(ErreurApi::RequeteInvalide)
}

fn sans_parametres(requete: &Requete<'_>) -> Result<(), ErreurApi> {
    parametres(requete.parametres, &[]).map(|_| ())
}

// --- Correspondance des refus de `formats` ----------------------------------------------------

/// Refus d'un profil : code stable, sans écho de la valeur.
fn refus_profil(erreur: &ErreurProfil) -> ErreurApi {
    let motif = match erreur {
        ErreurProfil::Syntaxe(_) => MotifReleve::ProfilIllisible,
        ErreurProfil::EncodageInconnu => MotifReleve::ProfilEncodage,
        ErreurProfil::Delimiteur => MotifReleve::ProfilDelimiteur,
        ErreurProfil::Guillemet => MotifReleve::ProfilGuillemet,
        ErreurProfil::SeparateurDecimal | ErreurProfil::SeparateurMilliers => {
            MotifReleve::ProfilSeparateur
        }
        ErreurProfil::ProfilTropGrand { .. } => MotifReleve::ProfilTropGrand,
        ErreurProfil::FormatDate(ErreurFormatDate::SansAnnee) => MotifReleve::ProfilDateSansAnnee,
        ErreurProfil::FormatDateTropLong | ErreurProfil::FormatDate(_) => {
            MotifReleve::ProfilFormatDate
        }
        ErreurProfil::ColonnesMontant => MotifReleve::ProfilColonnesMontant,
        ErreurProfil::TitreTropLong(_)
        | ErreurProfil::TitreSansEntete(_)
        | ErreurProfil::TitreVide(_) => MotifReleve::ProfilTitre,
    };
    refus(motif, None)
}

/// Refus d'un fichier : code stable et ligne fautive (ADR 0034 point 6 : 422 avec le rang de la
/// première ligne fautive). Les erreurs de `formats` ne portent que des positions (F2.11).
fn refus_fichier(erreur: &ErreurReleve) -> ErreurApi {
    let motif = match erreur {
        ErreurReleve::FichierTropGrand { .. } => MotifReleve::FichierTropGrand,
        ErreurReleve::OctetNul { .. } => MotifReleve::OctetNul,
        ErreurReleve::EncodageInvalide { .. } => MotifReleve::EncodageInvalide,
        ErreurReleve::CaractereDeControle { .. } => MotifReleve::CaractereDeControle,
        ErreurReleve::LigneTropLongue { .. } => MotifReleve::LigneTropLongue,
        ErreurReleve::TropDeLignes { .. } => MotifReleve::TropDeLignes,
        ErreurReleve::TropDeColonnes { .. } => MotifReleve::TropDeColonnes,
        ErreurReleve::CelluleTropLongue { .. } => MotifReleve::CelluleTropLongue,
        ErreurReleve::GuillemetNonFerme { .. } => MotifReleve::GuillemetNonFerme,
        ErreurReleve::TitresAbsents => MotifReleve::TitresAbsents,
        ErreurReleve::TitreIntrouvable { .. } => MotifReleve::TitreIntrouvable,
        ErreurReleve::ColonneAbsente { .. } => MotifReleve::ColonneAbsente,
        ErreurReleve::DateInvalide { .. } => MotifReleve::DateInvalide,
        ErreurReleve::NombreInvalide { .. } => MotifReleve::NombreInvalide,
    };
    refus(motif, erreur.ligne())
}

// --- Profils ----------------------------------------------------------------------------------

/// Corps de la création d'un profil et d'une nouvelle version : le nom et le profil (forme de
/// `formats::releve::ProfilBrut`, en JSON).
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CorpsProfil {
    nom: String,
    profil: serde_json::Value,
}

/// Profil validé, sous sa forme canonique stockée, et son empreinte.
struct ProfilValide {
    nom: String,
    definition: String,
    empreinte: Empreinte,
}

/// Lit et valide le corps : nom (1 à 200 caractères, non blanc, sans caractère de contrôle ni de
/// format), profil validé par `formats`, forme canonique bornée par la taille d'un profil de
/// `formats`.
fn profil_du_corps(corps: &[u8]) -> Result<ProfilValide, ErreurApi> {
    let Json(CorpsProfil { nom, profil }) =
        Json::<CorpsProfil>::from_bytes(corps).map_err(|_| ErreurApi::RequeteInvalide)?;
    let nom = nom.trim().to_owned();
    if nom.is_empty() || nom.chars().count() > NOM_MAX || !crate::pieces::texte_sur(&nom) {
        return Err(refus(MotifReleve::ProfilNom, None));
    }
    let brut: ProfilBrut =
        serde_json::from_value(profil).map_err(|_| refus(MotifReleve::ProfilIllisible, None))?;
    let valide = ProfilReleve::try_from(brut).map_err(|e| refus_profil(&e))?;
    let definition = serde_json::to_string(&ProfilBrut::from(valide)).map_err(|_| donnee())?;
    if definition.len() > TAILLE_MAX_PROFIL {
        return Err(refus(MotifReleve::ProfilTropGrand, None));
    }
    let empreinte = Empreinte::calculer(definition.as_bytes());
    Ok(ProfilValide {
        nom,
        definition,
        empreinte,
    })
}

/// Profil stocké, relu et **revalidé** par `formats` (une ligne altérée est refusée : 500).
fn profil_stocke(lu: &ProfilLu) -> Result<ProfilReleve, ErreurApi> {
    if Empreinte::calculer(lu.definition.as_bytes()) != empreinte_lue(&lu.empreinte)? {
        return Err(donnee());
    }
    serde_json::from_str(&lu.definition).map_err(|_| donnee())
}

#[derive(Debug, Serialize)]
struct VueVersion {
    version: i32,
    nom: String,
    empreinte: String,
    cree_par: String,
    cree_le: String,
}

#[derive(Debug, Serialize)]
struct VueProfil {
    id: String,
    version: i32,
    nom: String,
    profil: ProfilBrut,
    empreinte: String,
    cree_par: String,
    cree_le: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    versions: Option<Vec<VueVersion>>,
}

fn vue_profil(lu: &ProfilLu, versions: Option<&[ProfilLu]>) -> Result<VueProfil, ErreurApi> {
    let profil = profil_stocke(lu)?;
    Ok(VueProfil {
        id: lu.id.to_string(),
        version: lu.version,
        nom: lu.nom.clone(),
        profil: ProfilBrut::from(profil),
        empreinte: empreinte_lue(&lu.empreinte)?.to_string(),
        cree_par: lu.cree_par.to_string(),
        cree_le: interne::instant(lu.cree_le),
        versions: versions
            .map(|vs| {
                vs.iter()
                    .map(|v| {
                        Ok(VueVersion {
                            version: v.version,
                            nom: v.nom.clone(),
                            empreinte: empreinte_lue(&v.empreinte)?.to_string(),
                            cree_par: v.cree_par.to_string(),
                            cree_le: interne::instant(v.cree_le),
                        })
                    })
                    .collect::<Result<Vec<_>, ErreurApi>>()
            })
            .transpose()?,
    })
}

async fn lister_profils(
    tx: &mut TransactionTenant,
    requete: &Requete<'_>,
) -> Result<Resultat, ErreurApi> {
    sans_parametres(requete)?;
    let profils = base::lister_profils(tx).await?;
    let vues = profils
        .iter()
        .map(|p| vue_profil(p, None))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Resultat::lecture(Json(vues).into_response()))
}

/// `?version=N` (facultatif) : cette version ; sinon la dernière. Avec la liste des versions.
async fn lire_profil(
    tx: &mut TransactionTenant,
    requete: &Requete<'_>,
) -> Result<Resultat, ErreurApi> {
    let lus = parametres(requete.parametres, &["version"])?;
    let version = valeur(&lus, "version")
        .map(|v| entier::<i32>(v, 1..=u64::from(i32::MAX.unsigned_abs())))
        .transpose()?;
    let id = id_chemin(requete.chemin, "profil")?;
    let lu = base::lire_profil(tx, id, version)
        .await?
        .ok_or(ErreurApi::Introuvable)?;
    let versions = base::versions(tx, id).await?;
    let vue = vue_profil(&lu, Some(&versions))?;
    Ok(Resultat::lecture(Json(vue).into_response()))
}

/// Inscrit une version et rend la réponse 201 avec sa trace (identifiant, version, empreinte :
/// ni le nom ni la définition au journal d'audit).
async fn publier(
    tx: &mut TransactionTenant,
    dossier: &DossierAutorise,
    requete: &Requete<'_>,
    id: Uuid,
    precedente: Option<&ProfilLu>,
    profil: &ProfilValide,
) -> Result<Resultat, ErreurApi> {
    let version = precedente.map_or(Ok(1), |p| p.version.checked_add(1).ok_or_else(donnee))?;
    base::inserer_version(
        tx,
        &NouvelleVersion {
            id,
            version,
            nom: &profil.nom,
            definition: &profil.definition,
            empreinte: profil.empreinte.octets(),
            cree_le: instant(requete),
        },
    )
    .await?;
    let lu = base::lire_profil(tx, id, Some(version))
        .await?
        .ok_or_else(donnee)?;
    let mut trace = Trace::new(PorteeAudit::Dossier(dossier.id()), "profil_releve", id);
    trace.champs.push((
        "version",
        precedente.map(|p| p.version.to_string()),
        Some(version.to_string()),
    ));
    trace.champs.push((
        "empreinte",
        precedente
            .map(|p| empreinte_lue(&p.empreinte).map(|e| e.to_string()))
            .transpose()?,
        Some(profil.empreinte.to_string()),
    ));
    Ok(Resultat {
        reponse: (StatusCode::CREATED, Json(vue_profil(&lu, None)?)).into_response(),
        traces: vec![trace],
    })
}

/// Création d'un profil (version 1), sous le verrou du dossier (le nombre de profils est relu
/// sous ce verrou : deux créations concurrentes ne dépassent pas la borne).
async fn creer_profil(
    tx: &mut TransactionTenant,
    dossier: &DossierAutorise,
    requete: &Requete<'_>,
) -> Result<Resultat, ErreurApi> {
    sans_parametres(requete)?;
    let profil = profil_du_corps(requete.corps)?;
    if !verrouiller_dossier(tx).await? {
        return Err(ErreurApi::Introuvable);
    }
    let max = requete.releves.profils_max;
    if base::nombre_de_profils(tx).await? >= max {
        return Err(ErreurApi::Conflit {
            motif: MotifConflit::ProfilsReleveMax,
            nombre: u32::try_from(max).ok(),
        });
    }
    publier(tx, dossier, requete, Uuid::new_v4(), None, &profil).await
}

/// Version suivante d'un profil, sous le verrou du dossier (deux publications concurrentes
/// prennent deux numéros successifs). La version précédente reste telle quelle.
async fn versionner_profil(
    tx: &mut TransactionTenant,
    dossier: &DossierAutorise,
    requete: &Requete<'_>,
) -> Result<Resultat, ErreurApi> {
    sans_parametres(requete)?;
    let profil = profil_du_corps(requete.corps)?;
    let id = id_chemin(requete.chemin, "profil")?;
    if !verrouiller_dossier(tx).await? {
        return Err(ErreurApi::Introuvable);
    }
    let derniere = base::lire_profil(tx, id, None)
        .await?
        .ok_or(ErreurApi::Introuvable)?;
    publier(tx, dossier, requete, id, Some(&derniere), &profil).await
}

// --- Dépôts -----------------------------------------------------------------------------------

#[derive(Debug, Serialize)]
struct VueDepot {
    id: String,
    compte_tresorerie: String,
    piece: String,
    empreinte: String,
    taille: i64,
    nom: Option<String>,
    statut_probant: String,
    ecartee: bool,
    depose_par: String,
    depose_le: String,
}

fn vue_depot(d: &DepotLu) -> Result<VueDepot, ErreurApi> {
    Ok(VueDepot {
        id: d.id.to_string(),
        compte_tresorerie: d.compte_tresorerie_id.to_string(),
        piece: d.piece_id.to_string(),
        empreinte: empreinte_lue(&d.empreinte)?.to_string(),
        taille: d.taille,
        nom: d.nom_origine.clone(),
        statut_probant: d.statut_probant.clone(),
        ecartee: d.ecartee,
        depose_par: d.depose_par.to_string(),
        depose_le: interne::instant(d.depose_le),
    })
}

#[derive(Debug, Serialize)]
struct VueDoublon {
    doublon: bool,
    depots: Vec<VueDepot>,
}

/// Reconnaît le type « texte » d'un relevé par son **contenu** (ADR 0034 point 4) : les types à
/// signature priment (un PDF n'est jamais un relevé) ; puis le fichier doit se décoder, sans
/// octet nul ni caractère de contrôle et lignes bornées, dans **un** encodage de la liste fermée
/// de `formats` (le profil, qui fixe l'encodage, n'est choisi qu'à l'analyse).
fn reconnaitre_texte(octets: &[u8], limites: &Limites) -> Result<(), ErreurApi> {
    if TypePiece::reconnaitre(octets).is_some() {
        return Err(ErreurApi::TypeNonAdmis);
    }
    let mut premiere = None;
    for encodage in Encodage::libelles_admis()
        .iter()
        .filter_map(|l| Encodage::depuis_libelle(l))
    {
        match formats::releve::decoder(octets, encodage, limites) {
            Ok(_) => return Ok(()),
            Err(e) => {
                premiere.get_or_insert(e);
            }
        }
    }
    Err(match premiere {
        Some(ErreurReleve::FichierTropGrand { .. }) => ErreurApi::CorpsTropGrand,
        Some(e @ ErreurReleve::LigneTropLongue { .. }) => refus_fichier(&e),
        _ => ErreurApi::TypeNonAdmis,
    })
}

/// Dépôt d'un relevé sur le compte de trésorerie du chemin. Paramètres : `nom` (nom d'origine),
/// `empreinte` (SHA-256 hexadécimale annoncée par le client, 400 si elle diffère).
async fn deposer(
    tx: &mut TransactionTenant,
    dossier: &DossierAutorise,
    requete: &Requete<'_>,
) -> Result<Resultat, ErreurApi> {
    let lus = parametres(requete.parametres, &["nom", "empreinte"])?;
    let nom = valeur(&lus, "nom").map(nom_origine).transpose()?;
    let compte_id = id_chemin(requete.chemin, "compte_tresorerie")?;
    let compte = tresorerie::lire(tx, compte_id, false)
        .await?
        .ok_or(ErreurApi::Introuvable)?;
    if compte.archive {
        return Err(ErreurApi::Conflit {
            motif: MotifConflit::CompteTresorerieArchive,
            nombre: None,
        });
    }
    let octets = requete.corps;
    if octets.is_empty() {
        return Err(ErreurApi::RequeteInvalide);
    }
    let limites = &requete.releves.limites;
    if octets.len() > limites.taille_fichier {
        return Err(ErreurApi::CorpsTropGrand);
    }
    reconnaitre_texte(octets, limites)?;
    let empreinte = Empreinte::calculer(octets);
    if let Some(annoncee) = valeur(&lus, "empreinte") {
        let annoncee = Empreinte::parse(&annoncee.to_ascii_lowercase())
            .map_err(|_| ErreurApi::RequeteInvalide)?;
        if annoncee != empreinte {
            return Err(ErreurApi::RequeteInvalide);
        }
    }
    // Même fichier déjà déposé dans ce dossier : recherche après le verrou du dossier, qui
    // sérialise les dépôts concurrents, donc avant l'écriture de l'objet (F3.5 : jamais hors du
    // dossier).
    if !verrouiller_dossier(tx).await? {
        return Err(ErreurApi::Introuvable);
    }
    let precedents = base::depots_de_meme_empreinte(tx, empreinte.octets()).await?;
    if let Some(premier) = precedents.first() {
        let mut trace = Trace::new(
            PorteeAudit::Dossier(dossier.id()),
            "depot_releve",
            premier.id,
        );
        trace.donnees.push(("issue", "doublon_signale".to_owned()));
        trace.donnees.push(("empreinte", empreinte.to_string()));
        let vue = VueDoublon {
            doublon: true,
            depots: precedents.iter().map(vue_depot).collect::<Result<_, _>>()?,
        };
        return Ok(Resultat {
            reponse: (StatusCode::OK, Json(vue)).into_response(),
            traces: vec![trace],
        });
    }
    let (piece, trace_piece) = deposer_releve(tx, dossier, requete, octets, empreinte, nom).await?;
    let id = Uuid::new_v4();
    base::inserer_depot(
        tx,
        &NouveauDepot {
            id,
            compte_tresorerie_id: compte.id,
            piece_id: piece.id,
            depose_le: piece.deposee_le,
        },
    )
    .await?;
    let depot = base::lire_depot(tx, id).await?.ok_or_else(donnee)?;
    let mut trace = Trace::new(PorteeAudit::Dossier(dossier.id()), "depot_releve", id);
    let mut champ =
        |nom: &'static str, valeur: String| trace.champs.push((nom, None, Some(valeur)));
    champ("compte_tresorerie", compte.id.to_string());
    champ("piece", piece.id.to_string());
    champ("empreinte", empreinte.to_string());
    champ("taille", piece.taille.to_string());
    champ("statut_probant", depot.statut_probant.clone());
    Ok(Resultat {
        reponse: (StatusCode::CREATED, Json(vue_depot(&depot)?)).into_response(),
        traces: vec![trace_piece, trace],
    })
}

/// `?compte=<compte de trésorerie>` facultatif ; les 100 plus récents.
async fn lister_depots(
    tx: &mut TransactionTenant,
    requete: &Requete<'_>,
) -> Result<Resultat, ErreurApi> {
    let lus = parametres(requete.parametres, &["compte"])?;
    let compte = valeur(&lus, "compte")
        .map(|v| Uuid::try_parse(v).map_err(|_| ErreurApi::RequeteInvalide))
        .transpose()?;
    let depots = base::lister_depots(tx, compte, DEPOTS_MAX).await?;
    let vues = depots
        .iter()
        .map(vue_depot)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Resultat::lecture(Json(vues).into_response()))
}

async fn lire_depot(
    tx: &mut TransactionTenant,
    requete: &Requete<'_>,
) -> Result<Resultat, ErreurApi> {
    sans_parametres(requete)?;
    let id = id_chemin(requete.chemin, "depot")?;
    let depot = base::lire_depot(tx, id)
        .await?
        .ok_or(ErreurApi::Introuvable)?;
    Ok(Resultat::lecture(Json(vue_depot(&depot)?).into_response()))
}

// --- Analyse (aperçu, sans écriture) ----------------------------------------------------------

#[derive(Debug, Serialize)]
struct VueColonne {
    champ: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    rang: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    titre: Option<String>,
}

#[derive(Debug, Serialize)]
struct VueAlerte {
    ligne: usize,
    champ: &'static str,
    code: &'static str,
}

#[derive(Debug, Serialize)]
struct VueAlerteLigne {
    champ: &'static str,
    code: &'static str,
}

/// Ligne lue. Chaque texte tiré du fichier est accompagné des alertes de `formats` sur sa ligne
/// (`alertes`) : il n'est jamais rendu sans elles (F7.6, F11.8).
#[derive(Debug, Serialize)]
struct VueLigne {
    ligne: usize,
    date: String,
    date_valeur: Option<String>,
    libelle: String,
    reference: Option<String>,
    montant: String,
    solde: Option<String>,
    devise: Option<String>,
    identifiant_transaction: Option<String>,
    alertes: Vec<VueAlerteLigne>,
}

/// Totaux calculés en décimal exact (montant positif = entrée d'argent, ADR 0034 point 5) ;
/// absents en cas de dépassement de la précision décimale.
#[derive(Debug, Serialize)]
struct VueTotaux {
    entrees: Option<String>,
    sorties: Option<String>,
    net: Option<String>,
}

#[derive(Debug, Serialize)]
struct VueAnalyse {
    depot: String,
    profil: String,
    version: i32,
    empreinte: String,
    statut_probant: &'static str,
    colonnes: Vec<VueColonne>,
    nombre_lignes: usize,
    totaux: VueTotaux,
    premiere_date: Option<String>,
    derniere_date: Option<String>,
    /// Solde lu sur la première et la dernière ligne, dans l'ordre du fichier (colonne de solde
    /// du profil), tel quel : aucun contrôle ici (PR 12b).
    premier_solde: Option<String>,
    dernier_solde: Option<String>,
    nombre_alertes: usize,
    alertes: Vec<VueAlerte>,
    lignes: Vec<VueLigne>,
}

fn decimal(d: Decimal) -> String {
    d.normalize().to_string()
}

fn vue_colonne(champ: formats::releve::Champ, colonne: &Colonne) -> VueColonne {
    let (rang, titre) = match colonne {
        Colonne::Rang(r) => (Some(*r), None),
        Colonne::Titre(t) => (None, Some(t.clone())),
    };
    VueColonne {
        champ: champ.nom(),
        rang,
        titre,
    }
}

fn colonnes(profil: &ProfilReleve) -> Vec<VueColonne> {
    use formats::releve::Champ;
    let mut v: Vec<VueColonne> = profil
        .colonnes()
        .iter()
        .map(|(champ, col)| vue_colonne(*champ, col))
        .collect();
    match profil.montants() {
        Montants::Signe(c) => v.push(vue_colonne(Champ::Montant, c)),
        Montants::DebitCredit { debit, credit } => {
            v.push(vue_colonne(Champ::Debit, debit));
            v.push(vue_colonne(Champ::Credit, credit));
        }
    }
    v
}

fn totaux(releve: &Releve) -> VueTotaux {
    let mut entrees = Some(Decimal::ZERO);
    let mut sorties = Some(Decimal::ZERO);
    for m in releve.mouvements() {
        if m.montant.is_sign_negative() {
            sorties = sorties.and_then(|s| s.checked_add(m.montant.abs()));
        } else {
            entrees = entrees.and_then(|e| e.checked_add(m.montant));
        }
    }
    let net = entrees.zip(sorties).and_then(|(e, s)| e.checked_sub(s));
    VueTotaux {
        entrees: entrees.map(decimal),
        sorties: sorties.map(decimal),
        net: net.map(decimal),
    }
}

fn vue_analyse(
    depot: &DepotLu,
    profil_lu: &ProfilLu,
    profil: &ProfilReleve,
    releve: &Releve,
    apercu: usize,
) -> VueAnalyse {
    let lignes: Vec<VueLigne> = releve
        .lignes()
        .iter()
        .take(apercu)
        .map(|l| {
            let m = l.mouvement();
            VueLigne {
                ligne: l.ligne(),
                date: interne::date(m.date),
                date_valeur: m.date_valeur.map(interne::date),
                libelle: m.libelle.clone(),
                reference: m.reference.clone(),
                montant: decimal(m.montant),
                solde: m.solde.map(decimal),
                devise: m.devise.clone(),
                identifiant_transaction: m.identifiant_transaction.clone(),
                alertes: releve
                    .alertes()
                    .iter()
                    .filter(|a| a.ligne == l.ligne())
                    .map(|a| VueAlerteLigne {
                        champ: a.champ.nom(),
                        code: a.nature.code(),
                    })
                    .collect(),
            }
        })
        .collect();
    let premiere_date = releve.mouvements().map(|m| m.date).min().map(interne::date);
    let derniere_date = releve.mouvements().map(|m| m.date).max().map(interne::date);
    VueAnalyse {
        depot: depot.id.to_string(),
        profil: profil_lu.id.to_string(),
        version: profil_lu.version,
        empreinte: releve.empreinte().to_string(),
        statut_probant: STATUT_PROBANT,
        colonnes: colonnes(profil),
        nombre_lignes: releve.lignes().len(),
        totaux: totaux(releve),
        premiere_date,
        derniere_date,
        premier_solde: releve
            .mouvements()
            .next()
            .and_then(|m| m.solde)
            .map(decimal),
        dernier_solde: releve
            .mouvements()
            .last()
            .and_then(|m| m.solde)
            .map(decimal),
        nombre_alertes: releve.alertes().len(),
        alertes: releve
            .alertes()
            .iter()
            .take(ALERTES_MAX)
            .map(|a| VueAlerte {
                ligne: a.ligne,
                champ: a.champ.nom(),
                code: a.nature.code(),
            })
            .collect(),
        lignes,
    }
}

/// Analyse d'un dépôt avec une version de profil (la dernière par défaut) : aperçu, **aucune
/// écriture** (ni lignes, ni audit). Paramètres : `version`, `lignes` (0 à 100, 20 par défaut).
async fn analyser(
    tx: &mut TransactionTenant,
    dossier: &DossierAutorise,
    requete: &Requete<'_>,
) -> Result<Resultat, ErreurApi> {
    let lus = parametres(requete.parametres, &["version", "lignes"])?;
    let version = valeur(&lus, "version")
        .map(|v| entier::<i32>(v, 1..=u64::from(i32::MAX.unsigned_abs())))
        .transpose()?;
    let apercu = valeur(&lus, "lignes")
        .map(|v| entier::<usize>(v, 0..=APERCU_MAX as u64))
        .transpose()?
        .unwrap_or(APERCU_PAR_DEFAUT);
    let depot = base::lire_depot(tx, id_chemin(requete.chemin, "depot")?)
        .await?
        .ok_or(ErreurApi::Introuvable)?;
    let profil_lu = base::lire_profil(tx, id_chemin(requete.chemin, "profil")?, version)
        .await?
        .ok_or(ErreurApi::Introuvable)?;
    let profil = profil_stocke(&profil_lu)?;
    let piece = lire_piece(tx, depot.piece_id).await?.ok_or_else(donnee)?;
    let octets = match relire(dossier, requete, &piece).await? {
        Ok(octets) => octets,
        Err(alerte) => return Ok(alerte),
    };
    // Analyse bornée, hors de la boucle asynchrone (jusqu'à 5 Mio et 20 000 lignes).
    let limites = requete.releves.limites;
    let profil_analyse = profil.clone();
    let releve = tokio::task::spawn_blocking(move || {
        formats::releve::analyser(&octets, &profil_analyse, &limites)
    })
    .await
    .map_err(|_| ErreurApi::Interne(CategorieErreur::Panique))?
    .map_err(|e| refus_fichier(&e))?;
    let vue = vue_analyse(&depot, &profil_lu, &profil, &releve, apercu);
    Ok(Resultat::lecture(Json(vue).into_response()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn limites_par_defaut() -> Limites {
        ReglagesReleves::default().limites
    }

    #[test]
    fn valeurs_initiales_de_l_adr() {
        let r = ReglagesReleves::default();
        assert_eq!(r.limites.taille_fichier, 5 * 1024 * 1024);
        assert_eq!(r.limites.lignes, 20_000);
        assert_eq!(r.limites.colonnes, 50);
        assert_eq!(r.limites.longueur_ligne, 64 * 1024);
        assert_eq!(r.limites.longueur_cellule, 1000);
        assert_eq!(r.profils_max, 20);
        assert_eq!(
            limites_route(Cas::Deposer),
            Some((5 * 1024 * 1024, r.delai_depot))
        );
        for cas in [
            Cas::ListerProfils,
            Cas::CreerProfil,
            Cas::LireProfil,
            Cas::VersionnerProfil,
            Cas::ListerDepots,
            Cas::LireDepot,
            Cas::Analyser,
        ] {
            assert_eq!(limites_route(cas), None);
        }
    }

    fn limites_route(cas: Cas) -> Option<(usize, Duration)> {
        super::limites(cas, &ReglagesReleves::default())
    }

    #[test]
    fn type_texte_reconnu_par_le_contenu() {
        let l = limites_par_defaut();
        assert!(reconnaitre_texte(b"Date;Montant\r\n01/02/2026;1000\r\n", &l).is_ok());
        // windows-1252 : « é » sur un octet.
        assert!(reconnaitre_texte(b"Libell\xe9;1\n", &l).is_ok());
        // UTF-8 avec BOM.
        assert!(reconnaitre_texte("\u{feff}Libellé;1\n".as_bytes(), &l).is_ok());
        for refuse in [
            &b"%PDF-1.7\nDate;Montant\n"[..],
            b"\x89PNG\r\n\x1a\n",
            b"Date\0;Montant\n",
            b"Date;\x07Montant\n",
            b"\x81\x8d\x8f\x90\x9d\n",
        ] {
            assert_eq!(
                reconnaitre_texte(refuse, &l),
                Err(ErreurApi::TypeNonAdmis),
                "{refuse:?}"
            );
        }
        let longue = format!("a;b\n{}\n", "x".repeat(l.longueur_ligne + 1));
        assert_eq!(
            reconnaitre_texte(longue.as_bytes(), &l),
            Err(ErreurApi::Releve {
                motif: MotifReleve::LigneTropLongue,
                ligne: Some(2)
            })
        );
    }

    #[test]
    fn chaque_refus_de_formats_a_son_motif() {
        use formats::releve::Champ;
        assert_eq!(
            refus_fichier(&ErreurReleve::DateInvalide {
                ligne: 7,
                champ: Champ::Date
            }),
            ErreurApi::Releve {
                motif: MotifReleve::DateInvalide,
                ligne: Some(7)
            }
        );
        assert_eq!(
            refus_fichier(&ErreurReleve::TitresAbsents),
            ErreurApi::Releve {
                motif: MotifReleve::TitresAbsents,
                ligne: None
            }
        );
        assert_eq!(
            refus_profil(&ErreurProfil::FormatDate(ErreurFormatDate::SansAnnee)),
            ErreurApi::Releve {
                motif: MotifReleve::ProfilDateSansAnnee,
                ligne: None
            }
        );
    }

    const PROFIL: &str = r#"{"nom":" Banque test ","profil":{"encodage":"UTF-8","delimiteur":";",
        "lignes_entete":1,"format_date":"%d/%m/%Y","separateur_decimal":",",
        "colonnes":{"date":"Date","libelle":1,"montant":"Montant"}}}"#;

    #[test]
    fn profil_canonique_et_refus() {
        let p = profil_du_corps(PROFIL.as_bytes()).unwrap();
        assert_eq!(p.nom, "Banque test");
        // Forme canonique : encodage normalisé, guillemet par défaut explicite.
        assert!(
            p.definition.contains(r#""encodage":"utf-8""#),
            "{}",
            p.definition
        );
        assert!(
            p.definition.contains(r#""guillemet":"\"""#),
            "{}",
            p.definition
        );
        assert_eq!(
            profil_du_corps(PROFIL.replace("UTF-8", "utf-16").as_bytes()).err(),
            Some(refus(MotifReleve::ProfilEncodage, None))
        );
        assert_eq!(
            profil_du_corps(PROFIL.replace("%d/%m/%Y", "%d/%m").as_bytes()).err(),
            Some(refus(MotifReleve::ProfilDateSansAnnee, None))
        );
        assert_eq!(
            profil_du_corps(
                PROFIL
                    .replace(r#""libelle":1"#, r#""libelle":-1"#)
                    .as_bytes()
            )
            .err(),
            Some(refus(MotifReleve::ProfilIllisible, None))
        );
        assert_eq!(
            profil_du_corps(PROFIL.replace(" Banque test ", "  ").as_bytes()).err(),
            Some(refus(MotifReleve::ProfilNom, None))
        );
        assert_eq!(
            profil_du_corps(PROFIL.replace(" Banque test ", "Banque\u{202E}").as_bytes()).err(),
            Some(refus(MotifReleve::ProfilNom, None))
        );
        assert_eq!(
            profil_du_corps(br#"{"nom":"x","profil":{},"autre":1}"#).err(),
            Some(ErreurApi::RequeteInvalide)
        );
    }

    #[test]
    fn entiers_stricts() {
        assert_eq!(entier::<usize>("20", 0..=100), Ok(20));
        for refuse in ["", "-1", "+1", "101", "1e2", "99999999999"] {
            assert_eq!(
                entier::<usize>(refuse, 0..=100),
                Err(ErreurApi::RequeteInvalide),
                "{refuse}"
            );
        }
    }
}
