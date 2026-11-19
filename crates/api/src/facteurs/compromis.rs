//! Liste embarquée des mots de passe compromis (ADR 0012 ; NIST SP 800-63B §5.1.1.2).
//!
//! Source : `SecLists` (MIT), fichiers `Pwdb_top-1000000.txt` et
//! `xato-net-10-million-passwords-1000000.txt`, au commit épinglé par le script de génération
//! `scripts/generer-mots-de-passe-compromis.py` (attribution : `docs/provenance/seclists.md`).
//! Seuls les mots de passe d'au moins 12 caractères y figurent : les plus courts sont refusés
//! par la longueur.
//!
//! Format (`crates/api/donnees/mots-de-passe-compromis.bin`) : entiers de 64 bits
//! gros-boutistes, triés, sans doublon ; chacun, les 8 premiers octets de
//! SHA-256(`sigma-books mots-de-passe-compromis v1\0` ‖ mot de passe normalisé). Aucun mot de
//! passe en clair. Recherche dichotomique, correspondance **exacte** après normalisation (et non
//! la recherche de sous-chaîne de Tryton, `res/user.py`).
//!
//! Normalisation, identique à celle du script : lettres ASCII A à Z en minuscules, rien
//! d'autre. Un faux positif (deux empreintes tronquées égales) refuse à tort un mot de passe :
//! sens sûr, de probabilité négligeable (72 540 entrées sur 2^64).

use sha2::{Digest, Sha256};

/// Les empreintes, triées.
static LISTE: &[u8] = include_bytes!("../../donnees/mots-de-passe-compromis.bin");

/// Préfixe de l'empreinte (version 1).
const PREFIXE: &[u8] = b"sigma-books mots-de-passe-compromis v1\0";

/// Empreinte tronquée d'un mot de passe, après normalisation.
fn empreinte(mot_de_passe: &str) -> u64 {
    let mut h = Sha256::new();
    h.update(PREFIXE);
    h.update(mot_de_passe.to_ascii_lowercase().as_bytes());
    let complete = h.finalize();
    let mut octets = [0u8; 8];
    octets.copy_from_slice(&complete[..8]);
    u64::from_be_bytes(octets)
}

/// Nombre d'entrées de la liste.
#[must_use]
pub fn nombre() -> usize {
    LISTE.len() / 8
}

fn entree(rang: usize) -> u64 {
    let mut octets = [0u8; 8];
    octets.copy_from_slice(&LISTE[rang * 8..rang * 8 + 8]);
    u64::from_be_bytes(octets)
}

/// Vrai si `mot_de_passe` figure dans la liste (après normalisation).
#[must_use]
pub fn est_compromis(mot_de_passe: &str) -> bool {
    let cherchee = empreinte(mot_de_passe);
    let (mut bas, mut haut) = (0, nombre());
    while bas < haut {
        let milieu = bas + (haut - bas) / 2;
        match entree(milieu).cmp(&cherchee) {
            std::cmp::Ordering::Equal => return true,
            std::cmp::Ordering::Less => bas = milieu + 1,
            std::cmp::Ordering::Greater => haut = milieu,
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn la_liste_est_triee_sans_doublon() {
        assert_eq!(LISTE.len() % 8, 0);
        assert_eq!(nombre(), 72_540);
        for rang in 1..nombre() {
            assert!(entree(rang - 1) < entree(rang), "rang {rang}");
        }
    }

    #[test]
    fn recherche_exacte_apres_normalisation() {
        assert!(est_compromis("1q2w3e4r5t6y"));
        assert!(est_compromis("1Q2W3E4R5T6Y"));
        // Correspondance exacte : ni préfixe, ni suffixe, ni sous-chaîne.
        assert!(!est_compromis("1q2w3e4r5t6y-sigma"));
        assert!(!est_compromis("x1q2w3e4r5t6y"));
        assert!(!est_compromis("une phrase de passe introuvable"));
    }
}
