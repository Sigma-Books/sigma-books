//! Erreurs de chargement et de validation des données.

use compta_core::ErreurEtiquette;

/// Erreurs de chargement ou de validation des données.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ErreurFiscale {
    /// Fichier TOML illisible ou non conforme.
    #[error("données illisibles : {0}")]
    Syntaxe(String),
    /// Code mal formé.
    #[error("code invalide : {0}")]
    Code(ErreurEtiquette),
    /// Taux qui n'est pas un décimal exact canonique.
    #[error("taux invalide pour {code} : {erreur}")]
    Taux {
        /// Entrée concernée.
        code: String,
        /// Cause.
        erreur: ErreurEtiquette,
    },
    /// Code en double.
    #[error("code en double : {0}")]
    CodeEnDouble(String),
    /// Même article et même alinéa pour deux entrées.
    #[error("article et alinéa en double : {0}")]
    ArticleEnDouble(String),
    /// Entrée sans source : article vide ou aucune page.
    #[error("entrée {0} sans source (article et page)")]
    SansSource(String),
    /// Page hors de la plage du document source.
    #[error("entrée {code} : page {page} hors de la plage du document")]
    Page {
        /// Entrée concernée.
        code: String,
        /// Page fautive.
        page: u16,
    },
    /// Libellé vide.
    #[error("entrée {0} sans libellé")]
    SansLibelle(String),
    /// Incohérence entre champs (exonéré avec taux, motif d'un régime non exonéré…).
    #[error("entrée {code} incohérente : {motif}")]
    Incoherente {
        /// Entrée concernée.
        code: String,
        /// Description.
        motif: &'static str,
    },
    /// Motif rattaché à un régime absent de la liste.
    #[error("motif {motif} : régime {regime} absent de la liste")]
    RegimeAbsent {
        /// Motif.
        motif: String,
        /// Régime cité.
        regime: String,
    },
    /// Chevauchement vers un motif absent.
    #[error("chevauchement {0} : motif cible absent")]
    CibleAbsente(String),
    /// Régime absent du socle : un paquet de pays ne crée pas de code.
    #[error("régime {0} absent du socle")]
    HorsSocle(String),
    /// Pays qui n'est pas un code ISO 3166 alpha-2 en majuscules.
    #[error("pays invalide : {0}")]
    Pays(String),
    /// Identifiant de version invalide (attendu `AAAA.N`).
    #[error("version invalide : {0}")]
    Version(String),
    /// Date illisible (attendu `AAAA-MM-JJ`).
    #[error("date invalide pour {0}")]
    Date(String),
    /// Fin avant le début.
    #[error("période invalide pour {0} : la fin précède le début")]
    Periode(String),
    /// Rôle de taux inconnu du socle.
    #[error("rôle de taux inconnu : {0}")]
    RoleInconnu(String),
    /// Aucun taux défini pour un régime qui en exige un.
    #[error("régime {0} : aucun des rôles de taux du socle n'a de valeur dans cette version")]
    TauxAbsent(String),
    /// Rétention inférieure au plancher du socle.
    #[error("rétention de {annees} ans inférieure au plancher de {plancher} ans")]
    RetentionSousPlancher {
        /// Valeur du paquet.
        annees: u16,
        /// Plancher du socle.
        plancher: u16,
    },
    /// Document source inconnu ou ambigu.
    #[error("entrée {0} : document source inconnu ou ambigu")]
    Document(String),
    /// Deux versions d'une même portée avec la même date d'effet.
    #[error("deux versions du pays {pays} prennent effet le même jour")]
    DateEffetEnDouble {
        /// Pays.
        pays: String,
    },
    /// Identifiant de version en double dans le registre.
    #[error("version en double : {0}")]
    VersionEnDouble(String),
    /// Un code de motif d'une version précédente a disparu ou changé de régime, d'article,
    /// d'alinéa ou de droit à déduction.
    #[error("motif {0} retiré ou réaffecté par rapport à la version précédente")]
    MotifRetire(String),
    /// Version de dossier là où un paquet de pays est attendu (ou l'inverse).
    #[error("portée de version inattendue : {0}")]
    PorteeInattendue(String),
    /// Version de dossier pour un pays qui n'a aucun paquet : elle n'aurait pas de base.
    #[error("le pays {0} n'a aucun paquet : pas de version de dossier")]
    PaysSansPaquet(String),
    /// Surcharge qui cite un régime que la version de base n'offre pas.
    #[error("régime {0} non offert par la version de base")]
    RegimeNonOffert(String),
    /// Surcharge qui retire un régime exonéré ou qui a des motifs : ils ne disparaissent jamais.
    #[error("régime {0} non retirable : il a des motifs")]
    RegimeNonRetirable(String),
    /// Surcharge sans aucun changement.
    #[error("la surcharge ne change rien")]
    SurchargeVide,
    /// Texte de surcharge (motif) invalide.
    #[error("motif de surcharge invalide")]
    MotifInvalide,
    /// Plancher de rétention hors de 1 à 100 ans.
    #[error("plancher de rétention invalide : {0} ans")]
    PlancherInvalide(u16),
}
