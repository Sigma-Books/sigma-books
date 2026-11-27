//! Lettrage en ajout seul (spec §10.1 ; ADR 0032).
//!
//! - Un **lien** impute un même montant à une ligne au débit et à une ligne au crédit d'un même
//!   compte lettrable : débit imputé = crédit imputé **par construction**, sans tolérance.
//! - Un **lettrage** est une opération (lettre, date, origine) qui regroupe des liens. Il est
//!   total ou partiel (B1, modèle d'Odoo `account_partial_reconcile.py:15-56`) ; les conditions
//!   sont celles de Tryton (`move.py:646-690`) : même compte lettrable, **même tiers** (résolu
//!   au sens de la fusion, ADR 0031), lignes **validées** (une [`IdLigne`] désigne une ligne
//!   d'écriture validée par son rang de chaîne).
//! - Le **résiduel** d'une ligne (|montant| moins la somme des liens non rompus) est
//!   **calculé**, jamais écrit sur la ligne ; il reste compris entre 0 et le montant.
//! - Le dé-lettrage est une **rupture** ajoutée (B2), jamais une suppression ; elle est refusée
//!   si la date du lettrage tombe dans un exercice clos (spec §10.1).
//! - Les dates (lettrage, rupture) sont le **jour de l'opération** dans le fuseau du dossier,
//!   passé en paramètre (ADR 0032, point 3) : écart assumé avec Tryton et Odoo, qui datent par
//!   la plus haute date des lignes.
//!
//! Le [`RegistreLettrage`] d'un compte ne modifie jamais une ligne : il ne garde que les
//! lettrages et les ruptures ajoutés, et un état dérivé (montant imputé par ligne).

mod lettre;
mod propositions;

use std::collections::{BTreeMap, BTreeSet};

use rust_decimal::Decimal;
use time::Date;

use crate::compte::NumeroCompte;
use crate::ecriture::{IdTiers, Ligne, Sens};
use crate::exercice::{Exercices, StatutExercice};
use crate::livre::EcritureValidee;
use crate::montant::Montant;

pub use lettre::Lettre;
pub use propositions::{BilanGroupe, Bornes, Proposition, Propositions, Regle};

/// Longueur maximale d'un motif de rupture, en octets (ADR 0032, point 5).
pub const MOTIF_OCTETS_MAX: usize = 400;

/// Erreurs du lettrage.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ErreurLettrage {
    /// Rang de chaîne ou position de ligne nul.
    #[error("identifiant de ligne invalide : rang {rang}, position {position}")]
    IdLigneInvalide {
        /// Rang de chaîne de l'écriture.
        rang: u64,
        /// Position de la ligne dans l'écriture.
        position: u16,
    },
    /// Ligne absente de l'écriture ou du registre.
    #[error("ligne inconnue : {0}")]
    LigneInconnue(IdLigne),
    /// Tiers résolu fourni pour une ligne sans tiers, ou absent pour une ligne avec tiers.
    #[error("tiers résolu incohérent avec la ligne {0}")]
    TiersResoluIncoherent(IdLigne),
    /// Compte non marqué lettrable.
    #[error("compte non lettrable : {0}")]
    CompteNonLettrable(NumeroCompte),
    /// Ligne d'un autre compte que celui du registre.
    #[error("la ligne {0} n'est pas sur le compte du registre")]
    AutreCompte(IdLigne),
    /// Ligne citée deux fois.
    #[error("ligne en double : {0}")]
    LigneEnDouble(IdLigne),
    /// Sélection sans ligne de ce sens.
    #[error("aucune ligne au {0:?} dans la sélection")]
    SensManquant(Sens),
    /// Lignes de tiers différents (Tryton `move.py:676-682`).
    #[error("lignes de tiers différents")]
    TiersDifferents,
    /// Ligne déjà entièrement lettrée.
    #[error("ligne déjà soldée : {0}")]
    LigneSoldee(IdLigne),
    /// Montant d'un lien nul ou négatif.
    #[error("montant de lien non strictement positif : {0}")]
    MontantNonPositif(Decimal),
    /// Lien dont la ligne « débit » n'est pas au débit ou la ligne « crédit » pas au crédit.
    #[error("sens de la ligne {0} contraire au lien")]
    SensContraire(IdLigne),
    /// Liens dont le total dépasse le résiduel de la ligne.
    #[error("le résiduel de la ligne {0} deviendrait négatif")]
    ResiduelDepasse(IdLigne),
    /// Lettrage sans lien.
    #[error("lettrage sans lien")]
    SansLien,
    /// Ligne en devise étrangère non lettrable en L3 (ADR 0032, point 1 ; écarts de change : L6).
    #[error("lettrage en devise étrangère hors du périmètre de L3")]
    DeviseHorsL3,
    /// Texte de lettre invalide.
    #[error("lettre invalide : « {0} »")]
    LettreInvalide(String),
    /// Suite de lettres épuisée.
    #[error("suite de lettres épuisée")]
    LettresEpuisees,
    /// Lettre déjà employée sur le compte (une lettre n'est jamais réutilisée).
    #[error("lettre déjà employée : {0}")]
    LettreDejaEmployee(Lettre),
    /// Lettre inconnue sur le compte.
    #[error("lettre inconnue : {0}")]
    LettreInconnue(Lettre),
    /// Lettrage déjà rompu (au plus une rupture par lettrage).
    #[error("lettrage {0} déjà rompu")]
    DejaRompu(Lettre),
    /// Rupture datée avant le lettrage.
    #[error("rupture du lettrage {0} antérieure au lettrage")]
    RuptureAnterieure(Lettre),
    /// Rupture refusée : la date du lettrage tombe dans un exercice clos (B2, spec §10.1).
    #[error("lettrage {0} d'un exercice clos : rupture refusée")]
    ExerciceClos(Lettre),
    /// Motif de rupture vide, trop long ou contenant un caractère interdit.
    #[error("motif de rupture invalide")]
    MotifInvalide,
    /// Bornes de combinaison hors des limites admises.
    #[error("bornes de combinaison invalides")]
    BornesInvalides,
    /// Dépassement de capacité décimale.
    #[error("dépassement de capacité")]
    Depassement,
}

/// Identifiant d'une ligne **validée** : rang de chaîne de l'écriture (unique par dossier,
/// ADR 0020) et position de la ligne (à partir de 1). L'ordre (rang, position) départage les
/// imputations et les propositions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct IdLigne {
    rang: u64,
    position: u16,
}

impl IdLigne {
    /// Construit un identifiant.
    ///
    /// # Errors
    /// [`ErreurLettrage::IdLigneInvalide`] si le rang ou la position est nul.
    pub fn nouveau(rang: u64, position: u16) -> Result<Self, ErreurLettrage> {
        if rang == 0 || position == 0 {
            return Err(ErreurLettrage::IdLigneInvalide { rang, position });
        }
        Ok(Self { rang, position })
    }

    /// Rang de chaîne de l'écriture.
    #[must_use]
    pub fn rang(&self) -> u64 {
        self.rang
    }

    /// Position de la ligne dans l'écriture.
    #[must_use]
    pub fn position(&self) -> u16 {
        self.position
    }
}

impl std::fmt::Display for IdLigne {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{}", self.rang, self.position)
    }
}

/// Ligne validée vue par le lettrage : immuable, construite à partir d'une [`Ligne`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LigneLettrable {
    id: IdLigne,
    compte: NumeroCompte,
    tiers: Option<IdTiers>,
    sens: Sens,
    montant: Decimal,
    devise: Option<Montant>,
    date: Date,
    echeance: Option<Date>,
    reference: Option<String>,
}

impl LigneLettrable {
    /// Ligne relue d'une écriture validée (rang de chaîne dans `id`).
    ///
    /// - `date` : date comptable de l'écriture ;
    /// - `tiers_resolu` : tiers de la ligne **résolu** au sens de la fusion (ADR 0031) ;
    /// - `echeance` : échéance effective (ADR 0033), à défaut la date comptable ;
    /// - `reference` : référence de rapprochement, normalisée (casse, espaces) ; vide = aucune.
    ///
    /// # Errors
    /// [`ErreurLettrage::TiersResoluIncoherent`] si le tiers résolu manque ou est en trop.
    pub fn nouvelle(
        id: IdLigne,
        ligne: &Ligne,
        date: Date,
        tiers_resolu: Option<IdTiers>,
        echeance: Option<Date>,
        reference: Option<&str>,
    ) -> Result<Self, ErreurLettrage> {
        if ligne.tiers().is_some() != tiers_resolu.is_some() {
            return Err(ErreurLettrage::TiersResoluIncoherent(id));
        }
        Ok(Self {
            id,
            compte: ligne.compte().clone(),
            tiers: tiers_resolu,
            sens: ligne.sens(),
            montant: ligne.montant().valeur(),
            devise: ligne.devise().map(crate::ecriture::MontantDevise::montant),
            date,
            echeance,
            reference: reference.and_then(normaliser_reference),
        })
    }

    /// Ligne `position` (à partir de 1) d'une écriture validée de rang de chaîne `rang`.
    ///
    /// # Errors
    /// Identifiant invalide, position hors de l'écriture, tiers résolu incohérent.
    pub fn depuis_validee(
        ecriture: &EcritureValidee,
        rang: u64,
        position: u16,
        tiers_resolu: Option<IdTiers>,
        echeance: Option<Date>,
        reference: Option<&str>,
    ) -> Result<Self, ErreurLettrage> {
        let id = IdLigne::nouveau(rang, position)?;
        let ligne = ecriture
            .lignes()
            .get(usize::from(position) - 1)
            .ok_or(ErreurLettrage::LigneInconnue(id))?;
        Self::nouvelle(
            id,
            ligne,
            ecriture.date(),
            tiers_resolu,
            echeance,
            reference,
        )
    }

    /// Identifiant.
    #[must_use]
    pub fn id(&self) -> IdLigne {
        self.id
    }

    /// Compte.
    #[must_use]
    pub fn compte(&self) -> &NumeroCompte {
        &self.compte
    }

    /// Tiers résolu.
    #[must_use]
    pub fn tiers(&self) -> Option<&IdTiers> {
        self.tiers.as_ref()
    }

    /// Sens.
    #[must_use]
    pub fn sens(&self) -> Sens {
        self.sens
    }

    /// Montant en devise de tenue, strictement positif.
    #[must_use]
    pub fn montant(&self) -> Decimal {
        self.montant
    }

    /// Montant en devise étrangère, s'il y en a un.
    #[must_use]
    pub fn devise(&self) -> Option<Montant> {
        self.devise
    }

    /// Référence normalisée.
    #[must_use]
    pub fn reference(&self) -> Option<&str> {
        self.reference.as_deref()
    }

    /// Clé d'ordre : échéance effective, à défaut date comptable, puis rang de chaîne et
    /// position (Tryton `move.py:1873-1874`, Odoo `account_move_line.py:2735-2745`).
    fn cle(&self) -> (Date, IdLigne) {
        (self.echeance.unwrap_or(self.date), self.id)
    }
}

/// Normalise une référence : minuscules, espaces retirés. `None` si elle est vide (une
/// référence vide correspondrait à tout, OCA `account_bank_statement_line.py:1040-1057`).
fn normaliser_reference(texte: &str) -> Option<String> {
    let normalisee: String = texte
        .chars()
        .filter(|c| !c.is_whitespace())
        .flat_map(char::to_lowercase)
        .collect();
    (!normalisee.is_empty()).then_some(normalisee)
}

/// Lien : un même montant imputé à une ligne au débit et à une ligne au crédit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Lien {
    debit: IdLigne,
    credit: IdLigne,
    montant: Decimal,
}

impl Lien {
    /// Construit un lien (relecture de la persistance).
    ///
    /// # Errors
    /// [`ErreurLettrage::MontantNonPositif`] si le montant n'est pas strictement positif.
    pub fn nouveau(
        debit: IdLigne,
        credit: IdLigne,
        montant: Decimal,
    ) -> Result<Self, ErreurLettrage> {
        if montant <= Decimal::ZERO {
            return Err(ErreurLettrage::MontantNonPositif(montant));
        }
        Ok(Self {
            debit,
            credit,
            montant,
        })
    }

    /// Ligne au débit.
    #[must_use]
    pub fn debit(&self) -> IdLigne {
        self.debit
    }

    /// Ligne au crédit.
    #[must_use]
    pub fn credit(&self) -> IdLigne {
        self.credit
    }

    /// Montant imputé de chaque côté.
    #[must_use]
    pub fn montant(&self) -> Decimal {
        self.montant
    }
}

/// Origine d'un lettrage (ADR 0032, point 1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Origine {
    /// Saisi par un utilisateur.
    Manuelle,
    /// Appliqué automatiquement (paramètre du dossier, acteur `systeme`).
    Automatique,
    /// Proposition validée par un utilisateur.
    PropositionValidee,
}

/// Lettrage : une opération, immuable une fois inscrite.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lettrage {
    lettre: Lettre,
    date: Date,
    origine: Origine,
    liens: Vec<Lien>,
}

impl Lettrage {
    /// Lettre.
    #[must_use]
    pub fn lettre(&self) -> Lettre {
        self.lettre
    }

    /// Date de l'opération.
    #[must_use]
    pub fn date(&self) -> Date {
        self.date
    }

    /// Origine.
    #[must_use]
    pub fn origine(&self) -> Origine {
        self.origine
    }

    /// Liens.
    #[must_use]
    pub fn liens(&self) -> &[Lien] {
        &self.liens
    }

    /// Montant total lettré (débit imputé = crédit imputé).
    #[must_use]
    pub fn montant(&self) -> Decimal {
        self.liens.iter().map(Lien::montant).sum()
    }

    /// Lignes citées, dans l'ordre des identifiants.
    #[must_use]
    pub fn lignes(&self) -> BTreeSet<IdLigne> {
        self.liens
            .iter()
            .flat_map(|l| [l.debit, l.credit])
            .collect()
    }
}

/// Motif d'une rupture : non vide, au plus [`MOTIF_OCTETS_MAX`] octets, sans caractère de
/// contrôle ni de format (ADR 0032, point 5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MotifRupture(String);

impl MotifRupture {
    /// Valide un motif.
    ///
    /// # Errors
    /// [`ErreurLettrage::MotifInvalide`].
    pub fn parse(texte: &str) -> Result<Self, ErreurLettrage> {
        if texte.trim().is_empty()
            || texte.len() > MOTIF_OCTETS_MAX
            || texte.chars().any(caractere_interdit)
        {
            return Err(ErreurLettrage::MotifInvalide);
        }
        Ok(Self(texte.to_owned()))
    }

    /// Le motif.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Caractère de contrôle (`Cc`), séparateur de ligne ou de paragraphe, ou caractère de format
/// (`Cf`, Unicode 16.0) : même liste que `crates/api/src/pieces/mod.rs`.
fn caractere_interdit(c: char) -> bool {
    c.is_control()
        || matches!(
            c,
            '\u{00AD}'
                | '\u{0600}'..='\u{0605}'
                | '\u{061C}'
                | '\u{06DD}'
                | '\u{070F}'
                | '\u{0890}'..='\u{0891}'
                | '\u{08E2}'
                | '\u{180E}'
                | '\u{200B}'..='\u{200F}'
                | '\u{2028}'..='\u{202E}'
                | '\u{2060}'..='\u{2064}'
                | '\u{2066}'..='\u{206F}'
                | '\u{FEFF}'
                | '\u{FFF9}'..='\u{FFFB}'
                | '\u{110BD}'
                | '\u{110CD}'
                | '\u{13430}'..='\u{1343F}'
                | '\u{1BCA0}'..='\u{1BCA3}'
                | '\u{1D173}'..='\u{1D17A}'
                | '\u{E0001}'
                | '\u{E0020}'..='\u{E007F}'
        )
}

/// Rupture d'un lettrage : ajoutée, jamais supprimée.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rupture {
    lettre: Lettre,
    date: Date,
    motif: MotifRupture,
}

impl Rupture {
    /// Lettre du lettrage rompu.
    #[must_use]
    pub fn lettre(&self) -> Lettre {
        self.lettre
    }

    /// Date de l'opération.
    #[must_use]
    pub fn date(&self) -> Date {
        self.date
    }

    /// Motif.
    #[must_use]
    pub fn motif(&self) -> &MotifRupture {
        &self.motif
    }
}

/// Lettrages d'un compte lettrable : lignes, lettrages et ruptures, en ajout seul.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegistreLettrage {
    compte: NumeroCompte,
    lignes: BTreeMap<IdLigne, LigneLettrable>,
    /// État dérivé : somme des liens non rompus qui citent la ligne.
    impute: BTreeMap<IdLigne, Decimal>,
    lettrages: BTreeMap<Lettre, Lettrage>,
    ruptures: BTreeMap<Lettre, Rupture>,
}

impl RegistreLettrage {
    /// Ouvre le registre d'un compte ; `lettrable` vient de la marque de données du compte.
    ///
    /// # Errors
    /// [`ErreurLettrage::CompteNonLettrable`].
    pub fn ouvrir(compte: NumeroCompte, lettrable: bool) -> Result<Self, ErreurLettrage> {
        if !lettrable {
            return Err(ErreurLettrage::CompteNonLettrable(compte));
        }
        Ok(Self {
            compte,
            lignes: BTreeMap::new(),
            impute: BTreeMap::new(),
            lettrages: BTreeMap::new(),
            ruptures: BTreeMap::new(),
        })
    }

    /// Ajoute une ligne validée du compte.
    ///
    /// # Errors
    /// Ligne d'un autre compte ou déjà présente.
    pub fn ajouter_ligne(&mut self, ligne: LigneLettrable) -> Result<(), ErreurLettrage> {
        if ligne.compte != self.compte {
            return Err(ErreurLettrage::AutreCompte(ligne.id));
        }
        if self.lignes.contains_key(&ligne.id) {
            return Err(ErreurLettrage::LigneEnDouble(ligne.id));
        }
        self.lignes.insert(ligne.id, ligne);
        Ok(())
    }

    /// Compte du registre.
    #[must_use]
    pub fn compte(&self) -> &NumeroCompte {
        &self.compte
    }

    /// Lignes, dans l'ordre des identifiants.
    pub fn lignes(&self) -> impl Iterator<Item = &LigneLettrable> {
        self.lignes.values()
    }

    /// Lettrages, rompus compris, dans l'ordre des lettres.
    pub fn lettrages(&self) -> impl Iterator<Item = &Lettrage> {
        self.lettrages.values()
    }

    /// Lettrage d'une lettre.
    #[must_use]
    pub fn lettrage(&self, lettre: Lettre) -> Option<&Lettrage> {
        self.lettrages.get(&lettre)
    }

    /// Rupture d'un lettrage, s'il est rompu.
    #[must_use]
    pub fn rupture(&self, lettre: Lettre) -> Option<&Rupture> {
        self.ruptures.get(&lettre)
    }

    /// Lettre que recevra le prochain lettrage (jamais une lettre déjà employée).
    ///
    /// # Errors
    /// [`ErreurLettrage::LettresEpuisees`].
    pub fn prochaine_lettre(&self) -> Result<Lettre, ErreurLettrage> {
        self.lettrages
            .last_key_value()
            .map_or(Ok(Lettre::premiere()), |(l, _)| l.suivante())
    }

    /// Résiduel d'une ligne : montant moins les liens non rompus ; `None` si elle est inconnue.
    #[must_use]
    pub fn residuel(&self, id: IdLigne) -> Option<Decimal> {
        let ligne = self.lignes.get(&id)?;
        Some(ligne.montant - self.impute.get(&id).copied().unwrap_or_default())
    }

    /// Lettres non rompues qui citent la ligne (une ligne partielle peut en porter plusieurs).
    #[must_use]
    pub fn lettres(&self, id: IdLigne) -> Vec<Lettre> {
        self.lettrages
            .values()
            .filter(|l| !self.ruptures.contains_key(&l.lettre))
            .filter(|l| l.liens.iter().any(|k| k.debit == id || k.credit == id))
            .map(|l| l.lettre)
            .collect()
    }

    /// Vrai si le lettrage est non rompu et que toutes ses lignes sont soldées.
    #[must_use]
    pub fn est_total(&self, lettre: Lettre) -> bool {
        self.lettrages.get(&lettre).is_some_and(|l| {
            !self.ruptures.contains_key(&lettre)
                && l.lignes()
                    .iter()
                    .all(|id| self.residuel(*id) == Some(Decimal::ZERO))
        })
    }

    /// Lettre une sélection de lignes « au kilomètre » : débits et crédits sont imputés dans
    /// l'ordre de l'échéance effective, à défaut de la date comptable, puis du rang de chaîne,
    /// jusqu'à épuisement du côté le plus petit ; le reste demeure ouvert (ADR 0032, point 1).
    /// L'ordre de la sélection est indifférent.
    ///
    /// # Errors
    /// Sélection invalide (ligne inconnue, en double ou soldée, un seul sens, tiers différents,
    /// devise étrangère hors de L3) ou lettres épuisées.
    pub fn lettrer(
        &mut self,
        selection: &[IdLigne],
        date: Date,
        origine: Origine,
    ) -> Result<&Lettrage, ErreurLettrage> {
        let liens = self.imputer(selection)?;
        let lettre = self.prochaine_lettre()?;
        self.inscrire(lettre, date, origine, liens)
    }

    /// Calcule les liens de [`Self::lettrer`] sans rien inscrire.
    ///
    /// # Errors
    /// Comme [`Self::lettrer`].
    pub fn imputer(&self, selection: &[IdLigne]) -> Result<Vec<Lien>, ErreurLettrage> {
        let mut vues = BTreeSet::new();
        let (mut debits, mut credits) = (Vec::new(), Vec::new());
        for id in selection {
            if !vues.insert(*id) {
                return Err(ErreurLettrage::LigneEnDouble(*id));
            }
            let ligne = self
                .lignes
                .get(id)
                .ok_or(ErreurLettrage::LigneInconnue(*id))?;
            let residuel = self.residuel(*id).unwrap_or_default();
            if residuel <= Decimal::ZERO {
                return Err(ErreurLettrage::LigneSoldee(*id));
            }
            match ligne.sens {
                Sens::Debit => debits.push((ligne.cle(), residuel)),
                Sens::Credit => credits.push((ligne.cle(), residuel)),
            }
        }
        if debits.is_empty() {
            return Err(ErreurLettrage::SensManquant(Sens::Debit));
        }
        if credits.is_empty() {
            return Err(ErreurLettrage::SensManquant(Sens::Credit));
        }
        let liens = imputer_tries(debits, credits);
        self.controler(&liens)?;
        Ok(liens)
    }

    /// Inscrit un lettrage relu de la persistance, après contrôle des mêmes conditions. Les
    /// lettrages et les ruptures se relisent dans l'ordre où les opérations ont eu lieu.
    ///
    /// # Errors
    /// Lettre déjà employée, liens invalides (voir [`Self::lettrer`]).
    pub fn reprendre(
        &mut self,
        lettre: Lettre,
        date: Date,
        origine: Origine,
        liens: Vec<Lien>,
    ) -> Result<&Lettrage, ErreurLettrage> {
        self.inscrire(lettre, date, origine, liens)
    }

    /// Rompt un lettrage (dé-lettrage) : ajoute une rupture datée du jour de l'opération. Les
    /// liens ne comptent plus dans les résiduels ; re-lettrer crée un nouveau lettrage.
    ///
    /// # Errors
    /// [`ErreurLettrage::ExerciceClos`] si la date du lettrage tombe dans un exercice clos
    /// (B2, option 1) ; lettre inconnue, déjà rompue, date antérieure au lettrage.
    pub fn rompre(
        &mut self,
        lettre: Lettre,
        date: Date,
        motif: MotifRupture,
        exercices: &Exercices,
    ) -> Result<&Rupture, ErreurLettrage> {
        let lettrage = self
            .lettrages
            .get(&lettre)
            .ok_or(ErreurLettrage::LettreInconnue(lettre))?;
        if exercices
            .exercice_contenant(lettrage.date)
            .is_some_and(|e| e.statut() == StatutExercice::Cloture)
        {
            return Err(ErreurLettrage::ExerciceClos(lettre));
        }
        self.reprendre_rupture(lettre, date, motif)
    }

    /// Inscrit une rupture relue de la persistance (sans contrôle d'exercice : historique).
    ///
    /// # Errors
    /// Lettre inconnue, déjà rompue, date antérieure au lettrage.
    pub fn reprendre_rupture(
        &mut self,
        lettre: Lettre,
        date: Date,
        motif: MotifRupture,
    ) -> Result<&Rupture, ErreurLettrage> {
        let lettrage = self
            .lettrages
            .get(&lettre)
            .ok_or(ErreurLettrage::LettreInconnue(lettre))?;
        if self.ruptures.contains_key(&lettre) {
            return Err(ErreurLettrage::DejaRompu(lettre));
        }
        if date < lettrage.date {
            return Err(ErreurLettrage::RuptureAnterieure(lettre));
        }
        for lien in &lettrage.liens {
            for id in [lien.debit, lien.credit] {
                if let Some(impute) = self.impute.get_mut(&id) {
                    *impute -= lien.montant;
                }
            }
        }
        Ok(self.ruptures.entry(lettre).or_insert(Rupture {
            lettre,
            date,
            motif,
        }))
    }

    fn inscrire(
        &mut self,
        lettre: Lettre,
        date: Date,
        origine: Origine,
        liens: Vec<Lien>,
    ) -> Result<&Lettrage, ErreurLettrage> {
        if self.lettrages.contains_key(&lettre) {
            return Err(ErreurLettrage::LettreDejaEmployee(lettre));
        }
        let cumul = self.controler(&liens)?;
        for (id, montant) in cumul {
            *self.impute.entry(id).or_default() += montant;
        }
        Ok(self.lettrages.entry(lettre).or_insert(Lettrage {
            lettre,
            date,
            origine,
            liens,
        }))
    }

    /// Contrôle des liens d'un lettrage ; renvoie le montant imputé par ligne.
    fn controler(&self, liens: &[Lien]) -> Result<BTreeMap<IdLigne, Decimal>, ErreurLettrage> {
        if liens.is_empty() {
            return Err(ErreurLettrage::SansLien);
        }
        let mut cumul: BTreeMap<IdLigne, Decimal> = BTreeMap::new();
        let mut tiers = None;
        for lien in liens {
            if lien.montant <= Decimal::ZERO {
                return Err(ErreurLettrage::MontantNonPositif(lien.montant));
            }
            for (id, sens) in [(lien.debit, Sens::Debit), (lien.credit, Sens::Credit)] {
                let ligne = self
                    .lignes
                    .get(&id)
                    .ok_or(ErreurLettrage::LigneInconnue(id))?;
                if ligne.sens != sens {
                    return Err(ErreurLettrage::SensContraire(id));
                }
                match tiers {
                    None => tiers = Some(&ligne.tiers),
                    Some(t) if t != &ligne.tiers => return Err(ErreurLettrage::TiersDifferents),
                    Some(_) => {}
                }
                let total = cumul.entry(id).or_default();
                *total = total
                    .checked_add(lien.montant)
                    .ok_or(ErreurLettrage::Depassement)?;
                if *total > self.residuel(id).unwrap_or_default() {
                    return Err(ErreurLettrage::ResiduelDepasse(id));
                }
            }
        }
        self.controler_devise(&cumul)?;
        Ok(cumul)
    }

    /// Devise étrangère (ADR 0032, point 1) : admise seulement si toutes les lignes sont dans
    /// la même devise, entièrement lettrées par ce lettrage, et que les montants en devise
    /// s'équilibrent ; sinon l'écart de change relève de L6.
    fn controler_devise(&self, cumul: &BTreeMap<IdLigne, Decimal>) -> Result<(), ErreurLettrage> {
        let lignes: Vec<&LigneLettrable> =
            cumul.keys().filter_map(|id| self.lignes.get(id)).collect();
        if lignes.iter().all(|l| l.devise.is_none()) {
            return Ok(());
        }
        let mut solde: Option<Montant> = None;
        for ligne in lignes {
            let devise = ligne.devise.ok_or(ErreurLettrage::DeviseHorsL3)?;
            if self.impute.get(&ligne.id).is_some_and(|i| !i.is_zero())
                || cumul.get(&ligne.id) != Some(&ligne.montant)
            {
                return Err(ErreurLettrage::DeviseHorsL3);
            }
            let signe = match ligne.sens {
                Sens::Debit => devise,
                Sens::Credit => devise.oppose(),
            };
            solde = Some(match solde {
                None => signe,
                Some(s) => s
                    .ajouter(&signe)
                    .map_err(|_| ErreurLettrage::DeviseHorsL3)?,
            });
        }
        if solde.is_some_and(|s| !s.est_nul()) {
            return Err(ErreurLettrage::DeviseHorsL3);
        }
        Ok(())
    }
}

/// Imputation « au kilomètre » de deux côtés déjà résiduels : tri par clé, puis appariement
/// du plus ancien au plus récent jusqu'à épuisement d'un côté.
fn imputer_tries(
    mut debits: Vec<((Date, IdLigne), Decimal)>,
    mut credits: Vec<((Date, IdLigne), Decimal)>,
) -> Vec<Lien> {
    debits.sort_by_key(|(cle, _)| *cle);
    credits.sort_by_key(|(cle, _)| *cle);
    let mut liens = Vec::new();
    let (mut i, mut j) = (0, 0);
    while let (Some(debit), Some(credit)) = (debits.get(i), credits.get(j)) {
        let montant = debit.1.min(credit.1);
        liens.push(Lien {
            debit: (debit.0).1,
            credit: (credit.0).1,
            montant,
        });
        let (reste_d, reste_c) = (debit.1 - montant, credit.1 - montant);
        debits[i].1 = reste_d;
        credits[j].1 = reste_c;
        if reste_d.is_zero() {
            i += 1;
        }
        if reste_c.is_zero() {
            j += 1;
        }
    }
    liens
}

#[cfg(test)]
mod tests;
