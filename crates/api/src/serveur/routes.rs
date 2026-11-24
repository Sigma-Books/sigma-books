//! Table déclarative des routes de l'API (L2, « Tests exigés », « chaque route »).
//!
//! Le routeur se construit **uniquement** à partir de [`ROUTES`] ([`super::routeur`]) : une route
//! absente de la table n'existe pas. Le test d'isolation « chaque route »
//! (`crates/api/tests/isolation/`) parcourt cette table et échoue si une route n'a pas son cas
//! d'isolation.
//!
//! # Une tranche par module
//! La table n'est écrite nulle part d'un seul tenant : chaque module de routes (sous-modules de
//! celui-ci) déclare sa **tranche** ([`Module`] : ses routes, ses cas d'usage, ses champs de
//! journal), et [`ROUTES`] est l'assemblage, à la compilation, des tranches de [`MODULES`], dans
//! leur ordre. Une voie de travail n'édite que le fichier de son module
//! (`crates/api/README.md`, « Travailler en voies parallèles ») ; les modules prévus par le plan
//! du lot existent déjà, avec une tranche vide.
//!
//! Le cas d'usage d'une route ([`CasUsage`]) nomme son module : le test
//! `chaque_route_releve_de_son_module` refuse une route déclarée dans la tranche d'un autre.

pub mod administration;
pub mod audit;
pub mod brouillard;
pub mod editions;
pub mod facteurs;
pub mod integrite;
pub mod moi;
pub mod organisation;
pub mod passkeys;
pub mod pieces;
pub mod referentiel;
pub mod session;

use authz::{Action, ActionFacteur, ActionUtilisateur};
use axum::extract::RawPathParams;
use axum::http::Method;
use axum::response::Response;
use axum::routing::MethodFilter;
use sqlx::PgPool;

use super::audit::Trace;
use super::authentification::NatureSession;
use super::erreur::{CategorieErreur, ErreurApi};
use super::pipeline::Garde;
use crate::base::TransactionTenant;

/// Méthode HTTP d'une route.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Methode {
    /// Lecture.
    Get,
    /// Mutation : anti-CSRF, et inscription au journal d'audit exigée sur un cabinet.
    Post,
    /// Mutation (suppression ou révocation) : anti-CSRF.
    Delete,
}

impl Methode {
    /// Filtre du routeur.
    #[must_use]
    pub const fn filtre(self) -> MethodFilter {
        match self {
            Self::Get => MethodFilter::GET,
            Self::Post => MethodFilter::POST,
            Self::Delete => MethodFilter::DELETE,
        }
    }

    /// Nom HTTP.
    #[must_use]
    pub const fn nom(self) -> &'static str {
        match self {
            Self::Get => "GET",
            Self::Post => "POST",
            Self::Delete => "DELETE",
        }
    }

    /// Méthode HTTP.
    #[must_use]
    pub fn methode(self) -> Method {
        match self {
            Self::Get => Method::GET,
            Self::Post => Method::POST,
            Self::Delete => Method::DELETE,
        }
    }
}

/// Ressource que désigne le chemin, et donc ce que le pipeline vérifie avant le cas d'usage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Portee {
    /// Le cabinet du chemin (`{cabinet}`) : l'action est décidée sur le cabinet.
    Cabinet,
    /// Les dossiers du cabinet du chemin : chaque dossier est décidé un à un par le cas d'usage.
    DossiersDuCabinet,
    /// Un dossier (`{cabinet}`, `{dossier}`) : visibilité (`AccederDossier`), puis l'action.
    Dossier,
    /// Un brouillon d'un dossier (`{cabinet}`, `{dossier}`, `{brouillon}`) : visibilité du
    /// dossier (`AccederDossier`), puis l'action décidée sur l'**écriture**, avec son créateur
    /// et ses contributeurs lus en base (séparation des tâches, spec §20). Brouillon absent du
    /// dossier : 404.
    Brouillon,
    /// Une écriture validée d'un dossier (`{cabinet}`, `{dossier}`, `{ecriture}` ; PR 13c) :
    /// visibilité du dossier (`AccederDossier`), puis l'action décidée sur l'**écriture
    /// validée**, lue en base sous RLS. Écriture absente du dossier : 404.
    EcritureValidee,
    /// Ressources propres de l'utilisateur authentifié, hors de tout cabinet (ses sessions, ses
    /// cabinets) : transaction au contexte du seul utilisateur ; session complète exigée.
    Utilisateur,
    /// La session courante elle-même (état, déconnexion) : toute nature de session admise
    /// (une session restreinte doit pouvoir lire son état et se fermer).
    Session,
    /// Enrôlement d'un facteur (PR 8b) : toute nature de session admise par le pipeline, car
    /// la session restreinte d'un code de récupération ne sert qu'à cela (F1.12). Cedar décide
    /// selon la nature et la méthode de la session (`adr0012-enrolement-secours`).
    Facteurs,
    /// Connexion, **sans session** (PR 8b) : aucune authentification ; Cedar décide
    /// `SeConnecter` avec le principal anonyme, puis l'ouverture de la session après la
    /// vérification du facteur. Limitation de débit propre (adresse, courriel).
    Anonyme,
}

impl Portee {
    /// Vrai pour une portée d'un cabinet (`{cabinet}` dans le chemin) : pipeline complet,
    /// décision Cedar sur la ressource du chemin, audit des mutations.
    #[must_use]
    pub const fn du_cabinet(self) -> bool {
        matches!(
            self,
            Self::Cabinet
                | Self::DossiersDuCabinet
                | Self::Dossier
                | Self::Brouillon
                | Self::EcritureValidee
        )
    }

    /// Vrai si une session de cette nature est admise : seule une session complète ouvre les
    /// routes métier et les ressources de l'utilisateur (ADR 0012, F1.12). Une route de
    /// connexion n'admet aucune session : elle ne passe pas par l'authentification.
    #[must_use]
    pub const fn admet(self, nature: NatureSession) -> bool {
        match self {
            Self::Anonyme => false,
            Self::Session | Self::Facteurs => true,
            Self::Cabinet
            | Self::DossiersDuCabinet
            | Self::Dossier
            | Self::Brouillon
            | Self::EcritureValidee
            | Self::Utilisateur => {
                matches!(nature, NatureSession::Complete)
            }
        }
    }
}

/// Cas d'usage d'une route : celui de son module. Une variante par module, déclarée d'avance
/// pour les modules prévus (leur énumération est vide tant qu'ils n'ont pas de route).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CasUsage {
    /// Cabinet et dossiers ([`organisation`]).
    Organisation(organisation::Cas),
    /// Journal d'audit ([`audit`]).
    Audit(audit::Cas),
    /// Session courante ([`session`]).
    Session(session::Cas),
    /// Ressources propres de l'utilisateur ([`moi`]).
    Moi(moi::Cas),
    /// Facteurs de secours (PR 8b, [`facteurs`]).
    Facteurs(facteurs::Cas),
    /// Passkeys (PR 8c, [`passkeys`]).
    Passkeys(passkeys::Cas),
    /// Administration du cabinet (PR 9, [`administration`]).
    Administration(administration::Cas),
    /// Dossier et référentiel (PR 11, [`referentiel`]).
    Referentiel(referentiel::Cas),
    /// Pièces justificatives (PR 12, [`pieces`]).
    Pieces(pieces::Cas),
    /// Brouillard et validation (PR 13, [`brouillard`]).
    Brouillard(brouillard::Cas),
    /// Éditions et alertes (PR 14, [`editions`]).
    Editions(editions::Cas),
    /// Horodatage et intégrité (PR 15, [`integrite`]).
    Integrite(integrite::Cas),
}

impl CasUsage {
    /// Vrai pour un cas d'usage de l'authentification d'un utilisateur (facteurs de secours,
    /// passkeys), exécuté hors cabinet par `serveur::facteurs`.
    #[must_use]
    pub const fn est_facteur(self) -> bool {
        matches!(self, Self::Facteurs(_) | Self::Passkeys(_))
    }

    /// Nom du module du cas d'usage ([`Module::nom`]).
    #[must_use]
    pub const fn module(self) -> &'static str {
        match self {
            Self::Organisation(_) => organisation::MODULE.nom,
            Self::Audit(_) => audit::MODULE.nom,
            Self::Session(_) => session::MODULE.nom,
            Self::Moi(_) => moi::MODULE.nom,
            Self::Facteurs(_) => facteurs::MODULE.nom,
            Self::Passkeys(_) => passkeys::MODULE.nom,
            Self::Administration(_) => administration::MODULE.nom,
            Self::Referentiel(_) => referentiel::MODULE.nom,
            Self::Pieces(_) => pieces::MODULE.nom,
            Self::Brouillard(_) => brouillard::MODULE.nom,
            Self::Editions(_) => editions::MODULE.nom,
            Self::Integrite(_) => integrite::MODULE.nom,
        }
    }
}

/// Action Cedar d'une route : toute route est décidée par Cedar (règle absolue 7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionRoute {
    /// Action de cabinet : principal `Appartenance` (lue d'après le cabinet du chemin, ou,
    /// pour la liste de ses cabinets, chacune de ses appartenances).
    Cabinet(Action),
    /// Action de l'utilisateur sur lui-même (ses sessions, ADR 0012) : principal
    /// `Utilisateur`, ressource lue en base sous RLS (titulaire de la session).
    Utilisateur(ActionUtilisateur),
    /// Action de l'utilisateur sur ses facteurs (ADR 0012, PR 8b) : principal `Utilisateur`,
    /// ressource lui-même, contexte de la session lue en base (nature, méthode, fraîcheur).
    Facteur(ActionFacteur),
    /// Connexion (PR 8b) : `SeConnecter`, principal `Anonyme`, ressource l'instance ; puis
    /// `OuvrirSession` (principal Utilisateur) après la vérification du facteur.
    Connexion,
}

/// Une route : méthode, modèle de chemin, action Cedar, portée et cas d'usage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Route {
    /// Méthode HTTP.
    pub methode: Methode,
    /// Modèle de chemin (syntaxe d'`axum`), cabinet et dossier explicites (ADR 0011).
    pub chemin: &'static str,
    /// Action Cedar décidée, et donc son principal.
    pub action: ActionRoute,
    /// Ressource du chemin.
    pub portee: Portee,
    /// Cas d'usage exécuté dans la transaction.
    pub cas_usage: CasUsage,
}

/// Tranche d'un module de routes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Module {
    /// Nom du module (celui de son fichier).
    pub nom: &'static str,
    /// Ses routes, dans l'ordre de la table.
    pub routes: &'static [Route],
    /// Champs de journal technique propres au module, en plus du socle
    /// ([`super::journal::CHAMPS`]).
    pub champs_journal: &'static [&'static str],
}

/// Les modules de routes, dans l'ordre de la table. Les modules prévus y figurent déjà : une
/// voie n'a pas à modifier cette liste.
pub const MODULES: &[Module] = &[
    organisation::MODULE,
    audit::MODULE,
    session::MODULE,
    moi::MODULE,
    // Voie authentification (PR 8b, 8c, 9).
    facteurs::MODULE,
    passkeys::MODULE,
    administration::MODULE,
    // Voie comptabilité (PR 11, 13, 14).
    referentiel::MODULE,
    brouillard::MODULE,
    editions::MODULE,
    // Voie pièces (PR 12).
    pieces::MODULE,
    // Voie intégrité et horodatage (PR 15).
    integrite::MODULE,
];

/// Toutes les routes de l'API : les tranches de [`MODULES`], bout à bout.
pub const ROUTES: &[Route] = &TABLE;

const NOMBRE_DE_ROUTES: usize = nombre_de_routes(MODULES);
const TABLE: [Route; NOMBRE_DE_ROUTES] = assembler(MODULES);

const fn nombre_de_routes(modules: &[Module]) -> usize {
    let mut n = 0;
    let mut i = 0;
    while i < modules.len() {
        n += modules[i].routes.len();
        i += 1;
    }
    n
}

/// Concatène les tranches à la compilation. Chaque case de la table est réécrite par une route
/// d'un module : la valeur initiale n'en sort jamais (test `la_table_est_l_assemblage_des_tranches`).
const fn assembler<const N: usize>(modules: &[Module]) -> [Route; N] {
    let mut table = [organisation::ROUTES[0]; N];
    let mut n = 0;
    let mut i = 0;
    while i < modules.len() {
        let mut j = 0;
        while j < modules[i].routes.len() {
            table[n] = modules[i].routes[j];
            n += 1;
            j += 1;
        }
        i += 1;
    }
    table
}

/// Ce que le cas d'usage reçoit de la requête, en plus de la transaction et de la garde.
#[derive(Debug, Clone, Copy)]
pub struct Requete<'a> {
    /// Paramètres de l'URL (`apres=…&limite=…`), non décodés.
    pub parametres: Option<&'a str>,
    /// Corps, déjà borné.
    pub corps: &'a [u8],
    /// Instant de la requête, lu sur l'horloge de l'application (injectable en test) : date du
    /// jour des règles d'ouverture d'un dossier.
    pub maintenant: time::OffsetDateTime,
    /// Horloge de l'application : un cas d'usage qui a attendu un verrou relit l'instant
    /// (validation d'une écriture, PR 13b) au lieu de se fier à `maintenant`.
    pub horloge: &'a dyn super::session::Horloge,
    /// Pool du rôle de lecture d'audit.
    pub lecture_audit: &'a PgPool,
    /// Pool des lectures à instantané unique d'un cas d'usage
    /// ([`crate::base::ouvrir_transaction_lecture`]), **distinct** de celui de la transaction du
    /// pipeline (sinon deux connexions du même pool par requête : interblocage sous charge, voir
    /// `Application`).
    pub pool: &'a PgPool,
    /// Nombre maximal d'écritures lues pour une édition (réglage `editions_ecritures_max`).
    pub editions_max: i64,
    /// Paramètres du chemin, dont les sous-ressources (`{compte}`, `{tiers}`, `{piece}`…), à
    /// lire par `id_chemin`.
    pub chemin: &'a RawPathParams,
    /// Service des pièces, s'il est branché (voie pièces, PR 12).
    pub pieces: Option<&'a crate::pieces::Pieces>,
}

/// Résultat d'un cas d'usage : la réponse, et les traces à inscrire au journal d'audit avant
/// la validation de la transaction.
#[derive(Debug)]
pub struct Resultat {
    /// Réponse au client, envoyée seulement si la transaction est validée.
    pub reponse: Response,
    /// Traces de l'action ; vide pour une lecture.
    pub traces: Vec<Trace>,
}

impl Resultat {
    pub(crate) fn lecture(reponse: Response) -> Self {
        Self {
            reponse,
            traces: Vec::new(),
        }
    }
}

/// Exécute un cas d'usage d'un cabinet dans la transaction du pipeline : chaque module exécute
/// les siens.
pub(crate) async fn executer(
    cas: CasUsage,
    tx: &mut TransactionTenant,
    garde: &Garde,
    requete: &Requete<'_>,
) -> Result<Resultat, ErreurApi> {
    match cas {
        CasUsage::Organisation(cas) => organisation::executer(cas, tx, garde, requete).await,
        CasUsage::Audit(cas) => audit::executer(cas, tx, garde, requete).await,
        // Cas d'usage hors cabinet : modules `moi` et `facteurs` du serveur, jamais avec une
        // garde de cabinet.
        CasUsage::Session(_) | CasUsage::Moi(_) | CasUsage::Facteurs(_) | CasUsage::Passkeys(_) => {
            Err(ErreurApi::Interne(CategorieErreur::Donnee))
        }
        // Voie authentification (PR 9).
        CasUsage::Administration(cas) => match cas {},
        // Voie comptabilité (PR 11, 13, 14).
        CasUsage::Referentiel(cas) => referentiel::executer(cas, tx, garde, requete).await,
        CasUsage::Brouillard(cas) => brouillard::executer(cas, tx, garde, requete).await,
        CasUsage::Editions(cas) => editions::executer(cas, tx, garde, requete).await,
        // Voie pièces (PR 12).
        CasUsage::Pieces(cas) => pieces::executer(cas, tx, garde, requete).await,
        // Voie intégrité et horodatage (PR 15).
        CasUsage::Integrite(cas) => match cas {},
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    #[test]
    fn chaque_mutation_porte_sur_une_ressource_unique() {
        // Une mutation est décidée sur une ressource du chemin : sa trace a une portée. Que
        // chaque mutation produise bien des traces sur cette portée est vérifié contre la base
        // (`tests/audit.rs`, `chaque_mutation_produit_des_traces_sur_sa_portee`) ; le pipeline
        // refuse une mutation sans trace (`pipeline::exiger_traces`).
        for route in ROUTES
            .iter()
            .filter(|r| r.methode != Methode::Get && r.portee.du_cabinet())
        {
            assert_ne!(route.portee, Portee::DossiersDuCabinet, "{}", route.chemin);
        }
    }

    #[test]
    fn portees_actions_et_natures_coherentes() {
        for route in ROUTES {
            // Toute route est décidée par Cedar : principal Appartenance pour un cabinet et
            // pour la liste de ses cabinets (une décision par appartenance), Utilisateur pour
            // ses sessions.
            let de_cabinet = route.portee.du_cabinet()
                || route.cas_usage == CasUsage::Moi(moi::Cas::ListerCabinets);
            assert_eq!(
                matches!(route.action, ActionRoute::Cabinet(_)),
                de_cabinet,
                "{}",
                route.chemin
            );
            assert_eq!(
                route.portee.du_cabinet(),
                route.chemin.contains("{cabinet}"),
                "{}",
                route.chemin
            );
        }
        for nature in [NatureSession::Restreinte, NatureSession::Enrolement] {
            for portee in [
                Portee::Cabinet,
                Portee::DossiersDuCabinet,
                Portee::Dossier,
                Portee::Brouillon,
                Portee::EcritureValidee,
                Portee::Utilisateur,
                Portee::Anonyme,
            ] {
                assert!(!portee.admet(nature));
            }
            assert!(Portee::Session.admet(nature));
            assert!(Portee::Facteurs.admet(nature));
        }
        assert!(!Portee::Anonyme.admet(NatureSession::Complete));
        // Une route de connexion, et elle seule, est décidée avec le principal anonyme ; une
        // route d'enrôlement ne décide qu'une action sur ses facteurs.
        for route in ROUTES {
            assert_eq!(
                route.portee == Portee::Anonyme,
                route.action == ActionRoute::Connexion,
                "{}",
                route.chemin
            );
            if route.portee == Portee::Facteurs {
                assert!(
                    [ActionFacteur::EnrolerSecours, ActionFacteur::EnrolerPasskey]
                        .map(ActionRoute::Facteur)
                        .contains(&route.action),
                    "{}",
                    route.chemin
                );
            }
        }
    }

    #[test]
    fn la_table_est_l_assemblage_des_tranches() {
        let tranches: Vec<Route> = MODULES.iter().flat_map(|m| m.routes).copied().collect();
        assert_eq!(ROUTES, tranches.as_slice());
    }

    #[test]
    fn chaque_route_releve_de_son_module() {
        let mut noms = BTreeSet::new();
        for module in MODULES {
            assert!(noms.insert(module.nom), "module {} en double", module.nom);
            for route in module.routes {
                assert_eq!(route.cas_usage.module(), module.nom, "{}", route.chemin);
            }
        }
    }

    #[test]
    fn aucune_route_n_est_declaree_deux_fois() {
        let mut vues = BTreeSet::new();
        for route in ROUTES {
            assert!(
                vues.insert((route.methode.nom(), route.chemin)),
                "{} {} déclarée deux fois",
                route.methode.nom(),
                route.chemin
            );
        }
    }
}
