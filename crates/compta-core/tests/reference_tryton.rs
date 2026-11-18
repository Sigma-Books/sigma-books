//! Cas de référence portés de Tryton (GPL-3.0-or-later).
//!
//! Portions portées de Tryton (`modules/account/tests/scenario_move_cancel.rst`, commit
//! f5354ccd9231703470611d7ebf1c9cdc65e952bc), GPL-3.0-or-later, © B2CK SRL, Cédric Krier et
//! autres titulaires du fichier COPYRIGHT de Tryton. Voir docs/provenance/tryton.md#move-cancel.

// Fichier de test : les `unwrap` y sont voulus (une donnée de test invalide fait échouer le test).
#![allow(clippy::unwrap_used)]
mod commun;

use commun::*;
use compta_core::{
    Balance, CodeExercice, Cours, Devise, Filtre, IdTiers, Ligne, Mode, Montant, Regroupement, Sens,
};
use rust_decimal::Decimal;
use time::macros::date;

fn d(texte: &str) -> Decimal {
    texte.parse().unwrap()
}

/// « Cancel reversal Move » : une écriture de produit (42 au crédit du produit, 32 + 10 au débit
/// du compte client pour deux tiers) est contre-passée par inversion des sens. Après la
/// contre-passation, chaque compte porte 42 au débit et 42 au crédit, et l'écriture miroir
/// renvoie à son origine.
///
/// Adaptations : comptes du dossier de test au lieu du plan Tryton ; la ligne de 32 en seconde
/// devise (30 EUR) reçoit le cours 1.0667, car Sigma Books calcule la contre-valeur à partir du
/// cours (30 × 1.0667 = 32,001, arrondi à 32) ; la variante « Cancel Move » par montants
/// négatifs n'est pas portée (contraire à l'invariant 2).
#[test]
fn scenario_move_cancel_reversal() {
    let mut dos = Dossier::nouveau();
    let origine = dos
        .valider(equilibree(
            "VT",
            date!(2026 - 01 - 01),
            "Move to cancel",
            vec![
                ligne("7011", Sens::Credit, "42"),
                Ligne::en_devise(
                    n("4111"),
                    Sens::Debit,
                    Montant::parse("30", Devise::parse("EUR").unwrap()).unwrap(),
                    Cours::parse("1.0667").unwrap(),
                )
                .unwrap()
                .avec_tiers(IdTiers::parse("Customer").unwrap()),
                ligne_tiers("4111", Sens::Debit, "10", "Party"),
            ],
        ))
        .unwrap();
    let exercice = dos
        .exercices
        .exercice(&CodeExercice::parse("2026").unwrap())
        .unwrap()
        .clone();
    let filtre = Filtre::nouveau(
        &exercice,
        &dos.journaux,
        date!(2026 - 01 - 01),
        date!(2026 - 12 - 31),
    )
    .unwrap();
    let mouvements = |dos: &Dossier, compte: &str| {
        Balance::calculer(
            &dos.livre,
            &[],
            Mode::Definitif,
            &filtre,
            Regroupement::ParCompte,
        )
        .unwrap()
        .lignes()
        .iter()
        .find(|l| l.cle() == compte)
        .unwrap()
        .mouvements()
    };
    assert_eq!(mouvements(&dos, "7011"), (d("0"), d("42")));
    assert_eq!(mouvements(&dos, "4111"), (d("42"), d("0")));

    let miroir = dos.contre_passer(&origine, date!(2026 - 01 - 01)).unwrap();
    assert_eq!(
        dos.livre.ecriture(&miroir).unwrap().origine(),
        Some(&origine)
    );
    assert_eq!(mouvements(&dos, "7011"), (d("42"), d("42")));
    assert_eq!(mouvements(&dos, "4111"), (d("42"), d("42")));
}
