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
    /// 409 ou 422 : validation d'une écriture refusée par `compta-core` ou par un contrôle de
    /// forme (PR 13b) ; motif stable, et ligne concernée (à partir de 1) s'il y en a une.
    Validation {
        /// Motif, code du corps (le statut en dépend).
        motif: MotifValidation,
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
    /// Lien d'invitation ou de réinitialisation inconnu, expiré, fermé ou sans objet : une seule
    /// réponse pour tous ces cas.
    LienInvalide,
    /// Passkey demandée par un compte qui n'en a aucune (ré-authentification).
    AucunePasskey,
    /// Cérémonie d'enrôlement d'une passkey refusée : défi absent, expiré ou déjà utilisé,
    /// réponse invalide, nom invalide.
    PasskeyRefusee,
    /// Cet authentificateur est déjà enregistré (WebAuthn : un identifiant de credential ne
    /// s'enregistre qu'une fois).
    PasskeyDejaEnregistree,
    /// Nombre maximal de passkeys atteint.
    PasskeysMax,
    /// Suppression refusée : ce serait le dernier facteur fort du compte (F1.1).
    DernierFacteur,
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
            Self::LienInvalide => "lien_invalide",
            Self::AucunePasskey => "aucune_passkey",
            Self::PasskeyRefusee => "passkey_refusee",
            Self::PasskeyDejaEnregistree => "passkey_deja_enregistree",
            Self::PasskeysMax => "passkeys_max",
            Self::DernierFacteur => "dernier_facteur",
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
    /// Validation par lot sans aucune écriture à valider (toutes « à revoir »).
    LotSansEcritureAValider,
    /// Édition au-delà du nombre d'écritures traitables en une requête : restreindre la période.
    EditionTropVolumineuse,
    /// Le courriel invité est déjà celui d'un membre du cabinet.
    DejaMembre,
    /// Une invitation est déjà en attente pour ce courriel.
    InvitationEnCours,
    /// L'invitation n'est plus en attente.
    InvitationNonEnAttente,
    /// Action refusée sur soi-même (rôle, suspension, réinitialisation).
    ActionSurSoiMeme,
    /// Ce serait retirer le dernier administrateur actif du cabinet.
    DernierAdministrateur,
    /// Une réinitialisation est déjà en attente pour ce membre.
    ReinitialisationEnCours,
    /// La réinitialisation n'est plus en attente.
    ReinitialisationNonEnAttente,
    /// Un compte existe déjà pour ce courriel : il accepte l'invitation une fois connecté.
    CompteExistant,
    /// Une ligne étiquetée « taxe » ne porte pas le compte de TVA de son sens, ou une base porte un
    /// compte de TVA (ADR 0026, validation unitaire et par lot).
    EtiquetteTvaIncoherente,
    /// Le total des étiquettes « taxe » diffère du solde des comptes de TVA de l'écriture
    /// (ADR 0026, validation unitaire et par lot).
    EcartTvaEtiquettes,
    /// Le membre appartient aussi à d'autres cabinets : sa réinitialisation retirerait ses accès
    /// à des cabinets que l'administrateur n'administre pas (ADR 0023, §4).
    MembreDUnAutreCabinet,
}

impl MotifConflit {
    /// Tous les motifs.
    pub const TOUS: [Self; 31] = [
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
        Self::LotSansEcritureAValider,
        Self::EditionTropVolumineuse,
        Self::DejaMembre,
        Self::InvitationEnCours,
        Self::InvitationNonEnAttente,
        Self::ActionSurSoiMeme,
        Self::DernierAdministrateur,
        Self::ReinitialisationEnCours,
        Self::ReinitialisationNonEnAttente,
        Self::CompteExistant,
        Self::MembreDUnAutreCabinet,
        Self::EtiquetteTvaIncoherente,
        Self::EcartTvaEtiquettes,
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
            Self::LotSansEcritureAValider => "lot_sans_ecriture_a_valider",
            Self::EditionTropVolumineuse => "edition_trop_volumineuse",
            Self::DejaMembre => "deja_membre",
            Self::InvitationEnCours => "invitation_en_cours",
            Self::InvitationNonEnAttente => "invitation_non_en_attente",
            Self::ActionSurSoiMeme => "action_sur_soi_meme",
            Self::DernierAdministrateur => "dernier_administrateur",
            Self::ReinitialisationEnCours => "reinitialisation_en_cours",
            Self::ReinitialisationNonEnAttente => "reinitialisation_non_en_attente",
            Self::CompteExistant => "compte_existant",
            Self::MembreDUnAutreCabinet => "membre_d_un_autre_cabinet",
            Self::EtiquetteTvaIncoherente => "etiquette_tva_incoherente",
            Self::EcartTvaEtiquettes => "ecart_tva_etiquettes",
        }
    }
}

/// Motif d'un refus de validation (liste fermée, un code stable par motif de `compta-core` ;
/// PR 13b). 409 : l'état du dossier (calendrier, séquence) s'oppose à la validation ; 422 :
/// l'écriture elle-même est invalide.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MotifValidation {
    /// Période verrouillée (réversible).
    PeriodeVerrouillee,
    /// Période clôturée.
    PeriodeCloturee,
    /// Échéance de la clôture informatique dépassée.
    EcheanceDeCloture,
    /// Exercice clôturé.
    ExerciceCloture,
    /// Exercice en clôture, journal non admis.
    ExerciceEnCloture,
    /// Date de valeur comptable dans une période ouverte.
    DateValeurPeriodeOuverte,
    /// Date de valeur comptable dans une période verrouillée.
    DateValeurPeriodeVerrouillee,
    /// Date de valeur comptable dans une période dont l'échéance est passée.
    DateValeurPeriodeEchue,
    /// Période qui doit accueillir l'opération tardive verrouillée.
    PeriodeAccueilVerrouillee,
    /// Opération tardive mal datée.
    OperationTardiveMalDatee,
    /// Séquence du journal épuisée.
    SequenceEpuisee,
    /// ∑ débits ≠ ∑ crédits.
    Desequilibree,
    /// Une seule ligne.
    UneSeuleLigne,
    /// Aucune ligne.
    Vide,
    /// Compte inconnu du plan.
    CompteInconnu,
    /// Compte désactivé.
    CompteInactif,
    /// Compte de regroupement.
    CompteDeRegroupement,
    /// Ligne sur un compte collectif sans tiers.
    TiersManquant,
    /// Aucune pièce justificative.
    JustificationManquante,
    /// Journal de trésorerie sans mouvement de son compte.
    CompteTresorerieAbsent,
    /// Journal inconnu.
    JournalInconnu,
    /// Journal qui n'accepte pas la saisie.
    JournalSansSaisie,
    /// Date hors de tout exercice.
    DateHorsExercice,
    /// Date de valeur comptable non antérieure à la date comptable.
    DateValeurNonAnterieure,
    /// Montant dont l'échelle dépasse celle du format canonique.
    MontantEchelleExcessive,
    /// Autre invalidité de l'écriture (montant, texte).
    EcritureInvalide,
    /// Contenu qui ne se relit pas à l'identique du format canonique.
    NonRepresentable,
    /// Contre-passation datée avant l'écriture d'origine.
    DateAnterieureAOrigine,
}

impl MotifValidation {
    /// Tous les motifs.
    pub const TOUS: [Self; 28] = [
        Self::PeriodeVerrouillee,
        Self::PeriodeCloturee,
        Self::EcheanceDeCloture,
        Self::ExerciceCloture,
        Self::ExerciceEnCloture,
        Self::DateValeurPeriodeOuverte,
        Self::DateValeurPeriodeVerrouillee,
        Self::DateValeurPeriodeEchue,
        Self::PeriodeAccueilVerrouillee,
        Self::OperationTardiveMalDatee,
        Self::SequenceEpuisee,
        Self::Desequilibree,
        Self::UneSeuleLigne,
        Self::Vide,
        Self::CompteInconnu,
        Self::CompteInactif,
        Self::CompteDeRegroupement,
        Self::TiersManquant,
        Self::JustificationManquante,
        Self::CompteTresorerieAbsent,
        Self::JournalInconnu,
        Self::JournalSansSaisie,
        Self::DateHorsExercice,
        Self::DateValeurNonAnterieure,
        Self::MontantEchelleExcessive,
        Self::EcritureInvalide,
        Self::NonRepresentable,
        Self::DateAnterieureAOrigine,
    ];

    /// Statut HTTP : 409 pour l'état du dossier, 422 pour l'écriture invalide.
    #[must_use]
    pub const fn statut(self) -> StatusCode {
        match self {
            Self::PeriodeVerrouillee
            | Self::PeriodeCloturee
            | Self::EcheanceDeCloture
            | Self::ExerciceCloture
            | Self::ExerciceEnCloture
            | Self::DateValeurPeriodeOuverte
            | Self::DateValeurPeriodeVerrouillee
            | Self::DateValeurPeriodeEchue
            | Self::PeriodeAccueilVerrouillee
            | Self::OperationTardiveMalDatee
            | Self::SequenceEpuisee => StatusCode::CONFLICT,
            _ => StatusCode::UNPROCESSABLE_ENTITY,
        }
    }

    /// Code du corps de la réponse.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::PeriodeVerrouillee => "periode_verrouillee",
            Self::PeriodeCloturee => "periode_cloturee",
            Self::EcheanceDeCloture => "echeance_de_cloture_depassee",
            Self::ExerciceCloture => "exercice_cloture",
            Self::ExerciceEnCloture => "exercice_en_cloture",
            Self::DateValeurPeriodeOuverte => "date_valeur_dans_periode_ouverte",
            Self::DateValeurPeriodeVerrouillee => "date_valeur_dans_periode_verrouillee",
            Self::DateValeurPeriodeEchue => "date_valeur_dans_periode_echue",
            Self::PeriodeAccueilVerrouillee => "periode_accueil_verrouillee",
            Self::OperationTardiveMalDatee => "operation_tardive_mal_datee",
            Self::SequenceEpuisee => "sequence_epuisee",
            Self::Desequilibree => "ecriture_desequilibree",
            Self::UneSeuleLigne => "ecriture_une_seule_ligne",
            Self::Vide => "ecriture_vide",
            Self::CompteInconnu => "validation_compte_inconnu",
            Self::CompteInactif => "validation_compte_inactif",
            Self::CompteDeRegroupement => "validation_compte_de_regroupement",
            Self::TiersManquant => "tiers_manquant",
            Self::JustificationManquante => "justification_manquante",
            Self::CompteTresorerieAbsent => "compte_tresorerie_absent",
            Self::JournalInconnu => "validation_journal_inconnu",
            Self::JournalSansSaisie => "validation_journal_sans_saisie",
            Self::DateHorsExercice => "validation_date_hors_exercice",
            Self::DateValeurNonAnterieure => "validation_date_valeur_non_anterieure",
            Self::MontantEchelleExcessive => "montant_echelle_excessive",
            Self::EcritureInvalide => "ecriture_invalide",
            Self::NonRepresentable => "ecriture_non_representable",
            Self::DateAnterieureAOrigine => "date_anterieure_a_l_origine",
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
            Self::Validation { motif, .. } => motif.statut(),
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
            Self::Validation { motif, .. } => motif.code(),
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

/// Écriture d'un lot refusée : son identifiant (celui du corps de la requête) et le code stable
/// du motif (PR 13c).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefusLot {
    /// Identifiant de l'écriture, tel que la requête le cite.
    pub ecriture: uuid::Uuid,
    /// Code stable du motif (liste fermée : [`MotifConflit`], [`MotifValidation`]).
    pub code: &'static str,
}

#[derive(Serialize)]
struct VueRefusLot {
    ecriture: String,
    code: &'static str,
}

#[derive(Serialize)]
struct CorpsLot {
    erreur: &'static str,
    refus: Vec<VueRefusLot>,
}

/// Réponse 409 d'une validation par lot refusée : le code `lot_refuse` et la liste des écritures
/// refusées avec leur motif. À rendre **sans trace** : le pipeline annule la transaction de toute
/// réponse d'erreur du cas d'usage (rien n'est écrit, ADR 0011).
#[must_use]
pub fn reponse_lot_refuse(refus: &[RefusLot]) -> Response {
    let mut reponse = (
        StatusCode::CONFLICT,
        Json(CorpsLot {
            erreur: "lot_refuse",
            refus: refus
                .iter()
                .map(|r| VueRefusLot {
                    ecriture: r.ecriture.to_string(),
                    code: r.code,
                })
                .collect(),
        }),
    )
        .into_response();
    reponse.extensions_mut().insert(CorpsUniforme);
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
                    Self::Saisie { ligne, .. } | Self::Validation { ligne, .. } => ligne,
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
            ErreurApi::Refus(MotifRefus::LienInvalide),
            ErreurApi::Refus(MotifRefus::FacteursRefuses),
            ErreurApi::Refus(MotifRefus::AucunePasskey),
            ErreurApi::Refus(MotifRefus::PasskeyRefusee),
            ErreurApi::Refus(MotifRefus::PasskeyDejaEnregistree),
            ErreurApi::Refus(MotifRefus::PasskeysMax),
            ErreurApi::Refus(MotifRefus::DernierFacteur),
            ErreurApi::TropDeRequetes { reessayer_dans: 1 },
            ErreurApi::Interne(CategorieErreur::Base),
            ErreurApi::DelaiDepasse,
        ];
        toutes.extend(RefusSaisie::TOUS.map(|refus| ErreurApi::Saisie { refus, ligne: None }));
        toutes.extend(
            MotifValidation::TOUS.map(|motif| ErreurApi::Validation { motif, ligne: None }),
        );
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
