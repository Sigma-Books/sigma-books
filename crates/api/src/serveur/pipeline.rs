//! Pipeline d'une requête de l'API (`crates/api/CLAUDE.md` ; ADR 0011 ; L2, « Ordre de
//! traitement d'une requête ») :
//!
//! 1. **authentification** ([`super::Authentification`]) : 401 sans identité ; puis limitation
//!    de débit par utilisateur (429) ;
//! 2. **contexte** : cabinet et dossier sont lus **dans le chemin** (UUID, 400 sinon), jamais dans
//!    une session ni un corps (F1.14). La transaction s'ouvre aussitôt avec ce contexte
//!    ([`ouvrir_transaction`]) : la RLS borne ainsi dès la première lecture tout ce que le
//!    pipeline lit en base, à savoir l'appartenance de l'utilisateur au cabinet du chemin (rôle,
//!    état), le cabinet, le dossier et son parent, et l'affectation (doc du crate `authz`) ;
//! 3. **décision Cedar** ([`Decideur`]), avant toute requête du cas d'usage. Ressource d'autrui
//!    (non-membre du cabinet, dossier d'un autre cabinet, inexistant ou non affecté, refus de
//!    `AccederDossier`) : **404**, même corps qu'une ressource absente. Action refusée sur une
//!    ressource visible : **403** ;
//! 4. **cas d'usage**, dans la même transaction, avec ses propres filtres de requête ;
//! 5. **audit** (PR 7) : l'entrée d'audit d'une action métier sera insérée **ici**, dans la même
//!    transaction, juste avant [`TransactionTenant::confirmer`] (ADR 0014, « Même
//!    transaction ») : si l'audit échoue, l'action est annulée. Aucune route de cette PR ne
//!    modifie de donnée métier : rien n'est encore à auditer.
//!
//! Une seule transaction couvre les étapes 2 à 5 : la décision porte sur l'état exact que le
//! cas d'usage voit (aucun intervalle entre la vérification et l'usage). Tout refus ou toute
//! erreur l'abandonne, donc l'annule.

use std::sync::Arc;
use std::time::Instant;

use authz::{Action, Autorisation, Contexte, Decision, Entites, Ressource, Role, Session};
use axum::extract::{FromRequestParts, RawPathParams, Request};
use axum::response::{IntoResponse, Response};
use time::OffsetDateTime;
use uuid::Uuid;

use super::Etat;
use super::erreur::{CategorieErreur, ErreurApi};
use super::routes::{Portee, Route, executer};
use crate::base::organisation::{
    AccesDossier, Appartenance, lire_acces_dossier, lire_appartenance, lire_cabinet, lire_dossier,
};
use crate::base::{
    ContexteTenant, IdCabinet, IdDossier, IdUtilisateur, TransactionTenant, ouvrir_transaction,
};

/// Point de décision d'autorisation. Le binaire utilise [`Autorisation`] (Cedar) ; l'interface
/// existe pour que les tests vérifient que la requête et la RLS tiennent **même si Cedar se
/// trompe** (décision forcée).
pub trait Decideur: Send + Sync + 'static {
    /// Décide si `principal` peut effectuer `action` sur `ressource` ([`Autorisation::decider`]).
    fn decider(
        &self,
        principal: &authz::Appartenance,
        action: Action,
        ressource: Ressource,
        entites: &Entites,
        contexte: &Contexte,
    ) -> Decision;
}

impl Decideur for Autorisation {
    fn decider(
        &self,
        principal: &authz::Appartenance,
        action: Action,
        ressource: Ressource,
        entites: &Entites,
        contexte: &Contexte,
    ) -> Decision {
        Autorisation::decider(self, principal, action, ressource, entites, contexte)
    }
}

/// Ce que le pipeline a établi avant le cas d'usage : principal, cabinet, session, et le moyen
/// de décider d'autres ressources (filtrage d'une liste).
pub struct Garde {
    decideur: Arc<dyn Decideur>,
    principal: authz::Appartenance,
    cabinet: authz::Cabinet,
    session: Session,
    action: Action,
}

impl Garde {
    /// Décide l'action de la route sur un dossier du cabinet, avec l'affectation lue en base.
    pub(crate) fn autorise_dossier(
        &self,
        dossier: IdDossier,
        acces: Option<&AccesDossier>,
    ) -> Result<bool, ErreurApi> {
        let (ressource, entites, contexte) = self.entites_dossier(dossier, acces)?;
        Ok(self.decider(self.action, ressource, &entites, &contexte))
    }

    fn decider(
        &self,
        action: Action,
        ressource: Ressource,
        entites: &Entites,
        contexte: &Contexte,
    ) -> bool {
        self.decideur
            .decider(&self.principal, action, ressource, entites, contexte)
            == Decision::Autorise
    }

    fn entites_dossier(
        &self,
        dossier: IdDossier,
        acces: Option<&AccesDossier>,
    ) -> Result<(Ressource, Entites, Contexte), ErreurApi> {
        let id = id_authz(dossier.uuid(), authz::IdDossier::new)?;
        let mut entites = Entites::new()
            .avec(self.cabinet.clone())
            .avec(authz::Dossier {
                id,
                cabinet: self.cabinet.id,
            });
        let mut contexte = Contexte::new(self.session);
        if let Some(acces) = acces {
            let id_acces = id_authz(acces.id.uuid(), authz::IdAccesDossier::new)?;
            entites.ajouter(authz::AccesDossier {
                id: id_acces,
                appartenance: self.principal.id,
                dossier: id_authz(acces.dossier.uuid(), authz::IdDossier::new)?,
                delegation_validation: acces.validation_deleguee,
            });
            contexte = contexte.avec_acces(id_acces);
        }
        Ok((Ressource::Dossier(id), entites, contexte))
    }
}

/// Convertit un identifiant lu en base en identifiant de `authz` (qui refuse l'UUID nul).
fn id_authz<T>(
    uuid: Uuid,
    nouveau: fn(Uuid) -> Result<T, authz::ErreurIdentifiant>,
) -> Result<T, ErreurApi> {
    nouveau(uuid).map_err(|_| ErreurApi::Interne(CategorieErreur::Donnee))
}

fn role(texte: &str) -> Result<Role, ErreurApi> {
    Role::TOUS
        .into_iter()
        .find(|r| r.nom() == texte)
        .ok_or(ErreurApi::Interne(CategorieErreur::Donnee))
}

/// Identifiant lu dans le chemin : 400 s'il n'est pas un UUID (sans écho), 404 pour l'UUID nul
/// ou maximal, jamais attribués (même réponse qu'un identifiant inexistant).
fn id_chemin(parametres: &RawPathParams, nom: &str) -> Result<Uuid, ErreurApi> {
    let texte = parametres
        .iter()
        .find_map(|(cle, valeur)| (cle == nom).then_some(valeur))
        .ok_or(ErreurApi::Introuvable)?;
    let uuid = Uuid::try_parse(texte).map_err(|_| ErreurApi::RequeteInvalide)?;
    if uuid.is_nil() || uuid.is_max() {
        return Err(ErreurApi::Introuvable);
    }
    Ok(uuid)
}

/// Traite une requête d'une route de la table.
pub(crate) async fn traiter(etat: Etat, route: &'static Route, requete: Request) -> Response {
    match traiter_requete(&etat, route, requete).await {
        Ok(reponse) => reponse,
        Err(erreur) => erreur.into_response(),
    }
}

async fn traiter_requete(
    etat: &Etat,
    route: &'static Route,
    requete: Request,
) -> Result<Response, ErreurApi> {
    let (mut parts, _corps) = requete.into_parts();

    // 1. Authentification, puis débit par utilisateur.
    let identite = etat
        .authentification
        .authentifier(&parts)
        .await
        .map_err(|_| ErreurApi::Interne(CategorieErreur::Authentification))?
        .ok_or(ErreurApi::NonAuthentifie)?;
    etat.limiteur_utilisateur
        .verifier(identite.utilisateur, Instant::now())
        .map_err(|reessayer_dans| ErreurApi::TropDeRequetes { reessayer_dans })?;
    let utilisateur = id_authz(identite.utilisateur.uuid(), authz::IdUtilisateur::new)?;

    // 2. Contexte : chemin, puis transaction au contexte du chemin.
    let parametres = RawPathParams::from_request_parts(&mut parts, &())
        .await
        .map_err(|_| ErreurApi::RequeteInvalide)?;
    let cabinet = IdCabinet::depuis_uuid(id_chemin(&parametres, "cabinet")?);
    let dossier = match route.portee {
        Portee::Cabinet | Portee::DossiersDuCabinet => None,
        Portee::Dossier => Some(IdDossier::depuis_uuid(id_chemin(&parametres, "dossier")?)),
    };
    let contexte = match dossier {
        None => ContexteTenant::cabinet(cabinet, identite.utilisateur),
        Some(d) => ContexteTenant::dossier(cabinet, d, identite.utilisateur),
    };
    let mut tx = ouvrir_transaction(&etat.pool, contexte).await?;

    let garde = etablir(
        etat,
        route,
        &mut tx,
        identite.utilisateur,
        utilisateur,
        identite.derniere_verification,
    )
    .await?;

    // 3. Décision Cedar sur la ressource du chemin.
    match (route.portee, dossier) {
        (Portee::Cabinet, _) => {
            let entites = Entites::new().avec(garde.cabinet.clone());
            let ressource = Ressource::Cabinet(garde.cabinet.id);
            if !garde.decider(
                route.action,
                ressource,
                &entites,
                &Contexte::new(garde.session),
            ) {
                return Err(ErreurApi::Interdit);
            }
        }
        (Portee::Dossier, Some(dossier)) => {
            // Dossier lu dans le contexte du cabinet du chemin : absent s'il est d'autrui.
            if lire_dossier(&mut tx, dossier).await?.is_none() {
                return Err(ErreurApi::Introuvable);
            }
            let acces = lire_acces_dossier(&mut tx, dossier).await?;
            let (ressource, entites, contexte) = garde.entites_dossier(dossier, acces.as_ref())?;
            // Visibilité d'abord : un dossier non visible est une ressource d'autrui (404).
            if !garde.decider(Action::AccederDossier, ressource, &entites, &contexte) {
                return Err(ErreurApi::Introuvable);
            }
            if !garde.decider(route.action, ressource, &entites, &contexte) {
                return Err(ErreurApi::Interdit);
            }
        }
        // Liste : chaque élément est décidé par le cas d'usage ([`Garde::autorise_dossier`]).
        (Portee::DossiersDuCabinet | Portee::Dossier, _) => {}
    }

    // 4. Cas d'usage, dans la transaction.
    let reponse = executer(route.cas_usage, &mut tx, &garde).await?;

    // 5. Audit (PR 7) : l'entrée d'audit d'une action métier s'insère ici, dans `tx`.
    tx.confirmer().await?;
    Ok(reponse)
}

/// Lit le principal et le cabinet du chemin. Non-membre : 404 (le cabinet est d'autrui).
async fn etablir(
    etat: &Etat,
    route: &'static Route,
    tx: &mut TransactionTenant,
    id_utilisateur: IdUtilisateur,
    utilisateur: authz::IdUtilisateur,
    derniere_verification: OffsetDateTime,
) -> Result<Garde, ErreurApi> {
    let Some(Appartenance {
        id,
        cabinet,
        utilisateur: membre,
        role: texte_role,
        actif,
    }) = lire_appartenance(tx).await?
    else {
        return Err(ErreurApi::Introuvable);
    };
    if membre != id_utilisateur || cabinet != tx.contexte().id_cabinet() {
        return Err(ErreurApi::Interne(CategorieErreur::Donnee));
    }
    let Some(cabinet_lu) = lire_cabinet(tx).await? else {
        return Err(ErreurApi::Introuvable);
    };
    let id_cabinet = id_authz(cabinet_lu.id.uuid(), authz::IdCabinet::new)?;
    Ok(Garde {
        decideur: Arc::clone(&etat.decideur),
        principal: authz::Appartenance {
            id: id_authz(id.uuid(), authz::IdAppartenance::new)?,
            utilisateur,
            cabinet: id_cabinet,
            role: role(&texte_role)?,
            actif,
        },
        cabinet: authz::Cabinet {
            id: id_cabinet,
            separation_des_taches: cabinet_lu.separation_taches,
        },
        session: Session::depuis(derniere_verification, OffsetDateTime::now_utc()),
        action: route.action,
    })
}
