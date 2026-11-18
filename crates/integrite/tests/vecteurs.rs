//! Vecteurs figés du format v1 (`tests/vecteurs/`). Un échec ici signale un changement du
//! format canonique : il faut créer une version 2, jamais modifier ces vecteurs
//! (`crates/integrite/CLAUDE.md`). Les empreintes ont été recalculées indépendamment
//! (Python, `hashlib`) à partir de la définition de `docs/integrite/format-v1.md`.

#![allow(clippy::unwrap_used)]

mod commun;

use integrite::{Empreinte, Options, genese, verifier};

const EXPORT_V1: &[u8] = include_bytes!("vecteurs/export-v1.txt");
const GENESE: &str = "ea5595d5652ca52169b5d87e9bfbcae14eafc1031c7e2f7985af5252dff7229c";
const TETE: &str = "13b4cf3aa80611d5f46d9bdb6fdc110d1adcc51f9ce1be0a737146af7b6a9e62";

#[test]
fn empreinte_de_depart_figee() {
    assert_eq!(genese(&commun::dossier_id()).to_string(), GENESE);
}

#[test]
fn export_regenere_identique_au_vecteur() {
    let regenere = commun::export_de_reference();
    assert_eq!(
        String::from_utf8(regenere).unwrap(),
        String::from_utf8(EXPORT_V1.to_vec()).unwrap()
    );
}

#[test]
fn vecteur_integre_et_ancre() {
    let options = Options {
        dossier: Some(commun::dossier_id()),
        tete: Some(Empreinte::parse(TETE).unwrap()),
    };
    let rapport = verifier(EXPORT_V1, &options).unwrap();
    assert!(rapport.est_integre(), "{rapport}");
    assert_eq!(rapport.tete.unwrap().to_string(), TETE);
}
