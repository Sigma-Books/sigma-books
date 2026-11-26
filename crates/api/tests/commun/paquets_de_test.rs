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

/// Paquet évolutif du pays CM autour de l'année `annee` : version 2024.1 (taux 15) jusqu'au 30 juin
/// où le motif x1 s'éteint, version 2026.1 (taux 16) à partir du 1er juillet, **sans le régime
/// `TAXABLE_NORMAL`** (régime disparu). Les dates suivent l'année injectée, jamais l'horloge.
pub fn registre_cm_evolutif(annee: i32) -> Registre {
    let socle = Socle::officiel().unwrap();
    let jour = |m: u8, j: u8| {
        Date::from_calendar_date(annee, time::Month::try_from(m).unwrap(), j).unwrap()
    };
    let echeance = format!("fin = \"{annee}-06-30\"");
    let v1 = factice(
        "2024.1",
        Date::from_calendar_date(annee - 1, time::Month::January, 1).unwrap(),
        "15",
    )
    .replace("fin = \"2025-12-31\"", &echeance);
    let v2 = factice("2026.1", jour(7, 1), "16").replace("fin = \"2025-12-31\"", &echeance);
    let debut = v2.find("[[regime]]\ncode = \"TAXABLE_NORMAL\"").unwrap();
    let suite = v2[debut + 1..].find("[[regime]]").unwrap() + debut + 1;
    let v2 = format!("{}{}", &v2[..debut], &v2[suite..]);
    let mut toutes: Vec<ParametresFiscaux> = Registre::officiel()
        .unwrap()
        .versions_du_pays("SN")
        .to_vec();
    for t in [v1, v2] {
        toutes.push(ParametresFiscaux::depuis_texte(&socle, Portee::Pack, &t).unwrap());
    }
    Registre::depuis_versions(socle, toutes).unwrap()
}
