//! Lettre d'un lettrage : code par (dossier, compte), suite `A … Z, AA … AZ, BA …`
//! (numération bijective en base 26), jamais réutilisée (ADR 0032, point 4 ; spec §10.1).

use std::fmt;

use super::ErreurLettrage;

/// Nombre de lettres de l'alphabet latin.
const BASE: u64 = 26;

/// Lettre d'un lettrage. L'ordre est celui de la suite (`Z` < `AA`), comme le tri de Tryton
/// par longueur puis texte (`move.py:583-585`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Lettre(u64);

impl Lettre {
    /// Première lettre de la suite : `A`.
    #[must_use]
    pub fn premiere() -> Self {
        Self(1)
    }

    /// Lettre qui suit dans la suite.
    ///
    /// # Errors
    /// [`ErreurLettrage::LettresEpuisees`] au-delà de la capacité (jamais atteinte en pratique).
    pub fn suivante(self) -> Result<Self, ErreurLettrage> {
        self.0
            .checked_add(1)
            .map(Self)
            .ok_or(ErreurLettrage::LettresEpuisees)
    }

    /// Analyse une lettre écrite en majuscules `A` à `Z` (relecture de la persistance).
    ///
    /// # Errors
    /// [`ErreurLettrage::LettreInvalide`] si le texte est vide, contient autre chose que `A`
    /// à `Z` ou dépasse la capacité.
    pub fn parse(texte: &str) -> Result<Self, ErreurLettrage> {
        let invalide = || ErreurLettrage::LettreInvalide(texte.to_owned());
        if texte.is_empty() {
            return Err(invalide());
        }
        let mut rang: u64 = 0;
        for octet in texte.bytes() {
            if !octet.is_ascii_uppercase() {
                return Err(invalide());
            }
            rang = rang
                .checked_mul(BASE)
                .and_then(|r| r.checked_add(u64::from(octet - b'A') + 1))
                .ok_or_else(invalide)?;
        }
        Ok(Self(rang))
    }
}

impl fmt::Display for Lettre {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut chiffres = Vec::new();
        let mut reste = self.0;
        while reste > 0 {
            reste -= 1;
            // `reste % 26` < 26 : la conversion ne perd rien.
            chiffres.push(b'A' + u8::try_from(reste % BASE).unwrap_or(0));
            reste /= BASE;
        }
        chiffres.reverse();
        f.write_str(&String::from_utf8_lossy(&chiffres))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn suite_bijective() {
        let mut lettre = Lettre::premiere();
        let mut vues = Vec::new();
        for _ in 0..=(26 * 27) {
            vues.push(lettre.to_string());
            lettre = lettre.suivante().unwrap();
        }
        assert_eq!(vues[0], "A");
        assert_eq!(vues[25], "Z");
        assert_eq!(vues[26], "AA");
        assert_eq!(vues[27], "AB");
        assert_eq!(vues[51], "AZ");
        assert_eq!(vues[52], "BA");
        assert_eq!(vues[26 * 27 - 1], "ZZ");
        assert_eq!(vues[26 * 27], "AAA");
    }

    #[test]
    fn analyse_refusee() {
        for texte in ["", "a", "A1", "É", "A B"] {
            assert!(Lettre::parse(texte).is_err(), "{texte}");
        }
        assert!(Lettre::parse(&"Z".repeat(14)).is_err());
        assert!(Lettre::parse(&"Z".repeat(13)).is_ok());
    }

    #[test]
    fn ordre_de_la_suite() {
        assert!(Lettre::parse("Z").unwrap() < Lettre::parse("AA").unwrap());
        assert!(Lettre::parse("AZ").unwrap() < Lettre::parse("BA").unwrap());
    }

    proptest! {
        #[test]
        fn aller_retour(rang in 1u64..u64::MAX / 2) {
            let lettre = Lettre(rang);
            prop_assert_eq!(Lettre::parse(&lettre.to_string()).unwrap(), lettre);
        }
    }
}
