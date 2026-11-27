//! Erreurs de l'analyse d'un relevé. Elles ne portent que des positions (ligne physique,
//! numérotée à partir de 1, et champ), jamais un libellé, un montant ni un identifiant
//! bancaire (F2.11) : l'appelant peut les journaliser et les renvoyer telles quelles (422 avec
//! le rang de la première ligne fautive, ADR 0034 §6).

use thiserror::Error;

use super::Champ;

/// Refus d'un fichier de relevé.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ErreurReleve {
    /// Le fichier dépasse la taille admise (vérifié avant toute lecture).
    #[error("fichier trop grand : {taille} octets pour {limite} admis")]
    FichierTropGrand {
        /// Taille du fichier, en octets.
        taille: usize,
        /// Taille admise.
        limite: usize,
    },
    /// Octet nul : ce n'est pas un fichier texte (ADR 0034 §4).
    #[error("ligne {ligne} : octet nul")]
    OctetNul {
        /// Ligne physique.
        ligne: usize,
    },
    /// Octets non décodables dans l'encodage du profil.
    #[error("ligne {ligne} : octets invalides pour l'encodage du profil")]
    EncodageInvalide {
        /// Ligne physique.
        ligne: usize,
    },
    /// Caractère de contrôle autre que tabulation, fin de ligne et retour chariot (ADR 0034 §4).
    #[error("ligne {ligne} : caractère de contrôle")]
    CaractereDeControle {
        /// Ligne physique.
        ligne: usize,
    },
    /// Ligne physique plus longue que la limite (en octets du fichier).
    #[error("ligne {ligne} : ligne trop longue ({limite} octets admis)")]
    LigneTropLongue {
        /// Ligne physique.
        ligne: usize,
        /// Longueur admise.
        limite: usize,
    },
    /// Plus d'enregistrements que la limite (en-tête et pied compris).
    #[error("ligne {ligne} : trop de lignes ({limite} admises)")]
    TropDeLignes {
        /// Ligne physique du premier enregistrement en trop.
        ligne: usize,
        /// Nombre admis.
        limite: usize,
    },
    /// Plus de colonnes que la limite.
    #[error("ligne {ligne} : trop de colonnes ({limite} admises)")]
    TropDeColonnes {
        /// Ligne physique où commence l'enregistrement.
        ligne: usize,
        /// Nombre admis.
        limite: usize,
    },
    /// Cellule plus longue que la limite (en caractères).
    #[error("ligne {ligne}, colonne {colonne} : cellule trop longue ({limite} caractères admis)")]
    CelluleTropLongue {
        /// Ligne physique où commence l'enregistrement.
        ligne: usize,
        /// Rang de la colonne (0 = première).
        colonne: usize,
        /// Longueur admise.
        limite: usize,
    },
    /// Fin du fichier dans un champ entre guillemets : fichier tronqué (F11.4).
    #[error("ligne {ligne} : guillemet non fermé")]
    GuillemetNonFerme {
        /// Ligne physique où commence l'enregistrement.
        ligne: usize,
    },
    /// Le fichier n'a pas la ligne de titres que demandent les colonnes nommées du profil.
    #[error("ligne de titres absente")]
    TitresAbsents,
    /// Un titre de colonne du profil est absent de la ligne de titres.
    #[error("ligne {ligne} : titre de la colonne « {champ} » introuvable")]
    TitreIntrouvable {
        /// Ligne physique de la ligne de titres.
        ligne: usize,
        /// Champ du profil.
        champ: Champ,
    },
    /// Une ligne n'a pas la colonne demandée par le profil.
    #[error("ligne {ligne} : colonne « {champ} » absente")]
    ColonneAbsente {
        /// Ligne physique.
        ligne: usize,
        /// Champ du profil.
        champ: Champ,
    },
    /// Date illisible avec le format du profil, ou date inexistante.
    #[error("ligne {ligne} : date « {champ} » invalide")]
    DateInvalide {
        /// Ligne physique.
        ligne: usize,
        /// Champ du profil.
        champ: Champ,
    },
    /// Nombre illisible avec les séparateurs du profil, ou hors de la précision admise.
    #[error("ligne {ligne} : nombre « {champ} » invalide")]
    NombreInvalide {
        /// Ligne physique.
        ligne: usize,
        /// Champ du profil.
        champ: Champ,
    },
}

impl ErreurReleve {
    /// Ligne physique fautive (à partir de 1), quand l'erreur en a une.
    #[must_use]
    pub fn ligne(&self) -> Option<usize> {
        match self {
            Self::FichierTropGrand { .. } | Self::TitresAbsents => None,
            Self::OctetNul { ligne }
            | Self::EncodageInvalide { ligne }
            | Self::CaractereDeControle { ligne }
            | Self::LigneTropLongue { ligne, .. }
            | Self::TropDeLignes { ligne, .. }
            | Self::TropDeColonnes { ligne, .. }
            | Self::CelluleTropLongue { ligne, .. }
            | Self::GuillemetNonFerme { ligne }
            | Self::TitreIntrouvable { ligne, .. }
            | Self::ColonneAbsente { ligne, .. }
            | Self::DateInvalide { ligne, .. }
            | Self::NombreInvalide { ligne, .. } => Some(*ligne),
        }
    }

    /// Code stable de l'erreur, pour l'API et l'interface.
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::FichierTropGrand { .. } => "fichier_trop_grand",
            Self::OctetNul { .. } => "octet_nul",
            Self::EncodageInvalide { .. } => "encodage_invalide",
            Self::CaractereDeControle { .. } => "caractere_de_controle",
            Self::LigneTropLongue { .. } => "ligne_trop_longue",
            Self::TropDeLignes { .. } => "trop_de_lignes",
            Self::TropDeColonnes { .. } => "trop_de_colonnes",
            Self::CelluleTropLongue { .. } => "cellule_trop_longue",
            Self::GuillemetNonFerme { .. } => "guillemet_non_ferme",
            Self::TitresAbsents => "titres_absents",
            Self::TitreIntrouvable { .. } => "titre_introuvable",
            Self::ColonneAbsente { .. } => "colonne_absente",
            Self::DateInvalide { .. } => "date_invalide",
            Self::NombreInvalide { .. } => "nombre_invalide",
        }
    }
}
