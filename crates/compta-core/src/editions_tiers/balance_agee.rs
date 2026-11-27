//! Balance âgée à une date (spec §10.3 ; ADR 0033, point 4 ; plan L3, A12 et C8).
//!
//! - **Lignes prises en compte** : lignes de tiers de date comptable au plus tard `D`, selon le
//!   mode (définitif ou provisoire) ; Tryton ignore son option `posted` (`account.py:2900`),
//!   Sigma applique le mode.
//! - **Solde ouvert** d'une ligne : son résiduel à `D` ([`Lettrages::residuel_a`] : liens non
//!   rompus dont les deux lignes sont datées au plus tard `D`) ; une ligne soldée à `D`
//!   n'apparaît pas (Tryton `account.py:3045-3047` : lignes non lettrées ou dont le lettrage,
//!   daté par la plus haute date des lignes, est postérieur à la date).
//! - **Retard** = `D` moins l'échéance effective à `D`, en jours (Tryton
//!   `Coalesce(maturity_date, move.date)`, `account.py:3020`). Non échu : retard nul ou
//!   négatif ; puis une tranche par borne, `1 à b1`, `b1+1 à b2`… et « plus de » la dernière
//!   borne (OCA `aged_partner_balance.py:55-74` : `today <= due_date + 30 jours`).
//! - **Bornes** paramétrables, dans l'ordre croissant (égalités admises), au plus
//!   [`BORNES_MAX`] ; par défaut [`BORNES_PAR_DEFAUT`] (Tryton `account.py:2937-2938`). Une
//!   borne nulle ou répétée donne une tranche vide, comme chez Tryton et OCA, qui ne contrôlent
//!   pas les bornes ; sans borne, deux colonnes : non échu et échu.
//! - **Aucune compensation** (A'1) : dans chaque tranche, débits (créances) et crédits (avoirs,
//!   acomptes) restent séparés ; le solde d'un tiers est net, mais les totaux séparent les
//!   soldes débiteurs et créditeurs des tiers.

use std::collections::BTreeMap;

use rust_decimal::Decimal;
use time::Date;

use super::{
    ErreurEditionTiers, Lettrages, LigneTiers, SourceLigne, controler_doublons, difference, somme,
};
use crate::compte::NumeroCompte;
use crate::ecriture::{IdTiers, Sens};
use crate::editions::Mode;

/// Nombre maximal de bornes de tranches (ADR 0033, point 4).
pub const BORNES_MAX: usize = 4;

/// Bornes par défaut, en jours (Tryton `account.py:2937-2938`, OCA).
pub const BORNES_PAR_DEFAUT: [u32; 3] = [30, 60, 90];

/// Tranche de retard.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Tranche {
    /// Retard nul ou négatif : échéance non atteinte, ou atteinte le jour même.
    NonEchu,
    /// Retard de `de` à `a` jours inclus (vide si `a < de`) ; sans `a`, plus de `de - 1` jours.
    Retard {
        /// Premier jour de retard de la tranche (au moins 1).
        de: u32,
        /// Dernier jour de retard de la tranche ; `None` pour la dernière.
        a: Option<u32>,
    },
}

/// Bornes des tranches de retard, en jours : dans l'ordre croissant (égalités admises), au plus
/// [`BORNES_MAX`]. `n` bornes donnent `n + 2` colonnes (non échu, `n` tranches bornées, plus
/// de la dernière borne) ; aucune borne : non échu et échu.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Tranches {
    bornes: Vec<u32>,
}

impl Default for Tranches {
    fn default() -> Self {
        Self {
            bornes: BORNES_PAR_DEFAUT.to_vec(),
        }
    }
}

impl Tranches {
    /// Valide des bornes (ADR 0033, point 4 : croissantes, au plus quatre). Une borne nulle ou
    /// répétée est admise et donne une tranche vide.
    ///
    /// # Errors
    /// [`ErreurEditionTiers::BornesInvalides`] : plus de [`BORNES_MAX`] bornes, ou bornes
    /// décroissantes.
    pub fn nouvelles(bornes: &[u32]) -> Result<Self, ErreurEditionTiers> {
        let croissantes = bornes.windows(2).all(|p| matches!(p, [a, b] if a <= b));
        if bornes.len() > BORNES_MAX || !croissantes {
            return Err(ErreurEditionTiers::BornesInvalides);
        }
        Ok(Self {
            bornes: bornes.to_vec(),
        })
    }

    /// Bornes, en jours.
    #[must_use]
    pub fn bornes(&self) -> &[u32] {
        &self.bornes
    }

    /// Colonnes, dans l'ordre : non échu, tranches bornées, plus de la dernière borne.
    #[must_use]
    pub fn colonnes(&self) -> Vec<Tranche> {
        let mut colonnes = vec![Tranche::NonEchu];
        let mut de = 1;
        for borne in &self.bornes {
            colonnes.push(Tranche::Retard {
                de,
                a: Some(*borne),
            });
            de = borne.saturating_add(1);
        }
        colonnes.push(Tranche::Retard { de, a: None });
        colonnes
    }

    /// Indice de la colonne d'un retard en jours : 0 si non échu, sinon 1 plus le nombre de
    /// bornes strictement dépassées.
    #[must_use]
    pub fn indice(&self, retard_jours: i64) -> usize {
        if retard_jours <= 0 {
            return 0;
        }
        1 + self
            .bornes
            .iter()
            .filter(|b| retard_jours > i64::from(**b))
            .count()
    }
}

/// Montants d'une tranche : résiduels au débit et au crédit, jamais compensés.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ColonneAgee {
    debit: Decimal,
    credit: Decimal,
}

impl ColonneAgee {
    /// Résiduels au débit (créances).
    #[must_use]
    pub fn debit(&self) -> Decimal {
        self.debit
    }

    /// Résiduels au crédit (avoirs, acomptes, règlements non lettrés).
    #[must_use]
    pub fn credit(&self) -> Decimal {
        self.credit
    }

    /// Solde signé (débit − crédit).
    ///
    /// # Errors
    /// Dépassement de capacité.
    pub fn solde(&self) -> Result<Decimal, ErreurEditionTiers> {
        difference(self.debit, self.credit)
    }

    fn ajouter(&mut self, sens: Sens, montant: Decimal) -> Result<(), ErreurEditionTiers> {
        match sens {
            Sens::Debit => self.debit = somme(self.debit, montant)?,
            Sens::Credit => self.credit = somme(self.credit, montant)?,
        }
        Ok(())
    }

    fn cumuler(&mut self, autre: &Self) -> Result<(), ErreurEditionTiers> {
        self.debit = somme(self.debit, autre.debit)?;
        self.credit = somme(self.credit, autre.credit)?;
        Ok(())
    }
}

/// Ligne ouverte à la date, avec son retard.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LigneAgee {
    source: SourceLigne,
    date: Date,
    echeance: Date,
    retard_jours: i64,
    tranche: usize,
    sens: Sens,
    montant: Decimal,
    residuel: Decimal,
}

impl LigneAgee {
    /// Provenance.
    #[must_use]
    pub fn source(&self) -> &SourceLigne {
        &self.source
    }

    /// Date comptable.
    #[must_use]
    pub fn date(&self) -> Date {
        self.date
    }

    /// Échéance effective à la date de la balance.
    #[must_use]
    pub fn echeance(&self) -> Date {
        self.echeance
    }

    /// Retard en jours (nul ou négatif : non échu).
    #[must_use]
    pub fn retard_jours(&self) -> i64 {
        self.retard_jours
    }

    /// Indice de la colonne ([`Tranches::colonnes`]).
    #[must_use]
    pub fn tranche(&self) -> usize {
        self.tranche
    }

    /// Sens.
    #[must_use]
    pub fn sens(&self) -> Sens {
        self.sens
    }

    /// Montant de la ligne.
    #[must_use]
    pub fn montant(&self) -> Decimal {
        self.montant
    }

    /// Résiduel à la date, strictement positif.
    #[must_use]
    pub fn residuel(&self) -> Decimal {
        self.residuel
    }
}

/// Tiers d'un compte collectif dans la balance âgée.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TiersAgee {
    tiers: IdTiers,
    colonnes: Vec<ColonneAgee>,
    lignes: Vec<LigneAgee>,
    solde: Decimal,
}

impl TiersAgee {
    /// Tiers résolu.
    #[must_use]
    pub fn tiers(&self) -> &IdTiers {
        &self.tiers
    }

    /// Montants par colonne ([`Tranches::colonnes`]).
    #[must_use]
    pub fn colonnes(&self) -> &[ColonneAgee] {
        &self.colonnes
    }

    /// Lignes ouvertes, par date puis numéro.
    #[must_use]
    pub fn lignes(&self) -> &[LigneAgee] {
        &self.lignes
    }

    /// Solde ouvert signé du tiers (débit − crédit) : somme des colonnes.
    #[must_use]
    pub fn solde(&self) -> Decimal {
        self.solde
    }
}

/// Totaux sans compensation entre tiers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TotalAgee {
    colonnes: Vec<ColonneAgee>,
    soldes_debiteurs: Decimal,
    soldes_crediteurs: Decimal,
}

impl TotalAgee {
    fn nouveau(largeur: usize) -> Self {
        Self {
            colonnes: vec![ColonneAgee::default(); largeur],
            soldes_debiteurs: Decimal::ZERO,
            soldes_crediteurs: Decimal::ZERO,
        }
    }

    /// Montants par colonne : débits et crédits cumulés séparément.
    #[must_use]
    pub fn colonnes(&self) -> &[ColonneAgee] {
        &self.colonnes
    }

    /// Somme des soldes des tiers débiteurs.
    #[must_use]
    pub fn soldes_debiteurs(&self) -> Decimal {
        self.soldes_debiteurs
    }

    /// Somme des soldes des tiers créditeurs, en valeur absolue.
    #[must_use]
    pub fn soldes_crediteurs(&self) -> Decimal {
        self.soldes_crediteurs
    }

    fn cumuler_tiers(&mut self, tiers: &TiersAgee) -> Result<(), ErreurEditionTiers> {
        for (total, colonne) in self.colonnes.iter_mut().zip(&tiers.colonnes) {
            total.cumuler(colonne)?;
        }
        if tiers.solde > Decimal::ZERO {
            self.soldes_debiteurs = somme(self.soldes_debiteurs, tiers.solde)?;
        } else {
            self.soldes_crediteurs = difference(self.soldes_crediteurs, tiers.solde)?;
        }
        Ok(())
    }

    fn cumuler(&mut self, autre: &Self) -> Result<(), ErreurEditionTiers> {
        for (total, colonne) in self.colonnes.iter_mut().zip(&autre.colonnes) {
            total.cumuler(colonne)?;
        }
        self.soldes_debiteurs = somme(self.soldes_debiteurs, autre.soldes_debiteurs)?;
        self.soldes_crediteurs = somme(self.soldes_crediteurs, autre.soldes_crediteurs)?;
        Ok(())
    }
}

/// Compte collectif de la balance âgée.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompteAgee {
    compte: NumeroCompte,
    tiers: Vec<TiersAgee>,
    total: TotalAgee,
}

impl CompteAgee {
    /// Compte collectif.
    #[must_use]
    pub fn compte(&self) -> &NumeroCompte {
        &self.compte
    }

    /// Tiers ayant au moins une ligne ouverte, triés.
    #[must_use]
    pub fn tiers(&self) -> &[TiersAgee] {
        &self.tiers
    }

    /// Totaux du compte.
    #[must_use]
    pub fn total(&self) -> &TotalAgee {
        &self.total
    }
}

/// Balance âgée à une date.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BalanceAgee {
    date: Date,
    tranches: Tranches,
    comptes: Vec<CompteAgee>,
    total: TotalAgee,
}

impl BalanceAgee {
    /// Date de la balance.
    #[must_use]
    pub fn date(&self) -> Date {
        self.date
    }

    /// Tranches.
    #[must_use]
    pub fn tranches(&self) -> &Tranches {
        &self.tranches
    }

    /// Comptes collectifs, triés.
    #[must_use]
    pub fn comptes(&self) -> &[CompteAgee] {
        &self.comptes
    }

    /// Totaux généraux.
    #[must_use]
    pub fn total(&self) -> &TotalAgee {
        &self.total
    }
}

/// Ligne ouverte et sa source, avant tri.
type Ouverte<'a> = (&'a LigneTiers, LigneAgee);

/// Balance âgée à la date `date`, par compte collectif puis par tiers résolu.
///
/// `lignes` : lignes de tiers d'**un seul dossier**, chacune une fois, dans un ordre
/// quelconque ; celles de date postérieure sont ignorées.
///
/// **Contrat de l'appelant (PR 10)** : toutes les lignes fournies sont additionnées, quel que
/// soit leur exercice. Fournir à la fois les lignes de l'exercice N et les **à-nouveaux de
/// N+1** qui les reprennent (détaillés par tiers, spec §15.3) compte deux fois les mêmes
/// soldes : pour une date de N+1, l'appelant passe les à-nouveaux de N+1 et les lignes de N+1
/// (pas celles de N) ; pour une date de N, les lignes de N et de ses exercices antérieurs
/// selon la même règle.
///
/// # Errors
/// Ligne en double, lettrage incohérent, dépassement de capacité.
pub fn balance_agee(
    lignes: &[LigneTiers],
    lettrages: &Lettrages,
    mode: Mode,
    date: Date,
    tranches: &Tranches,
) -> Result<BalanceAgee, ErreurEditionTiers> {
    controler_doublons(lignes)?;
    let largeur = tranches.bornes.len() + 2;
    let mut ouvertes: BTreeMap<NumeroCompte, BTreeMap<IdTiers, Vec<Ouverte<'_>>>> = BTreeMap::new();
    for ligne in lignes {
        if !ligne.retenue(mode) || ligne.date > date {
            continue;
        }
        let residuel = lettrages.residuel_a(ligne, date)?.residuel();
        if residuel.is_zero() {
            continue;
        }
        let echeance = ligne.echeance.effective(date);
        let retard_jours = (date - echeance).whole_days();
        ouvertes
            .entry(ligne.compte.clone())
            .or_default()
            .entry(ligne.tiers.clone())
            .or_default()
            .push((
                ligne,
                LigneAgee {
                    source: ligne.source.clone(),
                    date: ligne.date,
                    echeance,
                    retard_jours,
                    tranche: tranches.indice(retard_jours),
                    sens: ligne.sens,
                    montant: ligne.montant,
                    residuel,
                },
            ));
    }
    let mut comptes = Vec::with_capacity(ouvertes.len());
    let mut total = TotalAgee::nouveau(largeur);
    for (compte, par_tiers) in ouvertes {
        let mut total_compte = TotalAgee::nouveau(largeur);
        let mut tiers_compte = Vec::with_capacity(par_tiers.len());
        for (tiers, mut lignes_tiers) in par_tiers {
            lignes_tiers.sort_by(|a, b| a.0.cle_tri().cmp(&b.0.cle_tri()));
            let mut colonnes = vec![ColonneAgee::default(); largeur];
            let mut solde = Decimal::ZERO;
            for (ligne, agee) in &lignes_tiers {
                if let Some(colonne) = colonnes.get_mut(agee.tranche) {
                    colonne.ajouter(agee.sens, agee.residuel)?;
                }
                solde = somme(solde, ligne.signer(agee.residuel))?;
            }
            let tiers = TiersAgee {
                tiers,
                colonnes,
                lignes: lignes_tiers.into_iter().map(|(_, agee)| agee).collect(),
                solde,
            };
            total_compte.cumuler_tiers(&tiers)?;
            tiers_compte.push(tiers);
        }
        total.cumuler(&total_compte)?;
        comptes.push(CompteAgee {
            compte,
            tiers: tiers_compte,
            total: total_compte,
        });
    }
    Ok(BalanceAgee {
        date,
        tranches: tranches.clone(),
        comptes,
        total,
    })
}
