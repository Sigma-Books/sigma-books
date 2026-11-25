//! Journal d'audit côté serveur (ADR 0014, ADR 0016 ; `docs/integrite/audit-v1.md`).
//!
//! - **Écriture** : un cas d'usage rend des [`Trace`] ; le pipeline les complète de ce qu'il a
//!   établi (acteur, rôle, session, action, politiques déterminantes, version du jeu de
//!   politiques, ré-authentification) et les inscrit **dans la transaction de l'action**
//!   ([`inscrire_traces`]), juste avant sa validation.
//! - **Lecture** ([`consulter`], [`verifier`], [`exporter`]) : sous le rôle de lecture d'audit,
//!   pool distinct, dans une transaction au même contexte de tenant ; sur la seule portée que le
//!   pipeline a décidée ([`Garde::portee_audit`]). Portée cabinet : administrateur seul
//!   (ADR 0016) ; portée dossier : administrateur, valideur, auditeur affectés (Cedar).
//!   Défense en profondeur : même si Cedar se trompait, la requête exige le rôle
//!   d'administrateur lu en base (portée cabinet) ou un dossier visible, c'est-à-dire affecté
//!   sauf pour l'administrateur (portée dossier) ; sinon 404 ([`portee_verifiee`]).
//! - Les lectures s'exécutent dans une transaction `REPEATABLE READ READ ONLY` : un seul
//!   instantané, bornée au rang de la tête qu'il contient. Une action validée pendant la
//!   lecture n'y apparaît pas à moitié.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::OnceLock;

use axum::Json;
use axum::http::{HeaderValue, header};
use axum::response::{IntoResponse, Response};
use integrite::Empreinte;
use integrite::audit::{
    Acteur, Champ, DecisionCedar, EcrivainExportAudit, EntreeAudit, Evenement, Ressource, Symbole,
    Texte, Uuid as UuidAudit, Verificateur, genese,
};
use serde::Serialize;
use time::OffsetDateTime;

use super::erreur::{CategorieErreur, ErreurApi};
use super::pipeline::Garde;
use super::routes::Requete;
use crate::base::audit::{PorteeAudit, inscrire, lire_page, lire_tete, verrouiller_tete};
use crate::base::organisation::{lire_appartenance, lire_dossier_visible, lire_noms_membres};
use crate::base::taches::{SujetNotification, notifier};
use crate::base::{IdUtilisateur, TransactionTenant, ouvrir_transaction_lecture};

/// Taille d'une page de consultation par défaut.
pub const PAGE_PAR_DEFAUT: u32 = 50;
/// Taille maximale d'une page de consultation (pagination imposée, F2.9).
pub const PAGE_MAX: u32 = 100;
/// Entrées lues par lot pour la vérification et l'export.
const LOT: u32 = 1000;

/// Ce qu'un cas d'usage demande d'inscrire au journal d'audit : portée, ressource, motif,
/// données citées et champs modifiés. Le pipeline y ajoute l'acteur et la décision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Trace {
    /// Portée de la chaîne : le cabinet du contexte, ou un dossier (qui devient le contexte).
    pub portee: PorteeAudit,
    /// Genre de la ressource (`cabinet`, `dossier`…).
    pub genre: &'static str,
    /// Identifiant de la ressource.
    pub ressource: uuid::Uuid,
    /// Motif, si l'action l'exige.
    pub motif: Option<String>,
    /// Données citées (nom, valeur), dans l'ordre.
    pub donnees: Vec<(&'static str, String)>,
    /// Champs modifiés (nom, avant, après), dans l'ordre.
    pub champs: Vec<(&'static str, Option<String>, Option<String>)>,
}

impl Trace {
    /// Trace sans motif, donnée ni champ.
    #[must_use]
    pub fn new(portee: PorteeAudit, genre: &'static str, ressource: uuid::Uuid) -> Self {
        Self {
            portee,
            genre,
            ressource,
            motif: None,
            donnees: Vec::new(),
            champs: Vec::new(),
        }
    }
}

/// Version du jeu de politiques Cedar embarqué (`audit-v1.md` §6) : SHA-256 de chaque fragment
/// du schéma puis de chaque fichier de politiques, chacun précédé de son nom et de sa longueur.
#[must_use]
pub fn version_politiques() -> Empreinte {
    static VERSION: OnceLock<Empreinte> = OnceLock::new();
    *VERSION.get_or_init(|| {
        let mut suite = Vec::new();
        let fichiers = authz::SCHEMA.into_iter().chain(authz::POLITIQUES);
        for (nom, contenu) in fichiers {
            suite.extend_from_slice(nom.as_bytes());
            suite.push(b'\n');
            suite.extend_from_slice(contenu.len().to_string().as_bytes());
            suite.push(b'\n');
            suite.extend_from_slice(contenu.as_bytes());
        }
        Empreinte::calculer(&suite)
    })
}

fn uuid_audit(uuid: uuid::Uuid) -> UuidAudit {
    UuidAudit::depuis_octets(*uuid.as_bytes())
}

fn invalide<T>(_: T) -> ErreurApi {
    ErreurApi::Interne(CategorieErreur::Audit)
}

fn texte(valeur: &str) -> Result<Texte, ErreurApi> {
    Texte::parse(valeur).map_err(invalide)
}

/// Événement d'une trace : acteur et décision établis par le pipeline.
fn evenement(garde: &Garde, trace: &Trace) -> Result<Evenement, ErreurApi> {
    let verdict = garde
        .verdict()
        .ok_or(ErreurApi::Interne(CategorieErreur::Audit))?;
    let politiques = verdict
        .politiques
        .iter()
        .map(|p| Symbole::parse(p))
        .collect::<Result<Vec<_>, _>>()
        .map_err(invalide)?;
    let decision = DecisionCedar::new(politiques, version_politiques(), garde.session_fraiche())
        .map_err(invalide)?;
    let acteur = Acteur::Utilisateur {
        id: uuid_audit(garde.utilisateur().uuid()),
        role: Symbole::parse(garde.role().nom()).map_err(invalide)?,
        session: uuid_audit(garde.id_session().uuid()),
        decision,
    };
    let mut e = Evenement::new(
        acteur,
        Symbole::parse(garde.action().nom()).map_err(invalide)?,
        Ressource {
            genre: Symbole::parse(trace.genre).map_err(invalide)?,
            id: uuid_audit(trace.ressource),
        },
    );
    if let Some(motif) = &trace.motif {
        e = e.avec_motif(texte(motif)?).map_err(invalide)?;
    }
    for (nom, valeur) in &trace.donnees {
        e = e
            .avec_donnee(Symbole::parse(nom).map_err(invalide)?, texte(valeur)?)
            .map_err(invalide)?;
    }
    for (nom, avant, apres) in &trace.champs {
        let avant = avant.as_deref().map(texte).transpose()?;
        let apres = apres.as_deref().map(texte).transpose()?;
        let champ =
            Champ::new(Symbole::parse(nom).map_err(invalide)?, avant, apres).map_err(invalide)?;
        e = e.avec_champ(champ).map_err(invalide)?;
    }
    Ok(e)
}

/// Inscrit les traces d'une action, dans l'ordre, dans la transaction de l'action.
///
/// Ordre des verrous (ADR 0014) : une trace du cabinet ne peut pas suivre une trace d'un
/// dossier. Une trace d'un dossier, dans une action du cabinet (création d'un dossier),
/// restreint le contexte de la transaction à ce dossier
/// ([`TransactionTenant::restreindre_au_dossier`]).
///
/// # Errors
/// Trace incohérente ou inscription impossible : l'appelant abandonne la transaction.
pub async fn inscrire_traces(
    tx: &mut TransactionTenant,
    garde: &Garde,
    traces: Vec<Trace>,
) -> Result<(), ErreurApi> {
    let instant = OffsetDateTime::now_utc();
    let mut dossier_vu = false;
    for trace in traces {
        let portee = trace.portee;
        match portee {
            PorteeAudit::Cabinet if dossier_vu => {
                return Err(ErreurApi::Interne(CategorieErreur::Audit));
            }
            PorteeAudit::Cabinet => {}
            PorteeAudit::Dossier(dossier) => {
                dossier_vu = true;
                tx.restreindre_au_dossier(dossier).await?;
            }
        }
        let evenement = evenement(garde, &trace)?;
        inscrire(tx, portee, instant, evenement).await?;
    }
    Ok(())
}

/// Pagination d'une consultation : `apres` (rang, défaut 0) et `limite` (1 à [`PAGE_MAX`],
/// défaut [`PAGE_PAR_DEFAUT`]). Tout autre paramètre, doublon ou valeur non décimale : 400.
fn pagination(parametres: Option<&str>) -> Result<(u64, u32), ErreurApi> {
    let (mut apres, mut limite) = (None, None);
    for paire in parametres
        .unwrap_or("")
        .split('&')
        .filter(|p| !p.is_empty())
    {
        let (cle, valeur) = paire.split_once('=').ok_or(ErreurApi::RequeteInvalide)?;
        let decimal = !valeur.is_empty() && valeur.bytes().all(|b| b.is_ascii_digit());
        if !decimal {
            return Err(ErreurApi::RequeteInvalide);
        }
        let place = match cle {
            "apres" => &mut apres,
            "limite" => &mut limite,
            _ => return Err(ErreurApi::RequeteInvalide),
        };
        if place.replace(valeur).is_some() {
            return Err(ErreurApi::RequeteInvalide);
        }
    }
    // Un rang est un `bigint` : au-delà de `i64::MAX`, la requête est invalide (400).
    let apres = apres
        .map(str::parse::<i64>)
        .transpose()
        .map_err(|_| ErreurApi::RequeteInvalide)?
        .map_or(Ok(0), u64::try_from)
        .map_err(|_| ErreurApi::RequeteInvalide)?;
    let limite = limite
        .map(str::parse::<u32>)
        .transpose()
        .map_err(|_| ErreurApi::RequeteInvalide)?
        .unwrap_or(PAGE_PAR_DEFAUT);
    if !(1..=PAGE_MAX).contains(&limite) {
        return Err(ErreurApi::RequeteInvalide);
    }
    Ok((apres, limite))
}

/// Transaction du rôle de lecture d'audit, au contexte de la transaction de l'action, en
/// lecture seule à instantané unique.
async fn transaction_lecture(
    tx: &TransactionTenant,
    requete: &Requete<'_>,
) -> Result<TransactionTenant, ErreurApi> {
    Ok(ouvrir_transaction_lecture(requete.lecture_audit, *tx.contexte()).await?)
}

/// Portée d'audit décidée par Cedar, revérifiée au niveau de la requête (défense en
/// profondeur, ADR 0011) : la portée cabinet exige le rôle `administrateur` **lu en base**
/// (ADR 0016) ; la portée dossier, un dossier visible par l'utilisateur du contexte (affecté,
/// sauf pour l'administrateur). Sinon : ressource d'autrui (404).
///
/// # Errors
/// [`ErreurApi::Introuvable`], erreur de base.
async fn portee_verifiee(
    tx: &mut TransactionTenant,
    garde: &Garde,
) -> Result<PorteeAudit, ErreurApi> {
    let portee = garde.portee_audit()?;
    match portee {
        PorteeAudit::Cabinet => {
            let administrateur = lire_appartenance(tx)
                .await?
                .is_some_and(|a| a.actif && a.role == authz::Role::Administrateur.nom());
            if !administrateur {
                return Err(ErreurApi::Introuvable);
            }
        }
        PorteeAudit::Dossier(dossier) => {
            if lire_dossier_visible(tx, dossier).await?.is_none() {
                return Err(ErreurApi::Introuvable);
            }
        }
    }
    Ok(portee)
}

/// Noms des acteurs membres du cabinet, lus par le rôle applicatif (le rôle de lecture d'audit
/// ne lit que l'audit).
async fn noms(
    tx: &mut TransactionTenant,
    acteurs: &BTreeSet<UuidAudit>,
) -> Result<BTreeMap<UuidAudit, Texte>, ErreurApi> {
    let ids: Vec<IdUtilisateur> = acteurs
        .iter()
        .map(|a| IdUtilisateur::depuis_uuid(uuid::Uuid::from_bytes(*a.octets())))
        .collect();
    Ok(lire_noms_membres(tx, &ids)
        .await?
        .into_iter()
        .filter_map(|(id, nom)| {
            let nom = Texte::parse(&nom).ok().filter(|n| !n.as_str().is_empty())?;
            Some((uuid_audit(id.uuid()), nom))
        })
        .collect())
}

#[derive(Serialize)]
struct VueDonnee {
    nom: String,
    valeur: String,
}

#[derive(Serialize)]
struct VueChamp {
    nom: String,
    avant: Option<String>,
    apres: Option<String>,
}

#[derive(Serialize)]
struct VueEntree {
    rang: u64,
    empreinte: String,
    /// Vrai si l'entrée enregistrée n'est pas une entrée canonique (voir la vérification).
    illisible: bool,
    instant: Option<String>,
    acteur: Option<String>,
    acteur_nom: Option<String>,
    role: Option<String>,
    action: Option<String>,
    ressource: Option<String>,
    reauthentification: Option<bool>,
    motif: Option<String>,
    donnees: Vec<VueDonnee>,
    champs: Vec<VueChamp>,
}

impl VueEntree {
    fn illisible(rang: u64, empreinte: String) -> Self {
        Self {
            rang,
            empreinte,
            illisible: true,
            instant: None,
            acteur: None,
            acteur_nom: None,
            role: None,
            action: None,
            ressource: None,
            reauthentification: None,
            motif: None,
            donnees: Vec::new(),
            champs: Vec::new(),
        }
    }

    fn depuis(entree: &EntreeAudit, empreinte: String, noms: &BTreeMap<UuidAudit, Texte>) -> Self {
        let e = entree.evenement();
        let (acteur, acteur_nom, role, reauthentification) = match e.acteur() {
            Acteur::Systeme => (integrite::audit::SYSTEME.to_owned(), None, None, None),
            Acteur::Utilisateur {
                id, role, decision, ..
            } => (
                id.to_string(),
                noms.get(id).map(ToString::to_string),
                Some(role.to_string()),
                Some(decision.reauthentification()),
            ),
        };
        Self {
            rang: entree.rang(),
            empreinte,
            illisible: false,
            instant: Some(integrite::audit::formater_instant(entree.instant())),
            acteur: Some(acteur),
            acteur_nom,
            role,
            action: Some(e.action().to_string()),
            ressource: Some(e.ressource().to_string()),
            reauthentification,
            motif: e.motif().map(ToString::to_string),
            donnees: e
                .donnees()
                .iter()
                .map(|(nom, valeur)| VueDonnee {
                    nom: nom.to_string(),
                    valeur: valeur.to_string(),
                })
                .collect(),
            champs: e
                .champs()
                .iter()
                .map(|c| VueChamp {
                    nom: c.nom().to_string(),
                    avant: c.avant().map(ToString::to_string),
                    apres: c.apres().map(ToString::to_string),
                })
                .collect(),
        }
    }
}

#[derive(Serialize)]
struct VuePage {
    entrees: Vec<VueEntree>,
    /// Valeur de `apres` pour la page suivante ; absente à la fin de la chaîne.
    suivant: Option<u64>,
}

/// Consultation paginée de la chaîne de la portée décidée.
///
/// # Errors
/// Paramètres invalides (400), erreur de base.
pub async fn consulter(
    tx: &mut TransactionTenant,
    garde: &Garde,
    requete: &Requete<'_>,
) -> Result<Response, ErreurApi> {
    let (apres, limite) = pagination(requete.parametres)?;
    let portee = portee_verifiee(tx, garde).await?;
    let mut lecture = transaction_lecture(tx, requete).await?;
    let mut lignes = lire_page(&mut lecture, portee, apres, None, limite + 1).await?;
    lecture.annuler().await?;
    let suivant = if lignes.len() > limite as usize {
        lignes.truncate(limite as usize);
        lignes.last().map(|l| l.rang)
    } else {
        None
    };
    let lues: Vec<(u64, String, Option<EntreeAudit>)> = lignes
        .into_iter()
        .map(|l| {
            let entree = EntreeAudit::lire(&l.contenu).ok();
            (l.rang, l.empreinte.to_string(), entree)
        })
        .collect();
    let acteurs: BTreeSet<UuidAudit> = lues
        .iter()
        .filter_map(|(_, _, e)| match e.as_ref()?.evenement().acteur() {
            Acteur::Utilisateur { id, .. } => Some(*id),
            Acteur::Systeme => None,
        })
        .collect();
    let noms = noms(tx, &acteurs).await?;
    let entrees = lues
        .into_iter()
        .map(|(rang, empreinte, entree)| match entree {
            Some(e) => VueEntree::depuis(&e, empreinte, &noms),
            None => VueEntree::illisible(rang, empreinte),
        })
        .collect();
    Ok(Json(VuePage { entrees, suivant }).into_response())
}

#[derive(Serialize)]
struct VueAnomalie {
    code: &'static str,
    position: Option<u64>,
}

#[derive(Serialize)]
struct VueVerification {
    integre: bool,
    nombre: u64,
    tete: String,
    anomalies: Vec<VueAnomalie>,
}

/// Vérification de la chaîne de la portée décidée : recalcul depuis l'empreinte de départ,
/// comparaison à la tête enregistrée.
///
/// # Errors
/// Erreur de base.
pub async fn verifier(
    tx: &mut TransactionTenant,
    garde: &Garde,
    requete: &Requete<'_>,
) -> Result<Response, ErreurApi> {
    let portee = portee_verifiee(tx, garde).await?;
    let mut lecture = transaction_lecture(tx, requete).await?;
    let portee_integrite = crate::base::audit::portee_integrite(&lecture, portee)?;
    let (rang_tete, tete) = lire_tete(&mut lecture, portee)
        .await?
        .unwrap_or((0, genese(&portee_integrite)));
    let mut verificateur = Verificateur::nouveau(portee_integrite);
    let mut apres = 0;
    loop {
        // Bornée au rang de la tête du même instantané.
        let lot = lire_page(&mut lecture, portee, apres, Some(rang_tete), LOT).await?;
        let Some(derniere) = lot.last() else { break };
        apres = derniere.rang;
        for ligne in &lot {
            verificateur.entree(&ligne.contenu, &ligne.empreinte, Some(ligne.rang));
        }
    }
    lecture.annuler().await?;
    let rapport = verificateur.terminer(Some(&tete), Some(rang_tete));
    Ok(Json(VueVerification {
        integre: rapport.est_integre(),
        nombre: rapport.nombre,
        tete: rapport.tete.to_string(),
        anomalies: rapport
            .anomalies
            .iter()
            .map(|a| VueAnomalie {
                code: a.code(),
                position: a.position(),
            })
            .collect(),
    })
    .into_response())
}

/// Export de la chaîne de la portée décidée (`audit-v1.md` §5), inscrit lui-même à l'audit avec
/// la tête et le nombre exportés. La tête de la portée est verrouillée d'abord : aucune entrée
/// ne s'ajoute pendant l'export.
///
/// Limites acceptées en L2 (`crates/api/README.md`) : l'export est construit en mémoire, et le
/// verrou de la tête dure tout l'export. L'alerte à l'administrateur (spec §21) est une tâche de
/// la file ([`notifier`]) ; son courriel arrive avec la PR 9.
///
/// # Errors
/// Entrée enregistrée illisible (l'export est refusé : la vérification la localise), erreur de
/// base.
pub async fn exporter(
    tx: &mut TransactionTenant,
    garde: &Garde,
    requete: &Requete<'_>,
) -> Result<(Response, Trace), ErreurApi> {
    let portee = portee_verifiee(tx, garde).await?;
    let chaine = verrouiller_tete(tx, portee).await?;
    let mut ecrivain =
        EcrivainExportAudit::nouveau(*chaine.portee(), Vec::new()).map_err(invalide)?;
    let mut lecture = transaction_lecture(tx, requete).await?;
    let mut apres = 0;
    while apres < chaine.longueur() {
        let lot = lire_page(&mut lecture, portee, apres, Some(chaine.longueur()), LOT).await?;
        let Some(derniere) = lot.last() else { break };
        apres = derniere.rang;
        for ligne in &lot {
            ecrivain
                .entree(&ligne.contenu, &ligne.empreinte)
                .map_err(invalide)?;
        }
    }
    lecture.annuler().await?;
    let noms = noms(tx, ecrivain.acteurs()).await?;
    let tete = *chaine.tete();
    let (octets, nombre) = ecrivain.terminer(&tete, &noms).map_err(invalide)?;

    let (genre, id, nom_fichier) = match portee {
        PorteeAudit::Cabinet => {
            let id = tx.contexte().id_cabinet().uuid();
            ("cabinet", id, format!("audit-cabinet-{id}.txt"))
        }
        PorteeAudit::Dossier(dossier) => (
            "dossier",
            dossier.uuid(),
            format!("audit-dossier-{dossier}.txt"),
        ),
    };
    // Alerte à l'administrateur (spec §21, ADR 0014, #157) : une tâche de la file, créée dans la
    // transaction de l'export, donc seulement si l'export est validé. Le courriel est celui de la
    // PR 9 ; la tâche n'emporte aucune donnée du dossier.
    let sujet = match portee {
        PorteeAudit::Cabinet => SujetNotification::ExportAuditCabinet,
        PorteeAudit::Dossier(_) => SujetNotification::ExportAuditDossier,
    };
    notifier(tx, sujet, Some(garde.utilisateur().uuid())).await?;
    let mut trace = Trace::new(portee, genre, id);
    if genre == "dossier" {
        // `ExporterComplet` couvre aussi les livres et le FEC : l'objet de l'export est précisé.
        trace.donnees.push(("export", "audit".to_owned()));
    }
    trace.donnees.push(("tete", tete.to_string()));
    trace.donnees.push(("nombre", nombre.to_string()));

    let mut reponse = octets.into_response();
    let entetes = reponse.headers_mut();
    entetes.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/plain; charset=utf-8"),
    );
    entetes.insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_str(&format!("attachment; filename=\"{nom_fichier}\""))
            .map_err(invalide)?,
    );
    Ok((reponse, trace))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pagination_stricte() {
        assert_eq!(pagination(None), Ok((0, PAGE_PAR_DEFAUT)));
        assert_eq!(pagination(Some("apres=12&limite=100")), Ok((12, 100)));
        assert_eq!(pagination(Some("limite=1")), Ok((0, 1)));
        assert_eq!(
            pagination(Some("apres=9223372036854775807")),
            Ok((9_223_372_036_854_775_807, PAGE_PAR_DEFAUT))
        );
        for invalide in [
            "limite=0",
            "limite=101",
            "limite=-1",
            "apres=+1",
            "apres=1&apres=2",
            "tri=rang",
            "apres",
            "apres=",
            "apres=99999999999999999999999",
            "apres=9223372036854775808",
        ] {
            assert_eq!(
                pagination(Some(invalide)),
                Err(ErreurApi::RequeteInvalide),
                "{invalide}"
            );
        }
    }

    #[test]
    fn version_des_politiques_stable_et_sensible() {
        assert_eq!(version_politiques(), version_politiques());
        assert_ne!(version_politiques(), Empreinte::calculer(b""));
    }
}
