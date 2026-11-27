//! Échéancier (spec §10.2 ; ADR 0033).
//!
//! - **Conditions de règlement** ([`ConditionReglement`]) : termes en pourcentage du total,
//!   montant fixe ou solde, le dernier terme étant toujours le solde ; répartition en décimal
//!   exact, sans centime perdu ; délais en jours, fin de mois ou jour du mois suivant ([`Delai`]).
//! - **Échéance d'une ligne de tiers** ([`EcheanceLigne`]) : échéance initiale copiée à la
//!   validation, puis **révisions en ajout seul** ; l'échéance **effective** à une date est
//!   celle de la dernière révision dont la date d'opération est antérieure ou égale, à défaut
//!   l'échéance initiale, à défaut la date comptable (Tryton `move.py:776`, ADR 0033 point 2).
//!   Une révision est refusée quand le résiduel de la ligne est nul.
//! - **Statut calculé**, jamais stocké : [`StatutReglement`] (ouverte, partiellement réglée,
//!   soldée) à partir du [`Residuel`] ; [`Situation`] (soldée, à échoir, échue) à une date.
//!
//! Le module est pur : les dates (pièce, opération, référence) et le résiduel (calculé par le
//! lettrage) sont des paramètres.

mod condition;
mod delai;

use rust_decimal::Decimal;
use time::Date;

use crate::montant::Devise;

pub use condition::{
    ConditionReglement, DECIMALES_POURCENTAGE_MAX, MontantFixe, Pourcentage, Terme, TermeCalcule,
    ValeurTerme,
};
pub use delai::{Delai, JourDuMois};

/// Longueur maximale d'un motif de révision, en octets (ADR 0033, point 2).
pub const MOTIF_OCTETS_MAX: usize = 400;

/// Erreurs de l'échéancier.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ErreurEcheance {
    /// Condition de règlement sans terme.
    #[error("condition de règlement sans terme")]
    ConditionSansTerme,
    /// Le dernier terme n'est pas le solde (Tryton `payment_term.py:40-47`).
    #[error("le dernier terme d'une condition de règlement doit être le solde")]
    DernierTermePasSolde,
    /// Terme « solde » ailleurs qu'en dernière position (rang à partir de 1).
    #[error("terme « solde » au rang {0}, avant le dernier terme")]
    SoldeAvantLaFin(usize),
    /// Somme des pourcentages supérieure ou égale à 100.
    #[error("la somme des pourcentages doit rester strictement inférieure à 100")]
    PourcentagesExcessifs,
    /// Pourcentage hors de ]0, 100[ ou trop précis.
    #[error("pourcentage invalide : {0}")]
    PourcentageInvalide(Decimal),
    /// Montant fixe nul ou négatif.
    #[error("montant fixe invalide : {0} (strictement positif attendu)")]
    MontantFixeInvalide(Decimal),
    /// Jour du mois au-delà de 31.
    #[error("jour du mois invalide : {0} (0 à 31 attendu)")]
    JourDuMoisInvalide(u8),
    /// Total à répartir dans une autre devise que la devise de tenue.
    #[error("le montant est en {0}, pas dans la devise de tenue")]
    PasEnDeviseTenue(Devise),
    /// Total à répartir plus précis que l'échelle de la devise de tenue.
    #[error("montant invalide : plus de {decimales} décimale(s) pour la devise de tenue")]
    EchelleExcessive {
        /// Échelle de la devise de tenue.
        decimales: u8,
    },
    /// Date calculée hors des bornes du calendrier.
    #[error("date d'échéance hors des limites du calendrier")]
    DateHorsLimites,
    /// Dépassement de capacité décimale ou du rang des révisions.
    #[error("dépassement de capacité")]
    Depassement,
    /// Résiduel négatif ou supérieur au montant de la ligne.
    #[error("résiduel {residuel} hors de [0, {montant}]")]
    ResiduelHorsBornes {
        /// Montant de la ligne (valeur absolue).
        montant: Decimal,
        /// Résiduel fourni.
        residuel: Decimal,
    },
    /// Révision refusée : le résiduel de la ligne est nul (ADR 0033, point 2 : 409
    /// `echeance_soldee`).
    #[error("échéance soldée : révision refusée")]
    EcheanceSoldee,
    /// Motif de révision vide, trop long ou contenant un caractère interdit.
    #[error("motif de révision invalide")]
    MotifInvalide,
    /// Rang de révision nul.
    #[error("rang de révision invalide : 0")]
    RangInvalide,
    /// Révisions dont les rangs ne croissent pas strictement ou dont les dates d'opération
    /// décroissent (une révision s'ajoute après les précédentes).
    #[error("révision {0} hors d'ordre (rang ou date d'opération)")]
    RevisionHorsOrdre(u32),
}

/// Résiduel d'une ligne de tiers : montant de la ligne et part non encore lettrée, en valeur
/// absolue, avec `0 ≤ résiduel ≤ montant`. Calculé par le lettrage (ADR 0032), jamais stocké.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Residuel {
    montant: Decimal,
    residuel: Decimal,
}

impl Residuel {
    /// Construit un résiduel.
    ///
    /// # Errors
    /// [`ErreurEcheance::ResiduelHorsBornes`] si le résiduel est négatif ou dépasse le montant.
    pub fn nouveau(montant: Decimal, residuel: Decimal) -> Result<Self, ErreurEcheance> {
        if residuel < Decimal::ZERO || residuel > montant {
            return Err(ErreurEcheance::ResiduelHorsBornes { montant, residuel });
        }
        Ok(Self { montant, residuel })
    }

    /// Montant de la ligne (valeur absolue).
    #[must_use]
    pub fn montant(&self) -> Decimal {
        self.montant
    }

    /// Part non lettrée.
    #[must_use]
    pub fn residuel(&self) -> Decimal {
        self.residuel
    }

    /// Statut de règlement (ADR 0033, point 2) : soldée si le résiduel est nul, partiellement
    /// réglée s'il est strictement entre 0 et le montant, ouverte sinon.
    #[must_use]
    pub fn statut(&self) -> StatutReglement {
        if self.residuel.is_zero() {
            StatutReglement::Soldee
        } else if self.residuel < self.montant {
            StatutReglement::PartiellementReglee
        } else {
            StatutReglement::Ouverte
        }
    }
}

/// Statut de règlement d'une échéance, calculé du résiduel (Odoo `account_move.py:49-57`,
/// Tryton `invoice.py:883-895`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StatutReglement {
    /// Aucun règlement lettré.
    Ouverte,
    /// Règlement partiel.
    PartiellementReglee,
    /// Résiduel nul.
    Soldee,
}

/// Situation d'une échéance à une date : le retard est la date moins l'échéance effective, en
/// jours ; non échu tant qu'il est nul ou négatif (ADR 0033, point 4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Situation {
    /// Résiduel nul.
    Soldee,
    /// Échéance effective postérieure ou égale à la date.
    AEchoir,
    /// Échéance effective dépassée.
    Echue {
        /// Retard en jours, strictement positif.
        retard_jours: i64,
    },
}

/// Motif d'une révision : non vide, au plus [`MOTIF_OCTETS_MAX`] octets, sans caractère de
/// contrôle ni de format (ADR 0033, point 2).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct MotifRevision(String);

impl MotifRevision {
    /// Valide un motif, conservé tel que saisi.
    ///
    /// # Errors
    /// [`ErreurEcheance::MotifInvalide`].
    pub fn parse(texte: &str) -> Result<Self, ErreurEcheance> {
        if texte.trim().is_empty()
            || texte.len() > MOTIF_OCTETS_MAX
            || texte.chars().any(caractere_interdit)
        {
            return Err(ErreurEcheance::MotifInvalide);
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
/// (`Cf`, Unicode 16.0) : même liste que `crates/api/src/pieces/mod.rs` (à factoriser dans
/// `texte.rs` avec le motif de rupture du lettrage).
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

/// Révision d'échéance (`echeance_report`) : ajoutée, jamais modifiée ni supprimée ; une
/// révision se corrige par une nouvelle.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Revision {
    rang: u32,
    nouvelle_echeance: Date,
    date_operation: Date,
    motif: MotifRevision,
}

impl Revision {
    /// Relit une révision stockée (rang à partir de 1, date d'opération : jour de l'opération
    /// dans le fuseau du dossier).
    ///
    /// # Errors
    /// [`ErreurEcheance::RangInvalide`] si le rang est nul.
    pub fn nouvelle(
        rang: u32,
        nouvelle_echeance: Date,
        date_operation: Date,
        motif: MotifRevision,
    ) -> Result<Self, ErreurEcheance> {
        if rang == 0 {
            return Err(ErreurEcheance::RangInvalide);
        }
        Ok(Self {
            rang,
            nouvelle_echeance,
            date_operation,
            motif,
        })
    }

    /// Rang de la révision sur la ligne.
    #[must_use]
    pub fn rang(&self) -> u32 {
        self.rang
    }

    /// Nouvelle échéance.
    #[must_use]
    pub fn nouvelle_echeance(&self) -> Date {
        self.nouvelle_echeance
    }

    /// Jour de l'opération.
    #[must_use]
    pub fn date_operation(&self) -> Date {
        self.date_operation
    }

    /// Motif.
    #[must_use]
    pub fn motif(&self) -> &MotifRevision {
        &self.motif
    }
}

/// Échéance d'une ligne de tiers validée : date comptable de l'écriture (repli), échéance
/// initiale (`echeance_ligne`, facultative) et révisions dans l'ordre de leur rang.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct EcheanceLigne {
    date_comptable: Date,
    initiale: Option<Date>,
    revisions: Vec<Revision>,
}

impl EcheanceLigne {
    /// Échéance d'une ligne sans révision.
    #[must_use]
    pub fn nouvelle(date_comptable: Date, initiale: Option<Date>) -> Self {
        Self {
            date_comptable,
            initiale,
            revisions: Vec::new(),
        }
    }

    /// Reconstitue l'échéance d'une ligne à partir de ses révisions stockées, dans n'importe
    /// quel ordre : elles sont triées par rang.
    ///
    /// # Errors
    /// [`ErreurEcheance::RevisionHorsOrdre`] si deux révisions ont le même rang ou si une
    /// révision a une date d'opération antérieure à celle d'une révision de rang inférieur.
    pub fn reconstituer(
        date_comptable: Date,
        initiale: Option<Date>,
        mut revisions: Vec<Revision>,
    ) -> Result<Self, ErreurEcheance> {
        revisions.sort_by_key(|r| r.rang);
        for paire in revisions.windows(2) {
            if let [avant, apres] = paire
                && (avant.rang == apres.rang || avant.date_operation > apres.date_operation)
            {
                return Err(ErreurEcheance::RevisionHorsOrdre(apres.rang));
            }
        }
        Ok(Self {
            date_comptable,
            initiale,
            revisions,
        })
    }

    /// Date comptable de l'écriture.
    #[must_use]
    pub fn date_comptable(&self) -> Date {
        self.date_comptable
    }

    /// Échéance initiale copiée à la validation.
    #[must_use]
    pub fn initiale(&self) -> Option<Date> {
        self.initiale
    }

    /// Révisions, par rang croissant.
    #[must_use]
    pub fn revisions(&self) -> &[Revision] {
        &self.revisions
    }

    /// Échéance effective à la date `date` : nouvelle échéance de la dernière révision dont la
    /// date d'opération est antérieure ou égale, à défaut l'échéance initiale, à défaut la date
    /// comptable. Ne dépend d'aucune révision postérieure à `date`.
    #[must_use]
    pub fn effective(&self, date: Date) -> Date {
        self.revisions
            .iter()
            .rev()
            .find(|r| r.date_operation <= date)
            .map(|r| r.nouvelle_echeance)
            .or(self.initiale)
            .unwrap_or(self.date_comptable)
    }

    /// Situation de l'échéance à la date `date`, d'après le résiduel de la ligne à cette date.
    #[must_use]
    pub fn situation(&self, date: Date, residuel: &Residuel) -> Situation {
        if residuel.residuel.is_zero() {
            return Situation::Soldee;
        }
        let retard_jours = (date - self.effective(date)).whole_days();
        if retard_jours > 0 {
            Situation::Echue { retard_jours }
        } else {
            Situation::AEchoir
        }
    }

    /// Ajoute une révision, permise tant que le résiduel de la ligne n'est pas nul (ADR 0033,
    /// point 2). Renvoie la révision à enregistrer.
    ///
    /// # Errors
    /// [`ErreurEcheance::EcheanceSoldee`] si le résiduel est nul ;
    /// [`ErreurEcheance::RevisionHorsOrdre`] si la date d'opération précède celle de la
    /// dernière révision ; [`ErreurEcheance::Depassement`] si le rang dépasse sa capacité.
    pub fn reviser(
        &mut self,
        nouvelle_echeance: Date,
        date_operation: Date,
        motif: MotifRevision,
        residuel: &Residuel,
    ) -> Result<&Revision, ErreurEcheance> {
        if residuel.residuel.is_zero() {
            return Err(ErreurEcheance::EcheanceSoldee);
        }
        let rang = match self.revisions.last() {
            None => 1,
            Some(derniere) => {
                let rang = derniere
                    .rang
                    .checked_add(1)
                    .ok_or(ErreurEcheance::Depassement)?;
                if date_operation < derniere.date_operation {
                    return Err(ErreurEcheance::RevisionHorsOrdre(rang));
                }
                rang
            }
        };
        self.revisions.push(Revision {
            rang,
            nouvelle_echeance,
            date_operation,
            motif,
        });
        self.revisions.last().ok_or(ErreurEcheance::Depassement)
    }
}

#[cfg(test)]
mod tests;
