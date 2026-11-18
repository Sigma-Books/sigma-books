//! Vecteurs figés des formats d'export (`tests/vecteurs/`). Un échec ici signale un changement
//! de format : il faut créer une nouvelle version, jamais modifier ces vecteurs
//! (`crates/integrite/CLAUDE.md`).
//!
//! - `export-v1.txt` : format v1, figé ; il n'est plus produit mais reste lu et vérifié. Ses
//!   empreintes ont été recalculées indépendamment (Python, `hashlib`) à partir de
//!   `docs/integrite/format-v1.md`.
//! - `export-v2.txt` : format v2, produit indépendamment (Python) à partir de la v1 et de deux
//!   jetons réels (`Sectigo` sur les 3 premières écritures, `DigiCert` sur les 5), selon
//!   `docs/integrite/format-v2.md`. Comme la v1, il contient une écriture « sans pièce », que le
//!   moteur ne produit plus (AUDCIF art. 17 et 22) : il reste lu et vérifié.
//! - `export-v2-reference.txt` : export v2 du dossier de référence actuel, où cette écriture
//!   s'appuie sur une pièce interne. Produit indépendamment (Python : écriture remplacée, chaîne
//!   recalculée), avec deux jetons réels obtenus pour cette chaîne.

#![allow(clippy::unwrap_used)]

mod commun;

use integrite::{Empreinte, Options, Version, genese, verifier};

const EXPORT_V1: &[u8] = include_bytes!("vecteurs/export-v1.txt");
const EXPORT_V2: &[u8] = include_bytes!("vecteurs/export-v2.txt");
const EXPORT_V2_REFERENCE: &[u8] = include_bytes!("vecteurs/export-v2-reference.txt");
const GENESE: &str = "ea5595d5652ca52169b5d87e9bfbcae14eafc1031c7e2f7985af5252dff7229c";
const TETE: &str = "13b4cf3aa80611d5f46d9bdb6fdc110d1adcc51f9ce1be0a737146af7b6a9e62";

#[test]
fn empreinte_de_depart_figee() {
    assert_eq!(genese(&commun::dossier_id()).to_string(), GENESE);
}

#[test]
fn export_regenere_identique_au_vecteur_de_reference() {
    let regenere = commun::export_avec_jetons(&commun::jetons_de_reference());
    assert_eq!(
        String::from_utf8(regenere).unwrap(),
        String::from_utf8(EXPORT_V2_REFERENCE.to_vec()).unwrap()
    );
    let rapport = verifier(EXPORT_V2_REFERENCE, &Options::default()).unwrap();
    assert!(rapport.est_integre(), "{rapport}");
    assert_eq!(rapport.horodatages.len(), 2);
}

#[test]
fn vecteur_v1_integre_et_ancre() {
    let options = Options {
        dossier: Some(commun::dossier_id()),
        tete: Some(Empreinte::parse(TETE).unwrap()),
        ..Options::default()
    };
    let rapport = verifier(EXPORT_V1, &options).unwrap();
    assert!(rapport.est_integre(), "{rapport}");
    assert_eq!(rapport.version, Some(Version::V1));
    assert_eq!(rapport.tete.unwrap().to_string(), TETE);
}

#[test]
fn vecteur_v2_integre_et_horodate() {
    let options = Options {
        dossier: Some(commun::dossier_id()),
        ..Options::default()
    };
    let rapport = verifier(EXPORT_V2, &options).unwrap();
    assert!(rapport.est_integre(), "{rapport}");
    assert_eq!(rapport.version, Some(Version::V2));
    let couverts: Vec<u64> = rapport.horodatages.iter().map(|h| h.nombre).collect();
    assert_eq!(couverts, [3, 5]);
    assert!(rapport.horodatages[0].jeton.autorite.contains("Sectigo"));
    assert!(rapport.horodatages[1].jeton.autorite.contains("DigiCert"));
    let texte = rapport.to_string();
    assert!(texte.contains("2 jeton(s) vérifié(s)"), "{texte}");
    assert!(
        texte.contains("5 premières écritures au plus tard le 2026-11-18 11:17:48 UTC"),
        "{texte}"
    );
    assert!(!texte.contains("Attention"), "{texte}");
}
