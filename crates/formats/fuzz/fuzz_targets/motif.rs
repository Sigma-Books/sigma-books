//! Motifs des règles de suggestion (F11.4) : compilation d'une source quelconque dans des
//! bornes fixes, puis recherche dans un texte quelconque. Aucune panique ; une source au-delà
//! de la borne est toujours refusée.
//!
//! Entrée : le premier octet donne la longueur de la source ; la suite est la source puis le
//! texte.

#![no_main]
#![forbid(unsafe_code)]

use formats::motif::{BornesMotif, ErreurMotif, Motif};
use libfuzzer_sys::fuzz_target;

const BORNES: BornesMotif = BornesMotif {
    longueur_source: 128,
    taille_programme: 1 << 16,
};

fuzz_target!(|octets: &[u8]| {
    let [n, reste @ ..] = octets else {
        return;
    };
    let (source, texte) = reste.split_at(usize::from(*n).min(reste.len()));
    let Ok(source) = std::str::from_utf8(source) else {
        return;
    };
    let texte = String::from_utf8_lossy(texte);
    match Motif::compiler(source, &BORNES) {
        Ok(motif) => {
            assert!(source.len() <= BORNES.longueur_source);
            let _ = motif.correspond(&texte);
        }
        Err(ErreurMotif::SourceTropLongue { .. }) => {
            assert!(source.len() > BORNES.longueur_source);
        }
        Err(_) => {}
    }
});
