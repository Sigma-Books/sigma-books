//! Erreurs uniformes de l'API (F1.8, F2.11 ; ASVS 4.0.3 V7.4.1).
//!
//! Un seul format de corps, `{"erreur":"<code>"}`, dont le code appartient à une liste fermée.
//! Aucun détail interne n'en sort : ni SQL, ni décision Cedar, ni pile, ni écho de l'entrée.
//! Une ressource d'autrui et une ressource inexistante donnent **le même** 404, octet pour octet
//! (ADR 0011). L'identifiant de la requête, qui relie la réponse aux journaux techniques, part
//! dans l'en-tête `x-sigma-requete`, jamais dans le corps : deux 404 restent identiques.

use axum::Json;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::Serialize;

/// Erreur rendue au client.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErreurApi {
    /// 400 : requête mal formée (UUID invalide dans le chemin…), sans écho de l'entrée.
    RequeteInvalide,
    /// 401 : aucune authentification valide.
    NonAuthentifie,
    /// 403 : action refusée sur une ressource visible, ou mutation refusée par l'anti-CSRF.
    Interdit,
    /// 404 : ressource absente **ou d'autrui** (autre cabinet, dossier non affecté).
    Introuvable,
    /// 405 : méthode non admise sur la ressource.
    MethodeNonAdmise,
    /// 413 : corps trop grand.
    CorpsTropGrand,
    /// 429 : limite de débit atteinte ; délai en secondes avant de réessayer.
    TropDeRequetes {
        /// Valeur de `Retry-After`, en secondes.
        reessayer_dans: u64,
    },
    /// 500 : erreur interne. La catégorie va aux journaux techniques, jamais au client.
    Interne(CategorieErreur),
    /// 503 : délai de traitement dépassé.
    DelaiDepasse,
}

/// Catégorie d'une erreur interne, pour les journaux techniques (liste fermée : aucun message
/// d'erreur de PostgreSQL n'y est recopié, car il peut contenir des valeurs, F2.11).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CategorieErreur {
    /// Erreur d'accès à la base.
    Base,
    /// Donnée lue en base incohérente (rôle inconnu, identifiant réservé).
    Donnee,
    /// Échec de l'authentification elle-même (pas un refus).
    Authentification,
    /// Panique capturée (le message n'est jamais recopié).
    Panique,
    /// Erreur d'une couche (`axum`, `tower-http`) remplacée par le corps uniforme.
    Couche,
}

impl CategorieErreur {
    /// Nom inscrit au champ `erreur` des journaux.
    #[must_use]
    pub const fn nom(self) -> &'static str {
        match self {
            Self::Base => "base",
            Self::Donnee => "donnee",
            Self::Authentification => "authentification",
            Self::Panique => "panique",
            Self::Couche => "couche",
        }
    }
}

impl ErreurApi {
    /// Statut HTTP.
    #[must_use]
    pub const fn statut(self) -> StatusCode {
        match self {
            Self::RequeteInvalide => StatusCode::BAD_REQUEST,
            Self::NonAuthentifie => StatusCode::UNAUTHORIZED,
            Self::Interdit => StatusCode::FORBIDDEN,
            Self::Introuvable => StatusCode::NOT_FOUND,
            Self::MethodeNonAdmise => StatusCode::METHOD_NOT_ALLOWED,
            Self::CorpsTropGrand => StatusCode::PAYLOAD_TOO_LARGE,
            Self::TropDeRequetes { .. } => StatusCode::TOO_MANY_REQUESTS,
            Self::Interne(_) => StatusCode::INTERNAL_SERVER_ERROR,
            Self::DelaiDepasse => StatusCode::SERVICE_UNAVAILABLE,
        }
    }

    /// Code du corps de la réponse (liste fermée).
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::RequeteInvalide => "requete_invalide",
            Self::NonAuthentifie => "non_authentifie",
            Self::Interdit => "interdit",
            Self::Introuvable => "introuvable",
            Self::MethodeNonAdmise => "methode_non_admise",
            Self::CorpsTropGrand => "corps_trop_grand",
            Self::TropDeRequetes { .. } => "trop_de_requetes",
            Self::Interne(_) => "erreur_interne",
            Self::DelaiDepasse => "delai_depasse",
        }
    }
}

impl From<crate::base::ErreurBase> for ErreurApi {
    fn from(_: crate::base::ErreurBase) -> Self {
        // Le détail (message de PostgreSQL) est volontairement abandonné : il peut citer des
        // valeurs (F2.11). La catégorie suffit au diagnostic, avec l'identifiant de requête.
        Self::Interne(CategorieErreur::Base)
    }
}

/// Marque d'une réponse au corps uniforme, posée par [`ErreurApi`] : les autres réponses
/// d'erreur sont réécrites par l'intergiciel `uniformiser`.
#[derive(Debug, Clone, Copy)]
pub(crate) struct CorpsUniforme;

/// Réponse uniforme pour un statut d'erreur produit hors d'[`ErreurApi`] : l'erreur
/// correspondante si elle existe, sinon le statut d'origine avec le code générique de sa classe.
pub(crate) fn pour_statut(statut: StatusCode) -> Response {
    let erreur = match statut {
        StatusCode::BAD_REQUEST => ErreurApi::RequeteInvalide,
        StatusCode::UNAUTHORIZED => ErreurApi::NonAuthentifie,
        StatusCode::FORBIDDEN => ErreurApi::Interdit,
        StatusCode::NOT_FOUND => ErreurApi::Introuvable,
        StatusCode::METHOD_NOT_ALLOWED => ErreurApi::MethodeNonAdmise,
        StatusCode::PAYLOAD_TOO_LARGE => ErreurApi::CorpsTropGrand,
        StatusCode::SERVICE_UNAVAILABLE => ErreurApi::DelaiDepasse,
        s if s.is_server_error() => ErreurApi::Interne(CategorieErreur::Couche),
        _ => ErreurApi::RequeteInvalide,
    };
    let mut reponse = erreur.into_response();
    *reponse.status_mut() = statut;
    reponse
}

#[derive(Serialize)]
struct Corps {
    erreur: &'static str,
}

impl IntoResponse for ErreurApi {
    fn into_response(self) -> Response {
        let mut reponse = (
            self.statut(),
            Json(Corps {
                erreur: self.code(),
            }),
        )
            .into_response();
        if let Self::TropDeRequetes { reessayer_dans } = self {
            reponse
                .headers_mut()
                .insert(header::RETRY_AFTER, HeaderValue::from(reessayer_dans));
        }
        if let Self::Interne(categorie) = self {
            reponse.extensions_mut().insert(categorie);
        }
        reponse.extensions_mut().insert(CorpsUniforme);
        reponse
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chaque_code_est_distinct_et_sans_detail() {
        let toutes = [
            ErreurApi::RequeteInvalide,
            ErreurApi::NonAuthentifie,
            ErreurApi::Interdit,
            ErreurApi::Introuvable,
            ErreurApi::MethodeNonAdmise,
            ErreurApi::CorpsTropGrand,
            ErreurApi::TropDeRequetes { reessayer_dans: 1 },
            ErreurApi::Interne(CategorieErreur::Base),
            ErreurApi::DelaiDepasse,
        ];
        let codes: std::collections::HashSet<_> = toutes.iter().map(|e| e.code()).collect();
        assert_eq!(codes.len(), toutes.len());
        assert!(
            codes
                .iter()
                .all(|c| c.bytes().all(|o| o.is_ascii_lowercase() || o == b'_'))
        );
    }
}
