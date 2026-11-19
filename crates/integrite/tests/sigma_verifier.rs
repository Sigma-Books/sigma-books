//! Le binaire `sigma-verifier` : rapport, codes de retour, arguments.

#![allow(clippy::unwrap_used)]

use std::path::PathBuf;
use std::process::{Command, Output};

const VERIFIER: &str = env!("CARGO_BIN_EXE_sigma-verifier");
const TETE: &str = "13b4cf3aa80611d5f46d9bdb6fdc110d1adcc51f9ce1be0a737146af7b6a9e62";

fn vecteur() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/vecteurs/export-v1.txt")
}

fn lancer(arguments: &[&str]) -> (i32, String, String) {
    let Output {
        status,
        stdout,
        stderr,
    } = Command::new(VERIFIER).args(arguments).output().unwrap();
    (
        status.code().unwrap(),
        String::from_utf8(stdout).unwrap(),
        String::from_utf8(stderr).unwrap(),
    )
}

#[test]
fn export_integre_code_0() {
    let chemin = vecteur();
    let (code, sortie, _) = lancer(&[
        "--dossier",
        "dossier-test",
        "--tete",
        TETE,
        chemin.to_str().unwrap(),
    ]);
    assert_eq!(code, 0, "{sortie}");
    assert!(sortie.contains("RÉSULTAT : INTÈGRE"));
}

#[test]
fn export_altere_code_1() {
    let altere =
        std::fs::read_to_string(vecteur())
            .unwrap()
            .replacen("montant=2500", "montant=25000", 2);
    let chemin = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("export-altere.txt");
    std::fs::write(&chemin, altere).unwrap();
    let (code, sortie, _) = lancer(&[chemin.to_str().unwrap()]);
    assert_eq!(code, 1, "{sortie}");
    assert!(sortie.contains("OD-2026-000001"), "{sortie}");
    assert!(sortie.contains("RÉSULTAT : ANOMALIES"), "{sortie}");
}

#[test]
fn usage_incorrect_code_2() {
    for arguments in [
        &[][..],
        &["--tete", "zz", "x"],
        &["--inconnue", "x"],
        &["a", "b"],
        &["--dossier"],
    ] {
        let (code, _, erreur) = lancer(arguments);
        assert_eq!(code, 2, "{arguments:?}");
        assert!(erreur.contains("Usage"), "{erreur}");
    }
    let (code, _, erreur) = lancer(&["/chemin/inexistant"]);
    assert_eq!(code, 2);
    assert!(erreur.contains("impossible"), "{erreur}");
}

#[test]
fn aide_et_version() {
    let (code, sortie, _) = lancer(&["--aide"]);
    assert_eq!(code, 0);
    assert!(sortie.contains("Codes de retour"));
    let (code, sortie, _) = lancer(&["--version"]);
    assert_eq!(code, 0);
    assert!(sortie.starts_with("sigma-verifier "));
}

fn fichier(nom: &str) -> String {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join(nom)
        .to_str()
        .unwrap()
        .to_owned()
}

#[test]
fn export_v2_horodate_code_0() {
    let (code, sortie, _) = lancer(&[
        "--dossier",
        "dossier-test",
        &fichier("tests/vecteurs/export-v2.txt"),
    ]);
    assert_eq!(code, 0, "{sortie}");
    assert!(sortie.contains("(export v2)"), "{sortie}");
    assert!(sortie.contains("2 jeton(s) vérifié(s)"), "{sortie}");
    assert!(!sortie.contains("Attention"), "{sortie}");
}

#[test]
fn ancre_supplementaire() {
    // Une racine déjà intégrée, fournie en DER : acceptée, sans effet sur le verdict.
    let (code, sortie, _) = lancer(&[
        "--ancre",
        &fichier("src/horodatage/ancres/digicert-trusted-root-g4.der"),
        &fichier("tests/vecteurs/export-v2.txt"),
    ]);
    assert_eq!(code, 0, "{sortie}");
    for arguments in [
        &["--ancre", "/chemin/inexistant", "x"][..],
        &[
            "--ancre",
            fichier("tests/vecteurs/export-v1.txt").as_str(),
            "x",
        ],
        &["--ancre"],
    ] {
        let (code, _, erreur) = lancer(arguments);
        assert_eq!(code, 2, "{arguments:?}");
        assert!(erreur.contains("--ancre"), "{erreur}");
    }
}

#[test]
fn export_d_audit_reconnu_et_verifie() {
    let vecteur = fichier("tests/vecteurs/audit-v1/export-dossier.txt");
    let tete = "383591bb3c62f7f173bb7d0240c89aa9e336de54ae972721263740f3a1434509";
    let (code, sortie, _) = lancer(&["--tete", tete, &vecteur]);
    assert_eq!(code, 0, "{sortie}");
    assert!(sortie.contains("chaîne d'audit intègre"), "{sortie}");
    // Ancrage différent, entrée altérée, export tronqué : code 1.
    let (code, sortie, _) = lancer(&["--tete", TETE, &vecteur]);
    assert_eq!(code, 1, "{sortie}");
    let original = std::fs::read_to_string(&vecteur).unwrap();
    for (nom, contenu) in [
        ("audit-altere.txt", original.replacen("rang=3", "rang=4", 1)),
        (
            "audit-tronque.txt",
            original.replacen("fin-export\n", "", 1),
        ),
    ] {
        let chemin = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(nom);
        std::fs::write(&chemin, contenu).unwrap();
        let (code, sortie, _) = lancer(&[chemin.to_str().unwrap()]);
        assert_eq!(code, 1, "{nom} : {sortie}");
    }
    // Options sans objet pour un export d'audit : code 2.
    let (code, _, _) = lancer(&["--dossier", "dossier-test", &vecteur]);
    assert_eq!(code, 2);
}
