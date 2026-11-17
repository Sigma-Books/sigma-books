//! Exercices et périodes (spec §4.2, §4.3).
//!
//! Règles de l'AUDCIF, art. 7 (JO OHADA du 15/02/2017, p. 15), reprises par la spec §4.2 :
//! - l'exercice **coïncide avec l'année civile** : du 1er janvier au 31 décembre ;
//! - le **premier exercice** commence à la date de début d'activité et se termine le
//!   31 décembre de la même année ; s'il débute au second semestre (après le 30 juin), il
//!   peut, au choix fait à sa création, se terminer le 31 décembre de l'année suivante ;
//! - un exercice est désigné par son **année de clôture** ([`CodeExercice`]), reprise dans
//!   la numérotation des écritures (spec §6.3). Le code est dérivé de la date de clôture,
//!   jamais choisi par l'appelant.
//!
//! La cessation d'activité (exercice de liquidation) relève du lot L6.
//!
//! Statuts :
//! - exercice : `ouvert` → `en_cloture` → `cloture` (irréversible), sans retour. Au plus
//!   **deux** exercices non clôturés à la fois (N en clôture, N+1 courant) ; ils passent en
//!   clôture et se clôturent dans l'ordre chronologique ;
//! - période (mois civil, le premier éventuellement partiel) : `ouverte` ⇄ `verrouillee` ;
//!   le déverrouillage exige un **motif**, renvoyé à l'appelant pour la journalisation (L2) ;
//!   `cloturee` suit la clôture de l'exercice.
//!
//! Aucune lecture d'horloge : toutes les dates sont des paramètres. Une opération refusée
//! laisse l'état inchangé.

use std::fmt;

use time::{Date, Month};

use crate::texte::TexteCourt;

/// Nombre maximal d'exercices non clôturés en même temps.
const EXERCICES_NON_CLOTURES_MAX: usize = 2;

/// Erreurs sur les exercices et les périodes.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ErreurExercice {
    /// Code d'exercice invalide.
    #[error("code d'exercice invalide : « {0} » (quatre chiffres attendus)")]
    CodeInvalide(String),
    /// Prolongation jusqu'à l'année suivante demandée pour un premier exercice qui débute
    /// au premier semestre.
    #[error(
        "le premier exercice débute le {0} : il ne peut se terminer l'année suivante que s'il \
         débute au second semestre (AUDCIF art. 7)"
    )]
    ProlongationImpossible(Date),
    /// Le premier exercice existe déjà.
    #[error("le premier exercice existe déjà")]
    PremierExerciceExistant,
    /// Aucun exercice n'a encore été ouvert.
    #[error("aucun exercice n'est ouvert")]
    AucunExercice,
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

/// Code d'un exercice : son **année de clôture**, sur quatre chiffres, reprise dans la
/// numérotation des écritures (`AC-2026-000231`).
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

    /// Code d'un exercice qui se clôture dans l'année donnée.
    fn de_l_annee(annee: i32) -> Result<Self, ErreurExercice> {
        if (1000..=9999).contains(&annee) {
            Ok(Self(annee.to_string()))
        } else {
            Err(ErreurExercice::DateHorsPlage)
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

/// Fin du premier exercice (AUDCIF art. 7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FinPremierExercice {
    /// Le 31 décembre de l'année de début d'activité.
    AnneeDeDebut,
    /// Le 31 décembre de l'année suivante : seulement si l'activité débute au second semestre.
    AnneeSuivante,
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
    /// Exercice du `debut` au 31 décembre de `annee_de_cloture` (≥ année de `debut`).
    fn nouveau(debut: Date, annee_de_cloture: i32) -> Result<Self, ErreurExercice> {
        let code = CodeExercice::de_l_annee(annee_de_cloture)?;
        let fin = Date::from_calendar_date(annee_de_cloture, Month::December, 31)
            .map_err(|_| ErreurExercice::DateHorsPlage)?;
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
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Exercices {
    exercices: Vec<Exercice>,
}

impl Exercices {
    /// Aucun exercice.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
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

    /// Ouvre le premier exercice du dossier, à la date de début d'activité (AUDCIF art. 7).
    ///
    /// # Errors
    /// Premier exercice déjà ouvert, ou [`FinPremierExercice::AnneeSuivante`] demandée alors
    /// que l'activité débute au premier semestre.
    pub fn ouvrir_premier(
        &mut self,
        debut_activite: Date,
        fin: FinPremierExercice,
    ) -> Result<&Exercice, ErreurExercice> {
        if !self.exercices.is_empty() {
            return Err(ErreurExercice::PremierExerciceExistant);
        }
        let annee = debut_activite.year();
        let annee_de_cloture = match fin {
            FinPremierExercice::AnneeDeDebut => annee,
            FinPremierExercice::AnneeSuivante => {
                // Second semestre : à partir du 1er juillet.
                if debut_activite.month() <= Month::June {
                    return Err(ErreurExercice::ProlongationImpossible(debut_activite));
                }
                annee.checked_add(1).ok_or(ErreurExercice::DateHorsPlage)?
            }
        };
        let exercice = Exercice::nouveau(debut_activite, annee_de_cloture)?;
        Ok(self.pousser(exercice))
    }

    /// Ouvre l'exercice suivant : l'année civile qui suit la clôture du dernier.
    ///
    /// # Errors
    /// Aucun exercice, ou déjà deux exercices non clôturés.
    pub fn ouvrir_suivant(&mut self) -> Result<&Exercice, ErreurExercice> {
        let dernier = self.exercices.last().ok_or(ErreurExercice::AucunExercice)?;
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
        let exercice = Exercice::nouveau(debut, debut.year())?;
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

/// Découpe `[debut, fin]` en mois civils, le premier éventuellement partiel.
fn decouper_en_mois(debut: Date, fin: Date) -> Result<Vec<Periode>, ErreurExercice> {
    let mut periodes = Vec::new();
    let mut courant = debut;
    while courant <= fin {
        let debut_mois_suivant = premier_du_mois_suivant(courant)?;
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

/// Premier jour du mois qui suit celui de la date.
fn premier_du_mois_suivant(date: Date) -> Result<Date, ErreurExercice> {
    let (annee, mois) = match date.month() {
        Month::December => (
            date.year()
                .checked_add(1)
                .ok_or(ErreurExercice::DateHorsPlage)?,
            Month::January,
        ),
        mois => (date.year(), mois.next()),
    };
    Date::from_calendar_date(annee, mois, 1).map_err(|_| ErreurExercice::DateHorsPlage)
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

    fn dossier(debut: Date) -> Exercices {
        let mut exercices = Exercices::new();
        exercices
            .ouvrir_premier(debut, FinPremierExercice::AnneeDeDebut)
            .unwrap();
        exercices
    }

    #[test]
    fn codes() {
        assert!(CodeExercice::parse("2026").is_ok());
        for texte in ["26", "20266", "2O26", ""] {
            assert!(CodeExercice::parse(texte).is_err(), "{texte}");
        }
    }

    #[test]
    fn premier_exercice_court_au_premier_semestre() {
        let mut exercices = Exercices::new();
        let e = exercices
            .ouvrir_premier(date!(2026 - 03 - 15), FinPremierExercice::AnneeDeDebut)
            .unwrap();
        assert_eq!(e.code(), &code("2026"));
        assert_eq!(e.fin(), date!(2026 - 12 - 31));
        assert_eq!(e.periodes().len(), 10);
        assert_eq!(e.periodes()[0].debut(), date!(2026 - 03 - 15));
        assert_eq!(e.periodes()[0].fin(), date!(2026 - 03 - 31));
        assert_eq!(e.periodes()[1].debut(), date!(2026 - 04 - 01));
        assert_eq!(
            exercices.ouvrir_premier(date!(2027 - 01 - 01), FinPremierExercice::AnneeDeDebut),
            Err(ErreurExercice::PremierExerciceExistant)
        );

        let mut autre = Exercices::new();
        for debut in [date!(2026 - 01 - 01), date!(2026 - 06 - 30)] {
            assert_eq!(
                autre.ouvrir_premier(debut, FinPremierExercice::AnneeSuivante),
                Err(ErreurExercice::ProlongationImpossible(debut))
            );
        }
        assert_eq!(autre.exercices(), &[] as &[Exercice]);
    }

    #[test]
    fn premier_exercice_au_second_semestre() {
        // Au choix : clôture la même année…
        let court = dossier(date!(2026 - 07 - 01));
        assert_eq!(court.exercices()[0].code(), &code("2026"));
        assert_eq!(court.exercices()[0].periodes().len(), 6);
        // … ou le 31 décembre de l'année suivante ; le code est l'année de clôture.
        let mut long = Exercices::new();
        let e = long
            .ouvrir_premier(date!(2026 - 07 - 01), FinPremierExercice::AnneeSuivante)
            .unwrap();
        assert_eq!(e.code(), &code("2027"));
        assert_eq!(e.fin(), date!(2027 - 12 - 31));
        assert_eq!(e.periodes().len(), 18);
    }

    #[test]
    fn exercices_suivants_sur_l_annee_civile() {
        let mut exercices = dossier(date!(2026 - 09 - 10));
        let e = exercices.ouvrir_suivant().unwrap();
        assert_eq!(e.code(), &code("2027"));
        assert_eq!(e.debut(), date!(2027 - 01 - 01));
        assert_eq!(e.fin(), date!(2027 - 12 - 31));
        assert_eq!(e.periodes().len(), 12);
        assert_eq!(e.periodes()[1].fin(), date!(2027 - 02 - 28));
        assert_eq!(
            exercices.ouvrir_suivant(),
            Err(ErreurExercice::TropDExercicesOuverts)
        );
        assert_eq!(
            Exercices::new().ouvrir_suivant(),
            Err(ErreurExercice::AucunExercice)
        );
    }

    #[test]
    fn cloture_dans_l_ordre() {
        let mut exercices = dossier(date!(2026 - 01 - 01));
        exercices.ouvrir_suivant().unwrap();
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
        assert_eq!(exercices.ouvrir_suivant().unwrap().code(), &code("2028"));
        assert_eq!(
            exercices.cloturer(&code("1999")),
            Err(ErreurExercice::ExerciceInconnu(code("1999")))
        );
    }

    #[test]
    fn pas_de_retour_de_la_cloture_vers_ouvert() {
        let mut exercices = dossier(date!(2026 - 01 - 01));
        exercices.passer_en_cloture(&code("2026")).unwrap();
        assert!(exercices.passer_en_cloture(&code("2026")).is_err());
        assert_eq!(
            exercices.exercice(&code("2026")).unwrap().statut(),
            StatutExercice::EnCloture
        );
    }

    #[test]
    fn verrouillage_et_deverrouillage() {
        let mut exercices = dossier(date!(2026 - 01 - 01));
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
        Suivant,
        EnCloture(usize),
        Cloturer(usize),
        Verrouiller(i64),
        Deverrouiller(i64),
    }

    fn operation() -> impl Strategy<Value = Operation> {
        prop_oneof![
            Just(Operation::Suivant),
            (0usize..6).prop_map(Operation::EnCloture),
            (0usize..6).prop_map(Operation::Cloturer),
            (0i64..2500).prop_map(Operation::Verrouiller),
            (0i64..2500).prop_map(Operation::Deverrouiller),
        ]
    }

    fn invariants(exercices: &Exercices) -> Result<(), TestCaseError> {
        let liste = exercices.exercices();
        let non_clotures = liste
            .iter()
            .filter(|e| e.statut() != StatutExercice::Cloture)
            .count();
        prop_assert!(non_clotures <= EXERCICES_NON_CLOTURES_MAX);
        for (rang, e) in liste.iter().enumerate() {
            // Année civile ; le code est l'année de clôture.
            prop_assert_eq!(e.fin().month(), Month::December);
            prop_assert_eq!(e.fin().day(), 31);
            prop_assert_eq!(e.code().as_str(), e.fin().year().to_string());
            if rang > 0 {
                prop_assert_eq!(
                    e.debut(),
                    date!(2000 - 01 - 01).replace_year(e.fin().year()).unwrap()
                );
            }
        }
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
        /// Pour toute date de début d'activité, le premier exercice respecte l'AUDCIF art. 7.
        #[test]
        fn premier_exercice(jour in 0i64..3650, prolonger in any::<bool>()) {
            let debut = date!(2020 - 01 - 01) + Duration::days(jour);
            let fin = if prolonger { FinPremierExercice::AnneeSuivante } else { FinPremierExercice::AnneeDeDebut };
            let mut exercices = Exercices::new();
            match exercices.ouvrir_premier(debut, fin) {
                Ok(e) => {
                    let annee = debut.year() + i32::from(prolonger);
                    prop_assert!(!prolonger || debut.month() >= Month::July);
                    prop_assert_eq!(e.debut(), debut);
                    prop_assert_eq!(e.fin(), date!(2000 - 12 - 31).replace_year(annee).unwrap());
                    prop_assert_eq!(e.code().as_str(), annee.to_string());
                }
                Err(erreur) => {
                    prop_assert!(prolonger && debut.month() <= Month::June);
                    prop_assert_eq!(erreur, ErreurExercice::ProlongationImpossible(debut));
                }
            }
        }

        /// Aucune suite d'opérations ne viole les invariants ; une opération refusée laisse
        /// l'état inchangé ; un exercice clôturé ne change plus jamais.
        #[test]
        fn transitions(ops in proptest::collection::vec(operation(), 0..40)) {
            let mut exercices = Exercices::new();
            exercices.ouvrir_premier(date!(2026 - 07 - 14), FinPremierExercice::AnneeSuivante).unwrap();
            for op in ops {
                let avant = exercices.clone();
                let codes: Vec<CodeExercice> = exercices.exercices().iter().map(|e| e.code().clone()).collect();
                let jour = |d: i64| date!(2026 - 07 - 14) + Duration::days(d);
                let resultat = match op {
                    Operation::Suivant => exercices.ouvrir_suivant().map(|_| ()),
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
