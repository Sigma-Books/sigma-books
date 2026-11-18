//! Propriétés du moteur (spec §22.1 ; `crates/compta-core/CLAUDE.md`) :
//! - toute suite d'écritures validées donne une balance équilibrée, quel que soit le format,
//!   le regroupement et la période ;
//! - écriture + contre-passation = effet nul exact sur chaque compte ;
//! - une écriture n'est jamais contre-passée deux fois.

// Fichier de test : les `unwrap` y sont voulus (une donnée de test invalide fait échouer le test).
#![allow(clippy::unwrap_used)]
mod commun;

use commun::*;
use compta_core::{
    Balance, CodeExercice, Devise, ErreurValidation, Filtre, FormatBalance, Ligne, Mode, Montant,
    Regroupement, Sens,
};
use proptest::prelude::*;
use rust_decimal::Decimal;
use time::Date;

const COMPTES: [&str; 6] = ["4011", "4111", "521", "571", "6052", "7011"];
const FORMATS: [FormatBalance; 4] = [
    FormatBalance::Deux,
    FormatBalance::Quatre,
    FormatBalance::Six,
    FormatBalance::Huit,
];

/// Ligne sur un compte du dossier de test ; un tiers est toujours fourni (comptes collectifs).
fn ligne_test(compte: usize, sens: Sens, montant: Decimal) -> Ligne {
    Ligne::en_xof(
        n(COMPTES[compte]),
        sens,
        Montant::nouveau(montant, Devise::XOF),
    )
    .unwrap()
    .avec_tiers(compta_core::IdTiers::parse("T").unwrap())
}

/// Écriture d'OD : des débits, et les mêmes montants au crédit sur d'autres comptes.
#[derive(Debug, Clone)]
struct Modele {
    jour: u16,
    lignes: Vec<(usize, usize, Decimal)>,
}

fn modele() -> impl Strategy<Value = Modele> {
    let montant = (1i64..1_000_000_000_000, 0u32..=2).prop_map(|(m, e)| Decimal::new(m, e));
    (
        1u16..=365,
        proptest::collection::vec((0..COMPTES.len(), 0..COMPTES.len(), montant), 1..5),
    )
        .prop_map(|(jour, lignes)| Modele { jour, lignes })
}

fn date_de(jour: u16) -> Date {
    Date::from_ordinal_date(2026, jour).unwrap()
}

/// Valide les écritures dans l'ordre des dates (chronologie, AUDCIF art. 22 3°).
fn valider_tout(dos: &mut Dossier, modeles: &[Modele]) -> Vec<compta_core::NumeroEcriture> {
    let mut modeles = modeles.to_vec();
    modeles.sort_by_key(|m| m.jour);
    let mut numeros = Vec::new();
    for m in &modeles {
        let mut lignes = Vec::new();
        for (debit, credit, montant) in &m.lignes {
            lignes.push(ligne_test(*debit, Sens::Debit, *montant));
            lignes.push(ligne_test(*credit, Sens::Credit, *montant));
        }
        let e = equilibree("OD", date_de(m.jour), "Écriture générée", lignes);
        numeros.push(dos.valider(e).unwrap());
    }
    numeros
}

fn filtre(dos: &Dossier, du: u16, au: u16) -> Filtre {
    let exercice = dos
        .exercices
        .exercice(&CodeExercice::parse("2026").unwrap())
        .unwrap();
    Filtre::nouveau(
        exercice,
        &dos.journaux,
        date_de(du.min(au)),
        date_de(du.max(au)),
    )
    .unwrap()
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    #[test]
    fn toute_suite_validee_donne_une_balance_equilibree(
        modeles in proptest::collection::vec(modele(), 1..25),
        du in 1u16..=365,
        au in 1u16..=365,
    ) {
        let mut dos = Dossier::nouveau();
        let numeros = valider_tout(&mut dos, &modeles);
        // Numérotation continue : OD-2026-000001 … sans trou ni doublon.
        for (i, numero) in numeros.iter().enumerate() {
            prop_assert_eq!(numero.sequence() as usize, i + 1);
        }
        let filtre = filtre(&dos, du, au);
        for regroupement in [Regroupement::ParCompte, Regroupement::ParTronc] {
            let balance = Balance::calculer(&dos.livre, &[], Mode::Definitif, &filtre, regroupement).unwrap();
            for format in FORMATS {
                let totaux = balance.totaux(format).unwrap();
                for paire in totaux.chunks(2) {
                    prop_assert_eq!(paire[0], paire[1], "{:?} {:?}", format, totaux);
                }
            }
        }
    }

    #[test]
    fn chronologie_par_journal(jours in proptest::collection::vec(1u16..=365, 1..30)) {
        let mut dos = Dossier::nouveau();
        let mut derniere: Option<u16> = None;
        for jour in jours {
            let e = equilibree(
                "OD",
                date_de(jour),
                "Écriture générée",
                vec![ligne_test(0, Sens::Debit, Decimal::ONE), ligne_test(1, Sens::Credit, Decimal::ONE)],
            );
            let resultat = dos.valider(e);
            if derniere.is_some_and(|d| jour < d) {
                prop_assert!(
                    matches!(resultat, Err(ErreurValidation::DateAnterieureAuJournal { .. })),
                    "{:?}", resultat
                );
            } else {
                prop_assert!(resultat.is_ok(), "{:?}", resultat);
                derniere = Some(jour);
            }
        }
    }

    #[test]
    fn contre_passation_annule_exactement(
        modeles in proptest::collection::vec(modele(), 1..15),
    ) {
        let mut dos = Dossier::nouveau();
        let numeros = valider_tout(&mut dos, &modeles);
        // Contre-passations au 31 décembre : jamais avant la dernière écriture du journal.
        for numero in &numeros {
            dos.contre_passer(numero, date_de(365)).unwrap();
        }
        let balance = Balance::calculer(
            &dos.livre, &[], Mode::Definitif, &filtre(&dos, 1, 365), Regroupement::ParCompte,
        ).unwrap();
        for ligne in balance.lignes() {
            prop_assert_eq!(
                ligne.solde_final().unwrap(), (Decimal::ZERO, Decimal::ZERO),
                "compte {}", ligne.cle()
            );
            let (debit, credit) = ligne.mouvements();
            prop_assert_eq!(debit, credit);
        }
        // Aucune écriture n'est contre-passée deux fois.
        for numero in &numeros {
            let refus = dos.contre_passer(numero, date_de(365));
            prop_assert!(
                matches!(refus, Err(ErreurValidation::DejaContrePassee { .. })),
                "{:?}", refus
            );
        }
    }
}
