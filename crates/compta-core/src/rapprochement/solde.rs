//! Avertissements de solde d'un compte de trésorerie (ADR 0034, point 2 ; décision D2).
//!
//! **Jamais de refus** : aucun moteur ne bloque un solde négatif (Odoo et Tryton n'ont rien ;
//! le « solde minimum autorisé » de Dolibarr, `account.class.php:296-310`, est informatif) et
//! le texte parle de « présomption d'irrégularité » sans dire de bloquer (AUDCIF fiche 57,
//! p. PDF 428). L'appelant renvoie l'avertissement à la validation, comme
//! `compte_tresorerie_absent` (ADR 0029, point 3).
//!
//! - **Caisse** ou **portefeuille** (mobile money) **créditeur** : le solde « ne doit être que
//!   débiteur ou nul » (fiche 57, p. PDF 428 ; même règle pour le 55, fiche 55, p. PDF 423).
//! - **Plafond de caisse** dépassé : seulement si le dossier en a fixé un (facultatif, aucune
//!   valeur par défaut ; le texte n'en définit aucun).
//! - **Banque** créditrice : un découvert, porté au passif sans compensation (fiche 52,
//!   p. PDF 416) : aucun avertissement.
//!
//! Le solde est celui du compte comptable du compte de trésorerie, débit positif.

use rust_decimal::Decimal;

/// Type d'un compte de trésorerie (ADR 0034, point 1 ; spec §9.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TypeTresorerie {
    /// Compte bancaire.
    Banque,
    /// Caisse.
    Caisse,
    /// Portefeuille de monnaie électronique (mobile money).
    Portefeuille,
}

/// Avertissement de solde : n'empêche rien.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AvertissementSolde {
    /// Caisse au solde créditeur.
    CaisseSoldeCrediteur {
        /// Solde (négatif).
        solde: Decimal,
    },
    /// Portefeuille au solde créditeur.
    PortefeuilleSoldeCrediteur {
        /// Solde (négatif).
        solde: Decimal,
    },
    /// Caisse au-delà du plafond fixé par le dossier.
    CaissePlafondDepasse {
        /// Solde.
        solde: Decimal,
        /// Plafond du compte.
        plafond: Decimal,
    },
}

impl AvertissementSolde {
    /// Code de l'avertissement (ADR 0034, point 2).
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::CaisseSoldeCrediteur { .. } => "caisse_solde_crediteur",
            Self::PortefeuilleSoldeCrediteur { .. } => "portefeuille_solde_crediteur",
            Self::CaissePlafondDepasse { .. } => "caisse_plafond_depasse",
        }
    }
}

/// Avertissements pour un compte de trésorerie de type `type_tresorerie`, au solde `solde`
/// (débit positif). `plafond` est le plafond facultatif du compte ; seul celui d'une caisse
/// est contrôlé (ADR 0034 : `caisse_plafond_depasse`). Dépassé = strictement supérieur.
#[must_use]
pub fn avertissements_solde(
    type_tresorerie: TypeTresorerie,
    solde: Decimal,
    plafond: Option<Decimal>,
) -> Vec<AvertissementSolde> {
    let mut avertissements = Vec::new();
    let crediteur = solde < Decimal::ZERO;
    match type_tresorerie {
        TypeTresorerie::Banque => {}
        TypeTresorerie::Caisse => {
            if crediteur {
                avertissements.push(AvertissementSolde::CaisseSoldeCrediteur { solde });
            }
            if let Some(plafond) = plafond.filter(|p| solde > *p) {
                avertissements.push(AvertissementSolde::CaissePlafondDepasse { solde, plafond });
            }
        }
        TypeTresorerie::Portefeuille => {
            if crediteur {
                avertissements.push(AvertissementSolde::PortefeuilleSoldeCrediteur { solde });
            }
        }
    }
    avertissements
}
