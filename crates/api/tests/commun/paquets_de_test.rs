//! Paquet fiscal **factice** pour les tests (pays CM, valeurs inventées) : voir
//! `crates/fiscal/tests/fixtures/zz`.
#![allow(dead_code, clippy::unwrap_used)]

use ::fiscal::{ParametresFiscaux, Portee, Registre, Socle};
use time::Date;

/// Texte d'une version du paquet factice (`tests/fixtures/zz` de la crate `fiscal`), pour le
/// pays CM, avec les dates données. Valeurs inventées, sans rapport avec le droit d'un pays.
pub fn factice(version: &str, effet: Date, taux_normal: &str) -> String {
    let lire = |f: &str| {
        std::fs::read_to_string(format!(
            "{}/../fiscal/tests/fixtures/zz/{version}/{f}",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap()
    };
    let ancien = if version == "2024.1" {
        "2024-01-01"
    } else {
        "2026-07-01"
    };
    let ancien_taux = if version == "2024.1" { "15" } else { "16" };
    [
        lire("version.toml"),
        lire("regimes-tva.toml"),
        lire("motifs-exoneration.toml"),
    ]
    .join("\n")
    .replace("pays = \"ZZ\"", "pays = \"CM\"")
    .replace(
        &format!("date_effet = \"{ancien}\""),
        &format!("date_effet = \"{effet}\""),
    )
    .replace(
        &format!("valeur = \"{ancien_taux}\""),
        &format!("valeur = \"{taux_normal}\""),
    )
}

pub fn registre_cm(versions: &[(&str, Date, &str)]) -> Registre {
    let socle = Socle::officiel().unwrap();
    let mut toutes: Vec<ParametresFiscaux> = Registre::officiel()
        .unwrap()
        .versions_du_pays("SN")
        .to_vec();
    for (v, effet, taux) in versions {
        toutes.push(
            ParametresFiscaux::depuis_texte(&socle, Portee::Pack, &factice(v, *effet, taux))
                .unwrap(),
        );
    }
    Registre::depuis_versions(socle, toutes).unwrap()
}
