//! Motif de libellé des règles de suggestion de relevé (ADR 0034 §8) : expression régulière
//! **linéaire** (`regex` : automates finis, aucun retour arrière, temps linéaire en la taille
//! du texte) et de **taille bornée**, refusée à la création si elle dépasse la borne (menace
//! F11.4). Les bornes sont des paramètres de l'instance passés par l'appelant.

use regex::{Regex, RegexBuilder};
use thiserror::Error;

/// Bornes d'un motif, fournies par l'appelant (paramètres de l'instance).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BornesMotif {
    /// Longueur maximale de la source du motif, en octets.
    pub longueur_source: usize,
    /// Taille maximale du programme compilé et du cache de l'automate, en octets
    /// (`RegexBuilder::size_limit` et `dfa_size_limit`).
    pub taille_programme: usize,
}

/// Refus d'un motif. Le texte du motif n'est jamais repris (il peut venir d'un libellé).
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ErreurMotif {
    /// La source dépasse la longueur admise.
    #[error("motif trop long : {longueur} octets pour {limite} admis")]
    SourceTropLongue {
        /// Longueur de la source, en octets.
        longueur: usize,
        /// Longueur admise.
        limite: usize,
    },
    /// Le programme compilé dépasse la taille admise.
    #[error("motif trop complexe : programme compilé au-delà de la taille admise")]
    ProgrammeTropGrand,
    /// La syntaxe est invalide.
    #[error("motif invalide")]
    Invalide,
}

/// Motif compilé, borné en taille et en temps.
#[derive(Debug, Clone)]
pub struct Motif(Regex);

impl Motif {
    /// Compile un motif dans les bornes données.
    ///
    /// # Errors
    /// [`ErreurMotif::SourceTropLongue`] avant toute analyse si la source dépasse la borne ;
    /// [`ErreurMotif::ProgrammeTropGrand`] si le programme compilé la dépasse ;
    /// [`ErreurMotif::Invalide`] pour une syntaxe refusée.
    pub fn compiler(source: &str, bornes: &BornesMotif) -> Result<Self, ErreurMotif> {
        if source.len() > bornes.longueur_source {
            return Err(ErreurMotif::SourceTropLongue {
                longueur: source.len(),
                limite: bornes.longueur_source,
            });
        }
        RegexBuilder::new(source)
            .size_limit(bornes.taille_programme)
            .dfa_size_limit(bornes.taille_programme)
            .build()
            .map(Self)
            .map_err(|e| match e {
                regex::Error::CompiledTooBig(_) => ErreurMotif::ProgrammeTropGrand,
                _ => ErreurMotif::Invalide,
            })
    }

    /// Vrai si le motif trouve une correspondance dans `texte` (temps linéaire).
    #[must_use]
    pub fn correspond(&self, texte: &str) -> bool {
        self.0.is_match(texte)
    }

    /// Source du motif.
    #[must_use]
    pub fn source(&self) -> &str {
        self.0.as_str()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BORNES: BornesMotif = BornesMotif {
        longueur_source: 256,
        taille_programme: 1 << 16,
    };

    #[test]
    fn correspondance_simple() {
        let m = Motif::compiler(r"(?i)^frais\s+\d+", &BORNES).unwrap();
        assert!(m.correspond("FRAIS 12 tenue de compte"));
        assert!(!m.correspond("virement frais"));
        assert_eq!(m.source(), r"(?i)^frais\s+\d+");
    }

    #[test]
    fn source_trop_longue_refusee_avant_analyse() {
        let source = "a".repeat(257);
        assert_eq!(
            Motif::compiler(&source, &BORNES).unwrap_err(),
            ErreurMotif::SourceTropLongue {
                longueur: 257,
                limite: 256
            }
        );
    }

    #[test]
    fn programme_trop_grand_refuse() {
        // Répétition bornée qui déplie un programme bien plus grand que la borne.
        assert_eq!(
            Motif::compiler(r"\w{1000}\w{1000}", &BORNES).unwrap_err(),
            ErreurMotif::ProgrammeTropGrand
        );
    }

    #[test]
    fn syntaxe_invalide_et_retour_arriere_refuses() {
        assert_eq!(
            Motif::compiler("(a", &BORNES).unwrap_err(),
            ErreurMotif::Invalide
        );
        // Les références arrière (temps exponentiel) n'existent pas dans `regex`.
        assert_eq!(
            Motif::compiler(r"(a+)\1", &BORNES).unwrap_err(),
            ErreurMotif::Invalide
        );
    }

    #[test]
    fn motif_pathologique_en_temps_lineaire() {
        // Catastrophique pour un moteur à retour arrière ; linéaire ici.
        let m = Motif::compiler(r"^(a+)+$", &BORNES).unwrap();
        let texte = format!("{}b", "a".repeat(10_000));
        assert!(!m.correspond(&texte));
    }
}
