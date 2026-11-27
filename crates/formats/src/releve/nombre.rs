//! Lecture d'un nombre avec les séparateurs du profil, en décimal exact (jamais de `f32` ni de
//! `f64` : OCA convertit en `float`, `…sheet_parser.py:427`, non repris).
//!
//! Comme Tryton (`statement.py:1571-1576`) : retrait du séparateur de milliers, séparateur
//! décimal remplacé par le point, puis lecture d'un `Decimal`. Les espaces de bord (espace
//! insécable comprise) sont ignorées. Forme admise ensuite : un signe facultatif en tête, des
//! chiffres ASCII, au plus un séparateur décimal, au moins un chiffre. Exposant, `NaN` et
//! infinis (admis par `Decimal` de Python) sont refusés ; au-delà de la précision de
//! `rust_decimal` (28 chiffres), le nombre est refusé plutôt qu'arrondi.
//!
//! Le séparateur de milliers est retiré **où qu'il soit** (comme Tryton, sans contrôle des
//! groupes de trois chiffres) ; il n'est ni un chiffre ni un signe (contrôlé à la validation du
//! profil). Conséquence : un profil mal réglé (milliers `,` et décimale `.` pour un fichier
//! écrit `12,50`) lit `1250` sans erreur ; seul le contrôle des soldes du relevé (ADR 0034
//! §7, A17), quand ils sont connus, ou l'aperçu le révèle.

use rust_decimal::Decimal;

/// Lit un nombre ; `None` s'il n'a pas la forme admise.
pub(crate) fn lire(texte: &str, decimal: char, milliers: Option<char>) -> Option<Decimal> {
    let mut negatif = false;
    let mut signe_possible = true;
    let mut entiere = String::new();
    let mut fraction: Option<String> = None;
    for c in texte.trim().chars() {
        if Some(c) == milliers {
            continue;
        }
        if signe_possible && (c == '-' || c == '+') {
            negatif = c == '-';
            signe_possible = false;
            continue;
        }
        signe_possible = false;
        if c == decimal {
            if fraction.is_some() {
                return None;
            }
            fraction = Some(String::new());
        } else if c.is_ascii_digit() {
            fraction.as_mut().unwrap_or(&mut entiere).push(c);
        } else {
            return None;
        }
    }
    let fraction = fraction.unwrap_or_default();
    if entiere.is_empty() && fraction.is_empty() {
        return None;
    }
    let mut ascii = String::with_capacity(entiere.len() + fraction.len() + 3);
    if negatif {
        ascii.push('-');
    }
    ascii.push_str(if entiere.is_empty() { "0" } else { &entiere });
    if !fraction.is_empty() {
        ascii.push('.');
        ascii.push_str(&fraction);
    }
    Decimal::from_str_exact(&ascii).ok()
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use super::*;

    fn d(s: &str) -> Decimal {
        Decimal::from_str(s).unwrap()
    }

    #[test]
    fn formes_admises() {
        assert_eq!(lire("1234.50", '.', None), Some(d("1234.50")));
        assert_eq!(lire(" -1 234,5 ", ',', Some(' ')), Some(d("-1234.5")));
        assert_eq!(
            lire("1\u{a0}500\u{a0}000", ',', Some('\u{a0}')),
            Some(d("1500000"))
        );
        assert_eq!(lire("1.500,00", ',', Some('.')), Some(d("1500.00")));
        assert_eq!(lire("+12", ',', None), Some(d("12")));
        assert_eq!(lire(",5", ',', None), Some(d("0.5")));
        assert_eq!(lire("5,", ',', None), Some(d("5")));
        assert_eq!(lire("-0", '.', None), Some(Decimal::ZERO));
    }

    #[test]
    fn formes_refusees() {
        for texte in [
            "", " ", "-", ",", "1,2,3", "1e3", "NaN", "inf", "12-", "1 2", "--1", "€12", "١٢",
        ] {
            assert_eq!(lire(texte, ',', None), None, "{texte}");
        }
        assert_eq!(lire(&"9".repeat(40), ',', None), None);
        assert_eq!(lire(&format!("0,{}", "1".repeat(40)), ',', None), None);
    }
}
