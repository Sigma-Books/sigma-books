//! Types communs aux paquets de pays.

use compta_core::TauxTva;
use serde::Deserialize;

use crate::erreur::ErreurFiscale;

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

impl Deduction {
    /// Nom dans les fichiers de données.
    #[must_use]
    pub const fn nom(self) -> &'static str {
        match self {
            Self::Oui => "oui",
            Self::Non => "non",
            Self::NonSaufExceptions => "non_sauf_exceptions",
            Self::OuiSiDeclaree => "oui_si_declaree",
            Self::SansObjet => "sans_objet",
            Self::NonTrouve => "non_trouve",
        }
    }
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

impl ExigibiliteLegale {
    /// Nom dans les fichiers de données.
    #[must_use]
    pub const fn nom(self) -> &'static str {
        match self {
            Self::FaitGenerateur => "fait_generateur",
            Self::Encaissement => "encaissement",
            Self::General => "general",
            Self::DeclarationPeriodique => "declaration_periodique",
            Self::DelaiApresEvenement => "delai_apres_evenement",
            Self::SansObjet => "sans_objet",
            Self::NonTrouve => "non_trouve",
        }
    }
}

/// Assiette légale d'un prélèvement (la règle exacte et ses exclusions sont dans la source).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Assiette {
    /// Chaque paiement reçu, tel qu'enregistré au compte du bénéficiaire.
    PaiementRecu,
    /// Montant transféré, hors commission de l'intermédiaire et hors taxes (dont la taxe
    /// elle-même).
    MontantTransfereHorsCommissionEtTaxes,
    /// Non trouvée dans le texte de référence.
    NonTrouve,
}

impl Assiette {
    /// Nom dans les fichiers de données.
    #[must_use]
    pub const fn nom(self) -> &'static str {
        match self {
            Self::PaiementRecu => "paiement_recu",
            Self::MontantTransfereHorsCommissionEtTaxes => {
                "montant_transfere_hors_commission_et_taxes"
            }
            Self::NonTrouve => "non_trouve",
        }
    }
}

/// Code générique d'un prélèvement (socle) : majuscules ASCII et `_`, sans chiffre (aucun code
/// ne porte un taux ni un article de loi), 3 à 40 caractères.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CodePrelevement(String);

impl CodePrelevement {
    /// Lit un code.
    ///
    /// # Errors
    /// [`ErreurFiscale::CodePrelevement`] si le code est mal formé.
    pub fn parse(texte: &str) -> Result<Self, ErreurFiscale> {
        let forme = (3..=40).contains(&texte.len())
            && texte.bytes().all(|b| b.is_ascii_uppercase() || b == b'_')
            && !texte.starts_with('_')
            && !texte.ends_with('_');
        if forme {
            Ok(Self(texte.to_owned()))
        } else {
            Err(ErreurFiscale::CodePrelevement(texte.to_owned()))
        }
    }

    /// Le code.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
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
