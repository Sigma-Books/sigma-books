//! Fichiers statiques de l'interface compilée (`web/`, `adapter-static` ; ADR 0002).
//!
//! `tower_http::services::ServeDir` refuse la traversée de chemin (tout segment `..`, préfixe ou
//! racine, après décodage des `%xx` : réponse « absent ») et ne liste jamais un répertoire : un
//! répertoire ne sert que son `index.html`, sinon 404 (ASVS 4.0.3 V12.3.1, V14.3.1 ; F1.5).
//! Avant lui, deux contrôles :
//! - tout segment qui commence par `.` après décodage (`/.env`, `/%2e%65nv`, `/.git/config`)
//!   est absent : un fichier caché ne sort jamais, même s'il se trouve dans le répertoire ;
//! - le chemin **canonique** du fichier visé (liens symboliques résolus) doit rester sous la
//!   racine canonique : un lien symbolique qui sort de la racine donne 404. Le répertoire est
//!   produit par la compilation de l'interface et n'est pas modifiable par un client ; un lien
//!   remplacé entre le contrôle et la lecture relève de l'exploitation, pas d'une requête.
//!
//! Aucune redirection n'est émise (`redirect_to_trailing_slash(false)`), pour qu'un chemin forgé
//! ne produise jamais d'en-tête `Location`. Seules `GET` et `HEAD` sont admises ; toute erreur
//! prend le corps uniforme de l'API.

use std::convert::Infallible;
use std::path::{Path, PathBuf};
use std::sync::Arc;

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
    // Racine canonique ; si elle ne se résout pas, aucun fichier ne passera le contrôle.
    let racine: Arc<Option<PathBuf>> = Arc::new(std::fs::canonicalize(repertoire).ok());
    service_fn(move |requete: Request| {
        let fichiers = fichiers.clone();
        let racine = Arc::clone(&racine);
        async move {
            let admis = match racine.as_ref() {
                Some(racine) => chemin_admis(racine, requete.uri().path()).await,
                None => false,
            };
            let mut reponse = if !matches!(*requete.method(), Method::GET | Method::HEAD) {
                ErreurApi::MethodeNonAdmise.into_response()
            } else if !admis {
                ErreurApi::Introuvable.into_response()
            } else {
                match fichiers.oneshot(requete).await {
                    Ok(reponse) => reponse.map(Body::new),
                    Err(infaillible) => match infaillible {},
                }
            };
            reponse.extensions_mut().insert(RouteStatique);
            Ok::<_, Infallible>(reponse)
        }
    })
}

/// Segments décodés du chemin ; `None` si un segment est invalide (UTF-8, `%` mal formé) ou
/// caché (commence par `.`, ce qui couvre aussi `.` et `..`).
fn segments(chemin: &str) -> Option<Vec<String>> {
    chemin
        .split('/')
        .filter(|s| !s.is_empty())
        .map(|s| {
            let decode = decoder(s)?;
            let cache = decode.starts_with('.') || decode.contains(['/', '\\', '\0']);
            (!cache).then_some(decode)
        })
        .collect()
}

/// Décodage des `%xx` d'un segment (une seule passe, comme `ServeDir`).
fn decoder(segment: &str) -> Option<String> {
    let octets = segment.as_bytes();
    let mut sortie = Vec::with_capacity(octets.len());
    let mut i = 0;
    while i < octets.len() {
        if octets[i] == b'%' {
            let hex = std::str::from_utf8(octets.get(i + 1..i + 3)?).ok()?;
            sortie.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            sortie.push(octets[i]);
            i += 1;
        }
    }
    String::from_utf8(sortie).ok()
}

/// Vrai si le chemin ne vise aucun segment caché et si sa cible canonique (le fichier, ou
/// l'`index.html` d'un répertoire) reste sous la racine. Une cible inexistante est admise ici :
/// `ServeDir` répondra « absent ».
async fn chemin_admis(racine: &Path, chemin: &str) -> bool {
    let Some(segments) = segments(chemin) else {
        return false;
    };
    let mut cible = racine.to_path_buf();
    cible.extend(&segments);
    let Ok(canonique) = tokio::fs::canonicalize(&cible).await else {
        return true;
    };
    if !canonique.starts_with(racine) {
        return false;
    }
    if tokio::fs::metadata(&canonique)
        .await
        .is_ok_and(|m| m.is_dir())
        && let Ok(index) = tokio::fs::canonicalize(canonique.join("index.html")).await
    {
        return index.starts_with(racine);
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn les_segments_caches_ou_mal_formes_sont_refuses() {
        for chemin in [
            "/.env",
            "/%2e%65nv",
            "/%2E%65nv",
            "/sous/.git/config",
            "/..",
            "/%2e%2e/x",
            "/a%2fb",
            "/a%5cb",
            "/a%00",
            "/%zz",
            "/%e9",
        ] {
            assert_eq!(segments(chemin), None, "{chemin}");
        }
        assert_eq!(
            segments("/sous/fichier%20a.txt"),
            Some(vec!["sous".to_owned(), "fichier a.txt".to_owned()])
        );
        assert_eq!(segments("/"), Some(vec![]));
    }
}
