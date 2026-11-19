//! Codes de récupération (ADR 0012 ; F1.12) : dix codes à usage unique, de **120 bits** d'aléa
//! chacun (l'ADR exige 80 bits au moins ; ASVS 4.0.3 V2.6.2 en demande 112 pour un secret
//! stocké par un hachage non salé). Remis une seule fois ; stockés hachés (SHA-256 : leur aléa
//! rend un dictionnaire inutile).
//!
//! Forme remise : 24 caractères de la base32 de Crockford, en six groupes de quatre séparés par
//! des tirets (`7K3M-…`). Saisie tolérante : tirets et espaces ignorés, casse indifférente,
//! `I` et `L` lus `1`, `O` lu `0` (alphabet de Crockford).

use std::fmt;

use sha2::{Digest, Sha256};
use zeroize::Zeroize;

use super::{ErreurAlea, tirer};

/// Nombre de codes d'une série.
pub const NOMBRE: usize = 10;
/// Aléa d'un code, en octets (120 bits).
const OCTETS: usize = 15;
/// Caractères d'un code, hors tirets.
const CARACTERES: usize = 24;
/// Alphabet de Crockford.
const ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

/// Un code de récupération, en clair : remis une fois, jamais stocké. Ni `Debug` lisible, ni
/// `Clone` ; effacé à sa libération.
pub struct CodeRecuperation(String);

impl Drop for CodeRecuperation {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

impl fmt::Debug for CodeRecuperation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("CodeRecuperation(<masqué>)")
    }
}

impl CodeRecuperation {
    /// Forme remise à l'utilisateur, avec tirets.
    #[must_use]
    pub fn texte(&self) -> &str {
        &self.0
    }

    /// Empreinte stockée.
    #[must_use]
    pub fn empreinte(&self) -> [u8; 32] {
        // La forme remise est canonique : sa lecture réussit toujours.
        empreinte_saisie(&self.0).unwrap_or([0; 32])
    }
}

/// Tire une série de [`NOMBRE`] codes.
///
/// # Errors
/// [`ErreurAlea`].
pub fn tirer_serie() -> Result<Vec<CodeRecuperation>, ErreurAlea> {
    (0..NOMBRE).map(|_| tirer_code()).collect()
}

fn tirer_code() -> Result<CodeRecuperation, ErreurAlea> {
    let mut alea = [0u8; OCTETS];
    tirer(&mut alea)?;
    let mut caracteres = String::with_capacity(CARACTERES + 5);
    let mut tampon: u16 = 0;
    let mut bits = 0u32;
    let mut ecrits = 0;
    for &o in &alea {
        tampon = (tampon << 8) | u16::from(o);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            if ecrits > 0 && ecrits % 4 == 0 {
                caracteres.push('-');
            }
            caracteres.push(char::from(ALPHABET[usize::from((tampon >> bits) & 31)]));
            ecrits += 1;
        }
        tampon &= (1 << bits) - 1;
    }
    alea.zeroize();
    Ok(CodeRecuperation(caracteres))
}

/// Empreinte d'un code saisi, après normalisation ; `None` s'il n'a pas la forme d'un code.
#[must_use]
pub fn empreinte_saisie(saisi: &str) -> Option<[u8; 32]> {
    if saisi.len() > 64 {
        return None;
    }
    let mut canonique = Vec::with_capacity(CARACTERES);
    for c in saisi.bytes() {
        let c = match c.to_ascii_uppercase() {
            b'-' | b' ' => continue,
            b'I' | b'L' => b'1',
            b'O' => b'0',
            c if ALPHABET.contains(&c) => c,
            _ => return None,
        };
        canonique.push(c);
    }
    if canonique.len() != CARACTERES {
        return None;
    }
    let mut h = Sha256::new();
    h.update(b"sigma-books code-recuperation v1\0");
    h.update(&canonique);
    canonique.zeroize();
    Some(h.finalize().into())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    #[test]
    fn une_serie_de_dix_codes_distincts_de_120_bits() {
        let serie = tirer_serie().unwrap();
        assert_eq!(serie.len(), NOMBRE);
        let empreintes: BTreeSet<[u8; 32]> =
            serie.iter().map(CodeRecuperation::empreinte).collect();
        assert_eq!(empreintes.len(), NOMBRE);
        for code in &serie {
            let texte = code.texte();
            assert_eq!(texte.len(), CARACTERES + 5, "{texte}");
            assert_eq!(texte.split('-').count(), 6);
            assert!(texte.split('-').all(|g| g.len() == 4));
            assert!(!format!("{code:?}").contains(&texte[..4]));
        }
    }

    #[test]
    fn saisie_tolerante_et_formes_refusees() {
        let code = tirer_code().unwrap();
        let attendue = Some(code.empreinte());
        let sans_tirets: String = code.texte().chars().filter(|c| *c != '-').collect();
        assert_eq!(empreinte_saisie(&sans_tirets), attendue);
        assert_eq!(empreinte_saisie(&code.texte().to_lowercase()), attendue);
        assert_eq!(empreinte_saisie(&code.texte().replace('-', " ")), attendue);
        // Lettres ambiguës de Crockford.
        assert_eq!(
            empreinte_saisie("IIII-LLLL-OOOO-0000-1111-2222"),
            empreinte_saisie("1111-1111-0000-0000-1111-2222")
        );
        for invalide in [
            "",
            "1111-1111-0000-0000-1111-222",
            "1111-1111-0000-0000-1111-22222",
            "UUUU-1111-0000-0000-1111-2222",
            "1111_1111_0000_0000_1111_2222",
        ] {
            assert_eq!(empreinte_saisie(invalide), None, "{invalide}");
        }
        assert_eq!(empreinte_saisie(&"1".repeat(65)), None);
    }
}
