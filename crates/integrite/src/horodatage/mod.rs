//! Horodatage RFC 3161 de la tête de chaîne (spec §6.4, ADR 0007 et 0010).
//!
//! Le **témoin** décrit l'état de la chaîne d'un dossier à un instant : identifiant du dossier,
//! empreinte de tête, nombre d'écritures. Son empreinte SHA-256 est soumise à une autorité
//! d'horodatage (TSA), qui renvoie un **jeton** signé : la preuve que la chaîne avait cet état
//! au plus tard à la date du jeton. Toute réécriture ultérieure de l'export, même avec une
//! chaîne entièrement recalculée, ne correspond plus au jeton.
//!
//! Ce module est **pur** : il construit la requête et vérifie la réponse, sans réseau ni
//! horloge (l'envoi de la requête relève du worker, lot L2). Le nonce et la date sont fournis
//! par l'appelant.

mod ancres;
mod jeton;
mod planification;

use std::fmt;

use der::Encode;
use der::asn1::{Int, OctetString};
use der::oid::ObjectIdentifier;
use sha2::{Digest, Sha256};
use x509_cert::spki::AlgorithmIdentifier;
use x509_tsp::{MessageImprint, TimeStampReq, TspVersion};

use crate::chaine::{Empreinte, IdDossier};

pub use ancres::{Ancre, ErreurAncre, ancres_par_defaut};
pub use jeton::{ErreurJeton, JetonVerifie, verifier_jeton, verifier_reponse};
pub use planification::{Declencheur, DernierJeton, horodatage_requis};

/// OID de SHA-256 (NIST).
pub(crate) const OID_SHA256: ObjectIdentifier =
    ObjectIdentifier::new_unwrap("2.16.840.1.101.3.4.2.1");

/// Taille maximale acceptée pour un jeton ou une réponse (les jetons réels font 5 à 7 Kio).
pub const JETON_MAX: usize = 64 * 1024;

/// État de la chaîne d'un dossier soumis à l'horodatage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Temoin {
    dossier: IdDossier,
    tete: Empreinte,
    nombre: u64,
}

impl Temoin {
    /// Témoin d'une chaîne de `nombre` écritures dont la tête est `tete`.
    #[must_use]
    pub fn nouveau(dossier: IdDossier, tete: Empreinte, nombre: u64) -> Self {
        Self {
            dossier,
            tete,
            nombre,
        }
    }

    /// Dossier.
    #[must_use]
    pub fn dossier(&self) -> &IdDossier {
        &self.dossier
    }

    /// Empreinte de tête.
    #[must_use]
    pub fn tete(&self) -> &Empreinte {
        &self.tete
    }

    /// Nombre d'écritures chaînées.
    #[must_use]
    pub fn nombre(&self) -> u64 {
        self.nombre
    }

    /// Texte canonique du témoin (format figé, `docs/integrite/format-v2.md` §4).
    #[must_use]
    pub fn contenu(&self) -> String {
        format!(
            "sigma-books temoin v1\ndossier={}\ntete={}\nnombre={}\n",
            self.dossier, self.tete, self.nombre
        )
    }

    /// Empreinte SHA-256 du texte canonique : la valeur horodatée.
    #[must_use]
    pub fn empreinte(&self) -> [u8; 32] {
        Sha256::digest(self.contenu().as_bytes()).into()
    }
}

impl fmt::Display for Temoin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "dossier {}, {} écriture(s), tête {}",
            self.dossier, self.nombre, self.tete
        )
    }
}

/// Erreur de construction d'une requête (encodage DER).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("encodage de la requête d'horodatage : {0}")]
pub struct ErreurRequete(String);

/// Requête RFC 3161 (`TimeStampReq`, DER) pour un témoin : empreinte SHA-256, nonce, et
/// demande du certificat de la TSA dans le jeton (nécessaire à la vérification hors ligne).
///
/// # Errors
/// [`ErreurRequete`] si l'encodage échoue (non attendu).
pub fn requete(temoin: &Temoin, nonce: u64) -> Result<Vec<u8>, ErreurRequete> {
    let erreur = |e: der::Error| ErreurRequete(e.to_string());
    let req = TimeStampReq {
        version: TspVersion::V1,
        message_imprint: MessageImprint {
            hash_algorithm: AlgorithmIdentifier {
                oid: OID_SHA256,
                parameters: Some(der::Any::null()),
            },
            hashed_message: OctetString::new(temoin.empreinte()).map_err(erreur)?,
        },
        req_policy: None,
        nonce: Some(entier(nonce).map_err(erreur)?),
        cert_req: true,
        extensions: None,
    };
    req.to_der().map_err(erreur)
}

/// Entier DER positif minimal (zéro de tête si le bit de poids fort est à 1).
pub(crate) fn entier(valeur: u64) -> der::Result<Int> {
    let octets = valeur.to_be_bytes();
    let debut = octets.iter().position(|&b| b != 0).unwrap_or(7);
    let mut minimal = Vec::with_capacity(9);
    if octets[debut] & 0x80 != 0 {
        minimal.push(0);
    }
    minimal.extend_from_slice(&octets[debut..]);
    Int::new(&minimal)
}

#[cfg(test)]
mod tests {
    use super::*;
    use der::Decode;

    fn temoin() -> Temoin {
        let dossier = IdDossier::parse("demo").unwrap();
        let tete = crate::chaine::genese(&dossier);
        Temoin::nouveau(dossier, tete, 7)
    }

    #[test]
    fn temoin_accesseurs_et_affichage() {
        let t = temoin();
        assert_eq!(t.dossier().as_str(), "demo");
        assert_eq!(t.nombre(), 7);
        assert_eq!(*t.tete(), crate::chaine::genese(t.dossier()));
        assert!(
            t.to_string()
                .starts_with("dossier demo, 7 écriture(s), tête ")
        );
        assert_eq!(t.empreinte(), <[u8; 32]>::from(Sha256::digest(t.contenu())));
    }

    #[test]
    fn entiers_der_minimaux() {
        for (valeur, attendu) in [
            (0u64, vec![0x02, 0x01, 0x00]),
            (0x7f, vec![0x02, 0x01, 0x7f]),
            (0x80, vec![0x02, 0x02, 0x00, 0x80]),
            (u64::MAX, [vec![0x02, 0x09, 0x00], vec![0xff; 8]].concat()),
        ] {
            let der = entier(valeur).unwrap().to_der().unwrap();
            assert_eq!(der, attendu, "{valeur:#x}");
            assert!(Int::from_der(&der).is_ok());
        }
    }

    #[test]
    fn requete_par_nonce() {
        let a = requete(&temoin(), 1).unwrap();
        let b = requete(&temoin(), 2).unwrap();
        assert_ne!(a, b);
        assert!(TimeStampReq::from_der(&a).unwrap().cert_req);
    }
}
