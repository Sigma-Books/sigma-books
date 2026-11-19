//! TOTP (RFC 6238 ; ADR 0012) : HMAC-SHA1, 6 chiffres, pas de 30 secondes, tolérance d'un pas
//! de part et d'autre.
//!
//! [`verifier`] rend le **pas** accepté, pas un booléen : l'appelant ne l'admet que s'il dépasse
//! le dernier pas accepté, mémorisé en base (anti-rejeu, mise à jour conditionnelle). Les trois
//! pas candidats sont toujours calculés et comparés en temps constant.

use std::fmt;
use std::fmt::Write as _;

use ctutils::CtEq;
use hmac::{Hmac, KeyInit, Mac};
use sha1::Sha1;
use time::OffsetDateTime;
use zeroize::Zeroize;

use super::{ErreurAlea, tirer};

/// Durée d'un pas, en secondes.
pub const PAS_SECONDES: i64 = 30;
/// Chiffres d'un code.
pub const CHIFFRES: u32 = 6;
/// Taille du secret (160 bits, celle de HMAC-SHA1, RFC 4226 §4).
pub const OCTETS_SECRET: usize = 20;

/// Secret TOTP. Ni `Display`, ni `Debug` lisible, ni `Clone` ; effacé à sa libération.
pub struct SecretTotp([u8; OCTETS_SECRET]);

impl Drop for SecretTotp {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

impl fmt::Debug for SecretTotp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SecretTotp(<masqué>)")
    }
}

impl SecretTotp {
    /// Tire un secret (aléa du système).
    ///
    /// # Errors
    /// [`ErreurAlea`].
    pub fn tirer() -> Result<Self, ErreurAlea> {
        let mut octets = [0u8; OCTETS_SECRET];
        tirer(&mut octets)?;
        Ok(Self(octets))
    }

    /// Reprend un secret (déchiffré, ou tiré par un test).
    #[must_use]
    pub const fn depuis_octets(octets: [u8; OCTETS_SECRET]) -> Self {
        Self(octets)
    }

    /// Les octets du secret.
    #[must_use]
    pub const fn octets(&self) -> &[u8; OCTETS_SECRET] {
        &self.0
    }

    /// Le secret en base32 (RFC 4648, sans remplissage), forme saisie dans une application
    /// d'authentification.
    #[must_use]
    pub fn base32(&self) -> String {
        base32(&self.0)
    }

    /// Le code attendu à `instant` (6 chiffres). Sert aux tests et à l'affichage de contrôle ;
    /// la vérification passe par [`verifier`].
    #[must_use]
    pub fn code_a(&self, instant: OffsetDateTime) -> Option<String> {
        let pas = pas_de(instant)?;
        Some(format!("{:06}", hotp(&self.0, pas, CHIFFRES)))
    }
}

/// Pas de `instant` (secondes Unix divisées par 30). `None` avant 1970.
#[must_use]
pub fn pas_de(instant: OffsetDateTime) -> Option<u64> {
    u64::try_from(instant.unix_timestamp().div_euclid(PAS_SECONDES)).ok()
}

/// HOTP (RFC 4226 §5.3) : troncature dynamique de HMAC-SHA1(secret, compteur), réduite à
/// `chiffres` chiffres (6 à 8).
#[must_use]
pub fn hotp(secret: &[u8], compteur: u64, chiffres: u32) -> u32 {
    // HMAC accepte toute longueur de clé : l'erreur est impossible ; refus prudent sinon.
    let Ok(mut mac) = <Hmac<Sha1> as KeyInit>::new_from_slice(secret) else {
        return u32::MAX;
    };
    mac.update(&compteur.to_be_bytes());
    let empreinte = mac.finalize().into_bytes();
    let decalage = usize::from(empreinte[19] & 0x0f);
    let binaire = u32::from_be_bytes([
        empreinte[decalage] & 0x7f,
        empreinte[decalage + 1],
        empreinte[decalage + 2],
        empreinte[decalage + 3],
    ]);
    binaire % 10u32.pow(chiffres)
}

/// Vérifie le code `saisi` (exactement 6 chiffres ASCII) à `instant`, avec une tolérance d'un
/// pas de part et d'autre. Rend le pas accepté, ou `None`.
#[must_use]
pub fn verifier(secret: &SecretTotp, saisi: &str, instant: OffsetDateTime) -> Option<u64> {
    let octets = saisi.as_bytes();
    if octets.len() != CHIFFRES as usize || !octets.iter().all(u8::is_ascii_digit) {
        return None;
    }
    let pas = pas_de(instant)?;
    let mut accepte = None;
    for candidat in [pas.checked_sub(1), Some(pas), pas.checked_add(1)]
        .into_iter()
        .flatten()
    {
        let attendu = format!("{:06}", hotp(&secret.0, candidat, CHIFFRES));
        // Tous les candidats sont comparés, en temps constant : la durée ne dit pas lequel.
        if attendu.as_bytes().ct_eq(octets).to_bool() {
            accepte = Some(candidat);
        }
    }
    accepte
}

const ALPHABET_BASE32: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";

/// Base32 (RFC 4648 §6), sans remplissage.
#[must_use]
pub fn base32(octets: &[u8]) -> String {
    let mut sortie = String::with_capacity(octets.len().div_ceil(5) * 8);
    let mut tampon: u16 = 0;
    let mut bits = 0u32;
    for &o in octets {
        tampon = (tampon << 8) | u16::from(o);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            sortie.push(char::from(
                ALPHABET_BASE32[usize::from((tampon >> bits) & 31)],
            ));
        }
        tampon &= (1 << bits) - 1;
    }
    if bits > 0 {
        sortie.push(char::from(
            ALPHABET_BASE32[usize::from((tampon << (5 - bits)) & 31)],
        ));
    }
    sortie
}

/// Encodage d'une composante d'URI (RFC 3986 : seuls les caractères non réservés restent).
fn encoder_composante(texte: &str) -> String {
    let mut sortie = String::with_capacity(texte.len());
    for o in texte.bytes() {
        if o.is_ascii_alphanumeric() || matches!(o, b'-' | b'.' | b'_' | b'~') {
            sortie.push(char::from(o));
        } else {
            let _ = write!(sortie, "%{o:02X}");
        }
    }
    sortie
}

/// URI `otpauth://totp/…` (format « Key Uri » de Google Authenticator), que l'interface rend
/// en QR code : émetteur, compte, secret, algorithme, chiffres et période explicites.
#[must_use]
pub fn uri_otpauth(secret: &SecretTotp, emetteur: &str, compte: &str) -> String {
    let emetteur = encoder_composante(emetteur);
    format!(
        "otpauth://totp/{emetteur}:{}?secret={}&issuer={emetteur}&algorithm=SHA1&digits={CHIFFRES}&period={PAS_SECONDES}",
        encoder_composante(compte),
        secret.base32(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Secret des vecteurs de la RFC 6238, annexe B (SHA-1) : « 12345678901234567890 ».
    const SECRET_RFC: &[u8; 20] = b"12345678901234567890";

    #[test]
    fn vecteurs_de_la_rfc_6238() {
        // Annexe B : instant (secondes Unix) et code à 8 chiffres, SHA-1.
        for (secondes, attendu) in [
            (59_i64, 94_287_082_u32),
            (1_111_111_109, 7_081_804),
            (1_111_111_111, 14_050_471),
            (1_234_567_890, 89_005_924),
            (2_000_000_000, 69_279_037),
            (20_000_000_000, 65_353_130),
        ] {
            let instant = OffsetDateTime::from_unix_timestamp(secondes).unwrap();
            let pas = pas_de(instant).unwrap();
            assert_eq!(hotp(SECRET_RFC, pas, 8), attendu, "{secondes}");
            // À 6 chiffres : les 6 derniers.
            let secret = SecretTotp::depuis_octets(*SECRET_RFC);
            let code = secret.code_a(instant).unwrap();
            assert_eq!(code, format!("{:06}", attendu % 1_000_000));
            assert_eq!(verifier(&secret, &code, instant), Some(pas));
        }
    }

    #[test]
    fn vecteurs_hotp_de_la_rfc_4226() {
        // Annexe D : compteurs 0 à 9.
        let attendus = [
            755_224, 287_082, 359_152, 969_429, 338_314, 254_676, 287_922, 162_583, 399_871,
            520_489,
        ];
        for (compteur, attendu) in attendus.into_iter().enumerate() {
            assert_eq!(hotp(SECRET_RFC, compteur as u64, 6), attendu);
        }
    }

    #[test]
    fn tolerance_d_un_pas_et_formes_refusees() {
        let secret = SecretTotp::depuis_octets(*SECRET_RFC);
        let t = OffsetDateTime::from_unix_timestamp(1_234_567_890).unwrap();
        let pas = pas_de(t).unwrap();
        let code = secret.code_a(t).unwrap();
        let s = time::Duration::seconds(PAS_SECONDES);
        // Pas précédent et suivant : admis, et le pas rendu est celui du code.
        assert_eq!(verifier(&secret, &code, t + s), Some(pas));
        assert_eq!(verifier(&secret, &code, t - s), Some(pas));
        // Deux pas d'écart : refusé.
        assert_eq!(verifier(&secret, &code, t + s * 2), None);
        assert_eq!(verifier(&secret, &code, t - s * 2), None);
        for invalide in ["", "12345", "1234567", "12345a", " 123456", "１２３４５６"] {
            assert_eq!(verifier(&secret, invalide, t), None, "{invalide}");
        }
        let autre = SecretTotp::depuis_octets([7; 20]);
        assert_eq!(verifier(&autre, &code, t), None);
    }

    #[test]
    fn base32_de_la_rfc_4648() {
        // RFC 4648 §10, sans remplissage.
        for (clair, attendu) in [
            ("", ""),
            ("f", "MY"),
            ("fo", "MZXQ"),
            ("foo", "MZXW6"),
            ("foob", "MZXW6YQ"),
            ("fooba", "MZXW6YTB"),
            ("foobar", "MZXW6YTBOI"),
        ] {
            assert_eq!(base32(clair.as_bytes()), attendu);
        }
        assert_eq!(
            SecretTotp::depuis_octets(*SECRET_RFC).base32(),
            "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ"
        );
    }

    #[test]
    fn uri_otpauth_et_secret_masque() {
        let secret = SecretTotp::depuis_octets(*SECRET_RFC);
        assert_eq!(
            uri_otpauth(&secret, "Sigma Books", "a+b@exemple.invalid"),
            "otpauth://totp/Sigma%20Books:a%2Bb%40exemple.invalid?\
             secret=GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ&issuer=Sigma%20Books&algorithm=SHA1\
             &digits=6&period=30"
        );
        assert!(!format!("{secret:?}").contains("GEZ"));
        assert_ne!(
            SecretTotp::tirer().unwrap().octets(),
            SecretTotp::tirer().unwrap().octets()
        );
    }
}
