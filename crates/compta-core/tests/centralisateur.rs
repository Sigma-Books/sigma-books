//! Centralisateur des journaux (AUDCIF art. 19, dernier alinéa) : cas unitaires et
//! propriétés (égalité avec les mouvements et l'ouverture de la balance).

// Fichier de test : les `unwrap` y sont voulus (une donnée de test invalide fait échouer le test).
#![allow(clippy::unwrap_used)]
mod commun;

use commun::*;
use compta_core::{
    Balance, Centralisateur, CodeExercice, Devise, EcritureEquilibree, Exercices, Filtre,
    FinPremierExercice, Ligne, Mode, MoisCivil, Montant, Regroupement, RubriqueCentralisateur,
    Sens,
};
use std::collections::BTreeMap;

use proptest::prelude::*;
use rust_decimal::Decimal;
use time::macros::date;
use time::{Date, Month};

fn d(texte: &str) -> Decimal {
    texte.parse().unwrap()
}

/// Exercice du 15 juillet 2026 au 31 décembre 2027 (premier exercice prolongé, AUDCIF art. 7) :
/// deux mois d'août distincts. À-nouveaux et une vente d'août 2027 au brouillard.
fn dossier_a_cheval() -> (Dossier, Vec<EcritureEquilibree>) {
    let mut dos = Dossier::nouveau();
    let mut exercices = Exercices::new();
    exercices
        .ouvrir_premier(date!(2026 - 07 - 15), FinPremierExercice::AnneeSuivante)
        .unwrap();
    dos.exercices = exercices;
    for (journal, date, debit, credit, montant) in [
        ("AC", date!(2026 - 08 - 10), "6052", "4011", "100"),
        ("VT", date!(2026 - 08 - 20), "4111", "7011", "300"),
        ("AC", date!(2026 - 09 - 05), "6052", "4011", "50"),
        ("AC", date!(2027 - 08 - 03), "6052", "4011", "70"),
    ] {
        dos.valider(equilibree(
            journal,
            date,
            "Écriture",
            vec![
                ligne_tiers(debit, Sens::Debit, montant, "T"),
                ligne_tiers(credit, Sens::Credit, montant, "T"),
            ],
        ))
        .unwrap();
    }
    let brouillard = vec![
        // Le journal AN refuse la saisie (générés en L6) : l'à-nouveau est pris au brouillard.
        equilibree(
            "AN",
            date!(2026 - 07 - 15),
            "À-nouveaux",
            vec![
                ligne("521", Sens::Debit, "1000"),
                ligne_tiers("4011", Sens::Credit, "1000", "F1"),
            ],
        ),
        equilibree(
            "VT",
            date!(2027 - 08 - 04),
            "Vente provisoire",
            vec![
                ligne_tiers("4111", Sens::Debit, "20", "C1"),
                ligne("7011", Sens::Credit, "20"),
            ],
        ),
    ];
    (dos, brouillard)
}

fn filtre_exercice(dos: &Dossier) -> Filtre {
    let exercice = dos
        .exercices
        .exercice(&CodeExercice::parse("2027").unwrap())
        .unwrap();
    Filtre::exercice(exercice, &dos.journaux, exercice.fin()).unwrap()
}

fn mois(annee: i32, mois: Month) -> MoisCivil {
    MoisCivil::de(Date::from_calendar_date(annee, mois, 1).unwrap())
}

fn resume(rubrique: &RubriqueCentralisateur) -> Vec<(i32, Month, &str, Decimal, Decimal)> {
    rubrique
        .lignes()
        .iter()
        .map(|l| {
            (
                l.mois.annee(),
                l.mois.mois(),
                l.journal.as_str(),
                l.debit,
                l.credit,
            )
        })
        .collect()
}

#[test]
fn lignes_par_mois_avec_l_annee_puis_par_journal() {
    let (dos, brouillard) = dossier_a_cheval();
    let filtre = filtre_exercice(&dos);
    let c = Centralisateur::calculer(dos.livre.ecritures(), &brouillard, Mode::Definitif, &filtre)
        .unwrap();
    assert_eq!(
        resume(c.mouvements()),
        [
            (2026, Month::August, "AC", d("100"), d("100")),
            (2026, Month::August, "VT", d("300"), d("300")),
            (2026, Month::September, "AC", d("50"), d("50")),
            (2027, Month::August, "AC", d("70"), d("70")),
        ]
    );
    let sous_totaux: Vec<_> = c
        .mouvements()
        .totaux_par_mois()
        .iter()
        .map(|t| (t.mois, t.debit, t.credit))
        .collect();
    assert_eq!(
        sous_totaux,
        [
            (mois(2026, Month::August), d("400"), d("400")),
            (mois(2026, Month::September), d("50"), d("50")),
            (mois(2027, Month::August), d("70"), d("70")),
        ]
    );
    assert_eq!(c.mouvements().total(), (d("520"), d("520")));
    // En définitif, le brouillard (dont l'à-nouveau) est ignoré.
    assert_eq!(c.a_nouveaux().lignes(), []);
    assert_eq!(c.a_nouveaux().total(), (Decimal::ZERO, Decimal::ZERO));

    let detail: Vec<_> = c
        .mouvements()
        .lignes_par_compte()
        .iter()
        .filter(|l| l.mois == mois(2026, Month::August))
        .map(|l| (l.journal.as_str(), l.compte.as_str(), l.debit, l.credit))
        .collect();
    assert_eq!(
        detail,
        [
            ("AC", "4011", d("0"), d("100")),
            ("AC", "6052", d("100"), d("0")),
            ("VT", "4111", d("300"), d("0")),
            ("VT", "7011", d("0"), d("300")),
        ]
    );
}

#[test]
fn provisoire_et_rubrique_des_a_nouveaux() {
    let (dos, brouillard) = dossier_a_cheval();
    let filtre = filtre_exercice(&dos);
    let c = Centralisateur::calculer(
        dos.livre.ecritures(),
        &brouillard,
        Mode::Provisoire,
        &filtre,
    )
    .unwrap();
    // La vente provisoire s'ajoute ; l'à-nouveau n'entre pas dans les mouvements.
    assert_eq!(
        resume(c.mouvements()).last().unwrap(),
        &(2027, Month::August, "VT", d("20"), d("20"))
    );
    assert_eq!(c.mouvements().total(), (d("540"), d("540")));
    assert!(
        c.mouvements()
            .lignes()
            .iter()
            .all(|l| l.journal.as_str() != "AN")
    );
    assert_eq!(
        resume(c.a_nouveaux()),
        [(2026, Month::July, "AN", d("1000"), d("1000"))]
    );
    let detail: Vec<_> = c
        .a_nouveaux()
        .lignes_par_compte()
        .iter()
        .map(|l| (l.compte.as_str(), l.debit, l.credit))
        .collect();
    assert_eq!(
        detail,
        [("4011", d("0"), d("1000")), ("521", d("1000"), d("0"))]
    );
    assert_eq!(c.a_nouveaux().total(), (d("1000"), d("1000")));
}

#[test]
fn filtres_de_journaux_de_comptes_et_de_dates() {
    let (dos, brouillard) = dossier_a_cheval();
    let exercice = dos
        .exercices
        .exercice(&CodeExercice::parse("2027").unwrap())
        .unwrap();
    // Journaux : seul AC est retenu.
    let filtre = filtre_exercice(&dos).journaux([j("AC")]);
    let c = Centralisateur::calculer(
        dos.livre.ecritures(),
        &brouillard,
        Mode::Provisoire,
        &filtre,
    )
    .unwrap();
    assert!(c.mouvements().lignes().iter().all(|l| l.journal == j("AC")));
    assert_eq!(c.mouvements().total(), (d("220"), d("220")));
    assert_eq!(c.a_nouveaux().lignes(), []);
    // Comptes : sans effet, une écriture est centralisée en entier.
    let filtre = filtre_exercice(&dos).comptes([n("6052")]);
    let c = Centralisateur::calculer(dos.livre.ecritures(), &[], Mode::Definitif, &filtre).unwrap();
    assert_eq!(c.mouvements().total(), (d("520"), d("520")));
    // Dates : septembre 2026 seul ; l'à-nouveau du 15 juillet est hors bornes.
    let filtre = Filtre::nouveau(
        exercice,
        &dos.journaux,
        date!(2026 - 09 - 01),
        date!(2026 - 09 - 30),
    )
    .unwrap();
    let c = Centralisateur::calculer(
        dos.livre.ecritures(),
        &brouillard,
        Mode::Provisoire,
        &filtre,
    )
    .unwrap();
    assert_eq!(
        resume(c.mouvements()),
        [(2026, Month::September, "AC", d("50"), d("50"))]
    );
    assert_eq!(c.a_nouveaux().lignes(), []);
}

/// La rubrique des à-nouveaux ne retient que ceux datés dans les bornes, comme l'édition des
/// journaux ; la balance, elle, les range dans sa colonne d'ouverture. Les deux ne coïncident
/// que pour un filtre qui part du début de l'exercice.
#[test]
fn a_nouveaux_hors_bornes_absents_de_la_rubrique() {
    let mut dos = Dossier::nouveau();
    dos.valider(equilibree(
        "AC",
        date!(2026 - 02 - 10),
        "Achat",
        vec![
            ligne("6052", Sens::Debit, "100"),
            ligne("521", Sens::Credit, "100"),
        ],
    ))
    .unwrap();
    let brouillard = [equilibree(
        "AN",
        date!(2026 - 01 - 01),
        "À-nouveaux",
        vec![
            ligne("521", Sens::Debit, "1000"),
            ligne_tiers("4011", Sens::Credit, "1000", "F1"),
        ],
    )];
    let exercice = dos
        .exercices
        .exercice(&CodeExercice::parse("2026").unwrap())
        .unwrap();
    let filtre = Filtre::nouveau(
        exercice,
        &dos.journaux,
        date!(2026 - 02 - 01),
        date!(2026 - 03 - 31),
    )
    .unwrap();
    let c = Centralisateur::calculer(
        dos.livre.ecritures(),
        &brouillard,
        Mode::Provisoire,
        &filtre,
    )
    .unwrap();
    assert_eq!(c.a_nouveaux().lignes(), []);
    assert_eq!(c.a_nouveaux().total(), (Decimal::ZERO, Decimal::ZERO));
    assert_eq!(c.mouvements().total(), (d("100"), d("100")));
    assert_eq!(
        (c.du(), c.au()),
        (date!(2026 - 02 - 01), date!(2026 - 03 - 31))
    );
    // La balance du même filtre, elle, porte l'à-nouveau en ouverture.
    let balance = Balance::calculer(
        dos.livre.ecritures(),
        &brouillard,
        Mode::Provisoire,
        &filtre,
        Regroupement::ParCompte,
    )
    .unwrap();
    let banque = balance.lignes().iter().find(|l| l.cle() == "521").unwrap();
    assert_eq!(
        banque.solde_ouverture().unwrap(),
        (d("1000"), Decimal::ZERO)
    );
}

/// Bornes en milieu de mois : la première et la dernière ligne sont des mois partiels.
#[test]
fn mois_partiels_aux_bornes() {
    let (dos, brouillard) = dossier_a_cheval();
    let exercice = dos
        .exercices
        .exercice(&CodeExercice::parse("2027").unwrap())
        .unwrap();
    let filtre = Filtre::nouveau(
        exercice,
        &dos.journaux,
        date!(2026 - 08 - 15),
        date!(2026 - 09 - 10),
    )
    .unwrap();
    let c = Centralisateur::calculer(
        dos.livre.ecritures(),
        &brouillard,
        Mode::Provisoire,
        &filtre,
    )
    .unwrap();
    assert_eq!(
        (c.du(), c.au()),
        (date!(2026 - 08 - 15), date!(2026 - 09 - 10))
    );
    // Août : seule la vente du 20 ; l'achat du 10 est avant la borne.
    assert_eq!(
        resume(c.mouvements()),
        [
            (2026, Month::August, "VT", d("300"), d("300")),
            (2026, Month::September, "AC", d("50"), d("50")),
        ]
    );
}

// --- Propriétés ------------------------------------------------------------------------------

const COMPTES: [&str; 6] = ["4011", "4111", "521", "571", "6052", "7011"];
/// Journaux sans contrainte de compte ; « AN » n'est alimenté que par le brouillard.
const JOURNAUX: [&str; 6] = ["AC", "VT", "OD", "PA", "IN", "AN"];

fn ligne_test(compte: usize, sens: Sens, montant: Decimal) -> Ligne {
    Ligne::en_xof(
        n(COMPTES[compte]),
        sens,
        Montant::nouveau(montant, Devise::XOF),
    )
    .unwrap()
    .avec_tiers(compta_core::IdTiers::parse("T").unwrap())
}

#[derive(Debug, Clone)]
struct Modele {
    journal: usize,
    jour: u16,
    au_brouillard: bool,
    lignes: Vec<(usize, usize, Decimal)>,
}

/// Nombre de jours de l'exercice à cheval (1er juillet 2026 au 31 décembre 2027).
const JOURS_A_CHEVAL: u16 = 549;

fn modele() -> impl Strategy<Value = Modele> {
    let montant = (1i64..1_000_000_000_000, 0u32..=2).prop_map(|(m, e)| Decimal::new(m, e));
    (
        0..JOURNAUX.len(),
        0..JOURS_A_CHEVAL,
        any::<bool>(),
        proptest::collection::vec((0..COMPTES.len(), 0..COMPTES.len(), montant), 1..5),
    )
        .prop_map(|(journal, jour, au_brouillard, lignes)| Modele {
            journal,
            jour,
            au_brouillard,
            lignes,
        })
}

/// Exercice des propriétés : l'année civile 2026, ou un premier exercice du 1er juillet 2026
/// au 31 décembre 2027 (AUDCIF art. 7), où chaque mois de juillet à décembre revient deux fois.
#[derive(Debug, Clone, Copy)]
struct Calendrier {
    a_cheval: bool,
}

impl Calendrier {
    fn debut(self) -> Date {
        if self.a_cheval {
            date!(2026 - 07 - 01)
        } else {
            date!(2026 - 01 - 01)
        }
    }

    fn longueur(self) -> u16 {
        if self.a_cheval { JOURS_A_CHEVAL } else { 365 }
    }

    fn code(self) -> CodeExercice {
        CodeExercice::parse(if self.a_cheval { "2027" } else { "2026" }).unwrap()
    }

    /// Jour `rang` de l'exercice (à partir de 0, ramené dans l'exercice).
    fn date(self, rang: u16) -> Date {
        self.debut() + time::Duration::days(i64::from(rang % self.longueur()))
    }

    fn dossier(self) -> Dossier {
        let mut dos = Dossier::nouveau();
        if self.a_cheval {
            let mut exercices = Exercices::new();
            exercices
                .ouvrir_premier(self.debut(), FinPremierExercice::AnneeSuivante)
                .unwrap();
            dos.exercices = exercices;
        }
        dos
    }
}

/// Écritures validées, sauf celles marquées au brouillard et celles du journal d'à-nouveaux
/// (jamais saisies).
fn construire(cal: Calendrier, modeles: &[Modele]) -> (Dossier, Vec<EcritureEquilibree>) {
    let mut dos = cal.dossier();
    let mut brouillard = Vec::new();
    for m in modeles {
        let mut lignes = Vec::new();
        for (debit, credit, montant) in &m.lignes {
            lignes.push(ligne_test(*debit, Sens::Debit, *montant));
            lignes.push(ligne_test(*credit, Sens::Credit, *montant));
        }
        let journal = JOURNAUX[m.journal];
        let e = equilibree(journal, cal.date(m.jour), "Écriture générée", lignes);
        if au_brouillard(m) {
            brouillard.push(e);
        } else {
            dos.valider(e).unwrap();
        }
    }
    (dos, brouillard)
}

fn au_brouillard(m: &Modele) -> bool {
    m.au_brouillard || JOURNAUX[m.journal] == "AN"
}

fn filtre(dos: &Dossier, cal: Calendrier, du: u16, au: u16, journaux: &[usize]) -> Filtre {
    let exercice = dos.exercices.exercice(&cal.code()).unwrap();
    let (du, au) = (cal.date(du), cal.date(au));
    Filtre::nouveau(exercice, &dos.journaux, du.min(au), du.max(au))
        .unwrap()
        .journaux(journaux.iter().map(|i| j(JOURNAUX[*i])))
}

/// Oracle indépendant de la ventilation du centralisateur : total de chaque (année, mois,
/// journal), recalculé directement depuis les modèles d'écritures.
fn oracle(
    cal: Calendrier,
    modeles: &[Modele],
    filtre_journaux: &[usize],
    bornes: (Date, Date),
    mode: Mode,
    a_nouveaux: bool,
) -> BTreeMap<(i32, u8, &'static str), Decimal> {
    let mut attendu = BTreeMap::new();
    for m in modeles {
        let date = cal.date(m.jour);
        let journal = JOURNAUX[m.journal];
        let retenu = (filtre_journaux.is_empty() || filtre_journaux.contains(&m.journal))
            && bornes.0 <= date
            && date <= bornes.1
            && (mode == Mode::Provisoire || !au_brouillard(m))
            && (journal == "AN") == a_nouveaux;
        if retenu {
            let total: Decimal = m.lignes.iter().map(|(_, _, montant)| *montant).sum();
            *attendu
                .entry((date.year(), u8::from(date.month()), journal))
                .or_insert(Decimal::ZERO) += total;
        }
    }
    attendu
}

fn obtenu(rubrique: &RubriqueCentralisateur) -> BTreeMap<(i32, u8, &str), Decimal> {
    rubrique
        .lignes()
        .iter()
        .map(|l| {
            (
                (l.mois.annee(), u8::from(l.mois.mois()), l.journal.as_str()),
                l.debit,
            )
        })
        .collect()
}

fn mode(provisoire: bool) -> Mode {
    if provisoire {
        Mode::Provisoire
    } else {
        Mode::Definitif
    }
}

/// Sous-totaux et détail par compte cohérents avec les lignes d'une rubrique.
fn coherente(rubrique: &RubriqueCentralisateur) -> Result<(), TestCaseError> {
    for ligne in rubrique.lignes() {
        let (mut debit, mut credit) = (Decimal::ZERO, Decimal::ZERO);
        for detail in rubrique
            .lignes_par_compte()
            .iter()
            .filter(|l| l.mois == ligne.mois && l.journal == ligne.journal)
        {
            debit += detail.debit;
            credit += detail.credit;
        }
        prop_assert_eq!((debit, credit), (ligne.debit, ligne.credit));
        prop_assert_eq!(ligne.debit, ligne.credit);
    }
    for total in rubrique.totaux_par_mois() {
        let (mut debit, mut credit) = (Decimal::ZERO, Decimal::ZERO);
        for ligne in rubrique.lignes().iter().filter(|l| l.mois == total.mois) {
            debit += ligne.debit;
            credit += ligne.credit;
        }
        prop_assert_eq!((debit, credit), (total.debit, total.credit));
    }
    let (mut debit, mut credit) = (Decimal::ZERO, Decimal::ZERO);
    for total in rubrique.totaux_par_mois() {
        debit += total.debit;
        credit += total.credit;
    }
    prop_assert_eq!((debit, credit), rubrique.total());
    prop_assert_eq!(debit, credit);
    // Tri : mois, journal, compte ; sans doublon.
    let cles: Vec<_> = rubrique
        .lignes_par_compte()
        .iter()
        .map(|l| (l.mois, l.journal.clone(), l.compte.clone()))
        .collect();
    prop_assert!(cles.windows(2).all(|p| p[0] < p[1]));
    let cles: Vec<_> = rubrique
        .lignes()
        .iter()
        .map(|l| (l.mois, l.journal.clone()))
        .collect();
    prop_assert!(cles.windows(2).all(|p| p[0] < p[1]));
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    /// (a) Total général du centralisateur = mouvements de la balance (même filtre, même mode) ;
    /// (b) détail par compte = ligne (mois, journal) correspondante, sous-totaux cohérents.
    #[test]
    fn centralisateur_egale_les_mouvements_de_la_balance(
        modeles in proptest::collection::vec(modele(), 1..25),
        a_cheval in any::<bool>(),
        du in 0..JOURS_A_CHEVAL,
        au in 0..JOURS_A_CHEVAL,
        journaux in proptest::collection::btree_set(0..JOURNAUX.len(), 0..3),
        provisoire in any::<bool>(),
    ) {
        let cal = Calendrier { a_cheval };
        let (dos, brouillard) = construire(cal, &modeles);
        let journaux: Vec<usize> = journaux.into_iter().collect();
        let filtre = filtre(&dos, cal, du, au, &journaux);
        let mode = mode(provisoire);
        let c = Centralisateur::calculer(dos.livre.ecritures(), &brouillard, mode, &filtre).unwrap();
        let balance = Balance::calculer(dos.livre.ecritures(), &brouillard, mode, &filtre, Regroupement::ParCompte).unwrap();
        let (mut debit, mut credit) = (Decimal::ZERO, Decimal::ZERO);
        for ligne in balance.lignes() {
            let (d, c) = ligne.mouvements();
            debit += d;
            credit += c;
        }
        prop_assert_eq!(c.mouvements().total(), (debit, credit));
        prop_assert_eq!(debit, credit);
        coherente(c.mouvements())?;
        coherente(c.a_nouveaux())?;
        prop_assert!(c.mouvements().lignes().iter().all(|l| l.journal.as_str() != "AN"));
        prop_assert!(c.a_nouveaux().lignes().iter().all(|l| l.journal.as_str() == "AN"));
    }

    /// Oracle : pour chaque (année, mois, journal), la ligne égale la somme recalculée
    /// directement depuis les écritures, dans les deux modes et les deux rubriques, sur un
    /// exercice civil ou à cheval sur deux années civiles.
    #[test]
    fn lignes_egalent_l_oracle_par_annee_mois_et_journal(
        modeles in proptest::collection::vec(modele(), 1..25),
        a_cheval in any::<bool>(),
        du in 0..JOURS_A_CHEVAL,
        au in 0..JOURS_A_CHEVAL,
        journaux in proptest::collection::btree_set(0..JOURNAUX.len(), 0..3),
    ) {
        let cal = Calendrier { a_cheval };
        let (dos, brouillard) = construire(cal, &modeles);
        let journaux: Vec<usize> = journaux.into_iter().collect();
        let filtre = filtre(&dos, cal, du, au, &journaux);
        for mode in [Mode::Definitif, Mode::Provisoire] {
            let c = Centralisateur::calculer(dos.livre.ecritures(), &brouillard, mode, &filtre).unwrap();
            let bornes = (c.du(), c.au());
            prop_assert_eq!(
                obtenu(c.mouvements()),
                oracle(cal, &modeles, &journaux, bornes, mode, false),
                "mouvements, {:?}", mode
            );
            prop_assert_eq!(
                obtenu(c.a_nouveaux()),
                oracle(cal, &modeles, &journaux, bornes, mode, true),
                "à-nouveaux, {:?}", mode
            );
        }
    }

    /// (c) Depuis le début de l'exercice ([`Filtre::exercice`]), la rubrique des à-nouveaux
    /// donne, compte par compte, le solde d'ouverture de la balance.
    #[test]
    fn a_nouveaux_egalent_l_ouverture_de_la_balance(
        modeles in proptest::collection::vec(modele(), 1..25),
        a_cheval in any::<bool>(),
        au in 0..JOURS_A_CHEVAL,
        provisoire in any::<bool>(),
    ) {
        let cal = Calendrier { a_cheval };
        let (dos, brouillard) = construire(cal, &modeles);
        let exercice = dos.exercices.exercice(&cal.code()).unwrap();
        let filtre = Filtre::exercice(exercice, &dos.journaux, cal.date(au)).unwrap();
        let mode = mode(provisoire);
        let c = Centralisateur::calculer(dos.livre.ecritures(), &brouillard, mode, &filtre).unwrap();
        let balance = Balance::calculer(dos.livre.ecritures(), &brouillard, mode, &filtre, Regroupement::ParCompte).unwrap();
        for ligne in balance.lignes() {
            let (od, oc) = ligne.solde_ouverture().unwrap();
            let solde_an: Decimal = c
                .a_nouveaux()
                .lignes_par_compte()
                .iter()
                .filter(|l| l.compte.as_str() == ligne.cle())
                .map(|l| l.debit - l.credit)
                .sum();
            prop_assert_eq!(od - oc, solde_an, "compte {}", ligne.cle());
        }
        let (debit, credit) = c.a_nouveaux().total();
        prop_assert_eq!(debit, credit);
        if !provisoire {
            prop_assert_eq!(debit, Decimal::ZERO);
        }
    }
}
