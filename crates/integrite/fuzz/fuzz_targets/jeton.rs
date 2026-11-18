//! Analyse d'un jeton et d'une réponse RFC 3161 sur des octets quelconques (jetons de l'export,
//! réponses d'une TSA) : aucune panique, et aucun jeton accepté pour un témoin qui n'est pas le
//! sien.

#![no_main]
#![forbid(unsafe_code)]

use std::sync::LazyLock;

use integrite::horodatage::{Ancre, Temoin, verifier_jeton, verifier_reponse};
use integrite::{Empreinte, IdDossier, ancres_par_defaut};
use libfuzzer_sys::fuzz_target;

static ANCRES: LazyLock<Vec<Ancre>> =
    LazyLock::new(|| ancres_par_defaut().unwrap_or_default());

fuzz_target!(|octets: &[u8]| {
    let (Ok(dossier), Ok(tete)) = (IdDossier::parse("fuzz"), Empreinte::parse(&"0".repeat(64)))
    else {
        return;
    };
    let temoin = Temoin::nouveau(dossier, tete, 1);
    // Un jeton forgé ne peut pas être accepté : l'empreinte de ce témoin n'a jamais été signée.
    assert!(verifier_jeton(octets, &temoin, &ANCRES).is_err());
    let nonce = octets.iter().take(8).fold(0u64, |n, &b| (n << 8) | u64::from(b));
    assert!(verifier_reponse(octets, &temoin, nonce, &ANCRES).is_err());
});
