//! Autorité d'horodatage **locale** de la démonstration de bout en bout (L2-37) : elle signe de
//! vrais jetons RFC 3161 (`TimeStampToken`, CMS `SignedData`, RSA PKCS#1 v1.5 SHA-256) pour le
//! témoin du dossier, avec une clé et un certificat **tirés à chaque exécution** (aucune valeur
//! cryptographique n'est écrite dans le dépôt). Aucun accès réseau.
//!
//! Le certificat est auto-signé (usage étendu unique et critique `id-kp-timeStamping`) : il sert
//! d'ancre de confiance à `sigma-verifier` (option `--ancre`), comme le ferait une racine de
//! `DigiCert` ou de `Sectigo`. La date des jetons est celle de l'horloge injectée.
//!
//! Déclaré par `demonstration.rs` seulement (`#[path]`) : les autres tests n'en ont pas besoin.

#![allow(clippy::expect_used, clippy::unwrap_used, dead_code)]

use std::future::Future;
use std::pin::Pin;
use std::str::FromStr;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use api::horodatage::{Autorite, ErreurHorodatage, Horodateur, JetonObtenu, Nonce};
use api::serveur::Horloge;
use cms::cert::{CertificateChoices, IssuerAndSerialNumber};
use cms::content_info::{CmsVersion, ContentInfo};
use cms::signed_data::{
    CertificateSet, EncapsulatedContentInfo, SignedAttributes, SignedData, SignerIdentifier,
    SignerInfo, SignerInfos,
};
use crypto_glue::rsa::{MIN_BITS, RS256SigningKey, new_key};
use crypto_glue::traits::{Keypair, SignatureEncoding, Signer};
use crypto_glue::x509::profile::BuilderProfile;
use crypto_glue::x509::{
    Builder, Certificate, CertificateBuilder, ExtendedKeyUsage, Extension, Name, ObjectIdentifier,
    OctetString, SerialNumber, SubjectPublicKeyInfoOwned, Time, Validity,
};
use der::asn1::{Int, SetOfVec};
use der::{Any, Encode, Sequence};
use integrite::horodatage::{Ancre, Temoin, verifier_jeton};
use sha2::{Digest, Sha256};
use spki::AlgorithmIdentifierOwned;
use x509_tsp::{GeneralizedTimeNanos, MessageImprint, TspVersion, TstInfo};

const OID_SIGNED_DATA: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.7.2");
const OID_TST_INFO: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.9.16.1.4");
const OID_CONTENT_TYPE: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.9.3");
const OID_MESSAGE_DIGEST: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.9.4");
const OID_SIGNING_CERTIFICATE_V2: ObjectIdentifier =
    ObjectIdentifier::new_unwrap("1.2.840.113549.1.9.16.2.47");
const OID_TIME_STAMPING: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.3.6.1.5.5.7.3.8");
const OID_EXTENDED_KEY_USAGE: ObjectIdentifier = ObjectIdentifier::new_unwrap("2.5.29.37");
const OID_SHA256: ObjectIdentifier = ObjectIdentifier::new_unwrap("2.16.840.1.101.3.4.2.1");
const OID_RSA_SHA256: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.1.11");
const POLITIQUE: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.3.4");

/// `ESSCertIDv2` (RFC 5035) sans algorithme (SHA-256 par défaut) ni numéro de série.
#[derive(Clone, Debug, Eq, PartialEq, Sequence)]
struct EssCertIdV2 {
    cert_hash: OctetString,
}

/// `SigningCertificateV2` (RFC 5035) sans politiques.
#[derive(Clone, Debug, Eq, PartialEq, Sequence)]
struct SigningCertificateV2 {
    certs: Vec<EssCertIdV2>,
}

/// Profil d'un certificat de TSA auto-signé : usage étendu unique et critique.
struct ProfilTsa(Name);

impl BuilderProfile for ProfilTsa {
    fn get_issuer(&self, subject: &Name) -> Name {
        subject.clone()
    }

    fn get_subject(&self) -> Name {
        self.0.clone()
    }

    fn build_extensions(
        &self,
        _spk: spki::SubjectPublicKeyInfoRef<'_>,
        _issuer_spk: spki::SubjectPublicKeyInfoRef<'_>,
        _tbs: &x509_cert::certificate::TbsCertificate,
    ) -> x509_cert::builder::Result<Vec<Extension>> {
        let usage = ExtendedKeyUsage(vec![OID_TIME_STAMPING])
            .to_der()
            .expect("encodage de l'usage étendu");
        Ok(vec![Extension {
            extn_id: OID_EXTENDED_KEY_USAGE,
            critical: true,
            extn_value: OctetString::new(usage).expect("octets"),
        }])
    }
}

/// Entier DER positif minimal d'un `u64`.
fn entier(valeur: u64) -> Int {
    let octets = valeur.to_be_bytes();
    let debut = octets.iter().position(|&b| b != 0).unwrap_or(7);
    let mut minimal = Vec::with_capacity(9);
    if octets[debut] & 0x80 != 0 {
        minimal.push(0);
    }
    minimal.extend_from_slice(&octets[debut..]);
    Int::new(&minimal).expect("entier")
}

fn temps(unix: Duration) -> Time {
    Time::from(der::DateTime::from_unix_duration(unix).expect("date"))
}

/// Autorité d'horodatage locale : clé et certificat tirés à la création.
pub struct AutoriteLocale {
    cle: RS256SigningKey,
    certificat: Certificate,
    horloge: std::sync::Arc<dyn Horloge>,
    serie: AtomicU64,
    temoins: Mutex<Vec<Temoin>>,
}

impl std::fmt::Debug for AutoriteLocale {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AutoriteLocale").finish_non_exhaustive()
    }
}

impl AutoriteLocale {
    /// Tire la clé (RSA 2048) et le certificat auto-signé, valable autour de l'instant de
    /// l'horloge.
    pub fn nouvelle(horloge: std::sync::Arc<dyn Horloge>) -> Self {
        let cle = RS256SigningKey::new(new_key(MIN_BITS).expect("clé RSA"));
        let maintenant = u64::try_from(horloge.maintenant().unix_timestamp()).expect("après 1970");
        let validite = Validity::new(
            temps(Duration::from_secs(maintenant - 86_400)),
            temps(Duration::from_secs(maintenant + 10 * 86_400)),
        );
        let sujet =
            Name::from_str("CN=Autorite d'horodatage locale du test,O=Sigma Books test").unwrap();
        let spki = SubjectPublicKeyInfoOwned::from_key(&cle.verifying_key()).expect("clé publique");
        let serie = SerialNumber::new(&[0x01, 0x02, 0x03, 0x04]).expect("numéro de série");
        let constructeur =
            CertificateBuilder::new(ProfilTsa(sujet), serie, validite, spki).expect("certificat");
        let certificat = constructeur
            .build::<_, crypto_glue::rsa::pkcs1v15::Signature>(&cle)
            .expect("certificat signé");
        Self {
            cle,
            certificat,
            horloge,
            serie: AtomicU64::new(1),
            temoins: Mutex::new(Vec::new()),
        }
    }

    /// Certificat de l'autorité, DER : l'ancre de confiance de `sigma-verifier` (`--ancre`).
    pub fn certificat_der(&self) -> Vec<u8> {
        self.certificat.to_der().expect("DER")
    }

    /// Ancre de confiance de la bibliothèque.
    pub fn ancre(&self) -> Ancre {
        Ancre::depuis_der(&self.certificat_der()).expect("ancre")
    }

    /// Témoins que l'autorité a signés, dans l'ordre.
    pub fn temoins(&self) -> Vec<Temoin> {
        self.temoins.lock().unwrap().clone()
    }

    /// `TimeStampToken` (DER) pour le témoin et le nonce.
    fn jeton(&self, temoin: &Temoin, nonce: Nonce) -> Vec<u8> {
        let instant = u64::try_from(self.horloge.maintenant().unix_timestamp()).expect("date");
        let tst = TstInfo {
            version: TspVersion::V1,
            policy: POLITIQUE,
            message_imprint: MessageImprint {
                hash_algorithm: AlgorithmIdentifierOwned {
                    oid: OID_SHA256,
                    parameters: Some(Any::null()),
                },
                hashed_message: OctetString::new(temoin.empreinte()).expect("empreinte"),
            },
            serial_number: entier(self.serie.fetch_add(1, Ordering::SeqCst)),
            gen_time: GeneralizedTimeNanos::from_unix_duration(Duration::from_secs(instant))
                .expect("date"),
            accuracy: None,
            ordering: false,
            nonce: Some(entier(nonce.valeur())),
            tsa: None,
            extensions: None,
        };
        let tst_der = tst.to_der().expect("TSTInfo");
        let der_certificat = self.certificat.to_der().expect("DER");

        let attribut = |oid: ObjectIdentifier, valeur: Any| {
            let mut valeurs = SetOfVec::new();
            valeurs.insert(valeur).expect("valeur");
            x509_cert::attr::Attribute {
                oid,
                values: valeurs,
            }
        };
        let signe = SigningCertificateV2 {
            certs: vec![EssCertIdV2 {
                cert_hash: OctetString::new(Sha256::digest(&der_certificat).to_vec())
                    .expect("empreinte"),
            }],
        };
        let mut attributs: SignedAttributes = SetOfVec::new();
        for a in [
            attribut(
                OID_CONTENT_TYPE,
                Any::encode_from(&OID_TST_INFO).expect("type"),
            ),
            attribut(
                OID_MESSAGE_DIGEST,
                Any::encode_from(
                    &OctetString::new(Sha256::digest(&tst_der).to_vec()).expect("empreinte"),
                )
                .expect("empreinte"),
            ),
            attribut(
                OID_SIGNING_CERTIFICATE_V2,
                Any::encode_from(&signe).expect("certificat désigné"),
            ),
        ] {
            attributs.insert(a).expect("attribut");
        }
        let signature = self.cle.sign(&attributs.to_der().expect("attributs"));

        let algorithme = |oid, parametres| AlgorithmIdentifierOwned {
            oid,
            parameters: parametres,
        };
        let signataire = SignerInfo {
            version: CmsVersion::V1,
            sid: SignerIdentifier::IssuerAndSerialNumber(IssuerAndSerialNumber {
                issuer: self.certificat.tbs_certificate().issuer().clone(),
                serial_number: self.certificat.tbs_certificate().serial_number().clone(),
            }),
            digest_alg: algorithme(OID_SHA256, None),
            signed_attrs: Some(attributs),
            signature_algorithm: algorithme(OID_RSA_SHA256, Some(Any::null())),
            signature: OctetString::new(signature.to_bytes().to_vec()).expect("signature"),
            unsigned_attrs: None,
        };
        let mut algorithmes = SetOfVec::new();
        algorithmes
            .insert(algorithme(OID_SHA256, None))
            .expect("algorithme");
        let mut certificats = SetOfVec::new();
        certificats
            .insert(CertificateChoices::Certificate(self.certificat.clone()))
            .expect("certificat");
        let mut signataires = SetOfVec::new();
        signataires.insert(signataire).expect("signataire");
        let donnees = SignedData {
            version: CmsVersion::V3,
            digest_algorithms: algorithmes,
            encap_content_info: EncapsulatedContentInfo {
                econtent_type: OID_TST_INFO,
                econtent: Some(
                    Any::encode_from(&OctetString::new(tst_der).expect("TSTInfo")).expect("any"),
                ),
            },
            certificates: Some(CertificateSet(certificats)),
            crls: None,
            signer_infos: SignerInfos(signataires),
        };
        ContentInfo {
            content_type: OID_SIGNED_DATA,
            content: Any::encode_from(&donnees).expect("SignedData"),
        }
        .to_der()
        .expect("ContentInfo")
    }
}

impl Horodateur for AutoriteLocale {
    fn horodater<'a>(
        &'a self,
        temoin: &'a Temoin,
        nonce: Nonce,
    ) -> Pin<Box<dyn Future<Output = Result<JetonObtenu, ErreurHorodatage>> + Send + 'a>> {
        Box::pin(async move {
            self.temoins.lock().unwrap().push(temoin.clone());
            let jeton = self.jeton(temoin, nonce);
            // Le jeton est vérifié comme le fait le client réel : autorité de confiance = la
            // nôtre, mêmes contrôles (empreinte, signature, usage, chaîne, validité).
            let verifie = verifier_jeton(&jeton, temoin, &[self.ancre()])
                .expect("le jeton de l'autorité locale se vérifie");
            Ok(JetonObtenu {
                autorite: Autorite::parse("http://tsa.exemple.invalid").expect("autorité"),
                jeton,
                verifie,
            })
        })
    }
}
