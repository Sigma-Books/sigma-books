//! Vecteurs de la pièce interne, forme canonique v1 (`docs/integrite/piece-interne-v1.md`).
//!
//! Les fichiers de `tests/vecteurs/piece-interne-v1/` et leurs SHA-256 ont été produits
//! indépendamment du code (Python, `hashlib`). Chaque note admise est relue, resérialisée à
//! l'identique et hachée ; chaque `rejet-*` est refusé.
// Fichier de test : les `expect`/`unwrap` y sont voulus.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::path::{Path, PathBuf};

use api::pieces::interne::NoteOd;
use integrite::Empreinte;

fn repertoire() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/vecteurs/piece-interne-v1")
}

fn lire(nom: &str) -> String {
    std::fs::read_to_string(repertoire().join(nom)).unwrap()
}

#[test]
fn les_notes_admises_se_relisent_et_se_hachent_a_l_identique() {
    for nom in ["note-od", "note-od-minimale"] {
        let texte = lire(&format!("{nom}.txt"));
        let note = NoteOd::lire(&texte).unwrap_or_else(|e| panic!("{nom} : {e}"));
        assert_eq!(note.serialiser(), texte, "{nom}");
        let attendue = lire(&format!("{nom}.sha256"));
        assert_eq!(
            Empreinte::calculer(texte.as_bytes()).to_string(),
            attendue.trim_end(),
            "{nom}"
        );
    }
}

#[test]
fn les_rejets_sont_refuses() {
    let mut rejets: Vec<(String, String)> = std::fs::read_dir(repertoire())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with("rejet-"))
        .map(|n| (n.clone(), lire(&n)))
        .collect();
    assert!(rejets.len() >= 8, "vecteurs de rejet introuvables");
    // Fins de ligne CRLF : construites ici (le dépôt normalise les fins de ligne des fichiers).
    rejets.push((
        "crlf".to_owned(),
        lire("note-od-minimale.txt").replace('\n', "\r\n"),
    ));
    for (nom, texte) in rejets {
        assert!(NoteOd::lire(&texte).is_err(), "{nom} admis à tort");
    }
}
