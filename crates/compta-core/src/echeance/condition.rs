//! Conditions de règlement et répartition d'un total en termes (ADR 0033, point 3).
//!
//! - Valeur d'un terme : pourcentage **du total**, montant fixe ou **solde**. Le **dernier
//!   terme est toujours le solde**, et lui seul (Tryton `payment_term.py:40-47`, Odoo
//!   `account_payment_term.py:226-230`) ; la somme des pourcentages est strictement inférieure
//!   à 100.
//! - Calcul en décimal exact : chaque terme est arrondi à l'échelle de la devise de tenue,
//!   demi éloigné de zéro ([`DeviseTenue::arrondir`]), et le solde absorbe l'arrondi : la somme
//!   des termes est **exactement** le total. Un terme qui dépasse le reste est plafonné au
//!   reste ; un terme nul est omis (Tryton `:65-70`). Le signe du total est reporté sur chaque
//!   terme (avoir). Un total nul donne un seul terme nul daté de la référence (Tryton `:72-74`).
//! - Le résultat est trié par date (tri stable : à date égale, l'ordre des termes).

// Portions portées de Tryton (modules/account_invoice/payment_term.py, `PaymentTerm.compute`,
// commit f5354ccd9231703470611d7ebf1c9cdc65e952bc), GPL-3.0-or-later, © 2008-2026 Cédric
// Krier, © 2008-2026 B2CK SRL et autres titulaires du fichier COPYRIGHT.
// Voir docs/provenance/tryton.md#payment-term.

use rust_decimal::Decimal;
use time::Date;

use super::ErreurEcheance;
use super::delai::Delai;
use crate::montant::{DeviseTenue, Montant};

/// Nombre maximal de décimales d'un pourcentage (Odoo : précision « Payment Terms » à 6
/// chiffres, `data/account_data.xml:5-8` ; Tryton : ratio à 8 décimales, soit 6 en pourcentage).
pub const DECIMALES_POURCENTAGE_MAX: u32 = 6;

/// Pourcentage du total : strictement entre 0 et 100, au plus
/// [`DECIMALES_POURCENTAGE_MAX`] décimales.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Pourcentage(Decimal);

impl Pourcentage {
    /// Construit un pourcentage (`25` pour 25 %).
    ///
    /// # Errors
    /// [`ErreurEcheance::PourcentageInvalide`] hors de ]0, 100[ ou au-delà de six décimales.
    pub fn nouveau(valeur: Decimal) -> Result<Self, ErreurEcheance> {
        if valeur <= Decimal::ZERO
            || valeur >= Decimal::ONE_HUNDRED
            || valeur.normalize().scale() > DECIMALES_POURCENTAGE_MAX
        {
            return Err(ErreurEcheance::PourcentageInvalide(valeur));
        }
        Ok(Self(valeur))
    }

    /// La valeur (`25` pour 25 %).
    #[must_use]
    pub fn valeur(&self) -> Decimal {
        self.0
    }
}

/// Montant fixe d'un terme, en devise de tenue, strictement positif ; arrondi à l'échelle de
/// la devise au moment de la répartition (Odoo `currency.round(line.value_amount)`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MontantFixe(Decimal);

impl MontantFixe {
    /// Construit un montant fixe.
    ///
    /// # Errors
    /// [`ErreurEcheance::MontantFixeInvalide`] si le montant n'est pas strictement positif.
    pub fn nouveau(valeur: Decimal) -> Result<Self, ErreurEcheance> {
        if valeur <= Decimal::ZERO {
            return Err(ErreurEcheance::MontantFixeInvalide(valeur));
        }
        Ok(Self(valeur))
    }

    /// La valeur.
    #[must_use]
    pub fn valeur(&self) -> Decimal {
        self.0
    }
}

/// Valeur d'un terme.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ValeurTerme {
    /// Pourcentage du total.
    Pourcentage(Pourcentage),
    /// Montant fixe.
    Fixe(MontantFixe),
    /// Solde : ce qui reste après les termes précédents (dernier terme seulement).
    Solde,
}

/// Terme d'une condition de règlement : une valeur et un délai.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Terme {
    valeur: ValeurTerme,
    delai: Delai,
}

impl Terme {
    /// Construit un terme.
    #[must_use]
    pub fn nouveau(valeur: ValeurTerme, delai: Delai) -> Self {
        Self { valeur, delai }
    }

    /// Valeur du terme.
    #[must_use]
    pub fn valeur(&self) -> ValeurTerme {
        self.valeur
    }

    /// Délai du terme.
    #[must_use]
    pub fn delai(&self) -> Delai {
        self.delai
    }
}

/// Condition de règlement validée : au moins un terme, le dernier est le solde et lui seul,
/// somme des pourcentages strictement inférieure à 100.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ConditionReglement {
    termes: Vec<Terme>,
}

impl ConditionReglement {
    /// Valide une condition de règlement (termes dans l'ordre de leur rang).
    ///
    /// # Errors
    /// [`ErreurEcheance::ConditionSansTerme`], [`ErreurEcheance::DernierTermePasSolde`],
    /// [`ErreurEcheance::SoldeAvantLaFin`], [`ErreurEcheance::PourcentagesExcessifs`].
    pub fn nouvelle(termes: Vec<Terme>) -> Result<Self, ErreurEcheance> {
        let Some((dernier, autres)) = termes.split_last() else {
            return Err(ErreurEcheance::ConditionSansTerme);
        };
        if dernier.valeur != ValeurTerme::Solde {
            return Err(ErreurEcheance::DernierTermePasSolde);
        }
        let mut somme = Decimal::ZERO;
        for (rang, terme) in autres.iter().enumerate() {
            match terme.valeur {
                ValeurTerme::Solde => return Err(ErreurEcheance::SoldeAvantLaFin(rang + 1)),
                ValeurTerme::Pourcentage(p) => {
                    // Chaque pourcentage est < 100 : la somme reste petite tant qu'elle est
                    // < 100, on s'arrête dès qu'elle l'atteint.
                    somme += p.0;
                    if somme >= Decimal::ONE_HUNDRED {
                        return Err(ErreurEcheance::PourcentagesExcessifs);
                    }
                }
                ValeurTerme::Fixe(_) => {}
            }
        }
        Ok(Self { termes })
    }

    /// Les termes, dans l'ordre de leur rang.
    #[must_use]
    pub fn termes(&self) -> &[Terme] {
        &self.termes
    }

    /// Répartit `total` (montant de la ligne de tiers, en devise de tenue, à l'échelle de
    /// cette devise) en termes datés à partir de `reference` (date de la pièce ; en L3, la date
    /// comptable). La somme des termes est exactement `total`.
    ///
    /// # Errors
    /// [`ErreurEcheance::PasEnDeviseTenue`], [`ErreurEcheance::EchelleExcessive`],
    /// [`ErreurEcheance::Depassement`], [`ErreurEcheance::DateHorsLimites`].
    pub fn repartir(
        &self,
        total: Montant,
        reference: Date,
        tenue: &DeviseTenue,
    ) -> Result<Vec<TermeCalcule>, ErreurEcheance> {
        let devise = tenue.devise();
        if total.devise() != devise {
            return Err(ErreurEcheance::PasEnDeviseTenue(total.devise()));
        }
        if tenue.arrondir(total.valeur()) != total.valeur() {
            return Err(ErreurEcheance::EchelleExcessive {
                decimales: tenue.decimales(),
            });
        }
        let absolu = total.valeur().abs();
        let negatif = total.valeur().is_sign_negative();
        if absolu.is_zero() {
            return Ok(vec![TermeCalcule {
                date: reference,
                montant: Montant::zero(devise),
            }]);
        }
        let mut reste = absolu;
        let mut termes = Vec::with_capacity(self.termes.len());
        for terme in &self.termes {
            let brut = match terme.valeur {
                ValeurTerme::Pourcentage(p) => tenue.arrondir(
                    absolu
                        .checked_mul(p.0)
                        .and_then(|v| v.checked_div(Decimal::ONE_HUNDRED))
                        .ok_or(ErreurEcheance::Depassement)?,
                ),
                ValeurTerme::Fixe(f) => tenue.arrondir(f.0),
                ValeurTerme::Solde => reste,
            };
            let valeur = brut.min(reste);
            if valeur.is_zero() {
                continue;
            }
            reste -= valeur;
            let montant = if negatif { -valeur } else { valeur };
            termes.push(TermeCalcule {
                date: terme.delai.echeance(reference)?,
                montant: Montant::nouveau(montant, devise),
            });
        }
        // Le dernier terme est le solde (invariant de construction) : le reste est nul.
        termes.sort_by_key(|t| t.date);
        Ok(termes)
    }
}

/// Terme calculé : une échéance et son montant (du signe du total).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TermeCalcule {
    date: Date,
    montant: Montant,
}

impl TermeCalcule {
    /// Date d'échéance.
    #[must_use]
    pub fn date(&self) -> Date {
        self.date
    }

    /// Montant du terme.
    #[must_use]
    pub fn montant(&self) -> Montant {
        self.montant
    }
}
