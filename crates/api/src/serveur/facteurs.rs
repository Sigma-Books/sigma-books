//! Méthode de secours (ADR 0012, ADR 0017 ; L2 PR 8b, issue #170) : connexion par le secours
//! (mot de passe et TOTP) ou par un code de récupération, ré-authentification, enrôlement du
//! secours, codes de récupération, état de ses facteurs, ses événements d'authentification.
//! Routes : `routes/facteurs.rs` ; cryptographie : [`crate::facteurs`] ; base :
//! [`crate::base::facteurs`], [`crate::base::evenement`].
//!
//! # Configuration
//! Lue dans l'environnement par [`super::config::Configuration`] ([`configurer`]).
//!
//! | Variable | Obligatoire | Contenu |
//! |---|---|---|
//! | `SIGMA_FICHIER_CLE_INSTANCE` | oui | Chemin du fichier de la clé de l'instance : 32 octets en base64, droits 0600 (ou 0400), hors de la base. Sans clé, le serveur refuse de démarrer. |
//!
//! # Décisions Cedar (règle absolue 7)
//! - **Connexion** : le demandeur n'a pas de session. Cedar décide d'abord `SeConnecter`
//!   (principal `Anonyme`, ressource l'instance), action publique et bornée : elle ne rend
//!   rien, la limitation de débit borne les essais (par adresse, puis par courriel). Une fois le
//!   facteur vérifié et consommé, Cedar décide `OuvrirSession` avec le principal `Utilisateur`,
//!   la méthode vérifiée et la nature de la session : la politique n'admet que le secours pour
//!   une session complète et le code de récupération pour une session restreinte, et un
//!   invariant interdit toute autre nature au code (F1.12). La nature est en outre fixée par le
//!   code (le code n'ouvre qu'une session restreinte), même si Cedar se trompait.
//! - **Autres routes** : principal, l'utilisateur de la session authentifiée ; ressource,
//!   lui-même ; contexte, la session **lue en base** (nature, méthode, fraîcheur). La RLS
//!   (`utilisateur_id = sigma.utilisateur`) et le filtre de chaque requête bornent tout à cet
//!   utilisateur : les facteurs ou les événements d'un autre restent hors de portée même si
//!   Cedar se trompait.
//!
//! # Anti-énumération (OWASP, aide-mémoire « Authentication »)
//! Courriel inconnu, compte sans secours ni code, facteur faux ou rejoué, compte plafonné :
//! même réponse, 401 `non_authentifie`, et **même durée** : toute réponse d'échec d'une
//! connexion part au plus tôt après une durée plancher constante
//! ([`ReglagesFacteurs::duree_plancher_connexion`], 500 ms), supérieure au chemin le plus
//! long ; Argon2id est en outre toujours exécuté, sur une chaîne factice si le compte n'a pas
//! de secours, et pour la connexion par un code. Rien n'est inscrit pour un courriel inconnu
//! (ADR 0017) ; un échec sur un compte connu est inscrit à ses événements.
//!
//! # Plafond par compte
//! Outre le débit en mémoire (par adresse, par courriel), un plafond **persistant** :
//! au-delà de [`ReglagesFacteurs::echecs_max_par_compte`] échecs inscrits sur la fenêtre, le
//! facteur n'est plus vérifié (ni pas TOTP, ni code consommé), avec la même réponse 401 et la
//! même durée, jamais un 429 qui révélerait le compte. Les tentatives restent inscrites.
//!
//! # Hachage
//! Argon2id (19 Mio, 2 passes) s'exécute hors de la boucle asynchrone (`spawn_blocking`), au
//! plus [`ReglagesFacteurs::hachages_simultanes`] à la fois, et **hors de toute transaction** :
//! la connexion lit le secours dans une courte transaction, vérifie, puis consomme le pas TOTP
//! dans une seconde, à condition que le mot de passe n'ait pas changé entre-temps.
//!
//! # Journal d'audit des cabinets (ADR 0017 §2)
//! La connexion par le secours ou par un code, l'enrôlement du secours et une nouvelle série de
//! codes sont inscrits dans la chaîne d'audit de chaque cabinet où l'utilisateur a une
//! appartenance active, dans la transaction de l'action (acteur : l'utilisateur, avec son rôle
//! dans ce cabinet et la décision de Cedar).

use std::net::IpAddr;
use std::sync::{Arc, OnceLock};
use std::time::Instant;

use authz::{
    ActionFacteur, ContexteFacteur, Decision, MethodeDeSession, NatureDeSession,
    Session as SessionAuthz, Verdict,
};
use axum::Json;
use axum::http::request::Parts;
use axum::http::{HeaderValue, Request, StatusCode, header};
use axum::response::{IntoResponse, Response};
use integrite::audit::{
    Acteur, DecisionCedar, Evenement, Ressource as RessourceAudit, Symbole, Texte,
    Uuid as UuidAudit,
};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use tokio::sync::Semaphore;
use zeroize::Zeroizing;

use super::Etat;
use super::audit::version_politiques;
use super::authentification::Identite;
use super::config::{Environnement, ErreurConfiguration};
use super::erreur::{CategorieErreur, ErreurApi, MotifRefus};
use super::limites::{CleAdresse, LimiteurDebit, ReglageDebit, adresse_client};
use super::pipeline::id_authz;
use super::routes::facteurs::Cas;
use super::routes::passkeys::Cas as PasskeysCas;
use super::routes::{ActionRoute, CasUsage, Route};
use super::session::{
    JetonSession, SessionOuverte, cookie, jeton_de_la_requete, ouvrir_session_dans,
};
use crate::base::evenement::{self, Issue, NatureEvenement, NouvelEvenement};
use crate::base::facteurs::{
    self as base_facteurs, MembreActif, TransactionConnexion, inscrire_dans_les_cabinets,
    ouvrir_transaction_connexion,
};
use crate::base::passkeys as base_passkeys;
use crate::base::session::{
    self, MethodeSession, MotifRevocation, NatureSession, NouvelleSession, Session,
};
use crate::base::{
    ErreurBase, IdSession, IdUtilisateur, TransactionUtilisateur, ouvrir_transaction_utilisateur,
};
use crate::facteurs::cle::CleInstance;
use crate::facteurs::codes::{self, CodeRecuperation};
use crate::facteurs::mot_de_passe::{self, RefusMotDePasse, Verification};
use crate::facteurs::passkey::Passkeys;
use crate::facteurs::totp::{self, SecretTotp};

/// Émetteur affiché par l'application d'authentification.
const EMETTEUR: &str = "Sigma Books";
/// Validité d'un jeton d'enrôlement (préparation, puis confirmation par un code).
const VALIDITE_ENROLEMENT: time::Duration = time::Duration::minutes(10);
/// Nombre d'événements rendus à l'utilisateur.
const EVENEMENTS_MAX: i64 = 100;

/// Réglages de la méthode de secours. [`Default`] : aucune clé (les routes des facteurs
/// répondent alors 500 ; le binaire exige la clé, [`configurer`]), et les débits documentés.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReglagesFacteurs {
    /// Clé de l'instance (`SIGMA_FICHIER_CLE_INSTANCE`).
    pub cle_instance: Option<Arc<CleInstance>>,
    /// Tentatives de connexion par adresse (20 par quart d'heure).
    pub debit_connexion_adresse: ReglageDebit,
    /// Tentatives de connexion par courriel (10 par quart d'heure), que le compte existe ou non.
    pub debit_connexion_courriel: ReglageDebit,
    /// Vérifications de facteur par utilisateur authentifié : ré-authentification, enrôlement
    /// (10 par quart d'heure).
    pub debit_verification: ReglageDebit,
    /// Hachages Argon2id simultanés (4) : borne la mémoire (19 Mio chacun) et le calcul.
    pub hachages_simultanes: usize,
    /// Durée plancher de toute réponse d'échec d'une connexion (500 ms) : la réponse part au
    /// plus tôt à ce terme depuis l'arrivée de la requête, quel que soit le chemin suivi
    /// (courriel inconnu, compte sans facteur, facteur faux, compte plafonné). Elle doit
    /// dépasser le pire cas mesuré du chemin le plus long.
    pub duree_plancher_connexion: std::time::Duration,
    /// Plafond persistant d'échecs de connexion par compte (10), sur [`Self::fenetre_echecs`] :
    /// au-delà, le facteur n'est plus vérifié, avec la même réponse 401 (jamais un 429, qui
    /// dirait que le compte existe). Tiré des échecs inscrits aux événements : une éviction de
    /// la table de débit en mémoire ne le contourne pas.
    pub echecs_max_par_compte: i64,
    /// Fenêtre du plafond d'échecs par compte (15 minutes).
    pub fenetre_echecs: time::Duration,
}

impl Default for ReglagesFacteurs {
    fn default() -> Self {
        let quart_d_heure = std::time::Duration::from_mins(15);
        Self {
            cle_instance: None,
            debit_connexion_adresse: ReglageDebit {
                capacite: 20,
                periode: quart_d_heure,
                cles_max: 100_000,
            },
            debit_connexion_courriel: ReglageDebit {
                capacite: 10,
                periode: quart_d_heure,
                cles_max: 100_000,
            },
            debit_verification: ReglageDebit {
                capacite: 10,
                periode: quart_d_heure,
                cles_max: 100_000,
            },
            hachages_simultanes: 4,
            duree_plancher_connexion: std::time::Duration::from_millis(500),
            echecs_max_par_compte: 10,
            fenetre_echecs: time::Duration::minutes(15),
        }
    }
}

/// Lit la clé de l'instance (variable obligatoire).
///
/// # Errors
/// [`ErreurConfiguration::Manquante`] sans la variable ; [`ErreurConfiguration::Invalide`] si
/// le fichier est illisible, trop ouvert ou mal formé (son contenu n'est jamais recopié).
pub fn configurer(
    env: &Environnement<'_>,
    reglages: &mut ReglagesFacteurs,
) -> Result<(), ErreurConfiguration> {
    const VARIABLE: &str = "SIGMA_FICHIER_CLE_INSTANCE";
    let chemin = env.obligatoire(VARIABLE)?;
    let cle = CleInstance::lire_fichier(std::path::Path::new(chemin.trim()))
        .map_err(|_| ErreurConfiguration::Invalide(VARIABLE))?;
    reglages.cle_instance = Some(Arc::new(cle));
    Ok(())
}

/// État partagé de la méthode de secours : limiteurs et borne des hachages.
pub(crate) struct EtatFacteurs {
    /// Partie de confiance WebAuthn (PR 8c), tirée de l'origine de l'instance ; absente si
    /// l'origine n'est pas utilisable (le binaire refuse alors de démarrer, `config.rs`).
    pub(super) passkeys: Option<Passkeys>,
    limiteur_adresse: LimiteurDebit<CleAdresse>,
    limiteur_courriel: LimiteurDebit<[u8; 16]>,
    limiteur_verification: LimiteurDebit<IdUtilisateur>,
    hachages: Semaphore,
    /// Chaîne PHC d'un mot de passe aléatoire jamais révélé : vérifiée quand le compte n'a pas
    /// de secours, pour une durée comparable (calculée au premier besoin).
    factice: OnceLock<Option<String>>,
}

impl EtatFacteurs {
    pub(crate) fn new(reglages: &ReglagesFacteurs, origine: &str) -> Self {
        Self {
            passkeys: Passkeys::depuis_origine(origine).ok(),
            limiteur_adresse: LimiteurDebit::new(reglages.debit_connexion_adresse),
            limiteur_courriel: LimiteurDebit::new(reglages.debit_connexion_courriel),
            limiteur_verification: LimiteurDebit::new(reglages.debit_verification),
            hachages: Semaphore::new(reglages.hachages_simultanes.max(1)),
            factice: OnceLock::new(),
        }
    }
}

// --- Outils communs ------------------------------------------------------------------------

/// Erreur interne de la méthode de secours.
pub(super) fn interne<T>(_: T) -> ErreurApi {
    ErreurApi::Interne(CategorieErreur::Authentification)
}

pub(super) fn cle_instance(etat: &Etat) -> Result<Arc<CleInstance>, ErreurApi> {
    etat.reglages
        .facteurs
        .cle_instance
        .clone()
        .ok_or(ErreurApi::Interne(CategorieErreur::Authentification))
}

pub(super) fn limiter<K: Eq + std::hash::Hash + Copy>(
    limiteur: &LimiteurDebit<K>,
    cle: K,
) -> Result<(), ErreurApi> {
    limiteur
        .verifier(cle, Instant::now())
        .map_err(|reessayer_dans| ErreurApi::TropDeRequetes { reessayer_dans })
}

/// Le corps JSON d'une requête ; champ inconnu ou absent : `refus`.
pub(super) fn corps_json<T: DeserializeOwned>(
    corps: &[u8],
    refus: ErreurApi,
) -> Result<T, ErreurApi> {
    Json::<T>::from_bytes(corps)
        .map(|Json(valeur)| valeur)
        .map_err(|_| refus)
}

/// Client de la requête : appareil déclaré et adresse, pour la session et les événements.
pub(super) struct Client {
    pub(super) appareil: Option<String>,
    pub(super) adresse: Option<IpAddr>,
    pub(super) cle_adresse: CleAdresse,
}

pub(super) fn client(etat: &Etat, parts: &Parts) -> Client {
    let cle_adresse = adresse_client(
        &Request::from_parts(parts.clone(), ()),
        etat.reglages.en_tete_adresse_client.as_ref(),
    );
    let appareil = parts
        .headers
        .get(header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .map(|v| v.chars().take(session::APPAREIL_MAX).collect());
    Client {
        appareil,
        adresse: match cle_adresse {
            CleAdresse::Ip(ip) => Some(ip),
            CleAdresse::Inconnue => None,
        },
        cle_adresse,
    }
}

pub(super) fn nouvel_evenement(
    utilisateur: IdUtilisateur,
    session: Option<IdSession>,
    nature: NatureEvenement,
    methode: Option<MethodeSession>,
    issue: Issue,
    instant: OffsetDateTime,
    client: &Client,
) -> NouvelEvenement {
    NouvelEvenement {
        utilisateur,
        session,
        nature,
        methode,
        issue,
        instant,
        appareil: client.appareil.clone(),
        adresse: client.adresse,
    }
}

/// Courriel saisi : sans espaces autour, 3 à 254 caractères, avec un `@` après le premier.
fn courriel_saisi(texte: &str) -> Option<String> {
    let texte = texte.trim();
    let longueur = texte.chars().count();
    ((3..=254).contains(&longueur) && texte.find('@').is_some_and(|i| i > 0))
        .then(|| texte.to_owned())
}

/// Vérifie un mot de passe hors de la boucle asynchrone, au plus N à la fois. Sans chaîne
/// stockée, vérifie une chaîne factice (durée comparable) et refuse.
async fn verifier_mot_de_passe(
    etat: &Etat,
    stockee: Option<String>,
    mot_de_passe: Zeroizing<String>,
) -> Result<Verification, ErreurApi> {
    let _permis = etat.facteurs.hachages.acquire().await.map_err(interne)?;
    let etat = Arc::clone(etat);
    tokio::task::spawn_blocking(move || {
        if let Some(stockee) = stockee {
            return mot_de_passe::verifier(&stockee, &mot_de_passe);
        }
        let factice = etat.facteurs.factice.get_or_init(|| {
            let mut alea = Zeroizing::new([0u8; 32]);
            crate::facteurs::tirer(alea.as_mut()).ok()?;
            let texte = Zeroizing::new(<base64ct::Base64 as base64ct::Encoding>::encode_string(
                &alea[..],
            ));
            mot_de_passe::hacher(&texte).ok()
        });
        if let Some(factice) = factice {
            let _ = mot_de_passe::verifier(factice, &mot_de_passe);
        }
        Verification::Refuse
    })
    .await
    .map_err(interne)
}

/// Hache un mot de passe hors de la boucle asynchrone, au plus N à la fois.
async fn hacher_mot_de_passe(
    etat: &Etat,
    mot_de_passe: Zeroizing<String>,
) -> Result<String, ErreurApi> {
    let _permis = etat.facteurs.hachages.acquire().await.map_err(interne)?;
    tokio::task::spawn_blocking(move || mot_de_passe::hacher(&mot_de_passe))
        .await
        .map_err(interne)?
        .map_err(interne)
}

/// Pas TOTP accepté du secours lu pour le code saisi, s'il dépasse le dernier pas accepté.
fn pas_accepte(
    cle: &CleInstance,
    secours: &base_facteurs::SecoursLu,
    code: &str,
    maintenant: OffsetDateTime,
) -> Option<u64> {
    let secret = cle.dechiffrer_totp(
        secours.utilisateur.uuid(),
        &secours.totp_cle_id,
        &secours.totp_chiffre,
    )?;
    totp::verifier(&secret, code, maintenant).filter(|pas| *pas > secours.totp_dernier_pas)
}

const fn nature_authz(nature: NatureSession) -> NatureDeSession {
    match nature {
        NatureSession::Complete => NatureDeSession::Complete,
        NatureSession::Restreinte => NatureDeSession::Restreinte,
        NatureSession::Enrolement => NatureDeSession::Enrolement,
    }
}

const fn methode_authz(methode: MethodeSession) -> MethodeDeSession {
    match methode {
        MethodeSession::Passkey => MethodeDeSession::Passkey,
        MethodeSession::Secours => MethodeDeSession::Secours,
        MethodeSession::CodeRecuperation => MethodeDeSession::CodeRecuperation,
        MethodeSession::LienEnrolement => MethodeDeSession::LienEnrolement,
    }
}

/// Décision Cedar d'une action de l'utilisateur sur ses facteurs : verdict autorisé, ou 403.
fn decider(
    etat: &Etat,
    utilisateur: IdUtilisateur,
    action: ActionFacteur,
    nature: NatureSession,
    methode: MethodeSession,
    session: SessionAuthz,
) -> Result<Verdict, ErreurApi> {
    let principal = id_authz(utilisateur.uuid(), authz::IdUtilisateur::new)?;
    let verdict = etat.decideur.verdict_facteur(
        principal,
        action,
        principal,
        ContexteFacteur {
            nature: nature_authz(nature),
            methode: methode_authz(methode),
            session,
        },
    );
    if verdict.decision == Decision::Autorise {
        Ok(verdict)
    } else {
        Err(ErreurApi::Interdit)
    }
}

/// Entrée d'audit d'un cabinet pour une action sur ses facteurs : acteur, l'utilisateur avec
/// son rôle dans ce cabinet, sa session et la décision de Cedar ; ressource, l'utilisateur ;
/// donnée, la méthode. Aucun secret.
fn entree_audit(
    membre: &MembreActif,
    utilisateur: IdUtilisateur,
    session: IdSession,
    action: ActionFacteur,
    verdict: &Verdict,
    fraiche: bool,
    methode: MethodeSession,
) -> Result<Evenement, ErreurBase> {
    let invalide = |_| ErreurBase::Audit("entrée d'audit des facteurs");
    let politiques = verdict
        .politiques
        .iter()
        .map(|p| Symbole::parse(p))
        .collect::<Result<Vec<_>, _>>()
        .map_err(invalide)?;
    let decision =
        DecisionCedar::new(politiques, version_politiques(), fraiche).map_err(invalide)?;
    let id = UuidAudit::depuis_octets(*utilisateur.uuid().as_bytes());
    let acteur = Acteur::Utilisateur {
        id,
        role: Symbole::parse(&membre.role).map_err(invalide)?,
        session: UuidAudit::depuis_octets(*session.uuid().as_bytes()),
        decision,
    };
    Evenement::new(
        acteur,
        Symbole::parse(action.nom()).map_err(invalide)?,
        RessourceAudit {
            genre: Symbole::parse("utilisateur").map_err(invalide)?,
            id,
        },
    )
    .avec_donnee(
        Symbole::parse("methode").map_err(invalide)?,
        Texte::parse(methode.nom()).map_err(invalide)?,
    )
    .map_err(invalide)
}

/// Réponse portant un `Set-Cookie`.
pub(super) fn avec_cookie(mut reponse: Response, valeur: HeaderValue) -> Response {
    reponse.headers_mut().insert(header::SET_COOKIE, valeur);
    reponse
}

pub(super) fn instant(i: OffsetDateTime) -> String {
    integrite::audit::formater_instant(i)
}

// --- Connexion -----------------------------------------------------------------------------

/// Session ouverte par une connexion : jeton anti-CSRF et échéance ; le jeton est dans le
/// cookie.
#[derive(Debug, Serialize)]
struct VueConnexion {
    session: String,
    nature: &'static str,
    methode: &'static str,
    fin_absolue: String,
    csrf: String,
}

/// Connexion (portée anonyme) : décision `SeConnecter`, débit par adresse, puis le cas.
pub(crate) async fn connecter(
    etat: &Etat,
    route: &Route,
    parts: &Parts,
    corps: &[u8],
) -> Result<Response, ErreurApi> {
    // Durée plancher de tout échec : la durée ne dit rien du chemin suivi (anti-énumération).
    let fin_au_plus_tot =
        tokio::time::Instant::now() + etat.reglages.facteurs.duree_plancher_connexion;
    let resultat = connecter_sans_plancher(etat, route, parts, corps).await;
    // Le défi d'une connexion par passkey reçoit la même durée plancher en cas de succès :
    // un compte sans passkey (faux défi, rien d'écrit) ne s'y distingue pas par la durée.
    let plancher_aussi_en_succes =
        route.cas_usage == CasUsage::Passkeys(PasskeysCas::DefiConnexion);
    if resultat.is_err() || plancher_aussi_en_succes {
        tokio::time::sleep_until(fin_au_plus_tot).await;
    }
    resultat
}

async fn connecter_sans_plancher(
    etat: &Etat,
    route: &Route,
    parts: &Parts,
    corps: &[u8],
) -> Result<Response, ErreurApi> {
    if route.action != ActionRoute::Connexion {
        return Err(ErreurApi::Interne(CategorieErreur::Donnee));
    }
    if etat.decideur.verdict_connexion().decision != Decision::Autorise {
        return Err(ErreurApi::Interdit);
    }
    let client = client(etat, parts);
    limiter(&etat.facteurs.limiteur_adresse, client.cle_adresse)?;
    match route.cas_usage {
        CasUsage::Facteurs(Cas::ConnexionSecours) => connecter_secours(etat, &client, corps).await,
        CasUsage::Facteurs(Cas::ConnexionCode) => connecter_code(etat, &client, corps).await,
        CasUsage::Passkeys(PasskeysCas::DefiConnexion) => {
            super::passkeys::defi_connexion(etat, corps).await
        }
        CasUsage::Passkeys(PasskeysCas::ConnexionPasskey) => {
            super::passkeys::connecter_passkey(etat, &client, parts, corps).await
        }
        _ => Err(ErreurApi::Interne(CategorieErreur::Donnee)),
    }
}

/// Vrai si le compte a atteint le plafond persistant d'échecs de connexion.
pub(super) async fn plafonne(
    etat: &Etat,
    tx: &mut TransactionConnexion,
    utilisateur: Option<IdUtilisateur>,
    maintenant: OffsetDateTime,
) -> Result<bool, ErreurApi> {
    let Some(utilisateur) = utilisateur else {
        return Ok(false);
    };
    let reglages = &etat.reglages.facteurs;
    let echecs = tx
        .compter_echecs(utilisateur, maintenant - reglages.fenetre_echecs)
        .await?;
    Ok(echecs >= reglages.echecs_max_par_compte)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CorpsConnexionSecours {
    courriel: String,
    mot_de_passe: String,
    code: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CorpsConnexionCode {
    courriel: String,
    code: String,
}

/// Courriel de la connexion, et contrôle du débit par courriel (que le compte existe ou non).
pub(super) fn courriel_limite(
    etat: &Etat,
    cle: &CleInstance,
    saisi: &str,
) -> Result<String, ErreurApi> {
    let courriel = courriel_saisi(saisi).ok_or(ErreurApi::NonAuthentifie)?;
    limiter(
        &etat.facteurs.limiteur_courriel,
        cle.cle_de_debit(&courriel),
    )?;
    Ok(courriel)
}

async fn connecter_secours(
    etat: &Etat,
    client: &Client,
    corps: &[u8],
) -> Result<Response, ErreurApi> {
    // Corps illisible : aucune preuve, donc non authentifié (même réponse qu'un échec).
    let corps: CorpsConnexionSecours = corps_json(corps, ErreurApi::NonAuthentifie)?;
    let mot_de_passe = Zeroizing::new(corps.mot_de_passe);
    let code = Zeroizing::new(corps.code);
    let cle = cle_instance(etat)?;
    let courriel = courriel_limite(etat, &cle, &corps.courriel)?;
    let maintenant = etat.horloge.maintenant();

    // 1. Lecture, dans une courte transaction au contexte du seul courriel.
    let mut tx = ouvrir_transaction_connexion(&etat.pool, &courriel).await?;
    let secours = tx.lire_secours().await?;
    let utilisateur = match &secours {
        Some(s) => Some(s.utilisateur),
        None => tx.lire_utilisateur().await?,
    };
    let plafonne = plafonne(etat, &mut tx, utilisateur, maintenant).await?;
    tx.confirmer().await?;

    // 2. Vérifications, hors transaction : Argon2id toujours (chaîne factice sans secours), même
    // pour un compte plafonné, dont le facteur n'est pourtant pas admis.
    let verification = verifier_mot_de_passe(
        etat,
        secours.as_ref().map(|s| s.mot_de_passe.clone()),
        mot_de_passe.clone(),
    )
    .await?;
    let pas = secours
        .as_ref()
        .and_then(|s| pas_accepte(&cle, s, &code, maintenant));
    // Courriel inconnu : rien n'est inscrit (ADR 0017).
    let Some(utilisateur) = utilisateur else {
        return Err(ErreurApi::NonAuthentifie);
    };
    let a_rehacher = matches!(verification, Verification::Admis { a_rehacher: true });
    let nouvelle_chaine = if a_rehacher && pas.is_some() {
        Some(hacher_mot_de_passe(etat, mot_de_passe).await?)
    } else {
        None
    };

    // 3. Consommation du pas TOTP (anti-rejeu), session, événement et audit : une transaction.
    let mut tx = ouvrir_transaction_connexion(&etat.pool, &courriel).await?;
    let preuve = match (&secours, verification, pas) {
        (Some(s), Verification::Admis { .. }, Some(pas)) if !plafonne => {
            tx.consommer_pas_totp(utilisateur, &s.mot_de_passe, pas)
                .await?
        }
        _ => None,
    };
    let Some(preuve) = preuve else {
        return refuser_connexion(tx, utilisateur, MethodeSession::Secours, maintenant, client)
            .await;
    };
    let mut tx = tx.authentifier(preuve).await?;
    if let (Some(s), Some(nouvelle)) = (&secours, &nouvelle_chaine) {
        base_facteurs::rehacher(&mut tx, &s.mot_de_passe, nouvelle).await?;
    }
    ouvrir(
        etat,
        tx,
        utilisateur,
        NatureSession::Complete,
        MethodeSession::Secours,
        maintenant,
        client,
    )
    .await
}

async fn connecter_code(etat: &Etat, client: &Client, corps: &[u8]) -> Result<Response, ErreurApi> {
    let corps: CorpsConnexionCode = corps_json(corps, ErreurApi::NonAuthentifie)?;
    let code = Zeroizing::new(corps.code);
    let cle = cle_instance(etat)?;
    let courriel = courriel_limite(etat, &cle, &corps.courriel)?;
    let maintenant = etat.horloge.maintenant();
    let empreinte = codes::empreinte_saisie(&code);

    // 1. Lecture, dans une courte transaction au contexte du seul courriel.
    let mut tx = ouvrir_transaction_connexion(&etat.pool, &courriel).await?;
    let utilisateur = tx.lire_utilisateur().await?;
    let plafonne = plafonne(etat, &mut tx, utilisateur, maintenant).await?;
    tx.confirmer().await?;
    // 2. Argon2id sur une chaîne factice, dans tous les cas : même coût et même durée que la
    // connexion par le secours, quel que soit le compte (la durée plancher s'y ajoute).
    verifier_mot_de_passe(etat, None, code.clone()).await?;
    // Courriel inconnu : rien n'est inscrit (ADR 0017).
    let Some(utilisateur) = utilisateur else {
        return Err(ErreurApi::NonAuthentifie);
    };
    // 3. Usage du code (atomique), session, événement et audit : une transaction.
    let mut tx = ouvrir_transaction_connexion(&etat.pool, &courriel).await?;
    let preuve = match empreinte {
        Some(empreinte) if !plafonne => {
            tx.consommer_code(utilisateur, &empreinte, maintenant)
                .await?
        }
        Some(_) | None => None,
    };
    let Some(preuve) = preuve else {
        return refuser_connexion(
            tx,
            utilisateur,
            MethodeSession::CodeRecuperation,
            maintenant,
            client,
        )
        .await;
    };
    let tx = tx.authentifier(preuve).await?;
    // Un code de récupération n'ouvre qu'une session restreinte (F1.12), fixé ici et non
    // laissé à Cedar, qui le décide aussi.
    ouvrir(
        etat,
        tx,
        utilisateur,
        NatureSession::Restreinte,
        MethodeSession::CodeRecuperation,
        maintenant,
        client,
    )
    .await
}

/// Échec d'une connexion sur un compte connu : événement d'échec, puis 401.
pub(super) async fn refuser_connexion(
    mut tx: TransactionConnexion,
    utilisateur: IdUtilisateur,
    methode: MethodeSession,
    maintenant: OffsetDateTime,
    client: &Client,
) -> Result<Response, ErreurApi> {
    tx.inscrire_evenement(&nouvel_evenement(
        utilisateur,
        None,
        NatureEvenement::Connexion,
        Some(methode),
        Issue::Echec,
        maintenant,
        client,
    ))
    .await?;
    tx.confirmer().await?;
    Err(ErreurApi::NonAuthentifie)
}

/// Ouverture de la session après la vérification du facteur : décision `OuvrirSession`,
/// session, événement de réussite, audit de chaque cabinet actif ; une seule transaction.
pub(super) async fn ouvrir(
    etat: &Etat,
    mut tx: TransactionUtilisateur,
    utilisateur: IdUtilisateur,
    nature: NatureSession,
    methode: MethodeSession,
    maintenant: OffsetDateTime,
    client: &Client,
) -> Result<Response, ErreurApi> {
    // Facteur vérifié à l'instant : la session à ouvrir est fraîche.
    let fraiche = SessionAuthz::depuis(maintenant, maintenant);
    let verdict = decider(
        etat,
        utilisateur,
        ActionFacteur::OuvrirSession,
        nature,
        methode,
        fraiche,
    )?;
    let durees = etat.reglages.sessions.raccourcies();
    let ouverte: SessionOuverte = ouvrir_session_dans(
        &mut tx,
        &NouvelleSession {
            utilisateur,
            nature,
            methode,
            instant: maintenant,
            appareil: client.appareil.clone(),
            adresse: client.adresse,
        },
        &durees,
    )
    .await
    .map_err(interne)?;
    evenement::inscrire(
        &mut tx,
        &nouvel_evenement(
            utilisateur,
            Some(ouverte.id),
            NatureEvenement::Connexion,
            Some(methode),
            Issue::Succes,
            maintenant,
            client,
        ),
    )
    .await?;
    let (tx, _) = inscrire_dans_les_cabinets(tx, maintenant, |membre| {
        entree_audit(
            membre,
            utilisateur,
            ouverte.id,
            ActionFacteur::OuvrirSession,
            &verdict,
            fraiche.est_fraiche(),
            methode,
        )
    })
    .await?;
    tx.confirmer().await?;
    let valeur = ouverte.cookie(maintenant).map_err(interne)?;
    let vue = VueConnexion {
        session: ouverte.id.to_string(),
        nature: nature.nom(),
        methode: methode.nom(),
        fin_absolue: instant(ouverte.fin_absolue),
        csrf: ouverte.jeton.csrf(),
    };
    Ok(avec_cookie(Json(vue).into_response(), valeur))
}

// --- Routes d'un utilisateur authentifié ---------------------------------------------------

/// Exécute un cas d'usage de l'utilisateur authentifié sur ses facteurs. Le pipeline a
/// authentifié la requête, vérifié la nature de la session selon la portée et, pour une
/// mutation, le jeton anti-CSRF lié à la session.
pub(crate) async fn executer(
    etat: &Etat,
    route: &Route,
    parts: &Parts,
    identite: &Identite,
    corps: &[u8],
) -> Result<Response, ErreurApi> {
    let ActionRoute::Facteur(action) = route.action else {
        return Err(ErreurApi::Interne(CategorieErreur::Donnee));
    };
    let soi = Soi {
        etat,
        identite,
        action,
        client: client(etat, parts),
        maintenant: etat.horloge.maintenant(),
    };
    let cas = match route.cas_usage {
        CasUsage::Facteurs(cas) => cas,
        CasUsage::Passkeys(cas) => return super::passkeys::executer(&soi, cas, parts, corps).await,
        _ => return Err(ErreurApi::Interne(CategorieErreur::Donnee)),
    };
    match cas {
        Cas::Reauthentifier => reauthentifier(&soi, parts, corps).await,
        Cas::ConsulterFacteurs => consulter_facteurs(&soi).await,
        Cas::ConsulterEvenements => consulter_evenements(&soi).await,
        Cas::PreparerSecours => preparer_secours(&soi).await,
        Cas::EnrolerSecours => enroler_secours(&soi, corps).await,
        Cas::RegenererCodes => regenerer_codes(&soi).await,
        Cas::ConnexionSecours | Cas::ConnexionCode => {
            Err(ErreurApi::Interne(CategorieErreur::Donnee))
        }
    }
}

/// L'utilisateur de la session authentifiée, l'action de la route et la requête.
pub(super) struct Soi<'a> {
    pub(super) etat: &'a Etat,
    pub(super) identite: &'a Identite,
    pub(super) action: ActionFacteur,
    pub(super) client: Client,
    pub(super) maintenant: OffsetDateTime,
}

impl Soi<'_> {
    pub(super) fn utilisateur(&self) -> IdUtilisateur {
        self.identite.utilisateur
    }

    pub(super) async fn transaction(&self) -> Result<TransactionUtilisateur, ErreurApi> {
        Ok(ouvrir_transaction_utilisateur(&self.etat.pool, self.utilisateur()).await?)
    }

    /// Lit la session de la requête (sous RLS) et décide l'action de la route dans son
    /// contexte (nature, méthode, fraîcheur lues en base).
    pub(super) async fn decider(
        &self,
        tx: &mut TransactionUtilisateur,
    ) -> Result<(Session, Verdict), ErreurApi> {
        let s = session::lire(tx, self.identite.session)
            .await?
            .ok_or(ErreurApi::NonAuthentifie)?;
        if s.utilisateur != self.utilisateur() {
            return Err(ErreurApi::NonAuthentifie);
        }
        let verdict = decider(
            self.etat,
            self.utilisateur(),
            self.action,
            s.nature,
            s.methode,
            SessionAuthz::depuis(s.derniere_verification, self.maintenant),
        )?;
        Ok((s, verdict))
    }

    pub(super) fn evenement(
        &self,
        nature: NatureEvenement,
        methode: Option<MethodeSession>,
        issue: Issue,
    ) -> NouvelEvenement {
        nouvel_evenement(
            self.utilisateur(),
            Some(self.identite.session),
            nature,
            methode,
            issue,
            self.maintenant,
            &self.client,
        )
    }

    pub(super) fn limiter(&self) -> Result<(), ErreurApi> {
        limiter(
            &self.etat.facteurs.limiteur_verification,
            self.utilisateur(),
        )
    }

    /// Audit de chaque cabinet actif, puis validation.
    pub(super) async fn auditer_et_confirmer(
        &self,
        tx: TransactionUtilisateur,
        verdict: &Verdict,
        session: &Session,
        methode: MethodeSession,
    ) -> Result<(), ErreurApi> {
        let fraiche = SessionAuthz::depuis(session.derniere_verification, self.maintenant);
        let (tx, _) = inscrire_dans_les_cabinets(tx, self.maintenant, |membre| {
            entree_audit(
                membre,
                self.utilisateur(),
                session.id,
                self.action,
                verdict,
                fraiche.est_fraiche(),
                methode,
            )
        })
        .await?;
        tx.confirmer().await?;
        Ok(())
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CorpsReauthentification {
    mot_de_passe: String,
    code: String,
}

/// Jeton anti-CSRF et fraîcheur après la ré-authentification ; le nouveau jeton de session
/// est dans le cookie.
#[derive(Debug, Serialize)]
struct VueReauthentification {
    csrf: String,
    fraiche_jusqu_a: String,
}

async fn reauthentifier(soi: &Soi<'_>, parts: &Parts, corps: &[u8]) -> Result<Response, ErreurApi> {
    soi.limiter()?;
    let corps: CorpsReauthentification = corps_json(corps, ErreurApi::RequeteInvalide)?;
    let mot_de_passe = Zeroizing::new(corps.mot_de_passe);
    let code = Zeroizing::new(corps.code);
    let present = jeton_de_la_requete(parts).ok_or(ErreurApi::NonAuthentifie)?;
    let cle = cle_instance(soi.etat)?;

    // 1. Décision et lecture du secours, dans une courte transaction.
    let mut tx = soi.transaction().await?;
    let (s, _) = soi.decider(&mut tx).await?;
    let secours = base_facteurs::lire_secours(&mut tx).await?;
    tx.confirmer().await?;

    // 2. Vérifications hors transaction.
    let verification = verifier_mot_de_passe(
        soi.etat,
        secours.as_ref().map(|s| s.mot_de_passe.clone()),
        mot_de_passe,
    )
    .await?;
    let pas = secours
        .as_ref()
        .and_then(|sec| pas_accepte(&cle, sec, &code, soi.maintenant));

    // 3. Consommation du pas, fraîcheur, rotation du jeton, événement.
    let mut tx = soi.transaction().await?;
    let admis = match (&secours, verification, pas) {
        (Some(sec), Verification::Admis { .. }, Some(pas)) => {
            base_facteurs::consommer_pas_totp(&mut tx, &sec.mot_de_passe, pas).await?
        }
        _ => false,
    };
    if !admis {
        evenement::inscrire(
            &mut tx,
            &soi.evenement(
                NatureEvenement::Reauthentification,
                Some(MethodeSession::Secours),
                Issue::Echec,
            ),
        )
        .await?;
        tx.confirmer().await?;
        return Err(ErreurApi::Refus(MotifRefus::FacteursRefuses));
    }
    terminer_reauthentification(soi, tx, &s, &present, MethodeSession::Secours).await
}

/// Fin d'une ré-authentification réussie (secours ou passkey), dans la transaction qui vient de
/// consommer le facteur : fraîcheur, rotation du jeton (ADR 0012), événement. Le nouveau jeton
/// de session est dans le cookie.
pub(super) async fn terminer_reauthentification(
    soi: &Soi<'_>,
    mut tx: TransactionUtilisateur,
    s: &Session,
    present: &JetonSession,
    methode: MethodeSession,
) -> Result<Response, ErreurApi> {
    if !session::marquer_verification(&mut tx, s.id, soi.maintenant).await? {
        return Err(ErreurApi::NonAuthentifie);
    }
    // Rotation à chaque ré-authentification (ADR 0012) : l'ancien jeton ne vaut plus rien.
    let jeton = JetonSession::tirer().map_err(interne)?;
    if !session::tourner(&mut tx, s.id, &present.empreinte(), &jeton.empreinte()).await? {
        return Err(ErreurApi::NonAuthentifie);
    }
    evenement::inscrire(
        &mut tx,
        &soi.evenement(
            NatureEvenement::Reauthentification,
            Some(methode),
            Issue::Succes,
        ),
    )
    .await?;
    tx.confirmer().await?;
    let durees = soi.etat.reglages.sessions.raccourcies();
    let valeur = cookie(&jeton, s.fin_absolue(&durees) - soi.maintenant).map_err(interne)?;
    let vue = VueReauthentification {
        csrf: jeton.csrf(),
        fraiche_jusqu_a: instant(soi.maintenant + authz::DUREE_FRAICHEUR),
    };
    Ok(avec_cookie(Json(vue).into_response(), valeur))
}

/// État de ses facteurs : jamais de secret ni d'empreinte.
#[derive(Debug, Serialize)]
struct VueFacteurs {
    secours: Option<VueSecours>,
    codes_restants: i64,
    /// Ses passkeys (PR 8c) : identifiant, nom, dates ; jamais la clé ni le compteur.
    passkeys: Vec<VuePasskey>,
}

#[derive(Debug, Serialize)]
struct VuePasskey {
    id: String,
    nom: String,
    cree_le: String,
    derniere_utilisation: Option<String>,
}

#[derive(Debug, Serialize)]
struct VueSecours {
    modifie_le: String,
    /// Faux si le secret TOTP a été chiffré par une autre clé de l'instance : à réenrôler.
    utilisable: bool,
}

async fn consulter_facteurs(soi: &Soi<'_>) -> Result<Response, ErreurApi> {
    let cle = cle_instance(soi.etat)?;
    let mut tx = soi.transaction().await?;
    let _autorisee = soi.decider(&mut tx).await?;
    let secours = base_facteurs::lire_secours(&mut tx).await?;
    let codes_restants = base_facteurs::compter_codes_restants(&mut tx).await?;
    let passkeys = base_passkeys::lister(&mut tx).await?;
    tx.confirmer().await?;
    let vue = VueFacteurs {
        secours: secours
            .filter(|s| s.utilisateur == soi.utilisateur())
            .map(|s| VueSecours {
                modifie_le: instant(s.modifie_le),
                utilisable: s.totp_cle_id == cle.identifiant(),
            }),
        codes_restants,
        passkeys: passkeys
            .into_iter()
            .filter(|p| p.utilisateur == soi.utilisateur())
            .map(|p| VuePasskey {
                id: p.id.to_string(),
                nom: p.nom,
                cree_le: instant(p.cree_le),
                derniere_utilisation: p.derniere_utilisation.map(instant),
            })
            .collect(),
    };
    Ok(Json(vue).into_response())
}

/// Un événement d'authentification, tel que son titulaire le lit.
#[derive(Debug, Serialize)]
struct VueEvenement {
    id: String,
    nature: String,
    methode: Option<String>,
    issue: String,
    instant: String,
    session: Option<String>,
    appareil: Option<String>,
    adresse: Option<String>,
}

async fn consulter_evenements(soi: &Soi<'_>) -> Result<Response, ErreurApi> {
    let mut tx = soi.transaction().await?;
    let _autorisee = soi.decider(&mut tx).await?;
    let lus = evenement::lister(&mut tx, EVENEMENTS_MAX).await?;
    tx.confirmer().await?;
    let vues: Vec<VueEvenement> = lus
        .into_iter()
        .map(|e| VueEvenement {
            id: e.id.to_string(),
            nature: e.nature,
            methode: e.methode,
            issue: e.issue,
            instant: instant(e.instant),
            session: e.session.map(|s| s.to_string()),
            appareil: e.appareil,
            adresse: e.adresse,
        })
        .collect();
    Ok(Json(vues).into_response())
}

/// Préparation de l'enrôlement : secret TOTP (à saisir ou à lire en QR code) et jeton
/// d'enrôlement qui le scelle, lié à l'utilisateur, à la session et à son échéance. Rien n'est
/// stocké.
#[derive(Debug, Serialize)]
struct VuePreparation {
    secret: String,
    uri: String,
    jeton_enrolement: String,
    expire_le: String,
}

async fn preparer_secours(soi: &Soi<'_>) -> Result<Response, ErreurApi> {
    let cle = cle_instance(soi.etat)?;
    let mut tx = soi.transaction().await?;
    let _autorisee = soi.decider(&mut tx).await?;
    let courriel = base_facteurs::lire_courriel(&mut tx)
        .await?
        .ok_or(ErreurApi::NonAuthentifie)?;
    tx.confirmer().await?;
    let secret = SecretTotp::tirer().map_err(interne)?;
    let echeance = soi.maintenant + VALIDITE_ENROLEMENT;
    let scelle = cle
        .sceller_enrolement(
            soi.utilisateur().uuid(),
            soi.identite.session.uuid(),
            echeance.unix_timestamp(),
            &secret,
        )
        .map_err(interne)?;
    let vue = VuePreparation {
        secret: secret.base32(),
        uri: totp::uri_otpauth(&secret, EMETTEUR, &courriel),
        jeton_enrolement: format!("{}.{scelle}", echeance.unix_timestamp()),
        expire_le: instant(echeance),
    };
    Ok(Json(vue).into_response())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CorpsEnrolement {
    jeton_enrolement: String,
    mot_de_passe: String,
    code: String,
}

/// Codes de récupération, remis une seule fois.
#[derive(Debug, Serialize)]
struct VueCodes {
    codes_recuperation: Vec<String>,
}

fn vue_codes(serie: &[CodeRecuperation]) -> VueCodes {
    VueCodes {
        codes_recuperation: serie.iter().map(|c| c.texte().to_owned()).collect(),
    }
}

/// Secret TOTP d'un jeton d'enrôlement non échu de cette session.
fn ouvrir_jeton(soi: &Soi<'_>, cle: &CleInstance, jeton: &str) -> Result<SecretTotp, ErreurApi> {
    let refus = ErreurApi::Refus(MotifRefus::JetonEnrolementInvalide);
    let (echeance, scelle) = jeton.split_once('.').ok_or(refus)?;
    let echeance: i64 = echeance.parse().map_err(|_| refus)?;
    if echeance <= soi.maintenant.unix_timestamp() {
        return Err(refus);
    }
    cle.ouvrir_enrolement(
        soi.utilisateur().uuid(),
        soi.identite.session.uuid(),
        echeance,
        scelle,
    )
    .ok_or(refus)
}

async fn enroler_secours(soi: &Soi<'_>, corps: &[u8]) -> Result<Response, ErreurApi> {
    soi.limiter()?;
    let corps: CorpsEnrolement = corps_json(corps, ErreurApi::RequeteInvalide)?;
    let mot_de_passe = Zeroizing::new(corps.mot_de_passe);
    let code = Zeroizing::new(corps.code);
    let cle = cle_instance(soi.etat)?;
    // Décision d'abord : rien n'est vérifié pour une session qui n'y a pas droit.
    let mut tx = soi.transaction().await?;
    let _autorisee = soi.decider(&mut tx).await?;
    tx.confirmer().await?;
    mot_de_passe::valider(&mot_de_passe).map_err(|refus| {
        ErreurApi::Refus(match refus {
            RefusMotDePasse::TropCourt => MotifRefus::MotDePasseTropCourt,
            RefusMotDePasse::TropLong => MotifRefus::MotDePasseTropLong,
            RefusMotDePasse::Compromis => MotifRefus::MotDePasseCompromis,
        })
    })?;
    let secret = ouvrir_jeton(soi, &cle, &corps.jeton_enrolement)?;
    let pas = totp::verifier(&secret, &code, soi.maintenant)
        .ok_or(ErreurApi::Refus(MotifRefus::CodeInvalide))?;
    let chaine = hacher_mot_de_passe(soi.etat, mot_de_passe).await?;
    let chiffre = cle
        .chiffrer_totp(soi.utilisateur().uuid(), &secret)
        .map_err(interne)?;
    let serie = codes::tirer_serie().map_err(interne)?;
    let empreintes: Vec<[u8; 32]> = serie.iter().map(CodeRecuperation::empreinte).collect();

    // Une transaction : décision revérifiée sur l'état exact, secours, codes, révocation des
    // autres sessions, événement, audit.
    let mut tx = soi.transaction().await?;
    let (s, verdict) = soi.decider(&mut tx).await?;
    base_facteurs::enregistrer_secours(
        &mut tx,
        &chaine,
        &chiffre,
        &cle.identifiant(),
        pas,
        soi.maintenant,
    )
    .await?;
    base_facteurs::remplacer_codes(&mut tx, &empreintes, soi.maintenant).await?;
    session::revoquer_autres(
        &mut tx,
        s.id,
        soi.maintenant,
        MotifRevocation::ChangementFacteur,
    )
    .await?;
    evenement::inscrire(
        &mut tx,
        &soi.evenement(
            NatureEvenement::EnrolementSecours,
            Some(MethodeSession::Secours),
            Issue::Succes,
        ),
    )
    .await?;
    soi.auditer_et_confirmer(tx, &verdict, &s, MethodeSession::Secours)
        .await?;
    Ok((StatusCode::CREATED, Json(vue_codes(&serie))).into_response())
}

async fn regenerer_codes(soi: &Soi<'_>) -> Result<Response, ErreurApi> {
    let serie = codes::tirer_serie().map_err(interne)?;
    let empreintes: Vec<[u8; 32]> = serie.iter().map(CodeRecuperation::empreinte).collect();
    let mut tx = soi.transaction().await?;
    let (s, verdict) = soi.decider(&mut tx).await?;
    base_facteurs::remplacer_codes(&mut tx, &empreintes, soi.maintenant).await?;
    session::revoquer_autres(
        &mut tx,
        s.id,
        soi.maintenant,
        MotifRevocation::ChangementFacteur,
    )
    .await?;
    evenement::inscrire(
        &mut tx,
        &soi.evenement(
            NatureEvenement::GenerationCodes,
            Some(MethodeSession::CodeRecuperation),
            Issue::Succes,
        ),
    )
    .await?;
    soi.auditer_et_confirmer(tx, &verdict, &s, MethodeSession::CodeRecuperation)
        .await?;
    Ok((StatusCode::CREATED, Json(vue_codes(&serie))).into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn courriel_saisi_borne() {
        assert_eq!(
            courriel_saisi("  a@b.invalid "),
            Some("a@b.invalid".to_owned())
        );
        for invalide in ["", "ab", "@b.invalid", "sans-arobase", &"a".repeat(255)] {
            assert_eq!(courriel_saisi(invalide), None, "{invalide}");
        }
    }

    #[test]
    fn reglages_par_defaut_sans_cle() {
        let r = ReglagesFacteurs::default();
        assert!(r.cle_instance.is_none());
        assert_eq!(r.debit_connexion_courriel.capacite, 10);
        assert_eq!(r.hachages_simultanes, 4);
    }
}
