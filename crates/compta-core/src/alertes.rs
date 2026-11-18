//! Alerte de brouillard non validé (spec §6.1 ; lot L2, alertes).
//!
//! **Fondement.** AUDCIF art. 22 2° (p. PDF 23) : toute donnée entrée fait l'objet d'une
//! validation, mise en œuvre « au terme de chaque période qui ne peut excéder un mois » ; la
//! période est « préalablement déterminée » (art. 16 al. 4, p. PDF 21). Ici, la période est
//! celle du dossier : le mois civil, réduit aux bornes de l'exercice ([`crate::Periode`]).
//!
//! **Règles.**
//! - La période de rattachement d'une écriture au brouillard est celle qui contient sa date
//!   comptable ; si aucun exercice ne la contient, le mois civil de cette date.
//! - L'écriture est **en retard** dès que la date du jour dépasse la fin de cette période : le
//!   dernier jour de la période n'est pas en retard, le lendemain l'est.
//! - **Aucun délai de grâce** : le 2° n'en prévoit pas, à la différence du 3° (clôture
//!   informatique, qui a son échéance : [`crate::echeance_de_cloture`]). C'est un choix fondé sur
//!   ce contraste.
//! - C'est une **alerte**, jamais un blocage : la saisie et la validation restent régies par
//!   [`crate::Livre::valider`].
//!
//! **Moteurs de référence.** Aucun n'alerte sur l'âge du brouillard : Dolibarr compte les
//! lignes non validées par mois à la clôture, Odoo compte les brouillons par journal.
//!
//! Aucune lecture d'horloge : la date du jour est un paramètre.

use std::collections::BTreeMap;

use time::Date;

use crate::exercice::Exercices;
use crate::journal::CodeJournal;

/// Retard de validation d'une écriture au brouillard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetardValidation {
    /// Premier jour de la période de rattachement.
    pub debut_periode: Date,
    /// Dernier jour de la période de rattachement : la validation devait intervenir au plus
    /// tard ce jour-là.
    pub fin_periode: Date,
    /// Jours écoulés depuis la fin de la période (au moins 1).
    pub jours_de_retard: i64,
}

/// Écritures au brouillard en retard, pour un journal et une période.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlerteBrouillard {
    /// Journal.
    pub journal: CodeJournal,
    /// Premier jour de la période.
    pub debut_periode: Date,
    /// Dernier jour de la période.
    pub fin_periode: Date,
    /// Nombre d'écritures en retard.
    pub nombre: usize,
    /// Date comptable la plus ancienne parmi elles.
    pub plus_ancienne: Date,
    /// Jours écoulés depuis la fin de la période (au moins 1).
    pub jours_de_retard: i64,
}

/// Bornes de la période de rattachement : celle de l'exercice qui contient la date, sinon le
/// mois civil de la date. `None` seulement si la fin du mois n'est pas représentable.
fn periode_de_rattachement(date: Date, exercices: &Exercices) -> Option<(Date, Date)> {
    if let Some(periode) = exercices
        .exercice_contenant(date)
        .and_then(|e| e.periode_contenant(date))
    {
        return Some((periode.debut(), periode.fin()));
    }
    let debut = date.replace_day(1).ok()?;
    let fin = date.replace_day(date.month().length(date.year())).ok()?;
    Some((debut, fin))
}

/// Retard de validation d'une écriture au brouillard datée de `date`, ou `None` si elle n'est
/// pas en retard au `date_du_jour` (voir la documentation du module).
#[must_use]
pub fn retard_de_validation(
    date: Date,
    exercices: &Exercices,
    date_du_jour: Date,
) -> Option<RetardValidation> {
    let (debut_periode, fin_periode) = periode_de_rattachement(date, exercices)?;
    (date_du_jour > fin_periode).then(|| RetardValidation {
        debut_periode,
        fin_periode,
        jours_de_retard: (date_du_jour - fin_periode).whole_days(),
    })
}

/// Écritures au brouillard en retard de validation, agrégées par (période, journal) et triées
/// par période (la plus ancienne d'abord) puis par code de journal. L'entrée est une suite de
/// paires (journal, date comptable), pour que l'appelant passe directement ses lignes de
/// brouillard, équilibrées ou non.
#[must_use]
pub fn brouillard_en_retard<'a>(
    brouillons: impl IntoIterator<Item = (&'a CodeJournal, Date)>,
    exercices: &Exercices,
    date_du_jour: Date,
) -> Vec<AlerteBrouillard> {
    let mut groupes: BTreeMap<(Date, Date, &'a CodeJournal), AlerteBrouillard> = BTreeMap::new();
    for (journal, date) in brouillons {
        let Some(retard) = retard_de_validation(date, exercices, date_du_jour) else {
            continue;
        };
        groupes
            .entry((retard.debut_periode, retard.fin_periode, journal))
            .and_modify(|alerte| {
                alerte.nombre = alerte.nombre.saturating_add(1);
                alerte.plus_ancienne = alerte.plus_ancienne.min(date);
            })
            .or_insert_with(|| AlerteBrouillard {
                journal: journal.clone(),
                debut_periode: retard.debut_periode,
                fin_periode: retard.fin_periode,
                nombre: 1,
                plus_ancienne: date,
                jours_de_retard: retard.jours_de_retard,
            });
    }
    groupes.into_values().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exercice::FinPremierExercice;
    use time::macros::date;

    fn exercices(debut: Date) -> Exercices {
        let mut exercices = Exercices::new();
        assert!(
            exercices
                .ouvrir_premier(debut, FinPremierExercice::AnneeDeDebut)
                .is_ok()
        );
        exercices
    }

    fn j(code: &str) -> CodeJournal {
        CodeJournal::parse(code).unwrap_or_else(|e| panic!("{e}"))
    }

    #[test]
    fn veille_jour_meme_et_lendemain_de_la_fin_de_periode() {
        let ex = exercices(date!(2026 - 01 - 01));
        let date = date!(2026 - 03 - 10);
        assert_eq!(retard_de_validation(date, &ex, date!(2026 - 03 - 30)), None);
        assert_eq!(retard_de_validation(date, &ex, date!(2026 - 03 - 31)), None);
        assert_eq!(
            retard_de_validation(date, &ex, date!(2026 - 04 - 01)),
            Some(RetardValidation {
                debut_periode: date!(2026 - 03 - 01),
                fin_periode: date!(2026 - 03 - 31),
                jours_de_retard: 1,
            })
        );
        assert_eq!(
            retard_de_validation(date, &ex, date!(2026 - 05 - 01)).map(|r| r.jours_de_retard),
            Some(31)
        );
    }

    #[test]
    fn date_hors_exercice_rattachee_au_mois_civil() {
        let ex = exercices(date!(2026 - 01 - 01));
        // Février d'une année bissextile, sans exercice.
        let retard = retard_de_validation(date!(2028 - 02 - 10), &ex, date!(2028 - 03 - 02));
        assert_eq!(
            retard,
            Some(RetardValidation {
                debut_periode: date!(2028 - 02 - 01),
                fin_periode: date!(2028 - 02 - 29),
                jours_de_retard: 2,
            })
        );
        assert_eq!(
            retard_de_validation(date!(2028 - 02 - 10), &ex, date!(2028 - 02 - 29)),
            None
        );
        // Aucun exercice du tout.
        assert_eq!(
            retard_de_validation(
                date!(2025 - 12 - 31),
                &Exercices::new(),
                date!(2026 - 01 - 01)
            )
            .map(|r| (r.debut_periode, r.fin_periode)),
            Some((date!(2025 - 12 - 01), date!(2025 - 12 - 31)))
        );
    }

    /// Premier exercice ouvert le 15 décembre : sa seule période, qui est aussi la dernière,
    /// est partielle (15 au 31 décembre). Un exercice se termine toujours un 31 décembre
    /// (AUDCIF art. 7) : la dernière période ne peut être partielle que de cette façon.
    #[test]
    fn derniere_periode_partielle() {
        let ex = exercices(date!(2026 - 12 - 15));
        assert_eq!(ex.exercices()[0].periodes().len(), 1);
        assert_eq!(
            retard_de_validation(date!(2026 - 12 - 20), &ex, date!(2027 - 01 - 01)),
            Some(RetardValidation {
                debut_periode: date!(2026 - 12 - 15),
                fin_periode: date!(2026 - 12 - 31),
                jours_de_retard: 1,
            })
        );
        // Avant le début de l'exercice : mois civil complet.
        assert_eq!(
            retard_de_validation(date!(2026 - 12 - 10), &ex, date!(2027 - 01 - 01))
                .map(|r| r.debut_periode),
            Some(date!(2026 - 12 - 01))
        );
        // Premier exercice ouvert le 15 mars : première période partielle.
        let ex = exercices(date!(2026 - 03 - 15));
        assert_eq!(
            retard_de_validation(date!(2026 - 03 - 20), &ex, date!(2026 - 04 - 03)).map(|r| (
                r.debut_periode,
                r.fin_periode,
                r.jours_de_retard
            )),
            Some((date!(2026 - 03 - 15), date!(2026 - 03 - 31), 3))
        );
    }

    #[test]
    fn agregation_par_periode_et_journal() {
        let ex = exercices(date!(2026 - 01 - 01));
        let (ac, vt) = (j("AC"), j("VT"));
        let brouillons = [
            (&vt, date!(2026 - 02 - 20)),
            (&ac, date!(2026 - 03 - 12)),
            (&ac, date!(2026 - 02 - 25)),
            (&ac, date!(2026 - 02 - 03)),
            // Mois courant : pas en retard.
            (&ac, date!(2026 - 04 - 01)),
        ];
        let alertes = brouillard_en_retard(brouillons, &ex, date!(2026 - 04 - 10));
        let resume: Vec<_> = alertes
            .iter()
            .map(|a| {
                (
                    a.journal.as_str(),
                    a.fin_periode,
                    a.nombre,
                    a.plus_ancienne,
                    a.jours_de_retard,
                )
            })
            .collect();
        assert_eq!(
            resume,
            [
                ("AC", date!(2026 - 02 - 28), 2, date!(2026 - 02 - 03), 41),
                ("VT", date!(2026 - 02 - 28), 1, date!(2026 - 02 - 20), 41),
                ("AC", date!(2026 - 03 - 31), 1, date!(2026 - 03 - 12), 10),
            ]
        );
        assert_eq!(
            brouillard_en_retard([(&ac, date!(2026 - 04 - 01))], &ex, date!(2026 - 04 - 30)),
            []
        );
    }
}
