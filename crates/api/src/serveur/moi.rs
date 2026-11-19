//! Routes hors de tout cabinet (ADR 0012 ; L2-17) : la session courante et les ressources
//! propres de l'utilisateur authentifié.
//!
//! Le pipeline a déjà authentifié la requête, vérifié la nature de la session (complète, sauf
//! pour la session courante elle-même) et, pour une mutation, le jeton anti-CSRF lié à la
//! session. Chaque cas d'usage s'exécute dans une [`TransactionUtilisateur`] : sous RLS, elle ne
//! voit que les sessions de cet utilisateur, ses appartenances et leurs cabinets ; chaque
//! requête filtre aussi par l'utilisateur. La session d'un autre est donc **absente** (404).
//!
//! # Cedar ou non
//! - `GET /api/moi/cabinets` liste des cabinets : **Cedar** décide `AccederCabinet` pour chaque
//!   appartenance (principal : l'appartenance ; ressource : son cabinet). Une appartenance
//!   suspendue est refusée par `invariant-appartenance-active` (et écartée par la requête).
//! - Session courante, liste et révocation de ses sessions, rotation, déconnexion : **hors
//!   Cedar**. Le principal Cedar est une appartenance à un cabinet ; une session n'appartient à
//!   aucun cabinet et ne regarde que son titulaire, comme la gestion de ses propres facteurs
//!   (doc du crate `authz`, « Hors Cedar »). Deux niveaux la bornent : la RLS
//!   (`utilisateur_id = sigma.utilisateur`) et le filtre de chaque requête. La consultation et
//!   la révocation des sessions **d'un autre membre** par l'administrateur du cabinet (spec §21,
//!   PR 9) seront, elles, des actions Cedar du cabinet (`GererCabinet`).
//!
//! # Rotation et cabinet actif
//! L'ADR 0012 demande une rotation du jeton « au changement de cabinet actif » ; l'ADR 0011
//! interdit tout cabinet courant mémorisé en session (le cabinet est lu dans le chemin de chaque
//! requête, F1.14). Choix retenu : `POST /api/session/rotation`, que l'interface appelle quand
//! l'utilisateur change de cabinet ; le serveur tourne le jeton sans rien mémoriser du cabinet.
//!
//! # Audit
//! Ces actions ne touchent aucun cabinet : leur traçabilité (événements hors cabinet) attend une
//! décision du mainteneur (ADR 0012 renvoie à l'ADR 0014, qui ne la couvre pas). Les points
//! d'insertion sont marqués « Point d'insertion de l'audit ». Aucune trace n'est donc exigée
//! d'elles par le pipeline.

use authz::{Action, Contexte, Decision, Entites, Ressource, Session as SessionAuthz};
use axum::Json;
use axum::extract::{FromRequestParts, RawPathParams};
use axum::http::request::Parts;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::Serialize;
use time::OffsetDateTime;

use super::Etat;
use super::authentification::Identite;
use super::erreur::{CategorieErreur, ErreurApi};
use super::pipeline::{id_authz, id_chemin, role};
use super::routes::{CasUsage, Route};
use super::session::{JetonSession, cookie, cookie_efface, jeton_de_la_requete};
use crate::base::organisation::lister_cabinets_de_l_utilisateur;
use crate::base::session::{self, DureesSession, MotifRevocation, Session};
use crate::base::{IdSession, TransactionUtilisateur, ouvrir_transaction_utilisateur};

/// Exécute un cas d'usage hors cabinet, dans une transaction au contexte du seul utilisateur.
pub(crate) async fn executer(
    etat: &Etat,
    route: &'static Route,
    parts: &mut Parts,
    identite: &Identite,
) -> Result<Response, ErreurApi> {
    let maintenant = etat.horloge.maintenant();
    let durees = etat.reglages.sessions.raccourcies();
    let mut tx = ouvrir_transaction_utilisateur(&etat.pool, identite.utilisateur).await?;
    let reponse = match route.cas_usage {
        CasUsage::EtatSession => etat_session(&mut tx, parts, identite, maintenant, &durees).await,
        CasUsage::TournerJeton => tourner(&mut tx, identite, maintenant, &durees).await,
        CasUsage::Deconnecter => deconnecter(&mut tx, identite, maintenant).await,
        CasUsage::ListerSessions => lister(&mut tx, identite, maintenant, &durees).await,
        CasUsage::RevoquerSession => revoquer(&mut tx, parts, maintenant).await,
        CasUsage::ListerCabinets => {
            lister_cabinets(etat, &mut tx, route, identite, maintenant).await
        }
        CasUsage::LireCabinet
        | CasUsage::ListerDossiers
        | CasUsage::LireDossier
        | CasUsage::CreerDossier
        | CasUsage::ConsulterAudit
        | CasUsage::VerifierAudit
        | CasUsage::ExporterAudit => Err(ErreurApi::Interne(CategorieErreur::Donnee)),
    }?;
    tx.confirmer().await?;
    Ok(reponse)
}

/// Instant au format de l'audit (`AAAA-MM-JJTHH:MM:SS.ffffffZ`).
fn instant(i: OffsetDateTime) -> String {
    integrite::audit::formater_instant(i)
}

/// État de la session courante.
#[derive(Debug, Serialize)]
struct VueEtatSession {
    id: String,
    nature: &'static str,
    methode: &'static str,
    cree_le: String,
    /// Fin absolue (12 heures pour une session complète).
    fin_absolue: String,
    /// Fin par inactivité, si aucune requête ne survient d'ici là.
    fin_inactivite: String,
    derniere_verification: String,
    /// Vrai si une action sensible est admise sans nouvelle vérification de facteur.
    fraiche: bool,
    fraiche_jusqu_a: String,
    /// Jeton anti-CSRF lié à la session, à renvoyer dans `x-sigma-csrf` sur toute mutation.
    csrf: String,
}

async fn session_courante(
    tx: &mut TransactionUtilisateur,
    identite: &Identite,
) -> Result<Session, ErreurApi> {
    session::lire(tx, identite.session)
        .await?
        .ok_or(ErreurApi::NonAuthentifie)
}

async fn etat_session(
    tx: &mut TransactionUtilisateur,
    parts: &Parts,
    identite: &Identite,
    maintenant: OffsetDateTime,
    durees: &DureesSession,
) -> Result<Response, ErreurApi> {
    // Le jeton du cookie que l'authentification vient de valider : il donne le jeton
    // anti-CSRF, jamais renvoyé lui-même.
    let jeton = jeton_de_la_requete(parts).ok_or(ErreurApi::NonAuthentifie)?;
    let s = session_courante(tx, identite).await?;
    let vue = VueEtatSession {
        id: s.id.to_string(),
        nature: s.nature.nom(),
        methode: s.methode.nom(),
        cree_le: instant(s.cree_le),
        fin_absolue: instant(s.fin_absolue(durees)),
        fin_inactivite: instant(s.fin_inactivite(durees)),
        derniere_verification: instant(s.derniere_verification),
        fraiche: SessionAuthz::depuis(s.derniere_verification, maintenant).est_fraiche(),
        fraiche_jusqu_a: instant(s.derniere_verification + authz::DUREE_FRAICHEUR),
        csrf: jeton.csrf(),
    };
    Ok(Json(vue).into_response())
}

/// Réponse portant un `Set-Cookie`.
fn avec_cookie(mut reponse: Response, valeur: HeaderValue) -> Response {
    reponse.headers_mut().insert(header::SET_COOKIE, valeur);
    reponse
}

/// Jeton anti-CSRF de la session après rotation.
#[derive(Debug, Serialize)]
struct VueRotation {
    csrf: String,
}

async fn tourner(
    tx: &mut TransactionUtilisateur,
    identite: &Identite,
    maintenant: OffsetDateTime,
    durees: &DureesSession,
) -> Result<Response, ErreurApi> {
    let jeton =
        JetonSession::tirer().map_err(|_| ErreurApi::Interne(CategorieErreur::Authentification))?;
    if !session::tourner(tx, identite.session, &jeton.empreinte()).await? {
        return Err(ErreurApi::NonAuthentifie);
    }
    let s = session_courante(tx, identite).await?;
    let valeur = cookie(&jeton, s.fin_absolue(durees) - maintenant)
        .map_err(|_| ErreurApi::Interne(CategorieErreur::Donnee))?;
    // Point d'insertion de l'audit (hors cabinet) : rotation du jeton de la session.
    let vue = VueRotation { csrf: jeton.csrf() };
    Ok(avec_cookie(Json(vue).into_response(), valeur))
}

async fn deconnecter(
    tx: &mut TransactionUtilisateur,
    identite: &Identite,
    maintenant: OffsetDateTime,
) -> Result<Response, ErreurApi> {
    // Déjà révoquée entre-temps : la déconnexion reste acquise.
    session::revoquer(
        tx,
        identite.session,
        maintenant,
        MotifRevocation::Deconnexion,
    )
    .await?;
    // Point d'insertion de l'audit (hors cabinet) : déconnexion.
    Ok(avec_cookie(
        StatusCode::NO_CONTENT.into_response(),
        cookie_efface(),
    ))
}

/// Une session de l'utilisateur, sans jeton ni empreinte.
#[derive(Debug, Serialize)]
struct VueSession {
    id: String,
    nature: &'static str,
    methode: &'static str,
    cree_le: String,
    derniere_activite: String,
    echeance: String,
    appareil: Option<String>,
    adresse: Option<String>,
    /// Vrai pour la session de la requête.
    courante: bool,
}

async fn lister(
    tx: &mut TransactionUtilisateur,
    identite: &Identite,
    maintenant: OffsetDateTime,
    durees: &DureesSession,
) -> Result<Response, ErreurApi> {
    let vues: Vec<VueSession> = session::lister(tx)
        .await?
        .into_iter()
        .filter(|s| s.utilisateur == identite.utilisateur && s.est_valide(maintenant, durees))
        .map(|s| VueSession {
            id: s.id.to_string(),
            nature: s.nature.nom(),
            methode: s.methode.nom(),
            cree_le: instant(s.cree_le),
            derniere_activite: instant(s.derniere_activite),
            echeance: instant(s.echeance(durees)),
            courante: s.id == identite.session,
            appareil: s.appareil,
            adresse: s.adresse,
        })
        .collect();
    Ok(Json(vues).into_response())
}

async fn revoquer(
    tx: &mut TransactionUtilisateur,
    parts: &mut Parts,
    maintenant: OffsetDateTime,
) -> Result<Response, ErreurApi> {
    let parametres = RawPathParams::from_request_parts(parts, &())
        .await
        .map_err(|_| ErreurApi::RequeteInvalide)?;
    let id = IdSession::depuis_uuid(id_chemin(&parametres, "session")?);
    // Session d'un autre utilisateur, inexistante ou déjà révoquée : absente (404).
    if !session::revoquer(tx, id, maintenant, MotifRevocation::RevocationUtilisateur).await? {
        return Err(ErreurApi::Introuvable);
    }
    // Point d'insertion de l'audit (hors cabinet) : révocation d'une session.
    Ok(StatusCode::NO_CONTENT.into_response())
}

/// Un cabinet de l'utilisateur, avec son rôle.
#[derive(Debug, Serialize)]
struct VueCabinetMembre {
    id: String,
    nom: String,
    role: String,
}

async fn lister_cabinets(
    etat: &Etat,
    tx: &mut TransactionUtilisateur,
    route: &Route,
    identite: &Identite,
    maintenant: OffsetDateTime,
) -> Result<Response, ErreurApi> {
    let action = route
        .action
        .filter(|a| *a == Action::AccederCabinet)
        .ok_or(ErreurApi::Interne(CategorieErreur::Donnee))?;
    let utilisateur = id_authz(identite.utilisateur.uuid(), authz::IdUtilisateur::new)?;
    let contexte = Contexte::new(SessionAuthz::depuis(
        identite.derniere_verification,
        maintenant,
    ));
    let mut vues = Vec::new();
    for (appartenance, cabinet) in lister_cabinets_de_l_utilisateur(tx).await? {
        if appartenance.utilisateur != identite.utilisateur || appartenance.cabinet != cabinet.id {
            return Err(ErreurApi::Interne(CategorieErreur::Donnee));
        }
        let id_cabinet = id_authz(cabinet.id.uuid(), authz::IdCabinet::new)?;
        let principal = authz::Appartenance {
            id: id_authz(appartenance.id.uuid(), authz::IdAppartenance::new)?,
            utilisateur,
            cabinet: id_cabinet,
            role: role(&appartenance.role)?,
            actif: appartenance.actif,
        };
        let entites = Entites::new().avec(authz::Cabinet {
            id: id_cabinet,
            separation_des_taches: cabinet.separation_taches,
        });
        let decision = etat.decideur.decider(
            &principal,
            action,
            Ressource::Cabinet(id_cabinet),
            &entites,
            &contexte,
        );
        if decision == Decision::Autorise {
            vues.push(VueCabinetMembre {
                id: cabinet.id.to_string(),
                nom: cabinet.nom,
                role: appartenance.role,
            });
        }
    }
    Ok(Json(vues).into_response())
}
