//! Fichiers statiques de l'interface compilée (`web/`, `adapter-static` ; ADR 0002).
//!
//! `tower_http::services::ServeDir` refuse la traversée de chemin (tout segment `..`, préfixe ou
//! racine, après décodage des `%xx` : réponse « absent ») et ne liste jamais un répertoire : un
//! répertoire ne sert que son `index.html`, sinon 404 (ASVS 4.0.3 V12.3.1, V14.3.1 ; F1.5).
//! Aucune redirection n'est émise (`redirect_to_trailing_slash(false)`), pour qu'un chemin forgé
//! ne produise jamais d'en-tête `Location`. Seules `GET` et `HEAD` sont admises ; toute erreur
//! prend le corps uniforme de l'API.

use std::convert::Infallible;
use std::path::Path;

use axum::body::Body;
use axum::extract::Request;
use axum::http::Method;
use axum::response::{IntoResponse, Response};
use tower::ServiceExt;
use tower::service_fn;
use tower_http::services::ServeDir;

use super::erreur::ErreurApi;
use super::journal::RouteStatique;

/// Service des fichiers de `repertoire`.
pub(super) fn service(
    repertoire: &Path,
) -> impl tower::Service<Request, Response = Response, Error = Infallible, Future: Send>
+ Clone
+ Send
+ Sync
+ 'static {
    let absent = service_fn(|_: Request| async {
        Ok::<_, Infallible>(ErreurApi::Introuvable.into_response())
    });
    let fichiers = ServeDir::new(repertoire)
        .append_index_html_on_directories(true)
        .redirect_to_trailing_slash(false)
        .not_found_service(absent);
    service_fn(move |requete: Request| {
        let fichiers = fichiers.clone();
        async move {
            let mut reponse = if matches!(*requete.method(), Method::GET | Method::HEAD) {
                match fichiers.oneshot(requete).await {
                    Ok(reponse) => reponse.map(Body::new),
                    Err(infaillible) => match infaillible {},
                }
            } else {
                ErreurApi::MethodeNonAdmise.into_response()
            };
            reponse.extensions_mut().insert(RouteStatique);
            Ok::<_, Infallible>(reponse)
        }
    })
}
