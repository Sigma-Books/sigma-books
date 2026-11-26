//! Propriétés du moteur (spec §22.1 ; `crates/compta-core/CLAUDE.md`) :
//! - toute suite d'écritures validées donne une balance équilibrée, quel que soit le format,
//!   le regroupement et la période ;
//! - écriture + contre-passation = effet nul exact sur chaque compte ;
//! - une écriture peut être contre-passée plusieurs fois (ADR 0021 point 4, révisé) : origine +
//!   deux contre-passations = effet inversé net.

// Fichier de test : les `unwrap` y sont voulus (une donnée de test invalide fait échouer le test).
#![allow(clippy::unwrap_used)]
mod commun;

use commun::*;
use compta_core::{
    Balance, CodeExercice, Devise, DeviseTenue, Filtre, FormatBalance, Ligne, Mode, Montant,
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
    Ligne::en_devise_tenue(
        n(COMPTES[compte]),
        sens,
        Montant::nouveau(montant, Devise::parse("XOF").unwrap()),
        &DeviseTenue::nouvelle(Devise::parse("XOF").unwrap(), 0).unwrap(),
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

/// Valide les écritures dans l'ordre donné, chacune le jour de sa date.
fn valider_tout(dos: &mut Dossier, modeles: &[Modele]) -> Vec<compta_core::NumeroEcriture> {
    let mut numeros = Vec::new();
    for m in modeles {
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
            let balance = Balance::calculer(dos.livre.ecritures(), &[], Mode::Definitif, &filtre, regroupement).unwrap();
            for format in FORMATS {
                let totaux = balance.totaux(format).unwrap();
                for paire in totaux.chunks(2) {
                    prop_assert_eq!(paire[0], paire[1], "{:?} {:?}", format, totaux);
                }
            }
        }
    }

    /// Dans une période ouverte, l'ordre de validation est libre (AUDCIF art. 22 : l'ordre
    /// n'est figé qu'à la clôture) ; l'édition du journal classe par date puis par numéro.
    #[test]
    fn ordre_libre_et_edition_classee(jours in proptest::collection::vec(1u16..=365, 1..30)) {
        let mut dos = Dossier::nouveau();
        for jour in &jours {
            let e = equilibree(
                "OD",
                date_de(*jour),
                "Écriture générée",
                vec![ligne_test(4, Sens::Debit, Decimal::ONE), ligne_test(5, Sens::Credit, Decimal::ONE)],
            );
            prop_assert!(dos.valider(e).is_ok());
        }
        let editions = compta_core::journaux(dos.livre.ecritures(), &[], Mode::Definitif, &filtre(&dos, 1, 365)).unwrap();
        let cles: Vec<_> = editions[0]
            .ecritures
            .iter()
            .map(|e| (e.date, e.numero.clone()))
            .collect();
        let mut triees = cles.clone();
        triees.sort();
        prop_assert_eq!(cles, triees);
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
            dos.livre.ecritures(), &[], Mode::Definitif, &filtre(&dos, 1, 365), Regroupement::ParCompte,
        ).unwrap();
        for ligne in balance.lignes() {
            prop_assert_eq!(
                ligne.solde_final().unwrap(), (Decimal::ZERO, Decimal::ZERO),
                "compte {}", ligne.cle()
            );
            let (debit, credit) = ligne.mouvements();
            prop_assert_eq!(debit, credit);
        }
        // Une seconde contre-passation réussit : numéro propre, effet inversé net.
        for numero in &numeros {
            let seconde = dos.contre_passer(numero, date_de(365)).unwrap();
            prop_assert_eq!(dos.livre.contre_passations(numero).len(), 2);
            prop_assert_eq!(dos.livre.ecriture(&seconde).unwrap().origine(), Some(numero));
        }
        let apres = Balance::calculer(
            dos.livre.ecritures(), &[], Mode::Definitif, &filtre(&dos, 1, 365), Regroupement::ParCompte,
        ).unwrap();
        let origines: Vec<_> = numeros
            .iter()
            .map(|n| dos.livre.ecriture(n).unwrap().clone())
            .collect();
        let seules = Balance::calculer(
            &origines, &[], Mode::Definitif, &filtre(&dos, 1, 365), Regroupement::ParCompte,
        ).unwrap();
        for ligne in seules.lignes() {
            let (d0, c0) = ligne.solde_final().unwrap();
            let net = apres
                .lignes()
                .iter()
                .find(|l| l.cle() == ligne.cle())
                .map(|l| l.solde_final().unwrap())
                .unwrap();
            prop_assert_eq!(net.0 - net.1, c0 - d0, "compte {}", ligne.cle());
        }
    }
}
