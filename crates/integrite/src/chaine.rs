//! Chaîne de hachage SHA-256 d'un dossier (spec §6.4, ADR 0007).
//!
//! `empreinte_n = SHA-256(empreinte_{n-1} ‖ contenu canonique de l'écriture n)`, où
//! `empreinte_{n-1}` entre sous forme de 32 octets bruts. L'empreinte de départ (rang 0) est
//! propre au dossier : `SHA-256("sigma-books chaine v1\n" ‖ identifiant du dossier)`. Une
//! chaîne ne peut donc pas être greffée sur celle d'un autre dossier.

use std::fmt;

use compta_core::EcritureValidee;
use sha2::{Digest, Sha256};

use crate::canonique::EcritureCanonique;

/// Préfixe de l'empreinte de départ, version 1 de la chaîne.
const GENESE_V1: &[u8] = b"sigma-books chaine v1\n";
/// Longueur maximale d'un identifiant de dossier.
pub const ID_DOSSIER_MAX: usize = 64;

/// Erreurs sur les empreintes et identifiants.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ErreurChaine {
    /// Empreinte qui n'est pas 64 caractères parmi `0-9a-f`.
    #[error("empreinte invalide : 64 caractères hexadécimaux minuscules attendus")]
    EmpreinteInvalide,
    /// Identifiant de dossier vide, trop long ou hors de `A-Z a-z 0-9 . _ -`.
    #[error("identifiant de dossier invalide")]
    IdDossierInvalide,
}

/// Empreinte SHA-256, affichée en hexadécimal minuscule.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Empreinte([u8; 32]);

impl Empreinte {
    /// Les 32 octets.
    #[must_use]
    pub fn octets(&self) -> &[u8; 32] {
        &self.0
    }

    /// Empreinte de 32 octets bruts (relue en base, par exemple).
    #[must_use]
    pub const fn depuis_octets(octets: [u8; 32]) -> Self {
        Self(octets)
    }

    /// SHA-256 d'un contenu.
    #[must_use]
    pub fn calculer(contenu: &[u8]) -> Self {
        Self::sha256(&[contenu])
    }

    /// Lit une empreinte hexadécimale minuscule (64 caractères).
    ///
    /// # Errors
    /// [`ErreurChaine::EmpreinteInvalide`].
    pub fn parse(texte: &str) -> Result<Self, ErreurChaine> {
        fn chiffre(b: u8) -> Option<u8> {
            match b {
                b'0'..=b'9' => Some(b - b'0'),
                b'a'..=b'f' => Some(b - b'a' + 10),
                _ => None,
            }
        }
        let octets = texte.as_bytes();
        if octets.len() != 64 {
            return Err(ErreurChaine::EmpreinteInvalide);
        }
        let mut sortie = [0u8; 32];
        for (i, [a, b]) in octets.as_chunks::<2>().0.iter().enumerate() {
            let (Some(fort), Some(faible)) = (chiffre(*a), chiffre(*b)) else {
                return Err(ErreurChaine::EmpreinteInvalide);
            };
            sortie[i] = fort << 4 | faible;
        }
        Ok(Self(sortie))
    }

    pub(crate) fn sha256(morceaux: &[&[u8]]) -> Self {
        let mut h = Sha256::new();
        for m in morceaux {
            h.update(m);
        }
        Self(h.finalize().into())
    }
}

impl fmt::Display for Empreinte {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for b in self.0 {
            write!(f, "{b:02x}")?;
        }
        Ok(())
    }
}

impl fmt::Debug for Empreinte {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Empreinte({self})")
    }
}

/// Identifiant opaque d'un dossier : 1 à 64 caractères parmi `A-Z a-z 0-9 . _ -`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct IdDossier(String);

impl IdDossier {
    /// Valide un identifiant de dossier.
    ///
    /// # Errors
    /// [`ErreurChaine::IdDossierInvalide`].
    pub fn parse(texte: &str) -> Result<Self, ErreurChaine> {
        let valide = (1..=ID_DOSSIER_MAX).contains(&texte.len())
            && texte
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'));
        if valide {
            Ok(Self(texte.to_owned()))
        } else {
            Err(ErreurChaine::IdDossierInvalide)
        }
    }

    /// L'identifiant.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for IdDossier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Empreinte de départ de la chaîne d'un dossier.
#[must_use]
pub fn genese(dossier: &IdDossier) -> Empreinte {
    Empreinte::sha256(&[GENESE_V1, dossier.as_str().as_bytes()])
}

/// Maillon suivant : `SHA-256(precedente ‖ contenu)`.
#[must_use]
pub fn chainer(precedente: &Empreinte, contenu: &[u8]) -> Empreinte {
    Empreinte::sha256(&[precedente.octets(), contenu])
}

/// Chaîne d'un dossier en cours de construction : empreinte de tête et nombre de maillons.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chaine {
    tete: Empreinte,
    longueur: u64,
}

impl Chaine {
    /// Chaîne vide d'un dossier : la tête est l'empreinte de départ.
    #[must_use]
    pub fn ouvrir(dossier: &IdDossier) -> Self {
        Self {
            tete: genese(dossier),
            longueur: 0,
        }
    }

    /// Reprend une chaîne à partir de sa tête enregistrée (lot L2 : tête lue en base).
    #[must_use]
    pub fn reprendre(tete: Empreinte, longueur: u64) -> Self {
        Self { tete, longueur }
    }

    /// Ajoute une écriture validée et renvoie sa propre empreinte (nouvelle tête).
    pub fn ajouter(&mut self, ecriture: &EcritureValidee) -> Empreinte {
        let contenu = EcritureCanonique::depuis(ecriture).serialiser();
        self.tete = chainer(&self.tete, contenu.as_bytes());
        self.longueur += 1;
        self.tete
    }

    /// Empreinte de tête.
    #[must_use]
    pub fn tete(&self) -> &Empreinte {
        &self.tete
    }

    /// Nombre d'écritures chaînées.
    #[must_use]
    pub fn longueur(&self) -> u64 {
        self.longueur
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empreinte_aller_retour() {
        let e = genese(&IdDossier::parse("demo").unwrap());
        assert_eq!(Empreinte::parse(&e.to_string()), Ok(e));
        assert!(Empreinte::parse(&e.to_string().to_uppercase()).is_err());
        assert!(Empreinte::parse("00").is_err());
        assert!(Empreinte::parse(&"g".repeat(64)).is_err());
    }

    #[test]
    fn vecteur_sha256_connu() {
        // FIPS 180-2, exemple « abc ».
        let e = Empreinte::sha256(&[b"a", b"bc"]);
        assert_eq!(
            e.to_string(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn identifiants_de_dossier() {
        assert!(IdDossier::parse("0193e0c2-7d1a-7c4e-9f00-1a2b3c4d5e6f").is_ok());
        for invalide in ["", "a b", "é", "a\n", &"a".repeat(65)] {
            assert!(IdDossier::parse(invalide).is_err(), "{invalide:?}");
        }
    }

    #[test]
    fn genese_propre_au_dossier() {
        let a = genese(&IdDossier::parse("a").unwrap());
        let b = genese(&IdDossier::parse("b").unwrap());
        assert_ne!(a, b);
    }
}
