//! Analyseur d'export d'audit et d'entrée d'audit sur des octets quelconques (menaces F7.3,
//! F7.5) : aucune panique, et une entrée acceptée est exactement sa forme canonique.

#![no_main]
#![forbid(unsafe_code)]

use integrite::audit::{AttentesAudit, EntreeAudit, verifier_export_audit};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|octets: &[u8]| {
    let _ = verifier_export_audit(octets, &AttentesAudit::default());
    if let Ok(texte) = std::str::from_utf8(octets)
        && let Ok(entree) = EntreeAudit::lire(texte)
    {
        assert_eq!(entree.serialiser(), texte);
    }
});
