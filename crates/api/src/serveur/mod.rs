//! Serveur HTTP (`axum` sur `hyper`, ADR 0002 et 0003 ; Fondations §3.3 et §6.3 ; L2 PR 6).
//!
//! Intergiciels, de l'extérieur vers l'intérieur (chacun voit les réponses des suivants) :
//! 1. [`journal::journaliser`] : identifiant de requête, durée, journal en liste blanche ;
//! 2. [`entetes::securiser`] : en-têtes de sécurité sur toute réponse, erreurs comprises ;
//! 3. délai d'expiration du traitement (503) ;
//! 4. limitation de débit par adresse (429) et taille maximale du corps (413) ;
//! 5. [`csrf::proteger`] : anti-CSRF des mutations, avant toute authentification (403) ;
//! 6. routeur : table [`routes::ROUTES`] et [`pipeline`], sinon 404 uniforme sous `/api`, sinon
//!    fichiers statiques de l'interface.
//!
//! Les limites de la couche HTTP (taille et nombre des en-têtes, délai de lecture des en-têtes)
//! sont posées sur la connexion par [`servir`] (431, fermeture).

pub mod authentification;
pub mod config;
pub mod csrf;
pub mod entetes;
pub mod erreur;
pub mod journal;
pub mod limites;
pub mod pipeline;
pub mod routes;
mod statique;

use std::convert::Infallible;
use std::future::Future;
use std::io;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::Router;
use axum::extract::{ConnectInfo, DefaultBodyLimit, MatchedPath, Request, State};
use axum::http::{HeaderName, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{any, on};
use hyper::body::Incoming;
use hyper_util::rt::{TokioIo, TokioTimer};
use sqlx::PgPool;
use tokio::net::TcpListener;
use tower::ServiceExt;

pub use authentification::{Authentification, Identite, RefusTotal};
pub use erreur::{CategorieErreur, ErreurApi};
pub use limites::ReglageDebit;
pub use pipeline::Decideur;

use crate::base::IdUtilisateur;
use limites::{CleAdresse, LimiteurDebit, adresse_client};

/// Réglages du serveur. [`Reglages::new`] donne les valeurs par défaut documentées.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reglages {
    /// Origine publique de l'instance (`https://hote[:port]`), seule admise en `Origin` d'une
    /// mutation (anti-CSRF).
    pub origine: String,
    /// Répertoire de l'interface compilée (`web/build`) ; aucun fichier servi s'il est absent.
    pub repertoire_web: Option<PathBuf>,
    /// En-tête de confiance qui porte l'adresse du client derrière un mandataire inverse (L9).
    /// Absent par défaut : l'adresse de la connexion fait foi.
    pub en_tete_adresse_client: Option<HeaderName>,
    /// Débit par adresse.
    pub debit_adresse: ReglageDebit,
    /// Débit par utilisateur authentifié.
    pub debit_utilisateur: ReglageDebit,
    /// Taille maximale d'un corps de requête, en octets.
    pub taille_corps_max: usize,
    /// Durée maximale du traitement d'une requête.
    pub delai_requete: Duration,
    /// Durée maximale de réception des en-têtes (connexions lentes).
    pub delai_en_tetes: Duration,
    /// Taille maximale des en-têtes (ligne de requête comprise), en octets.
    pub taille_en_tetes_max: usize,
    /// Nombre maximal d'en-têtes.
    pub nombre_en_tetes_max: usize,
}

impl Reglages {
    /// Réglages par défaut pour l'origine donnée.
    #[must_use]
    pub fn new(origine: String) -> Self {
        Self {
            origine,
            repertoire_web: None,
            en_tete_adresse_client: None,
            debit_adresse: ReglageDebit {
                capacite: 600,
                periode: Duration::from_secs(60),
                cles_max: 100_000,
            },
            debit_utilisateur: ReglageDebit {
                capacite: 300,
                periode: Duration::from_secs(60),
                cles_max: 100_000,
            },
            taille_corps_max: 64 * 1024,
            delai_requete: Duration::from_secs(30),
            delai_en_tetes: Duration::from_secs(10),
            taille_en_tetes_max: 16 * 1024,
            nombre_en_tetes_max: 64,
        }
    }
}

/// État partagé du serveur.
pub struct Application {
    pool: PgPool,
    decideur: Arc<dyn Decideur>,
    authentification: Arc<dyn Authentification>,
    reglages: Reglages,
    limiteur_adresse: LimiteurDebit<CleAdresse>,
    limiteur_utilisateur: LimiteurDebit<IdUtilisateur>,
}

/// État partagé, tel que le reçoivent les intergiciels et le pipeline.
pub type Etat = Arc<Application>;

impl Application {
    /// Assemble l'application. Le binaire passe le pool du rôle applicatif, [`authz::Autorisation`]
    /// et [`RefusTotal`] (jusqu'aux sessions, PR 8).
    #[must_use]
    pub fn new(
        pool: PgPool,
        decideur: Arc<dyn Decideur>,
        authentification: Arc<dyn Authentification>,
        reglages: Reglages,
    ) -> Etat {
        Arc::new(Self {
            pool,
            decideur,
            authentification,
            limiteur_adresse: LimiteurDebit::new(reglages.debit_adresse),
            limiteur_utilisateur: LimiteurDebit::new(reglages.debit_utilisateur),
            reglages,
        })
    }

    /// Les réglages.
    #[must_use]
    pub fn reglages(&self) -> &Reglages {
        &self.reglages
    }
}

/// Construit le routeur à partir de la seule table [`routes::ROUTES`].
pub fn routeur(etat: &Etat) -> Router {
    let mut routeur = Router::new();
    for route in routes::ROUTES {
        let etat_route = Arc::clone(etat);
        routeur = routeur.route(
            route.chemin,
            on(route.methode.filtre(), move |requete: Request| {
                pipeline::traiter(Arc::clone(&etat_route), route, requete)
            }),
        );
    }
    let routeur = routeur
        .route(entetes::PREFIXE_API, any(introuvable))
        .route("/api/{*reste}", any(introuvable))
        .method_not_allowed_fallback(methode_non_admise)
        .route_layer(middleware::from_fn(recopier_route));
    let routeur = match &etat.reglages.repertoire_web {
        Some(repertoire) => routeur.fallback_service(statique::service(repertoire)),
        None => routeur.fallback(introuvable),
    };
    routeur
        .layer(DefaultBodyLimit::max(etat.reglages.taille_corps_max))
        .layer(middleware::from_fn_with_state(
            Arc::clone(etat),
            csrf::proteger,
        ))
        .layer(middleware::from_fn_with_state(Arc::clone(etat), limiter))
        .layer(middleware::from_fn_with_state(
            Arc::clone(etat),
            borner_duree,
        ))
        .layer(middleware::from_fn(entetes::securiser))
        .layer(middleware::from_fn(journal::journaliser))
}

async fn introuvable() -> ErreurApi {
    ErreurApi::Introuvable
}

async fn methode_non_admise() -> ErreurApi {
    ErreurApi::MethodeNonAdmise
}

/// Recopie le modèle de la route dans la réponse, pour le journal.
async fn recopier_route(requete: Request, suivant: Next) -> Response {
    let modele = requete.extensions().get::<MatchedPath>().cloned();
    let mut reponse = suivant.run(requete).await;
    if let Some(modele) = modele {
        reponse.extensions_mut().insert(modele);
    }
    reponse
}

/// Débit par adresse, puis taille annoncée du corps.
async fn limiter(State(etat): State<Etat>, requete: Request, suivant: Next) -> Response {
    let cle = adresse_client(&requete, etat.reglages.en_tete_adresse_client.as_ref());
    if let Err(reessayer_dans) = etat.limiteur_adresse.verifier(cle, Instant::now()) {
        return ErreurApi::TropDeRequetes { reessayer_dans }.into_response();
    }
    let annoncee = requete
        .headers()
        .get(header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .map(str::parse::<u64>);
    match annoncee {
        Some(Ok(taille)) if taille > etat.reglages.taille_corps_max as u64 => {
            return ErreurApi::CorpsTropGrand.into_response();
        }
        Some(Err(_)) => return ErreurApi::RequeteInvalide.into_response(),
        _ => {}
    }
    // Corps sans longueur annoncée : borné à la lecture par `DefaultBodyLimit`.
    suivant.run(requete).await
}

/// Délai d'expiration du traitement : au-delà, la requête est abandonnée (sa transaction est
/// annulée) et le client reçoit 503.
async fn borner_duree(State(etat): State<Etat>, requete: Request, suivant: Next) -> Response {
    match tokio::time::timeout(etat.reglages.delai_requete, suivant.run(requete)).await {
        Ok(reponse) => reponse,
        Err(_) => ErreurApi::DelaiDepasse.into_response(),
    }
}

/// Sert le routeur sur l'écouteur jusqu'à `arret`, en HTTP/1.1, avec les limites de la couche
/// HTTP des [`Reglages`] : taille et nombre des en-têtes (431 au-delà), délai de réception des
/// en-têtes. L'adresse de la connexion est transmise au routeur (`ConnectInfo`).
///
/// # Errors
/// Erreur de l'écouteur autre qu'une erreur passagère d'acceptation.
pub async fn servir(
    ecouteur: TcpListener,
    routeur: Router,
    reglages: &Reglages,
    arret: impl Future<Output = ()>,
) -> io::Result<()> {
    let mut http = hyper::server::conn::http1::Builder::new();
    http.timer(TokioTimer::new())
        .header_read_timeout(reglages.delai_en_tetes)
        .max_header_size(reglages.taille_en_tetes_max)
        .max_headers(reglages.nombre_en_tetes_max);
    tokio::pin!(arret);
    loop {
        let (flux, pair) = tokio::select! {
            () = &mut arret => return Ok(()),
            accepte = ecouteur.accept() => {
                // Erreur passagère (descripteurs épuisés…) : on patiente sans boucler à vide.
                let Ok(connexion) = accepte else {
                    tokio::time::sleep(Duration::from_millis(50)).await;
                    continue;
                };
                connexion
            }
        };
        let routeur = routeur.clone();
        let service = hyper::service::service_fn(move |mut requete: hyper::Request<Incoming>| {
            requete
                .extensions_mut()
                .insert(ConnectInfo::<SocketAddr>(pair));
            let routeur = routeur.clone();
            async move { Ok::<_, Infallible>(routeur.oneshot(requete).await.into_response()) }
        });
        let connexion = http.serve_connection(TokioIo::new(flux), service);
        tokio::spawn(async move {
            // Une connexion en erreur (client parti, en-têtes trop grands) ne concerne qu'elle.
            let _ = connexion.await;
        });
    }
}
