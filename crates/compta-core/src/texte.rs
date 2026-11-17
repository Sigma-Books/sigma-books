//! Textes courts saisis : intitulés de comptes et de journaux, motifs.
//!
//! Le texte est conservé **exactement** tel que saisi (aucune normalisation, aucun retrait de
//! caractère), y compris s'il commence par `=`, `+`, `-` ou `@` : la neutralisation des
//! formules relève de l'export tableur (docs/lots/L1.md, Q11).

use std::fmt;

/// Longueur maximale d'un texte court, en caractères.
pub const LONGUEUR_MAX: usize = 200;

/// Erreurs sur un texte court.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ErreurTexte {
    /// Texte vide ou fait uniquement d'espaces.
    #[error("texte vide")]
    Vide,
    /// Texte trop long.
    #[error("texte trop long : {0} caractères (au plus {LONGUEUR_MAX})")]
    TropLong(usize),
    /// Caractère de contrôle (retour à la ligne, tabulation…).
    #[error("caractère de contrôle interdit")]
    CaractereDeControle,
}

/// Texte court non vide, sans caractère de contrôle, d'au plus [`LONGUEUR_MAX`] caractères.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TexteCourt(String);

impl TexteCourt {
    /// Valide un texte court.
    ///
    /// # Errors
    /// [`ErreurTexte`] si le texte est vide, trop long ou contient un caractère de contrôle.
    pub fn parse(texte: &str) -> Result<Self, ErreurTexte> {
        if texte.trim().is_empty() {
            return Err(ErreurTexte::Vide);
        }
        let longueur = texte.chars().count();
        if longueur > LONGUEUR_MAX {
            return Err(ErreurTexte::TropLong(longueur));
        }
        if texte.chars().any(char::is_control) {
            return Err(ErreurTexte::CaractereDeControle);
        }
        Ok(Self(texte.to_owned()))
    }

    /// Le texte, tel que saisi.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for TexteCourt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn texte_conserve_exactement() {
        for texte in [
            "Achats",
            "-50 % remise",
            "=SOMME(A1)",
            "@client",
            " Caisse ",
        ] {
            assert_eq!(TexteCourt::parse(texte).unwrap().as_str(), texte);
        }
    }

    #[test]
    fn textes_refuses() {
        assert_eq!(TexteCourt::parse(""), Err(ErreurTexte::Vide));
        assert_eq!(TexteCourt::parse("   "), Err(ErreurTexte::Vide));
        assert_eq!(
            TexteCourt::parse("a\nb"),
            Err(ErreurTexte::CaractereDeControle)
        );
        assert_eq!(
            TexteCourt::parse(&"é".repeat(201)),
            Err(ErreurTexte::TropLong(201))
        );
        assert!(TexteCourt::parse(&"é".repeat(200)).is_ok());
    }

    proptest! {
        #[test]
        fn aucune_entree_ne_fait_paniquer(texte in "\\PC{0,300}") {
            if let Ok(t) = TexteCourt::parse(&texte) {
                prop_assert_eq!(t.as_str(), texte.as_str());
            }
        }
    }
}
