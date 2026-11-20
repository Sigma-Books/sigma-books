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
//! - un **dossier repris** (entité déjà en activité à son entrée dans Sigma Books) ouvre
//!   l'**exercice civil en cours**, du 1er janvier au 31 décembre, sans durée dérogatoire :
//!   l'option « second semestre » ne vaut que pour le premier exercice de l'entité, celui qui
//!   commence à sa création (art. 7, p. PDF 18) ([`Exercices::ouvrir_repris`]).
//!
//! La cessation d'activité (exercice de liquidation) relève du lot L6.
//!
//! Les exercices relus de la persistance sont reconstitués par [`Exercices::reconstituer`],
//! à partir des lignes plates produites par [`Exercices::aplatir`].
//!
//! Statuts :
//! - exercice : `ouvert` → `en_cloture` → `cloture` (irréversible), sans retour. Au plus
//!   **deux** exercices non clôturés à la fois (N en clôture, N+1 courant) ; ils passent en
//!   clôture et se clôturent dans l'ordre chronologique ;
//! - période (mois civil, le premier éventuellement partiel) : `ouverte` ⇄ `verrouillee` ;
//!   le déverrouillage exige un **motif**, renvoyé à l'appelant pour la journalisation (L2) ;
//!   `cloturee` est la **clôture informatique** de l'AUDCIF (art. 22 3°, p. PDF 24) :
//!   irréversible, dans l'ordre des périodes, au plus tard à son **échéance**
//!   ([`echeance_de_cloture`]). La dernière période d'un exercice se clôture avec lui.
//!   Aucune période ni aucun exercice ne se clôture tant qu'une écriture du brouillard y est
//!   datée (décision du mainteneur du 2026-11-18 ; [`Exercices::cloturer_periode`]).
//!
//! Aucune lecture d'horloge : toutes les dates sont des paramètres. Une opération refusée
//! laisse l'état inchangé.

use std::fmt;

use time::{Date, Month};

use crate::texte::TexteCourt;

mod reconstitution;

pub use reconstitution::{ErreurReconstitutionExercices, LigneExercice, LignePeriode};

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
    /// Une période antérieure doit être clôturée d'abord (art. 22 3° et 4°).
    #[error("la période commençant le {0} doit être clôturée avant")]
    PeriodeAnterieureNonCloturee(Date),
    /// Un exercice plus ancien doit d'abord avancer.
    #[error("l'exercice {0}, plus ancien, doit être clôturé (ou en clôture) avant")]
    ExerciceAnterieurNonClos(CodeExercice),
    /// Des écritures du brouillard sont datées dans ce qui serait clôturé : elles doivent être
    /// validées, redatées ou supprimées avant la clôture. Période : décision du mainteneur du
    /// 2026-11-18 (les textes ne tranchent pas) ; exercice : spec §15.1 (« Aucune écriture
    /// non validée sur l'exercice »).
    #[error(
        "{nombre} écriture(s) du brouillard datée(s) du {du} au {au} : validez-les ou \
         supprimez-les avant la clôture"
    )]
    BrouillardNonValide {
        /// Premier jour de ce qui serait clôturé.
        du: Date,
        /// Dernier jour de ce qui serait clôturé.
        au: Date,
        /// Nombre d'écritures du brouillard concernées.
        nombre: usize,
    },
}

/// Nombre de dates du brouillard comprises dans `[du, au]`.
fn brouillard_entre(brouillard: impl IntoIterator<Item = Date>, du: Date, au: Date) -> usize {
    brouillard
        .into_iter()
        .filter(|d| du <= *d && *d <= au)
        .count()
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
    /// Clôture informatique (AUDCIF art. 22 3°) ; irréversible.
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

/// Échéance de la clôture informatique d'une période (AUDCIF art. 22 3°, décision du mainteneur
/// du 2026-11-18) : le dernier jour du **troisième mois** qui suit la fin de la période. Pour
/// la dernière période d'un exercice, qui se clôture avec lui : le dernier jour du
/// **quatrième mois** qui suit la fin de l'exercice, délai d'arrêté des comptes (art. 23).
///
/// # Errors
/// [`ErreurExercice::DateHorsPlage`].
pub fn echeance_de_cloture(exercice: &Exercice, periode: &Periode) -> Result<Date, ErreurExercice> {
    let mois = if periode.fin == exercice.fin { 4 } else { 3 };
    let mut debut = periode.fin;
    for _ in 0..=mois {
        debut = premier_du_mois_suivant(debut)?;
    }
    debut.previous_day().ok_or(ErreurExercice::DateHorsPlage)
}

/// Période non clôturée et son échéance de clôture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EcheanceCloture {
    /// Exercice.
    pub exercice: CodeExercice,
    /// Premier jour de la période.
    pub debut: Date,
    /// Dernier jour de la période.
    pub fin: Date,
    /// Dernier jour pour la clôturer.
    pub echeance: Date,
    /// Statut actuel (ouverte ou verrouillée).
    pub statut: StatutPeriode,
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

    /// Ouvre l'exercice d'un **dossier repris** : entité déjà en activité à son entrée dans
    /// Sigma Books. L'exercice ouvert est l'**exercice civil en cours** (`annee`, fournie par
    /// l'appelant), du 1er janvier au 31 décembre, sans durée dérogatoire : l'option « second
    /// semestre » de l'AUDCIF art. 7 (p. PDF 18) ne vaut que pour le premier exercice de
    /// l'entité, celui qui commence à sa création ([`Exercices::ouvrir_premier`]).
    ///
    /// **À-nouveaux** : au 1er janvier, le bilan d'ouverture est égal au bilan de clôture de
    /// l'exercice précédent (AUDCIF art. 34, p. PDF 27). Leur saisie, depuis la balance de
    /// clôture N-1 du logiciel d'origine (spec §18.1), relève d'un lot ultérieur (L8) ; cette
    /// fonction n'ouvre que l'exercice et ses périodes.
    ///
    /// # Errors
    /// Premier exercice déjà ouvert, ou année hors de la plage représentable.
    pub fn ouvrir_repris(&mut self, annee: i32) -> Result<&Exercice, ErreurExercice> {
        if !self.exercices.is_empty() {
            return Err(ErreurExercice::PremierExerciceExistant);
        }
        let debut = Date::from_calendar_date(annee, Month::January, 1)
            .map_err(|_| ErreurExercice::DateHorsPlage)?;
        let exercice = Exercice::nouveau(debut, annee)?;
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
    /// `brouillard` : date comptable de chaque écriture du brouillard du dossier, **une par
    /// écriture** (l'appelant peut ne fournir que celles de l'exercice). Aucune ne doit tomber
    /// dans l'exercice : spec §15.1 (« Aucune écriture non validée sur l'exercice ») ; c'est
    /// aussi la règle des périodes, que cette clôture clôture toutes. Même obligation de
    /// verrouillage que pour [`Exercices::cloturer_periode`], sur toutes les périodes de
    /// l'exercice.
    ///
    /// # Errors
    /// Exercice inconnu, pas en clôture, exercice antérieur non clôturé, ou brouillard non
    /// validé dans l'exercice ([`ErreurExercice::BrouillardNonValide`]).
    pub fn cloturer(
        &mut self,
        code: &CodeExercice,
        brouillard: impl IntoIterator<Item = Date>,
    ) -> Result<(), ErreurExercice> {
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
        let (du, au) = (self.exercices[index].debut, self.exercices[index].fin);
        let nombre = brouillard_entre(brouillard, du, au);
        if nombre > 0 {
            return Err(ErreurExercice::BrouillardNonValide { du, au, nombre });
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

    /// Clôture informatique de la période contenant la date (AUDCIF art. 22 3°) :
    /// irréversible, aucune réouverture. Les périodes se clôturent dans l'ordre ; la dernière
    /// période d'un exercice se clôture avec lui ([`Exercices::cloturer`]).
    ///
    /// **Brouillard** : la clôture est refusée tant qu'une écriture du brouillard est datée
    /// dans la période (décision du mainteneur du 2026-11-18). Les textes ne le disent pas en
    /// toutes lettres : l'art. 22 2° exige la validation « au terme de chaque période qui ne
    /// peut excéder un mois », le 3° fige la chronologie, sans faire de la validation un
    /// préalable de la clôture. Tous les moteurs de référence bloquent : Tryton
    /// (`Period.close`), Odoo (verrou définitif, `_validate_locks`), Dolibarr (clôture proposée
    /// seulement sans ligne à valider), OCA (`account_fiscal_year_closing`, en option).
    /// `brouillard` : date comptable de chaque écriture du brouillard du dossier, **une par
    /// écriture** (le nombre renvoyé en dépend ; l'appelant peut ne fournir que celles de la
    /// période).
    ///
    /// **Concurrence** (à la charge de l'appelant, lot L2) : en `READ COMMITTED`, un brouillon
    /// créé ou redaté dans la période entre la lecture des dates et la fin de la clôture
    /// échapperait au contrôle. La clôture prend donc la ligne de la période `FOR UPDATE` avant
    /// de lire les dates ; toute création ou tout changement de date d'un brouillon prend la
    /// ligne de la période visée `FOR SHARE` (en tête de l'ordre fixe des verrous, ADR 0011) et
    /// refuse une période clôturée.
    ///
    /// # Errors
    /// Aucune période ne contient la date, période déjà clôturée, dernière période de
    /// l'exercice, période antérieure non clôturée, ou brouillard non validé dans la période
    /// ([`ErreurExercice::BrouillardNonValide`]).
    pub fn cloturer_periode(
        &mut self,
        date: Date,
        brouillard: impl IntoIterator<Item = Date>,
    ) -> Result<(), ErreurExercice> {
        let anterieure = self
            .exercices
            .iter()
            .flat_map(|e| e.periodes.iter())
            .find(|p| p.fin < date && p.statut != StatutPeriode::Cloturee)
            .map(|p| p.debut);
        let exercice = self
            .exercices
            .iter_mut()
            .find(|e| e.contient(date))
            .ok_or(ErreurExercice::PeriodeInconnue(date))?;
        let fin_exercice = exercice.fin;
        let periode = exercice
            .periodes
            .iter_mut()
            .find(|p| p.contient(date))
            .ok_or(ErreurExercice::PeriodeInconnue(date))?;
        if periode.statut == StatutPeriode::Cloturee {
            return Err(ErreurExercice::TransitionInterdite(
                "la période est déjà clôturée",
            ));
        }
        if periode.fin == fin_exercice {
            return Err(ErreurExercice::TransitionInterdite(
                "la dernière période se clôture avec l'exercice",
            ));
        }
        if let Some(debut) = anterieure {
            return Err(ErreurExercice::PeriodeAnterieureNonCloturee(debut));
        }
        let nombre = brouillard_entre(brouillard, periode.debut, periode.fin);
        if nombre > 0 {
            return Err(ErreurExercice::BrouillardNonValide {
                du: periode.debut,
                au: periode.fin,
                nombre,
            });
        }
        periode.statut = StatutPeriode::Cloturee;
        Ok(())
    }

    /// Périodes non clôturées et leurs échéances, dans l'ordre chronologique : de quoi
    /// alerter avant l'échéance (L2).
    ///
    /// # Errors
    /// [`ErreurExercice::DateHorsPlage`].
    pub fn echeances_de_cloture(&self) -> Result<Vec<EcheanceCloture>, ErreurExercice> {
        let mut echeances = Vec::new();
        for exercice in &self.exercices {
            for periode in &exercice.periodes {
                if periode.statut != StatutPeriode::Cloturee {
                    echeances.push(EcheanceCloture {
                        exercice: exercice.code.clone(),
                        debut: periode.debut,
                        fin: periode.fin,
                        echeance: echeance_de_cloture(exercice, periode)?,
                        statut: periode.statut,
                    });
                }
            }
        }
        Ok(echeances)
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
    fn cloture_informatique_dans_l_ordre_et_irreversible() {
        let mut exercices = dossier(date!(2026 - 01 - 01));
        assert_eq!(
            exercices.cloturer_periode(date!(2026 - 02 - 10), []),
            Err(ErreurExercice::PeriodeAnterieureNonCloturee(date!(
                2026 - 01 - 01
            )))
        );
        exercices
            .verrouiller_periode(date!(2026 - 01 - 10))
            .unwrap();
        exercices
            .cloturer_periode(date!(2026 - 01 - 10), [])
            .unwrap();
        let janvier = |e: &Exercices| e.exercices()[0].periodes()[0].statut();
        assert_eq!(janvier(&exercices), StatutPeriode::Cloturee);
        // Aucune réouverture, aucun verrouillage, pas de double clôture.
        assert!(
            exercices
                .deverrouiller_periode(date!(2026 - 01 - 10), motif())
                .is_err()
        );
        assert!(
            exercices
                .verrouiller_periode(date!(2026 - 01 - 10))
                .is_err()
        );
        assert!(
            exercices
                .cloturer_periode(date!(2026 - 01 - 10), [])
                .is_err()
        );
        assert_eq!(janvier(&exercices), StatutPeriode::Cloturee);
        // Février à novembre ; décembre se clôture avec l'exercice.
        for mois in 2..=11u8 {
            let jour = Date::from_calendar_date(2026, Month::try_from(mois).unwrap(), 1).unwrap();
            exercices.cloturer_periode(jour, []).unwrap();
        }
        assert!(matches!(
            exercices.cloturer_periode(date!(2026 - 12 - 01), []),
            Err(ErreurExercice::TransitionInterdite(_))
        ));
        assert!(
            exercices
                .cloturer_periode(date!(2030 - 01 - 01), [])
                .is_err()
        );
    }

    #[test]
    fn echeances_de_cloture() {
        let mut exercices = Exercices::new();
        exercices
            .ouvrir_premier(date!(2026 - 08 - 17), FinPremierExercice::AnneeSuivante)
            .unwrap();
        let echeances = exercices.echeances_de_cloture().unwrap();
        // Août 2026 (partiel) : fin novembre ; novembre : fin février (année non bissextile) ;
        // décembre 2027, dernière période : fin avril 2028 (art. 23).
        assert_eq!(echeances.len(), 17);
        assert_eq!(echeances[0].debut, date!(2026 - 08 - 17));
        assert_eq!(echeances[0].echeance, date!(2026 - 11 - 30));
        assert_eq!(echeances[3].echeance, date!(2027 - 02 - 28));
        assert_eq!(echeances[16].fin, date!(2027 - 12 - 31));
        assert_eq!(echeances[16].echeance, date!(2028 - 04 - 30));
        assert!(echeances.iter().all(|e| e.statut == StatutPeriode::Ouverte));
        exercices
            .cloturer_periode(date!(2026 - 08 - 20), [])
            .unwrap();
        assert_eq!(exercices.echeances_de_cloture().unwrap().len(), 16);
    }

    #[test]
    fn cloture_de_periode_refusee_tant_qu_il_reste_du_brouillard() {
        let mut exercices = dossier(date!(2026 - 01 - 01));
        let avant = exercices.clone();
        // Deux brouillons de janvier (premier et dernier jour), un de mars : refus, état
        // inchangé.
        let brouillard = [
            date!(2026 - 03 - 05),
            date!(2026 - 01 - 31),
            date!(2026 - 01 - 01),
        ];
        assert_eq!(
            exercices.cloturer_periode(date!(2026 - 01 - 10), brouillard),
            Err(ErreurExercice::BrouillardNonValide {
                du: date!(2026 - 01 - 01),
                au: date!(2026 - 01 - 31),
                nombre: 2,
            })
        );
        assert_eq!(exercices, avant);
        // Les règles d'ordre passent avant le brouillard : février, qui a un brouillon, ne se
        // clôture pas avant janvier ; décembre, qui en a un aussi, se clôture avec l'exercice.
        let avec_fevrier_et_decembre = [date!(2026 - 02 - 14), date!(2026 - 12 - 31)];
        assert_eq!(
            exercices.cloturer_periode(date!(2026 - 02 - 10), avec_fevrier_et_decembre),
            Err(ErreurExercice::PeriodeAnterieureNonCloturee(date!(
                2026 - 01 - 01
            )))
        );
        assert!(matches!(
            exercices.cloturer_periode(date!(2026 - 12 - 10), avec_fevrier_et_decembre),
            Err(ErreurExercice::TransitionInterdite(_))
        ));
        assert_eq!(exercices, avant);
        // Brouillard hors de la période (veille et lendemain) : clôture permise.
        exercices
            .cloturer_periode(
                date!(2026 - 01 - 10),
                [date!(2025 - 12 - 31), date!(2026 - 02 - 01)],
            )
            .unwrap();
        assert_eq!(
            exercices
                .exercice_contenant(date!(2026 - 01 - 10))
                .and_then(|e| e.periode_contenant(date!(2026 - 01 - 10)))
                .map(Periode::statut),
            Some(StatutPeriode::Cloturee)
        );
    }

    #[test]
    fn cloture_d_exercice_refusee_tant_qu_il_reste_du_brouillard() {
        let mut exercices = dossier(date!(2026 - 01 - 01));
        exercices.ouvrir_suivant().unwrap();
        exercices.passer_en_cloture(&code("2026")).unwrap();
        let avant = exercices.clone();
        // Un brouillon dans une période non encore clôturée de 2026 (juin) et un dans la
        // dernière (décembre), qui se clôture avec l'exercice ; un en 2027, hors exercice.
        let brouillard = [
            date!(2026 - 06 - 15),
            date!(2026 - 12 - 31),
            date!(2027 - 01 - 01),
        ];
        assert_eq!(
            exercices.cloturer(&code("2026"), brouillard),
            Err(ErreurExercice::BrouillardNonValide {
                du: date!(2026 - 01 - 01),
                au: date!(2026 - 12 - 31),
                nombre: 2,
            })
        );
        assert_eq!(exercices, avant);
        // L'ordre des exercices passe avant le brouillard : 2027 (avec un brouillon) ne se
        // clôture pas avant 2026.
        exercices.passer_en_cloture(&code("2027")).unwrap();
        assert_eq!(
            exercices.cloturer(&code("2027"), brouillard),
            Err(ErreurExercice::ExerciceAnterieurNonClos(code("2026")))
        );
        exercices
            .cloturer(&code("2026"), [date!(2027 - 01 - 01)])
            .unwrap();
        assert_eq!(
            exercices.exercice(&code("2026")).unwrap().statut(),
            StatutExercice::Cloture
        );
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
    fn dossier_repris_sur_l_exercice_civil_en_cours() {
        // AUDCIF art. 7 et 34 : une entité déjà en activité ouvre l'exercice civil en cours,
        // sans durée dérogatoire ; ses à-nouveaux (lot ultérieur) reprennent le bilan de
        // clôture N-1.
        let mut exercices = Exercices::new();
        let e = exercices.ouvrir_repris(2026).unwrap();
        assert_eq!(e.code(), &code("2026"));
        assert_eq!(e.debut(), date!(2026 - 01 - 01));
        assert_eq!(e.fin(), date!(2026 - 12 - 31));
        assert_eq!(e.statut(), StatutExercice::Ouvert);
        assert_eq!(e.periodes().len(), 12);
        assert_eq!(e.periodes()[0].fin(), date!(2026 - 01 - 31));
        assert_eq!(
            exercices.ouvrir_repris(2027),
            Err(ErreurExercice::PremierExerciceExistant)
        );
        assert_eq!(
            exercices.ouvrir_premier(date!(2027 - 08 - 01), FinPremierExercice::AnneeSuivante),
            Err(ErreurExercice::PremierExerciceExistant)
        );
        // La suite est celle de tout dossier : l'année civile suivante.
        assert_eq!(exercices.ouvrir_suivant().unwrap().code(), &code("2027"));
        // Années hors de la plage des codes à quatre chiffres.
        for annee in [999, 10_000] {
            assert_eq!(
                Exercices::new().ouvrir_repris(annee),
                Err(ErreurExercice::DateHorsPlage)
            );
        }
        let mut premier = Exercices::new();
        premier
            .ouvrir_premier(date!(2026 - 03 - 01), FinPremierExercice::AnneeDeDebut)
            .unwrap();
        assert_eq!(
            premier.ouvrir_repris(2026),
            Err(ErreurExercice::PremierExerciceExistant)
        );
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
            exercices.cloturer(&code("2026"), []).is_err(),
            "pas encore en clôture"
        );
        exercices.passer_en_cloture(&code("2026")).unwrap();
        exercices.passer_en_cloture(&code("2027")).unwrap();
        assert_eq!(
            exercices.cloturer(&code("2027"), []),
            Err(ErreurExercice::ExerciceAnterieurNonClos(code("2026")))
        );
        exercices.cloturer(&code("2026"), []).unwrap();
        let e = exercices.exercice(&code("2026")).unwrap();
        assert!(
            e.periodes()
                .iter()
                .all(|p| p.statut() == StatutPeriode::Cloturee)
        );
        assert!(exercices.passer_en_cloture(&code("2026")).is_err());
        assert!(exercices.cloturer(&code("2026"), []).is_err());
        assert!(
            exercices
                .verrouiller_periode(date!(2026 - 05 - 10))
                .is_err()
        );
        assert_eq!(exercices.ouvrir_suivant().unwrap().code(), &code("2028"));
        assert_eq!(
            exercices.cloturer(&code("1999"), []),
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

    /// Début d'activité du dossier de la propriété `transitions` : second semestre, premier
    /// exercice prolongé jusqu'au 31 décembre de l'année suivante (18 périodes).
    const DEBUT_ACTIVITE: Date = date!(2026 - 07 - 14);

    #[derive(Debug, Clone)]
    enum Operation {
        Suivant,
        EnCloture(usize),
        Cloturer(usize),
        Verrouiller(i64),
        Deverrouiller(i64),
        CloturerPeriode(CiblePeriode),
    }

    /// Cible d'une clôture informatique de période (AUDCIF art. 22 3°).
    #[derive(Debug, Clone)]
    enum CiblePeriode {
        /// Période de rang `rang` compté depuis la première période non clôturée de tous les
        /// exercices (0 : la suivante dans l'ordre chronologique ; négatif : une période déjà
        /// clôturée ; positif : une période qui n'est pas la suivante), un jour quelconque de
        /// la période (`jour` modulo sa durée). Hors de la liste : une date hors de tout exercice.
        Relative(i64, u8),
        /// Jour quelconque compté depuis le début du premier exercice tiré (souvent hors de
        /// l'ordre, parfois hors de tout exercice).
        Jour(i64),
    }

    fn cible_periode() -> impl Strategy<Value = CiblePeriode> {
        let rang = prop_oneof![6 => Just(0i64), 1 => -3i64..0, 1 => 1i64..4];
        prop_oneof![
            4 => (rang, any::<u8>()).prop_map(|(rang, jour)| CiblePeriode::Relative(rang, jour)),
            1 => (-400i64..2500).prop_map(CiblePeriode::Jour),
        ]
    }

    fn operation() -> impl Strategy<Value = Operation> {
        prop_oneof![
            1 => Just(Operation::Suivant),
            1 => (0usize..6).prop_map(Operation::EnCloture),
            1 => (0usize..6).prop_map(Operation::Cloturer),
            1 => (0i64..2500).prop_map(Operation::Verrouiller),
            1 => (0i64..2500).prop_map(Operation::Deverrouiller),
            4 => cible_periode().prop_map(Operation::CloturerPeriode),
        ]
    }

    /// Toutes les périodes de tous les exercices, dans l'ordre chronologique.
    fn toutes_les_periodes(exercices: &Exercices) -> Vec<Periode> {
        exercices
            .exercices()
            .iter()
            .flat_map(|e| e.periodes().iter().copied())
            .collect()
    }

    /// Date visée par une clôture de période, calculée sur l'état avant l'opération ; `debut`
    /// est le début du premier exercice tiré.
    fn date_cible(exercices: &Exercices, debut: Date, cible: &CiblePeriode) -> Date {
        match *cible {
            CiblePeriode::Jour(d) => debut + Duration::days(d),
            CiblePeriode::Relative(rang, jour) => {
                let periodes = toutes_les_periodes(exercices);
                let premiere = periodes
                    .iter()
                    .position(|p| p.statut() != StatutPeriode::Cloturee)
                    .unwrap_or(periodes.len());
                let index = i64::try_from(premiere).unwrap() + rang;
                let decalage = Duration::days(1 + i64::from(jour));
                match usize::try_from(index).ok().and_then(|i| periodes.get(i)) {
                    Some(p) => {
                        let duree = (p.fin() - p.debut()).whole_days() + 1;
                        p.debut() + Duration::days(i64::from(jour) % duree)
                    }
                    None if index < 0 => periodes[0].debut() - decalage,
                    None => periodes[periodes.len() - 1].fin() + decalage,
                }
            }
        }
    }

    /// Oracle de la clôture informatique (AUDCIF art. 22 3°) : elle est acceptée si et
    /// seulement si la date tombe dans la première période non clôturée de tous les
    /// exercices (ordre chronologique) et que cette période n'est pas la dernière de son
    /// exercice (qui se clôture avec lui).
    fn cloture_periode_acceptable(exercices: &Exercices, date: Date) -> bool {
        exercices
            .exercices()
            .iter()
            .flat_map(|e| e.periodes().iter().map(move |p| (e.fin(), p)))
            .find(|(_, p)| p.statut() != StatutPeriode::Cloturee)
            .is_some_and(|(fin_exercice, p)| p.contient(date) && p.fin() != fin_exercice)
    }

    fn rang_statut(statut: StatutExercice) -> u8 {
        match statut {
            StatutExercice::Ouvert => 0,
            StatutExercice::EnCloture => 1,
            StatutExercice::Cloture => 2,
        }
    }

    /// Invariants d'un état.
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
            if e.statut() == StatutExercice::Cloture {
                // La clôture de l'exercice clôture toutes ses périodes.
                prop_assert!(p.iter().all(|x| x.statut() == StatutPeriode::Cloturee));
            } else {
                // La dernière période se clôture avec l'exercice, jamais avant (art. 22 3°).
                prop_assert_ne!(p.last().map(Periode::statut), Some(StatutPeriode::Cloturee));
            }
        }
        // Clôture informatique dans l'ordre (AUDCIF art. 22 3°) : les périodes clôturées
        // forment un préfixe chronologique de toutes les périodes de tous les exercices.
        let periodes = toutes_les_periodes(exercices);
        let prefixe = periodes
            .iter()
            .take_while(|p| p.statut() == StatutPeriode::Cloturee)
            .count();
        prop_assert!(
            periodes[prefixe..]
                .iter()
                .all(|p| p.statut() != StatutPeriode::Cloturee),
            "période clôturée après une période non clôturée"
        );
        Ok(())
    }

    /// Invariants d'une transition, que l'opération ait réussi ou échoué.
    fn invariants_de_transition(avant: &Exercices, apres: &Exercices) -> Result<(), TestCaseError> {
        // Les exercices s'ajoutent à la fin ; leurs bornes et leurs périodes ne bougent pas.
        prop_assert!(avant.exercices().len() <= apres.exercices().len());
        for (a, b) in avant.exercices().iter().zip(apres.exercices()) {
            prop_assert_eq!(a.code(), b.code());
            prop_assert_eq!((a.debut(), a.fin()), (b.debut(), b.fin()));
            prop_assert_eq!(a.periodes().len(), b.periodes().len());
            // Statut de l'exercice : ouvert → en clôture → clôturé, sans retour.
            prop_assert!(rang_statut(a.statut()) <= rang_statut(b.statut()));
            // Un exercice clôturé ne change plus jamais.
            if a.statut() == StatutExercice::Cloture {
                prop_assert_eq!(a, b);
            }
            // Clôture informatique irréversible (AUDCIF art. 22 3°).
            for (pa, pb) in a.periodes().iter().zip(b.periodes()) {
                prop_assert_eq!((pa.debut(), pa.fin()), (pb.debut(), pb.fin()));
                if pa.statut() == StatutPeriode::Cloturee {
                    prop_assert_eq!(pb.statut(), StatutPeriode::Cloturee);
                }
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
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(1024))]

        /// Aucune suite d'opérations ne viole les invariants, vérifiés après chaque opération
        /// réussie ou refusée ; une opération refusée laisse l'état strictement inchangé ; la
        /// clôture informatique (AUDCIF art. 22 3°) est acceptée exactement quand l'oracle
        /// l'attend, ne clôture que la période visée et reste irréversible.
        #[test]
        fn transitions(ops in proptest::collection::vec(operation(), 0..80)) {
            let mut exercices = Exercices::new();
            exercices.ouvrir_premier(DEBUT_ACTIVITE, FinPremierExercice::AnneeSuivante).unwrap();
            invariants(&exercices)?;
            for op in ops {
                let avant = exercices.clone();
                let codes: Vec<CodeExercice> = exercices.exercices().iter().map(|e| e.code().clone()).collect();
                let jour = |d: i64| DEBUT_ACTIVITE + Duration::days(d);
                let resultat = match &op {
                    Operation::Suivant => exercices.ouvrir_suivant().map(|_| ()),
                    Operation::EnCloture(i) => exercices.passer_en_cloture(&codes[i % codes.len()]),
                    Operation::Cloturer(i) => exercices.cloturer(&codes[i % codes.len()], []),
                    Operation::Verrouiller(d) => exercices.verrouiller_periode(jour(*d)),
                    Operation::Deverrouiller(d) => exercices.deverrouiller_periode(jour(*d), motif()).map(|_| ()),
                    Operation::CloturerPeriode(cible) => {
                        let date = date_cible(&avant, DEBUT_ACTIVITE, cible);
                        let resultat = exercices.cloturer_periode(date, []);
                        prop_assert_eq!(
                            resultat.is_ok(),
                            cloture_periode_acceptable(&avant, date),
                            "clôture de la période du {} : {:?}", date, resultat
                        );
                        if resultat.is_ok() {
                            // Seule la période visée change, et elle devient clôturée.
                            let attendu: Vec<StatutPeriode> = toutes_les_periodes(&avant)
                                .iter()
                                .map(|p| if p.contient(date) { StatutPeriode::Cloturee } else { p.statut() })
                                .collect();
                            let obtenu: Vec<StatutPeriode> =
                                toutes_les_periodes(&exercices).iter().map(Periode::statut).collect();
                            prop_assert_eq!(attendu, obtenu);
                            let statuts = |e: &Exercices| e.exercices().iter().map(Exercice::statut).collect::<Vec<_>>();
                            prop_assert_eq!(statuts(&avant), statuts(&exercices));
                        }
                        resultat
                    }
                };
                if resultat.is_err() {
                    prop_assert_eq!(&exercices, &avant, "{:?} refusée a modifié l'état", op);
                }
                invariants_de_transition(&avant, &exercices)?;
                invariants(&exercices)?;
                aller_retour(&exercices)?;
            }
        }

        /// Pour tout premier exercice (début d'activité quelconque, prolongé ou non, ou dossier
        /// repris) et toute suite de transitions : `reconstituer(aplatir(x)) == x`, y compris
        /// lignes relues dans l'ordre inverse.
        #[test]
        fn reconstitution_apres_transitions(
            premier in (0i64..1500, 0u8..3),
            ops in proptest::collection::vec(operation(), 0..60),
        ) {
            let (jour, genre) = premier;
            let mut exercices = Exercices::new();
            let Some(debut) = premier_exercice_tire(&mut exercices, jour, genre) else {
                return Err(TestCaseError::reject("premier exercice refusé"));
            };
            aller_retour(&exercices)?;
            for op in ops {
                appliquer(&mut exercices, debut, &op);
                aller_retour(&exercices)?;
            }
        }

        /// Réciproque : partant des lignes plates d'un état atteignable, une altération
        /// quelconque (date, statut ou code changé, ligne supprimée ou doublée) est soit
        /// refusée par `reconstituer`, soit acceptée avec un état qui respecte les invariants
        /// (et se relit lui-même sans perte).
        #[test]
        fn reconstitution_d_un_etat_altere(
            premier in (0i64..1500, 0u8..3),
            ops in proptest::collection::vec(operation(), 0..60),
            alterations in proptest::collection::vec(alteration(), 1..4),
        ) {
            let (jour, genre) = premier;
            let mut exercices = Exercices::new();
            let Some(debut) = premier_exercice_tire(&mut exercices, jour, genre) else {
                return Err(TestCaseError::reject("premier exercice refusé"));
            };
            for op in &ops {
                appliquer(&mut exercices, debut, op);
            }
            let (mut lignes, mut periodes) = exercices.aplatir();
            for alteration in &alterations {
                alterer(&mut lignes, &mut periodes, alteration);
            }
            if let Ok(relu) = Exercices::reconstituer(lignes, periodes) {
                invariants(&relu)?;
                aller_retour(&relu)?;
            }
        }
    }

    /// Ouvre le premier exercice tiré : début d'activité décalé de `jour`, prolongé ou non
    /// (`genre` 0 ou 1), ou dossier repris (`genre` 2). Renvoie le début tiré, ou `None` si
    /// l'ouverture est refusée.
    fn premier_exercice_tire(exercices: &mut Exercices, jour: i64, genre: u8) -> Option<Date> {
        let debut = DEBUT_ACTIVITE + Duration::days(jour);
        let ouvert = match genre {
            0 => exercices
                .ouvrir_premier(debut, FinPremierExercice::AnneeDeDebut)
                .map(|_| ()),
            1 => exercices
                .ouvrir_premier(debut, FinPremierExercice::AnneeSuivante)
                .map(|_| ()),
            _ => exercices.ouvrir_repris(debut.year()).map(|_| ()),
        };
        ouvert.is_ok().then_some(debut)
    }

    /// Applique une opération, qu'elle réussisse ou non ; `debut` est le début tiré.
    fn appliquer(exercices: &mut Exercices, debut: Date, op: &Operation) {
        let codes: Vec<CodeExercice> = exercices
            .exercices()
            .iter()
            .map(|e| e.code().clone())
            .collect();
        let jour = |d: i64| debut + Duration::days(d);
        let _ = match op {
            Operation::Suivant => exercices.ouvrir_suivant().map(|_| ()),
            Operation::EnCloture(i) => exercices.passer_en_cloture(&codes[i % codes.len()]),
            Operation::Cloturer(i) => exercices.cloturer(&codes[i % codes.len()], []),
            Operation::Verrouiller(d) => exercices.verrouiller_periode(jour(*d)),
            Operation::Deverrouiller(d) => exercices
                .deverrouiller_periode(jour(*d), motif())
                .map(|_| ()),
            Operation::CloturerPeriode(cible) => {
                let date = date_cible(exercices, debut, cible);
                exercices.cloturer_periode(date, [])
            }
        };
    }

    /// Altération d'une ligne plate (exercice ou période), désignée par son rang modulo le
    /// nombre de lignes.
    #[derive(Debug, Clone)]
    enum Alteration {
        /// Décale le premier (`true`) ou le dernier jour d'un nombre de jours non nul.
        Date {
            periode: bool,
            rang: usize,
            premier_jour: bool,
            decalage: i64,
        },
        /// Remplace le statut (rang du statut modulo 3).
        Statut {
            periode: bool,
            rang: usize,
            statut: u8,
        },
        /// Décale l'année du code d'un nombre d'années non nul.
        Code {
            periode: bool,
            rang: usize,
            decalage: i32,
        },
        /// Supprime la ligne.
        Supprimer { periode: bool, rang: usize },
        /// Double la ligne.
        Doubler { periode: bool, rang: usize },
    }

    fn alteration() -> impl Strategy<Value = Alteration> {
        let decalage_jours = prop_oneof![-400i64..0, 1i64..400];
        let decalage_annees = prop_oneof![-3i32..0, 1i32..4];
        prop_oneof![
            (any::<bool>(), any::<usize>(), any::<bool>(), decalage_jours).prop_map(
                |(periode, rang, premier_jour, decalage)| Alteration::Date {
                    periode,
                    rang,
                    premier_jour,
                    decalage,
                }
            ),
            (any::<bool>(), any::<usize>(), any::<u8>()).prop_map(|(periode, rang, statut)| {
                Alteration::Statut {
                    periode,
                    rang,
                    statut,
                }
            }),
            (any::<bool>(), any::<usize>(), decalage_annees).prop_map(
                |(periode, rang, decalage)| Alteration::Code {
                    periode,
                    rang,
                    decalage,
                }
            ),
            (any::<bool>(), any::<usize>())
                .prop_map(|(periode, rang)| Alteration::Supprimer { periode, rang }),
            (any::<bool>(), any::<usize>())
                .prop_map(|(periode, rang)| Alteration::Doubler { periode, rang }),
        ]
    }

    /// Applique une altération aux lignes plates (sans effet sur une liste vide).
    fn alterer(
        lignes: &mut Vec<LigneExercice>,
        periodes: &mut Vec<LignePeriode>,
        alteration: &Alteration,
    ) {
        match *alteration {
            Alteration::Supprimer {
                periode: true,
                rang,
            } => supprimer_ou_doubler(periodes, rang, false),
            Alteration::Supprimer {
                periode: false,
                rang,
            } => supprimer_ou_doubler(lignes, rang, false),
            Alteration::Doubler {
                periode: true,
                rang,
            } => supprimer_ou_doubler(periodes, rang, true),
            Alteration::Doubler {
                periode: false,
                rang,
            } => supprimer_ou_doubler(lignes, rang, true),
            Alteration::Date {
                periode: true,
                rang,
                premier_jour,
                decalage,
            } => {
                if let Some(p) = choisir(periodes, rang) {
                    let date = if premier_jour {
                        &mut p.debut
                    } else {
                        &mut p.fin
                    };
                    *date = decaler_date(*date, decalage);
                }
            }
            Alteration::Date {
                periode: false,
                rang,
                premier_jour,
                decalage,
            } => {
                if let Some(e) = choisir(lignes, rang) {
                    let date = if premier_jour {
                        &mut e.debut
                    } else {
                        &mut e.fin
                    };
                    *date = decaler_date(*date, decalage);
                }
            }
            Alteration::Statut {
                periode: true,
                rang,
                statut,
            } => {
                if let Some(p) = choisir(periodes, rang) {
                    p.statut = match statut % 3 {
                        0 => StatutPeriode::Ouverte,
                        1 => StatutPeriode::Verrouillee,
                        _ => StatutPeriode::Cloturee,
                    };
                }
            }
            Alteration::Statut {
                periode: false,
                rang,
                statut,
            } => {
                if let Some(e) = choisir(lignes, rang) {
                    e.statut = match statut % 3 {
                        0 => StatutExercice::Ouvert,
                        1 => StatutExercice::EnCloture,
                        _ => StatutExercice::Cloture,
                    };
                }
            }
            Alteration::Code {
                periode: true,
                rang,
                decalage,
            } => {
                if let Some(p) = choisir(periodes, rang) {
                    p.exercice = decaler_code(&p.exercice, decalage);
                }
            }
            Alteration::Code {
                periode: false,
                rang,
                decalage,
            } => {
                if let Some(e) = choisir(lignes, rang) {
                    e.code = decaler_code(&e.code, decalage);
                }
            }
        }
    }

    /// Ligne de rang `rang` modulo la longueur (aucune si la liste est vide).
    fn choisir<T>(liste: &mut [T], rang: usize) -> Option<&mut T> {
        let n = liste.len();
        liste.get_mut(rang % n.max(1))
    }

    /// Supprime ou double la ligne de rang `rang` modulo la longueur.
    fn supprimer_ou_doubler<T: Clone>(liste: &mut Vec<T>, rang: usize, doubler: bool) {
        if liste.is_empty() {
            return;
        }
        let i = rang % liste.len();
        if doubler {
            liste.push(liste[i].clone());
        } else {
            liste.remove(i);
        }
    }

    fn decaler_date(date: Date, jours: i64) -> Date {
        date.checked_add(Duration::days(jours)).unwrap_or(date)
    }

    /// Code décalé de `annees` ans (inchangé si le résultat n'est pas un code valide).
    fn decaler_code(code: &CodeExercice, annees: i32) -> CodeExercice {
        code.as_str()
            .parse::<i32>()
            .ok()
            .and_then(|annee| CodeExercice::parse(&(annee + annees).to_string()).ok())
            .unwrap_or_else(|| code.clone())
    }

    /// `reconstituer(aplatir(x)) == x`, lignes dans l'ordre ou en ordre inverse.
    fn aller_retour(exercices: &Exercices) -> Result<(), TestCaseError> {
        let (mut lignes, mut periodes) = exercices.aplatir();
        let relu = Exercices::reconstituer(lignes.clone(), periodes.clone());
        prop_assert_eq!(relu.as_ref(), Ok(exercices));
        lignes.reverse();
        periodes.reverse();
        let relu = Exercices::reconstituer(lignes, periodes);
        prop_assert_eq!(relu.as_ref(), Ok(exercices));
        Ok(())
    }

    proptest! {
        /// Décision du mainteneur du 2026-11-18 : une période (ou un exercice) n'est clôturée
        /// que si aucune date du brouillard n'y tombe ; un refus laisse l'état inchangé et
        /// compte exactement les brouillons concernés.
        #[test]
        fn le_brouillard_bloque_la_cloture(
            decalages in proptest::collection::vec(-40i64..600, 0..8),
        ) {
            let brouillard: Vec<Date> =
                decalages.iter().map(|d| DEBUT_ACTIVITE + Duration::days(*d)).collect();
            let dans = |du: Date, au: Date| brouillard.iter().filter(|d| du <= **d && **d <= au).count();

            // Première période (juillet 2026, partielle) : clôturable sans brouillard.
            let mut exercices = Exercices::new();
            exercices.ouvrir_premier(DEBUT_ACTIVITE, FinPremierExercice::AnneeSuivante).unwrap();
            let avant = exercices.clone();
            let (du, au) = {
                let p = &avant.exercices()[0].periodes()[0];
                (p.debut(), p.fin())
            };
            let nombre = dans(du, au);
            let resultat = exercices.cloturer_periode(DEBUT_ACTIVITE, brouillard.iter().copied());
            if nombre == 0 {
                prop_assert!(resultat.is_ok());
                prop_assert_eq!(exercices.exercices()[0].periodes()[0].statut(), StatutPeriode::Cloturee);
            } else {
                prop_assert_eq!(resultat, Err(ErreurExercice::BrouillardNonValide { du, au, nombre }));
                prop_assert_eq!(&exercices, &avant);
            }

            // Exercice entier (14/07/2026 au 31/12/2027).
            let mut exercices = avant;
            let code = exercices.exercices()[0].code().clone();
            exercices.passer_en_cloture(&code).unwrap();
            let avant = exercices.clone();
            let (du, au) = (avant.exercices()[0].debut(), avant.exercices()[0].fin());
            let nombre = dans(du, au);
            let resultat = exercices.cloturer(&code, brouillard.iter().copied());
            if nombre == 0 {
                prop_assert!(resultat.is_ok());
                prop_assert_eq!(exercices.exercices()[0].statut(), StatutExercice::Cloture);
            } else {
                prop_assert_eq!(resultat, Err(ErreurExercice::BrouillardNonValide { du, au, nombre }));
                prop_assert_eq!(&exercices, &avant);
            }
        }
    }
}
