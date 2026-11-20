//! En-têtes de sécurité posés sur **toutes** les réponses (F1.5 ; Fondations §3.3 ; L2, « En-têtes
//! HTTP »). Exigences ASVS 4.0.3 : V14.4.3 (CSP), V14.4.4 (`nosniff`), V14.4.5 (HSTS), V14.4.6
//! (`Referrer-Policy`), V14.4.7 (`frame-ancestors`), V8.2.1 (`Cache-Control: no-store` sur l'API).
//!
//! CSP : celle de l'interface reste celle de L0, posée par `SvelteKit` dans une balise `<meta>` des
//! pages prérendues (`web/vite.config.ts`, mode « hash »). Un navigateur applique **toutes** les
//! politiques reçues : l'en-tête ne peut donc que restreindre. Pour les fichiers de l'interface, il
//! n'ajoute que ce qu'une balise `<meta>` ne sait pas exprimer (`frame-ancestors`), plus deux
//! directives identiques à celles de la balise : il ne casse rien. Les réponses de l'API (JSON) ne
//! chargent rien : `default-src 'none'` ; elles sont en outre isolées (`sandbox`), ce qu'exige le
//! téléchargement d'une pièce justificative ouverte par erreur dans un onglet (ADR 0015, F1.5).

use axum::extract::Request;
use axum::http::{HeaderMap, HeaderName, HeaderValue, header};
use axum::middleware::Next;
use axum::response::Response;

/// Préfixe des routes de l'API.
pub const PREFIXE_API: &str = "/api";

/// CSP des réponses de l'API.
pub const CSP_API: &str =
    "sandbox; default-src 'none'; frame-ancestors 'none'; base-uri 'none'; form-action 'none'";

/// CSP des fichiers de l'interface : complète la balise `<meta>` de `SvelteKit` sans la contredire.
pub const CSP_INTERFACE: &str = "frame-ancestors 'none'; base-uri 'none'; object-src 'none'";

/// HSTS : deux ans, sous-domaines compris. Ignoré par les navigateurs en HTTP simple
/// (développement) ; en production, TLS est terminé par le mandataire inverse (L9).
pub const HSTS: &str = "max-age=63072000; includeSubDomains";

const COMMUNS: [(HeaderName, &str); 6] = [
    (header::STRICT_TRANSPORT_SECURITY, HSTS),
    (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
    (header::REFERRER_POLICY, "no-referrer"),
    (
        HeaderName::from_static("cross-origin-opener-policy"),
        "same-origin",
    ),
    (
        HeaderName::from_static("cross-origin-resource-policy"),
        "same-origin",
    ),
    (
        HeaderName::from_static("cross-origin-embedder-policy"),
        "require-corp",
    ),
];

/// Vrai si le chemin relève de l'API.
#[must_use]
pub fn est_api(chemin: &str) -> bool {
    chemin == PREFIXE_API
        || chemin
            .strip_prefix(PREFIXE_API)
            .is_some_and(|reste| reste.starts_with('/'))
}

/// Pose les en-têtes de sécurité, en remplaçant toute valeur antérieure.
pub fn poser(entetes: &mut HeaderMap, api: bool) {
    for (nom, valeur) in COMMUNS {
        entetes.insert(nom, HeaderValue::from_static(valeur));
    }
    let csp = if api { CSP_API } else { CSP_INTERFACE };
    entetes.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(csp),
    );
    if api {
        entetes.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    }
}

/// Intergiciel : en-têtes de sécurité sur toute réponse, erreurs comprises.
pub async fn securiser(requete: Request, suivant: Next) -> Response {
    let api = est_api(requete.uri().path());
    let mut reponse = suivant.run(requete).await;
    poser(reponse.headers_mut(), api);
    reponse
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn le_prefixe_de_l_api_est_exact() {
        assert!(est_api("/api"));
        assert!(est_api("/api/cabinets"));
        assert!(!est_api("/apiculture"));
        assert!(!est_api("/index.html"));
    }
}
