//! Pipeline d'une requête de l'API (`crates/api/CLAUDE.md` ; ADR 0011 ; L2, « Ordre de
//! traitement d'une requête ») :
//!
//! 1. **authentification** ([`super::Authentification`]) : 401 sans identité, ou pour une
//!    session restreinte ou d'enrôlement hors des routes de la session elle-même (ADR 0012) ;
//!    pour une mutation, 403 si l'en-tête anti-CSRF n'est pas le jeton lié à la session ; puis
//!    limitation de débit par utilisateur (429). L'activité de la session n'est notée qu'à la
//!    fin d'une requête admise ([`super::Authentification::noter_activite`]). Les routes hors cabinet (portées
//!    [`Portee::Utilisateur`] et [`Portee::Session`]) passent ensuite au module [`super::moi`] ;
//! 2. **contexte** : cabinet et dossier sont lus **dans le chemin** (UUID, 400 sinon), jamais dans
//!    une session ni un corps (F1.14). La transaction s'ouvre aussitôt avec ce contexte
//!    ([`ouvrir_transaction`]) : la RLS borne ainsi dès la première lecture tout ce que le
//!    pipeline lit en base, à savoir l'appartenance de l'utilisateur au cabinet du chemin (rôle,
//!    état), le cabinet, le dossier et son parent, et l'affectation (doc du crate `authz`) ;
//! 3. **décision Cedar** ([`Decideur`]), avant toute requête du cas d'usage. Ressource d'autrui
//!    (non-membre du cabinet ou membre **suspendu**, dossier d'un autre cabinet, inexistant ou
//!    non affecté, refus de `AccederDossier`) : **404**, même corps qu'une ressource absente.
//!    Action refusée sur une ressource visible : **403** ;
//! 4. **cas d'usage**, dans la même transaction, avec ses propres filtres de requête. Ses vues
//!    ne se construisent que depuis ce que fabrique la [`Garde`] après décision
//!    ([`Garde::filtrer`] pour une liste) : aucun élément ne sort sans décision Cedar ;
//! 5. **audit** : le cas d'usage rend ses traces ([`super::audit::Trace`]) ; le pipeline les
//!    inscrit au journal d'audit chaîné **dans la même transaction**, juste avant
//!    [`TransactionTenant::confirmer`] (ADR 0014, « Même transaction ») : si l'audit échoue,
//!    l'action est annulée. Une mutation sans trace est refusée (erreur interne) : aucune action
//!    sans trace, aucune trace sans action.
//!
//! Le corps de la requête est lu (borné par [`super::Reglages::taille_corps_max`]) après
//! l'authentification et avant l'ouverture de la transaction : un client lent ne retient pas de
//! connexion à la base.
//!
//! Une seule transaction couvre les étapes 2 à 5 : la décision porte sur l'état exact que le
//! cas d'usage voit (aucun intervalle entre la vérification et l'usage). Tout refus ou toute
//! erreur l'abandonne, donc l'annule.

use std::sync::Arc;
use std::time::Instant;

use authz::{
    Action, ActionUtilisateur, Autorisation, Contexte, Decision, Entites, Ressource,
    RessourceUtilisateur, Role, Session, Verdict,
};
use axum::extract::{FromRequestParts, RawPathParams, Request};
use axum::response::{IntoResponse, Response};
use uuid::Uuid;

use super::Etat;
use super::audit::inscrire_traces;
use super::csrf::est_mutation;
use super::erreur::{CategorieErreur, ErreurApi};
use super::routes::{ActionRoute, Portee, Requete, Route, executer};
use crate::base::audit::PorteeAudit;
use crate::base::organisation::{
    AccesDossier, Appartenance, Cabinet, Dossier, DossierVisible, lire_acces_dossier,
    lire_appartenance, lire_cabinet, lire_dossier,
};
use crate::base::{
    ContexteTenant, IdCabinet, IdDossier, IdSession, IdUtilisateur, TransactionTenant,
    ouvrir_transaction,
};

/// Point de décision d'autorisation. Le binaire utilise [`Autorisation`] (Cedar) ; l'interface
/// existe pour que les tests vérifient que la requête et la RLS tiennent **même si Cedar se
/// trompe** (décision forcée).
pub trait Decideur: Send + Sync + 'static {
    /// Décide si `principal` peut effectuer `action` sur `ressource`, avec les politiques
    /// déterminantes ([`Autorisation::verdict`]), inscrites au journal d'audit (ADR 0014).
    fn verdict(
        &self,
        principal: &authz::Appartenance,
        action: Action,
        ressource: Ressource,
        entites: &Entites,
        contexte: &Contexte,
    ) -> Verdict;

    /// Décide si l'utilisateur `principal` peut effectuer `action` sur lui-même (ses sessions,
    /// ADR 0012), ressource lue en base ([`Autorisation::verdict_utilisateur`]).
    fn verdict_utilisateur(
        &self,
        principal: authz::IdUtilisateur,
        action: ActionUtilisateur,
        ressource: RessourceUtilisateur,
    ) -> Verdict;

    /// La décision seule.
    fn decider(
        &self,
        principal: &authz::Appartenance,
        action: Action,
        ressource: Ressource,
        entites: &Entites,
        contexte: &Contexte,
    ) -> Decision {
        self.verdict(principal, action, ressource, entites, contexte)
            .decision
    }
}

impl Decideur for Autorisation {
    fn verdict(
        &self,
        principal: &authz::Appartenance,
        action: Action,
        ressource: Ressource,
        entites: &Entites,
        contexte: &Contexte,
    ) -> Verdict {
        Autorisation::verdict(self, principal, action, ressource, entites, contexte)
    }

    fn verdict_utilisateur(
        &self,
        principal: authz::IdUtilisateur,
        action: ActionUtilisateur,
        ressource: RessourceUtilisateur,
    ) -> Verdict {
        Autorisation::verdict_utilisateur(self, principal, action, ressource)
    }
}

/// Ce que le pipeline a établi avant le cas d'usage : principal, cabinet, session, ressource
/// du chemin décidée, et le moyen de décider d'autres ressources (filtrage d'une liste).
///
/// La `Garde` est **la seule** fabrique de [`DossierAutorise`] et de [`CabinetAutorise`], dont
/// les cas d'usage tirent leurs vues : un dossier ne peut sortir dans une réponse sans être
/// passé par une décision Cedar ([`Garde::filtrer`], ou décision du chemin par le pipeline).
pub struct Garde {
    decideur: Arc<dyn Decideur>,
    principal: authz::Appartenance,
    cabinet: authz::Cabinet,
    session: Session,
    action: Action,
    /// Ressource du chemin, une fois l'action autorisée sur elle.
    decidee: Option<RessourceChemin>,
    /// Utilisateur et session authentifiés, inscrits au journal d'audit.
    utilisateur: IdUtilisateur,
    id_session: IdSession,
    /// Verdict de Cedar sur l'action de la route et la ressource du chemin (politiques
    /// déterminantes, ADR 0014). Absent pour une liste, décidée élément par élément.
    verdict: Option<Verdict>,
}

/// Un dossier dont l'action de la route est autorisée. Champs privés : seule la [`Garde`] le
/// construit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DossierAutorise {
    id: IdDossier,
    nom: String,
}

impl DossierAutorise {
    /// Identifiant.
    #[must_use]
    pub fn id(&self) -> IdDossier {
        self.id
    }

    /// Nom.
    #[must_use]
    pub fn nom(&self) -> &str {
        &self.nom
    }
}

/// Le cabinet du chemin, une fois l'action de la route autorisée sur lui. Champs privés : seule
/// la [`Garde`] le construit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CabinetAutorise(Cabinet);

impl CabinetAutorise {
    /// Le cabinet.
    #[must_use]
    pub fn cabinet(&self) -> &Cabinet {
        &self.0
    }
}

/// Ressource désignée par le chemin d'une route.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RessourceChemin {
    Cabinet,
    DossiersDuCabinet,
    Dossier(IdDossier),
}

/// Ressource du chemin selon la portée de la route. Une incohérence (portée de dossier sans
/// dossier, ou l'inverse) est une erreur interne : rien ne passe sans décision.
fn ressource_chemin(
    portee: Portee,
    dossier: Option<IdDossier>,
) -> Result<RessourceChemin, ErreurApi> {
    match (portee, dossier) {
        (Portee::Cabinet, None) => Ok(RessourceChemin::Cabinet),
        (Portee::DossiersDuCabinet, None) => Ok(RessourceChemin::DossiersDuCabinet),
        (Portee::Dossier, Some(dossier)) => Ok(RessourceChemin::Dossier(dossier)),
        (Portee::Cabinet | Portee::DossiersDuCabinet, Some(_))
        | (Portee::Dossier, None)
        | (Portee::Utilisateur | Portee::Session, _) => {
            Err(ErreurApi::Interne(CategorieErreur::Donnee))
        }
    }
}

impl Garde {
    /// Garde les seuls dossiers sur lesquels l'action de la route est autorisée, chacun décidé
    /// par Cedar avec son affectation lue en base. Seule voie vers une liste de dossiers.
    ///
    /// # Errors
    /// Erreur interne si la route ne porte pas sur les dossiers du cabinet, ou sur une donnée
    /// incohérente.
    pub fn filtrer(
        &self,
        visibles: Vec<DossierVisible>,
    ) -> Result<Vec<DossierAutorise>, ErreurApi> {
        if self.decidee != Some(RessourceChemin::DossiersDuCabinet) {
            return Err(ErreurApi::Interne(CategorieErreur::Donnee));
        }
        let mut autorises = Vec::new();
        for visible in visibles {
            let id = visible.dossier.id;
            let (ressource, entites, contexte) =
                self.entites_dossier(id, visible.acces.as_ref())?;
            if self.decider(self.action, ressource, &entites, &contexte) {
                autorises.push(DossierAutorise {
                    id,
                    nom: visible.dossier.nom,
                });
            }
        }
        Ok(autorises)
    }

    /// Le dossier du chemin, lu par le cas d'usage, sur lequel le pipeline a déjà autorisé
    /// l'action. Tout autre dossier est absent.
    ///
    /// # Errors
    /// [`ErreurApi::Introuvable`] si ce n'est pas le dossier décidé.
    pub fn dossier_du_chemin(&self, dossier: Dossier) -> Result<DossierAutorise, ErreurApi> {
        match self.decidee {
            Some(RessourceChemin::Dossier(id)) if id == dossier.id => Ok(DossierAutorise {
                id,
                nom: dossier.nom,
            }),
            _ => Err(ErreurApi::Introuvable),
        }
    }

    /// Le cabinet du chemin, lu par le cas d'usage, sur lequel le pipeline a déjà autorisé
    /// l'action.
    ///
    /// # Errors
    /// [`ErreurApi::Introuvable`] si l'action n'a pas été décidée sur ce cabinet.
    pub fn cabinet_du_chemin(&self, cabinet: Cabinet) -> Result<CabinetAutorise, ErreurApi> {
        if self.decidee == Some(RessourceChemin::Cabinet)
            && cabinet.id.uuid() == self.cabinet.id.uuid()
        {
            Ok(CabinetAutorise(cabinet))
        } else {
            Err(ErreurApi::Introuvable)
        }
    }

    /// Exige que l'action de la route ait été autorisée sur le cabinet du chemin (cas d'usage
    /// d'une action du cabinet, comme la création d'un dossier).
    ///
    /// # Errors
    /// [`ErreurApi::Introuvable`] sinon.
    pub fn exiger_cabinet(&self) -> Result<(), ErreurApi> {
        if self.decidee == Some(RessourceChemin::Cabinet) {
            Ok(())
        } else {
            Err(ErreurApi::Introuvable)
        }
    }

    /// Portée d'audit de la ressource décidée : le cabinet du chemin ou le dossier du chemin.
    /// Seule voie des cas d'usage de l'audit vers une chaîne.
    ///
    /// # Errors
    /// [`ErreurApi::Introuvable`] si aucune ressource unique n'a été décidée.
    pub fn portee_audit(&self) -> Result<PorteeAudit, ErreurApi> {
        match self.decidee {
            Some(RessourceChemin::Cabinet) => Ok(PorteeAudit::Cabinet),
            Some(RessourceChemin::Dossier(dossier)) => Ok(PorteeAudit::Dossier(dossier)),
            Some(RessourceChemin::DossiersDuCabinet) | None => Err(ErreurApi::Introuvable),
        }
    }

    /// Action de la route.
    #[must_use]
    pub fn action(&self) -> Action {
        self.action
    }

    /// Utilisateur authentifié.
    #[must_use]
    pub fn utilisateur(&self) -> IdUtilisateur {
        self.utilisateur
    }

    /// Identifiant public de la session.
    #[must_use]
    pub fn id_session(&self) -> IdSession {
        self.id_session
    }

    /// Rôle du principal dans le cabinet.
    #[must_use]
    pub fn role(&self) -> Role {
        self.principal.role
    }

    /// Vrai si la session est « fraîche » (ré-authentification, ADR 0012).
    #[must_use]
    pub fn session_fraiche(&self) -> bool {
        self.session.est_fraiche()
    }

    /// Verdict de Cedar sur l'action de la route et la ressource du chemin.
    #[must_use]
    pub fn verdict(&self) -> Option<&Verdict> {
        self.verdict.as_ref()
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

    /// Décide l'action de la route sur la ressource du chemin et garde le verdict.
    fn decider_route(
        &mut self,
        ressource: Ressource,
        entites: &Entites,
        contexte: &Contexte,
    ) -> bool {
        let verdict =
            self.decideur
                .verdict(&self.principal, self.action, ressource, entites, contexte);
        let autorise = verdict.decision == Decision::Autorise;
        self.verdict = Some(verdict);
        autorise
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
pub(crate) fn id_authz<T>(
    uuid: Uuid,
    nouveau: fn(Uuid) -> Result<T, authz::ErreurIdentifiant>,
) -> Result<T, ErreurApi> {
    nouveau(uuid).map_err(|_| ErreurApi::Interne(CategorieErreur::Donnee))
}

pub(crate) fn role(texte: &str) -> Result<Role, ErreurApi> {
    Role::TOUS
        .into_iter()
        .find(|r| r.nom() == texte)
        .ok_or(ErreurApi::Interne(CategorieErreur::Donnee))
}

/// Identifiant lu dans le chemin : 400 s'il n'est pas un UUID (sans écho), 404 pour l'UUID nul
/// ou maximal, jamais attribués (même réponse qu'un identifiant inexistant).
pub(crate) fn id_chemin(parametres: &RawPathParams, nom: &str) -> Result<Uuid, ErreurApi> {
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
    let (mut parts, corps) = requete.into_parts();

    // 1. Authentification, puis débit par utilisateur.
    let identite = etat
        .authentification
        .authentifier(&parts)
        .await
        .map_err(|_| ErreurApi::Interne(CategorieErreur::Authentification))?
        .ok_or(ErreurApi::NonAuthentifie)?;
    // Session restreinte ou d'enrôlement : seulement la session elle-même (ADR 0012, F1.12).
    if !route.portee.admet(identite.nature) {
        return Err(ErreurApi::NonAuthentifie);
    }
    // Mutation : jeton anti-CSRF lié à la session, comparé en temps constant (F1.4).
    if est_mutation(&route.methode.methode()) && !identite.csrf_lie {
        return Err(ErreurApi::Interdit);
    }
    etat.limiteur_utilisateur
        .verifier(identite.utilisateur, Instant::now())
        .map_err(|reessayer_dans| ErreurApi::TropDeRequetes { reessayer_dans })?;
    let utilisateur = id_authz(identite.utilisateur.uuid(), authz::IdUtilisateur::new)?;
    // Corps lu avant d'ouvrir la transaction, borné (413 au-delà).
    let corps = axum::body::to_bytes(corps, etat.reglages.taille_corps_max)
        .await
        .map_err(|_| ErreurApi::CorpsTropGrand)?;

    // Hors cabinet : session courante et ressources propres de l'utilisateur.
    if !route.portee.du_cabinet() {
        let reponse = super::moi::executer(etat, route, &mut parts, &identite).await?;
        etat.authentification.noter_activite(&identite).await;
        return Ok(reponse);
    }

    // 2. Contexte : chemin, puis transaction au contexte du chemin.
    let parametres = RawPathParams::from_request_parts(&mut parts, &())
        .await
        .map_err(|_| ErreurApi::RequeteInvalide)?;
    let cabinet = IdCabinet::depuis_uuid(id_chemin(&parametres, "cabinet")?);
    let dossier = match route.portee {
        Portee::Cabinet | Portee::DossiersDuCabinet | Portee::Utilisateur | Portee::Session => None,
        Portee::Dossier => Some(IdDossier::depuis_uuid(id_chemin(&parametres, "dossier")?)),
    };
    // Sous-ressource du dossier (`{compte}`, `{tiers}`…) : au plus une, UUID validé comme le
    // cabinet et le dossier (400, ou 404 pour l'UUID nul ou maximal).
    let mut autres = parametres
        .iter()
        .filter(|(cle, _)| *cle != "cabinet" && *cle != "dossier")
        .map(|(cle, _)| id_chemin(&parametres, cle));
    let sous_ressource = autres.next().transpose()?;
    if autres.next().is_some() {
        return Err(ErreurApi::Interne(CategorieErreur::Donnee));
    }
    let ressource_chemin = ressource_chemin(route.portee, dossier)?;
    let ActionRoute::Cabinet(action) = route.action else {
        return Err(ErreurApi::Interne(CategorieErreur::Donnee));
    };
    let contexte = match dossier {
        None => ContexteTenant::cabinet(cabinet, identite.utilisateur),
        Some(d) => ContexteTenant::dossier(cabinet, d, identite.utilisateur),
    };
    // Transaction au contexte du tenant : jusqu'à la décision, lectures seules sous RLS.
    let mut tx = ouvrir_transaction(&etat.pool, contexte).await?;

    let mut garde = etablir(etat, action, &mut tx, &identite, utilisateur).await?;

    // 3. Décision Cedar sur la ressource du chemin.
    match ressource_chemin {
        RessourceChemin::Cabinet => {
            let entites = Entites::new().avec(garde.cabinet.clone());
            let ressource = Ressource::Cabinet(garde.cabinet.id);
            let contexte = Contexte::new(garde.session);
            if !garde.decider_route(ressource, &entites, &contexte) {
                return Err(ErreurApi::Interdit);
            }
        }
        RessourceChemin::Dossier(dossier) => {
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
            if !garde.decider_route(ressource, &entites, &contexte) {
                return Err(ErreurApi::Interdit);
            }
        }
        // Liste : chaque élément est décidé par [`Garde::filtrer`], seule voie vers une vue.
        RessourceChemin::DossiersDuCabinet => {}
    }
    garde.decidee = Some(ressource_chemin);

    // 4. Cas d'usage, dans la transaction.
    let requete = Requete {
        parametres: parts.uri.query(),
        corps: &corps,
        sous_ressource,
        maintenant: etat.horloge.maintenant(),
        lecture_audit: &etat.pool_lecture_audit,
    };
    let resultat = executer(route.cas_usage, &mut tx, &garde, &requete).await?;

    // 5. Audit, dans la même transaction : un échec annule l'action. Une mutation sans trace
    // est une erreur du cas d'usage.
    exiger_traces(route, &resultat.traces)?;
    inscrire_traces(&mut tx, &garde, resultat.traces).await?;
    tx.confirmer().await?;
    // Requête admise et traitée : seule une telle requête prolonge la session.
    etat.authentification.noter_activite(&identite).await;
    Ok(resultat.reponse)
}

/// Une mutation doit laisser au moins une trace au journal d'audit (ADR 0014 : aucune action
/// sans trace) ; sinon, erreur interne, et la transaction est annulée.
fn exiger_traces(route: &Route, traces: &[super::audit::Trace]) -> Result<(), ErreurApi> {
    if est_mutation(&route.methode.methode()) && traces.is_empty() {
        Err(ErreurApi::Interne(CategorieErreur::Audit))
    } else {
        Ok(())
    }
}

/// Lit le principal et le cabinet du chemin. Non-membre, ou membre suspendu (ADR 0014) : 404,
/// le cabinet est d'autrui (la politique `invariant-appartenance-active` le refuserait aussi).
async fn etablir(
    etat: &Etat,
    action: Action,
    tx: &mut TransactionTenant,
    identite: &super::Identite,
    utilisateur: authz::IdUtilisateur,
) -> Result<Garde, ErreurApi> {
    let id_utilisateur = identite.utilisateur;
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
    if !actif {
        return Err(ErreurApi::Introuvable);
    }
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
        session: Session::depuis(identite.derniere_verification, etat.horloge.maintenant()),
        action,
        decidee: None,
        utilisateur: id_utilisateur,
        id_session: identite.session,
        verdict: None,
    })
}

#[cfg(test)]
mod tests {
    use time::OffsetDateTime;

    use super::*;

    /// Refuse un seul dossier, autorise le reste.
    struct RefuseUn(authz::IdDossier);

    impl Decideur for RefuseUn {
        fn verdict(
            &self,
            _: &authz::Appartenance,
            _: Action,
            ressource: Ressource,
            _: &Entites,
            _: &Contexte,
        ) -> Verdict {
            Verdict {
                decision: if ressource == Ressource::Dossier(self.0) {
                    Decision::Refuse
                } else {
                    Decision::Autorise
                },
                politiques: Vec::new(),
            }
        }

        fn verdict_utilisateur(
            &self,
            _: authz::IdUtilisateur,
            _: ActionUtilisateur,
            _: RessourceUtilisateur,
        ) -> Verdict {
            Verdict {
                decision: Decision::Refuse,
                politiques: Vec::new(),
            }
        }
    }

    fn garde(decideur: Arc<dyn Decideur>, decidee: Option<RessourceChemin>) -> Garde {
        let cabinet = authz::IdCabinet::new(Uuid::new_v4()).unwrap();
        Garde {
            decideur,
            principal: authz::Appartenance {
                id: authz::IdAppartenance::new(Uuid::new_v4()).unwrap(),
                utilisateur: authz::IdUtilisateur::new(Uuid::new_v4()).unwrap(),
                cabinet,
                role: Role::Collaborateur,
                actif: true,
            },
            cabinet: authz::Cabinet {
                id: cabinet,
                separation_des_taches: true,
            },
            session: Session::depuis(OffsetDateTime::UNIX_EPOCH, OffsetDateTime::UNIX_EPOCH),
            action: Action::AccederDossier,
            decidee,
            utilisateur: IdUtilisateur::nouveau(),
            id_session: IdSession::nouveau(),
            verdict: None,
        }
    }

    fn visible(id: IdDossier) -> DossierVisible {
        DossierVisible {
            dossier: Dossier {
                id,
                cabinet: IdCabinet::nouveau(),
                nom: "nom".to_owned(),
                cree_le: OffsetDateTime::UNIX_EPOCH,
            },
            acces: None,
        }
    }

    #[test]
    fn filtrer_ne_garde_que_les_dossiers_autorises_par_la_decision() {
        let (refuse, admis) = (IdDossier::nouveau(), IdDossier::nouveau());
        let decideur = Arc::new(RefuseUn(authz::IdDossier::new(refuse.uuid()).unwrap()));
        let g = garde(decideur, Some(RessourceChemin::DossiersDuCabinet));
        let autorises = g.filtrer(vec![visible(refuse), visible(admis)]).unwrap();
        assert_eq!(
            autorises
                .iter()
                .map(DossierAutorise::id)
                .collect::<Vec<_>>(),
            vec![admis]
        );
    }

    #[test]
    fn la_garde_ne_fabrique_rien_hors_de_la_ressource_decidee() {
        let decideur: Arc<dyn Decideur> =
            Arc::new(RefuseUn(authz::IdDossier::new(Uuid::new_v4()).unwrap()));
        let d = IdDossier::nouveau();
        // Liste demandée alors que la route ne porte pas sur les dossiers du cabinet.
        for decidee in [
            None,
            Some(RessourceChemin::Cabinet),
            Some(RessourceChemin::Dossier(d)),
        ] {
            assert_eq!(
                garde(Arc::clone(&decideur), decidee).filtrer(vec![visible(d)]),
                Err(ErreurApi::Interne(CategorieErreur::Donnee))
            );
        }
        // Dossier du chemin : seulement celui qui a été décidé.
        let g = garde(Arc::clone(&decideur), Some(RessourceChemin::Dossier(d)));
        assert!(g.dossier_du_chemin(visible(d).dossier).is_ok());
        assert_eq!(
            g.dossier_du_chemin(visible(IdDossier::nouveau()).dossier),
            Err(ErreurApi::Introuvable)
        );
        let g = garde(decideur, None);
        assert_eq!(
            g.dossier_du_chemin(visible(d).dossier),
            Err(ErreurApi::Introuvable)
        );
    }

    #[test]
    fn une_mutation_sans_trace_est_refusee() {
        use super::super::audit::Trace;
        use super::super::routes::{Methode, ROUTES};
        let trace = Trace::new(PorteeAudit::Cabinet, "cabinet", Uuid::new_v4());
        let mutations: Vec<_> = ROUTES
            .iter()
            .filter(|r| r.methode == Methode::Post && r.portee.du_cabinet())
            .collect();
        assert_eq!(mutations.len(), 10);
        for route in mutations {
            assert_eq!(
                exiger_traces(route, &[]),
                Err(ErreurApi::Interne(CategorieErreur::Audit)),
                "{}",
                route.chemin
            );
            assert_eq!(exiger_traces(route, std::slice::from_ref(&trace)), Ok(()));
        }
        // Une lecture n'en exige pas.
        for route in ROUTES.iter().filter(|r| r.methode == Methode::Get) {
            assert_eq!(exiger_traces(route, &[]), Ok(()));
        }
    }

    #[test]
    fn une_portee_incoherente_est_une_erreur_interne() {
        let interne = Err(ErreurApi::Interne(CategorieErreur::Donnee));
        assert_eq!(ressource_chemin(Portee::Dossier, None), interne);
        assert_eq!(
            ressource_chemin(Portee::Cabinet, Some(IdDossier::nouveau())),
            interne
        );
        assert_eq!(
            ressource_chemin(Portee::DossiersDuCabinet, Some(IdDossier::nouveau())),
            interne
        );
        assert_eq!(
            ressource_chemin(Portee::Cabinet, None),
            Ok(RessourceChemin::Cabinet)
        );
    }
}
