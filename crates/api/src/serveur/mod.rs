//! Serveur HTTP (`axum` sur `hyper`, ADR 0002 et 0003 ; Fondations §3.3 et §6.3 ; L2 PR 6).
//!
//! Intergiciels, de l'extérieur vers l'intérieur (chacun voit les réponses des suivants) :
//! 1. [`journal::journaliser`] : identifiant de requête, durée, journal en liste blanche ;
//! 2. [`entetes::securiser`] : en-têtes de sécurité sur toute réponse, erreurs comprises ;
//! 3. [`uniformiser`] : toute réponse d'erreur qui ne vient pas de [`ErreurApi`] (rejet d'un
//!    extracteur d'`axum`, limite de `tower-http`, fichier statique) reçoit le corps uniforme ;
//! 4. capture des paniques (`CatchPanicLayer`) : 500 au corps uniforme, catégorie `panique` ;
//! 5. délai d'expiration du traitement (503) ;
//! 6. limitation de débit par adresse (429) et taille **annoncée** du corps (413) ;
//! 7. [`csrf::proteger`] : anti-CSRF des mutations, avant toute authentification (403) ;
//! 8. taille **lue** du corps ([`borner_corps`]) : un corps sans `Content-Length` (envoyé par
//!    morceaux) est coupé à la même limite ;
//! 9. routeur : table [`routes::ROUTES`] (assemblée des tranches des modules de routes,
//!    [`routes::MODULES`]) et [`pipeline`], sinon 404 uniforme sous `/api`, sinon fichiers
//!    statiques de l'interface. Aucune route n'est déclarée hors de la table (test structurel
//!    `tests/structure.rs`).
//!
//! Les limites de la couche HTTP (taille et nombre des en-têtes, délai de lecture des en-têtes)
//! sont posées sur la connexion par [`servir`].
//!
//! Taille du corps et délai de traitement valent ceux des [`Reglages`], sauf pour une route qui
//! a les siens ([`Application::limites`] : le dépôt d'une pièce, ADR 0015). Les étapes 5, 6 et
//! 8 retrouvent la route d'après le modèle de chemin reconnu et la méthode.
//!
//! # Limites connues
//! - Les réponses que `hyper` émet lui-même, **avant** le routeur (431 pour des en-têtes trop
//!   grands ou trop nombreux, 400 pour une requête qu'il ne sait pas analyser), partent **sans**
//!   les en-têtes de sécurité ni le corps uniforme, et sans ligne de journal : elles ne portent
//!   aucune donnée, et la connexion est fermée.
//! - Le nombre de connexions simultanées n'est pas borné ici : cette limite revient au
//!   mandataire inverse (Fondations §3.3, L9), comme la terminaison TLS.

pub mod audit;
pub mod authentification;
pub mod config;
pub mod csrf;
pub mod entetes;
pub mod erreur;
pub mod facteurs;
pub mod integrite;
pub mod journal;
pub mod limites;
mod moi;
mod passkeys;
pub mod pipeline;
pub mod routes;
pub mod session;
mod statique;

use std::convert::Infallible;
use std::future::Future;
use std::io;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use axum::Router;
use axum::extract::{ConnectInfo, DefaultBodyLimit, MatchedPath, Request, State};
use axum::http::{HeaderName, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{any, on};
use http_body_util::Limited;
use hyper::body::Incoming;
use hyper_util::rt::{TokioIo, TokioTimer};
use sqlx::PgPool;
use tokio::net::TcpListener;
use tower::ServiceExt;
use tower_http::catch_panic::CatchPanicLayer;

pub use authentification::{Authentification, Identite, NatureSession};
pub use erreur::{CategorieErreur, ErreurApi};
pub use limites::ReglageDebit;
pub use pipeline::Decideur;
pub use session::{AuthentificationSession, Horloge, HorlogeSysteme};

use crate::base::IdUtilisateur;
use crate::base::session::DureesSession;
use crate::pieces::Pieces;
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
    /// Durées des sessions (ADR 0012) et conservation des sessions closes.
    pub sessions: DureesSession,
    /// Méthode de secours (PR 8b) : clé de l'instance, débits des connexions et des
    /// vérifications, hachages simultanés ([`facteurs::configurer`]).
    pub facteurs: facteurs::ReglagesFacteurs,
    /// Nombre maximal d'écritures lues pour une édition (PR 14) ; au-delà, 409
    /// `edition_trop_volumineuse`.
    pub editions_ecritures_max: i64,
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
            sessions: DureesSession::default(),
            facteurs: facteurs::ReglagesFacteurs::default(),
            editions_ecritures_max: crate::base::editions::ECRITURES_MAX,
        }
    }
}

/// État partagé du serveur.
pub struct Application {
    pool: PgPool,
    /// Pool **distinct** des lectures à instantané unique d'un cas d'usage (même rôle
    /// applicatif, mêmes options de connexion). La transaction du pipeline tient déjà une
    /// connexion de `pool` : une seconde prise dans le même pool ferait attendre chaque requête
    /// d'une connexion que seules les autres requêtes, elles aussi en attente, pourraient
    /// libérer (interblocage, déni de service par un seul utilisateur). Ici, aucun cas d'usage ne
    /// tient une connexion de ce pool en en attendant une autre : il se vide toujours.
    pool_lectures: PgPool,
    /// Pool du rôle de lecture d'audit (ADR 0014), distinct.
    pool_lecture_audit: PgPool,
    decideur: Arc<dyn Decideur>,
    authentification: Arc<dyn Authentification>,
    horloge: Arc<dyn Horloge>,
    reglages: Reglages,
    limiteur_adresse: LimiteurDebit<CleAdresse>,
    limiteur_utilisateur: LimiteurDebit<IdUtilisateur>,
    /// Voie authentification : état de la méthode de secours (débits, hachages).
    facteurs: facteurs::EtatFacteurs,
    // Services des voies, branchés après l'assemblage.
    //
    // Voie pièces (PR 12) : stockage objet des pièces ([`Application::brancher_pieces`]).
    pieces: OnceLock<Pieces>,
}

/// Connexions du pool des lectures à instantané unique ([`Application::pool_lectures`]).
const CONNEXIONS_LECTURES: u32 = 4;

/// Pool des lectures à instantané unique : celui du rôle applicatif (déjà vérifié à son
/// ouverture), ouvert à la demande, avec ses options de connexion et son délai d'attente.
fn pool_des_lectures(pool: &PgPool) -> PgPool {
    sqlx::postgres::PgPoolOptions::new()
        .max_connections(CONNEXIONS_LECTURES)
        .acquire_timeout(pool.options().get_acquire_timeout())
        .connect_lazy_with((*pool.connect_options()).clone())
}

/// État partagé, tel que le reçoivent les intergiciels et le pipeline.
pub type Etat = Arc<Application>;

impl Application {
    /// Assemble l'application avec l'horloge du système. Le binaire passe le pool du rôle
    /// applicatif, celui du rôle de lecture d'audit, [`authz::Autorisation`] et
    /// [`AuthentificationSession`] (sessions lues dans PostgreSQL).
    #[must_use]
    pub fn new(
        pool: PgPool,
        pool_lecture_audit: PgPool,
        decideur: Arc<dyn Decideur>,
        authentification: Arc<dyn Authentification>,
        reglages: Reglages,
    ) -> Etat {
        Self::avec_horloge(
            pool,
            pool_lecture_audit,
            decideur,
            authentification,
            reglages,
            Arc::new(HorlogeSysteme),
        )
    }

    /// Comme [`Application::new`], avec une horloge donnée (tests : expiration des sessions,
    /// fraîcheur). Elle doit être celle de l'authentification.
    #[must_use]
    pub fn avec_horloge(
        pool: PgPool,
        pool_lecture_audit: PgPool,
        decideur: Arc<dyn Decideur>,
        authentification: Arc<dyn Authentification>,
        reglages: Reglages,
        horloge: Arc<dyn Horloge>,
    ) -> Etat {
        let pool_lectures = pool_des_lectures(&pool);
        Arc::new(Self {
            pool,
            pool_lectures,
            pool_lecture_audit,
            decideur,
            authentification,
            horloge,
            limiteur_adresse: LimiteurDebit::new(reglages.debit_adresse),
            limiteur_utilisateur: LimiteurDebit::new(reglages.debit_utilisateur),
            facteurs: facteurs::EtatFacteurs::new(&reglages.facteurs, &reglages.origine),
            reglages,
            pieces: OnceLock::new(),
        })
    }

    /// Branche le service des pièces (ADR 0015), une seule fois, avant de servir. Sans lui,
    /// les routes des pièces répondent par une erreur interne (catégorie `stockage`).
    ///
    /// # Errors
    /// Le service rendu s'il était déjà branché.
    pub fn brancher_pieces(&self, pieces: Pieces) -> Result<(), Pieces> {
        self.pieces.set(pieces)
    }

    /// Le service des pièces, s'il est branché.
    #[must_use]
    pub fn pieces(&self) -> Option<&Pieces> {
        self.pieces.get()
    }

    /// Taille maximale du corps et délai de traitement de la route : ceux des [`Reglages`],
    /// sauf exigence propre de la route.
    #[must_use]
    pub fn limites(&self, route: &routes::Route) -> (usize, Duration) {
        let defaut = (self.reglages.taille_corps_max, self.reglages.delai_requete);
        match route.cas_usage {
            // Voie pièces (PR 12) : le dépôt d'une pièce.
            routes::CasUsage::Pieces(cas) => {
                routes::pieces::limites(cas, self.pieces()).unwrap_or(defaut)
            }
            _ => defaut,
        }
    }

    /// Limites de la requête : celles de sa route si elle en a une (modèle de chemin reconnu et
    /// méthode), sinon celles des [`Reglages`].
    fn limites_requete(&self, requete: &Request) -> (usize, Duration) {
        let route = requete
            .extensions()
            .get::<MatchedPath>()
            .and_then(|modele| {
                routes::ROUTES.iter().find(|r| {
                    r.chemin == modele.as_str() && r.methode.methode() == *requete.method()
                })
            });
        route.map_or(
            (self.reglages.taille_corps_max, self.reglages.delai_requete),
            |r| self.limites(r),
        )
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
    envelopper(routeur, etat)
}

/// Pose les intergiciels, dans l'ordre documenté en tête du module.
fn envelopper(routeur: Router, etat: &Etat) -> Router {
    let taille = etat.reglages.taille_corps_max;
    routeur
        .layer(DefaultBodyLimit::max(taille))
        .layer(middleware::from_fn_with_state(
            Arc::clone(etat),
            borner_corps,
        ))
        .layer(middleware::from_fn_with_state(
            Arc::clone(etat),
            csrf::proteger,
        ))
        .layer(middleware::from_fn_with_state(Arc::clone(etat), limiter))
        .layer(middleware::from_fn_with_state(
            Arc::clone(etat),
            borner_duree,
        ))
        .layer(CatchPanicLayer::custom(reponse_de_panique))
        .layer(middleware::from_fn(uniformiser))
        .layer(middleware::from_fn(entetes::securiser))
        .layer(middleware::from_fn(journal::journaliser))
}

/// Réponse à une panique : 500 uniforme. Le message de la panique n'est ni renvoyé ni journalisé
/// (il pourrait citer des données, F2.11).
fn reponse_de_panique(_panique: Box<dyn std::any::Any + Send + 'static>) -> Response {
    ErreurApi::Interne(CategorieErreur::Panique).into_response()
}

/// Remplace le corps de toute réponse d'erreur (4xx, 5xx) qui ne vient pas de [`ErreurApi`]
/// par le corps uniforme du même statut : aucun texte d'`axum` ni de `tower-http` (qui peut
/// citer l'entrée) ne sort.
async fn uniformiser(requete: Request, suivant: Next) -> Response {
    let reponse = suivant.run(requete).await;
    let statut = reponse.status();
    if (statut.is_client_error() || statut.is_server_error())
        && reponse
            .extensions()
            .get::<erreur::CorpsUniforme>()
            .is_none()
    {
        return erreur::pour_statut(statut);
    }
    reponse
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
    let (taille_max, _) = etat.limites_requete(&requete);
    match annoncee {
        Some(Ok(taille)) if taille > taille_max as u64 => {
            return ErreurApi::CorpsTropGrand.into_response();
        }
        Some(Err(_)) => return ErreurApi::RequeteInvalide.into_response(),
        _ => {}
    }
    // Corps sans longueur annoncée : borné à la lecture par `borner_corps`.
    suivant.run(requete).await
}

/// Délai d'expiration du traitement : au-delà, la requête est abandonnée (sa transaction est
/// annulée) et le client reçoit 503.
async fn borner_duree(State(etat): State<Etat>, requete: Request, suivant: Next) -> Response {
    let (_, delai) = etat.limites_requete(&requete);
    match tokio::time::timeout(delai, suivant.run(requete)).await {
        Ok(reponse) => reponse,
        Err(_) => ErreurApi::DelaiDepasse.into_response(),
    }
}

/// Taille **lue** du corps : coupé à la limite de sa route (erreur de lecture, que le pipeline
/// et les extracteurs d'`axum` traduisent en 413), qu'il ait ou non une longueur annoncée.
async fn borner_corps(State(etat): State<Etat>, requete: Request, suivant: Next) -> Response {
    let (taille_max, _) = etat.limites_requete(&requete);
    let (parts, corps) = requete.into_parts();
    let requete = Request::from_parts(
        parts,
        axum::body::Body::new(Limited::new(corps, taille_max)),
    );
    suivant.run(requete).await
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

#[cfg(test)]
mod tests {
    use super::*;

    use axum::body::Bytes;
    use axum::routing::post;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[tokio::test]
    async fn un_corps_par_morceaux_sans_longueur_est_coupe_a_la_limite() {
        let mut reglages = Reglages::new("https://sigma.exemple.invalid".to_owned());
        reglages.taille_corps_max = 1024;
        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect_lazy("postgres://sigma_inexistant@127.0.0.1:1/sigma_inexistante")
            .unwrap();
        let etat = Application::new(
            pool.clone(),
            pool.clone(),
            Arc::new(authz::Autorisation::charger().unwrap()),
            Arc::new(AuthentificationSession::new(
                pool,
                Arc::new(HorlogeSysteme),
                reglages.sessions,
            )),
            reglages.clone(),
        );
        // Route de test qui lit le corps (aucune route de la table n'en lit en PR 6).
        let routeur = envelopper(
            Router::new().route(
                "/api/essai",
                post(|corps: Bytes| async move { corps.len().to_string() }),
            ),
            &etat,
        );
        let ecouteur = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let adresse = ecouteur.local_addr().unwrap();
        let (arreter, arret) = tokio::sync::oneshot::channel::<()>();
        let serveur = tokio::spawn(async move {
            servir(ecouteur, routeur, &reglages, async {
                let _ = arret.await;
            })
            .await
        });

        let envoyer = |morceaux: usize| async move {
            let mut flux = tokio::net::TcpStream::connect(adresse).await.unwrap();
            let mut requete = String::from(
                "POST /api/essai HTTP/1.1\r\nHost: x\r\nx-sigma-csrf: 1\r\n\
                 Transfer-Encoding: chunked\r\nConnection: close\r\n\r\n",
            );
            for _ in 0..morceaux {
                requete.push_str("100\r\n");
                requete.push_str(&"a".repeat(256));
                requete.push_str("\r\n");
            }
            requete.push_str("0\r\n\r\n");
            // Le serveur peut fermer avant la fin de l'envoi : l'erreur d'écriture est attendue.
            let _ = flux.write_all(requete.as_bytes()).await;
            let mut reponse = Vec::new();
            let _ = flux.read_to_end(&mut reponse).await;
            String::from_utf8_lossy(&reponse).into_owned()
        };
        // 4 × 256 octets : à la limite, admis.
        let reponse = envoyer(4).await;
        assert!(reponse.starts_with("HTTP/1.1 200"), "{reponse}");
        // 8 × 256 octets sans Content-Length : 413 au corps uniforme, en-têtes de sécurité.
        let reponse = envoyer(8).await;
        assert!(reponse.starts_with("HTTP/1.1 413"), "{reponse}");
        assert!(
            reponse.ends_with("{\"erreur\":\"corps_trop_grand\"}"),
            "{reponse}"
        );
        assert!(
            reponse.contains("x-content-type-options: nosniff"),
            "{reponse}"
        );

        arreter.send(()).unwrap();
        serveur.await.unwrap().unwrap();
    }
}
