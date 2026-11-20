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

use crate::base::brouillard::RefusSaisie;

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
    /// 409 : action refusée par une règle métier, sur une ressource visible : motif stable, et,
    /// pour certains motifs, un nombre (jamais une donnée d'autrui).
    Conflit {
        /// Motif, code du corps.
        motif: MotifConflit,
        /// Nombre cité par le motif (écritures du brouillard concernées).
        nombre: Option<u32>,
    },
    /// 413 : corps trop grand.
    CorpsTropGrand,
    /// 415 : type de fichier non admis (pièce justificative reconnue par sa signature,
    /// ADR 0015).
    TypeNonAdmis,
    /// 422 : facteur ou mot de passe refusé à l'enrôlement ou à la ré-authentification
    /// (méthode de secours, PR 8b) ; le motif appartient à une liste fermée.
    Refus(MotifRefus),
    /// 422 : saisie du brouillard invalide en soi (compte, journal, tiers, étiquette fiscale…) ;
    /// motif stable, et position (à partir de 1) de la ligne concernée s'il y en a une.
    Saisie {
        /// Motif, code du corps.
        refus: RefusSaisie,
        /// Ligne concernée.
        ligne: Option<u32>,
    },
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

/// Motif d'un refus de facteur (422), rendu au client pour qu'il corrige sa saisie. Aucun ne
/// dit lequel du mot de passe ou du code était faux à la ré-authentification
/// ([`MotifRefus::FacteursRefuses`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MotifRefus {
    /// Mot de passe de moins de 12 caractères (ADR 0012).
    MotDePasseTropCourt,
    /// Mot de passe trop long (borne du coût du hachage).
    MotDePasseTropLong,
    /// Mot de passe présent dans la liste des mots de passe compromis.
    MotDePasseCompromis,
    /// Code TOTP faux ou déjà utilisé, à l'enrôlement.
    CodeInvalide,
    /// Jeton d'enrôlement invalide, d'une autre session ou expiré.
    JetonEnrolementInvalide,
    /// Ré-authentification refusée (mot de passe ou code).
    FacteursRefuses,
}

impl MotifRefus {
    /// Code du corps de la réponse.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::MotDePasseTropCourt => "mot_de_passe_trop_court",
            Self::MotDePasseTropLong => "mot_de_passe_trop_long",
            Self::MotDePasseCompromis => "mot_de_passe_compromis",
            Self::CodeInvalide => "code_invalide",
            Self::JetonEnrolementInvalide => "jeton_enrolement_invalide",
            Self::FacteursRefuses => "facteurs_refuses",
        }
    }
}

/// Motif d'un refus 409 (liste fermée, codes stables pour l'interface).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MotifConflit {
    /// La période est déjà clôturée.
    PeriodeDejaCloturee,
    /// Verrouillage d'une période qui n'est pas ouverte.
    PeriodeNonOuverte,
    /// Déverrouillage d'une période qui n'est pas verrouillée.
    PeriodeNonVerrouillee,
    /// Dernière période d'un exercice : elle se clôture avec lui.
    DernierePeriode,
    /// Une période antérieure n'est pas clôturée.
    PeriodeAnterieureNonCloturee,
    /// Écritures du brouillard datées dans la période (avec leur nombre).
    BrouillardNonValide,
    /// Deux exercices sont déjà non clôturés.
    ExercicesNonCloturesMax,
    /// Brouillon modifié depuis que le client l'a lu (verrou optimiste).
    BrouillonPerime,
    /// Date comptable dans une période ou un exercice clôturé.
    DateDansPeriodeCloturee,
    /// Exercice en clôture, journal non admis.
    JournalNonAdmisEnCloture,
    /// Modification qui ne change rien.
    AucunChangement,
    /// Drapeau « à revoir » déjà dans l'état demandé.
    DrapeauInchange,
    /// Pièce écartée : elle ne se lie pas.
    PieceEcartee,
    /// Pièce déjà liée au brouillon.
    PieceDejaLiee,
    /// Le brouillon a déjà une pièce principale.
    PrincipaleExistante,
    /// Pièce liée à un brouillon : elle ne s'écarte pas.
    PieceLiee,
    /// Le drapeau « à revoir » ne se lève pas par celui qui l'a posé.
    RevueParLePoseur,
    /// Trop de pièces liées au brouillon.
    TropDePieces,
}

impl MotifConflit {
    /// Tous les motifs.
    pub const TOUS: [Self; 18] = [
        Self::PeriodeDejaCloturee,
        Self::PeriodeNonOuverte,
        Self::PeriodeNonVerrouillee,
        Self::DernierePeriode,
        Self::PeriodeAnterieureNonCloturee,
        Self::BrouillardNonValide,
        Self::ExercicesNonCloturesMax,
        Self::BrouillonPerime,
        Self::DateDansPeriodeCloturee,
        Self::JournalNonAdmisEnCloture,
        Self::AucunChangement,
        Self::DrapeauInchange,
        Self::PieceEcartee,
        Self::PieceDejaLiee,
        Self::PrincipaleExistante,
        Self::PieceLiee,
        Self::RevueParLePoseur,
        Self::TropDePieces,
    ];

    /// Code du corps de la réponse.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::PeriodeDejaCloturee => "periode_deja_cloturee",
            Self::PeriodeNonOuverte => "periode_non_ouverte",
            Self::PeriodeNonVerrouillee => "periode_non_verrouillee",
            Self::DernierePeriode => "derniere_periode",
            Self::PeriodeAnterieureNonCloturee => "periode_anterieure_non_cloturee",
            Self::BrouillardNonValide => "brouillard_non_valide",
            Self::ExercicesNonCloturesMax => "exercices_non_clotures_max",
            Self::BrouillonPerime => "brouillon_perime",
            Self::DateDansPeriodeCloturee => "date_dans_periode_cloturee",
            Self::JournalNonAdmisEnCloture => "journal_non_admis_en_cloture",
            Self::AucunChangement => "aucun_changement",
            Self::DrapeauInchange => "drapeau_inchange",
            Self::PieceEcartee => "piece_ecartee",
            Self::PieceDejaLiee => "piece_deja_liee",
            Self::PrincipaleExistante => "piece_principale_existante",
            Self::PieceLiee => "piece_liee",
            Self::RevueParLePoseur => "revue_par_le_poseur",
            Self::TropDePieces => "trop_de_pieces",
        }
    }
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
    /// Inscription au journal d'audit impossible, ou mutation sans trace : l'action est annulée.
    Audit,
    /// Stockage objet des pièces indisponible, non configuré, ou objet altéré (ADR 0015).
    Stockage,
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
            Self::Audit => "audit",
            Self::Stockage => "stockage",
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
            Self::Conflit { .. } => StatusCode::CONFLICT,
            Self::CorpsTropGrand => StatusCode::PAYLOAD_TOO_LARGE,
            Self::TypeNonAdmis => StatusCode::UNSUPPORTED_MEDIA_TYPE,
            Self::Refus(_) | Self::Saisie { .. } => StatusCode::UNPROCESSABLE_ENTITY,
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
            Self::Conflit { motif, .. } => motif.code(),
            Self::CorpsTropGrand => "corps_trop_grand",
            Self::TypeNonAdmis => "type_non_admis",
            Self::Refus(motif) => motif.code(),
            Self::Saisie { refus, .. } => refus.code(),
            Self::TropDeRequetes { .. } => "trop_de_requetes",
            Self::Interne(_) => "erreur_interne",
            Self::DelaiDepasse => "delai_depasse",
        }
    }
}

impl From<crate::base::ErreurBase> for ErreurApi {
    fn from(erreur: crate::base::ErreurBase) -> Self {
        // Le détail (message de PostgreSQL) est volontairement abandonné : il peut citer des
        // valeurs (F2.11). La catégorie suffit au diagnostic, avec l'identifiant de requête.
        match erreur {
            crate::base::ErreurBase::Audit(_) => Self::Interne(CategorieErreur::Audit),
            crate::base::ErreurBase::Donnee(_) => Self::Interne(CategorieErreur::Donnee),
            _ => Self::Interne(CategorieErreur::Base),
        }
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
        StatusCode::UNSUPPORTED_MEDIA_TYPE => ErreurApi::TypeNonAdmis,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    nombre: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    ligne: Option<u32>,
}

impl IntoResponse for ErreurApi {
    fn into_response(self) -> Response {
        let mut reponse = (
            self.statut(),
            Json(Corps {
                erreur: self.code(),
                nombre: match self {
                    Self::Conflit { nombre, .. } => nombre,
                    _ => None,
                },
                ligne: match self {
                    Self::Saisie { ligne, .. } => ligne,
                    _ => None,
                },
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
        let mut toutes = vec![
            ErreurApi::RequeteInvalide,
            ErreurApi::NonAuthentifie,
            ErreurApi::Interdit,
            ErreurApi::Introuvable,
            ErreurApi::MethodeNonAdmise,
            ErreurApi::CorpsTropGrand,
            ErreurApi::TypeNonAdmis,
            ErreurApi::Refus(MotifRefus::MotDePasseTropCourt),
            ErreurApi::Refus(MotifRefus::MotDePasseTropLong),
            ErreurApi::Refus(MotifRefus::MotDePasseCompromis),
            ErreurApi::Refus(MotifRefus::CodeInvalide),
            ErreurApi::Refus(MotifRefus::JetonEnrolementInvalide),
            ErreurApi::Refus(MotifRefus::FacteursRefuses),
            ErreurApi::TropDeRequetes { reessayer_dans: 1 },
            ErreurApi::Interne(CategorieErreur::Base),
            ErreurApi::DelaiDepasse,
        ];
        toutes.extend(RefusSaisie::TOUS.map(|refus| ErreurApi::Saisie { refus, ligne: None }));
        toutes.extend(MotifConflit::TOUS.map(|motif| ErreurApi::Conflit {
            motif,
            nombre: None,
        }));
        let codes: std::collections::HashSet<_> = toutes.iter().map(|e| e.code()).collect();
        assert_eq!(codes.len(), toutes.len());
        assert!(
            codes
                .iter()
                .all(|c| c.bytes().all(|o| o.is_ascii_lowercase() || o == b'_'))
        );
    }

    #[tokio::test]
    async fn un_conflit_rend_son_motif_et_son_nombre_seulement() {
        for (erreur, attendu) in [
            (
                ErreurApi::Conflit {
                    motif: MotifConflit::BrouillardNonValide,
                    nombre: Some(3),
                },
                r#"{"erreur":"brouillard_non_valide","nombre":3}"#,
            ),
            (
                ErreurApi::Conflit {
                    motif: MotifConflit::DernierePeriode,
                    nombre: None,
                },
                r#"{"erreur":"derniere_periode"}"#,
            ),
        ] {
            let reponse = erreur.into_response();
            assert_eq!(reponse.status(), StatusCode::CONFLICT);
            let corps = axum::body::to_bytes(reponse.into_body(), 1024)
                .await
                .unwrap();
            assert_eq!(corps, attendu.as_bytes());
        }
    }
}
