//! Vérification d'un jeton d'horodatage RFC 3161 (`TimeStampToken`, CMS `SignedData`).
//!
//! Contrôles, dans l'ordre :
//! 1. structure : `ContentInfo` → `SignedData` → `TSTInfo` (RFC 3161 §2.4.2) ;
//! 2. empreinte horodatée = SHA-256 du témoin attendu ; nonce si l'appelant l'impose ;
//! 3. un seul signataire ; attributs signés `contentType` = `id-ct-TSTInfo` et
//!    `messageDigest` = empreinte du `TSTInfo` (RFC 5652 §5.4) ;
//! 4. certificat du signataire désigné par `signingCertificate` ou `signingCertificateV2`
//!    (RFC 2634, RFC 5035), usage étendu **unique et critique** `id-kp-timeStamping`
//!    (RFC 3161 §2.3) ;
//! 5. signature RSA PKCS#1 v1.5 des attributs signés ;
//! 6. chaîne de certificats jusqu'à une ancre de confiance, chaque certificat étant valide à la
//!    date du jeton (et non à la date de vérification : un jeton reste vérifiable après
//!    l'expiration du certificat de la TSA).
//!
//! Limite connue : la révocation des certificats n'est pas contrôlée (vérification hors ligne).

use der::asn1::{ObjectIdentifier, OctetString};
use der::{Any, Decode, Encode, Sequence};
use rsa::PublicKey;
use rsa::pkcs1v15::{Signature, VerifyingKey};
use rsa::pkcs8::DecodePublicKey;
use rsa::signature::Verifier;
use sha1::Sha1;
use sha2::{Digest, Sha256, Sha384, Sha512};
use time::OffsetDateTime;
use x509_cert::Certificate;
use x509_cert::ext::pkix::{BasicConstraints, ExtendedKeyUsage, SubjectKeyIdentifier};
use x509_cert::spki::{AlgorithmIdentifierOwned, SubjectPublicKeyInfoOwned};
use x509_tsp::cmpv2::status::PkiStatus;
use x509_tsp::{TimeStampResp, TstInfo};

use cms::cert::CertificateChoices;
use cms::content_info::ContentInfo;
use cms::signed_data::{SignedData, SignerIdentifier, SignerInfo};

use super::ancres::Ancre;
use super::{JETON_MAX, OID_SHA256, Temoin, entier};

const OID_SIGNED_DATA: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.7.2");
const OID_TST_INFO: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.9.16.1.4");
const OID_CONTENT_TYPE: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.9.3");
const OID_MESSAGE_DIGEST: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.9.4");
const OID_SIGNING_CERTIFICATE: ObjectIdentifier =
    ObjectIdentifier::new_unwrap("1.2.840.113549.1.9.16.2.12");
const OID_SIGNING_CERTIFICATE_V2: ObjectIdentifier =
    ObjectIdentifier::new_unwrap("1.2.840.113549.1.9.16.2.47");
const OID_TIME_STAMPING: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.3.6.1.5.5.7.3.8");
const OID_SHA384: ObjectIdentifier = ObjectIdentifier::new_unwrap("2.16.840.1.101.3.4.2.2");
const OID_SHA512: ObjectIdentifier = ObjectIdentifier::new_unwrap("2.16.840.1.101.3.4.2.3");
const OID_RSA: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.1.1");
const OID_RSA_SHA256: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.1.11");
const OID_RSA_SHA384: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.1.12");
const OID_RSA_SHA512: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.1.13");

/// Longueur maximale d'une chaîne de certificats (signataire, intermédiaires).
const PROFONDEUR_MAX: usize = 5;

/// Motif de refus d'un jeton ou d'une réponse d'horodatage.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ErreurJeton {
    /// Taille au-delà de [`JETON_MAX`].
    #[error("jeton trop volumineux : {0} octets")]
    TropVolumineux(usize),
    /// Structure ASN.1 illisible ou inattendue.
    #[error("jeton illisible : {0}")]
    Illisible(String),
    /// La TSA a refusé la requête.
    #[error("requête refusée par l'autorité d'horodatage (statut {0})")]
    Refusee(String),
    /// L'empreinte horodatée n'est pas celle du témoin.
    #[error("le jeton porte sur une autre empreinte que celle du témoin")]
    EmpreinteDifferente,
    /// Nonce absent ou différent de celui de la requête.
    #[error("nonce absent ou différent de celui de la requête")]
    NonceDifferent,
    /// Algorithme non pris en charge.
    #[error("algorithme non pris en charge : {0}")]
    AlgorithmeNonPrisEnCharge(String),
    /// Attribut signé absent ou incohérent.
    #[error("attribut signé invalide : {0}")]
    AttributInvalide(&'static str),
    /// Certificat du signataire absent du jeton.
    #[error("certificat de l'autorité d'horodatage absent du jeton")]
    CertificatAbsent,
    /// Certificat du signataire impropre à l'horodatage.
    #[error("certificat impropre à l'horodatage : {0}")]
    UsageInvalide(&'static str),
    /// Signature invalide.
    #[error("signature invalide : {0}")]
    SignatureInvalide(&'static str),
    /// Certificat hors de sa période de validité à la date du jeton.
    #[error("certificat « {0} » non valide à la date du jeton")]
    CertificatHorsValidite(String),
    /// Aucune chaîne vers une ancre de confiance.
    #[error("aucune chaîne de certificats vers une ancre de confiance")]
    ChaineNonFiable,
}

fn illisible(e: impl std::fmt::Display) -> ErreurJeton {
    ErreurJeton::Illisible(e.to_string())
}

/// Jeton vérifié : ce qu'il atteste.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JetonVerifie {
    /// Date de l'horodatage (`genTime`, UTC).
    pub date: OffsetDateTime,
    /// Autorité d'horodatage (sujet du certificat signataire).
    pub autorite: String,
    /// Politique d'horodatage de la TSA (OID).
    pub politique: String,
    /// Racine de confiance qui valide la chaîne.
    pub ancre: String,
}

/// `ESSCertIDv2` (RFC 5035) : seuls l'algorithme et l'empreinte servent.
#[derive(Clone, Debug, Eq, PartialEq, Sequence)]
struct EssCertIdV2 {
    /// `DEFAULT sha256` : absent quand c'est SHA-256.
    #[asn1(optional = "true")]
    hash_algorithm: Option<AlgorithmIdentifierOwned>,
    cert_hash: OctetString,
    #[asn1(optional = "true")]
    issuer_serial: Option<Any>,
}

/// `SigningCertificateV2` (RFC 5035).
#[derive(Clone, Debug, Eq, PartialEq, Sequence)]
struct SigningCertificateV2 {
    certs: Vec<EssCertIdV2>,
    #[asn1(optional = "true")]
    policies: Option<Any>,
}

/// `ESSCertID` (RFC 2634) : empreinte SHA-1 du certificat.
#[derive(Clone, Debug, Eq, PartialEq, Sequence)]
struct EssCertId {
    cert_hash: OctetString,
    #[asn1(optional = "true")]
    issuer_serial: Option<Any>,
}

/// `SigningCertificate` (RFC 2634).
#[derive(Clone, Debug, Eq, PartialEq, Sequence)]
struct SigningCertificate {
    certs: Vec<EssCertId>,
    #[asn1(optional = "true")]
    policies: Option<Any>,
}

/// Fonction de hachage admise pour les empreintes et les signatures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Hachage {
    Sha256,
    Sha384,
    Sha512,
}

impl Hachage {
    fn depuis_oid(oid: &ObjectIdentifier) -> Result<Self, ErreurJeton> {
        match *oid {
            OID_SHA256 => Ok(Self::Sha256),
            OID_SHA384 => Ok(Self::Sha384),
            OID_SHA512 => Ok(Self::Sha512),
            _ => Err(ErreurJeton::AlgorithmeNonPrisEnCharge(oid.to_string())),
        }
    }

    fn hacher(self, donnees: &[u8]) -> Vec<u8> {
        match self {
            Self::Sha256 => Sha256::digest(donnees).to_vec(),
            Self::Sha384 => Sha384::digest(donnees).to_vec(),
            Self::Sha512 => Sha512::digest(donnees).to_vec(),
        }
    }
}

/// Vérifie une signature RSA PKCS#1 v1.5.
fn verifier_rsa(
    cle: &SubjectPublicKeyInfoOwned,
    hachage: Hachage,
    message: &[u8],
    signature: &[u8],
) -> Result<(), ErreurJeton> {
    if cle.algorithm.oid != OID_RSA {
        return Err(ErreurJeton::AlgorithmeNonPrisEnCharge(
            cle.algorithm.oid.to_string(),
        ));
    }
    let der = cle.to_der().map_err(illisible)?;
    let cle = PublicKey::from_public_key_der(&der).map_err(illisible)?;
    let signature =
        Signature::try_from(signature).map_err(|_| ErreurJeton::SignatureInvalide("format"))?;
    let resultat = match hachage {
        Hachage::Sha256 => VerifyingKey::<Sha256>::new(cle).verify(message, &signature),
        Hachage::Sha384 => VerifyingKey::<Sha384>::new(cle).verify(message, &signature),
        Hachage::Sha512 => VerifyingKey::<Sha512>::new(cle).verify(message, &signature),
    };
    resultat.map_err(|_| ErreurJeton::SignatureInvalide("clé ou contenu"))
}

/// Algorithme de signature d'un certificat ou d'un signataire CMS.
fn hachage_de_signature(
    algorithme: &ObjectIdentifier,
    digest_cms: Option<Hachage>,
) -> Result<Hachage, ErreurJeton> {
    match *algorithme {
        OID_RSA_SHA256 => Ok(Hachage::Sha256),
        OID_RSA_SHA384 => Ok(Hachage::Sha384),
        OID_RSA_SHA512 => Ok(Hachage::Sha512),
        OID_RSA => digest_cms.ok_or(ErreurJeton::AlgorithmeNonPrisEnCharge(
            "rsaEncryption sans fonction de hachage".to_owned(),
        )),
        _ => Err(ErreurJeton::AlgorithmeNonPrisEnCharge(
            algorithme.to_string(),
        )),
    }
}

/// Vérifie une **réponse** de TSA (`TimeStampResp`) à une requête portant ce témoin et ce
/// nonce. Renvoie le jeton (DER, à conserver et à exporter) et ce qu'il atteste.
///
/// # Errors
/// [`ErreurJeton`] : réponse illisible ou refusée, ou jeton invalide.
pub fn verifier_reponse(
    reponse: &[u8],
    temoin: &Temoin,
    nonce: u64,
    ancres: &[Ancre],
) -> Result<(Vec<u8>, JetonVerifie), ErreurJeton> {
    if reponse.len() > JETON_MAX {
        return Err(ErreurJeton::TropVolumineux(reponse.len()));
    }
    let reponse = TimeStampResp::from_der(reponse).map_err(illisible)?;
    match reponse.status.status {
        PkiStatus::Accepted | PkiStatus::GrantedWithMods => {}
        autre => return Err(ErreurJeton::Refusee(format!("{autre:?}"))),
    }
    let jeton = reponse
        .time_stamp_token
        .ok_or_else(|| illisible("réponse sans jeton"))?
        .to_der()
        .map_err(illisible)?;
    let verifie = verifier(&jeton, temoin, Some(nonce), ancres)?;
    Ok((jeton, verifie))
}

/// Vérifie un **jeton** conservé (`TimeStampToken`, DER) pour ce témoin.
///
/// # Errors
/// [`ErreurJeton`] : jeton illisible, sur une autre empreinte, mal signé ou non fiable.
pub fn verifier_jeton(
    jeton: &[u8],
    temoin: &Temoin,
    ancres: &[Ancre],
) -> Result<JetonVerifie, ErreurJeton> {
    verifier(jeton, temoin, None, ancres)
}

fn verifier(
    jeton: &[u8],
    temoin: &Temoin,
    nonce: Option<u64>,
    ancres: &[Ancre],
) -> Result<JetonVerifie, ErreurJeton> {
    // 1. Structure.
    let (signe, tst_der, tst) = lire(jeton)?;

    // 2. Empreinte et nonce.
    if tst.message_imprint.hash_algorithm.oid != OID_SHA256
        || tst.message_imprint.hashed_message.as_bytes() != temoin.empreinte()
    {
        return Err(ErreurJeton::EmpreinteDifferente);
    }
    if let Some(attendu) = nonce {
        let attendu = entier(attendu).map_err(illisible)?;
        if tst.nonce.as_ref() != Some(&attendu) {
            return Err(ErreurJeton::NonceDifferent);
        }
    }
    let date_unix = tst.gen_time.to_unix_duration();

    // 3. Signataire et attributs signés.
    let mut signataires = signe.signer_infos.0.iter();
    let (Some(signataire), None) = (signataires.next(), signataires.next()) else {
        return Err(illisible("un seul signataire attendu"));
    };
    let digest = Hachage::depuis_oid(&signataire.digest_alg.oid)?;
    let attributs = signataire
        .signed_attrs
        .as_ref()
        .ok_or(ErreurJeton::AttributInvalide("attributs signés absents"))?;
    let valeur = |oid: ObjectIdentifier| -> Result<&Any, ErreurJeton> {
        let mut trouves = attributs.iter().filter(|a| a.oid == oid);
        match (trouves.next(), trouves.next()) {
            (Some(a), None) if a.values.len() == 1 => a
                .values
                .iter()
                .next()
                .ok_or(ErreurJeton::AttributInvalide("valeur absente")),
            _ => Err(ErreurJeton::AttributInvalide("attribut absent ou répété")),
        }
    };
    let type_contenu: ObjectIdentifier =
        valeur(OID_CONTENT_TYPE)?.decode_as().map_err(illisible)?;
    if type_contenu != OID_TST_INFO {
        return Err(ErreurJeton::AttributInvalide("contentType"));
    }
    let empreinte_signee: OctetString =
        valeur(OID_MESSAGE_DIGEST)?.decode_as().map_err(illisible)?;
    if empreinte_signee.as_bytes() != digest.hacher(&tst_der) {
        return Err(ErreurJeton::AttributInvalide("messageDigest"));
    }

    // 4. Certificat du signataire.
    let certificats: Vec<&Certificate> = signe
        .certificates
        .iter()
        .flat_map(|ensemble| ensemble.0.iter())
        .filter_map(|c| match c {
            CertificateChoices::Certificate(c) => Some(c),
            CertificateChoices::Other(_) => None,
        })
        .collect();
    let certificat = certificat_du_signataire(signataire, &certificats)?;
    controler_certificat_designe(&valeur, certificat)?;
    controler_usage(certificat)?;

    // 5. Signature des attributs signés (encodés en SET OF, RFC 5652 §5.4).
    let message = attributs.to_der().map_err(illisible)?;
    let hachage = hachage_de_signature(&signataire.signature_algorithm.oid, Some(digest))?;
    verifier_rsa(
        certificat.tbs_certificate().subject_public_key_info(),
        hachage,
        &message,
        signataire.signature.as_bytes(),
    )?;

    // 6. Chaîne de confiance, à la date du jeton.
    let ancre = remonter_la_chaine(certificat, &certificats, ancres, date_unix)?;

    let date =
        OffsetDateTime::from_unix_timestamp(i64::try_from(date_unix.as_secs()).map_err(illisible)?)
            .map_err(illisible)?;
    Ok(JetonVerifie {
        date,
        autorite: certificat.tbs_certificate().subject().to_string(),
        politique: tst.policy.to_string(),
        ancre: ancre.nom().to_string(),
    })
}

/// `ContentInfo` → `SignedData` → `TSTInfo` (et son encodage DER, qui est l'objet signé).
fn lire(jeton: &[u8]) -> Result<(SignedData, Box<[u8]>, TstInfo), ErreurJeton> {
    if jeton.len() > JETON_MAX {
        return Err(ErreurJeton::TropVolumineux(jeton.len()));
    }
    let info = ContentInfo::from_der(jeton).map_err(illisible)?;
    if info.content_type != OID_SIGNED_DATA {
        return Err(illisible("ContentInfo autre que SignedData"));
    }
    let signe: SignedData = info.content.decode_as().map_err(illisible)?;
    if signe.encap_content_info.econtent_type != OID_TST_INFO {
        return Err(illisible("contenu autre que TSTInfo"));
    }
    let contenu = signe
        .encap_content_info
        .econtent
        .as_ref()
        .ok_or_else(|| illisible("TSTInfo absent"))?;
    let tst_der = OctetString::from_der(&contenu.to_der().map_err(illisible)?)
        .map_err(illisible)?
        .into_bytes();
    let tst = TstInfo::from_der(&tst_der).map_err(illisible)?;
    Ok((signe, tst_der, tst))
}

/// Usage étendu unique et critique `id-kp-timeStamping` (RFC 3161 §2.3).
fn controler_usage(certificat: &Certificate) -> Result<(), ErreurJeton> {
    match certificat
        .tbs_certificate()
        .get_extension::<ExtendedKeyUsage>()
        .map_err(illisible)?
    {
        Some((true, usage)) if usage.0 == [OID_TIME_STAMPING] => Ok(()),
        Some((false, _)) => Err(ErreurJeton::UsageInvalide("usage étendu non critique")),
        _ => Err(ErreurJeton::UsageInvalide(
            "usage étendu autre que l'horodatage",
        )),
    }
}

fn certificat_du_signataire<'a>(
    signataire: &SignerInfo,
    certificats: &[&'a Certificate],
) -> Result<&'a Certificate, ErreurJeton> {
    certificats
        .iter()
        .copied()
        .find(|c| {
            let tbs = c.tbs_certificate();
            match &signataire.sid {
                SignerIdentifier::IssuerAndSerialNumber(id) => {
                    tbs.issuer() == &id.issuer && tbs.serial_number() == &id.serial_number
                }
                SignerIdentifier::SubjectKeyIdentifier(id) => matches!(
                    tbs.get_extension::<SubjectKeyIdentifier>(),
                    Ok(Some((_, ski))) if &ski == id
                ),
            }
        })
        .ok_or(ErreurJeton::CertificatAbsent)
}

/// `signingCertificateV2` (de préférence) ou `signingCertificate` : le premier identifiant
/// désigne le certificat du signataire par son empreinte (RFC 3161 §2.4.1, RFC 5816).
fn controler_certificat_designe<'a>(
    valeur: &impl Fn(ObjectIdentifier) -> Result<&'a Any, ErreurJeton>,
    certificat: &Certificate,
) -> Result<(), ErreurJeton> {
    let der = certificat.to_der().map_err(illisible)?;
    if let Ok(v2) = valeur(OID_SIGNING_CERTIFICATE_V2) {
        let v2: SigningCertificateV2 = v2.decode_as().map_err(illisible)?;
        let premier = v2
            .certs
            .first()
            .ok_or(ErreurJeton::AttributInvalide("signingCertificateV2 vide"))?;
        let hachage = match &premier.hash_algorithm {
            Some(algorithme) => Hachage::depuis_oid(&algorithme.oid)?,
            None => Hachage::Sha256,
        };
        if premier.cert_hash.as_bytes() != hachage.hacher(&der) {
            return Err(ErreurJeton::AttributInvalide("signingCertificateV2"));
        }
        return Ok(());
    }
    let v1: SigningCertificate = valeur(OID_SIGNING_CERTIFICATE)?
        .decode_as()
        .map_err(illisible)?;
    let premier = v1
        .certs
        .first()
        .ok_or(ErreurJeton::AttributInvalide("signingCertificate vide"))?;
    if premier.cert_hash.as_bytes() != Sha1::digest(&der).as_slice() {
        return Err(ErreurJeton::AttributInvalide("signingCertificate"));
    }
    Ok(())
}

fn valide_a(certificat: &Certificate, date: std::time::Duration) -> Result<(), ErreurJeton> {
    let validite = certificat.tbs_certificate().validity();
    if validite.not_before.to_unix_duration() <= date
        && date <= validite.not_after.to_unix_duration()
    {
        Ok(())
    } else {
        Err(ErreurJeton::CertificatHorsValidite(
            certificat.tbs_certificate().subject().to_string(),
        ))
    }
}

fn verifier_signature_certificat(
    certificat: &Certificate,
    cle: &SubjectPublicKeyInfoOwned,
) -> Result<(), ErreurJeton> {
    let tbs = certificat.tbs_certificate().to_der().map_err(illisible)?;
    let hachage = hachage_de_signature(&certificat.signature_algorithm().oid, None)?;
    let signature = certificat
        .signature()
        .as_bytes()
        .ok_or(ErreurJeton::SignatureInvalide("bits de bourrage"))?;
    verifier_rsa(cle, hachage, &tbs, signature)
}

/// Remonte du certificat du signataire jusqu'à une ancre : chaque certificat est signé par son
/// émetteur, valide à la date du jeton ; les émetteurs intermédiaires sont des autorités.
fn remonter_la_chaine<'a>(
    signataire: &Certificate,
    certificats: &[&Certificate],
    ancres: &'a [Ancre],
    date: std::time::Duration,
) -> Result<&'a Ancre, ErreurJeton> {
    let mut courant = signataire;
    for _ in 0..PROFONDEUR_MAX {
        valide_a(courant, date)?;
        let emetteur = courant.tbs_certificate().issuer();
        if let Some(ancre) = ancres.iter().find(|a| {
            a.nom() == emetteur && verifier_signature_certificat(courant, a.cle()).is_ok()
        }) {
            return Ok(ancre);
        }
        let parent = certificats
            .iter()
            .copied()
            .find(|c| {
                !std::ptr::eq(*c, courant)
                    && c.tbs_certificate().subject() == emetteur
                    && verifier_signature_certificat(
                        courant,
                        c.tbs_certificate().subject_public_key_info(),
                    )
                    .is_ok()
            })
            .ok_or(ErreurJeton::ChaineNonFiable)?;
        match parent
            .tbs_certificate()
            .get_extension::<BasicConstraints>()
            .map_err(illisible)?
        {
            Some((_, contraintes)) if contraintes.ca => {}
            _ => return Err(ErreurJeton::ChaineNonFiable),
        }
        // Une racine contresignée portant la clé d'une ancre suffit.
        if let Some(ancre) = ancres.iter().find(|a| {
            a.nom() == parent.tbs_certificate().subject()
                && a.a_la_cle(parent.tbs_certificate().subject_public_key_info())
        }) {
            return Ok(ancre);
        }
        courant = parent;
    }
    Err(ErreurJeton::ChaineNonFiable)
}
