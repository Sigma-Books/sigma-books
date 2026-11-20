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

use integrite::{Anomalie, ErreurCanonique, ErreurExport, Options, Version, verifier};

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

/// Cas de rejet : (fichier, ligne du fichier, cause précise attendue).
fn rejets() -> Vec<(&'static str, u64, ErreurExport)> {
    use ErreurCanonique::{Attendu, EtiquetteIncoherente, NonCanonique, ValeurInvalide};
    let ecriture = ErreurExport::Ecriture;
    vec![
        ("v3-sans-etiquette", 114, ecriture(NonCanonique)),
        (
            "taxe-avec-motif",
            138,
            ecriture(EtiquetteIncoherente { ligne: 25 }),
        ),
        (
            "taux-non-canonique",
            132,
            ecriture(ValeurInvalide {
                ligne: 19,
                cle: "fiscal-taux",
            }),
        ),
        (
            "taux-nul",
            132,
            ecriture(ValeurInvalide {
                ligne: 19,
                cle: "fiscal-taux",
            }),
        ),
        (
            "cles-en-desordre",
            130,
            ecriture(Attendu {
                ligne: 17,
                attendu: "fiscal-sens",
            }),
        ),
        (
            "nature-inconnue",
            129,
            ecriture(ValeurInvalide {
                ligne: 16,
                cle: "fiscal-nature",
            }),
        ),
        (
            "etiquette-incomplete",
            131,
            ecriture(Attendu {
                ligne: 18,
                attendu: "fiscal-regime",
            }),
        ),
        (
            "motif-hors-regime",
            129,
            ecriture(EtiquetteIncoherente { ligne: 16 }),
        ),
        (
            "exigibilite-inconnue",
            133,
            ecriture(ValeurInvalide {
                ligne: 20,
                cle: "fiscal-exigibilite",
            }),
        ),
        (
            "regime-mal-forme",
            131,
            ecriture(ValeurInvalide {
                ligne: 18,
                cle: "fiscal-regime",
            }),
        ),
        (
            "v3-dans-export-v3",
            114,
            ErreurExport::Attendu("ecriture v1 », « ecriture v2 » ou « tete="),
        ),
        (
            "cles-fiscales-en-v1",
            129,
            ecriture(Attendu {
                ligne: 16,
                attendu: "ligne",
            }),
        ),
    ]
}

#[test]
fn les_exports_defectueux_sont_refuses_pour_la_cause_attendue() {
    let cas = rejets();
    assert_eq!(cas.len(), 12);
    for (nom, ligne, erreur) in cas {
        let chemin = format!(
            "{}/tests/vecteurs/v4-rejets/{nom}.txt",
            env!("CARGO_MANIFEST_DIR")
        );
        let octets = std::fs::read(&chemin).unwrap();
        let trouvees = anomalies(&octets);
        assert_eq!(trouvees, [Anomalie::Illisible { ligne, erreur }], "{nom}");
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
