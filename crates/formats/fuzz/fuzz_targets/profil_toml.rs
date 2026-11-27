//! Chargeur de profil sur un TOML hostile (F11.4) : aucune panique, et un profil accepté se
//! réécrit puis se relit à l'identique.

#![no_main]
#![forbid(unsafe_code)]

use formats::releve::ProfilReleve;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|octets: &[u8]| {
    let Ok(texte) = std::str::from_utf8(octets) else {
        return;
    };
    let Ok(profil) = ProfilReleve::depuis_toml(texte) else {
        return;
    };
    let ecrit = toml::to_string(&profil).expect("profil sérialisable");
    assert_eq!(ProfilReleve::depuis_toml(&ecrit), Ok(profil));
});
