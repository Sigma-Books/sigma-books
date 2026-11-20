//! Pièce interne : note d'opérations diverses, datée et motivée (spec §2 principe 2 ; ADR 0015),
//! sous forme canonique **figée**, version 1 : `docs/integrite/piece-interne-v1.md`.
//!
//! Le serveur sérialise la note ([`NoteOd::serialiser`]), en calcule la SHA-256 et la stocke
//! comme une pièce externe. La lecture ([`NoteOd::lire`]) est stricte : un texte n'est admis que
//! s'il est **exactement** la sérialisation de la note qu'il décrit. Une correction crée une
//! nouvelle pièce. Fonctions pures : ni horloge, ni aléa (instant et identifiants en paramètre).

use std::fmt::Write as _;

use time::{Date, Month, OffsetDateTime, UtcOffset};
use uuid::Uuid;

/// Marqueur de version, première ligne du texte (il entre dans le contenu haché).
pub const MARQUEUR: &str = "sigma-books piece-interne v1";
/// Taille maximale d'un texte (motif, ligne), en octets UTF-8.
pub const TEXTE_MAX: usize = 1000;
/// Nombre maximal de lignes d'explication.
pub const LIGNES_MAX: usize = 200;

/// Note refusée.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ErreurNoteOd {
    /// Motif vide ou blanc.
    #[error("motif vide")]
    MotifVide,
    /// Texte de plus de [`TEXTE_MAX`] octets, ou qui contient un caractère de contrôle.
    #[error("texte trop long ou caractère de contrôle")]
    TexteInvalide,
    /// Plus de [`LIGNES_MAX`] lignes.
    #[error("trop de lignes")]
    TropDeLignes,
    /// Instant hors de l'an 0 à 9999, ou plus précis que la microseconde.
    #[error("instant invalide")]
    InstantInvalide,
    /// Date hors de l'an 0 à 9999.
    #[error("date invalide")]
    DateInvalide,
    /// Texte qui n'est pas la sérialisation canonique d'une note.
    #[error("texte non canonique")]
    NonCanonique,
}

/// Note d'opérations diverses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoteOd {
    cabinet: Uuid,
    dossier: Uuid,
    auteur: Uuid,
    instant: OffsetDateTime,
    date: Date,
    motif: String,
    lignes: Vec<String>,
}

fn texte_admis(texte: &str) -> bool {
    texte.len() <= TEXTE_MAX && !texte.chars().any(char::is_control)
}

impl NoteOd {
    /// Note du dossier `dossier` du cabinet `cabinet`, rédigée par `auteur` à `instant` (UTC,
    /// à la microseconde), pour l'opération du `date`, motivée par `motif`, expliquée par
    /// `lignes` (zéro ou plusieurs).
    ///
    /// # Errors
    /// [`ErreurNoteOd`] si un champ est refusé.
    pub fn new(
        cabinet: Uuid,
        dossier: Uuid,
        auteur: Uuid,
        instant: OffsetDateTime,
        date: Date,
        motif: String,
        lignes: Vec<String>,
    ) -> Result<Self, ErreurNoteOd> {
        let instant = instant.to_offset(UtcOffset::UTC);
        if !(0..=9999).contains(&instant.year()) || !instant.nanosecond().is_multiple_of(1000) {
            return Err(ErreurNoteOd::InstantInvalide);
        }
        if !(0..=9999).contains(&date.year()) {
            return Err(ErreurNoteOd::DateInvalide);
        }
        if motif.trim().is_empty() {
            return Err(ErreurNoteOd::MotifVide);
        }
        if !texte_admis(&motif) || !lignes.iter().all(|l| texte_admis(l)) {
            return Err(ErreurNoteOd::TexteInvalide);
        }
        if lignes.len() > LIGNES_MAX {
            return Err(ErreurNoteOd::TropDeLignes);
        }
        Ok(Self {
            cabinet,
            dossier,
            auteur,
            instant,
            date,
            motif,
            lignes,
        })
    }

    /// Date de l'opération.
    #[must_use]
    pub fn date(&self) -> Date {
        self.date
    }

    /// Motif.
    #[must_use]
    pub fn motif(&self) -> &str {
        &self.motif
    }

    /// Instant de rédaction (UTC).
    #[must_use]
    pub fn instant(&self) -> OffsetDateTime {
        self.instant
    }

    /// Forme canonique (`docs/integrite/piece-interne-v1.md` §2) : le contenu stocké et haché.
    #[must_use]
    pub fn serialiser(&self) -> String {
        let mut s = String::new();
        // `write!` sur une `String` n'échoue jamais.
        let _ = writeln!(s, "{MARQUEUR}");
        let _ = writeln!(s, "nature=note-od");
        let _ = writeln!(s, "cabinet={}", self.cabinet.hyphenated());
        let _ = writeln!(s, "dossier={}", self.dossier.hyphenated());
        let _ = writeln!(s, "auteur={}", self.auteur.hyphenated());
        let _ = writeln!(s, "instant={}", instant(self.instant));
        let _ = writeln!(s, "date={}", date(self.date));
        let _ = writeln!(s, "motif={}", self.motif);
        for ligne in &self.lignes {
            let _ = writeln!(s, "ligne={ligne}");
        }
        s.push_str("fin\n");
        s
    }

    /// Lecture stricte : le texte doit être exactement la sérialisation de la note lue.
    ///
    /// # Errors
    /// [`ErreurNoteOd::NonCanonique`] ou l'erreur du champ refusé.
    pub fn lire(texte: &str) -> Result<Self, ErreurNoteOd> {
        let corps = texte
            .strip_suffix("fin\n")
            .ok_or(ErreurNoteOd::NonCanonique)?;
        let mut lignes = corps.split_terminator('\n');
        let mut suivante = |cle: &str| -> Result<&str, ErreurNoteOd> {
            lignes
                .next()
                .and_then(|l| l.strip_prefix(cle))
                .ok_or(ErreurNoteOd::NonCanonique)
        };
        if !suivante(MARQUEUR)?.is_empty() || suivante("nature=")? != "note-od" {
            return Err(ErreurNoteOd::NonCanonique);
        }
        let cabinet = uuid_canonique(suivante("cabinet=")?)?;
        let dossier = uuid_canonique(suivante("dossier=")?)?;
        let auteur = uuid_canonique(suivante("auteur=")?)?;
        let instant_lu = lire_instant(suivante("instant=")?)?;
        let date_lue = lire_date(suivante("date=")?)?;
        let motif = suivante("motif=")?.to_owned();
        let mut explication = Vec::new();
        for ligne in lignes {
            explication.push(
                ligne
                    .strip_prefix("ligne=")
                    .ok_or(ErreurNoteOd::NonCanonique)?
                    .to_owned(),
            );
        }
        let note = Self::new(
            cabinet,
            dossier,
            auteur,
            instant_lu,
            date_lue,
            motif,
            explication,
        )?;
        if note.serialiser() == texte {
            Ok(note)
        } else {
            Err(ErreurNoteOd::NonCanonique)
        }
    }
}

fn uuid_canonique(texte: &str) -> Result<Uuid, ErreurNoteOd> {
    let uuid = Uuid::try_parse(texte).map_err(|_| ErreurNoteOd::NonCanonique)?;
    if uuid.hyphenated().to_string() == texte {
        Ok(uuid)
    } else {
        Err(ErreurNoteOd::NonCanonique)
    }
}

/// `AAAA-MM-JJ`.
#[must_use]
pub fn date(d: Date) -> String {
    format!("{:04}-{:02}-{:02}", d.year(), u8::from(d.month()), d.day())
}

/// `AAAA-MM-JJTHH:MM:SS.ffffffZ` (UTC, six décimales).
#[must_use]
pub fn instant(i: OffsetDateTime) -> String {
    let i = i.to_offset(UtcOffset::UTC);
    format!(
        "{}T{:02}:{:02}:{:02}.{:06}Z",
        date(i.date()),
        i.hour(),
        i.minute(),
        i.second(),
        i.microsecond()
    )
}

/// Chiffres décimaux exacts d'un champ de longueur fixe.
fn nombre(texte: &str) -> Option<u32> {
    (!texte.is_empty() && texte.bytes().all(|o| o.is_ascii_digit()))
        .then(|| texte.parse().ok())
        .flatten()
}

/// Lit une date `AAAA-MM-JJ` (forme exacte).
///
/// # Errors
/// [`ErreurNoteOd::DateInvalide`].
pub fn lire_date(texte: &str) -> Result<Date, ErreurNoteOd> {
    let o = texte.as_bytes();
    if o.len() != 10 || o[4] != b'-' || o[7] != b'-' {
        return Err(ErreurNoteOd::DateInvalide);
    }
    let (Some(a), Some(m), Some(j)) = (
        nombre(&texte[0..4]),
        nombre(&texte[5..7]),
        nombre(&texte[8..10]),
    ) else {
        return Err(ErreurNoteOd::DateInvalide);
    };
    let mois = u8::try_from(m)
        .ok()
        .and_then(|m| Month::try_from(m).ok())
        .ok_or(ErreurNoteOd::DateInvalide)?;
    let jour = u8::try_from(j).map_err(|_| ErreurNoteOd::DateInvalide)?;
    let annee = i32::try_from(a).map_err(|_| ErreurNoteOd::DateInvalide)?;
    Date::from_calendar_date(annee, mois, jour).map_err(|_| ErreurNoteOd::DateInvalide)
}

fn lire_instant(texte: &str) -> Result<OffsetDateTime, ErreurNoteOd> {
    let invalide = ErreurNoteOd::NonCanonique;
    let o = texte.as_bytes();
    if o.len() != 27 || o[10] != b'T' || o[13] != b':' || o[16] != b':' || o[19] != b'.' {
        return Err(invalide);
    }
    if o[26] != b'Z' {
        return Err(invalide);
    }
    let d = lire_date(&texte[0..10]).map_err(|_| invalide)?;
    let (Some(h), Some(mi), Some(s), Some(us)) = (
        nombre(&texte[11..13]),
        nombre(&texte[14..16]),
        nombre(&texte[17..19]),
        nombre(&texte[20..26]),
    ) else {
        return Err(invalide);
    };
    let heure = time::Time::from_hms_micro(
        u8::try_from(h).map_err(|_| invalide)?,
        u8::try_from(mi).map_err(|_| invalide)?,
        u8::try_from(s).map_err(|_| invalide)?,
        us,
    )
    .map_err(|_| invalide)?;
    Ok(d.with_time(heure).assume_utc())
}

#[cfg(test)]
mod tests {
    use time::macros::{date, datetime};

    use super::*;

    fn note() -> NoteOd {
        NoteOd::new(
            Uuid::parse_str("0193e0c2-7d1a-4c4e-9f00-1a2b3c4d5e6f").unwrap(),
            Uuid::parse_str("0193e0c3-1111-4a2b-8c3d-000000000001").unwrap(),
            Uuid::parse_str("0193e0c4-aaaa-4bbb-8ccc-000000000001").unwrap(),
            datetime!(2026-11-19 08:30:00.000001 UTC),
            date!(2026 - 11 - 30),
            "Dotation aux amortissements = novembre".to_owned(),
            vec![
                "Matériel de bureau, linéaire 5 ans".to_owned(),
                String::new(),
            ],
        )
        .unwrap()
    }

    #[test]
    fn aller_retour_et_lecture_stricte() {
        let n = note();
        let texte = n.serialiser();
        assert_eq!(NoteOd::lire(&texte), Ok(n));
        for altere in [
            texte.replace("fin\n", "fin"),
            texte.replace("nature=note-od", "nature=autre"),
            texte.replace("0193e0c2-7d1a-4c4e-9f00", "0193E0C2-7D1A-4C4E-9F00"),
            texte.replace(".000001Z", ".000001+00:00"),
            texte.replace("ligne=\n", "ligne=\r\n"),
            texte.replace("motif=", "motif=\t"),
            texte.replace("auteur=", "auteur=\nligne="),
            format!("\u{feff}{texte}"),
            texte.replace("ligne=Mat", "Mat"),
            texte.replace("date=2026-11-30", "date=2026-11-31"),
        ] {
            assert!(NoteOd::lire(&altere).is_err(), "{altere}");
        }
    }

    #[test]
    fn champs_refuses() {
        let n = note();
        let refaire = |motif: &str, lignes: Vec<String>, instant: OffsetDateTime| {
            NoteOd::new(
                n.cabinet,
                n.dossier,
                n.auteur,
                instant,
                n.date,
                motif.to_owned(),
                lignes,
            )
        };
        let i = n.instant;
        assert_eq!(refaire("  ", vec![], i), Err(ErreurNoteOd::MotifVide));
        assert_eq!(refaire("a\tb", vec![], i), Err(ErreurNoteOd::TexteInvalide));
        assert_eq!(
            refaire("a", vec!["x\ny".to_owned()], i),
            Err(ErreurNoteOd::TexteInvalide)
        );
        assert_eq!(
            refaire(&"é".repeat(501), vec![], i),
            Err(ErreurNoteOd::TexteInvalide)
        );
        assert!(refaire(&"é".repeat(500), vec![], i).is_ok());
        assert_eq!(
            refaire("a", vec![String::new(); 201], i),
            Err(ErreurNoteOd::TropDeLignes)
        );
        assert_eq!(
            refaire("a", vec![], datetime!(2026-11-19 08:30:00.0000001 UTC)),
            Err(ErreurNoteOd::InstantInvalide)
        );
    }

    #[test]
    fn dates() {
        assert_eq!(lire_date("2028-02-29"), Ok(date!(2028 - 02 - 29)));
        for refusee in [
            "2027-02-29",
            "2026-1-01",
            "2026-13-01",
            "+026-01-01",
            "2026/01/01",
        ] {
            assert_eq!(
                lire_date(refusee),
                Err(ErreurNoteOd::DateInvalide),
                "{refusee}"
            );
        }
    }
}
