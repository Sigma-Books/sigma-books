//! Analyseur d'export de `sigma-verifier` sur des octets quelconques (menace F7.5) : aucune
//! panique, et une écriture acceptée est exactement sa forme canonique.

#![no_main]
#![forbid(unsafe_code)]

use integrite::{EcritureCanonique, Options, verifier};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|octets: &[u8]| {
    let rapport = verifier(octets, &Options::default());
    assert!(
        rapport.is_ok(),
        "aucune erreur d'entrée-sortie sur une tranche en mémoire"
    );
    if let Ok(texte) = std::str::from_utf8(octets)
        && let Ok(ecriture) = EcritureCanonique::lire(texte)
    {
        assert_eq!(ecriture.serialiser(), texte);
    }
});
