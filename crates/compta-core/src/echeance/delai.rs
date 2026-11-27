//! Délai d'un terme de règlement : la date d'échéance à partir d'une date de référence
//! (ADR 0033, point 3).
//!
//! Liste fermée des quatre types de délai d'Odoo (`account_payment_term.py:294-299`,
//! `_get_due_date` `:310-327`) : la seule à couvrir « 30 jours fin de mois » et « le X du mois
//! suivant » sans généraliser à la façon de Tryton (`relativedelta`, `payment_term.py:222-263`).
//! Tout calcul de date est vérifié : une date hors des bornes du calendrier renvoie une erreur.

// Portions portées d'Odoo (addons/account/models/account_payment_term.py, `_get_due_date`,
// commit fa175fb2b1dd69883188ddbd07d0d0b7a6cf9118), LGPL-3.0, © Odoo S.A.
// Voir docs/provenance/odoo.md#delais-de-reglement.

use time::{Date, Duration, Month};

use super::ErreurEcheance;

/// Jour du mois d'une échéance « le X du mois suivant » : 1 à 31, borné au dernier jour du
/// mois (le 31 donne le 30 avril, le 28 ou le 29 février) ; **0 désigne la fin du mois**
/// (Odoo `:323-324`, contrainte `0 <= X <= 31` `:328-336`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct JourDuMois(u8);

impl JourDuMois {
    /// Plus grand jour admis.
    pub const MAX: u8 = 31;

    /// Construit un jour du mois.
    ///
    /// # Errors
    /// [`ErreurEcheance::JourDuMoisInvalide`] au-delà de [`Self::MAX`].
    pub fn nouveau(jour: u8) -> Result<Self, ErreurEcheance> {
        if jour > Self::MAX {
            return Err(ErreurEcheance::JourDuMoisInvalide(jour));
        }
        Ok(Self(jour))
    }

    /// Le jour (0 : fin du mois).
    #[must_use]
    pub fn jour(&self) -> u8 {
        self.0
    }
}

/// Délai d'un terme. Les nombres de jours sont des entiers signés, comme chez Tryton (`days`,
/// `Integer`) et Odoo (`nb_days`, `Integer`), qui n'interdisent pas un nombre négatif ; une date
/// calculée hors des bornes du calendrier renvoie [`ErreurEcheance::DateHorsLimites`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Delai {
    /// Jours après la date de référence (Odoo `days_after`).
    Jours(i32),
    /// Jours après la fin du mois de la date de référence (Odoo `days_after_end_of_month`) :
    /// `JoursApresFinDeMois(0)` est la fin du mois, `JoursApresFinDeMois(10)` le 10 du mois
    /// suivant.
    JoursApresFinDeMois(i32),
    /// Jours après la fin du mois qui suit la date de référence (Odoo
    /// `days_after_end_of_next_month`).
    JoursApresFinDuMoisSuivant(i32),
    /// Jours après la date de référence, puis le jour X du mois suivant ; X nul : la fin du
    /// mois atteint après les jours, **sans** passer au mois suivant (Odoo
    /// `days_end_of_month_on_the`, `:318-326`).
    JoursPuisJourDuMoisSuivant {
        /// Jours ajoutés d'abord.
        jours: i32,
        /// Jour du mois suivant (0 : fin du mois atteint).
        jour: JourDuMois,
    },
}

impl Delai {
    /// Date d'échéance à partir de la date de référence (date de la pièce ; en L3, la date
    /// comptable de l'écriture).
    ///
    /// # Errors
    /// [`ErreurEcheance::DateHorsLimites`] si la date calculée sort du calendrier.
    pub fn echeance(&self, reference: Date) -> Result<Date, ErreurEcheance> {
        match *self {
            Delai::Jours(jours) => ajouter_jours(reference, jours),
            Delai::JoursApresFinDeMois(jours) => ajouter_jours(fin_de_mois(reference), jours),
            Delai::JoursApresFinDuMoisSuivant(jours) => {
                ajouter_jours(fin_de_mois(premier_du_mois_suivant(reference)?), jours)
            }
            Delai::JoursPuisJourDuMoisSuivant { jours, jour } => {
                let date = ajouter_jours(reference, jours)?;
                if jour.0 == 0 {
                    return Ok(fin_de_mois(date));
                }
                let suivant = premier_du_mois_suivant(date)?;
                let dernier = suivant.month().length(suivant.year());
                suivant
                    .replace_day(jour.0.min(dernier))
                    .map_err(|_| ErreurEcheance::DateHorsLimites)
            }
        }
    }
}

/// Dernier jour du mois de `date` (28 ou 29 février selon l'année).
pub(super) fn fin_de_mois(date: Date) -> Date {
    let dernier = date.month().length(date.year());
    // Le dernier jour du mois existe toujours dans l'année de `date`.
    date.replace_day(dernier).unwrap_or(date)
}

/// Premier jour du mois qui suit celui de `date`.
fn premier_du_mois_suivant(date: Date) -> Result<Date, ErreurEcheance> {
    let (annee, mois) = match date.month() {
        Month::December => (date.year().checked_add(1), Month::January),
        mois => (Some(date.year()), mois.next()),
    };
    annee
        .and_then(|a| Date::from_calendar_date(a, mois, 1).ok())
        .ok_or(ErreurEcheance::DateHorsLimites)
}

fn ajouter_jours(date: Date, jours: i32) -> Result<Date, ErreurEcheance> {
    date.checked_add(Duration::days(i64::from(jours)))
        .ok_or(ErreurEcheance::DateHorsLimites)
}
