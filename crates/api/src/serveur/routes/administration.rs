//! Routes de l'administration du cabinet (PR 9a de `docs/lots/L2.md` ; ADR 0012, 0014, 0023) :
//! membres, rôles, suspension, affectations aux dossiers, sessions d'un membre, séparation des
//! tâches, invitations, réinitialisation de l'accès d'un membre.
//!
//! Toutes sont des actions du cabinet, décidées par Cedar sur le cabinet du chemin :
//! `GererCabinet` (lectures et annulation d'une réinitialisation), `ModifierDroits` (invitation,
//! rôle, suspension, affectation, sessions : ré-authentification), `ReinitialiserAcces`
//! (ré-authentification), `ActiverSeparationDesTaches` et `DesactiverSeparationDesTaches`
//! (ré-authentification). En défense en profondeur, chaque cas relit l'appartenance de l'acteur
//! **en base** : administrateur actif, ou la ressource est d'autrui (404).
//!
//! Chaque mutation inscrit une trace dans l'audit **du cabinet**, dans sa transaction. Aucune
//! adresse de courriel ni aucun nom n'y figure : la chaîne est définitive, et les données
//! personnelles n'y ont pas leur place (loi 2008-12).
//!
//! Deux routes ne relèvent pas d'un cabinet : l'usage d'un lien (anonyme) et l'acceptation d'une
//! invitation par un utilisateur connecté, dans `serveur::liens`.

use authz::{Action, ActionFacteur};
use axum::Json;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{ActionRoute, CasUsage, Methode, Module, Portee, Requete, Resultat, Route};
use crate::base::administration::{
    self as adm, CreationInvitation, CreationReinitialisation, NouvelleInvitation, ROLES,
};
use crate::base::audit::PorteeAudit;
use crate::base::organisation::{lire_appartenance, lire_dossier};
use crate::base::session::MotifRevocation;
use crate::base::taches::{SujetCourriel, planifier_courriel};
use crate::base::{IdDossier, IdUtilisateur, TransactionTenant};
use crate::courriel;
use crate::serveur::audit::Trace;
use crate::serveur::erreur::{ErreurApi, MotifConflit};
use crate::serveur::pipeline::{Garde, id_chemin};

/// Cas d'usage du module.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cas {
    /// Membres du cabinet.
    ListerMembres,
    /// Changer le rôle d'un membre.
    ChangerRole,
    /// Suspendre un membre.
    Suspendre,
    /// Rétablir un membre suspendu.
    Retablir,
    /// Affecter un membre à un dossier, ou changer sa délégation de validation.
    Affecter,
    /// Retirer l'affectation d'un membre à un dossier.
    RetirerAffectation,
    /// Sessions actives d'un membre.
    ListerSessions,
    /// Révoquer une session d'un membre.
    RevoquerSession,
    /// Invitations en attente.
    ListerInvitations,
    /// Inviter un utilisateur.
    Inviter,
    /// Révoquer une invitation.
    RevoquerInvitation,
    /// Renvoyer le lien d'une invitation en attente (lien neuf : le précédent est fermé).
    RenvoyerInvitation,
    /// Réinitialisations en attente.
    ListerReinitialisations,
    /// Demander la réinitialisation de l'accès d'un membre.
    Reinitialiser,
    /// Annuler une réinitialisation en attente.
    AnnulerReinitialisation,
    /// Activer la séparation des tâches.
    ActiverSeparation,
    /// Désactiver la séparation des tâches.
    DesactiverSeparation,
    /// Utiliser un lien d'invitation ou de réinitialisation (anonyme, `serveur::liens`).
    UtiliserLien,
    /// Accepter une invitation, connecté (hors cabinet, `serveur::liens`).
    AccepterInvitation,
}

impl Cas {
    /// Vrai pour un cas exécuté hors cabinet, par `serveur::liens`.
    #[must_use]
    pub const fn hors_cabinet(self) -> bool {
        matches!(self, Self::UtiliserLien | Self::AccepterInvitation)
    }
}

/// Tranche du module.
pub const MODULE: Module = Module {
    nom: "administration",
    routes: ROUTES,
    champs_journal: &[],
};

const fn cabinet(methode: Methode, chemin: &'static str, action: Action, cas: Cas) -> Route {
    Route {
        methode,
        chemin,
        action: ActionRoute::Cabinet(action),
        portee: Portee::Cabinet,
        cas_usage: CasUsage::Administration(cas),
    }
}

/// Routes du module.
pub const ROUTES: &[Route] = &[
    cabinet(
        Methode::Get,
        "/api/cabinets/{cabinet}/membres",
        Action::GererCabinet,
        Cas::ListerMembres,
    ),
    cabinet(
        Methode::Post,
        "/api/cabinets/{cabinet}/membres/{membre}/role",
        Action::ModifierDroits,
        Cas::ChangerRole,
    ),
    cabinet(
        Methode::Post,
        "/api/cabinets/{cabinet}/membres/{membre}/suspension",
        Action::ModifierDroits,
        Cas::Suspendre,
    ),
    cabinet(
        Methode::Delete,
        "/api/cabinets/{cabinet}/membres/{membre}/suspension",
        Action::ModifierDroits,
        Cas::Retablir,
    ),
    cabinet(
        Methode::Post,
        "/api/cabinets/{cabinet}/membres/{membre}/acces",
        Action::ModifierDroits,
        Cas::Affecter,
    ),
    cabinet(
        Methode::Delete,
        "/api/cabinets/{cabinet}/membres/{membre}/acces/{acces}",
        Action::ModifierDroits,
        Cas::RetirerAffectation,
    ),
    cabinet(
        Methode::Get,
        "/api/cabinets/{cabinet}/membres/{membre}/sessions",
        Action::GererCabinet,
        Cas::ListerSessions,
    ),
    cabinet(
        Methode::Delete,
        "/api/cabinets/{cabinet}/membres/{membre}/sessions/{session}",
        Action::ModifierDroits,
        Cas::RevoquerSession,
    ),
    cabinet(
        Methode::Get,
        "/api/cabinets/{cabinet}/invitations",
        Action::GererCabinet,
        Cas::ListerInvitations,
    ),
    cabinet(
        Methode::Post,
        "/api/cabinets/{cabinet}/invitations",
        Action::ModifierDroits,
        Cas::Inviter,
    ),
    cabinet(
        Methode::Post,
        "/api/cabinets/{cabinet}/invitations/{invitation}/renvoi",
        Action::ModifierDroits,
        Cas::RenvoyerInvitation,
    ),
    cabinet(
        Methode::Delete,
        "/api/cabinets/{cabinet}/invitations/{invitation}",
        Action::ModifierDroits,
        Cas::RevoquerInvitation,
    ),
    cabinet(
        Methode::Get,
        "/api/cabinets/{cabinet}/reinitialisations",
        Action::GererCabinet,
        Cas::ListerReinitialisations,
    ),
    cabinet(
        Methode::Post,
        "/api/cabinets/{cabinet}/membres/{membre}/reinitialisation",
        Action::ReinitialiserAcces,
        Cas::Reinitialiser,
    ),
    cabinet(
        Methode::Delete,
        "/api/cabinets/{cabinet}/reinitialisations/{reinitialisation}",
        Action::GererCabinet,
        Cas::AnnulerReinitialisation,
    ),
    cabinet(
        Methode::Delete,
        "/api/cabinets/{cabinet}/separation-taches",
        Action::DesactiverSeparationDesTaches,
        Cas::DesactiverSeparation,
    ),
    cabinet(
        Methode::Post,
        "/api/cabinets/{cabinet}/separation-taches",
        Action::ActiverSeparationDesTaches,
        Cas::ActiverSeparation,
    ),
    // Usage d'un lien reçu par courriel : sans session, décidé comme une connexion (action
    // publique, bornée par la limitation de débit ; le lien, à usage unique, est la preuve).
    Route {
        methode: Methode::Post,
        chemin: "/api/liens/utilisation",
        action: ActionRoute::Connexion,
        portee: Portee::Anonyme,
        cas_usage: CasUsage::Administration(Cas::UtiliserLien),
    },
    // Invitation à un autre cabinet, acceptée par un utilisateur connecté (session complète).
    Route {
        methode: Methode::Post,
        chemin: "/api/moi/invitations/acceptation",
        action: ActionRoute::Facteur(ActionFacteur::AccepterInvitation),
        portee: Portee::Utilisateur,
        cas_usage: CasUsage::Administration(Cas::AccepterInvitation),
    },
];

// --- Corps et vues -----------------------------------------------------------------------------

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CorpsRole {
    role: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CorpsAffectation {
    dossier: Uuid,
    #[serde(default)]
    validation_deleguee: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CorpsInvitation {
    courriel: String,
    nom: String,
    role: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CorpsReinitialisation {
    /// Code du motif (liste fermée) : seul inscrit à l'audit.
    motif_code: String,
    /// Texte libre, conservé dans la table seulement (effaçable).
    motif: String,
    verification_hors_bande: bool,
}

#[derive(Serialize)]
struct VueMembre {
    id: String,
    nom: String,
    courriel: String,
    role: String,
    actif: bool,
}

#[derive(Serialize)]
struct VueInvitation {
    id: String,
    courriel: String,
    nom: String,
    role: String,
    cree_le: String,
}

#[derive(Serialize)]
struct VueReinitialisation {
    id: String,
    membre: String,
    demandee_par: Option<String>,
    motif: String,
    cree_le: String,
    echeance: String,
}

#[derive(Serialize)]
struct VueSessionMembre {
    id: String,
    nature: String,
    methode: String,
    cree_le: String,
    derniere_activite: String,
    appareil: Option<String>,
}

fn corps<T: serde::de::DeserializeOwned>(octets: &[u8]) -> Result<T, ErreurApi> {
    serde_json::from_slice(octets).map_err(|_| ErreurApi::RequeteInvalide)
}

fn horodatage(i: time::OffsetDateTime) -> String {
    i.to_offset(time::UtcOffset::UTC)
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_default()
}

fn conflit(motif: MotifConflit) -> ErreurApi {
    ErreurApi::Conflit {
        motif,
        nombre: None,
    }
}

fn role_valide(role: &str) -> Result<&str, ErreurApi> {
    ROLES
        .iter()
        .copied()
        .find(|r| *r == role)
        .ok_or(ErreurApi::RequeteInvalide)
}

/// Défense en profondeur : l'acteur est administrateur **actif**, lu en base et verrouillé avec
/// les autres administrateurs actifs (une rétrogradation croisée ne laisse pas le cabinet sans
/// administrateur). Rend les administrateurs actifs. Sinon, le cabinet est d'autrui (404) ou
/// l'action refusée (403).
async fn administrateurs_verrouilles(
    tx: &mut TransactionTenant,
    garde: &Garde,
) -> Result<Vec<IdUtilisateur>, ErreurApi> {
    let appartenance = lire_appartenance(tx)
        .await?
        .filter(|a| a.actif)
        .ok_or(ErreurApi::Introuvable)?;
    if appartenance.role != authz::Role::Administrateur.nom() {
        return Err(ErreurApi::Interdit);
    }
    let admins = adm::verrouiller_administrateurs_actifs(tx).await?;
    if !admins.contains(&garde.utilisateur()) {
        return Err(ErreurApi::Interdit);
    }
    Ok(admins)
}

fn trace(genre: &'static str, ressource: Uuid) -> Trace {
    Trace::new(PorteeAudit::Cabinet, genre, ressource)
}

fn rendre(statut: StatusCode, corps: impl Serialize, traces: Vec<Trace>) -> Resultat {
    Resultat {
        reponse: (statut, Json(corps)).into_response(),
        traces,
    }
}

async fn membre_du_chemin(
    tx: &mut TransactionTenant,
    requete: &Requete<'_>,
) -> Result<adm::Membre, ErreurApi> {
    let id = id_chemin(requete.chemin, "membre")?;
    adm::lire_membre(tx, IdUtilisateur::depuis_uuid(id))
        .await?
        .ok_or(ErreurApi::Introuvable)
}

// --- Exécution ---------------------------------------------------------------------------------

/// Exécute un cas d'usage du module dans la transaction du pipeline.
pub(crate) async fn executer(
    cas: Cas,
    tx: &mut TransactionTenant,
    garde: &Garde,
    requete: &Requete<'_>,
) -> Result<Resultat, ErreurApi> {
    garde.exiger_cabinet()?;
    match cas {
        Cas::ListerMembres => lister_membres(tx, garde).await,
        Cas::ChangerRole => changer_role(tx, garde, requete).await,
        Cas::Suspendre => changer_activite(tx, garde, requete, false).await,
        Cas::Retablir => changer_activite(tx, garde, requete, true).await,
        Cas::Affecter => affecter(tx, garde, requete).await,
        Cas::RetirerAffectation => retirer_affectation(tx, garde, requete).await,
        Cas::ListerSessions => lister_sessions(tx, garde, requete).await,
        Cas::RevoquerSession => revoquer_session(tx, garde, requete).await,
        Cas::ListerInvitations => lister_invitations(tx, garde).await,
        Cas::Inviter => inviter(tx, garde, requete).await,
        Cas::RenvoyerInvitation => renvoyer_invitation(tx, garde, requete).await,
        Cas::RevoquerInvitation => revoquer_invitation(tx, garde, requete).await,
        Cas::ListerReinitialisations => lister_reinitialisations(tx, garde).await,
        Cas::Reinitialiser => reinitialiser(tx, garde, requete).await,
        Cas::AnnulerReinitialisation => annuler_reinitialisation(tx, garde, requete).await,
        Cas::ActiverSeparation => separation(tx, garde, requete, true).await,
        Cas::DesactiverSeparation => separation(tx, garde, requete, false).await,
        Cas::UtiliserLien | Cas::AccepterInvitation => Err(ErreurApi::Interne(
            crate::serveur::erreur::CategorieErreur::Donnee,
        )),
    }
}

async fn lister_membres(tx: &mut TransactionTenant, garde: &Garde) -> Result<Resultat, ErreurApi> {
    administrateurs_verrouilles(tx, garde).await?;
    let vues: Vec<VueMembre> = adm::lister_membres(tx)
        .await?
        .into_iter()
        .map(|m| VueMembre {
            id: m.utilisateur.to_string(),
            nom: m.nom,
            courriel: m.courriel,
            role: m.role,
            actif: m.actif,
        })
        .collect();
    Ok(Resultat::lecture(Json(vues).into_response()))
}

async fn changer_role(
    tx: &mut TransactionTenant,
    garde: &Garde,
    requete: &Requete<'_>,
) -> Result<Resultat, ErreurApi> {
    let admins = administrateurs_verrouilles(tx, garde).await?;
    let corps: CorpsRole = corps(requete.corps)?;
    let role = role_valide(&corps.role)?;
    let membre = membre_du_chemin(tx, requete).await?;
    if membre.utilisateur == garde.utilisateur() {
        return Err(conflit(MotifConflit::ActionSurSoiMeme));
    }
    if membre.role == role {
        return Err(conflit(MotifConflit::AucunChangement));
    }
    // Le dernier administrateur actif ne se rétrograde pas.
    if membre.role == authz::Role::Administrateur.nom()
        && admins.iter().all(|a| *a == membre.utilisateur)
    {
        return Err(conflit(MotifConflit::DernierAdministrateur));
    }
    adm::changer_role(tx, membre.utilisateur, role).await?;
    // Un changement de droits révoque les sessions du membre (ADR 0012).
    adm::revoquer_sessions_du_membre(
        tx,
        membre.utilisateur,
        requete.maintenant,
        MotifRevocation::ChangementDroits,
    )
    .await?;
    let mut t = trace("membre", membre.utilisateur.uuid());
    t.champs
        .push(("role", Some(membre.role.clone()), Some(role.to_owned())));
    let vue = VueMembre {
        id: membre.utilisateur.to_string(),
        nom: membre.nom,
        courriel: membre.courriel,
        role: role.to_owned(),
        actif: membre.actif,
    };
    Ok(rendre(StatusCode::OK, vue, vec![t]))
}

async fn changer_activite(
    tx: &mut TransactionTenant,
    garde: &Garde,
    requete: &Requete<'_>,
    actif: bool,
) -> Result<Resultat, ErreurApi> {
    let admins = administrateurs_verrouilles(tx, garde).await?;
    let membre = membre_du_chemin(tx, requete).await?;
    if membre.utilisateur == garde.utilisateur() {
        return Err(conflit(MotifConflit::ActionSurSoiMeme));
    }
    if membre.actif == actif {
        return Err(conflit(MotifConflit::AucunChangement));
    }
    if !actif
        && admins.iter().all(|a| *a == membre.utilisateur)
        && admins.contains(&membre.utilisateur)
    {
        return Err(conflit(MotifConflit::DernierAdministrateur));
    }
    adm::changer_activite(tx, membre.utilisateur, actif).await?;
    if !actif {
        adm::revoquer_sessions_du_membre(
            tx,
            membre.utilisateur,
            requete.maintenant,
            MotifRevocation::ChangementDroits,
        )
        .await?;
    }
    let mut t = trace("membre", membre.utilisateur.uuid());
    t.champs.push((
        "actif",
        Some(membre.actif.to_string()),
        Some(actif.to_string()),
    ));
    let vue = VueMembre {
        id: membre.utilisateur.to_string(),
        nom: membre.nom,
        courriel: membre.courriel,
        role: membre.role,
        actif,
    };
    Ok(rendre(StatusCode::OK, vue, vec![t]))
}

async fn affecter(
    tx: &mut TransactionTenant,
    garde: &Garde,
    requete: &Requete<'_>,
) -> Result<Resultat, ErreurApi> {
    administrateurs_verrouilles(tx, garde).await?;
    let corps: CorpsAffectation = corps(requete.corps)?;
    let membre = membre_du_chemin(tx, requete).await?;
    let dossier = IdDossier::depuis_uuid(corps.dossier);
    if corps.dossier.is_nil() || lire_dossier(tx, dossier).await?.is_none() {
        return Err(ErreurApi::Introuvable);
    }
    let avant = adm::affecter(tx, membre.utilisateur, dossier, corps.validation_deleguee).await?;
    if avant == Some(corps.validation_deleguee) {
        return Err(conflit(MotifConflit::AucunChangement));
    }
    let mut t = trace("membre", membre.utilisateur.uuid());
    t.donnees.push(("dossier", dossier.to_string()));
    t.champs.push((
        "validation_deleguee",
        avant.map(|a| a.to_string()),
        Some(corps.validation_deleguee.to_string()),
    ));
    let statut = if avant.is_some() {
        StatusCode::OK
    } else {
        StatusCode::CREATED
    };
    let corps = serde_json::json!({
        "id": membre.utilisateur.to_string(),
        "dossier": dossier.to_string(),
        "validation_deleguee": corps.validation_deleguee,
    });
    Ok(rendre(statut, corps, vec![t]))
}

async fn retirer_affectation(
    tx: &mut TransactionTenant,
    garde: &Garde,
    requete: &Requete<'_>,
) -> Result<Resultat, ErreurApi> {
    administrateurs_verrouilles(tx, garde).await?;
    let membre = membre_du_chemin(tx, requete).await?;
    let dossier = IdDossier::depuis_uuid(id_chemin(requete.chemin, "acces")?);
    let avant = adm::retirer_affectation(tx, membre.utilisateur, dossier)
        .await?
        .ok_or(ErreurApi::Introuvable)?;
    let mut t = trace("membre", membre.utilisateur.uuid());
    t.donnees.push(("dossier", dossier.to_string()));
    t.champs
        .push(("validation_deleguee", Some(avant.to_string()), None));
    Ok(rendre(
        StatusCode::OK,
        serde_json::json!({"id": membre.utilisateur.to_string(), "dossier": dossier.to_string()}),
        vec![t],
    ))
}

async fn lister_sessions(
    tx: &mut TransactionTenant,
    garde: &Garde,
    requete: &Requete<'_>,
) -> Result<Resultat, ErreurApi> {
    administrateurs_verrouilles(tx, garde).await?;
    let membre = membre_du_chemin(tx, requete).await?;
    let vues: Vec<VueSessionMembre> = adm::lister_sessions_du_membre(tx, membre.utilisateur)
        .await?
        .into_iter()
        .map(|s| VueSessionMembre {
            id: s.id.hyphenated().to_string(),
            nature: s.nature,
            methode: s.methode,
            cree_le: horodatage(s.cree_le),
            derniere_activite: horodatage(s.derniere_activite),
            appareil: s.appareil,
        })
        .collect();
    Ok(Resultat::lecture(Json(vues).into_response()))
}

async fn revoquer_session(
    tx: &mut TransactionTenant,
    garde: &Garde,
    requete: &Requete<'_>,
) -> Result<Resultat, ErreurApi> {
    administrateurs_verrouilles(tx, garde).await?;
    let membre = membre_du_chemin(tx, requete).await?;
    let session = id_chemin(requete.chemin, "session")?;
    if !adm::revoquer_session_du_membre(tx, membre.utilisateur, session, requete.maintenant).await?
    {
        return Err(ErreurApi::Introuvable);
    }
    let mut t = trace("session", session);
    t.donnees.push(("membre", membre.utilisateur.to_string()));
    Ok(rendre(
        StatusCode::OK,
        serde_json::json!({"id": session.hyphenated().to_string()}),
        vec![t],
    ))
}

async fn lister_invitations(
    tx: &mut TransactionTenant,
    garde: &Garde,
) -> Result<Resultat, ErreurApi> {
    administrateurs_verrouilles(tx, garde).await?;
    let vues: Vec<VueInvitation> = adm::lister_invitations(tx)
        .await?
        .into_iter()
        .map(|i| VueInvitation {
            id: i.id.hyphenated().to_string(),
            courriel: i.courriel,
            nom: i.nom,
            role: i.role,
            cree_le: horodatage(i.cree_le),
        })
        .collect();
    Ok(Resultat::lecture(Json(vues).into_response()))
}

async fn inviter(
    tx: &mut TransactionTenant,
    garde: &Garde,
    requete: &Requete<'_>,
) -> Result<Resultat, ErreurApi> {
    administrateurs_verrouilles(tx, garde).await?;
    let corps: CorpsInvitation = corps(requete.corps)?;
    let courriel = courriel::adresse(&corps.courriel).ok_or(ErreurApi::RequeteInvalide)?;
    let nom = courriel::nom(&corps.nom).ok_or(ErreurApi::RequeteInvalide)?;
    let role = role_valide(&corps.role)?;
    let id = Uuid::new_v4();
    let issue = adm::creer_invitation(
        tx,
        &NouvelleInvitation {
            id,
            courriel: &courriel,
            nom: &nom,
            role,
            invite_par: Some(garde.utilisateur()),
            instant: requete.maintenant,
        },
    )
    .await?;
    match issue {
        CreationInvitation::Creee => {}
        CreationInvitation::DejaMembre => return Err(conflit(MotifConflit::DejaMembre)),
        CreationInvitation::EnCours => return Err(conflit(MotifConflit::InvitationEnCours)),
    }
    // Le lien est émis et envoyé par la file de tâches, après la validation de cette transaction.
    planifier_courriel(tx, SujetCourriel::Invitation, id, None).await?;
    // Ni courriel ni nom dans l'audit : le rôle proposé suffit.
    let mut t = trace("invitation", id);
    t.champs.push(("role", None, Some(role.to_owned())));
    let vue = VueInvitation {
        id: id.hyphenated().to_string(),
        courriel,
        nom,
        role: role.to_owned(),
        cree_le: horodatage(requete.maintenant),
    };
    Ok(rendre(StatusCode::CREATED, vue, vec![t]))
}

async fn renvoyer_invitation(
    tx: &mut TransactionTenant,
    garde: &Garde,
    requete: &Requete<'_>,
) -> Result<Resultat, ErreurApi> {
    administrateurs_verrouilles(tx, garde).await?;
    let id = id_chemin(requete.chemin, "invitation")?;
    let invitation = adm::lire_invitation(tx, id)
        .await?
        .ok_or(ErreurApi::Introuvable)?;
    if invitation.etat != "en_attente" {
        return Err(conflit(MotifConflit::InvitationNonEnAttente));
    }
    // Un envoi déjà en file (non terminé) vaut renvoi : la demande est idempotente.
    let planifie = planifier_courriel(tx, SujetCourriel::Invitation, id, None).await?;
    let mut t = trace("invitation", id);
    t.donnees.push(("renvoi", "demande".to_owned()));
    Ok(rendre(
        StatusCode::OK,
        serde_json::json!({"id": id.hyphenated().to_string(), "envoi_planifie": planifie}),
        vec![t],
    ))
}

async fn revoquer_invitation(
    tx: &mut TransactionTenant,
    garde: &Garde,
    requete: &Requete<'_>,
) -> Result<Resultat, ErreurApi> {
    administrateurs_verrouilles(tx, garde).await?;
    let id = id_chemin(requete.chemin, "invitation")?;
    let invitation = adm::lire_invitation(tx, id)
        .await?
        .ok_or(ErreurApi::Introuvable)?;
    if !adm::revoquer_invitation(tx, id, requete.maintenant).await? {
        return Err(conflit(MotifConflit::InvitationNonEnAttente));
    }
    crate::base::liens::fermer(
        tx,
        crate::base::liens::NatureLien::Invitation,
        id,
        requete.maintenant,
        "annule",
    )
    .await?;
    let mut t = trace("invitation", id);
    t.champs
        .push(("etat", Some(invitation.etat), Some("revoquee".to_owned())));
    Ok(rendre(
        StatusCode::OK,
        serde_json::json!({"id": id.hyphenated().to_string()}),
        vec![t],
    ))
}

async fn lister_reinitialisations(
    tx: &mut TransactionTenant,
    garde: &Garde,
) -> Result<Resultat, ErreurApi> {
    administrateurs_verrouilles(tx, garde).await?;
    let vues: Vec<VueReinitialisation> = adm::lister_reinitialisations(tx)
        .await?
        .into_iter()
        .map(|r| VueReinitialisation {
            id: r.id.hyphenated().to_string(),
            membre: r.utilisateur.to_string(),
            demandee_par: r.demandee_par.map(|d| d.to_string()),
            motif: r.motif,
            cree_le: horodatage(r.cree_le),
            echeance: horodatage(r.echeance),
        })
        .collect();
    Ok(Resultat::lecture(Json(vues).into_response()))
}

async fn reinitialiser(
    tx: &mut TransactionTenant,
    garde: &Garde,
    requete: &Requete<'_>,
) -> Result<Resultat, ErreurApi> {
    administrateurs_verrouilles(tx, garde).await?;
    let corps: CorpsReinitialisation = corps(requete.corps)?;
    // La vérification hors bande (appel, rencontre) est déclarée : sans elle, pas de demande.
    if !corps.verification_hors_bande {
        return Err(ErreurApi::RequeteInvalide);
    }
    let motif = corps.motif.trim();
    if motif.is_empty() || motif.chars().count() > 500 || motif.chars().any(char::is_control) {
        return Err(ErreurApi::RequeteInvalide);
    }
    let motif_code = adm::MOTIFS_REINITIALISATION
        .iter()
        .copied()
        .find(|c| *c == corps.motif_code)
        .ok_or(ErreurApi::RequeteInvalide)?;
    let membre = membre_du_chemin(tx, requete).await?;
    // Un administrateur ne réinitialise jamais son propre compte (ADR 0012).
    if membre.utilisateur == garde.utilisateur() {
        return Err(conflit(MotifConflit::ActionSurSoiMeme));
    }
    // Facteurs et sessions sont ceux du compte, pas du cabinet : un membre d'un autre cabinet ne
    // se réinitialise pas ici (ADR 0023, §4).
    if adm::appartient_a_d_autres_cabinets(tx, membre.utilisateur).await? {
        return Err(conflit(MotifConflit::MembreDUnAutreCabinet));
    }
    let id = Uuid::new_v4();
    let issue = adm::creer_reinitialisation(
        tx,
        id,
        membre.utilisateur,
        Some(garde.utilisateur()),
        motif_code,
        motif,
        requete.maintenant,
    )
    .await?;
    if issue == CreationReinitialisation::EnCours {
        return Err(conflit(MotifConflit::ReinitialisationEnCours));
    }
    let echeance = requete.maintenant + adm::DELAI_REINITIALISATION;
    // Le lien (1 h) est émis et envoyé au terme du délai de contestation, par la file de tâches.
    planifier_courriel(tx, SujetCourriel::Reinitialisation, id, Some(echeance)).await?;
    let mut t = trace("reinitialisation", id);
    // Le code seul : la chaîne d'audit est définitive, le texte libre reste dans la table.
    t.motif = Some(motif_code.to_owned());
    t.donnees.push(("membre", membre.utilisateur.to_string()));
    t.donnees.push(("echeance", horodatage(echeance)));
    t.donnees
        .push(("verification_hors_bande", "declaree".to_owned()));
    let vue = serde_json::json!({
        "id": id.hyphenated().to_string(),
        "membre": membre.utilisateur.to_string(),
        "echeance": horodatage(echeance),
    });
    Ok(rendre(StatusCode::CREATED, vue, vec![t]))
}

async fn annuler_reinitialisation(
    tx: &mut TransactionTenant,
    garde: &Garde,
    requete: &Requete<'_>,
) -> Result<Resultat, ErreurApi> {
    administrateurs_verrouilles(tx, garde).await?;
    let id = id_chemin(requete.chemin, "reinitialisation")?;
    let r = adm::lire_reinitialisation(tx, id)
        .await?
        .ok_or(ErreurApi::Introuvable)?;
    if !adm::annuler_reinitialisation(tx, id, requete.maintenant).await? {
        return Err(conflit(MotifConflit::ReinitialisationNonEnAttente));
    }
    crate::base::liens::fermer(
        tx,
        crate::base::liens::NatureLien::Reinitialisation,
        id,
        requete.maintenant,
        "annule",
    )
    .await?;
    let mut t = trace("reinitialisation", id);
    t.donnees.push(("membre", r.utilisateur.to_string()));
    t.champs
        .push(("etat", Some(r.etat), Some("annulee".to_owned())));
    Ok(rendre(
        StatusCode::OK,
        serde_json::json!({"id": id.hyphenated().to_string()}),
        vec![t],
    ))
}

async fn separation(
    tx: &mut TransactionTenant,
    garde: &Garde,
    requete: &Requete<'_>,
    active: bool,
) -> Result<Resultat, ErreurApi> {
    administrateurs_verrouilles(tx, garde).await?;
    let avant = adm::changer_separation_taches(tx, active).await?;
    if avant == active {
        return Err(conflit(MotifConflit::AucunChangement));
    }
    let _ = requete;
    let cabinet = adm::cabinet_du_contexte(tx).uuid();
    let mut t = trace("cabinet", cabinet);
    t.champs.push((
        "separation_taches",
        Some(avant.to_string()),
        Some(active.to_string()),
    ));
    Ok(rendre(
        StatusCode::OK,
        serde_json::json!({"separation_taches": active}),
        vec![t],
    ))
}
