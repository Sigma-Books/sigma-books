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
