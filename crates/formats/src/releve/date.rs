//! Format de date d'un profil, sous-ensemble de `strptime` (Tryton
//! `statement.py:1568-1569`, OCA `…sheet_parser.py:252`) **indépendant de la locale** (ADR
//! 0034 §6 : jamais la locale du serveur ; Tryton prend ses défauts dans la langue de
//! l'utilisateur, `:1513-1532`, non repris).
//!
//! Directives : `%d` (jour), `%m` (mois), `%Y` (année sur 4 chiffres), `%y` (année sur 2
//! chiffres, pivot POSIX de Python : 69 à 99 → 1969 à 1999, 00 à 68 → 2000 à 2068), `%H`,
//! `%M`, `%S` (heure lue et contrôlée, puis ignorée : le relevé porte une date), `%%`. Comme
//! Python : une suite d'espaces du format accepte une ou plusieurs espaces, les autres
//! caractères se comparent sans tenir compte de la casse, un jour ou un mois absent vaut 1, une
//! directive répétée est refusée. Un format **sans année** est refusé à la création du profil
//! (ADR 0034 §6 : OCA complète l'année par l'horloge, `:251-260`, non repris).

use std::fmt::Write as _;

use thiserror::Error;
use time::{Date, Month};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Element {
    Jour,
    Mois,
    Annee,
    AnneeCourte,
    Heure,
    Minute,
    Seconde,
    Espace,
    Litteral(char),
}

/// Refus d'un format de date.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum ErreurFormatDate {
    /// Format vide.
    #[error("format de date vide")]
    Vide,
    /// Directive `%` inconnue ou incomplète (les noms de mois, liés à une locale, sont exclus).
    #[error("directive de date inconnue")]
    DirectiveInconnue,
    /// Directive répétée.
    #[error("directive de date répétée")]
    DirectiveRepetee,
    /// Aucune année (`%Y` ou `%y`) : jamais complétée par l'horloge.
    #[error("format de date sans année")]
    SansAnnee,
}

/// Format de date validé.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormatDate {
    source: String,
    elements: Vec<Element>,
}

#[derive(Debug, Clone, Copy, Default)]
struct Composantes {
    annee: Option<i32>,
    annee_courte: Option<i32>,
    mois: Option<u8>,
    jour: Option<u8>,
}

impl FormatDate {
    /// Valide un format.
    ///
    /// # Errors
    /// Voir [`ErreurFormatDate`].
    pub fn analyser(source: &str) -> Result<Self, ErreurFormatDate> {
        if source.is_empty() {
            return Err(ErreurFormatDate::Vide);
        }
        let mut elements = Vec::new();
        let mut caracteres = source.chars().peekable();
        while let Some(c) = caracteres.next() {
            let element = if c == '%' {
                match caracteres.next() {
                    Some('d') => Element::Jour,
                    Some('m') => Element::Mois,
                    Some('Y') => Element::Annee,
                    Some('y') => Element::AnneeCourte,
                    Some('H') => Element::Heure,
                    Some('M') => Element::Minute,
                    Some('S') => Element::Seconde,
                    Some('%') => Element::Litteral('%'),
                    _ => return Err(ErreurFormatDate::DirectiveInconnue),
                }
            } else if c.is_whitespace() {
                while caracteres.next_if(|s| s.is_whitespace()).is_some() {}
                Element::Espace
            } else {
                Element::Litteral(c)
            };
            let directive = !matches!(element, Element::Espace | Element::Litteral(_));
            if directive && elements.contains(&element) {
                return Err(ErreurFormatDate::DirectiveRepetee);
            }
            elements.push(element);
        }
        if !elements
            .iter()
            .any(|e| matches!(e, Element::Annee | Element::AnneeCourte))
        {
            return Err(ErreurFormatDate::SansAnnee);
        }
        Ok(Self {
            source: source.to_owned(),
            elements,
        })
    }

    /// Source du format.
    #[must_use]
    pub fn source(&self) -> &str {
        &self.source
    }

    /// Lit une date ; `None` si le texte ne suit pas le format ou si la date n'existe pas.
    #[must_use]
    pub fn lire(&self, texte: &str) -> Option<Date> {
        let c = apparier(&self.elements, texte, Composantes::default())?;
        // %Y prime sur %y, comme Python.
        let annee = match (c.annee, c.annee_courte) {
            (Some(a), _) => a,
            (None, Some(a)) if a >= 69 => 1900 + a,
            (None, Some(a)) => 2000 + a,
            (None, None) => return None,
        };
        if annee < 1 {
            return None;
        }
        let mois = Month::try_from(c.mois.unwrap_or(1)).ok()?;
        Date::from_calendar_date(annee, mois, c.jour.unwrap_or(1)).ok()
    }

    /// Écrit une date selon le format (heures à zéro). Réciproque de [`Self::lire`] pour les
    /// années de 1 à 9999 (de 1969 à 2068 avec `%y` seul).
    #[must_use]
    pub fn ecrire(&self, date: Date) -> String {
        let mut s = String::new();
        for e in &self.elements {
            // L'écriture dans une `String` n'échoue pas.
            let _ = match e {
                Element::Jour => write!(s, "{:02}", date.day()),
                Element::Mois => write!(s, "{:02}", u8::from(date.month())),
                Element::Annee => write!(s, "{:04}", date.year()),
                Element::AnneeCourte => write!(s, "{:02}", date.year().rem_euclid(100)),
                Element::Heure | Element::Minute | Element::Seconde => write!(s, "00"),
                Element::Espace => write!(s, " "),
                Element::Litteral(c) => write!(s, "{c}"),
            };
        }
        s
    }
}

/// Appariement avec retour arrière sur la largeur des nombres (1 ou 2 chiffres), comme les
/// alternatives de l'expression de Python. La récursion n'a lieu qu'aux directives
/// numériques : profondeur au plus 7, quelle que soit la longueur du format.
fn apparier(elements: &[Element], mut texte: &str, c: Composantes) -> Option<Composantes> {
    for (i, e) in elements.iter().enumerate() {
        let (largeurs, min, max): (&[usize], u16, u16) = match e {
            Element::Litteral(l) => {
                let mut reste = texte.chars();
                let x = reste.next()?;
                if !x.to_lowercase().eq(l.to_lowercase()) {
                    return None;
                }
                texte = reste.as_str();
                continue;
            }
            Element::Espace => {
                let reste = texte.trim_start();
                if reste.len() == texte.len() {
                    return None;
                }
                texte = reste;
                continue;
            }
            Element::Jour => (&[2, 1], 1, 31),
            Element::Mois => (&[2, 1], 1, 12),
            Element::Annee => (&[4], 1, 9999),
            Element::AnneeCourte => (&[2], 0, 99),
            Element::Heure => (&[2, 1], 0, 23),
            Element::Minute => (&[2, 1], 0, 59),
            Element::Seconde => (&[2, 1], 0, 61),
        };
        for &largeur in largeurs {
            let Some(chiffres) = texte.get(..largeur) else {
                continue;
            };
            if !chiffres.bytes().all(|b| b.is_ascii_digit()) {
                continue;
            }
            let Ok(valeur) = chiffres.parse::<u16>() else {
                continue;
            };
            if valeur < min || valeur > max {
                continue;
            }
            let mut suite = c;
            match e {
                Element::Jour => suite.jour = u8::try_from(valeur).ok(),
                Element::Mois => suite.mois = u8::try_from(valeur).ok(),
                Element::Annee => suite.annee = Some(i32::from(valeur)),
                Element::AnneeCourte => suite.annee_courte = Some(i32::from(valeur)),
                _ => {}
            }
            let reste = texte.get(largeur..).unwrap_or_default();
            if let Some(r) = apparier(elements.get(i + 1..).unwrap_or_default(), reste, suite) {
                return Some(r);
            }
        }
        return None;
    }
    texte.is_empty().then_some(c)
}

#[cfg(test)]
mod tests {
    use time::macros::date;

    use super::*;

    fn f(s: &str) -> FormatDate {
        FormatDate::analyser(s).unwrap()
    }

    #[test]
    fn formats_courants() {
        assert_eq!(
            f("%d/%m/%Y").lire("05/01/2026"),
            Some(date!(2026 - 01 - 05))
        );
        assert_eq!(f("%d/%m/%Y").lire("5/1/2026"), Some(date!(2026 - 01 - 05)));
        assert_eq!(
            f("%Y-%m-%d").lire("2026-02-28"),
            Some(date!(2026 - 02 - 28))
        );
        assert_eq!(f("%m/%d/%y").lire("12/15/18"), Some(date!(2018 - 12 - 15)));
        assert_eq!(f("%d.%m.%y").lire("01.01.69"), Some(date!(1969 - 01 - 01)));
        assert_eq!(f("%d.%m.%y").lire("01.01.68"), Some(date!(2068 - 01 - 01)));
        assert_eq!(
            f("%Y-%m-%d %H:%M:%S").lire("2026-03-01   14:05:09"),
            Some(date!(2026 - 03 - 01))
        );
        assert_eq!(f("%d%m%Y").lire("01122026"), Some(date!(2026 - 12 - 01)));
        // Retour arrière : 11, puis 2 (22 n'est pas un mois).
        assert_eq!(f("%d%m%Y").lire("1122026"), Some(date!(2026 - 02 - 11)));
        assert_eq!(f("%Y-%m").lire("2026-07"), Some(date!(2026 - 07 - 01)));
        assert_eq!(
            f("Le %d/%m/%Y").lire("le 02/03/2026"),
            Some(date!(2026 - 03 - 02))
        );
    }

    #[test]
    fn dates_refusees() {
        let jma = f("%d/%m/%Y");
        for texte in [
            "31/02/2026",
            "00/01/2026",
            "01/13/2026",
            "01/01/0000",
            "01/01/26",
            "01/01/2026 ",
            "01-01-2026",
            "",
            "١/٠١/٢٠٢٦",
        ] {
            assert_eq!(jma.lire(texte), None, "{texte}");
        }
        assert_eq!(f("%Y-%m-%d %H:%M").lire("2026-01-01 24:00"), None);
    }

    #[test]
    fn formats_refuses() {
        assert_eq!(FormatDate::analyser(""), Err(ErreurFormatDate::Vide));
        assert_eq!(
            FormatDate::analyser("%d/%m"),
            Err(ErreurFormatDate::SansAnnee)
        );
        assert_eq!(
            FormatDate::analyser("%d %b %Y"),
            Err(ErreurFormatDate::DirectiveInconnue)
        );
        assert_eq!(
            FormatDate::analyser("%Y%"),
            Err(ErreurFormatDate::DirectiveInconnue)
        );
        assert_eq!(
            FormatDate::analyser("%d/%d/%Y"),
            Err(ErreurFormatDate::DirectiveRepetee)
        );
    }

    #[test]
    fn ecriture() {
        assert_eq!(f("%d/%m/%Y").ecrire(date!(2026 - 01 - 05)), "05/01/2026");
        assert_eq!(f("%y%m%d %H%%").ecrire(date!(2006 - 11 - 30)), "061130 00%");
    }
}
