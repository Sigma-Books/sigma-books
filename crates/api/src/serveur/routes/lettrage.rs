//! Lettrage manuel, dé-lettrage par rupture, lettres (L3-18, PR 9a de `docs/lots/L3.md` ;
//! ADR 0032 ; spec §10.1, §20). Le lettrage automatique (L3-19) est de la PR 9b.
//!
//! # Principe : ajout seul, règles de `compta-core`
//! Aucune écriture validée n'est modifiée (règle absolue 5) : un lettrage s'inscrit dans les tables
//! en ajout seul de la migration `0520` (`lettrage`, `lettrage_lien`, `lettrage_rupture`) ; le
//! dé-lettrage est une **rupture** ajoutée, jamais une suppression (B2). Les règles sont celles du
//! crate pur `compta_core::lettrage` : la persistance rebâtit un [`RegistreLettrage`] par compte
//! (lignes validées, opérations **rejouées par rang d'opération**), puis `imputer` (lettrage « au
//! kilomètre » : un appel lettre N lignes, du plus ancien au plus récent) et `rompre` décident.
//!
//! # Conditions (ADR 0032 point 2 ; Tryton `move.py:646-690`)
//! Lignes **validées** du dossier, d'un **même compte**, **lettrable** selon les données de
//! `syscohada` (marque `lettrable`, jamais un numéro dans le code : règle absolue 9), d'un **même
//! tiers résolu** (ADR 0031 ; [`base::tiers_resolus`]), au moins une au débit et une au crédit,
//! non soldées. **Aucun contrôle de période ni de verrou** au lettrage (consensus des moteurs, A9).
//! Une ligne en devise étrangère n'est lettrée que dans le cas admis par `compta-core` (422
//! `lettrage_devise_etrangere` sinon : écarts de change en L6).
//!
//! # Dé-lettrage (B2, option 1 ; spec §10.1)
//! Refusé (409 `lettrage_exercice_clos`) si la **date de l'opération** de lettrage tombe dans un
//! exercice clos : choix Sigma documenté (ADR 0032 point 5), aucun moteur ne contrôle la période.
//! Un lettrage fait alors que son exercice était déjà clos est **accepté** (aucun contrôle de
//! période au lettrage) avec l'avertissement `lettrage_non_rompable` (cas limite du point 5).
//!
//! # Date, lettre, rang (ADR 0032 points 3 et 4)
//! La date d'une opération est le jour du dossier dans son fuseau (ADR 0027), lu **après** les
//! verrous ; l'utilisateur ne la saisit pas. La lettre (`A`…`Z`, `AA`…) et le rang d'opération
//! sont tirés du compteur du compte (`compteur_lettrage`), jamais réutilisés.
//!
//! # Verrous (ADR 0032 point 8)
//! Lettrage : compte puis tiers `FOR SHARE` (rang 3), puis compteur du compte `FOR UPDATE`
//! (rang 7) ; rupture : calendrier `FOR SHARE` (rang 1, statut de l'exercice), puis compteur (rang
//! 7). Résiduels et tiers résolus sont **relus après** les verrous : deux lettrages concurrents de
//! la même ligne se suivent et ne dépassent jamais son montant ; deux lettrages concurrents du même
//! compte ne reçoivent jamais la même lettre. Le déclencheur `lettrage_lien_controle` (migration
//! `0520`) est la seconde barrière.
//!
//! # Défense en profondeur (ADR 0011)
//! Chaque cas relit le dossier **visible** et le rôle de l'appartenance active en base, en plus de
//! Cedar (`Lettrer`, `Delettrer` : administrateur, valideur, collaborateur ; lecture :
//! `ConsulterEditionsTiers`, les quatre rôles du cabinet). Un lettrage, un tiers ou un compte
//! d'un autre dossier est absent (404) ; une ligne d'un autre dossier est inconnue (422
//! `lettrage_ligne_inconnue`, comme une ligne inexistante).
//!
//! # Audit (ADR 0032 point 9)
//! Une entrée chaînée par mutation, **sans valeur personnelle** : identifiants (lettrage, compte),
//! lettre, rang, origine, nombres de liens et de lignes ; jamais un montant, un libellé ni le motif.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::OnceLock;

use authz::{Action, Role};
use axum::Json;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use compta_core::lettrage::Operation;
use compta_core::{
    DeviseTenue, ErreurLettrage, IdLigne, IdTiers, Lettre, Lien, LigneLettrable, Montant,
    MotifRupture, NumeroCompte, Origine, RegistreLettrage,
};
use serde::{Deserialize, Serialize};
use time::Date;
use uuid::Uuid;

use super::brouillard::validation::ligne_depuis_stockee;
use super::{ActionRoute, CasUsage, Methode, Module, Portee, Requete, Resultat, Route};
use crate::base::TransactionTenant;
use crate::base::audit::PorteeAudit;
use crate::base::brouillard::LigneStockee;
use crate::base::lettrage::{
    self as base, Compteur, LettrageLu, LigneLue, NouveauLettrage, NouvelleRupture, RefLigne,
};
use crate::base::organisation::{lire_appartenance, lire_dossier_visible};
use crate::base::validation::{verrouiller_calendrier, verrouiller_comptes_et_tiers};
use crate::pieces::interne;
use crate::serveur::audit::Trace;
use crate::serveur::erreur::{CategorieErreur, ErreurApi, MotifConflit, MotifValidation};
use crate::serveur::pipeline::{DossierAutorise, Garde, id_chemin};

/// Cas d'usage du module.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cas {
    /// Lettrer N lignes validées d'un même compte (« au kilomètre »).
    Lettrer,
    /// Rompre un lettrage (dé-lettrage, ligne de rupture).
    Rompre,
    /// Lignes lettrables d'un tiers : résiduels, lettres, lettrages et ruptures.
    LireTiers,
    /// Lignes d'un compte lettrable (585, compte d'attente…) : mêmes informations.
    LireCompte,
}

/// Tranche du module.
pub const MODULE: Module = Module {
    nom: "lettrage",
    routes: ROUTES,
    champs_journal: &[],
};

const fn route(methode: Methode, chemin: &'static str, action: Action, cas: Cas) -> Route {
    Route {
        methode,
        chemin,
        action: ActionRoute::Cabinet(action),
        portee: Portee::Dossier,
        cas_usage: CasUsage::Lettrage(cas),
    }
}

/// Routes du module (actions Cedar de la PR 6, `roles/lettrage.cedar` et `roles/tiers.cedar`).
pub const ROUTES: &[Route] = &[
    route(
        Methode::Post,
        "/api/cabinets/{cabinet}/dossiers/{dossier}/lettrage/lettrages",
        Action::Lettrer,
        Cas::Lettrer,
    ),
    route(
        Methode::Post,
        "/api/cabinets/{cabinet}/dossiers/{dossier}/lettrage/lettrages/{lettrage}/rupture",
        Action::Delettrer,
        Cas::Rompre,
    ),
    route(
        Methode::Get,
        "/api/cabinets/{cabinet}/dossiers/{dossier}/lettrage/tiers/{tiers}",
        Action::ConsulterEditionsTiers,
        Cas::LireTiers,
    ),
    route(
        Methode::Get,
        "/api/cabinets/{cabinet}/dossiers/{dossier}/lettrage/comptes/{compte}",
        Action::ConsulterEditionsTiers,
        Cas::LireCompte,
    ),
];

/// Lignes au plus dans un lettrage (borne de la requête, F2.16 ; une écriture a au plus 200
/// lignes).
pub const LIGNES_MAX: usize = 200;
/// Lignes au plus par page de lecture (F2.16 : pagination imposée, curseur `apres`).
pub const LIGNES_LUES_MAX: usize = 5_000;
/// Lignes par page de lecture sans paramètre `limite`.
pub const LIGNES_LUES_DEFAUT: usize = 1_000;

/// Rôles admis à lettrer et dé-lettrer (spec §20 « Lettrer / délettrer »).
const ROLES_LETTRAGE: &[Role] = &[Role::Administrateur, Role::Valideur, Role::Collaborateur];
/// Rôles admis à lire (comme « Consulter éditions et états »).
const ROLES_LECTURE: &[Role] = &[
    Role::Administrateur,
    Role::Valideur,
    Role::Collaborateur,
    Role::Auditeur,
];

/// Exécute un cas d'usage du module dans la transaction du pipeline.
pub(crate) async fn executer(
    cas: Cas,
    tx: &mut TransactionTenant,
    garde: &Garde,
    requete: &Requete<'_>,
) -> Result<Resultat, ErreurApi> {
    let dossier = dossier_visible(tx, garde).await?;
    match cas {
        Cas::Lettrer => {
            exiger_role(tx, ROLES_LETTRAGE).await?;
            lettrer(tx, &dossier, requete).await
        }
        Cas::Rompre => {
            exiger_role(tx, ROLES_LETTRAGE).await?;
            rompre(tx, &dossier, requete).await
        }
        Cas::LireTiers => {
            exiger_role(tx, ROLES_LECTURE).await?;
            lire_tiers(tx, &dossier, requete).await
        }
        Cas::LireCompte => {
            exiger_role(tx, ROLES_LECTURE).await?;
            lire_compte(tx, &dossier, requete).await
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

fn sans_parametres(requete: &Requete<'_>) -> Result<(), ErreurApi> {
    match requete.parametres {
        None | Some("") => Ok(()),
        Some(_) => Err(ErreurApi::RequeteInvalide),
    }
}

/// Référentiel officiel embarqué (`syscohada`), chargé une fois : marque `lettrable`.
fn officiel() -> Result<&'static syscohada::Referentiel, ErreurApi> {
    static REFERENTIEL: OnceLock<Option<syscohada::Referentiel>> = OnceLock::new();
    REFERENTIEL
        .get_or_init(|| syscohada::Referentiel::officiel().ok())
        .as_ref()
        .ok_or_else(donnee)
}

/// Refus de `compta-core`, en réponse : 422 pour une sélection invalide en soi, 409 pour l'état
/// des lignes ou du lettrage ; tout autre motif ne peut venir que d'une donnée stockée incohérente
/// (500, catégorie `donnee`).
fn refus(e: &ErreurLettrage) -> ErreurApi {
    match e {
        ErreurLettrage::LigneInconnue(_) => validation(MotifValidation::LettrageLigneInconnue),
        ErreurLettrage::AutreCompte(_) => validation(MotifValidation::LettrageComptesDifferents),
        ErreurLettrage::CompteNonLettrable(_) => {
            validation(MotifValidation::LettrageCompteNonLettrable)
        }
        ErreurLettrage::LigneEnDouble(_) => validation(MotifValidation::LettrageLigneEnDouble),
        ErreurLettrage::SensManquant(_) => validation(MotifValidation::LettrageSensManquant),
        ErreurLettrage::TiersDifferents => validation(MotifValidation::LettrageTiersDifferents),
        ErreurLettrage::DeviseHorsL3 => validation(MotifValidation::LettrageDeviseEtrangere),
        ErreurLettrage::MotifInvalide => validation(MotifValidation::LettrageMotifInvalide),
        ErreurLettrage::LigneSoldee(_) | ErreurLettrage::ResiduelDepasse(_) => {
            conflit(MotifConflit::LettrageLigneSoldee)
        }
        ErreurLettrage::ExerciceClos(_) => conflit(MotifConflit::LettrageExerciceClos),
        ErreurLettrage::DejaRompu(_) => conflit(MotifConflit::LettrageDejaRompu),
        _ => donnee(),
    }
}

/// Jour du dossier dans son fuseau, relu sur l'horloge **après** les verrous (comme la
/// validation).
fn jour_du_dossier(dossier: &DossierAutorise, requete: &Requete<'_>) -> Result<Date, ErreurApi> {
    Ok(dossier
        .contexte()
        .date_du_jour(requete.horloge.maintenant())?)
}

const fn nom_origine(o: Origine) -> &'static str {
    match o {
        Origine::Manuelle => "manuelle",
        Origine::Automatique => "automatique",
        Origine::PropositionValidee => "proposition_validee",
    }
}

fn origine_lue(texte: &str) -> Result<Origine, ErreurApi> {
    match texte {
        "manuelle" => Ok(Origine::Manuelle),
        "automatique" => Ok(Origine::Automatique),
        "proposition_validee" => Ok(Origine::PropositionValidee),
        _ => Err(donnee()),
    }
}

fn sens_stocke(texte: &str) -> Result<&'static str, ErreurApi> {
    match texte {
        "debit" => Ok("debit"),
        "credit" => Ok("credit"),
        _ => Err(donnee()),
    }
}

// --- Registre d'un compte, rebâti de la persistance ------------------------------------------

/// Registre d'un compte rebâti de la base : les lignes demandées, les lettrages (rompus compris)
/// qui les citent et toutes les lignes de ces lettrages. Les résiduels des lignes demandées y sont
/// exacts : tout lien qui les cite est chargé.
struct Charge {
    registre: RegistreLettrage,
    ids: BTreeMap<RefLigne, IdLigne>,
    refs: BTreeMap<IdLigne, RefLigne>,
    lignes: BTreeMap<RefLigne, LigneLue>,
    lettrages: Vec<LettrageLu>,
}

impl Charge {
    fn id(&self, r: &RefLigne) -> Result<IdLigne, ErreurApi> {
        self.ids.get(r).copied().ok_or_else(donnee)
    }

    fn reference(&self, id: IdLigne) -> Result<RefLigne, ErreurApi> {
        self.refs.get(&id).copied().ok_or_else(donnee)
    }

    /// Plus haut rang d'opération relu.
    fn dernier_rang(&self) -> i64 {
        self.lettrages
            .iter()
            .flat_map(|l| std::iter::once(l.rang).chain(l.rupture.as_ref().map(|r| r.rang)))
            .max()
            .unwrap_or(0)
    }
}

fn id_ligne(l: &LigneLue) -> Result<IdLigne, ErreurApi> {
    let rang = u64::try_from(l.rang).map_err(|_| donnee())?;
    let position = u16::try_from(l.position).map_err(|_| donnee())?;
    IdLigne::nouveau(rang, position).map_err(|_| donnee())
}

/// Rebâtit le registre du compte `numero` à partir des lignes `initiales` (toutes sur ce compte).
async fn charger(
    tx: &mut TransactionTenant,
    tenue: &DeviseTenue,
    numero: &NumeroCompte,
    compte_id: Uuid,
    initiales: Vec<LigneLue>,
) -> Result<Charge, ErreurApi> {
    let lettrable = officiel()?.est_lettrable(numero);
    let mut registre =
        RegistreLettrage::ouvrir(numero.clone(), lettrable).map_err(|e| refus(&e))?;
    let refs_initiales: Vec<RefLigne> = initiales.iter().map(LigneLue::reference).collect();
    let ids_lettrages = base::lettrages_citant(tx, &refs_initiales).await?;
    let lettrages = base::lettrages(tx, &ids_lettrages).await?;
    if lettrages.iter().any(|l| l.compte_id != compte_id) {
        return Err(donnee());
    }
    let mut lignes: BTreeMap<RefLigne, LigneLue> =
        initiales.into_iter().map(|l| (l.reference(), l)).collect();
    let manquantes: Vec<RefLigne> = lettrages
        .iter()
        .flat_map(|l| l.liens.iter().flat_map(|k| [k.debit, k.credit]))
        .filter(|r| !lignes.contains_key(r))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    if !manquantes.is_empty() {
        let lues = base::lignes(tx, &manquantes).await?;
        if lues.len() != manquantes.len() {
            return Err(donnee());
        }
        lignes.extend(lues.into_iter().map(|l| (l.reference(), l)));
    }
    let tiers: Vec<Uuid> = lignes
        .values()
        .filter_map(|l| l.tiers)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let resolus = base::tiers_resolus(tx, &tiers).await?;

    let mut ids = BTreeMap::new();
    let mut refs = BTreeMap::new();
    for l in lignes.values() {
        let id = id_ligne(l)?;
        let stockee = LigneStockee {
            compte: l.compte.clone(),
            sens: sens_stocke(&l.sens)?,
            montant: l.montant.clone(),
            devise: l.devise.clone(),
            tiers: l.tiers,
            etiquette: None,
            // Reconstitution d'une ligne validée pour `compta-core` : l'échéance n'y entre pas
            // (l'ordre d'imputation par échéance sera branché avec l'échéancier, PR 8).
            date_echeance: None,
        };
        let ligne = ligne_depuis_stockee(&stockee, None, tenue)?;
        let resolu = l
            .tiers
            .map(|t| {
                let r = resolus.get(&t).ok_or_else(donnee)?;
                IdTiers::parse(&r.to_string()).map_err(|_| donnee())
            })
            .transpose()?;
        let lettrable = LigneLettrable::nouvelle(id, &ligne, l.date, resolu, None, None)
            .map_err(|_| donnee())?;
        registre.ajouter_ligne(lettrable).map_err(|e| refus(&e))?;
        ids.insert(l.reference(), id);
        refs.insert(id, l.reference());
    }

    let mut operations = Vec::new();
    for l in &lettrages {
        let lettre = Lettre::parse(&l.lettre).map_err(|_| donnee())?;
        let mut liens = Vec::with_capacity(l.liens.len());
        for k in &l.liens {
            let montant = Montant::parse(&k.montant, tenue.devise()).map_err(|_| donnee())?;
            let debit = ids.get(&k.debit).copied().ok_or_else(donnee)?;
            let credit = ids.get(&k.credit).copied().ok_or_else(donnee)?;
            liens.push(Lien::nouveau(debit, credit, montant.valeur()).map_err(|_| donnee())?);
        }
        let rang = u64::try_from(l.rang).map_err(|_| donnee())?;
        operations.push((
            rang,
            Operation::Lettrage {
                lettre,
                date: l.date,
                origine: origine_lue(&l.origine)?,
                liens,
            },
        ));
        if let Some(r) = &l.rupture {
            let rang = u64::try_from(r.rang).map_err(|_| donnee())?;
            operations.push((
                rang,
                Operation::Rupture {
                    lettre,
                    date: r.date,
                    motif: MotifRupture::parse(&r.motif).map_err(|_| donnee())?,
                },
            ));
        }
    }
    // Relecture par rang d'opération, avec les contrôles d'un lettrage et d'une rupture :
    // une donnée stockée qui les viole est une incohérence (500).
    registre.rejouer(operations).map_err(|_| donnee())?;
    Ok(Charge {
        registre,
        ids,
        refs,
        lignes,
        lettrages,
    })
}

/// Le compteur relu sous verrou couvre les opérations relues ; sinon la base est incohérente.
fn controler_compteur(compteur: &Compteur, charge: &Charge) -> Result<(), ErreurApi> {
    if charge.dernier_rang() > compteur.dernier_rang {
        return Err(donnee());
    }
    Ok(())
}

// --- Vues -------------------------------------------------------------------------------------

#[derive(Debug, Serialize)]
struct VueRefLigne {
    ecriture: String,
    position: i16,
}

fn vue_ref((ecriture, position): RefLigne) -> VueRefLigne {
    VueRefLigne {
        ecriture: ecriture.to_string(),
        position,
    }
}

#[derive(Debug, Serialize)]
struct VueLien {
    debit: VueRefLigne,
    credit: VueRefLigne,
    montant: String,
}

#[derive(Debug, Serialize)]
struct VueRupture {
    id: String,
    date: String,
    auteur: String,
    motif: String,
}

#[derive(Debug, Serialize)]
struct VueLettrage {
    id: String,
    lettre: String,
    date: String,
    origine: String,
    auteur: Option<String>,
    /// Lettrage non rompu dont toutes les lignes sont soldées (calculé, jamais stocké).
    total: bool,
    /// Plus haute date comptable des lignes liées : date à laquelle le lettrage compte dans les
    /// états à une date (ADR 0032, amendement du 2026-11-27).
    date_effet: Option<String>,
    rupture: Option<VueRupture>,
    liens: Vec<VueLien>,
}

#[derive(Debug, Serialize)]
struct VueLigne {
    ecriture: String,
    numero: String,
    position: i16,
    date: String,
    sens: String,
    montant: String,
    /// Résiduel : montant moins les liens non rompus (calculé, jamais stocké).
    residuel: String,
    /// Lettres des lettrages non rompus qui citent la ligne.
    lettres: Vec<String>,
    tiers: Option<String>,
}

#[derive(Debug, Serialize)]
struct VueReferenceCompte {
    id: String,
    numero: String,
}

#[derive(Debug, Serialize)]
struct VueCompte {
    compte: VueReferenceCompte,
    lignes: Vec<VueLigne>,
    lettrages: Vec<VueLettrage>,
}

/// Texte décimal exact, sans zéro final (le décimal de `compta-core` affiché tel quel).
fn texte_decimal(d: impl std::fmt::Display) -> String {
    crate::base::brouillard::decimal_canonique(&d.to_string())
}

fn vue_ligne(charge: &Charge, r: RefLigne) -> Result<VueLigne, ErreurApi> {
    let l = charge.lignes.get(&r).ok_or_else(donnee)?;
    let id = charge.id(&r)?;
    let residuel = charge.registre.residuel(id).ok_or_else(donnee)?;
    Ok(VueLigne {
        ecriture: l.ecriture_id.to_string(),
        numero: l.numero.clone(),
        position: l.position,
        date: interne::date(l.date),
        sens: l.sens.clone(),
        montant: l.montant.clone(),
        residuel: texte_decimal(residuel),
        lettres: charge
            .registre
            .lettres(id)
            .iter()
            .map(ToString::to_string)
            .collect(),
        tiers: l.tiers.map(|t| t.to_string()),
    })
}

fn vue_lettrage(charge: &Charge, l: &LettrageLu) -> Result<VueLettrage, ErreurApi> {
    let lettre = Lettre::parse(&l.lettre).map_err(|_| donnee())?;
    let date_effet = l
        .liens
        .iter()
        .flat_map(|k| [k.debit, k.credit])
        .filter_map(|r| charge.lignes.get(&r).map(|x| x.date))
        .max();
    Ok(VueLettrage {
        id: l.id.to_string(),
        lettre: l.lettre.clone(),
        date: interne::date(l.date),
        origine: l.origine.clone(),
        auteur: l.auteur.map(|a| a.to_string()),
        total: charge.registre.est_total(lettre),
        date_effet: date_effet.map(interne::date),
        rupture: l.rupture.as_ref().map(|r| VueRupture {
            id: r.id.to_string(),
            date: interne::date(r.date),
            auteur: r.auteur.to_string(),
            motif: r.motif.clone(),
        }),
        liens: l
            .liens
            .iter()
            .map(|k| VueLien {
                debit: vue_ref(k.debit),
                credit: vue_ref(k.credit),
                montant: k.montant.clone(),
            })
            .collect(),
    })
}

/// Vue d'un compte : les lignes `affichees` (dans l'ordre de la chaîne) et les lettrages relus.
fn vue_compte(
    charge: &Charge,
    compte_id: Uuid,
    numero: &NumeroCompte,
    affichees: &[RefLigne],
) -> Result<VueCompte, ErreurApi> {
    Ok(VueCompte {
        compte: VueReferenceCompte {
            id: compte_id.to_string(),
            numero: numero.to_string(),
        },
        lignes: affichees
            .iter()
            .map(|r| vue_ligne(charge, *r))
            .collect::<Result<_, _>>()?,
        lettrages: charge
            .lettrages
            .iter()
            .map(|l| vue_lettrage(charge, l))
            .collect::<Result<_, _>>()?,
    })
}

// --- Lettrer ----------------------------------------------------------------------------------

/// Ligne citée par le corps : écriture validée et position (à partir de 1).
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CorpsLigne {
    ecriture: String,
    position: u16,
}

/// Corps du lettrage : les lignes à lettrer ensemble (ordre indifférent).
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CorpsLettrage {
    lignes: Vec<CorpsLigne>,
}

#[derive(Debug, Serialize)]
struct VueLettrageCree {
    id: String,
    compte: VueReferenceCompte,
    lettre: String,
    date: String,
    origine: &'static str,
    total: bool,
    liens: Vec<VueLien>,
    lignes: Vec<VueLigne>,
    /// Avertissements, jamais des refus.
    avertissements: Vec<&'static str>,
}

fn refs_du_corps(c: &CorpsLettrage) -> Result<Vec<RefLigne>, ErreurApi> {
    if c.lignes.len() > LIGNES_MAX {
        return Err(validation(MotifValidation::LettrageTropDeLignes));
    }
    c.lignes
        .iter()
        .map(|l| {
            let ecriture = Uuid::try_parse(&l.ecriture).map_err(|_| ErreurApi::RequeteInvalide)?;
            let position = i16::try_from(l.position)
                .ok()
                .filter(|p| *p >= 1)
                .ok_or(ErreurApi::RequeteInvalide)?;
            Ok((ecriture, position))
        })
        .collect()
}

#[allow(clippy::too_many_lines)]
async fn lettrer(
    tx: &mut TransactionTenant,
    dossier: &DossierAutorise,
    requete: &Requete<'_>,
) -> Result<Resultat, ErreurApi> {
    sans_parametres(requete)?;
    let c: CorpsLettrage = corps(requete.corps)?;
    let refs = refs_du_corps(&c)?;
    let tenue = dossier.contexte().tenue()?;

    // 1. Lignes citées (immuables : lues avant les verrous pour savoir quoi verrouiller).
    let lues = base::lignes(tx, &refs).await?;
    let trouvees: BTreeSet<RefLigne> = lues.iter().map(LigneLue::reference).collect();
    if refs.iter().any(|r| !trouvees.contains(r)) {
        return Err(validation(MotifValidation::LettrageLigneInconnue));
    }
    let Some(premiere) = lues.first() else {
        return Err(validation(MotifValidation::LettrageSensManquant));
    };
    let (compte_id, compte) = (premiere.compte_id, premiere.compte.clone());
    if lues.iter().any(|l| l.compte_id != compte_id) {
        return Err(validation(MotifValidation::LettrageComptesDifferents));
    }
    let numero = NumeroCompte::parse(&compte).map_err(|_| donnee())?;
    if !officiel()?.est_lettrable(&numero) {
        return Err(validation(MotifValidation::LettrageCompteNonLettrable));
    }

    // 2. Verrous : compte puis tiers `FOR SHARE` (rang 3), compteur du compte (rang 7).
    let tiers: Vec<Uuid> = lues.iter().filter_map(|l| l.tiers).collect();
    verrouiller_comptes_et_tiers(tx, std::slice::from_ref(&compte), &tiers).await?;
    let compteur = base::verrouiller_compteur(tx, compte_id).await?;

    // 3. Relecture après les verrous : lettrages qui citent les lignes, tiers résolus.
    let mut charge = charger(tx, &tenue, &numero, compte_id, lues).await?;
    controler_compteur(&compteur, &charge)?;
    let selection: Vec<IdLigne> = refs
        .iter()
        .map(|r| charge.id(r))
        .collect::<Result<_, _>>()?;
    let liens = charge.registre.imputer(&selection).map_err(|e| refus(&e))?;

    // 4. Lettre et rang tirés du compteur ; inscription contrôlée par le rejeu de `compta-core`.
    let lettre = match &compteur.derniere_lettre {
        None => Lettre::premiere(),
        Some(t) => Lettre::parse(t)
            .map_err(|_| donnee())?
            .suivante()
            .map_err(|_| donnee())?,
    };
    let rang = compteur.dernier_rang.checked_add(1).ok_or_else(donnee)?;
    let date = jour_du_dossier(dossier, requete)?;
    charge
        .registre
        .rejouer([(
            u64::try_from(rang).map_err(|_| donnee())?,
            Operation::Lettrage {
                lettre,
                date,
                origine: Origine::Manuelle,
                liens: liens.clone(),
            },
        )])
        .map_err(|e| refus(&e))?;

    let id = Uuid::new_v4();
    let lettre_texte = lettre.to_string();
    let persistes: Vec<(RefLigne, RefLigne, String)> = liens
        .iter()
        .map(|k| {
            Ok((
                charge.reference(k.debit())?,
                charge.reference(k.credit())?,
                texte_decimal(k.montant()),
            ))
        })
        .collect::<Result<_, ErreurApi>>()?;
    base::inserer_lettrage(
        tx,
        &NouveauLettrage {
            id,
            compte_id,
            lettre: &lettre_texte,
            rang,
            date,
            origine: nom_origine(Origine::Manuelle),
            instant: requete.horloge.maintenant(),
            liens: &persistes,
        },
    )
    .await?;
    base::avancer_compteur(
        tx,
        compte_id,
        &Compteur {
            derniere_lettre: Some(lettre_texte.clone()),
            dernier_rang: rang,
        },
    )
    .await?;

    let mut avertissements = Vec::new();
    if base::dans_un_exercice_clos(tx, date).await? {
        avertissements.push("lettrage_non_rompable");
    }
    let mut trace = Trace::new(PorteeAudit::Dossier(dossier.id()), "lettrage", id);
    trace.donnees.push(("compte", compte_id.to_string()));
    trace.donnees.push(("lettre", lettre_texte.clone()));
    trace.donnees.push(("rang", rang.to_string()));
    trace
        .donnees
        .push(("origine", nom_origine(Origine::Manuelle).to_owned()));
    trace.donnees.push(("liens", persistes.len().to_string()));
    trace.donnees.push(("lignes", refs.len().to_string()));

    let vue = VueLettrageCree {
        id: id.to_string(),
        compte: VueReferenceCompte {
            id: compte_id.to_string(),
            numero: numero.to_string(),
        },
        lettre: lettre_texte,
        date: interne::date(date),
        origine: nom_origine(Origine::Manuelle),
        total: charge.registre.est_total(lettre),
        liens: persistes
            .iter()
            .map(|(d, c, m)| VueLien {
                debit: vue_ref(*d),
                credit: vue_ref(*c),
                montant: m.clone(),
            })
            .collect(),
        lignes: refs
            .iter()
            .map(|r| vue_ligne(&charge, *r))
            .collect::<Result<_, _>>()?,
        avertissements,
    };
    Ok(Resultat {
        reponse: (StatusCode::CREATED, Json(vue)).into_response(),
        traces: vec![trace],
    })
}

// --- Rompre -----------------------------------------------------------------------------------

/// Corps de la rupture : le motif (obligatoire, 400 octets au plus).
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CorpsRupture {
    motif: String,
}

#[derive(Debug, Serialize)]
struct VueRuptureCreee {
    id: String,
    lettrage: String,
    compte: VueReferenceCompte,
    lettre: String,
    date: String,
    /// Lignes du lettrage rompu, avec leur résiduel rétabli.
    lignes: Vec<VueLigne>,
}

#[allow(clippy::too_many_lines)]
async fn rompre(
    tx: &mut TransactionTenant,
    dossier: &DossierAutorise,
    requete: &Requete<'_>,
) -> Result<Resultat, ErreurApi> {
    sans_parametres(requete)?;
    let id_lettrage = id_chemin(requete.chemin, "lettrage")?;
    let c: CorpsRupture = corps(requete.corps)?;
    let motif = MotifRupture::parse(&c.motif).map_err(|e| refus(&e))?;
    let tenue = dossier.contexte().tenue()?;
    let compte_id = base::lettrages(tx, &[id_lettrage])
        .await?
        .first()
        .map(|l| l.compte_id)
        .ok_or(ErreurApi::Introuvable)?;

    // Verrous : calendrier `FOR SHARE` (rang 1), puis compteur du compte (rang 7).
    verrouiller_calendrier(tx).await?;
    let compteur = base::verrouiller_compteur(tx, compte_id).await?;

    // Relecture après les verrous.
    let lu = base::lettrages(tx, &[id_lettrage])
        .await?
        .pop()
        .ok_or_else(donnee)?;
    let numero = base::numero_du_compte(tx, compte_id)
        .await?
        .ok_or_else(donnee)?;
    let numero = NumeroCompte::parse(&numero).map_err(|_| donnee())?;
    let refs: Vec<RefLigne> = lu
        .liens
        .iter()
        .flat_map(|k| [k.debit, k.credit])
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let lignes = base::lignes(tx, &refs).await?;
    let mut charge = charger(tx, &tenue, &numero, compte_id, lignes).await?;
    controler_compteur(&compteur, &charge)?;
    let referentiel = crate::base::referentiel::charger(tx)
        .await?
        .ok_or(ErreurApi::Introuvable)?;
    let lettre = Lettre::parse(&lu.lettre).map_err(|_| donnee())?;
    // Une rupture ne précède jamais son lettrage : si le jour du dossier est antérieur à la date
    // du lettrage (fuseau du dossier changé entre-temps, ADR 0027), la rupture prend la date du
    // lettrage ; l'audit garde le jour du dossier (aucun refus : contrôle absent de la spec et des
    // moteurs).
    let jour = jour_du_dossier(dossier, requete)?;
    let date = jour.max(lu.date);
    charge
        .registre
        .rompre(lettre, date, motif.clone(), referentiel.exercices())
        .map_err(|e| refus(&e))?;

    let id = Uuid::new_v4();
    let rang = compteur.dernier_rang.checked_add(1).ok_or_else(donnee)?;
    base::inserer_rupture(
        tx,
        &NouvelleRupture {
            id,
            lettrage_id: lu.id,
            compte_id,
            rang,
            date,
            motif: motif.as_str(),
            instant: requete.horloge.maintenant(),
        },
    )
    .await?;
    base::avancer_compteur(
        tx,
        compte_id,
        &Compteur {
            derniere_lettre: compteur.derniere_lettre.clone(),
            dernier_rang: rang,
        },
    )
    .await?;
    let mut trace = Trace::new(PorteeAudit::Dossier(dossier.id()), "lettrage_rupture", id);
    trace.donnees.push(("lettrage", lu.id.to_string()));
    trace.donnees.push(("compte", compte_id.to_string()));
    trace.donnees.push(("lettre", lu.lettre.clone()));
    trace.donnees.push(("rang", rang.to_string()));
    if jour != date {
        trace.donnees.push(("jour_du_dossier", interne::date(jour)));
    }
    let vue = VueRuptureCreee {
        id: id.to_string(),
        lettrage: lu.id.to_string(),
        compte: VueReferenceCompte {
            id: compte_id.to_string(),
            numero: numero.to_string(),
        },
        lettre: lu.lettre.clone(),
        date: interne::date(date),
        lignes: refs
            .iter()
            .map(|r| vue_ligne(&charge, *r))
            .collect::<Result<_, _>>()?,
    };
    Ok(Resultat {
        reponse: (StatusCode::CREATED, Json(vue)).into_response(),
        traces: vec![trace],
    })
}

// --- Lectures ---------------------------------------------------------------------------------

#[derive(Debug, Serialize)]
struct VueLecture {
    /// Identifiant du tiers ou du compte lu.
    id: String,
    comptes: Vec<VueCompte>,
    /// Curseur de la page suivante (`apres=<rang>-<position>`), absent sur la dernière page.
    suivant: Option<String>,
}

/// Page demandée : clé stable (rang de chaîne, position) après laquelle lire, et taille.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Page {
    apres: (i64, i16),
    limite: usize,
}

/// Paramètres des lectures : `apres=<rang>-<position>` (curseur rendu par la page précédente) et
/// `limite=<1…5000>` (défaut 1 000), chacun au plus une fois. Tout autre paramètre : 400.
fn page(parametres: Option<&str>) -> Result<Page, ErreurApi> {
    let mut apres = None;
    let mut limite = None;
    for paire in parametres
        .unwrap_or("")
        .split('&')
        .filter(|p| !p.is_empty())
    {
        match paire.split_once('=') {
            Some(("apres", v)) if apres.is_none() => {
                let (rang, position) = v.split_once('-').ok_or(ErreurApi::RequeteInvalide)?;
                let rang: i64 = rang.parse().map_err(|_| ErreurApi::RequeteInvalide)?;
                let position: i16 = position.parse().map_err(|_| ErreurApi::RequeteInvalide)?;
                if rang < 0 || position < 0 {
                    return Err(ErreurApi::RequeteInvalide);
                }
                apres = Some((rang, position));
            }
            Some(("limite", v)) if limite.is_none() => {
                let n: usize = v.parse().map_err(|_| ErreurApi::RequeteInvalide)?;
                if !(1..=LIGNES_LUES_MAX).contains(&n) {
                    return Err(ErreurApi::RequeteInvalide);
                }
                limite = Some(n);
            }
            _ => return Err(ErreurApi::RequeteInvalide),
        }
    }
    Ok(Page {
        apres: apres.unwrap_or((0, 0)),
        limite: limite.unwrap_or(LIGNES_LUES_DEFAUT),
    })
}

/// Coupe la page : `limite` lignes au plus, et le curseur de la suivante si une ligne de plus a
/// été lue.
fn couper(mut lues: Vec<LigneLue>, limite: usize) -> (Vec<LigneLue>, Option<String>) {
    if lues.len() <= limite {
        return (lues, None);
    }
    lues.truncate(limite);
    let suivant = lues.last().map(|l| format!("{}-{}", l.rang, l.position));
    (lues, suivant)
}

fn limite_sql(p: Page) -> Result<i64, ErreurApi> {
    i64::try_from(p.limite + 1).map_err(|_| donnee())
}

/// Vues par compte lettrable des lignes lues (les comptes non lettrables sont omis), dans l'ordre
/// des numéros de comptes.
async fn vues_par_compte(
    tx: &mut TransactionTenant,
    tenue: &DeviseTenue,
    lues: Vec<LigneLue>,
) -> Result<Vec<VueCompte>, ErreurApi> {
    let mut par_compte: BTreeMap<String, (Uuid, Vec<LigneLue>)> = BTreeMap::new();
    for l in lues {
        par_compte
            .entry(l.compte.clone())
            .or_insert_with(|| (l.compte_id, Vec::new()))
            .1
            .push(l);
    }
    let mut vues = Vec::new();
    for (compte, (compte_id, lignes)) in par_compte {
        let numero = NumeroCompte::parse(&compte).map_err(|_| donnee())?;
        if !officiel()?.est_lettrable(&numero) {
            continue;
        }
        let affichees: Vec<RefLigne> = lignes.iter().map(LigneLue::reference).collect();
        let charge = charger(tx, tenue, &numero, compte_id, lignes).await?;
        vues.push(vue_compte(&charge, compte_id, &numero, &affichees)?);
    }
    Ok(vues)
}

async fn lire_tiers(
    tx: &mut TransactionTenant,
    dossier: &DossierAutorise,
    requete: &Requete<'_>,
) -> Result<Resultat, ErreurApi> {
    let p = page(requete.parametres)?;
    let id = id_chemin(requete.chemin, "tiers")?;
    crate::base::tiers::lire_entete(tx, id)
        .await?
        .ok_or(ErreurApi::Introuvable)?;
    let tenue = dossier.contexte().tenue()?;
    let rattaches = base::tiers_rattaches(tx, id).await?;
    let lues = base::lignes_des_tiers(tx, &rattaches, p.apres, limite_sql(p)?).await?;
    let (lues, suivant) = couper(lues, p.limite);
    let comptes = vues_par_compte(tx, &tenue, lues).await?;
    Ok(Resultat::lecture(
        Json(VueLecture {
            id: id.to_string(),
            comptes,
            suivant,
        })
        .into_response(),
    ))
}

async fn lire_compte(
    tx: &mut TransactionTenant,
    dossier: &DossierAutorise,
    requete: &Requete<'_>,
) -> Result<Resultat, ErreurApi> {
    let p = page(requete.parametres)?;
    let id = id_chemin(requete.chemin, "compte")?;
    let numero = base::numero_du_compte(tx, id)
        .await?
        .ok_or(ErreurApi::Introuvable)?;
    let numero = NumeroCompte::parse(&numero).map_err(|_| donnee())?;
    if !officiel()?.est_lettrable(&numero) {
        return Err(validation(MotifValidation::LettrageCompteNonLettrable));
    }
    let tenue = dossier.contexte().tenue()?;
    let lues = base::lignes_du_compte(tx, id, p.apres, limite_sql(p)?).await?;
    let (lues, suivant) = couper(lues, p.limite);
    let comptes = vues_par_compte(tx, &tenue, lues).await?;
    Ok(Resultat::lecture(
        Json(VueLecture {
            id: id.to_string(),
            comptes,
            suivant,
        })
        .into_response(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chaque_refus_de_compta_core_a_son_code() {
        let id = IdLigne::nouveau(1, 1).unwrap();
        let lettre = Lettre::premiere();
        for (erreur, code) in [
            (ErreurLettrage::LigneInconnue(id), "lettrage_ligne_inconnue"),
            (
                ErreurLettrage::AutreCompte(id),
                "lettrage_comptes_differents",
            ),
            (
                ErreurLettrage::LigneEnDouble(id),
                "lettrage_ligne_en_double",
            ),
            (
                ErreurLettrage::SensManquant(compta_core::Sens::Credit),
                "lettrage_sens_manquant",
            ),
            (ErreurLettrage::TiersDifferents, "lettrage_tiers_differents"),
            (ErreurLettrage::DeviseHorsL3, "lettrage_devise_etrangere"),
            (ErreurLettrage::MotifInvalide, "lettrage_motif_invalide"),
            (ErreurLettrage::LigneSoldee(id), "lettrage_ligne_soldee"),
            (ErreurLettrage::ResiduelDepasse(id), "lettrage_ligne_soldee"),
            (
                ErreurLettrage::ExerciceClos(lettre),
                "lettrage_exercice_clos",
            ),
            (ErreurLettrage::DejaRompu(lettre), "lettrage_deja_rompu"),
            (ErreurLettrage::RuptureAnterieure(lettre), "erreur_interne"),
            (ErreurLettrage::SansLien, "erreur_interne"),
            (ErreurLettrage::RangOperationInvalide(3), "erreur_interne"),
        ] {
            assert_eq!(refus(&erreur).code(), code, "{erreur:?}");
        }
        assert_eq!(
            refus(&ErreurLettrage::ExerciceClos(lettre)).statut(),
            StatusCode::CONFLICT
        );
        assert_eq!(
            refus(&ErreurLettrage::TiersDifferents).statut(),
            StatusCode::UNPROCESSABLE_ENTITY
        );
    }

    #[test]
    fn corps_borne_et_positions_valides() {
        let ligne = |position| CorpsLigne {
            ecriture: Uuid::new_v4().to_string(),
            position,
        };
        assert!(
            refs_du_corps(&CorpsLettrage {
                lignes: vec![ligne(1), ligne(200)],
            })
            .is_ok()
        );
        for position in [0, u16::MAX] {
            assert_eq!(
                refs_du_corps(&CorpsLettrage {
                    lignes: vec![ligne(position)],
                }),
                Err(ErreurApi::RequeteInvalide)
            );
        }
        assert_eq!(
            refs_du_corps(&CorpsLettrage {
                lignes: (0..=LIGNES_MAX).map(|_| ligne(1)).collect(),
            }),
            Err(validation(MotifValidation::LettrageTropDeLignes))
        );
        assert_eq!(
            refs_du_corps(&CorpsLettrage {
                lignes: vec![CorpsLigne {
                    ecriture: "pas-un-uuid".to_owned(),
                    position: 1,
                }],
            }),
            Err(ErreurApi::RequeteInvalide)
        );
    }

    #[test]
    fn page_stricte() {
        assert_eq!(
            page(None),
            Ok(Page {
                apres: (0, 0),
                limite: LIGNES_LUES_DEFAUT
            })
        );
        assert_eq!(
            page(Some("apres=12-3&limite=50")),
            Ok(Page {
                apres: (12, 3),
                limite: 50
            })
        );
        for invalide in [
            "limite=0",
            "limite=5001",
            "apres=12",
            "apres=a-1",
            "apres=-1-2",
            "apres=1-2&apres=1-2",
            "x=1",
            "limite",
        ] {
            assert_eq!(
                page(Some(invalide)),
                Err(ErreurApi::RequeteInvalide),
                "{invalide}"
            );
        }
    }

    #[test]
    fn origines_aller_retour() {
        for o in [
            Origine::Manuelle,
            Origine::Automatique,
            Origine::PropositionValidee,
        ] {
            assert_eq!(origine_lue(nom_origine(o)).unwrap(), o);
        }
        assert!(origine_lue("autre").is_err());
    }
}
