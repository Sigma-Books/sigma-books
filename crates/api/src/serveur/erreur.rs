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
    /// 422 : relevé ou profil d'import refusé (L3-22 ; ADR 0034 point 6) : motif stable et, pour
    /// un fichier, ligne physique fautive (à partir de 1). Ni libellé, ni montant, ni valeur lue
    /// (F2.11).
    Releve {
        /// Motif, code du corps.
        motif: MotifReleve,
        /// Ligne physique du fichier.
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
    /// Le membre appartient aussi à d'autres cabinets : sa réinitialisation retirerait ses accès
    /// à des cabinets que l'administrateur n'administre pas (ADR 0023, §4).
    MembreDUnAutreCabinet,
    /// Le dossier a déjà une écriture validée : pays et devise de tenue sont figés (ADR 0027).
    DossierDejaEcritures,
    /// Le dossier a un brouillon : changer le pays ou la devise réinterpréterait ses montants.
    DossierBrouillardNonVide,
    /// Le dossier a des versions de paramètres fiscaux propres (table en ajout seul) : son pays
    /// ne change plus (ADR 0030).
    DossierParametresFiscauxPropres,
    /// Le dossier a des pièces : leur rétention est celle de son pays à la date du dépôt, le pays
    /// ne change plus (ADR 0030).
    DossierPiecesPresentes,
    /// La date d'effet d'une version propre tombe dans une période clôturée ou un exercice clos.
    ParametresDateEffetCloturee,
    /// La date d'effet n'est pas postérieure à celle de la dernière version propre du dossier.
    ParametresDateEffetAnterieure,
    /// Une écriture validée du dossier utilise un des régimes que la version retirerait.
    ParametresRegimeUtilise,
    /// Le dossier a déjà le nombre maximal de versions propres.
    ParametresTropDeVersions,
    /// La devise de tenue du dossier a changé pendant l'attente du verrou : réessayer.
    ContexteDossierModifie,
    /// Brouillon saisi sous une autre devise de tenue ou une autre échelle que celles du dossier.
    BrouillonDeviseTenuePerimee,
    /// Compte de trésorerie archivé : aucun arrêté de caisse (L3-21).
    CompteTresorerieArchive,
    /// Journal ou compte déjà lié à un compte de trésorerie, ou journal qui n'est pas de
    /// trésorerie (L3-21).
    JournalDejaTresorerie,
    /// Plus aucun numéro de sous-compte ou code de journal libre pour la création assistée
    /// (`01` à `99`) : fournir le segment ou le code (L3-21).
    NumerotationTresorerieEpuisee,
    /// Le dossier a des arrêtés de caisse : sa devise de tenue ne change plus (L3-21).
    DossierArretesDeCaisse,
    /// Type demandé autre que celui du compte du journal repris (L3-21).
    TresorerieTypeIncoherent,
    /// Code de tiers déjà pris par un autre tiers du dossier (unique, ADR 0031 point 1).
    TiersCodeDejaUtilise,
    /// Aucun code ne peut être tiré d'office (suite épuisée par un code saisi) : le code est à
    /// saisir.
    TiersCodeTirageImpossible,
    /// Type ou compte collectif d'un tiers déjà cité par une ligne (brouillon ou écriture
    /// validée) : figés dès le premier mouvement (ADR 0031 point 1, amendement du 2026-11-27).
    TiersDejaMouvemente,
    /// Nombre maximal de profils d'import de relevé du dossier atteint (ADR 0034 point 6 : 20,
    /// réglage d'instance).
    ProfilsReleveMax,
    /// Nombre maximal de versions d'un profil d'import atteint (50, comme les versions de
    /// paramètres fiscaux, ADR 0030).
    ProfilVersionsMax,
    /// Code de condition de règlement déjà pris dans le dossier (L3-17 ; ADR 0033 point 3).
    ConditionReglementCodeDejaUtilise,
    /// Révision d'une échéance dont le résiduel est nul (ADR 0033 point 2).
    EcheanceSoldee,
    /// Révision concurrente de la même ligne (rang déjà pris) : relire puis réessayer (L3-17).
    EcheanceRevisionConcurrente,
    /// Dé-lettrage refusé : la date de l'opération de lettrage tombe dans un exercice clos (ADR
    /// 0032 point 5, B2 option 1 ; spec §10.1).
    LettrageExerciceClos,
    /// Lettrage déjà rompu (au plus une rupture par lettrage ; re-lettrer crée un lettrage).
    LettrageDejaRompu,
    /// Ligne déjà entièrement lettrée (résiduel nul).
    LettrageLigneSoldee,
    /// Rupture datée avant le lettrage (jour du dossier antérieur à celui du lettrage).
    LettrageRuptureAnterieure,
}

impl MotifConflit {
    /// Tous les motifs.
    pub const TOUS: [Self; 56] = [
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
        Self::DossierDejaEcritures,
        Self::DossierBrouillardNonVide,
        Self::DossierParametresFiscauxPropres,
        Self::DossierPiecesPresentes,
        Self::ParametresDateEffetCloturee,
        Self::ParametresDateEffetAnterieure,
        Self::ParametresRegimeUtilise,
        Self::ParametresTropDeVersions,
        Self::ContexteDossierModifie,
        Self::BrouillonDeviseTenuePerimee,
        Self::CompteTresorerieArchive,
        Self::JournalDejaTresorerie,
        Self::NumerotationTresorerieEpuisee,
        Self::DossierArretesDeCaisse,
        Self::TresorerieTypeIncoherent,
        Self::TiersCodeDejaUtilise,
        Self::TiersCodeTirageImpossible,
        Self::TiersDejaMouvemente,
        Self::ProfilsReleveMax,
        Self::ProfilVersionsMax,
        Self::ConditionReglementCodeDejaUtilise,
        Self::EcheanceSoldee,
        Self::EcheanceRevisionConcurrente,
        Self::LettrageExerciceClos,
        Self::LettrageDejaRompu,
        Self::LettrageLigneSoldee,
        Self::LettrageRuptureAnterieure,
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
            Self::DossierDejaEcritures => "dossier_deja_ecritures",
            Self::DossierParametresFiscauxPropres => "dossier_parametres_fiscaux_propres",
            Self::DossierPiecesPresentes => "dossier_pieces_presentes",
            Self::ParametresDateEffetCloturee => "parametres_date_effet_cloturee",
            Self::ParametresDateEffetAnterieure => "parametres_date_effet_anterieure",
            Self::ParametresRegimeUtilise => "parametres_regime_utilise",
            Self::ParametresTropDeVersions => "parametres_trop_de_versions",
            Self::DossierBrouillardNonVide => "dossier_brouillard_non_vide",
            Self::ContexteDossierModifie => "contexte_dossier_modifie",
            Self::BrouillonDeviseTenuePerimee => "brouillon_devise_tenue_perimee",
            Self::CompteTresorerieArchive => "compte_tresorerie_archive",
            Self::JournalDejaTresorerie => "journal_deja_tresorerie",
            Self::NumerotationTresorerieEpuisee => "numerotation_tresorerie_epuisee",
            Self::DossierArretesDeCaisse => "dossier_arretes_de_caisse",
            Self::TresorerieTypeIncoherent => "tresorerie_type_incoherent",
            Self::TiersCodeDejaUtilise => "tiers_code_deja_utilise",
            Self::TiersCodeTirageImpossible => "tiers_code_tirage_impossible",
            Self::TiersDejaMouvemente => "tiers_deja_mouvemente",
            Self::ProfilsReleveMax => "releve_profils_max",
            Self::ProfilVersionsMax => "releve_profil_versions_max",
            Self::ConditionReglementCodeDejaUtilise => "condition_reglement_code_deja_utilise",
            Self::EcheanceSoldee => "echeance_soldee",
            Self::EcheanceRevisionConcurrente => "echeance_revision_concurrente",
            Self::LettrageExerciceClos => "lettrage_exercice_clos",
            Self::LettrageDejaRompu => "lettrage_deja_rompu",
            Self::LettrageLigneSoldee => "lettrage_ligne_soldee",
            Self::LettrageRuptureAnterieure => "lettrage_rupture_anterieure",
        }
    }
}

/// Motif d'un refus de relevé ou de profil d'import (422 ; L3-22, ADR 0034 point 6). Les motifs
/// d'un fichier reprennent ceux de `formats::releve::ErreurReleve`, préfixés `releve_` ; ceux d'un
/// profil, les refus de `formats::releve::ErreurProfil`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MotifReleve {
    /// Fichier plus grand que la limite d'analyse (limite abaissée après le dépôt).
    FichierTropGrand,
    /// Octet nul.
    OctetNul,
    /// Octets non décodables dans l'encodage du profil.
    EncodageInvalide,
    /// Caractère de contrôle hors tabulation et fins de ligne.
    CaractereDeControle,
    /// Ligne physique plus longue que la limite.
    LigneTropLongue,
    /// Plus de lignes que la limite.
    TropDeLignes,
    /// Plus de colonnes que la limite.
    TropDeColonnes,
    /// Cellule plus longue que la limite.
    CelluleTropLongue,
    /// Guillemet non fermé en fin de fichier (fichier tronqué).
    GuillemetNonFerme,
    /// Ligne de titres absente alors que le profil nomme ses colonnes.
    TitresAbsents,
    /// Titre de colonne du profil introuvable.
    TitreIntrouvable,
    /// Colonne du profil absente d'une ligne.
    ColonneAbsente,
    /// Date illisible avec le format du profil.
    DateInvalide,
    /// Nombre illisible avec les séparateurs du profil.
    NombreInvalide,
    /// Profil illisible (forme JSON).
    ProfilIllisible,
    /// Encodage hors de la liste fermée.
    ProfilEncodage,
    /// Délimiteur invalide.
    ProfilDelimiteur,
    /// Guillemet invalide.
    ProfilGuillemet,
    /// Séparateur décimal ou de milliers invalide.
    ProfilSeparateur,
    /// Format de date invalide ou trop long.
    ProfilFormatDate,
    /// Format de date sans année (OCA complète par l'horloge : non repris).
    ProfilDateSansAnnee,
    /// Colonnes de montant incohérentes (montant signé, ou débit et crédit).
    ProfilColonnesMontant,
    /// Titre de colonne vide, trop long, ou sans ligne d'en-tête.
    ProfilTitre,
    /// Profil trop grand.
    ProfilTropGrand,
    /// Nom de profil vide, trop long ou avec un caractère interdit.
    ProfilNom,
}

impl MotifReleve {
    /// Tous les motifs.
    pub const TOUS: [Self; 25] = [
        Self::FichierTropGrand,
        Self::OctetNul,
        Self::EncodageInvalide,
        Self::CaractereDeControle,
        Self::LigneTropLongue,
        Self::TropDeLignes,
        Self::TropDeColonnes,
        Self::CelluleTropLongue,
        Self::GuillemetNonFerme,
        Self::TitresAbsents,
        Self::TitreIntrouvable,
        Self::ColonneAbsente,
        Self::DateInvalide,
        Self::NombreInvalide,
        Self::ProfilIllisible,
        Self::ProfilEncodage,
        Self::ProfilDelimiteur,
        Self::ProfilGuillemet,
        Self::ProfilSeparateur,
        Self::ProfilFormatDate,
        Self::ProfilDateSansAnnee,
        Self::ProfilColonnesMontant,
        Self::ProfilTitre,
        Self::ProfilTropGrand,
        Self::ProfilNom,
    ];

    /// Code du corps de la réponse.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::FichierTropGrand => "releve_fichier_trop_grand",
            Self::OctetNul => "releve_octet_nul",
            Self::EncodageInvalide => "releve_encodage_invalide",
            Self::CaractereDeControle => "releve_caractere_de_controle",
            Self::LigneTropLongue => "releve_ligne_trop_longue",
            Self::TropDeLignes => "releve_trop_de_lignes",
            Self::TropDeColonnes => "releve_trop_de_colonnes",
            Self::CelluleTropLongue => "releve_cellule_trop_longue",
            Self::GuillemetNonFerme => "releve_guillemet_non_ferme",
            Self::TitresAbsents => "releve_titres_absents",
            Self::TitreIntrouvable => "releve_titre_introuvable",
            Self::ColonneAbsente => "releve_colonne_absente",
            Self::DateInvalide => "releve_date_invalide",
            Self::NombreInvalide => "releve_nombre_invalide",
            Self::ProfilIllisible => "releve_profil_illisible",
            Self::ProfilEncodage => "releve_profil_encodage",
            Self::ProfilDelimiteur => "releve_profil_delimiteur",
            Self::ProfilGuillemet => "releve_profil_guillemet",
            Self::ProfilSeparateur => "releve_profil_separateur",
            Self::ProfilFormatDate => "releve_profil_format_date",
            Self::ProfilDateSansAnnee => "releve_profil_date_sans_annee",
            Self::ProfilColonnesMontant => "releve_profil_colonnes_montant",
            Self::ProfilTitre => "releve_profil_titre",
            Self::ProfilTropGrand => "releve_profil_trop_grand",
            Self::ProfilNom => "releve_profil_nom",
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
    /// Exercice clôturé.
    ExerciceCloture,
    /// Exercice en clôture, journal non admis.
    ExerciceEnCloture,
    /// Date de valeur comptable dans une période ouverte.
    DateValeurPeriodeOuverte,
    /// Date de valeur comptable dans une période verrouillée.
    DateValeurPeriodeVerrouillee,
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
    /// Montant de ligne qui a plus de décimales que l'échelle de la devise de tenue du dossier.
    MontantEchelleTenue,
    /// Étiquette fiscale refusée par la version des paramètres fiscaux de la date comptable
    /// (régime, motif ou taux que cette version ne connaît pas ou que sa validité exclut).
    EtiquetteInvalide,
    /// Ligne étiquetée alors qu'aucune version des paramètres fiscaux du pays ne s'applique à la
    /// date comptable (ADR 0030).
    ParametresFiscauxAbsents,
    /// Autre invalidité de l'écriture (montant, texte).
    EcritureInvalide,
    /// Contenu qui ne se relit pas à l'identique du format canonique.
    NonRepresentable,
    /// Création d'un dossier : pays absent du pack pays (États de l'OHADA).
    PaysInconnu,
    /// Création d'un dossier : devise de tenue absente de la table des devises du pack.
    DeviseTenueInconnue,
    /// Création d'un dossier : fuseau absent de la table des fuseaux du pack.
    FuseauInconnu,
    /// Création d'un dossier : langue, forme juridique ou identifiant légal invalide.
    ContexteDossierInvalide,
    /// Le pays du dossier n'a aucun paquet de paramètres fiscaux : pas de version propre.
    ParametresPaysSansPaquet,
    /// Aucune version de base ne s'applique à la date d'effet.
    ParametresAucuneBase,
    /// La version propre ne change ni taux ni régime.
    ParametresAucunChangement,
    /// Taux hors de ]0 ; 100], non décimal ou non canonique.
    ParametresTauxInvalide,
    /// Rôle de taux absent de la version de base.
    ParametresRoleInconnu,
    /// Régime absent de la version de base.
    ParametresRegimeInconnu,
    /// Régime exonéré ou qui a des motifs : il n'est jamais retiré.
    ParametresRegimeNonRetirable,
    /// Motif vide, de plus de 400 octets ou avec un caractère de contrôle.
    ParametresMotifInvalide,
    /// Date de fin antérieure à la date d'effet.
    ParametresPeriodeInvalide,
    /// Date d'effet au-delà d'un an après le jour du dossier : la croissance stricte des dates et
    /// l'ajout seul rendraient toute surcharge ultérieure impossible.
    ParametresDateEffetLointaine,
    /// Compte de trésorerie dans une autre devise que celle de tenue : relève de L6 (ADR 0034
    /// point 1).
    TresorerieDeviseEtrangere,
    /// Arrêté de caisse demandé sur un compte de trésorerie qui n'est pas une caisse.
    TresoreriePasUneCaisse,
    /// Plafond ou compte d'écart sur un compte de trésorerie qui n'est pas une caisse, ou
    /// plafond non strictement positif.
    TresorerieParametreInvalide,
    /// Type d'identifiant légal d'un tiers hors du pack du pays du dossier, ou en double.
    TiersIdentifiantTypeInconnu,
    /// Type d'identifiant bancaire hors du pack.
    TiersCompteBancaireTypeInconnu,
    /// Valeur de la fiche d'un tiers invalide (texte vide ou trop long ; code, devise ou pays mal
    /// formé ; type de contact inconnu).
    TiersValeurInvalide,
    /// Caractère de contrôle ou de format Unicode (catégorie `Cf`) dans un texte de la fiche
    /// (F11.8).
    TiersCaractereInvalide,
    /// Plus d'identifiants légaux que la limite (un par type, dix au plus).
    TiersTropDIdentifiants,
    /// Plus d'adresses que la limite (vingt).
    TiersTropDAdresses,
    /// Plus de moyens de contact que la limite (cinquante).
    TiersTropDeContacts,
    /// Plus de coordonnées bancaires que la limite (vingt).
    TiersTropDeComptesBancaires,
    /// Condition de règlement invalide : code, libellé, terme (valeur, délai), dernier terme
    /// autre que le solde, pourcentages de 100 ou plus, plus de 50 termes (L3-17 ; ADR 0033
    /// point 3).
    ConditionReglementInvalide,
    /// Motif de révision d'échéance vide, de plus de 400 octets ou avec un caractère de contrôle
    /// ou de format (ADR 0033 point 2).
    EcheanceMotifInvalide,
    /// Lettrage : ligne inconnue du dossier (écriture validée absente, ou d'un autre dossier).
    LettrageLigneInconnue,
    /// Lettrage : lignes de comptes différents (un lettrage porte sur un seul compte).
    LettrageComptesDifferents,
    /// Lettrage : compte non marqué lettrable dans les données de `syscohada`.
    LettrageCompteNonLettrable,
    /// Lettrage : ligne citée deux fois.
    LettrageLigneEnDouble,
    /// Lettrage : aucune ligne au débit, ou aucune au crédit.
    LettrageSensManquant,
    /// Lettrage : lignes de tiers (résolus) différents (Tryton `move.py:676-682`).
    LettrageTiersDifferents,
    /// Lettrage en devise étrangère hors de L3 (écarts de change : L6, ADR 0032 point 1).
    LettrageDeviseEtrangere,
    /// Motif de rupture vide, trop long ou avec un caractère de contrôle ou de format.
    LettrageMotifInvalide,
    /// Lettrage de trop de lignes en un appel.
    LettrageTropDeLignes,
}

impl MotifValidation {
    /// Tous les motifs.
    pub const TOUS: [Self; 63] = [
        Self::PeriodeVerrouillee,
        Self::PeriodeCloturee,
        Self::ExerciceCloture,
        Self::ExerciceEnCloture,
        Self::DateValeurPeriodeOuverte,
        Self::DateValeurPeriodeVerrouillee,
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
        Self::JournalInconnu,
        Self::JournalSansSaisie,
        Self::DateHorsExercice,
        Self::DateValeurNonAnterieure,
        Self::MontantEchelleExcessive,
        Self::MontantEchelleTenue,
        Self::EtiquetteInvalide,
        Self::ParametresFiscauxAbsents,
        Self::EcritureInvalide,
        Self::NonRepresentable,
        Self::PaysInconnu,
        Self::DeviseTenueInconnue,
        Self::FuseauInconnu,
        Self::ContexteDossierInvalide,
        Self::ParametresPaysSansPaquet,
        Self::ParametresAucuneBase,
        Self::ParametresAucunChangement,
        Self::ParametresTauxInvalide,
        Self::ParametresRoleInconnu,
        Self::ParametresRegimeInconnu,
        Self::ParametresRegimeNonRetirable,
        Self::ParametresMotifInvalide,
        Self::ParametresPeriodeInvalide,
        Self::ParametresDateEffetLointaine,
        Self::TresorerieDeviseEtrangere,
        Self::TresoreriePasUneCaisse,
        Self::TresorerieParametreInvalide,
        Self::TiersIdentifiantTypeInconnu,
        Self::TiersCompteBancaireTypeInconnu,
        Self::TiersValeurInvalide,
        Self::TiersCaractereInvalide,
        Self::TiersTropDIdentifiants,
        Self::TiersTropDAdresses,
        Self::TiersTropDeContacts,
        Self::TiersTropDeComptesBancaires,
        Self::ConditionReglementInvalide,
        Self::EcheanceMotifInvalide,
        Self::LettrageLigneInconnue,
        Self::LettrageComptesDifferents,
        Self::LettrageCompteNonLettrable,
        Self::LettrageLigneEnDouble,
        Self::LettrageSensManquant,
        Self::LettrageTiersDifferents,
        Self::LettrageDeviseEtrangere,
        Self::LettrageMotifInvalide,
        Self::LettrageTropDeLignes,
    ];

    /// Statut HTTP : 409 pour l'état du dossier, 422 pour l'écriture invalide.
    #[must_use]
    pub const fn statut(self) -> StatusCode {
        match self {
            Self::PeriodeVerrouillee
            | Self::PeriodeCloturee
            | Self::ExerciceCloture
            | Self::ExerciceEnCloture
            | Self::DateValeurPeriodeOuverte
            | Self::DateValeurPeriodeVerrouillee
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
            Self::ExerciceCloture => "exercice_cloture",
            Self::ExerciceEnCloture => "exercice_en_cloture",
            Self::DateValeurPeriodeOuverte => "date_valeur_dans_periode_ouverte",
            Self::DateValeurPeriodeVerrouillee => "date_valeur_dans_periode_verrouillee",
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
            Self::JournalInconnu => "validation_journal_inconnu",
            Self::JournalSansSaisie => "validation_journal_sans_saisie",
            Self::DateHorsExercice => "validation_date_hors_exercice",
            Self::DateValeurNonAnterieure => "validation_date_valeur_non_anterieure",
            Self::MontantEchelleExcessive => "montant_echelle_excessive",
            Self::MontantEchelleTenue => "validation_montant_echelle_devise_tenue",
            Self::EtiquetteInvalide => "etiquette_invalide",
            Self::ParametresFiscauxAbsents => "validation_parametres_fiscaux_absents",
            Self::EcritureInvalide => "ecriture_invalide",
            Self::NonRepresentable => "ecriture_non_representable",
            Self::PaysInconnu => "pays_inconnu",
            Self::DeviseTenueInconnue => "devise_tenue_inconnue",
            Self::FuseauInconnu => "fuseau_inconnu",
            Self::ContexteDossierInvalide => "contexte_dossier_invalide",
            Self::ParametresPaysSansPaquet => "parametres_pays_sans_paquet",
            Self::ParametresAucuneBase => "parametres_aucune_base",
            Self::ParametresAucunChangement => "parametres_aucun_changement",
            Self::ParametresTauxInvalide => "parametres_taux_invalide",
            Self::ParametresRoleInconnu => "parametres_role_inconnu",
            Self::ParametresRegimeInconnu => "parametres_regime_inconnu",
            Self::ParametresRegimeNonRetirable => "parametres_regime_non_retirable",
            Self::ParametresMotifInvalide => "parametres_motif_invalide",
            Self::ParametresPeriodeInvalide => "parametres_periode_invalide",
            Self::ParametresDateEffetLointaine => "parametres_date_effet_lointaine",
            Self::TresorerieDeviseEtrangere => "tresorerie_devise_etrangere",
            Self::TresoreriePasUneCaisse => "tresorerie_pas_une_caisse",
            Self::TresorerieParametreInvalide => "tresorerie_parametre_invalide",
            Self::TiersIdentifiantTypeInconnu => "tiers_identifiant_type_inconnu",
            Self::TiersCompteBancaireTypeInconnu => "tiers_compte_bancaire_type_inconnu",
            Self::TiersValeurInvalide => "tiers_valeur_invalide",
            Self::TiersCaractereInvalide => "tiers_caractere_invalide",
            Self::TiersTropDIdentifiants => "tiers_trop_d_identifiants",
            Self::TiersTropDAdresses => "tiers_trop_d_adresses",
            Self::TiersTropDeContacts => "tiers_trop_de_contacts",
            Self::TiersTropDeComptesBancaires => "tiers_trop_de_comptes_bancaires",
            Self::ConditionReglementInvalide => "condition_reglement_invalide",
            Self::EcheanceMotifInvalide => "echeance_motif_invalide",
            Self::LettrageLigneInconnue => "lettrage_ligne_inconnue",
            Self::LettrageComptesDifferents => "lettrage_comptes_differents",
            Self::LettrageCompteNonLettrable => "lettrage_compte_non_lettrable",
            Self::LettrageLigneEnDouble => "lettrage_ligne_en_double",
            Self::LettrageSensManquant => "lettrage_sens_manquant",
            Self::LettrageTiersDifferents => "lettrage_tiers_differents",
            Self::LettrageDeviseEtrangere => "lettrage_devise_etrangere",
            Self::LettrageMotifInvalide => "lettrage_motif_invalide",
            Self::LettrageTropDeLignes => "lettrage_trop_de_lignes",
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
    /// Version de paramètres fiscaux d'un dossier illisible ou altérée (ADR 0030) : la lecture
    /// échoue (fermeture sûre) et le journal technique porte ce code, sans aucune donnée.
    ParametresFiscauxIllisibles,
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
            Self::ParametresFiscauxIllisibles => "parametres_fiscaux_illisibles",
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
            Self::Refus(_) | Self::Saisie { .. } | Self::Releve { .. } => {
                StatusCode::UNPROCESSABLE_ENTITY
            }
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
            Self::Releve { motif, .. } => motif.code(),
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
            crate::base::ErreurBase::VersionParametresIllisible => {
                Self::Interne(CategorieErreur::ParametresFiscauxIllisibles)
            }
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
                    Self::Saisie { ligne, .. }
                    | Self::Validation { ligne, .. }
                    | Self::Releve { ligne, .. } => ligne,
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
        toutes.extend(MotifReleve::TOUS.map(|motif| ErreurApi::Releve { motif, ligne: None }));
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

    #[test]
    fn une_version_de_parametres_illisible_a_sa_categorie_sans_detail() {
        let erreur = ErreurApi::from(crate::base::ErreurBase::VersionParametresIllisible);
        assert_eq!(
            erreur,
            ErreurApi::Interne(CategorieErreur::ParametresFiscauxIllisibles)
        );
        assert_eq!(
            CategorieErreur::ParametresFiscauxIllisibles.nom(),
            "parametres_fiscaux_illisibles"
        );
        // Le client ne voit que le corps uniforme.
        assert_eq!(erreur.code(), "erreur_interne");
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
