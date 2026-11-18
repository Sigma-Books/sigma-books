//! Ancres de confiance : certificats racines des autorités d'horodatage (ADR 0010).
//!
//! Une ancre est retenue par son **nom** et sa **clé publique**. Une chaîne est acceptée dès
//! qu'un certificat est signé par la clé d'une ancre portant le nom de son émetteur : les
//! versions contresignées d'une racine (fournies dans les jetons) ont la même clé.
//!
//! Racines intégrées (voir `ancres/README.md` pour leur origine et leurs empreintes) :
//! - `DigiCert Trusted Root G4` (autorité par défaut) ;
//! - `Sectigo Public Time Stamping Root R46` (autorité de secours).

use der::{Decode, Encode};
use x509_cert::Certificate;
use x509_cert::name::Name;
use x509_cert::spki::SubjectPublicKeyInfoOwned;

const DIGICERT_G4: &[u8] = include_bytes!("ancres/digicert-trusted-root-g4.der");
const SECTIGO_R46: &[u8] = include_bytes!("ancres/sectigo-public-time-stamping-root-r46.der");

/// Erreur de lecture d'une ancre.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("certificat d'ancre illisible : {0}")]
pub struct ErreurAncre(String);

/// Ancre de confiance : nom et clé publique d'une racine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ancre {
    nom: Name,
    cle: SubjectPublicKeyInfoOwned,
}

impl Ancre {
    /// Lit une ancre depuis un certificat DER.
    ///
    /// # Errors
    /// [`ErreurAncre`] si le certificat est illisible.
    pub fn depuis_der(der: &[u8]) -> Result<Self, ErreurAncre> {
        let certificat = Certificate::from_der(der).map_err(|e| ErreurAncre(e.to_string()))?;
        Ok(Self::depuis_certificat(&certificat))
    }

    /// Lit une ancre depuis un ou plusieurs certificats PEM.
    ///
    /// # Errors
    /// [`ErreurAncre`] si le texte ne contient aucun certificat lisible.
    pub fn depuis_pem(pem: &[u8]) -> Result<Vec<Self>, ErreurAncre> {
        let certificats =
            Certificate::load_pem_chain(pem).map_err(|e| ErreurAncre(e.to_string()))?;
        if certificats.is_empty() {
            return Err(ErreurAncre("aucun certificat".to_owned()));
        }
        Ok(certificats.iter().map(Self::depuis_certificat).collect())
    }

    fn depuis_certificat(certificat: &Certificate) -> Self {
        let tbs = certificat.tbs_certificate();
        Self {
            nom: tbs.subject().clone(),
            cle: tbs.subject_public_key_info().clone(),
        }
    }

    /// Nom de la racine.
    #[must_use]
    pub fn nom(&self) -> &Name {
        &self.nom
    }

    /// Clé publique de la racine.
    #[must_use]
    pub fn cle(&self) -> &SubjectPublicKeyInfoOwned {
        &self.cle
    }

    /// Vrai si la clé publique est celle donnée (comparaison des encodages DER).
    pub(crate) fn a_la_cle(&self, cle: &SubjectPublicKeyInfoOwned) -> bool {
        matches!((self.cle.to_der(), cle.to_der()), (Ok(a), Ok(b)) if a == b)
    }
}

/// Ancres intégrées : racines de `DigiCert` et de `Sectigo` pour l'horodatage.
///
/// # Errors
/// [`ErreurAncre`] si un certificat intégré est illisible (vérifié par les tests).
pub fn ancres_par_defaut() -> Result<Vec<Ancre>, ErreurAncre> {
    [DIGICERT_G4, SECTIGO_R46]
        .into_iter()
        .map(Ancre::depuis_der)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64ct::{Base64, Encoding};

    #[test]
    fn ancres_der_et_pem_equivalentes() {
        let der = Ancre::depuis_der(SECTIGO_R46).unwrap();
        let mut pem = String::from("-----BEGIN CERTIFICATE-----\n");
        for ligne in Base64::encode_string(SECTIGO_R46).as_bytes().chunks(64) {
            pem.push_str(std::str::from_utf8(ligne).unwrap());
            pem.push('\n');
        }
        pem.push_str("-----END CERTIFICATE-----\n");
        assert_eq!(
            Ancre::depuis_pem(pem.as_bytes()).unwrap(),
            std::slice::from_ref(&der)
        );
        assert!(
            der.nom()
                .to_string()
                .contains("Sectigo Public Time Stamping Root R46")
        );
        assert!(der.a_la_cle(der.cle()));
        let autre = Ancre::depuis_der(DIGICERT_G4).unwrap();
        assert!(!der.a_la_cle(autre.cle()));
    }
}
