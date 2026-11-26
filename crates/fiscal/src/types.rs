//! Types communs aux paquets de pays.

use compta_core::TauxTva;
use serde::Deserialize;

/// Droit à déduction de la taxe d'amont.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Deduction {
    /// Oui.
    Oui,
    /// Non.
    Non,
    /// Non, sauf pour les motifs qui renvoient à un régime ouvrant droit (table des
    /// chevauchements du paquet).
    NonSaufExceptions,
    /// Oui si la taxe a été déclarée (précompte).
    OuiSiDeclaree,
    /// Sans objet.
    SansObjet,
    /// Le texte de référence ne le dit pas.
    NonTrouve,
}

/// Règle légale d'exigibilité (l'article et le délai sont dans les sources du paquet).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExigibiliteLegale {
    /// Fait générateur : livraison, service, facture.
    FaitGenerateur,
    /// Encaissement.
    Encaissement,
    /// Règle générale.
    General,
    /// Déclaration périodique propre à l'opération (le texte en fixe la période).
    DeclarationPeriodique,
    /// Dans un délai après l'événement (le texte en fixe le délai).
    DelaiApresEvenement,
    /// Sans objet.
    SansObjet,
    /// Non trouvée dans le texte de référence.
    NonTrouve,
}

/// Rôle d'un taux : le socle les nomme, chaque version de paramètres donne leur valeur.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RoleTaux(pub(crate) String);

impl RoleTaux {
    /// Nom du rôle (`normal`, `reduit`).
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Taux nommé d'une version : rôle, valeur décimale exacte et source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TauxNomme {
    /// Rôle.
    pub role: RoleTaux,
    /// Valeur, en pourcent.
    pub valeur: TauxTva,
    /// Article et page.
    pub source: String,
}

/// Rétention minimale des pièces justificatives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Retention {
    /// Années de conservation, au moins le plancher du socle.
    pub annees_min: u16,
    /// Texte et article qui la fondent.
    pub source: String,
}

/// Portée d'une version de paramètres.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Portee {
    /// Paquet du pays (valeurs par défaut sourcées).
    Pack,
    /// Version propre à un dossier, de numéro `n` (rattachée au pays du dossier).
    Dossier(u32),
}
