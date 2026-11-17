//! Exercices et périodes (spec §4.2, §4.3 ; §26 point 9).
//!
//! - Exercice : `ouvert` → `en_cloture` → `cloture` (irréversible). Au plus **deux**
//!   exercices non clôturés à la fois (N en clôture, N+1 courant) ; ils se clôturent dans
//!   l'ordre chronologique.
//! - Période (mois civil, éventuellement partiel) : `ouverte` ⇄ `verrouillee` ; le
//!   déverrouillage exige un **motif**, renvoyé à l'appelant pour la journalisation (L2) ;
//!   `cloturee` suit la clôture de l'exercice.
//! - Le premier exercice a une durée libre **dans des limites qui sont un paramètre
//!   obligatoire** ([`ReglesPremierExercice`], sans valeur par défaut : spec §26 point 9). Les
//!   exercices suivants s'enchaînent sans trou et durent douze mois.
//!
//! Aucune lecture d'horloge : toutes les dates sont des paramètres. Une opération refusée
//! laisse l'état inchangé.

use std::fmt;

use time::{Date, Month};

use crate::texte::TexteCourt;

/// Durée, en mois, d'un exercice autre que le premier.
const MOIS_PAR_EXERCICE: u8 = 12;

/// Nombre maximal d'exercices non clôturés en même temps.
const EXERCICES_NON_CLOTURES_MAX: usize = 2;

/// Erreurs sur les exercices et les périodes.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ErreurExercice {
    /// Code d'exercice invalide.
    #[error("code d'exercice invalide : « {0} » (quatre chiffres attendus)")]
    CodeInvalide(String),
    /// Limites de durée du premier exercice incohérentes.
    #[error("limites de durée du premier exercice invalides : {0} à {1} mois")]
    LimitesInvalides(u8, u8),
    /// La date de fin précède la date de début.
    #[error("dates invalides : fin {1} avant début {0}")]
    DatesInvalides(Date, Date),
    /// Durée du premier exercice hors des limites.
    #[error("durée du premier exercice : {duree} mois, hors des limites {min} à {max}")]
    DureeHorsLimites {
        /// Durée constatée, en mois civils couverts.
        duree: usize,
        /// Minimum autorisé.
        min: u8,
        /// Maximum autorisé.
        max: u8,
    },
    /// Le premier exercice existe déjà.
    #[error("le premier exercice existe déjà")]
    PremierExerciceExistant,
    /// Aucun exercice n'a encore été ouvert.
    #[error("aucun exercice n'est ouvert")]
    AucunExercice,
    /// Code déjà utilisé par un autre exercice.
    #[error("le code d'exercice {0} est déjà utilisé")]
    CodeDejaUtilise(CodeExercice),
    /// Trop d'exercices non clôturés.
    #[error("deux exercices sont déjà ouverts : clôturez le plus ancien d'abord")]
    TropDExercicesOuverts,
    /// Dates hors de la plage représentable.
    #[error("date hors de la plage représentable")]
    DateHorsPlage,
    /// Exercice inconnu.
    #[error("l'exercice {0} n'existe pas")]
    ExerciceInconnu(CodeExercice),
    /// Aucune période ne contient cette date.
    #[error("aucune période ne contient le {0}")]
    PeriodeInconnue(Date),
    /// Transition de statut interdite.
    #[error("transition interdite : {0}")]
    TransitionInterdite(&'static str),
    /// Un exercice plus ancien doit d'abord avancer.
    #[error("l'exercice {0}, plus ancien, doit être clôturé (ou en clôture) avant")]
    ExerciceAnterieurNonClos(CodeExercice),
}

/// Code d'un exercice, utilisé dans la numérotation des écritures (`AC-2026-000231`) :
/// quatre chiffres.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CodeExercice(String);

impl CodeExercice {
    /// Analyse un code d'exercice.
    ///
    /// # Errors
    /// [`ErreurExercice::CodeInvalide`].
    pub fn parse(texte: &str) -> Result<Self, ErreurExercice> {
        if texte.len() == 4 && texte.bytes().all(|b| b.is_ascii_digit()) {
            Ok(Self(texte.to_owned()))
        } else {
            Err(ErreurExercice::CodeInvalide(texte.to_owned()))
        }
    }

    /// Le code.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for CodeExercice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Limites de durée du premier exercice, en mois civils couverts (même partiellement).
/// Paramètre obligatoire : sa valeur doit être validée par un expert-comptable (§26 point 9).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReglesPremierExercice {
    min_mois: u8,
    max_mois: u8,
}

impl ReglesPremierExercice {
    /// Fixe les limites.
    ///
    /// # Errors
    /// [`ErreurExercice::LimitesInvalides`] si `min` vaut 0 ou dépasse `max`.
    pub fn nouvelles(min_mois: u8, max_mois: u8) -> Result<Self, ErreurExercice> {
        if min_mois == 0 || min_mois > max_mois {
            return Err(ErreurExercice::LimitesInvalides(min_mois, max_mois));
        }
        Ok(Self { min_mois, max_mois })
    }
}

/// Statut d'un exercice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StatutExercice {
    /// Saisie et validation possibles.
    Ouvert,
    /// Travaux de fin d'exercice.
    EnCloture,
    /// Irréversible.
    Cloture,
}

/// Statut d'une période.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StatutPeriode {
    /// Saisie autorisée.
    Ouverte,
    /// Plus aucune validation dans la période.
    Verrouillee,
    /// Suit la clôture de l'exercice ; irréversible.
    Cloturee,
}

/// Période : un mois civil, réduit aux bornes de l'exercice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Periode {
    debut: Date,
    fin: Date,
    statut: StatutPeriode,
}

impl Periode {
    /// Premier jour.
    #[must_use]
    pub fn debut(&self) -> Date {
        self.debut
    }

    /// Dernier jour.
    #[must_use]
    pub fn fin(&self) -> Date {
        self.fin
    }

    /// Statut.
    #[must_use]
    pub fn statut(&self) -> StatutPeriode {
        self.statut
    }

    /// Vrai si la date appartient à la période.
    #[must_use]
    pub fn contient(&self, date: Date) -> bool {
        self.debut <= date && date <= self.fin
    }
}

/// Exercice comptable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Exercice {
    code: CodeExercice,
    debut: Date,
    fin: Date,
    statut: StatutExercice,
    periodes: Vec<Periode>,
}

impl Exercice {
    fn nouveau(code: CodeExercice, debut: Date, fin: Date) -> Result<Self, ErreurExercice> {
        if fin < debut {
            return Err(ErreurExercice::DatesInvalides(debut, fin));
        }
        Ok(Self {
            code,
            debut,
            fin,
            statut: StatutExercice::Ouvert,
            periodes: decouper_en_mois(debut, fin)?,
        })
    }

    /// Code de l'exercice.
    #[must_use]
    pub fn code(&self) -> &CodeExercice {
        &self.code
    }

    /// Premier jour.
    #[must_use]
    pub fn debut(&self) -> Date {
        self.debut
    }

    /// Dernier jour.
    #[must_use]
    pub fn fin(&self) -> Date {
        self.fin
    }

    /// Statut.
    #[must_use]
    pub fn statut(&self) -> StatutExercice {
        self.statut
    }

    /// Périodes, dans l'ordre chronologique.
    #[must_use]
    pub fn periodes(&self) -> &[Periode] {
        &self.periodes
    }

    /// Période contenant la date, si elle appartient à l'exercice.
    #[must_use]
    pub fn periode_contenant(&self, date: Date) -> Option<&Periode> {
        self.periodes.iter().find(|p| p.contient(date))
    }

    fn contient(&self, date: Date) -> bool {
        self.debut <= date && date <= self.fin
    }
}

/// Trace d'un déverrouillage de période, à journaliser par l'appelant (L2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Deverrouillage {
    /// Exercice concerné.
    pub exercice: CodeExercice,
    /// Premier jour de la période déverrouillée.
    pub periode: Date,
    /// Motif saisi.
    pub motif: TexteCourt,
}

/// Exercices d'un dossier, dans l'ordre chronologique et sans trou.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Exercices {
    regles: ReglesPremierExercice,
    exercices: Vec<Exercice>,
}

impl Exercices {
    /// Aucun exercice ; les règles du premier exercice sont obligatoires.
    #[must_use]
    pub fn new(regles: ReglesPremierExercice) -> Self {
        Self {
            regles,
            exercices: Vec::new(),
        }
    }

    /// Tous les exercices, du plus ancien au plus récent.
    #[must_use]
    pub fn exercices(&self) -> &[Exercice] {
        &self.exercices
    }

    /// Exercice par son code.
    #[must_use]
    pub fn exercice(&self, code: &CodeExercice) -> Option<&Exercice> {
        self.exercices.iter().find(|e| &e.code == code)
    }

    /// Exercice contenant la date.
    #[must_use]
    pub fn exercice_contenant(&self, date: Date) -> Option<&Exercice> {
        self.exercices.iter().find(|e| e.contient(date))
    }

    /// Ouvre le premier exercice du dossier, aux dates libres dans les limites des règles.
    ///
    /// # Errors
    /// Premier exercice déjà ouvert, dates incohérentes ou durée hors limites.
    pub fn ouvrir_premier(
        &mut self,
        code: CodeExercice,
        debut: Date,
        fin: Date,
    ) -> Result<&Exercice, ErreurExercice> {
        if !self.exercices.is_empty() {
            return Err(ErreurExercice::PremierExerciceExistant);
        }
        let exercice = Exercice::nouveau(code, debut, fin)?;
        let duree = exercice.periodes.len();
        let ReglesPremierExercice { min_mois, max_mois } = self.regles;
        if duree < usize::from(min_mois) || duree > usize::from(max_mois) {
            return Err(ErreurExercice::DureeHorsLimites {
                duree,
                min: min_mois,
                max: max_mois,
            });
        }
        Ok(self.pousser(exercice))
    }

    /// Ouvre l'exercice suivant : il commence le lendemain de la fin du dernier et dure douze
    /// mois.
    ///
    /// # Errors
    /// Aucun exercice, code déjà utilisé ou déjà deux exercices non clôturés.
    pub fn ouvrir_suivant(&mut self, code: CodeExercice) -> Result<&Exercice, ErreurExercice> {
        let dernier = self.exercices.last().ok_or(ErreurExercice::AucunExercice)?;
        if self.exercice(&code).is_some() {
            return Err(ErreurExercice::CodeDejaUtilise(code));
        }
        let non_clotures = self
            .exercices
            .iter()
            .filter(|e| e.statut != StatutExercice::Cloture)
            .count();
        if non_clotures >= EXERCICES_NON_CLOTURES_MAX {
            return Err(ErreurExercice::TropDExercicesOuverts);
        }
        let debut = dernier
            .fin
            .next_day()
            .ok_or(ErreurExercice::DateHorsPlage)?;
        let fin = ajouter_mois(debut, MOIS_PAR_EXERCICE)?
            .previous_day()
            .ok_or(ErreurExercice::DateHorsPlage)?;
        let exercice = Exercice::nouveau(code, debut, fin)?;
        Ok(self.pousser(exercice))
    }

    /// Passe un exercice ouvert en clôture. Les exercices plus anciens doivent être en
    /// clôture ou clôturés.
    ///
    /// # Errors
    /// Exercice inconnu, pas ouvert, ou exercice antérieur encore ouvert.
    pub fn passer_en_cloture(&mut self, code: &CodeExercice) -> Result<(), ErreurExercice> {
        let index = self.index(code)?;
        if self.exercices[index].statut != StatutExercice::Ouvert {
            return Err(ErreurExercice::TransitionInterdite(
                "seul un exercice ouvert passe en clôture",
            ));
        }
        if let Some(anterieur) = self.exercices[..index]
            .iter()
            .find(|e| e.statut == StatutExercice::Ouvert)
        {
            return Err(ErreurExercice::ExerciceAnterieurNonClos(
                anterieur.code.clone(),
            ));
        }
        self.exercices[index].statut = StatutExercice::EnCloture;
        Ok(())
    }

    /// Clôture un exercice en clôture : irréversible ; toutes ses périodes sont clôturées.
    /// Les exercices plus anciens doivent être clôturés.
    ///
    /// # Errors
    /// Exercice inconnu, pas en clôture, ou exercice antérieur non clôturé.
    pub fn cloturer(&mut self, code: &CodeExercice) -> Result<(), ErreurExercice> {
        let index = self.index(code)?;
        if self.exercices[index].statut != StatutExercice::EnCloture {
            return Err(ErreurExercice::TransitionInterdite(
                "seul un exercice en clôture peut être clôturé",
            ));
        }
        if let Some(anterieur) = self.exercices[..index]
            .iter()
            .find(|e| e.statut != StatutExercice::Cloture)
        {
            return Err(ErreurExercice::ExerciceAnterieurNonClos(
                anterieur.code.clone(),
            ));
        }
        let exercice = &mut self.exercices[index];
        exercice.statut = StatutExercice::Cloture;
        for periode in &mut exercice.periodes {
            periode.statut = StatutPeriode::Cloturee;
        }
        Ok(())
    }

    /// Verrouille la période contenant la date.
    ///
    /// # Errors
    /// Aucune période ne contient la date, ou la période n'est pas ouverte.
    pub fn verrouiller_periode(&mut self, date: Date) -> Result<(), ErreurExercice> {
        let periode = self.periode_mut(date)?.1;
        if periode.statut != StatutPeriode::Ouverte {
            return Err(ErreurExercice::TransitionInterdite(
                "seule une période ouverte peut être verrouillée",
            ));
        }
        periode.statut = StatutPeriode::Verrouillee;
        Ok(())
    }

    /// Déverrouille la période contenant la date. Le motif est obligatoire ; la trace
    /// renvoyée doit être journalisée par l'appelant.
    ///
    /// # Errors
    /// Aucune période ne contient la date, ou la période n'est pas verrouillée.
    pub fn deverrouiller_periode(
        &mut self,
        date: Date,
        motif: TexteCourt,
    ) -> Result<Deverrouillage, ErreurExercice> {
        let (exercice, periode) = self.periode_mut(date)?;
        if periode.statut != StatutPeriode::Verrouillee {
            return Err(ErreurExercice::TransitionInterdite(
                "seule une période verrouillée peut être déverrouillée",
            ));
        }
        periode.statut = StatutPeriode::Ouverte;
        Ok(Deverrouillage {
            exercice,
            periode: periode.debut,
            motif,
        })
    }

    fn pousser(&mut self, exercice: Exercice) -> &Exercice {
        self.exercices.push(exercice);
        let dernier = self.exercices.len() - 1;
        &self.exercices[dernier]
    }

    fn index(&self, code: &CodeExercice) -> Result<usize, ErreurExercice> {
        self.exercices
            .iter()
            .position(|e| &e.code == code)
            .ok_or_else(|| ErreurExercice::ExerciceInconnu(code.clone()))
    }

    fn periode_mut(&mut self, date: Date) -> Result<(CodeExercice, &mut Periode), ErreurExercice> {
        let exercice = self
            .exercices
            .iter_mut()
            .find(|e| e.contient(date))
            .ok_or(ErreurExercice::PeriodeInconnue(date))?;
        let code = exercice.code.clone();
        let periode = exercice
            .periodes
            .iter_mut()
            .find(|p| p.contient(date))
            .ok_or(ErreurExercice::PeriodeInconnue(date))?;
        Ok((code, periode))
    }
}

/// Découpe `[debut, fin]` en mois civils, le premier et le dernier éventuellement partiels.
fn decouper_en_mois(debut: Date, fin: Date) -> Result<Vec<Periode>, ErreurExercice> {
    let mut periodes = Vec::new();
    let mut courant = debut;
    while courant <= fin {
        let debut_mois_suivant = ajouter_mois(
            courant
                .replace_day(1)
                .map_err(|_| ErreurExercice::DateHorsPlage)?,
            1,
        )?;
        let fin_mois = debut_mois_suivant
            .previous_day()
            .ok_or(ErreurExercice::DateHorsPlage)?;
        periodes.push(Periode {
            debut: courant,
            fin: fin_mois.min(fin),
            statut: StatutPeriode::Ouverte,
        });
        courant = debut_mois_suivant;
    }
    Ok(periodes)
}

/// Même jour `mois` mois plus tard ; si ce jour n'existe pas (31, 29 février), le premier
/// jour du mois suivant. Ainsi `ajouter_mois(d, 12) - 1 jour` est la fin d'un exercice de
/// douze mois commençant le `d`.
fn ajouter_mois(date: Date, mois: u8) -> Result<Date, ErreurExercice> {
    let index = i32::from(u8::from(date.month())) - 1 + i32::from(mois);
    let annee = date.year() + index.div_euclid(12);
    let mois_cible = Month::try_from(
        u8::try_from(index.rem_euclid(12) + 1).map_err(|_| ErreurExercice::DateHorsPlage)?,
    )
    .map_err(|_| ErreurExercice::DateHorsPlage)?;
    if let Ok(d) = Date::from_calendar_date(annee, mois_cible, date.day()) {
        return Ok(d);
    }
    // Le jour demandé dépasse la fin du mois cible : 1er du mois suivant.
    let premier = Date::from_calendar_date(annee, mois_cible, 1)
        .map_err(|_| ErreurExercice::DateHorsPlage)?;
    ajouter_mois(premier, 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use time::Duration;
    use time::macros::date;

    fn code(texte: &str) -> CodeExercice {
        CodeExercice::parse(texte).unwrap()
    }

    fn motif() -> TexteCourt {
        TexteCourt::parse("Correction de la déclaration de TVA").unwrap()
    }

    fn regles() -> ReglesPremierExercice {
        // Valeurs de test, sans portée normative (§26 point 9).
        ReglesPremierExercice::nouvelles(1, 24).unwrap()
    }

    #[test]
    fn codes_et_regles() {
        assert!(CodeExercice::parse("2026").is_ok());
        for texte in ["26", "20266", "2O26", ""] {
            assert!(CodeExercice::parse(texte).is_err(), "{texte}");
        }
        assert!(ReglesPremierExercice::nouvelles(0, 12).is_err());
        assert!(ReglesPremierExercice::nouvelles(13, 12).is_err());
    }

    #[test]
    fn premier_exercice_et_periodes() {
        let mut exercices = Exercices::new(regles());
        let e = exercices
            .ouvrir_premier(code("2026"), date!(2026 - 03 - 15), date!(2026 - 12 - 31))
            .unwrap();
        assert_eq!(e.periodes().len(), 10);
        assert_eq!(e.periodes()[0].debut(), date!(2026 - 03 - 15));
        assert_eq!(e.periodes()[0].fin(), date!(2026 - 03 - 31));
        assert_eq!(e.periodes()[1].debut(), date!(2026 - 04 - 01));
        assert_eq!(e.periodes()[9].fin(), date!(2026 - 12 - 31));
        assert_eq!(
            exercices.ouvrir_premier(code("2027"), date!(2027 - 01 - 01), date!(2027 - 12 - 31)),
            Err(ErreurExercice::PremierExerciceExistant)
        );
    }

    #[test]
    fn limites_du_premier_exercice() {
        let mut exercices = Exercices::new(ReglesPremierExercice::nouvelles(6, 18).unwrap());
        assert_eq!(
            exercices.ouvrir_premier(code("2026"), date!(2026 - 09 - 01), date!(2026 - 12 - 31)),
            Err(ErreurExercice::DureeHorsLimites {
                duree: 4,
                min: 6,
                max: 18
            })
        );
        assert_eq!(
            exercices.ouvrir_premier(code("2026"), date!(2026 - 01 - 01), date!(2027 - 12 - 31)),
            Err(ErreurExercice::DureeHorsLimites {
                duree: 24,
                min: 6,
                max: 18
            })
        );
        assert!(matches!(
            exercices.ouvrir_premier(code("2026"), date!(2026 - 12 - 31), date!(2026 - 01 - 01)),
            Err(ErreurExercice::DatesInvalides(_, _))
        ));
        assert_eq!(exercices.exercices(), &[] as &[Exercice]);
    }

    #[test]
    fn exercice_suivant_de_douze_mois() {
        let mut exercices = Exercices::new(regles());
        exercices
            .ouvrir_premier(code("2027"), date!(2027 - 03 - 01), date!(2028 - 02 - 29))
            .unwrap();
        let e = exercices.ouvrir_suivant(code("2028")).unwrap();
        assert_eq!(e.debut(), date!(2028 - 03 - 01));
        assert_eq!(e.fin(), date!(2029 - 02 - 28));
        assert_eq!(e.periodes().len(), 12);
        assert_eq!(
            exercices.ouvrir_suivant(code("2029")),
            Err(ErreurExercice::TropDExercicesOuverts)
        );
        assert_eq!(
            exercices.ouvrir_suivant(code("2028")),
            Err(ErreurExercice::CodeDejaUtilise(code("2028")))
        );
    }

    #[test]
    fn ajout_de_mois() {
        assert_eq!(
            ajouter_mois(date!(2024 - 02 - 29), 12).unwrap(),
            date!(2025 - 03 - 01)
        );
        assert_eq!(
            ajouter_mois(date!(2026 - 01 - 31), 1).unwrap(),
            date!(2026 - 03 - 01)
        );
        assert_eq!(
            ajouter_mois(date!(2026 - 11 - 15), 2).unwrap(),
            date!(2027 - 01 - 15)
        );
    }

    #[test]
    fn cloture_dans_l_ordre() {
        let mut exercices = Exercices::new(regles());
        exercices
            .ouvrir_premier(code("2026"), date!(2026 - 01 - 01), date!(2026 - 12 - 31))
            .unwrap();
        exercices.ouvrir_suivant(code("2027")).unwrap();
        assert_eq!(
            exercices.passer_en_cloture(&code("2027")),
            Err(ErreurExercice::ExerciceAnterieurNonClos(code("2026")))
        );
        assert!(
            exercices.cloturer(&code("2026")).is_err(),
            "pas encore en clôture"
        );
        exercices.passer_en_cloture(&code("2026")).unwrap();
        exercices.passer_en_cloture(&code("2027")).unwrap();
        assert_eq!(
            exercices.cloturer(&code("2027")),
            Err(ErreurExercice::ExerciceAnterieurNonClos(code("2026")))
        );
        exercices.cloturer(&code("2026")).unwrap();
        let e = exercices.exercice(&code("2026")).unwrap();
        assert!(
            e.periodes()
                .iter()
                .all(|p| p.statut() == StatutPeriode::Cloturee)
        );
        assert!(exercices.passer_en_cloture(&code("2026")).is_err());
        assert!(exercices.cloturer(&code("2026")).is_err());
        assert!(
            exercices
                .verrouiller_periode(date!(2026 - 05 - 10))
                .is_err()
        );
        exercices.ouvrir_suivant(code("2028")).unwrap();
        assert_eq!(
            exercices.cloturer(&code("1999")),
            Err(ErreurExercice::ExerciceInconnu(code("1999")))
        );
    }

    #[test]
    fn verrouillage_et_deverrouillage() {
        let mut exercices = Exercices::new(regles());
        exercices
            .ouvrir_premier(code("2026"), date!(2026 - 01 - 01), date!(2026 - 12 - 31))
            .unwrap();
        exercices
            .verrouiller_periode(date!(2026 - 03 - 31))
            .unwrap();
        assert!(
            exercices
                .verrouiller_periode(date!(2026 - 03 - 01))
                .is_err()
        );
        let trace = exercices
            .deverrouiller_periode(date!(2026 - 03 - 15), motif())
            .unwrap();
        assert_eq!(trace.exercice, code("2026"));
        assert_eq!(trace.periode, date!(2026 - 03 - 01));
        assert_eq!(trace.motif, motif());
        assert!(
            exercices
                .deverrouiller_periode(date!(2026 - 03 - 15), motif())
                .is_err()
        );
        assert_eq!(
            exercices.verrouiller_periode(date!(2027 - 01 - 01)),
            Err(ErreurExercice::PeriodeInconnue(date!(2027 - 01 - 01)))
        );
    }

    #[derive(Debug, Clone)]
    enum Operation {
        Suivant(u16),
        EnCloture(usize),
        Cloturer(usize),
        Verrouiller(i64),
        Deverrouiller(i64),
    }

    fn operation() -> impl Strategy<Value = Operation> {
        prop_oneof![
            (2000u16..2100).prop_map(Operation::Suivant),
            (0usize..6).prop_map(Operation::EnCloture),
            (0usize..6).prop_map(Operation::Cloturer),
            (0i64..2000).prop_map(Operation::Verrouiller),
            (0i64..2000).prop_map(Operation::Deverrouiller),
        ]
    }

    fn invariants(exercices: &Exercices) -> Result<(), TestCaseError> {
        let liste = exercices.exercices();
        let non_clotures = liste
            .iter()
            .filter(|e| e.statut() != StatutExercice::Cloture)
            .count();
        prop_assert!(non_clotures <= EXERCICES_NON_CLOTURES_MAX);
        for paire in liste.windows(2) {
            prop_assert_eq!(paire[0].fin().next_day(), Some(paire[1].debut()));
            // Clôture dans l'ordre : un exercice clôturé n'a que des prédécesseurs clôturés.
            if paire[1].statut() == StatutExercice::Cloture {
                prop_assert_eq!(paire[0].statut(), StatutExercice::Cloture);
            }
        }
        for e in liste {
            let p = e.periodes();
            prop_assert_eq!(p.first().map(Periode::debut), Some(e.debut()));
            prop_assert_eq!(p.last().map(Periode::fin), Some(e.fin()));
            for paire in p.windows(2) {
                prop_assert_eq!(paire[0].fin().next_day(), Some(paire[1].debut()));
            }
            let toutes_cloturees = p.iter().all(|x| x.statut() == StatutPeriode::Cloturee);
            let aucune_cloturee = p.iter().all(|x| x.statut() != StatutPeriode::Cloturee);
            if e.statut() == StatutExercice::Cloture {
                prop_assert!(toutes_cloturees);
            } else {
                prop_assert!(aucune_cloturee);
            }
        }
        Ok(())
    }

    proptest! {
        /// Aucune suite d'opérations ne viole les invariants ; une opération refusée laisse
        /// l'état inchangé ; un exercice clôturé ne change plus jamais.
        #[test]
        fn transitions(ops in proptest::collection::vec(operation(), 0..40)) {
            let mut exercices = Exercices::new(regles());
            exercices.ouvrir_premier(code("1999"), date!(2026 - 01 - 01), date!(2026 - 12 - 31)).unwrap();
            for op in ops {
                let avant = exercices.clone();
                let codes: Vec<CodeExercice> = exercices.exercices().iter().map(|e| e.code().clone()).collect();
                let jour = |d: i64| date!(2026 - 01 - 01) + Duration::days(d);
                let resultat = match op {
                    Operation::Suivant(a) => exercices.ouvrir_suivant(code(&a.to_string())).map(|_| ()),
                    Operation::EnCloture(i) => exercices.passer_en_cloture(&codes[i % codes.len()]),
                    Operation::Cloturer(i) => exercices.cloturer(&codes[i % codes.len()]),
                    Operation::Verrouiller(d) => exercices.verrouiller_periode(jour(d)),
                    Operation::Deverrouiller(d) => exercices.deverrouiller_periode(jour(d), motif()).map(|_| ()),
                };
                if resultat.is_err() {
                    prop_assert_eq!(&exercices, &avant);
                }
                for (a, b) in avant.exercices().iter().zip(exercices.exercices()) {
                    if a.statut() == StatutExercice::Cloture {
                        prop_assert_eq!(a, b);
                    }
                }
                invariants(&exercices)?;
            }
        }
    }
}
