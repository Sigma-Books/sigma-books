//! Validation, livre des écritures validées et contre-passation (spec §6.1 à §6.3 ; ADR 0007).
//!
//! - [`Livre::valider`] contrôle les invariants du §6.2 sur une [`EcritureEquilibree`], attribue
//!   le numéro définitif (compteur passé en paramètre, fonction pure côté numérotation) et
//!   inscrit l'écriture. C'est la **seule** voie vers une [`EcritureValidee`].
//! - Le livre est en **ajout seul** : aucune fonction ne modifie ni ne retire une écriture
//!   validée, aucune ne rend de `&mut` sur l'une d'elles (règle absolue 5).
//! - [`Livre::contre_passer`] crée l'écriture miroir (sens inversés, mêmes comptes, montants,
//!   devises et tiers), validée et liée à l'originale dans les deux sens ; une écriture n'est
//!   contre-passée qu'une fois.
//! - **Chronologie** : dans un journal et un exercice, une écriture n'est jamais datée avant
//!   la dernière écriture validée (AUDCIF art. 16 al. 3, art. 17 4°, art. 22 3° et 4° :
//!   enregistrement dans l'ordre des dates de valeur, sans insertion intercalaire). Une pièce
//!   externe reçue en retard prend sa date de réception (art. 16 al. 3).
//!
//! Comme Tryton (`account.move.post`, `cancel(reversal=True)`) : le numéro est attribué à la
//! validation, la contre-passation inverse débit et crédit et garde le lien vers l'origine.
//! Écarté : l'annulation par montants négatifs de Tryton (`reversal=False`), contraire à
//! l'invariant 2. L'AUDCIF (art. 20) parle d'« inscription en négatif des éléments erronés » :
//! la forme de la correction est soumise à l'expert-comptable (spec §26).

use std::collections::{BTreeMap, BTreeSet};

use rust_decimal::Decimal;
use time::Date;

use crate::compte::NumeroCompte;
use crate::ecriture::{
    EcritureBrouillon, EcritureEquilibree, ErreurEcriture, Justification, Ligne, Refus,
    VersionParametres,
};
use crate::exercice::{CodeExercice, Exercices, StatutExercice, StatutPeriode};
use crate::journal::{CodeJournal, Journaux};
use crate::numerotation::{Compteur, ErreurNumerotation, NumeroEcriture};
use crate::plan::{ErreurPlan, PlanComptes, Usage};
use crate::texte::TexteCourt;

/// Motifs de refus d'une validation ou d'une contre-passation.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ErreurValidation {
    /// Journal absent du dossier.
    #[error("le journal {0} n'existe pas")]
    JournalInconnu(CodeJournal),
    /// Journal qui n'accepte pas d'écriture saisie (à-nouveaux, générés uniquement ; inv. 7).
    #[error("le journal {0} n'accepte pas de saisie")]
    JournalSansSaisie(CodeJournal),
    /// Écriture d'un journal de trésorerie qui ne mouvemente pas son compte (inv. 7).
    #[error("l'écriture du journal {journal} ne mouvemente pas son compte de trésorerie {compte}")]
    CompteTresorerieAbsent {
        /// Journal de trésorerie.
        journal: CodeJournal,
        /// Compte de trésorerie associé.
        compte: NumeroCompte,
    },
    /// Date hors de tout exercice du dossier (inv. 5).
    #[error("aucun exercice ne contient la date {0}")]
    DateHorsExercice(Date),
    /// Exercice clôturé (inv. 5).
    #[error("l'exercice {0} est clôturé")]
    ExerciceCloture(CodeExercice),
    /// Exercice en clôture : seuls les journaux admis en clôture (OD et IN par défaut) sont
    /// ouverts (inv. 5).
    #[error(
        "l'exercice {0} est en clôture : seuls les journaux admis en clôture (OD, IN) sont ouverts"
    )]
    ExerciceEnCloture(CodeExercice),
    /// Période verrouillée ou clôturée (inv. 5).
    #[error("la période contenant le {0} n'est pas ouverte")]
    PeriodeFermee(Date),
    /// Compte inexistant, inactif ou de regroupement (inv. 3).
    #[error("ligne {ligne} : {erreur}")]
    Compte {
        /// Indice de la ligne (à partir de 0).
        ligne: usize,
        /// Motif.
        erreur: ErreurPlan,
    },
    /// Ligne sur un compte collectif sans tiers (inv. 4).
    #[error("ligne {ligne} : le compte collectif {compte} exige un tiers")]
    TiersManquant {
        /// Indice de la ligne (à partir de 0).
        ligne: usize,
        /// Compte collectif.
        compte: NumeroCompte,
    },
    /// Aucune justification (inv. 6).
    #[error("aucune pièce ni motif « sans pièce »")]
    JustificationManquante,
    /// Motif « sans pièce » dans un journal qui ne l'autorise pas (inv. 6).
    #[error("le journal {0} n'autorise pas les écritures sans pièce")]
    SansPieceInterdit(CodeJournal),
    /// Compteur d'un autre journal ou d'un autre exercice que l'écriture.
    #[error(
        "compteur {compteur_journal}-{compteur_exercice} fourni pour une écriture {journal}-{exercice}"
    )]
    CompteurInadapte {
        /// Journal du compteur.
        compteur_journal: CodeJournal,
        /// Exercice du compteur.
        compteur_exercice: CodeExercice,
        /// Journal de l'écriture.
        journal: CodeJournal,
        /// Exercice de la date de l'écriture.
        exercice: CodeExercice,
    },
    /// Compteur en retard ou en avance sur le livre : trou ou doublon évité.
    #[error(
        "compteur désynchronisé : dernière séquence {fournie} fournie, {attendue} dans le livre"
    )]
    CompteurDesynchronise {
        /// Dernière séquence du compteur fourni.
        fournie: u32,
        /// Dernière séquence inscrite dans le livre.
        attendue: u32,
    },
    /// Équilibre perdu (impossible pour un miroir d'écriture équilibrée ; contrôlé quand même).
    #[error("équilibre : {0}")]
    Equilibre(#[from] ErreurEcriture),
    /// Séquence épuisée.
    #[error("numérotation : {0}")]
    Numerotation(#[from] ErreurNumerotation),
    /// Écriture à contre-passer absente du livre.
    #[error("l'écriture {0} n'existe pas")]
    EcritureInconnue(NumeroEcriture),
    /// Écriture déjà contre-passée.
    #[error("l'écriture {origine} est déjà contre-passée par {par}")]
    DejaContrePassee {
        /// Écriture d'origine.
        origine: NumeroEcriture,
        /// Contre-passation existante.
        par: NumeroEcriture,
    },
    /// Écriture datée avant la dernière écriture validée du même journal et du même exercice :
    /// l'enregistrement suit l'ordre des dates de valeur, sans insertion intercalaire
    /// (AUDCIF art. 16 al. 3, art. 17 4°, art. 22 3° et 4°).
    #[error(
        "chronologie : l'écriture du {date} précède la dernière écriture validée du journal {journal} ({derniere})"
    )]
    DateAnterieureAuJournal {
        /// Date de l'écriture.
        date: Date,
        /// Journal.
        journal: CodeJournal,
        /// Date de la dernière écriture validée de ce journal dans l'exercice.
        derniere: Date,
    },
    /// Contre-passation datée avant l'écriture d'origine.
    #[error("la contre-passation ({date}) ne peut précéder l'écriture d'origine ({origine})")]
    DateAnterieureAOrigine {
        /// Date demandée.
        date: Date,
        /// Date de l'écriture d'origine.
        origine: Date,
    },
}

/// Règles de justification propres au dossier : journaux où le motif « sans pièce » est admis
/// (spec §6.2, invariant 6). Paramètre du dossier ; aucun journal par défaut.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReglesJustification {
    sans_piece: BTreeSet<CodeJournal>,
}

impl ReglesJustification {
    /// Aucun journal n'admet d'écriture sans pièce.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Journaux qui admettent le motif « sans pièce ».
    #[must_use]
    pub fn sans_piece_admis(journaux: impl IntoIterator<Item = CodeJournal>) -> Self {
        Self {
            sans_piece: journaux.into_iter().collect(),
        }
    }

    /// Vrai si le journal admet le motif « sans pièce ».
    #[must_use]
    pub fn admet_sans_piece(&self, journal: &CodeJournal) -> bool {
        self.sans_piece.contains(journal)
    }
}

/// État du dossier nécessaire à la validation : référentiel et calendrier, en lecture seule.
#[derive(Debug, Clone, Copy)]
pub struct Contexte<'a> {
    /// Plan de comptes du dossier.
    pub plan: &'a PlanComptes,
    /// Journaux du dossier.
    pub journaux: &'a Journaux,
    /// Exercices et périodes.
    pub exercices: &'a Exercices,
    /// Règles de justification.
    pub regles: &'a ReglesJustification,
}

/// Écriture validée : définitive, numérotée, en lecture seule. Ne se construit que par
/// [`Livre::valider`] ou [`Livre::contre_passer`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EcritureValidee {
    numero: NumeroEcriture,
    date: Date,
    libelle: TexteCourt,
    lignes: Vec<Ligne>,
    justification: Justification,
    version_parametres: Option<VersionParametres>,
    date_validation: Date,
    origine: Option<NumeroEcriture>,
}

impl EcritureValidee {
    /// Numéro définitif.
    #[must_use]
    pub fn numero(&self) -> &NumeroEcriture {
        &self.numero
    }

    /// Journal (celui du numéro).
    #[must_use]
    pub fn journal(&self) -> &CodeJournal {
        self.numero.journal()
    }

    /// Date comptable.
    #[must_use]
    pub fn date(&self) -> Date {
        self.date
    }

    /// Libellé, tel que saisi.
    #[must_use]
    pub fn libelle(&self) -> &TexteCourt {
        &self.libelle
    }

    /// Lignes, dans l'ordre de saisie.
    #[must_use]
    pub fn lignes(&self) -> &[Ligne] {
        &self.lignes
    }

    /// Justification.
    #[must_use]
    pub fn justification(&self) -> &Justification {
        &self.justification
    }

    /// Version des paramètres fiscaux utilisée.
    #[must_use]
    pub fn version_parametres(&self) -> Option<&VersionParametres> {
        self.version_parametres.as_ref()
    }

    /// Date de la validation (fournie par l'appelant : le moteur ne lit pas l'horloge).
    #[must_use]
    pub fn date_validation(&self) -> Date {
        self.date_validation
    }

    /// Pour une contre-passation : numéro de l'écriture d'origine.
    #[must_use]
    pub fn origine(&self) -> Option<&NumeroEcriture> {
        self.origine.as_ref()
    }

    /// Total des débits (égal au total des crédits) en XOF.
    #[must_use]
    pub fn total(&self) -> Decimal {
        self.lignes.iter().map(Ligne::debit).sum()
    }
}

/// Livre des écritures validées d'un dossier, en ajout seul.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Livre {
    ecritures: Vec<EcritureValidee>,
    index: BTreeMap<NumeroEcriture, usize>,
    contre_passations: BTreeMap<NumeroEcriture, NumeroEcriture>,
    /// Dernière séquence et dernière date validées, par journal et exercice.
    derniers: BTreeMap<(CodeJournal, CodeExercice), (u32, Date)>,
}

impl Livre {
    /// Livre vide.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Écritures validées, dans l'ordre de validation.
    #[must_use]
    pub fn ecritures(&self) -> &[EcritureValidee] {
        &self.ecritures
    }

    /// Écriture de numéro donné.
    #[must_use]
    pub fn ecriture(&self, numero: &NumeroEcriture) -> Option<&EcritureValidee> {
        self.index.get(numero).and_then(|i| self.ecritures.get(*i))
    }

    /// Contre-passation d'une écriture, si elle existe (l'autre sens du lien est
    /// [`EcritureValidee::origine`]).
    #[must_use]
    pub fn contre_passee_par(&self, numero: &NumeroEcriture) -> Option<&NumeroEcriture> {
        self.contre_passations.get(numero)
    }

    /// Usage d'un compte : mouvementé si une écriture validée l'utilise (plan, suppression).
    #[must_use]
    pub fn usage_compte(&self, compte: &NumeroCompte) -> Usage {
        let utilise = self
            .ecritures
            .iter()
            .any(|e| e.lignes.iter().any(|l| l.compte() == compte));
        if utilise {
            Usage::Mouvemente
        } else {
            Usage::SansEcriture
        }
    }

    /// Usage d'un journal : mouvementé si une écriture validée l'utilise (changement de code).
    #[must_use]
    pub fn usage_journal(&self, journal: &CodeJournal) -> Usage {
        if self.ecritures.iter().any(|e| e.journal() == journal) {
            Usage::Mouvemente
        } else {
            Usage::SansEcriture
        }
    }

    /// Valide une écriture équilibrée : contrôle les invariants du §6.2, attribue le numéro
    /// suivant du `compteur` et l'inscrit au livre. Renvoie le numéro et le compteur à
    /// enregistrer à la place de l'ancien (dans la même transaction, lot L2).
    ///
    /// # Errors
    /// Invariant non respecté, compteur inadapté ou désynchronisé. L'écriture est rendue
    /// intacte avec le motif ; le livre n'est pas modifié.
    pub fn valider(
        &mut self,
        ecriture: EcritureEquilibree,
        contexte: &Contexte<'_>,
        compteur: &Compteur,
        date_validation: Date,
    ) -> Result<(NumeroEcriture, Compteur), Refus<EcritureEquilibree, ErreurValidation>> {
        match self.inscrire(&ecriture, None, contexte, compteur, date_validation) {
            Ok(resultat) => Ok(resultat),
            Err(erreur) => Err(Refus::nouveau(ecriture, erreur)),
        }
    }

    /// Contre-passe l'écriture `origine` : écriture miroir dans le même journal, à la `date`
    /// donnée, avec la même justification, validée selon les mêmes contrôles.
    ///
    /// # Errors
    /// Écriture inconnue ou déjà contre-passée, date antérieure à l'origine, ou invariant non
    /// respecté à la date choisie. Le livre n'est pas modifié.
    pub fn contre_passer(
        &mut self,
        origine: &NumeroEcriture,
        date: Date,
        libelle: TexteCourt,
        contexte: &Contexte<'_>,
        compteur: &Compteur,
        date_validation: Date,
    ) -> Result<(NumeroEcriture, Compteur), ErreurValidation> {
        let source = self
            .ecriture(origine)
            .ok_or_else(|| ErreurValidation::EcritureInconnue(origine.clone()))?;
        if let Some(par) = self.contre_passations.get(origine) {
            return Err(ErreurValidation::DejaContrePassee {
                origine: origine.clone(),
                par: par.clone(),
            });
        }
        if date < source.date {
            return Err(ErreurValidation::DateAnterieureAOrigine {
                date,
                origine: source.date,
            });
        }
        let mut miroir = EcritureBrouillon::nouvelle(source.journal().clone(), date, libelle);
        for ligne in &source.lignes {
            miroir.ajouter_ligne(ligne.miroir());
        }
        miroir.justifier(source.justification.clone());
        if let Some(version) = &source.version_parametres {
            miroir.utiliser_parametres(version.clone());
        }
        // Le miroir d'une écriture équilibrée l'est aussi : l'échec est impossible, mais il
        // est traité comme une erreur plutôt que par un `expect`.
        let miroir = miroir
            .valider_equilibre()
            .map_err(|refus| ErreurValidation::Equilibre(refus.decomposer().1))?;
        self.inscrire(
            &miroir,
            Some(origine.clone()),
            contexte,
            compteur,
            date_validation,
        )
    }

    fn inscrire(
        &mut self,
        ecriture: &EcritureEquilibree,
        origine: Option<NumeroEcriture>,
        contexte: &Contexte<'_>,
        compteur: &Compteur,
        date_validation: Date,
    ) -> Result<(NumeroEcriture, Compteur), ErreurValidation> {
        let exercice = controler(ecriture, contexte)?;
        let justification = ecriture
            .justification()
            .cloned()
            .ok_or(ErreurValidation::JustificationManquante)?;
        if compteur.journal() != ecriture.journal() || compteur.exercice() != &exercice {
            return Err(ErreurValidation::CompteurInadapte {
                compteur_journal: compteur.journal().clone(),
                compteur_exercice: compteur.exercice().clone(),
                journal: ecriture.journal().clone(),
                exercice,
            });
        }
        let cle = (ecriture.journal().clone(), exercice);
        let (attendue, derniere) = match self.derniers.get(&cle) {
            Some(&(sequence, date)) => (sequence, Some(date)),
            None => (0, None),
        };
        if compteur.dernier() != attendue {
            return Err(ErreurValidation::CompteurDesynchronise {
                fournie: compteur.dernier(),
                attendue,
            });
        }
        if let Some(derniere) = derniere.filter(|&d| ecriture.date() < d) {
            return Err(ErreurValidation::DateAnterieureAuJournal {
                date: ecriture.date(),
                journal: ecriture.journal().clone(),
                derniere,
            });
        }
        let (numero, suivant) = compteur.suivant()?;
        let brouillon = ecriture.clone().decomposer();
        let validee = EcritureValidee {
            numero: numero.clone(),
            date: brouillon.date(),
            libelle: brouillon.libelle().clone(),
            lignes: brouillon.lignes().to_vec(),
            justification,
            version_parametres: ecriture.version_parametres().cloned(),
            date_validation,
            origine: origine.clone(),
        };
        self.derniers.insert(cle, (numero.sequence(), validee.date));
        self.index.insert(numero.clone(), self.ecritures.len());
        if let Some(origine) = origine {
            self.contre_passations.insert(origine, numero.clone());
        }
        self.ecritures.push(validee);
        Ok((numero, suivant))
    }
}

/// Contrôle des invariants 3 à 7 du §6.2 (1 et 2 sont garantis par les types). Renvoie le code
/// de l'exercice qui contient la date.
fn controler(
    ecriture: &EcritureEquilibree,
    contexte: &Contexte<'_>,
) -> Result<CodeExercice, ErreurValidation> {
    let code = ecriture.journal();
    // Invariant 7 : journal existant, ouvert à la saisie.
    let journal = contexte
        .journaux
        .journal(code)
        .ok_or_else(|| ErreurValidation::JournalInconnu(code.clone()))?;
    if !journal.genre().accepte_la_saisie() {
        return Err(ErreurValidation::JournalSansSaisie(code.clone()));
    }
    // Invariant 5 : date dans une période ouverte d'un exercice ouvert, ou en clôture pour les
    // journaux admis en clôture (OD et IN par défaut, pas PA).
    let date = ecriture.date();
    let exercice = contexte
        .exercices
        .exercice_contenant(date)
        .ok_or(ErreurValidation::DateHorsExercice(date))?;
    match exercice.statut() {
        StatutExercice::Ouvert => {}
        StatutExercice::EnCloture if journal.admis_en_cloture() => {}
        StatutExercice::EnCloture => {
            return Err(ErreurValidation::ExerciceEnCloture(exercice.code().clone()));
        }
        StatutExercice::Cloture => {
            return Err(ErreurValidation::ExerciceCloture(exercice.code().clone()));
        }
    }
    let periode_ouverte = exercice
        .periode_contenant(date)
        .is_some_and(|p| p.statut() == StatutPeriode::Ouverte);
    if !periode_ouverte {
        return Err(ErreurValidation::PeriodeFermee(date));
    }
    // Invariants 3 et 4 : comptes de mouvement actifs ; tiers sur les comptes collectifs.
    for (indice, ligne) in ecriture.lignes().iter().enumerate() {
        let compte = contexte
            .plan
            .compte_de_mouvement_actif(ligne.compte())
            .map_err(|erreur| ErreurValidation::Compte {
                ligne: indice,
                erreur,
            })?;
        if compte.est_collectif() && ligne.tiers().is_none() {
            return Err(ErreurValidation::TiersManquant {
                ligne: indice,
                compte: ligne.compte().clone(),
            });
        }
    }
    // Invariant 6 : pièce, ou motif « sans pièce » admis par le journal.
    match ecriture.justification() {
        None => return Err(ErreurValidation::JustificationManquante),
        Some(Justification::SansPiece(_)) if !contexte.regles.admet_sans_piece(code) => {
            return Err(ErreurValidation::SansPieceInterdit(code.clone()));
        }
        Some(_) => {}
    }
    // Invariant 7 : une écriture d'un journal de trésorerie mouvemente son compte.
    if let Some(tresorerie) = journal.compte_tresorerie()
        && !ecriture.lignes().iter().any(|l| l.compte() == tresorerie)
    {
        return Err(ErreurValidation::CompteTresorerieAbsent {
            journal: code.clone(),
            compte: tresorerie.clone(),
        });
    }
    Ok(exercice.code().clone())
}
