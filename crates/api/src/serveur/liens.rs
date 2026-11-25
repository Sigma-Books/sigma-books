//! Usage des liens d'invitation et de réinitialisation (PR 9a ; ADR 0012, 0023).
//!
//! - **Hors session** ([`utiliser_lien`], `POST /api/liens/utilisation`) : le porteur présente le
//!   lien reçu par courriel. Invitation : le compte est créé avec l'appartenance. Réinitialisation :
//!   les facteurs et sessions du membre sont retirés. Dans les deux cas, une session
//!   d'**enrôlement** s'ouvre (nature `enrolement`, méthode `lien_enrolement`) : elle ne permet
//!   que d'enrôler une passkey, sans accès aux dossiers (Cedar, `invariant-lien-enrolement`).
//! - **Connecté** ([`accepter_invitation`], `POST /api/moi/invitations/acceptation`) : un
//!   utilisateur qui a déjà un compte accepte l'invitation à un autre cabinet ; le courriel de
//!   l'invitation doit être celui de son compte.
//!
//! Un lien inconnu, expiré, fermé, d'une cible qui n'est plus en attente, ou d'un autre courriel :
//! **la même réponse** (401 hors session, comme un échec de connexion, après la même durée plancher ; 422 `lien_invalide`
//! connecté).
//! Le lien n'est jamais journalisé ni rendu ; il est haché avant toute lecture de la base.

use axum::Json;
use axum::response::{IntoResponse, Response};
use integrite::audit::{Acteur, Evenement, Ressource, Symbole, Texte, Uuid as UuidAudit};
use serde::Deserialize;
use zeroize::Zeroizing;

use super::Etat;
use super::erreur::{ErreurApi, MotifConflit, MotifRefus};
use super::facteurs::{Client, Soi, corps_json, ouvrir};
use crate::base::ErreurBase;
use crate::base::IdUtilisateur;
use crate::base::facteurs::inscrire_dans_les_cabinets;
use crate::base::liens::{ErreurLien, JetonLien, NatureLien, Utilisation, ouvrir_transaction_lien};
use crate::base::session::{MethodeSession, NatureSession};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CorpsLien {
    jeton: String,
}

/// Refus d'un lien : hors session, 401 comme un échec de connexion (le lien est un facteur) ;
/// connecté, 422 `lien_invalide` (un 401 y ferait croire que la session a expiré).
fn invalide(connecte: bool) -> ErreurApi {
    if connecte {
        ErreurApi::Refus(MotifRefus::LienInvalide)
    } else {
        ErreurApi::NonAuthentifie
    }
}

fn lien_presente(corps: &[u8], connecte: bool) -> Result<JetonLien, ErreurApi> {
    let corps: CorpsLien = corps_json(corps, invalide(connecte))?;
    let jeton = Zeroizing::new(corps.jeton);
    JetonLien::depuis_texte(&jeton).ok_or_else(|| invalide(connecte))
}

fn erreur_lien(e: ErreurLien, connecte: bool) -> ErreurApi {
    match e {
        ErreurLien::Invalide => invalide(connecte),
        ErreurLien::CompteExistant => ErreurApi::Conflit {
            motif: MotifConflit::CompteExistant,
            nombre: None,
        },
        ErreurLien::Base(e) => e.into(),
    }
}

fn evenement_lien(
    nature: NatureLien,
    cible: uuid::Uuid,
    utilisateur: IdUtilisateur,
) -> Result<Evenement, ErreurBase> {
    let invalide = |_| ErreurBase::Audit("événement d'usage d'un lien");
    let (action, genre) = match nature {
        NatureLien::Invitation => ("UtiliserLienInvitation", "invitation"),
        NatureLien::Reinitialisation => ("UtiliserLienReinitialisation", "reinitialisation"),
    };
    Evenement::new(
        Acteur::Systeme,
        Symbole::parse(action).map_err(invalide)?,
        Ressource {
            genre: Symbole::parse(genre).map_err(invalide)?,
            id: UuidAudit::depuis_octets(*cible.as_bytes()),
        },
    )
    .avec_donnee(
        Symbole::parse("utilisateur").map_err(invalide)?,
        Texte::parse(&utilisateur.to_string()).map_err(invalide)?,
    )
    .map_err(invalide)
}

/// Usage d'un lien hors session : voir la documentation du module.
pub(super) async fn utiliser_lien(
    etat: &Etat,
    client: &Client,
    corps: &[u8],
) -> Result<Response, ErreurApi> {
    let jeton = lien_presente(corps, false)?;
    let maintenant = etat.horloge.maintenant();
    let mut tx = ouvrir_transaction_lien(&etat.pool, &jeton.empreinte()).await?;
    let Some(lien) = tx.lire(maintenant).await? else {
        tx.confirmer().await?;
        return Err(invalide(false));
    };
    let Utilisation {
        tx,
        utilisateur,
        nature,
        cible,
        ..
    } = tx
        .utiliser(lien, maintenant, IdUtilisateur::nouveau())
        .await
        .map_err(|e| erreur_lien(e, false))?;
    // L'usage est inscrit à l'audit de chaque cabinet actif du compte (ADR 0017 §2), dans la
    // même transaction que la session d'enrôlement.
    let (tx, _) = inscrire_dans_les_cabinets(tx, maintenant, None, |_| {
        evenement_lien(nature, cible, utilisateur)
    })
    .await?;
    ouvrir(
        etat,
        tx,
        utilisateur,
        NatureSession::Enrolement,
        MethodeSession::LienEnrolement,
        maintenant,
        client,
    )
    .await
}

/// Acceptation, par un utilisateur connecté, d'une invitation à un autre cabinet.
pub(super) async fn accepter_invitation(
    soi: &Soi<'_>,
    corps: &[u8],
) -> Result<Response, ErreurApi> {
    soi.limiter()?;
    let jeton = lien_presente(corps, true)?;
    // Décision de Cedar sur l'état exact de la session, lue sous RLS.
    let mut lecture = soi.transaction().await?;
    let (session, verdict) = soi.decider(&mut lecture).await?;
    lecture.confirmer().await?;
    let mut tx = ouvrir_transaction_lien(&soi.etat.pool, &jeton.empreinte()).await?;
    let Some(lien) = tx.lire(soi.maintenant).await? else {
        tx.confirmer().await?;
        return Err(invalide(true));
    };
    let utilisation = tx
        .utiliser_connecte(lien, soi.maintenant, soi.utilisateur())
        .await
        .map_err(|e| erreur_lien(e, true))?;
    let cabinet = utilisation.cabinet;
    let role = utilisation.role.clone().unwrap_or_default();
    // Audit de chaque cabinet actif, nouveau cabinet compris, puis validation.
    soi.auditer_et_confirmer(utilisation.tx, &verdict, &session, session.methode)
        .await?;
    Ok(Json(serde_json::json!({"cabinet": cabinet.to_string(), "role": role})).into_response())
}
