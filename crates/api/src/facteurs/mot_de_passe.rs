//! Mot de passe de la méthode de secours (ADR 0012 ; NIST SP 800-63B §5.1.1.2 ; ASVS 4.0.3
//! V2.1, V2.4).
//!
//! - **Règles** : 12 caractères au moins (caractères Unicode, sur la forme normalisée : blancs
//!   de tête et de fin retirés, suites internes réduites à une espace), 128 au plus
//!   (borne du coût du hachage), aucune règle de composition, refus de tout mot de passe de la
//!   liste embarquée des mots de passe compromis ([`super::compromis`]).
//! - **Hachage** : Argon2id, 19 Mio de mémoire, 2 passes, 1 voie (repère OWASP, ADR 0012), sel
//!   de 128 bits tiré du système ; chaîne PHC stockée (`$argon2id$v=19$m=19456,t=2,p=1$…`).
//! - **Vérification** : selon les paramètres de la chaîne stockée ; si ceux-ci ne sont plus
//!   les paramètres courants, l'appelant re-hache le mot de passe vérifié
//!   ([`Verification::Admis`]).
//!
//! Le hachage coûte du temps de calcul et de la mémoire : l'appelant le fait hors de la
//! boucle asynchrone (`spawn_blocking`) et en borne le nombre simultané.

use argon2::password_hash::phc::PasswordHash;
use argon2::password_hash::{PasswordHasher, PasswordVerifier};
use argon2::{Algorithm, Argon2, Params, Version};

use super::compromis;
use super::{ErreurAlea, tirer};

/// Longueur minimale, en caractères (ADR 0012).
pub const LONGUEUR_MIN: usize = 12;
/// Longueur maximale, en caractères.
pub const LONGUEUR_MAX: usize = 128;
/// Mémoire d'Argon2id, en Kio (19 Mio).
pub const MEMOIRE_KIO: u32 = 19 * 1024;
/// Passes d'Argon2id.
pub const PASSES: u32 = 2;
/// Voies d'Argon2id.
pub const VOIES: u32 = 1;

/// Motif du refus d'un nouveau mot de passe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefusMotDePasse {
    /// Moins de [`LONGUEUR_MIN`] caractères.
    TropCourt,
    /// Plus de [`LONGUEUR_MAX`] caractères.
    TropLong,
    /// Présent dans la liste des mots de passe compromis.
    Compromis,
}

/// Contrôle un nouveau mot de passe (création ou changement).
///
/// # Errors
/// [`RefusMotDePasse`].
pub fn valider(mot_de_passe: &str) -> Result<(), RefusMotDePasse> {
    // Minimum compté sur la forme normalisée (blancs de tête et de fin retirés, suites internes
    // réduites) : des espaces ajoutées ne font pas d'un mot de passe court un mot de passe long,
    // ni d'un mot de passe compromis un mot de passe nouveau.
    if compromis::normaliser(mot_de_passe).chars().count() < LONGUEUR_MIN {
        return Err(RefusMotDePasse::TropCourt);
    }
    if mot_de_passe.chars().count() > LONGUEUR_MAX {
        return Err(RefusMotDePasse::TropLong);
    }
    if compromis::est_compromis(mot_de_passe) {
        return Err(RefusMotDePasse::Compromis);
    }
    Ok(())
}

/// Le hacheur aux paramètres courants.
fn argon2() -> Result<Argon2<'static>, ErreurHachage> {
    let params = Params::new(MEMOIRE_KIO, PASSES, VOIES, None).map_err(|_| ErreurHachage)?;
    Ok(Argon2::new(Algorithm::Argon2id, Version::V0x13, params))
}

/// Échec du hachage (aléa du sel indisponible, paramètres refusés).
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("hachage du mot de passe impossible")]
pub struct ErreurHachage;

impl From<ErreurAlea> for ErreurHachage {
    fn from(_: ErreurAlea) -> Self {
        Self
    }
}

/// Hache un mot de passe : chaîne PHC d'Argon2id aux paramètres courants.
///
/// # Errors
/// [`ErreurHachage`].
pub fn hacher(mot_de_passe: &str) -> Result<String, ErreurHachage> {
    let mut sel = [0u8; 16];
    tirer(&mut sel)?;
    let empreinte = argon2()?
        .hash_password_with_salt(mot_de_passe.as_bytes(), &sel)
        .map_err(|_| ErreurHachage)?;
    Ok(empreinte.to_string())
}

/// Résultat d'une vérification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verification {
    /// Mot de passe faux, ou chaîne stockée illisible.
    Refuse,
    /// Mot de passe juste ; `a_rehacher` si la chaîne stockée n'a pas les paramètres courants.
    Admis {
        /// Paramètres périmés : re-hacher le mot de passe vérifié.
        a_rehacher: bool,
    },
}

/// Vérifie `mot_de_passe` contre la chaîne PHC `stockee`. Un mot de passe trop long est
/// refusé sans hachage (sa longueur ne dit rien du compte).
#[must_use]
pub fn verifier(stockee: &str, mot_de_passe: &str) -> Verification {
    if mot_de_passe.chars().count() > LONGUEUR_MAX {
        return Verification::Refuse;
    }
    let Ok(empreinte) = PasswordHash::new(stockee) else {
        return Verification::Refuse;
    };
    // Seul Argon2id est admis : une chaîne d'un autre algorithme est refusée.
    if empreinte.algorithm != Algorithm::Argon2id.ident() {
        return Verification::Refuse;
    }
    let Ok(hacheur) = argon2() else {
        return Verification::Refuse;
    };
    if hacheur
        .verify_password(mot_de_passe.as_bytes(), &empreinte)
        .is_err()
    {
        return Verification::Refuse;
    }
    let a_rehacher = Params::try_from(&empreinte)
        .is_ok_and(|p| p.m_cost() != MEMOIRE_KIO || p.t_cost() != PASSES || p.p_cost() != VOIES)
        || empreinte.version != Some(Version::V0x13.into());
    Verification::Admis { a_rehacher }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn longueur_et_liste_des_mots_de_passe_compromis() {
        assert_eq!(valider("onze caract"), Err(RefusMotDePasse::TropCourt));
        assert_eq!(valider(""), Err(RefusMotDePasse::TropCourt));
        // Douze caractères Unicode (et non douze octets).
        assert_eq!(valider("éléphant été"), Ok(()));
        assert_eq!(valider("ééééééééééé"), Err(RefusMotDePasse::TropCourt));
        assert_eq!(valider(&"a".repeat(129)), Err(RefusMotDePasse::TropLong));
        assert_eq!(valider(&"a b".repeat(40)), Ok(()));
        // Présents dans SecLists (Pwdb_top-1000000), quelle que soit la casse ASCII.
        for compromis in ["q1w2e3r4t5y6", "1qaz2wsx3edc", "Q1W2E3R4T5Y6"] {
            assert_eq!(
                valider(compromis),
                Err(RefusMotDePasse::Compromis),
                "{compromis}"
            );
        }
        assert_eq!(valider("cheval agrafe pile correcte"), Ok(()));
        // Espaces ajoutées : ni plus long, ni nouveau.
        assert_eq!(valider(" q1w2e3r4t5y6"), Err(RefusMotDePasse::Compromis));
        assert_eq!(valider("  q1w2e3r4t5y6 "), Err(RefusMotDePasse::Compromis));
        assert_eq!(valider("password    "), Err(RefusMotDePasse::TropCourt));
    }

    #[test]
    fn hachage_argon2id_aux_parametres_de_l_adr() {
        let phc = hacher("cheval agrafe pile correcte").unwrap();
        assert!(phc.starts_with("$argon2id$v=19$m=19456,t=2,p=1$"), "{phc}");
        assert!(!phc.contains("cheval"));
        assert_eq!(
            verifier(&phc, "cheval agrafe pile correcte"),
            Verification::Admis { a_rehacher: false }
        );
        assert_eq!(
            verifier(&phc, "cheval agrafe pile correctE"),
            Verification::Refuse
        );
        assert_eq!(verifier(&phc, &"a".repeat(129)), Verification::Refuse);
        assert_eq!(verifier("illisible", "x"), Verification::Refuse);
        // Deux hachages du même mot de passe diffèrent (sel).
        assert_ne!(hacher("cheval agrafe pile correcte").unwrap(), phc);
    }

    #[test]
    fn une_chaine_aux_parametres_perimes_est_a_rehacher() {
        // Argon2id plus faible (m = 8 Mio, t = 1), puis Argon2i : refusé.
        let faible = Argon2::new(
            Algorithm::Argon2id,
            Version::V0x13,
            Params::new(8 * 1024, 1, 1, None).unwrap(),
        );
        let sel = [1u8; 16];
        let phc = faible
            .hash_password_with_salt(b"ancien mot de passe", &sel)
            .unwrap()
            .to_string();
        assert_eq!(
            verifier(&phc, "ancien mot de passe"),
            Verification::Admis { a_rehacher: true }
        );
        let argon2i = Argon2::new(
            Algorithm::Argon2i,
            Version::V0x13,
            Params::new(MEMOIRE_KIO, PASSES, VOIES, None).unwrap(),
        );
        let phc = argon2i
            .hash_password_with_salt(b"ancien mot de passe", &sel)
            .unwrap()
            .to_string();
        assert_eq!(verifier(&phc, "ancien mot de passe"), Verification::Refuse);
    }

    /// Compatibilité de format : une chaîne PHC produite avant la montée en `argon2` 0.6
    /// (`password-hash` 0.5) se vérifie toujours.
    ///
    /// Vecteur de référence public, cité tel quel : implémentation de référence d'Argon2
    /// (P-H-C/phc-winner-argon2, `src/test.c`, cas Argon2id v=19, t=2, m=2^8 Kio, p=1, mot de
    /// passe « password », sel « somesalt »), repris dans les tests de connaissance de la crate
    /// `argon2` (`tests/kat.rs`) et dans le brouillon IRTF/RFC 9106. Ce n'est ni un secret ni
    /// une valeur de l'application.
    #[test]
    fn une_chaine_phc_de_reference_publique_reste_verifiable() {
        const REFERENCE: &str =
            "$argon2id$v=19$m=256,t=2,p=1$c29tZXNhbHQ$nf65EOgLrQMR/uIPnA4rEsF5h7TKyQwu9U1bMCHGi/4";
        // Paramètres plus faibles que les courants : admis, à re-hacher.
        assert_eq!(
            verifier(REFERENCE, "password"),
            Verification::Admis { a_rehacher: true }
        );
        assert_eq!(verifier(REFERENCE, "passwore"), Verification::Refuse);
        // Le format produit aujourd'hui se relit : mêmes champs que la référence.
        let phc = hacher("cheval agrafe pile correcte").unwrap();
        let relu = PasswordHash::new(&phc).unwrap();
        assert_eq!(relu.algorithm, Algorithm::Argon2id.ident());
        assert_eq!(relu.version, Some(Version::V0x13.into()));
        assert_eq!(relu.to_string(), phc);
    }
}
