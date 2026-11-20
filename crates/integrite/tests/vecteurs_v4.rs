//! Vecteurs figés de l'export v4 (étiquette fiscale, ADR 0019 ; `docs/integrite/format-v4.md`).
//! Un échec ici signale un changement de format : il faut créer une nouvelle version, jamais
//! modifier ces vecteurs (`crates/integrite/CLAUDE.md`).
//!
//! - `export-v4-reference.txt` : les 6 écritures du vecteur v3 (inchangées, mêmes empreintes),
//!   puis 4 écritures au format canonique v3 : vente taxée à 18 %, vente exonérée (art. 361-4),
//!   contre-passation de la vente (étiquettes copiées), achat à l'encaissement avec date de
//!   valeur comptable. Produit **indépendamment** du moteur (Python, `hashlib`) à partir de la
//!   documentation du format, puis comparé à ce que produit le moteur.
//! - `v4-rejets/*.txt` : exports re-chaînés dont la 7e écriture (ligne 114) est défectueuse.
//!   Chacun doit être refusé, à la ligne attendue.

#![allow(clippy::unwrap_used)]

mod commun;

use integrite::{Anomalie, Options, Version, verifier};

const REFERENCE: &[u8] = include_bytes!("vecteurs/export-v4-reference.txt");
const TETE: &str = "b1b215cdafa1c86839a791244b2f9ff702abb9154ae62e33027190aafe70cf6b";

#[test]
fn le_moteur_regenere_le_vecteur() {
    let regenere = commun::export_de_reference_v4();
    assert_eq!(
        String::from_utf8(regenere).unwrap(),
        String::from_utf8(REFERENCE.to_vec()).unwrap()
    );
}

#[test]
fn vecteur_v4_integre() {
    let rapport = verifier(REFERENCE, &Options::default()).unwrap();
    assert!(rapport.est_integre(), "{rapport}");
    assert_eq!(rapport.version, Some(Version::V4));
    assert_eq!(rapport.nombre, 10);
    assert_eq!(rapport.tete.unwrap().to_string(), TETE);
}

/// Les 6 premières écritures sont celles du vecteur v3 : les écritures sans étiquette gardent
/// leur sérialisation et leur empreinte.
#[test]
fn les_six_premieres_ecritures_sont_celles_du_vecteur_v3() {
    let v3 =
        String::from_utf8(include_bytes!("vecteurs/export-v3-reference.txt").to_vec()).unwrap();
    let v4 = String::from_utf8(REFERENCE.to_vec()).unwrap();
    let corps = |t: &str| t[t.find("ecriture v1").unwrap()..t.find("tete=").unwrap()].to_owned();
    let (corps_v3, corps_v4) = (corps(&v3), corps(&v4));
    assert!(corps_v4.starts_with(&corps_v3));
    assert!(v4.starts_with("sigma-books export v4\n"));
}

#[test]
fn la_contre_passation_garde_les_etiquettes() {
    let v4 = String::from_utf8(REFERENCE.to_vec()).unwrap();
    let ecritures: Vec<&str> = v4.split("empreinte=").collect();
    // Écritures 7 (vente) et 9 (contre-passation) : mêmes lignes d'étiquette.
    let etiquettes = |e: &str| {
        e.lines()
            .filter(|l| l.starts_with("fiscal-"))
            .collect::<Vec<_>>()
            .join("|")
    };
    assert_eq!(etiquettes(ecritures[6]), etiquettes(ecritures[8]));
    assert_ne!(etiquettes(ecritures[6]), "");
}

fn anomalies(octets: &[u8]) -> Vec<Anomalie> {
    let rapport = verifier(octets, &Options::default()).unwrap();
    assert!(!rapport.est_integre());
    rapport.anomalies
}

/// Cas de rejet : (fichier, ligne du premier défaut).
const REJETS: [(&str, u64); 12] = [
    ("v3-sans-etiquette", 114),
    ("taxe-avec-motif", 138),
    ("taux-non-canonique", 132),
    ("taux-nul", 132),
    ("cles-en-desordre", 130),
    ("nature-inconnue", 129),
    ("etiquette-incomplete", 131),
    ("motif-hors-regime", 129),
    ("exigibilite-inconnue", 133),
    ("regime-mal-forme", 131),
    ("v3-dans-export-v3", 114),
    ("cles-fiscales-en-v1", 129),
];

#[test]
fn les_exports_defectueux_sont_refuses() {
    for (nom, ligne) in REJETS {
        let chemin = format!(
            "{}/tests/vecteurs/v4-rejets/{nom}.txt",
            env!("CARGO_MANIFEST_DIR")
        );
        let octets = std::fs::read(&chemin).unwrap();
        let trouvees = anomalies(&octets);
        assert!(
            matches!(trouvees.as_slice(), [Anomalie::Illisible { ligne: l, .. }] if *l == ligne),
            "{nom} : {trouvees:?}"
        );
    }
}

#[test]
fn une_etiquette_alteree_change_l_empreinte() {
    let octets = include_bytes!("vecteurs/v4-rejets/etiquette-alteree.txt");
    let trouvees = anomalies(octets);
    assert!(
        trouvees
            .iter()
            .any(|a| matches!(a, Anomalie::EmpreinteDifferente { numero, .. } if numero.to_string() == "VT-2026-000003")),
        "{trouvees:?}"
    );
}
