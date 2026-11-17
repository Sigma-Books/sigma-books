//! Numéros de comptes (spec §5.1).
//!
//! Un numéro est formé d'un **tronc** officiel de 2 à 4 chiffres (le premier de 1 à 9),
//! éventuellement suivi de segments libres séparés par `-` : `6052-01`, `521-BICIS`,
//! `401-SENELEC`. Le tronc est immuable : aucun numéro ne peut être modifié après sa
//! construction. Ce module ne connaît **aucun** numéro de compte : la liste officielle vient
//! du référentiel passé en paramètre (`crate::plan`).

use std::fmt;
use std::str::FromStr;

/// Longueur minimale et maximale du tronc officiel.
const TRONC_MIN: usize = 2;
const TRONC_MAX: usize = 4;
/// Longueur maximale d'un segment libre.
const SEGMENT_MAX: usize = 12;
/// Longueur maximale d'un numéro complet.
const NUMERO_MAX: usize = 32;

/// Erreurs d'analyse d'un numéro de compte.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ErreurNumeroCompte {
    /// Numéro vide ou trop long.
    #[error("numéro de compte de longueur invalide : « {0} » (1 à {NUMERO_MAX} caractères)")]
    Longueur(String),
    /// Le tronc n'est pas formé de 2 à 4 chiffres commençant par 1 à 9.
    #[error("tronc invalide dans « {0} » : 2 à 4 chiffres, le premier de 1 à 9")]
    Tronc(String),
    /// Un segment libre est vide, trop long ou contient autre chose que `A-Z` et `0-9`.
    #[error("segment invalide dans « {0} » : 1 à {SEGMENT_MAX} caractères parmi A-Z et 0-9")]
    Segment(String),
}

/// Numéro de compte valide. Seul constructeur : [`NumeroCompte::parse`].
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NumeroCompte(String);

impl NumeroCompte {
    /// Analyse un numéro de compte, sans normalisation (majuscules, espaces) : une saisie à
    /// corriger est refusée avec une erreur explicite.
    ///
    /// # Errors
    /// [`ErreurNumeroCompte`] décrivant la partie invalide.
    pub fn parse(texte: &str) -> Result<Self, ErreurNumeroCompte> {
        if texte.is_empty() || texte.len() > NUMERO_MAX {
            return Err(ErreurNumeroCompte::Longueur(texte.to_owned()));
        }
        let mut segments = texte.split('-');
        let tronc = segments.next().unwrap_or_default();
        let tronc_valide = (TRONC_MIN..=TRONC_MAX).contains(&tronc.len())
            && tronc.bytes().all(|b| b.is_ascii_digit())
            && !tronc.starts_with('0');
        if !tronc_valide {
            return Err(ErreurNumeroCompte::Tronc(texte.to_owned()));
        }
        for segment in segments {
            let valide = (1..=SEGMENT_MAX).contains(&segment.len())
                && segment
                    .bytes()
                    .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit());
            if !valide {
                return Err(ErreurNumeroCompte::Segment(texte.to_owned()));
            }
        }
        Ok(Self(texte.to_owned()))
    }

    /// Le numéro complet.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Le tronc officiel (chiffres avant le premier `-`).
    #[must_use]
    pub fn tronc(&self) -> &str {
        self.0.split('-').next().unwrap_or_default()
    }

    /// Classe du compte : premier chiffre du tronc (1 à 9).
    #[must_use]
    pub fn classe(&self) -> u8 {
        self.0.as_bytes().first().map_or(0, |b| b - b'0')
    }

    /// Vrai pour un numéro officiel (sans segment libre).
    #[must_use]
    pub fn est_officiel(&self) -> bool {
        !self.0.contains('-')
    }

    /// Parent dans la hiérarchie : on retire le dernier segment libre, ou, pour un numéro
    /// officiel, le dernier chiffre. Un tronc de 2 chiffres n'a pas de parent.
    #[must_use]
    pub fn parent(&self) -> Option<NumeroCompte> {
        if let Some((parent, _)) = self.0.rsplit_once('-') {
            return Some(Self(parent.to_owned()));
        }
        if self.0.len() > TRONC_MIN {
            return Some(Self(self.0[..self.0.len() - 1].to_owned()));
        }
        None
    }

    /// Vrai si `self` est `autre` ou l'un de ses ascendants.
    #[must_use]
    pub fn est_ascendant_ou_egal(&self, autre: &NumeroCompte) -> bool {
        let mut courant = Some(autre.clone());
        while let Some(numero) = courant {
            if &numero == self {
                return true;
            }
            courant = numero.parent();
        }
        false
    }
}

impl fmt::Display for NumeroCompte {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl FromStr for NumeroCompte {
    type Err = ErreurNumeroCompte;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn n(texte: &str) -> NumeroCompte {
        NumeroCompte::parse(texte).unwrap()
    }

    #[test]
    fn numeros_valides() {
        for texte in ["10", "601", "6052", "6052-01", "521-BICIS", "401-SENELEC-2"] {
            assert_eq!(n(texte).as_str(), texte);
        }
        assert_eq!(n("521-BICIS").tronc(), "521");
        assert_eq!(n("521-BICIS").classe(), 5);
        assert!(n("6052").est_officiel());
        assert!(!n("6052-01").est_officiel());
    }

    #[test]
    fn numeros_invalides() {
        let cas = [
            ("", "longueur"),
            ("1", "tronc"),
            ("12345", "tronc"),
            ("012", "tronc"),
            ("6a1", "tronc"),
            ("-01", "tronc"),
            ("601-", "segment"),
            ("601--A", "segment"),
            ("601-bicis", "segment"),
            ("601-É", "segment"),
            ("601 01", "tronc"),
            ("601-ABCDEFGHIJKLM", "segment"),
        ];
        for (texte, attendu) in cas {
            let erreur = NumeroCompte::parse(texte).unwrap_err();
            let genre = match erreur {
                ErreurNumeroCompte::Longueur(_) => "longueur",
                ErreurNumeroCompte::Tronc(_) => "tronc",
                ErreurNumeroCompte::Segment(_) => "segment",
            };
            assert_eq!(genre, attendu, "{texte:?}");
        }
        assert!(NumeroCompte::parse(&format!("601{}", "-ABCDEFGH".repeat(4))).is_err());
    }

    #[test]
    fn hierarchie() {
        assert_eq!(n("6052-01-A").parent(), Some(n("6052-01")));
        assert_eq!(n("6052-01").parent(), Some(n("6052")));
        assert_eq!(n("6052").parent(), Some(n("605")));
        assert_eq!(n("60").parent(), None);
        assert!(n("60").est_ascendant_ou_egal(&n("6052-01")));
        assert!(n("6052-01").est_ascendant_ou_egal(&n("6052-01")));
        assert!(!n("61").est_ascendant_ou_egal(&n("6052-01")));
    }

    proptest! {
        #[test]
        fn aller_retour(texte in "[1-9][0-9]{1,3}(-[A-Z0-9]{1,6}){0,3}") {
            let numero = NumeroCompte::parse(&texte).unwrap();
            prop_assert_eq!(numero.to_string(), texte.clone());
            prop_assert_eq!(NumeroCompte::parse(numero.as_str()).unwrap(), numero);
        }

        #[test]
        fn aucune_entree_ne_fait_paniquer(texte in "\\PC{0,40}") {
            let _ = NumeroCompte::parse(&texte);
        }

        #[test]
        fn le_parent_d_un_numero_valide_est_valide(texte in "[1-9][0-9]{1,3}(-[A-Z0-9]{1,6}){0,3}") {
            let mut courant = Some(NumeroCompte::parse(&texte).unwrap());
            while let Some(numero) = courant {
                prop_assert!(NumeroCompte::parse(numero.as_str()).is_ok());
                prop_assert_eq!(numero.tronc().chars().next(), texte.chars().next());
                courant = numero.parent();
            }
        }
    }
}
