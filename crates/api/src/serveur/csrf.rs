//! Anti-CSRF des mutations (F1.4 ; ADR 0012 ; ASVS 4.0.3 V4.2.2, V13.2.3).
//!
//! Toute **mutation**, c'est-à-dire toute méthode autre que `GET`, `HEAD` et `OPTIONS` (comparées
//! sans tenir compte de la casse : `POST`, `PUT`, `PATCH`, `DELETE`, mais aussi toute méthode
//! d'extension), est refusée (403), **avant même l'authentification**, si :
//! 1. `Sec-Fetch-Site` est présent et différent de `same-origin` (requête venue d'un autre site,
//!    d'un sous-domaine, ou d'une navigation directe) ;
//! 2. `Origin` est présent et différent de l'origine configurée de l'instance ;
//! 3. l'en-tête personnalisé [`EN_TETE_CSRF`] est absent ou vide : un formulaire HTML ne sait pas
//!    le poser, et une requête d'un autre site qui le pose déclenche un contrôle préalable CORS que
//!    le serveur n'accepte jamais (aucun en-tête CORS n'est émis).
//!
//! Le cookie de session `SameSite=Strict` (PR 8) s'y ajoute. Avec les sessions, l'en-tête
//! [`EN_TETE_CSRF`] portera le **jeton anti-CSRF lié à la session** (ADR 0012), comparé à
//! celui de la session en temps constant ; d'ici là, sa seule présence est exigée.

use axum::extract::{Request, State};
use axum::http::{HeaderName, Method, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use super::Etat;
use super::erreur::ErreurApi;

/// En-tête personnalisé exigé sur toute mutation.
pub const EN_TETE_CSRF: HeaderName = HeaderName::from_static("x-sigma-csrf");

const SEC_FETCH_SITE: HeaderName = HeaderName::from_static("sec-fetch-site");

/// Vrai pour toute méthode autre que `GET`, `HEAD` et `OPTIONS`, sans tenir compte de la casse
/// (refus en cas de doute : une méthode inconnue est traitée comme une mutation).
#[must_use]
pub fn est_mutation(methode: &Method) -> bool {
    let nom = methode.as_str();
    !["GET", "HEAD", "OPTIONS"]
        .iter()
        .any(|sure| nom.eq_ignore_ascii_case(sure))
}

/// Vrai si la mutation porte les marques d'une requête de la même origine.
#[must_use]
pub fn mutation_admise(requete: &Request, origine: &str) -> bool {
    let entetes = requete.headers();
    if let Some(site) = entetes.get(SEC_FETCH_SITE)
        && site.as_bytes() != b"same-origin"
    {
        return false;
    }
    if let Some(origin) = entetes.get(header::ORIGIN)
        && origin.as_bytes() != origine.as_bytes()
    {
        return false;
    }
    entetes
        .get(EN_TETE_CSRF)
        .is_some_and(|valeur| !valeur.is_empty())
}

/// Intergiciel anti-CSRF.
pub async fn proteger(State(etat): State<Etat>, requete: Request, suivant: Next) -> Response {
    if est_mutation(requete.method()) && !mutation_admise(&requete, &etat.reglages.origine) {
        return ErreurApi::Interdit.into_response();
    }
    suivant.run(requete).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toute_methode_hors_get_head_options_est_une_mutation() {
        let methode = |nom: &str| Method::from_bytes(nom.as_bytes()).unwrap();
        for sure in ["GET", "HEAD", "OPTIONS", "get", "Head", "options"] {
            assert!(!est_mutation(&methode(sure)), "{sure}");
        }
        for mutation in [
            "POST", "PUT", "PATCH", "DELETE", "post", "Delete", "PROPFIND", "MKCOL", "CONNECT",
            "TRACE", "GETX",
        ] {
            assert!(est_mutation(&methode(mutation)), "{mutation}");
        }
    }
}
