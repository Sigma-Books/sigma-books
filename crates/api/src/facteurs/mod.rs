//! Méthode de secours et codes de récupération (ADR 0012 ; L2 PR 8b) : la cryptographie, sans
//! base ni HTTP.
//!
//! - [`cle`] : clé de l'instance, lue dans un fichier ; sous-clés HKDF ; secret TOTP chiffré
//!   (AES-256-GCM, utilisateur en données associées) ; jeton d'enrôlement scellé ;
//! - [`totp`] : RFC 6238 (HMAC-SHA1, 6 chiffres, pas de 30 s, tolérance d'un pas), qui rend le
//!   **pas** accepté, base de l'anti-rejeu ; URI `otpauth` ;
//! - [`mot_de_passe`] : Argon2id (19 Mio, 2 passes), 12 caractères au moins, liste de mots de
//!   passe compromis ([`compromis`]) ;
//! - [`codes`] : codes de récupération de 120 bits, stockés hachés (SHA-256).
//!
//! Aucun secret n'a de `Debug` lisible ; tous sont effacés de la mémoire à leur libération.

pub mod cle;
pub mod codes;
pub mod compromis;
pub mod mot_de_passe;
pub mod totp;

/// Échec du tirage d'un aléa (source du système indisponible).
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("aléa du système indisponible")]
pub struct ErreurAlea;

/// Remplit `octets` d'aléa du système.
///
/// # Errors
/// [`ErreurAlea`].
pub fn tirer(octets: &mut [u8]) -> Result<(), ErreurAlea> {
    getrandom::fill(octets).map_err(|_| ErreurAlea)
}
