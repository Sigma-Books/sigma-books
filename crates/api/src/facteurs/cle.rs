//! Clé de l'instance (ADR 0012 ; note d'étude de la PR 8, §5) : 32 octets en base64 dans un
//! fichier désigné par `SIGMA_FICHIER_CLE_INSTANCE`, lisible par son seul propriétaire, tenu
//! hors de la base. Un fichier plutôt qu'une variable portant la clé : l'environnement d'un
//! processus se lit dans `/proc/<pid>/environ` et se recopie dans les journaux de lancement
//! (ASVS 4.0.3 V6.4.1, V2.8.2). Le binaire ne génère jamais de clé : sans elle, il refuse de
//! démarrer.
//!
//! # Sous-clés
//! Tirées par HKDF-SHA256 (RFC 5869), sel `sigma-books cle-instance v1`, une information par
//! usage : `totp v1` (chiffrement du secret TOTP), `enrolement v1` (jeton d'enrôlement),
//! `enumeration v1` (clé de limitation de débit par courriel, sans garder le courriel en
//! mémoire). Une sous-clé compromise ne donne pas les autres.
//!
//! # Identifiant de clé
//! Les 8 premiers octets de SHA-256(`sigma-books identifiant-cle v1\0` ‖ clé) : stocké avec
//! chaque secret chiffré, il dit si la clé courante peut le lire. Après un changement de clé,
//! un secret d'une autre clé est illisible : le TOTP doit être réenrôlé.
//!
//! # Secret TOTP chiffré
//! AES-256-GCM, nonce aléatoire de 96 bits, rangé en tête : `nonce ‖ chiffré ‖ étiquette`.
//! Données associées : `sigma-books totp v1\0` ‖ identifiant de l'utilisateur ‖ identifiant de
//! clé. Un secret recopié sur la ligne d'un autre utilisateur ne se déchiffre pas.

use std::fmt;
use std::path::Path;

use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Nonce};
use base64ct::{Base64, Encoding};
use hkdf::Hkdf;
use sha2::{Digest, Sha256};
use uuid::Uuid;
use zeroize::{Zeroize, Zeroizing};

use super::totp::{OCTETS_SECRET, SecretTotp};
use super::{ErreurAlea, tirer};

/// Taille de la clé de l'instance (256 bits).
pub const OCTETS_CLE: usize = 32;
/// Taille du nonce AES-GCM (96 bits).
const OCTETS_NONCE: usize = 12;
/// Taille de l'étiquette AES-GCM.
const OCTETS_ETIQUETTE: usize = 16;
/// Taille d'un secret TOTP chiffré : nonce, secret, étiquette.
pub const OCTETS_TOTP_CHIFFRE: usize = OCTETS_NONCE + OCTETS_SECRET + OCTETS_ETIQUETTE;

/// Clé de l'instance refusée. Aucun message ne recopie le contenu du fichier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ErreurCle {
    /// Fichier absent ou illisible.
    #[error("fichier de la clé d'instance illisible")]
    Illisible,
    /// Fichier lisible par le groupe ou par les autres, ou qui n'est pas un fichier ordinaire.
    #[error("fichier de la clé d'instance : droits trop larges (attendu 0600)")]
    DroitsTropLarges,
    /// Contenu qui n'est pas 32 octets en base64.
    #[error("fichier de la clé d'instance : 32 octets en base64 attendus")]
    Format,
}

/// Clé de l'instance et ses sous-clés. Ni `Display`, ni `Debug` lisible ; effacée à sa
/// libération. Deux clés sont égales si leurs identifiants le sont.
pub struct CleInstance {
    totp: [u8; OCTETS_CLE],
    enrolement: [u8; OCTETS_CLE],
    enumeration: [u8; OCTETS_CLE],
    identifiant: [u8; 8],
}

impl Drop for CleInstance {
    fn drop(&mut self) {
        self.totp.zeroize();
        self.enrolement.zeroize();
        self.enumeration.zeroize();
    }
}

impl fmt::Debug for CleInstance {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "CleInstance(id {})", hex(&self.identifiant))
    }
}

impl PartialEq for CleInstance {
    fn eq(&self, autre: &Self) -> bool {
        self.identifiant == autre.identifiant
    }
}

impl Eq for CleInstance {}

fn hex(octets: &[u8]) -> String {
    use std::fmt::Write as _;
    octets.iter().fold(String::new(), |mut texte, o| {
        let _ = write!(texte, "{o:02x}");
        texte
    })
}

/// Sous-clé de 256 bits pour l'usage `info`.
fn sous_cle(maitre: &[u8; OCTETS_CLE], info: &[u8]) -> [u8; OCTETS_CLE] {
    let hkdf = Hkdf::<Sha256>::new(Some(b"sigma-books cle-instance v1"), maitre);
    // Valeur de repli, dérivée de la clé maîtresse et de l'usage : jamais une constante. Le
    // tampon en est initialisé, puis l'extraction HKDF l'écrase. 32 octets sont toujours admis
    // (au plus 255 × 32) : en pratique, l'erreur est impossible et le repli ne sert pas.
    let mut h = Sha256::new();
    h.update(b"sigma-books sous-cle de repli v1\0");
    h.update(maitre);
    h.update(info);
    let mut cle: [u8; OCTETS_CLE] = h.finalize().into();
    let repli = cle;
    if hkdf.expand(info, &mut cle).is_err() {
        cle = repli;
    }
    cle
}

impl CleInstance {
    /// Clé de l'instance à partir de ses 32 octets (tests ; le binaire lit un fichier).
    #[must_use]
    pub fn depuis_octets(mut maitre: [u8; OCTETS_CLE]) -> Self {
        let mut h = Sha256::new();
        h.update(b"sigma-books identifiant-cle v1\0");
        h.update(maitre);
        let empreinte = h.finalize();
        let mut identifiant = [0u8; 8];
        identifiant.copy_from_slice(&empreinte[..8]);
        let cle = Self {
            totp: sous_cle(&maitre, b"totp v1"),
            enrolement: sous_cle(&maitre, b"enrolement v1"),
            enumeration: sous_cle(&maitre, b"enumeration v1"),
            identifiant,
        };
        maitre.zeroize();
        cle
    }

    /// Lit la clé dans le fichier `chemin` : fichier ordinaire, sans aucun droit pour le
    /// groupe ni pour les autres (0600 ou 0400), contenant 32 octets en base64 (espaces de
    /// début et de fin ignorés).
    ///
    /// # Errors
    /// [`ErreurCle`].
    pub fn lire_fichier(chemin: &Path) -> Result<Self, ErreurCle> {
        verifier_droits(chemin)?;
        let texte = Zeroizing::new(std::fs::read(chemin).map_err(|_| ErreurCle::Illisible)?);
        let texte = std::str::from_utf8(&texte).map_err(|_| ErreurCle::Format)?;
        let mut maitre = [0u8; OCTETS_CLE];
        let lus = Base64::decode(texte.trim(), &mut maitre).map_err(|_| ErreurCle::Format)?;
        if lus.len() != OCTETS_CLE {
            maitre.zeroize();
            return Err(ErreurCle::Format);
        }
        Ok(Self::depuis_octets(maitre))
    }

    /// Identifiant de la clé (public).
    #[must_use]
    pub const fn identifiant(&self) -> [u8; 8] {
        self.identifiant
    }

    fn donnees_associees(&self, usage: &[u8], identifiants: &[&[u8]]) -> Vec<u8> {
        let mut aad = Vec::with_capacity(64);
        aad.extend_from_slice(usage);
        for id in identifiants {
            aad.extend_from_slice(id);
        }
        aad.extend_from_slice(&self.identifiant);
        aad
    }

    /// Chiffre le secret TOTP de `utilisateur` : `nonce ‖ chiffré ‖ étiquette`
    /// ([`OCTETS_TOTP_CHIFFRE`] octets).
    ///
    /// # Errors
    /// [`ErreurAlea`] si l'aléa du nonce manque.
    pub fn chiffrer_totp(
        &self,
        utilisateur: Uuid,
        secret: &SecretTotp,
    ) -> Result<Vec<u8>, ErreurAlea> {
        let aad = self.donnees_associees(b"sigma-books totp v1\0", &[utilisateur.as_bytes()]);
        sceller(&self.totp, &aad, secret.octets())
    }

    /// Déchiffre le secret TOTP de `utilisateur`, chiffré par la clé d'identifiant `cle_id`.
    /// `None` si la clé n'est pas la clé courante, si le secret est d'un autre utilisateur ou
    /// altéré.
    #[must_use]
    pub fn dechiffrer_totp(
        &self,
        utilisateur: Uuid,
        cle_id: &[u8],
        chiffre: &[u8],
    ) -> Option<SecretTotp> {
        if cle_id != self.identifiant {
            return None;
        }
        let aad = self.donnees_associees(b"sigma-books totp v1\0", &[utilisateur.as_bytes()]);
        let clair = Zeroizing::new(ouvrir(&self.totp, &aad, chiffre)?);
        let octets: [u8; OCTETS_SECRET] = clair.as_slice().try_into().ok()?;
        Some(SecretTotp::depuis_octets(octets))
    }

    /// Scelle le secret TOTP d'un enrôlement en cours dans un jeton remis au client : lié à
    /// l'utilisateur, à la session et à l'échéance (secondes Unix). Rien n'est stocké entre la
    /// préparation et la confirmation.
    ///
    /// # Errors
    /// [`ErreurAlea`] si l'aléa du nonce manque.
    pub fn sceller_enrolement(
        &self,
        utilisateur: Uuid,
        session: Uuid,
        echeance: i64,
        secret: &SecretTotp,
    ) -> Result<String, ErreurAlea> {
        let aad = self.donnees_associees(
            b"sigma-books enrolement v1\0",
            &[
                utilisateur.as_bytes(),
                session.as_bytes(),
                &echeance.to_be_bytes(),
            ],
        );
        let scelle = sceller(&self.enrolement, &aad, secret.octets())?;
        Ok(base64ct::Base64UrlUnpadded::encode_string(&scelle))
    }

    /// Ouvre un jeton d'enrôlement de [`CleInstance::sceller_enrolement`]. `None` s'il n'est
    /// pas de cet utilisateur, de cette session et de cette échéance, ou s'il est altéré.
    #[must_use]
    pub fn ouvrir_enrolement(
        &self,
        utilisateur: Uuid,
        session: Uuid,
        echeance: i64,
        jeton: &str,
    ) -> Option<SecretTotp> {
        if jeton.len() > 128 {
            return None;
        }
        let scelle = base64ct::Base64UrlUnpadded::decode_vec(jeton).ok()?;
        let aad = self.donnees_associees(
            b"sigma-books enrolement v1\0",
            &[
                utilisateur.as_bytes(),
                session.as_bytes(),
                &echeance.to_be_bytes(),
            ],
        );
        let clair = Zeroizing::new(ouvrir(&self.enrolement, &aad, &scelle)?);
        let octets: [u8; OCTETS_SECRET] = clair.as_slice().try_into().ok()?;
        Some(SecretTotp::depuis_octets(octets))
    }

    /// Clé de limitation de débit d'un courriel : HMAC-SHA256 par la sous-clé d'énumération,
    /// tronqué à 128 bits. Le courriel lui-même n'est pas gardé en mémoire.
    #[must_use]
    pub fn cle_de_debit(&self, courriel: &str) -> [u8; 16] {
        use hmac::{Hmac, Mac};
        let mut cle = [0u8; 16];
        if let Ok(mut mac) = <Hmac<Sha256> as KeyInit>::new_from_slice(&self.enumeration) {
            mac.update(courriel.to_lowercase().as_bytes());
            cle.copy_from_slice(&mac.finalize().into_bytes()[..16]);
        }
        cle
    }
}

impl CleInstance {
    /// Octets stables et imprévisibles, de la `longueur` demandée, dérivés d'un courriel et d'un
    /// rang : HMAC-SHA256 par la sous-clé d'énumération, par blocs numérotés. Ils fournissent les
    /// **faux** identifiants de passkey rendus pour un courriel inconnu ou sans passkey
    /// (anti-énumération) : toujours les mêmes pour un même courriel, indiscernables d'identifiants
    /// réels sans la clé de l'instance.
    #[must_use]
    pub fn octets_factices(&self, courriel: &str, rang: u8, longueur: usize) -> Vec<u8> {
        use hmac::{Hmac, Mac};
        let mut sortie = Vec::with_capacity(longueur + 32);
        let mut bloc = 0u8;
        while sortie.len() < longueur {
            if let Ok(mut mac) = <Hmac<Sha256> as KeyInit>::new_from_slice(&self.enumeration) {
                mac.update(b"passkey-factice v2\0");
                mac.update(&[rang, bloc]);
                mac.update(courriel.to_lowercase().as_bytes());
                sortie.extend_from_slice(&mac.finalize().into_bytes());
            } else {
                sortie.extend_from_slice(&[0u8; 32]);
            }
            bloc = bloc.wrapping_add(1);
        }
        sortie.truncate(longueur);
        sortie
    }
}

/// AES-256-GCM : `nonce ‖ chiffré ‖ étiquette`.
fn sceller(cle: &[u8; OCTETS_CLE], aad: &[u8], clair: &[u8]) -> Result<Vec<u8>, ErreurAlea> {
    let mut nonce = [0u8; OCTETS_NONCE];
    tirer(&mut nonce)?;
    let chiffreur = Aes256Gcm::new_from_slice(cle).map_err(|_| ErreurAlea)?;
    let chiffre = chiffreur
        .encrypt(&Nonce::from(nonce), Payload { msg: clair, aad })
        .map_err(|_| ErreurAlea)?;
    let mut sortie = Vec::with_capacity(OCTETS_NONCE + chiffre.len());
    sortie.extend_from_slice(&nonce);
    sortie.extend_from_slice(&chiffre);
    Ok(sortie)
}

/// Ouvre `nonce ‖ chiffré ‖ étiquette`, ou `None`.
fn ouvrir(cle: &[u8; OCTETS_CLE], aad: &[u8], scelle: &[u8]) -> Option<Vec<u8>> {
    if scelle.len() < OCTETS_NONCE + OCTETS_ETIQUETTE {
        return None;
    }
    let (nonce, chiffre) = scelle.split_at(OCTETS_NONCE);
    let nonce: [u8; OCTETS_NONCE] = nonce.try_into().ok()?;
    let chiffreur = Aes256Gcm::new_from_slice(cle).ok()?;
    chiffreur
        .decrypt(&Nonce::from(nonce), Payload { msg: chiffre, aad })
        .ok()
}

#[cfg(unix)]
fn verifier_droits(chemin: &Path) -> Result<(), ErreurCle> {
    use std::os::unix::fs::PermissionsExt;
    // Le lien symbolique est suivi : ce sont les droits du fichier lu qui comptent.
    let meta = std::fs::metadata(chemin).map_err(|_| ErreurCle::Illisible)?;
    if !meta.is_file() || meta.permissions().mode() & 0o077 != 0 {
        return Err(ErreurCle::DroitsTropLarges);
    }
    Ok(())
}

#[cfg(not(unix))]
fn verifier_droits(_chemin: &Path) -> Result<(), ErreurCle> {
    // Droits non vérifiables : refus (l'instance tourne sous Linux, Fondations §3.3).
    Err(ErreurCle::DroitsTropLarges)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cle(octet: u8) -> CleInstance {
        CleInstance::depuis_octets([octet; OCTETS_CLE])
    }

    #[test]
    fn le_secret_totp_ne_se_dechiffre_que_pour_son_utilisateur_et_sa_cle() {
        let k = cle(1);
        let (u, v) = (Uuid::new_v4(), Uuid::new_v4());
        let secret = SecretTotp::depuis_octets([9; OCTETS_SECRET]);
        let chiffre = k.chiffrer_totp(u, &secret).unwrap();
        assert_eq!(chiffre.len(), OCTETS_TOTP_CHIFFRE);
        assert!(!chiffre.windows(OCTETS_SECRET).any(|w| w == [9; 20]));
        let id = k.identifiant();
        assert_eq!(
            k.dechiffrer_totp(u, &id, &chiffre).unwrap().octets(),
            &[9; 20]
        );
        // Données associées : un autre utilisateur ne le lit pas.
        assert!(k.dechiffrer_totp(v, &id, &chiffre).is_none());
        // Sans la clé (autre clé de l'instance), illisible, même en forçant l'identifiant.
        let autre = cle(2);
        assert!(autre.dechiffrer_totp(u, &id, &chiffre).is_none());
        assert!(
            autre
                .dechiffrer_totp(u, &autre.identifiant(), &chiffre)
                .is_none()
        );
        // Altéré : refusé.
        let mut altere = chiffre.clone();
        altere[20] ^= 1;
        assert!(k.dechiffrer_totp(u, &id, &altere).is_none());
        assert!(k.dechiffrer_totp(u, &id, &chiffre[..20]).is_none());
        // Deux chiffrements du même secret diffèrent (nonce aléatoire).
        assert_ne!(k.chiffrer_totp(u, &secret).unwrap(), chiffre);
    }

    #[test]
    fn le_jeton_d_enrolement_est_lie_a_l_utilisateur_a_la_session_et_a_l_echeance() {
        let k = cle(3);
        let (u, s) = (Uuid::new_v4(), Uuid::new_v4());
        let secret = SecretTotp::depuis_octets([5; OCTETS_SECRET]);
        let jeton = k.sceller_enrolement(u, s, 1000, &secret).unwrap();
        assert_eq!(
            k.ouvrir_enrolement(u, s, 1000, &jeton).unwrap().octets(),
            &[5; 20]
        );
        assert!(
            k.ouvrir_enrolement(Uuid::new_v4(), s, 1000, &jeton)
                .is_none()
        );
        assert!(
            k.ouvrir_enrolement(u, Uuid::new_v4(), 1000, &jeton)
                .is_none()
        );
        assert!(k.ouvrir_enrolement(u, s, 1001, &jeton).is_none());
        assert!(cle(4).ouvrir_enrolement(u, s, 1000, &jeton).is_none());
        assert!(k.ouvrir_enrolement(u, s, 1000, "pas du base64 !").is_none());
        // Un secret TOTP chiffré n'est pas un jeton d'enrôlement (sous-clés distinctes).
        let chiffre = k.chiffrer_totp(u, &secret).unwrap();
        let faux = base64ct::Base64UrlUnpadded::encode_string(&chiffre);
        assert!(k.ouvrir_enrolement(u, s, 1000, &faux).is_none());
    }

    #[test]
    fn identifiant_debug_et_cle_de_debit() {
        let (a, b) = (cle(1), cle(2));
        assert_ne!(a.identifiant(), b.identifiant());
        assert_eq!(a, cle(1));
        assert_ne!(a, b);
        let debug = format!("{a:?}");
        assert!(debug.starts_with("CleInstance(id "));
        assert!(!debug.contains("0101"));
        assert_eq!(
            a.cle_de_debit("Utilisateur@Exemple.invalid"),
            a.cle_de_debit("utilisateur@exemple.invalid")
        );
        assert_ne!(a.cle_de_debit("x@y.z"), b.cle_de_debit("x@y.z"));
    }

    #[cfg(unix)]
    #[test]
    fn le_fichier_de_cle_exige_des_droits_restreints_et_32_octets() {
        use std::os::unix::fs::PermissionsExt;
        let repertoire = std::env::temp_dir().join(format!("sigma-cle-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&repertoire).unwrap();
        let ecrire = |nom: &str, contenu: &str, mode: u32| {
            let chemin = repertoire.join(nom);
            std::fs::write(&chemin, contenu).unwrap();
            std::fs::set_permissions(&chemin, std::fs::Permissions::from_mode(mode)).unwrap();
            chemin
        };
        let valide = Base64::encode_string(&[7u8; 32]);
        let bonne = ecrire("bonne", &format!("{valide}\n"), 0o600);
        assert_eq!(
            CleInstance::lire_fichier(&bonne).unwrap(),
            CleInstance::depuis_octets([7; 32])
        );
        let lecture_seule = ecrire("lecture-seule", &valide, 0o400);
        assert!(CleInstance::lire_fichier(&lecture_seule).is_ok());
        for mode in [0o640, 0o604, 0o644, 0o660] {
            let chemin = ecrire(&format!("mode-{mode:o}"), &valide, mode);
            assert_eq!(
                CleInstance::lire_fichier(&chemin),
                Err(ErreurCle::DroitsTropLarges),
                "{mode:o}"
            );
        }
        let courte = ecrire("courte", &Base64::encode_string(&[7u8; 16]), 0o600);
        assert_eq!(CleInstance::lire_fichier(&courte), Err(ErreurCle::Format));
        let texte = ecrire("texte", "pas une clé", 0o600);
        assert_eq!(CleInstance::lire_fichier(&texte), Err(ErreurCle::Format));
        assert_eq!(
            CleInstance::lire_fichier(&repertoire.join("absente")),
            Err(ErreurCle::Illisible)
        );
        assert_eq!(
            CleInstance::lire_fichier(&repertoire),
            Err(ErreurCle::DroitsTropLarges)
        );
        std::fs::remove_dir_all(&repertoire).unwrap();
    }
}
