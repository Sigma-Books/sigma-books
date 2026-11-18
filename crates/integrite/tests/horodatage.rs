//! Horodatage RFC 3161 : requête et vérification sur des jetons réels de `DigiCert` et de `Sectigo`
//! (obtenus le 2026-11-18 pour le témoin `vecteurs/horodatage/temoin.txt`, dossier fictif).

// Fichier de test : les `unwrap` y sont voulus (une donnée de test invalide fait échouer le test).
#![allow(clippy::unwrap_used)]

use integrite::horodatage::{Ancre, ErreurJeton, Temoin, verifier_jeton, verifier_reponse};
use integrite::{Empreinte, IdDossier, ancres_par_defaut, requete};

const TEMOIN: &str = include_str!("vecteurs/horodatage/temoin.txt");
const REQUETE_OPENSSL: &[u8] = include_bytes!("vecteurs/horodatage/requete-openssl.tsq");
const DIGICERT: &[u8] = include_bytes!("vecteurs/horodatage/digicert.tsr");
const SECTIGO: &[u8] = include_bytes!("vecteurs/horodatage/sectigo.tsr");
/// Nonce de la requête envoyée aux deux autorités.
const NONCE: u64 = 0xB17F_41C7_83A6_60D7;

fn temoin(nombre: u64) -> Temoin {
    Temoin::nouveau(
        IdDossier::parse("demo").unwrap(),
        Empreinte::parse("ea5595d5652ca52169b5d87e9bfbcae14eafc1031c7e2f7985af5252dff7229c")
            .unwrap(),
        nombre,
    )
}

fn jeton(reponse: &[u8]) -> Vec<u8> {
    verifier_reponse(reponse, &temoin(0), NONCE, &ancres_par_defaut().unwrap())
        .unwrap()
        .0
}

#[test]
fn temoin_et_requete_conformes_aux_vecteurs() {
    assert_eq!(temoin(0).contenu(), TEMOIN);
    // Octet pour octet, la requête produite par `openssl ts -query -sha256 -cert`.
    assert_eq!(requete(&temoin(0), NONCE).unwrap(), REQUETE_OPENSSL);
}

#[test]
fn reponses_reelles_verifiees() {
    let ancres = ancres_par_defaut().unwrap();
    for (reponse, autorite, racine) in [
        (DIGICERT, "DigiCert", "DigiCert Trusted Root G4"),
        (SECTIGO, "Sectigo", "Sectigo Public Time Stamping Root R46"),
    ] {
        let (jeton, verifie) = verifier_reponse(reponse, &temoin(0), NONCE, &ancres).unwrap();
        assert_eq!(
            verifie.date.unix_timestamp(),
            1_795_000_288,
            "2026-11-18T11:11:28Z"
        );
        assert!(verifie.autorite.contains(autorite), "{}", verifie.autorite);
        assert!(verifie.ancre.contains(racine), "{}", verifie.ancre);
        // Le jeton conservé se vérifie seul, sans nonce.
        assert_eq!(
            verifier_jeton(&jeton, &temoin(0), &ancres).unwrap(),
            verifie
        );
    }
}

#[test]
fn autre_temoin_ou_autre_nonce_refuses() {
    let ancres = ancres_par_defaut().unwrap();
    assert_eq!(
        verifier_reponse(DIGICERT, &temoin(1), NONCE, &ancres),
        Err(ErreurJeton::EmpreinteDifferente)
    );
    assert_eq!(
        verifier_reponse(SECTIGO, &temoin(0), NONCE + 1, &ancres).map(|_| ()),
        Err(ErreurJeton::NonceDifferent)
    );
}

#[test]
fn chaine_sans_ancre_refusee() {
    let ancres = ancres_par_defaut().unwrap();
    assert_eq!(
        verifier_jeton(&jeton(SECTIGO), &temoin(0), &ancres[..1]),
        Err(ErreurJeton::ChaineNonFiable)
    );
    assert_eq!(
        verifier_jeton(&jeton(DIGICERT), &temoin(0), &[]),
        Err(ErreurJeton::ChaineNonFiable)
    );
}

/// Toute altération de l'empreinte horodatée ou de la date est détectée.
#[test]
fn alteration_du_contenu_signe_detectee() {
    let ancres = ancres_par_defaut().unwrap();
    let original = jeton(DIGICERT);
    let empreinte = original
        .windows(8)
        .position(|w| w == [0x8b, 0x66, 0xb8, 0x6d, 0xba, 0x1b, 0xa3, 0x21])
        .unwrap();
    let date = original
        .windows(14)
        .position(|w| w == b"20261118111128")
        .unwrap();
    for position in (empreinte..empreinte + 32).chain(date..date + 14) {
        let mut altere = original.clone();
        altere[position] ^= 0x01;
        assert!(
            verifier_jeton(&altere, &temoin(0), &ancres).is_err(),
            "octet {position}"
        );
    }
    // Signature : dernier octet du jeton DigiCert (signature RSA en fin de SignerInfo).
    let mut altere = original.clone();
    let dernier = altere.len() - 1;
    altere[dernier] ^= 0x01;
    assert!(matches!(
        verifier_jeton(&altere, &temoin(0), &ancres),
        Err(ErreurJeton::SignatureInvalide(_))
    ));
}

#[test]
fn ancres_integrees_et_pem() {
    let ancres = ancres_par_defaut().unwrap();
    assert_eq!(ancres.len(), 2);
    assert!(Ancre::depuis_pem(b"pas un certificat").is_err());
    assert!(Ancre::depuis_der(b"\x30\x00").is_err());
}

#[test]
fn reponse_trop_volumineuse_ou_illisible() {
    let ancres = ancres_par_defaut().unwrap();
    assert!(matches!(
        verifier_reponse(&vec![0u8; 70_000], &temoin(0), NONCE, &ancres),
        Err(ErreurJeton::TropVolumineux(_))
    ));
    assert!(matches!(
        verifier_jeton(b"\x30\x03\x02\x01\x00", &temoin(0), &ancres),
        Err(ErreurJeton::Illisible(_))
    ));
}
