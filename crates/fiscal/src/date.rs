//! Dates ISO `AAAA-MM-JJ` des fichiers de données.

use time::{Date, Month};

/// Date stricte `AAAA-MM-JJ` (quatre chiffres, deux chiffres, deux chiffres).
pub(crate) fn lire(texte: &str) -> Option<Date> {
    let octets = texte.as_bytes();
    if octets.len() != 10 || octets[4] != b'-' || octets[7] != b'-' {
        return None;
    }
    let nombre = |a: usize, b: usize| -> Option<u32> {
        let tranche = &texte[a..b];
        if tranche.bytes().all(|o| o.is_ascii_digit()) {
            tranche.parse().ok()
        } else {
            None
        }
    };
    let annee = i32::try_from(nombre(0, 4)?).ok()?;
    let mois = Month::try_from(u8::try_from(nombre(5, 7)?).ok()?).ok()?;
    let jour = u8::try_from(nombre(8, 10)?).ok()?;
    Date::from_calendar_date(annee, mois, jour).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_strictes() {
        assert!(lire("2026-12-31").is_some());
        for refus in [
            "2026-1-01",
            "2026-13-01",
            "2026-02-30",
            "26-01-01",
            "2026/01/01",
            "+026-01-01",
            "",
        ] {
            assert!(lire(refus).is_none(), "{refus}");
        }
    }
}
