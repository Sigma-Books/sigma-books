//! Routes hors de tout cabinet (ADR 0012 ; L2-17) : la session courante et les ressources
//! propres de l'utilisateur authentifié.
//!
//! Le pipeline a déjà authentifié la requête, vérifié la nature de la session (complète, sauf
//! pour la session courante elle-même) et, pour une mutation, le jeton anti-CSRF lié à la
//! session. Chaque cas d'usage s'exécute dans une [`TransactionUtilisateur`] : sous RLS, elle ne
//! voit que les sessions de cet utilisateur, ses appartenances et leurs cabinets ; chaque
//! requête filtre aussi par l'utilisateur. La session d'un autre est donc **absente** (404).
//!
//! # Décisions Cedar (règle absolue 7)
//! Toute route passe par Cedar ; trois niveaux bornent chacune, comme dans un cabinet.
//! - `GET /api/moi/cabinets` : Cedar décide `AccederCabinet` pour chaque appartenance
//!   (principal : l'appartenance ; ressource : son cabinet). Une appartenance suspendue est
//!   refusée par `invariant-appartenance-active` (et écartée par la requête).
//! - Session courante, rotation, déconnexion, liste et révocation de ses sessions : principal
//!   **Utilisateur** (l'utilisateur de la session authentifiée), ressource lue **en base sous
//!   RLS** (la session et son titulaire, ou l'utilisateur lui-même pour la liste), décision
//!   [`authz::Autorisation::decider_utilisateur`]. La politique n'autorise que le titulaire ;
//!   la RLS (`utilisateur_id = sigma.utilisateur`) et le filtre de chaque requête ne laissent
//!   de toute façon voir que les siennes : la session d'un autre est absente (404). La
//!   déconnexion est la révocation de la session courante (`RevoquerSaSession`). La gestion
//!   des sessions **d'un autre membre** par l'administrateur du cabinet (spec §21, PR 9) sera,
//!   elle, une action de cabinet (principal Appartenance).
//!
//! # Rotation et cabinet actif
//! L'ADR 0012 demande une rotation du jeton « au changement de cabinet actif » ; l'ADR 0011
//! interdit tout cabinet courant mémorisé en session (le cabinet est lu dans le chemin de chaque
//! requête, F1.14). Choix retenu : `POST /api/session/rotation`, que l'interface appelle quand
//! l'utilisateur change de cabinet ; le serveur tourne le jeton sans rien mémoriser du cabinet.
//!
//! # Audit
//! Ces actions ne touchent aucun cabinet : aucune trace d'audit chaîné n'est exigée d'elles par
//! le pipeline. La déconnexion et la révocation d'une session sont inscrites au journal
//! d'instance des événements d'authentification de l'utilisateur (ADR 0017), dans la
//! transaction de l'action ; la rotation, sans effet sur l'accès, ne l'est pas.

use authz::{
    Action, ActionUtilisateur, Contexte, Decision, Entites, Ressource, RessourceUtilisateur,
    Session as SessionAuthz,
};
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
use super::routes::{self, ActionRoute, CasUsage, Route};
use super::session::{JetonSession, cookie, cookie_efface, jeton_de_la_requete};
use crate::base::evenement::{self, Issue, NatureEvenement, NouvelEvenement};
use crate::base::organisation::lister_cabinets_de_l_utilisateur;
use crate::base::session::{self, DureesSession, MotifRevocation, Session};
use crate::base::{
    IdSession, IdUtilisateur, TransactionUtilisateur, ouvrir_transaction_utilisateur,
};

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
    let soi = || Soi::new(etat, route, identite);
    let reponse = match route.cas_usage {
        CasUsage::Session(routes::session::Cas::EtatSession) => {
            etat_session(&mut tx, &soi()?, parts, identite, maintenant, &durees).await
        }
        CasUsage::Session(routes::session::Cas::TournerJeton) => {
            tourner(&mut tx, &soi()?, parts, identite, maintenant, &durees).await
        }
        CasUsage::Session(routes::session::Cas::Deconnecter) => {
            deconnecter(&mut tx, &soi()?, identite, maintenant).await
        }
        CasUsage::Moi(routes::moi::Cas::ListerSessions) => {
            lister(&mut tx, &soi()?, identite, maintenant, &durees).await
        }
        CasUsage::Moi(routes::moi::Cas::RevoquerSession) => {
            revoquer(&mut tx, &soi()?, parts, identite, maintenant).await
        }
        CasUsage::Moi(routes::moi::Cas::ListerCabinets) => {
            lister_cabinets(etat, &mut tx, route, identite, maintenant).await
        }
        // Cas d'usage d'un cabinet ou d'un dossier (référentiel, pièces) : pipeline complet,
        // jamais ici ; facteurs (voie authentification) : module `serveur::facteurs`.
        CasUsage::Organisation(_)
        | CasUsage::Audit(_)
        | CasUsage::Facteurs(_)
        | CasUsage::Passkeys(_)
        | CasUsage::Referentiel(_)
        | CasUsage::Pieces(_)
        | CasUsage::Brouillard(_)
        | CasUsage::Editions(_)
        | CasUsage::Integrite(_)
        | CasUsage::Administration(_) => Err(ErreurApi::Interne(CategorieErreur::Donnee)),
    }?;
    tx.confirmer().await?;
    Ok(reponse)
}

/// Décision Cedar d'une action de l'utilisateur sur lui-même : principal, l'utilisateur de la
/// session authentifiée ; action, celle de la route. Seule voie des cas d'usage de ce module
/// (hors liste des cabinets) vers une ressource.
struct Soi<'a> {
    etat: &'a Etat,
    principal: authz::IdUtilisateur,
    utilisateur: IdUtilisateur,
    action: ActionUtilisateur,
}

impl<'a> Soi<'a> {
    fn new(etat: &'a Etat, route: &Route, identite: &Identite) -> Result<Self, ErreurApi> {
        let ActionRoute::Utilisateur(action) = route.action else {
            return Err(ErreurApi::Interne(CategorieErreur::Donnee));
        };
        Ok(Self {
            etat,
            principal: id_authz(identite.utilisateur.uuid(), authz::IdUtilisateur::new)?,
            utilisateur: identite.utilisateur,
            action,
        })
    }

    fn autorise(&self, action: ActionUtilisateur, ressource: RessourceUtilisateur) -> bool {
        self.etat
            .decideur
            .verdict_utilisateur(self.principal, action, ressource)
            .decision
            == Decision::Autorise
    }

    /// La session lue en base (sous RLS), comme ressource Cedar : son titulaire en base.
    fn ressource(s: &Session) -> Result<RessourceUtilisateur, ErreurApi> {
        Ok(RessourceUtilisateur::Session {
            id: id_authz(s.id.uuid(), authz::IdSession::new)?,
            titulaire: id_authz(s.utilisateur.uuid(), authz::IdUtilisateur::new)?,
        })
    }

    /// Exige l'action de la route sur une session lue en base. Refus : session d'autrui
    /// (titulaire différent), donc absente (404) ; sinon action refusée (403).
    fn exiger_session(&self, s: &Session) -> Result<(), ErreurApi> {
        if self.autorise(self.action, Self::ressource(s)?) {
            Ok(())
        } else if s.utilisateur == self.utilisateur {
            Err(ErreurApi::Interdit)
        } else {
            Err(ErreurApi::Introuvable)
        }
    }
}

/// Événement d'authentification d'une action sur ses sessions (ADR 0017) : la session de la
/// requête, sans appareil ni adresse (ceux de la session restent dans sa ligne).
fn evenement_de_session(
    identite: &Identite,
    nature: NatureEvenement,
    instant: OffsetDateTime,
) -> NouvelEvenement {
    NouvelEvenement {
        utilisateur: identite.utilisateur,
        session: Some(identite.session),
        nature,
        methode: None,
        issue: Issue::Succes,
        instant,
        appareil: None,
        adresse: None,
    }
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
    soi: &Soi<'_>,
    parts: &Parts,
    identite: &Identite,
    maintenant: OffsetDateTime,
    durees: &DureesSession,
) -> Result<Response, ErreurApi> {
    // Le jeton du cookie que l'authentification vient de valider : il donne le jeton
    // anti-CSRF, jamais renvoyé lui-même.
    let jeton = jeton_de_la_requete(parts).ok_or(ErreurApi::NonAuthentifie)?;
    let s = session_courante(tx, identite).await?;
    soi.exiger_session(&s)?;
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
    soi: &Soi<'_>,
    parts: &Parts,
    identite: &Identite,
    maintenant: OffsetDateTime,
    durees: &DureesSession,
) -> Result<Response, ErreurApi> {
    // Le jeton présenté, que l'authentification vient de valider : la rotation n'aboutit que si
    // la session porte encore son empreinte (rotation concurrente : 401).
    let present = jeton_de_la_requete(parts).ok_or(ErreurApi::NonAuthentifie)?;
    let s = session_courante(tx, identite).await?;
    soi.exiger_session(&s)?;
    let jeton =
        JetonSession::tirer().map_err(|_| ErreurApi::Interne(CategorieErreur::Authentification))?;
    if !session::tourner(
        tx,
        identite.session,
        &present.empreinte(),
        &jeton.empreinte(),
    )
    .await?
    {
        return Err(ErreurApi::NonAuthentifie);
    }
    // Même ligne, même ouverture : la fin absolue ne change pas.
    let valeur = cookie(&jeton, s.fin_absolue(durees) - maintenant)
        .map_err(|_| ErreurApi::Interne(CategorieErreur::Donnee))?;
    // Rotation sans effet sur l'accès (même session) : aucun événement (ADR 0017).
    let vue = VueRotation { csrf: jeton.csrf() };
    Ok(avec_cookie(Json(vue).into_response(), valeur))
}

async fn deconnecter(
    tx: &mut TransactionUtilisateur,
    soi: &Soi<'_>,
    identite: &Identite,
    maintenant: OffsetDateTime,
) -> Result<Response, ErreurApi> {
    // La déconnexion est la révocation de la session courante (`RevoquerSaSession`).
    soi.exiger_session(&session_courante(tx, identite).await?)?;
    // Déjà révoquée entre-temps : la déconnexion reste acquise.
    session::revoquer(
        tx,
        identite.session,
        maintenant,
        MotifRevocation::Deconnexion,
    )
    .await?;
    evenement::inscrire(
        tx,
        &evenement_de_session(identite, NatureEvenement::Deconnexion, maintenant),
    )
    .await?;
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
    soi: &Soi<'_>,
    identite: &Identite,
    maintenant: OffsetDateTime,
    durees: &DureesSession,
) -> Result<Response, ErreurApi> {
    // La liste porte sur l'utilisateur lui-même ; chaque session listée est en outre décidée
    // (`ConsulterSaSession`, titulaire lu en base) : aucune ne sort sans décision.
    if !soi.autorise(soi.action, RessourceUtilisateur::Utilisateur(soi.principal)) {
        return Err(ErreurApi::Interdit);
    }
    let mut visibles = Vec::new();
    for s in session::lister(tx).await? {
        if s.utilisateur == identite.utilisateur
            && s.est_valide(maintenant, durees)
            && soi.autorise(ActionUtilisateur::ConsulterSaSession, Soi::ressource(&s)?)
        {
            visibles.push(s);
        }
    }
    let vues: Vec<VueSession> = visibles
        .into_iter()
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
    soi: &Soi<'_>,
    parts: &mut Parts,
    identite: &Identite,
    maintenant: OffsetDateTime,
) -> Result<Response, ErreurApi> {
    let parametres = RawPathParams::from_request_parts(parts, &())
        .await
        .map_err(|_| ErreurApi::RequeteInvalide)?;
    let id = IdSession::depuis_uuid(id_chemin(&parametres, "session")?);
    // Session lue sous RLS (et filtrée par l'utilisateur) : celle d'un autre est absente (404),
    // avant même la décision, qui la refuserait aussi (titulaire différent).
    let cible = session::lire(tx, id).await?.ok_or(ErreurApi::Introuvable)?;
    soi.exiger_session(&cible)?;
    // Inexistante ou déjà révoquée : absente (404).
    if !session::revoquer(tx, id, maintenant, MotifRevocation::RevocationUtilisateur).await? {
        return Err(ErreurApi::Introuvable);
    }
    evenement::inscrire(
        tx,
        &NouvelEvenement {
            session: Some(id),
            ..evenement_de_session(identite, NatureEvenement::RevocationSession, maintenant)
        },
    )
    .await?;
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
    let action = match route.action {
        ActionRoute::Cabinet(Action::AccederCabinet) => Action::AccederCabinet,
        _ => return Err(ErreurApi::Interne(CategorieErreur::Donnee)),
    };
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
