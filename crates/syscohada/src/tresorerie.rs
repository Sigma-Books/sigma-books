//! Types de comptes de trésorerie et compte par défaut de chaque type (L3-12, ADR 0034 point 1).
//!
//! Le compte par défaut d'un type est une **marque de données** de `data/comptes.toml`
//! (`type_tresorerie`, avec la page de la fiche qui la fonde), jamais un numéro dans le code
//! (règle absolue 9) : [`crate::Referentiel::compte_par_defaut_tresorerie`].

use std::fmt;

/// Type d'un compte de trésorerie (spec §9.1) : banque, caisse ou portefeuille de monnaie
/// électronique (mobile money).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TypeTresorerie {
    /// Compte bancaire.
    Banque,
    /// Caisse (espèces).
    Caisse,
    /// Portefeuille de monnaie électronique (téléphone portable).
    Portefeuille,
}

impl TypeTresorerie {
    /// Tous les types, dans l'ordre.
    pub const TOUS: [Self; 3] = [Self::Banque, Self::Caisse, Self::Portefeuille];

    /// Code stable du type, tel qu'écrit dans les données (`banque`, `caisse`, `portefeuille`).
    #[must_use]
    pub fn code(self) -> &'static str {
        match self {
            Self::Banque => "banque",
            Self::Caisse => "caisse",
            Self::Portefeuille => "portefeuille",
        }
    }

    /// Type correspondant à un code, `None` s'il est inconnu.
    #[must_use]
    pub fn depuis_code(code: &str) -> Option<Self> {
        Self::TOUS.into_iter().find(|t| t.code() == code)
    }
}

impl fmt::Display for TypeTresorerie {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.code())
    }
}
