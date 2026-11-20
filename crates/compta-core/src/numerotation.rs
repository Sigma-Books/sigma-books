//! Numérotation définitive des écritures (spec §6.3 ; ADR 0007).
//!
//! Format `{journal}-{exercice}-{séquence sur 6 chiffres}` (ex. `AC-2026-000231`). La
//! séquence est continue, sans trou ni doublon, par journal et par exercice. Le calcul est
//! **pur** : le compteur courant est passé en paramètre et le suivant est renvoyé ; c'est à
//! l'appelant de l'enregistrer dans la même transaction que la validation (verrou, L2).

use std::fmt;
use std::str::FromStr;

use crate::exercice::{CodeExercice, ErreurExercice};
use crate::journal::{CodeJournal, ErreurJournal};

/// Plus grande séquence représentable sur six chiffres.
pub const SEQUENCE_MAX: u32 = 999_999;

/// Erreurs de numérotation.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ErreurNumerotation {
    /// La séquence du journal pour l'exercice est épuisée.
    #[error("séquence épuisée pour {0}-{1} ({SEQUENCE_MAX} écritures)")]
    SequenceEpuisee(CodeJournal, CodeExercice),
    /// Texte qui n'est pas un numéro d'écriture.
    #[error("numéro d'écriture invalide : « {0} »")]
    FormatInvalide(String),
}

impl From<ErreurJournal> for ErreurNumerotation {
    fn from(e: ErreurJournal) -> Self {
        Self::FormatInvalide(e.to_string())
    }
}

impl From<ErreurExercice> for ErreurNumerotation {
    fn from(e: ErreurExercice) -> Self {
        Self::FormatInvalide(e.to_string())
    }
}

/// Numéro définitif d'une écriture validée.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NumeroEcriture {
    journal: CodeJournal,
    exercice: CodeExercice,
    sequence: u32,
}

impl NumeroEcriture {
    /// Journal.
    #[must_use]
    pub fn journal(&self) -> &CodeJournal {
        &self.journal
    }

    /// Exercice.
    #[must_use]
    pub fn exercice(&self) -> &CodeExercice {
        &self.exercice
    }

    /// Rang dans la séquence (à partir de 1).
    #[must_use]
    pub fn sequence(&self) -> u32 {
        self.sequence
    }

    /// Analyse un numéro écrit `AC-2026-000231`.
    ///
    /// # Errors
    /// [`ErreurNumerotation::FormatInvalide`].
    pub fn parse(texte: &str) -> Result<Self, ErreurNumerotation> {
        let invalide = || ErreurNumerotation::FormatInvalide(texte.to_owned());
        let mut parties = texte.split('-');
        let (Some(journal), Some(exercice), Some(sequence), None) = (
            parties.next(),
            parties.next(),
            parties.next(),
            parties.next(),
        ) else {
            return Err(invalide());
        };
        if sequence.len() != 6 || !sequence.bytes().all(|b| b.is_ascii_digit()) {
            return Err(invalide());
        }
        let sequence: u32 = sequence.parse().map_err(|_| invalide())?;
        if sequence == 0 {
            return Err(invalide());
        }
        Ok(Self {
            journal: CodeJournal::parse(journal)?,
            exercice: CodeExercice::parse(exercice)?,
            sequence,
        })
    }
}

impl fmt::Display for NumeroEcriture {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}-{}-{:06}", self.journal, self.exercice, self.sequence)
    }
}

impl FromStr for NumeroEcriture {
    type Err = ErreurNumerotation;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

/// Compteur d'un journal pour un exercice : dernière séquence attribuée (0 si aucune).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Compteur {
    journal: CodeJournal,
    exercice: CodeExercice,
    dernier: u32,
}

impl Compteur {
    /// Compteur neuf : aucune écriture validée dans ce journal pour cet exercice.
    #[must_use]
    pub fn nouveau(journal: CodeJournal, exercice: CodeExercice) -> Self {
        Self {
            journal,
            exercice,
            dernier: 0,
        }
    }

    /// Compteur relu de la persistance : dernière séquence attribuée pour ce journal et cet
    /// exercice (lue sous verrou, lot L2).
    ///
    /// # Errors
    /// [`ErreurNumerotation::SequenceEpuisee`] si `dernier` dépasse [`SEQUENCE_MAX`].
    pub fn reprendre(
        journal: CodeJournal,
        exercice: CodeExercice,
        dernier: u32,
    ) -> Result<Self, ErreurNumerotation> {
        if dernier > SEQUENCE_MAX {
            return Err(ErreurNumerotation::SequenceEpuisee(journal, exercice));
        }
        Ok(Self {
            journal,
            exercice,
            dernier,
        })
    }

    /// Dernière séquence attribuée (0 si aucune).
    #[must_use]
    pub fn dernier(&self) -> u32 {
        self.dernier
    }

    /// Journal du compteur.
    #[must_use]
    pub fn journal(&self) -> &CodeJournal {
        &self.journal
    }

    /// Exercice du compteur.
    #[must_use]
    pub fn exercice(&self) -> &CodeExercice {
        &self.exercice
    }

    /// Attribue le numéro suivant et renvoie le compteur à enregistrer à sa place.
    /// Le compteur d'origine n'est pas modifié (fonction pure).
    ///
    /// # Errors
    /// [`ErreurNumerotation::SequenceEpuisee`] après 999 999 écritures.
    pub fn suivant(&self) -> Result<(NumeroEcriture, Compteur), ErreurNumerotation> {
        let sequence = self
            .dernier
            .checked_add(1)
            .filter(|s| *s <= SEQUENCE_MAX)
            .ok_or_else(|| {
                ErreurNumerotation::SequenceEpuisee(self.journal.clone(), self.exercice.clone())
            })?;
        let numero = NumeroEcriture {
            journal: self.journal.clone(),
            exercice: self.exercice.clone(),
            sequence,
        };
        let compteur = Compteur {
            dernier: sequence,
            ..self.clone()
        };
        Ok((numero, compteur))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn compteur() -> Compteur {
        Compteur::nouveau(
            CodeJournal::parse("AC").unwrap(),
            CodeExercice::parse("2026").unwrap(),
        )
    }

    #[test]
    fn format_de_la_spec() {
        let mut c = compteur();
        let mut dernier = None;
        for _ in 0..231 {
            let (numero, suivant) = c.suivant().unwrap();
            c = suivant;
            dernier = Some(numero);
        }
        let numero = dernier.unwrap();
        assert_eq!(numero.to_string(), "AC-2026-000231");
        assert_eq!(NumeroEcriture::parse("AC-2026-000231").unwrap(), numero);
    }

    #[test]
    fn fonction_pure() {
        let c = compteur();
        let (a, _) = c.suivant().unwrap();
        let (b, _) = c.suivant().unwrap();
        assert_eq!(a, b);
        assert_eq!(c.dernier(), 0);
    }

    #[test]
    fn sequence_epuisee() {
        let c = Compteur {
            dernier: SEQUENCE_MAX - 1,
            ..compteur()
        };
        let (numero, c) = c.suivant().unwrap();
        assert_eq!(numero.to_string(), "AC-2026-999999");
        assert!(matches!(
            c.suivant(),
            Err(ErreurNumerotation::SequenceEpuisee(_, _))
        ));
        let deborde = Compteur {
            dernier: u32::MAX,
            ..compteur()
        };
        assert!(deborde.suivant().is_err());
    }

    #[test]
    fn numeros_invalides() {
        for texte in [
            "",
            "AC-2026",
            "AC-2026-231",
            "AC-2026-0000231",
            "AC-2026-000000",
            "ac-2026-000001",
            "AC-26-000001",
            "AC-2026-000001-X",
            "AC-2026-00000A",
            "AC-2026-+00001",
        ] {
            assert!(NumeroEcriture::parse(texte).is_err(), "{texte:?}");
        }
    }

    proptest! {
        /// Une suite d'attributions donne 1, 2, 3… sans trou ni doublon.
        #[test]
        fn continuite_sans_trou(n in 1usize..2000) {
            let mut c = compteur();
            for attendu in 1..=n {
                let (numero, suivant) = c.suivant().unwrap();
                prop_assert_eq!(numero.sequence() as usize, attendu);
                prop_assert_eq!(suivant.dernier() as usize, attendu);
                c = suivant;
            }
        }

        #[test]
        fn aller_retour(journal in "[A-Z0-9]{1,8}", exercice in "[0-9]{4}", sequence in 1u32..=SEQUENCE_MAX) {
            let numero = NumeroEcriture {
                journal: CodeJournal::parse(&journal).unwrap(),
                exercice: CodeExercice::parse(&exercice).unwrap(),
                sequence,
            };
            prop_assert_eq!(NumeroEcriture::parse(&numero.to_string()).unwrap(), numero);
        }

        #[test]
        fn aucune_entree_ne_fait_paniquer(texte in "\\PC{0,30}") {
            let _ = NumeroEcriture::parse(&texte);
        }
    }
}
