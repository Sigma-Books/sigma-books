//! Centralisateur des journaux (spec §17.1) : totaux de chaque journal par mois, reportés
//! dans le livre-journal, et leur ventilation par compte, reportée dans le grand livre.
//!
//! **Fondement.** AUDCIF art. 19, dernier alinéa (p. PDF 23) : les journaux et livres
//! auxiliaires sont facultatifs ; quand ils sont tenus, leurs totaux sont « périodiquement et
//! au moins une fois par mois » centralisés dans le livre-journal et dans le grand livre. Ni
//! l'AUDCIF, ni le Guide d'application, ni les avis ne fixent la ventilation exacte, un report
//! cumulé ou un modèle d'état : ce module calcule les totaux mensuels par journal (livre-journal)
//! et par journal et compte (grand livre), sans cumul d'un mois sur l'autre. L'égalité du total
//! centralisé et des mouvements de la balance est une conséquence logique de la partie double,
//! pas une exigence du texte ; elle est vérifiée par les tests de propriétés. La numérotation
//! et la datation des états (art. 22 7°, p. PDF 24) relèvent du rendu (lot L7).
//!
//! **Moteurs de référence.** Aucun n'a de vrai centralisateur journal × mois ; OCA
//! `account_financial_report` regroupe par journal et par numéro de mois **sans l'année**, ce
//! qui fusionne deux janviers d'un exercice à cheval sur deux années civiles. Ici, la clé est
//! le mois civil **avec son année** ([`MoisCivil`]).
//!
//! **Règles.**
//! - Mouvements : écritures des journaux autres que d'à-nouveaux, datées dans `[du, au]` du
//!   [`Filtre`] ; une ligne par (mois civil, journal), triée par mois puis par code de journal
//!   (ordre du livre-journal), avec sous-totaux par mois et total général. Quand les bornes ne
//!   tombent pas sur un début ou une fin de mois, la première et la dernière ligne ne couvrent
//!   qu'une partie du mois : les bornes sont rendues par [`Centralisateur::du`] et
//!   [`Centralisateur::au`].
//! - **À-nouveaux** : les écritures des journaux d'à-nouveaux datées dans `[du, au]` forment
//!   une rubrique distincte ; elles ne sont jamais mêlées aux mouvements des mois ni comptées
//!   dans leur total. Comme l'édition des journaux, la rubrique ne retient que les à-nouveaux
//!   datés dans les bornes, alors que la balance range dans sa colonne d'ouverture tous ceux
//!   datés du début de l'exercice à `au` : les deux ne coïncident que pour un filtre qui part
//!   du début de l'exercice ([`Filtre::exercice`]). Un centralisateur de mars ne reprend donc
//!   pas les à-nouveaux de janvier, déjà centralisés dans le mois de janvier.
//! - Le filtre de journaux s'applique ; le filtre de comptes **ne s'applique pas** : comme dans
//!   l'édition des journaux, une écriture est centralisée en entier.
//! - [`Mode`] : définitif (écritures validées) ou provisoire (avec le brouillard équilibré).
//!
//! Tous les calculs sont exacts ; un dépassement de capacité est une erreur, jamais une panique.

use std::collections::BTreeMap;

use rust_decimal::Decimal;
use time::{Date, Month};

use crate::compte::NumeroCompte;
use crate::ecriture::{EcritureEquilibree, Ligne};
use crate::editions::{ErreurEdition, Filtre, Mode};
use crate::journal::CodeJournal;
use crate::livre::EcritureValidee;

/// Mois civil, avec son année : deux janviers d'un même exercice restent distincts. L'ordre
/// est chronologique.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct MoisCivil {
    // Le premier jour détermine le dernier : l'ordre dérivé est chronologique.
    premier_jour: Date,
    dernier_jour: Date,
}

impl MoisCivil {
    /// Mois civil donné par son année et son mois ; `None` si l'un de ses jours sort de la
    /// plage des dates représentables (seule cause d'échec).
    #[must_use]
    pub fn nouveau(annee: i32, mois: Month) -> Option<Self> {
        let premier_jour = Date::from_calendar_date(annee, mois, 1).ok()?;
        let dernier_jour = Date::from_calendar_date(annee, mois, mois.length(annee)).ok()?;
        Some(Self {
            premier_jour,
            dernier_jour,
        })
    }

    /// Mois civil contenant la date.
    #[must_use]
    pub fn de(date: Date) -> Self {
        // Le premier et le dernier jour du mois d'une date valide sont toujours représentables :
        // le repli sur `date` ne se produit pas.
        Self {
            premier_jour: date.replace_day(1).unwrap_or(date),
            dernier_jour: date
                .replace_day(date.month().length(date.year()))
                .unwrap_or(date),
        }
    }

    /// Année civile.
    #[must_use]
    pub fn annee(&self) -> i32 {
        self.premier_jour.year()
    }

    /// Mois.
    #[must_use]
    pub fn mois(&self) -> Month {
        self.premier_jour.month()
    }

    /// Premier jour du mois.
    #[must_use]
    pub fn premier_jour(&self) -> Date {
        self.premier_jour
    }

    /// Dernier jour du mois.
    #[must_use]
    pub fn dernier_jour(&self) -> Date {
        self.dernier_jour
    }
}

/// Totaux d'un journal pour un mois : ce qui est reporté dans le livre-journal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LigneCentralisateur {
    /// Mois civil.
    pub mois: MoisCivil,
    /// Journal.
    pub journal: CodeJournal,
    /// Total des débits.
    pub debit: Decimal,
    /// Total des crédits (égal au total des débits).
    pub credit: Decimal,
}

/// Totaux d'un compte dans un journal pour un mois : ce qui est reporté dans le grand livre.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LigneCentralisateurCompte {
    /// Mois civil.
    pub mois: MoisCivil,
    /// Journal.
    pub journal: CodeJournal,
    /// Compte (tel que saisi : auxiliaire compris).
    pub compte: NumeroCompte,
    /// Total des débits.
    pub debit: Decimal,
    /// Total des crédits.
    pub credit: Decimal,
}

/// Sous-total d'un mois, tous journaux confondus.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TotalMois {
    /// Mois civil.
    pub mois: MoisCivil,
    /// Total des débits.
    pub debit: Decimal,
    /// Total des crédits (égal au total des débits).
    pub credit: Decimal,
}

/// Rubrique du centralisateur : lignes par (mois, journal), détail par compte, sous-totaux par
/// mois et total.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RubriqueCentralisateur {
    lignes: Vec<LigneCentralisateur>,
    lignes_par_compte: Vec<LigneCentralisateurCompte>,
    totaux_par_mois: Vec<TotalMois>,
    total_debit: Decimal,
    total_credit: Decimal,
}

impl RubriqueCentralisateur {
    /// Une ligne par (mois, journal) mouvementé, triée par mois puis par journal.
    #[must_use]
    pub fn lignes(&self) -> &[LigneCentralisateur] {
        &self.lignes
    }

    /// Détail par compte de chaque ligne, trié par mois, journal, puis compte.
    #[must_use]
    pub fn lignes_par_compte(&self) -> &[LigneCentralisateurCompte] {
        &self.lignes_par_compte
    }

    /// Sous-totaux par mois, tous journaux confondus, dans l'ordre chronologique.
    #[must_use]
    pub fn totaux_par_mois(&self) -> &[TotalMois] {
        &self.totaux_par_mois
    }

    /// Total (débit, crédit) de la rubrique.
    #[must_use]
    pub fn total(&self) -> (Decimal, Decimal) {
        (self.total_debit, self.total_credit)
    }
}

/// Centralisateur des journaux : mouvements des mois et, à part, les à-nouveaux.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Centralisateur {
    du: Date,
    au: Date,
    mouvements: RubriqueCentralisateur,
    a_nouveaux: RubriqueCentralisateur,
}

fn somme(a: Decimal, b: Decimal) -> Result<Decimal, ErreurEdition> {
    a.checked_add(b).ok_or(ErreurEdition::Depassement)
}

/// Accumulateur d'une rubrique.
#[derive(Default)]
struct Cumul {
    journaux: BTreeMap<(MoisCivil, CodeJournal), (Decimal, Decimal)>,
    comptes: BTreeMap<(MoisCivil, CodeJournal, NumeroCompte), (Decimal, Decimal)>,
}

impl Cumul {
    fn ajouter(
        &mut self,
        journal: &CodeJournal,
        date: Date,
        lignes: &[Ligne],
    ) -> Result<(), ErreurEdition> {
        let mois = MoisCivil::de(date);
        for ligne in lignes {
            let total = self
                .journaux
                .entry((mois, journal.clone()))
                .or_insert((Decimal::ZERO, Decimal::ZERO));
            total.0 = somme(total.0, ligne.debit())?;
            total.1 = somme(total.1, ligne.credit())?;
            let detail = self
                .comptes
                .entry((mois, journal.clone(), ligne.compte().clone()))
                .or_insert((Decimal::ZERO, Decimal::ZERO));
            detail.0 = somme(detail.0, ligne.debit())?;
            detail.1 = somme(detail.1, ligne.credit())?;
        }
        Ok(())
    }

    fn rubrique(self) -> Result<RubriqueCentralisateur, ErreurEdition> {
        let mut par_mois: BTreeMap<MoisCivil, (Decimal, Decimal)> = BTreeMap::new();
        let (mut total_debit, mut total_credit) = (Decimal::ZERO, Decimal::ZERO);
        let mut lignes = Vec::with_capacity(self.journaux.len());
        for ((mois, journal), (debit, credit)) in self.journaux {
            let sous_total = par_mois
                .entry(mois)
                .or_insert((Decimal::ZERO, Decimal::ZERO));
            sous_total.0 = somme(sous_total.0, debit)?;
            sous_total.1 = somme(sous_total.1, credit)?;
            total_debit = somme(total_debit, debit)?;
            total_credit = somme(total_credit, credit)?;
            lignes.push(LigneCentralisateur {
                mois,
                journal,
                debit,
                credit,
            });
        }
        Ok(RubriqueCentralisateur {
            lignes,
            lignes_par_compte: self
                .comptes
                .into_iter()
                .map(
                    |((mois, journal, compte), (debit, credit))| LigneCentralisateurCompte {
                        mois,
                        journal,
                        compte,
                        debit,
                        credit,
                    },
                )
                .collect(),
            totaux_par_mois: par_mois
                .into_iter()
                .map(|(mois, (debit, credit))| TotalMois {
                    mois,
                    debit,
                    credit,
                })
                .collect(),
            total_debit,
            total_credit,
        })
    }
}

impl Centralisateur {
    /// Centralise les écritures validées et, en mode provisoire, le brouillard équilibré, sur
    /// la période et les journaux du filtre (voir la documentation du module, notamment pour
    /// les à-nouveaux et les mois partiels aux bornes).
    ///
    /// # Errors
    /// Dépassement de capacité.
    pub fn calculer(
        validees: &[EcritureValidee],
        brouillard: &[EcritureEquilibree],
        mode: Mode,
        filtre: &Filtre,
    ) -> Result<Self, ErreurEdition> {
        let provisoires: &[EcritureEquilibree] = match mode {
            Mode::Definitif => &[],
            Mode::Provisoire => brouillard,
        };
        let ecritures = validees
            .iter()
            .map(|e| (e.journal(), e.date(), e.lignes()))
            .chain(
                provisoires
                    .iter()
                    .map(|e| (e.journal(), e.date(), e.lignes())),
            );
        let (mut mouvements, mut a_nouveaux) = (Cumul::default(), Cumul::default());
        for (journal, date, lignes) in ecritures {
            if !filtre.dans_bornes(date) || !filtre.retient_journal(journal) {
                continue;
            }
            let cumul = if filtre.est_a_nouveaux(journal) {
                &mut a_nouveaux
            } else {
                &mut mouvements
            };
            cumul.ajouter(journal, date, lignes)?;
        }
        let (du, au) = filtre.bornes();
        Ok(Self {
            du,
            au,
            mouvements: mouvements.rubrique()?,
            a_nouveaux: a_nouveaux.rubrique()?,
        })
    }

    /// Premier jour centralisé : la première ligne peut ne couvrir qu'une partie de son mois.
    #[must_use]
    pub fn du(&self) -> Date {
        self.du
    }

    /// Dernier jour centralisé : la dernière ligne peut ne couvrir qu'une partie de son mois.
    #[must_use]
    pub fn au(&self) -> Date {
        self.au
    }

    /// Mouvements des mois, hors à-nouveaux.
    #[must_use]
    pub fn mouvements(&self) -> &RubriqueCentralisateur {
        &self.mouvements
    }

    /// Rubrique des à-nouveaux datés dans `[du, au]`, distincte des mouvements. Elle égale la
    /// part à-nouveaux de la colonne d'ouverture de la balance seulement si le filtre part du
    /// début de l'exercice ([`Filtre::exercice`]).
    #[must_use]
    pub fn a_nouveaux(&self) -> &RubriqueCentralisateur {
        &self.a_nouveaux
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::date;

    #[test]
    fn mois_civil_bornes_et_ordre() {
        let fevrier = MoisCivil::nouveau(2028, Month::February).unwrap();
        assert_eq!(
            (fevrier.premier_jour(), fevrier.dernier_jour()),
            (date!(2028 - 02 - 01), date!(2028 - 02 - 29))
        );
        assert_eq!(
            MoisCivil::nouveau(2026, Month::February)
                .unwrap()
                .dernier_jour(),
            date!(2026 - 02 - 28)
        );
        assert_eq!(
            MoisCivil::nouveau(2100, Month::February)
                .unwrap()
                .dernier_jour(),
            date!(2100 - 02 - 28)
        );
        assert_eq!(MoisCivil::de(date!(2028 - 02 - 17)), fevrier);
        assert_eq!((fevrier.annee(), fevrier.mois()), (2028, Month::February));
        assert_eq!(
            MoisCivil::de(date!(2026 - 12 - 31)).dernier_jour(),
            date!(2026 - 12 - 31)
        );
        // Hors de la plage représentable.
        assert_eq!(MoisCivil::nouveau(10_000, Month::January), None);
        // Ordre chronologique : l'année avant le mois.
        assert!(MoisCivil::de(date!(2026 - 12 - 01)) < MoisCivil::de(date!(2027 - 01 - 31)));
        assert!(MoisCivil::de(date!(2026 - 08 - 01)) < MoisCivil::de(date!(2027 - 07 - 01)));
    }
}
