//! Journaux techniques en **liste blanche** de champs (F2.11, VP3 ; ASVS 4.0.3 V7.1.1, V7.1.2).
//!
//! Une seule ligne par requête, émise à la fin du traitement sous la cible [`CIBLE`], avec les
//! seuls champs de [`CHAMPS`] :
//! - `methode` : méthode HTTP connue, sinon `autre` ;
//! - `route` : **modèle** de la route (`/api/cabinets/{cabinet}`), `statique` pour un fichier de
//!   l'interface, `aucune` hors table ; jamais le chemin réel, qui pourrait porter n'importe quoi ;
//! - `statut`, `duree_ms` ;
//! - `requete` : identifiant tiré par le serveur (UUID v4), renvoyé dans l'en-tête
//!   `x-sigma-requete` ; jamais repris du client (injection dans les journaux) ;
//! - `erreur` : catégorie d'une erreur interne (liste fermée, [`super::CategorieErreur`]).
//!
//! Jamais de montant, de libellé, de nom, de jeton, de cookie, d'en-tête ni de corps. Les
//! journaux des dépendances (`sqlx` cite le texte des requêtes) sont écartés par [`filtre`].

use std::time::Instant;

use axum::extract::{MatchedPath, Request};
use axum::http::{HeaderName, HeaderValue, Method};
use axum::middleware::Next;
use axum::response::Response;
use tracing_subscriber::filter::{LevelFilter, Targets};
use uuid::Uuid;

use super::erreur::CategorieErreur;

/// Cible `tracing` des journaux de Sigma Books.
pub const CIBLE: &str = "sigma";

/// Champs admis dans un événement de la cible [`CIBLE`]. `message` ne reçoit que des textes
/// constants.
pub const CHAMPS: &[&str] = &[
    "message",
    "methode",
    "route",
    "statut",
    "duree_ms",
    "requete",
    "erreur",
    "adresse_ecoute",
];

/// En-tête de réponse qui porte l'identifiant de la requête.
pub const EN_TETE_REQUETE: HeaderName = HeaderName::from_static("x-sigma-requete");

/// Filtre des journaux : la cible [`CIBLE`] au niveau `INFO`, rien d'autre.
#[must_use]
pub fn filtre() -> Targets {
    Targets::new()
        .with_target(CIBLE, LevelFilter::INFO)
        .with_default(LevelFilter::OFF)
}

/// Modèle de route d'une requête statique (fichiers de l'interface).
#[derive(Debug, Clone, Copy)]
pub(crate) struct RouteStatique;

fn nom_methode(methode: &Method) -> &'static str {
    match *methode {
        Method::GET => "GET",
        Method::HEAD => "HEAD",
        Method::POST => "POST",
        Method::PUT => "PUT",
        Method::PATCH => "PATCH",
        Method::DELETE => "DELETE",
        Method::OPTIONS => "OPTIONS",
        _ => "autre",
    }
}

/// Intergiciel le plus externe : identifiant de requête, durée, ligne de journal.
pub async fn journaliser(requete: Request, suivant: Next) -> Response {
    let debut = Instant::now();
    let methode = nom_methode(requete.method());
    let id = Uuid::new_v4().hyphenated().to_string();
    let mut reponse = suivant.run(requete).await;
    // Le modèle de route est connu après le routage : le routeur le recopie dans la réponse.
    let route = if let Some(modele) = reponse.extensions().get::<MatchedPath>() {
        modele.as_str().to_owned()
    } else if reponse.extensions().get::<RouteStatique>().is_some() {
        "statique".to_owned()
    } else {
        "aucune".to_owned()
    };
    let erreur = reponse
        .extensions()
        .get::<CategorieErreur>()
        .map(|c| c.nom());
    let duree_ms = u64::try_from(debut.elapsed().as_millis()).unwrap_or(u64::MAX);
    tracing::info!(
        target: CIBLE,
        methode,
        route = route.as_str(),
        statut = reponse.status().as_u16(),
        duree_ms,
        requete = id.as_str(),
        erreur,
        "requête traitée"
    );
    if let Ok(valeur) = HeaderValue::from_str(&id) {
        reponse.headers_mut().insert(EN_TETE_REQUETE, valeur);
    }
    reponse
}
