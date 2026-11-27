//! Tests des éditions de tiers : cas unitaires, cas de référence chiffrés et propriétés avec
//! oracle naïf (ADR 0033, point 4 ; plan L3, section Tests).

use proptest::prelude::*;
use rust_decimal::Decimal;
use time::macros::date;
use time::{Date, Duration};

use super::*;
use crate::echeance::{MotifRevision, Revision, StatutReglement};
use crate::ecriture::{EcritureBrouillon, Justification, ReferencePiece};
use crate::editions::{Filtre, FormatBalance};
use crate::exercice::{CodeExercice, Exercices, FinPremierExercice};
use crate::journal::{Journal, Journaux, TypeJournal};
use crate::lettrage::{MotifRupture, Origine};
use crate::montant::{Devise, DeviseTenue, Montant};
use crate::plan::PlanComptes;

const CLIENTS: &str = "4111";
const FOURNISSEURS: &str = "4011";
const CONTREPARTIE: &str = "5211";

fn xof() -> DeviseTenue {
    DeviseTenue::nouvelle(Devise::parse("XOF").unwrap(), 0).unwrap()
}

fn jour(n: i64) -> Date {
    date!(2026 - 01 - 01) + Duration::days(n)
}

fn d(n: i64) -> Decimal {
    Decimal::from(n)
}

fn n(compte: &str) -> NumeroCompte {
    NumeroCompte::parse(compte).unwrap()
}

fn t(tiers: &str) -> IdTiers {
    IdTiers::parse(tiers).unwrap()
}

fn id(rang: u64) -> IdLigne {
    IdLigne::nouveau(rang, 1).unwrap()
}

fn ligne_ecriture(compte: &str, sens: Sens, montant: i64, tiers: Option<&str>) -> Ligne {
    let montant = Montant::nouveau(d(montant), Devise::parse("XOF").unwrap());
    let ligne = Ligne::en_devise_tenue(n(compte), sens, montant, &xof()).unwrap();
    match tiers {
        Some(x) => ligne.avec_tiers(t(x)),
        None => ligne,
    }
}

/// Écriture équilibrée du journal `journal` : ligne de tiers en position 1, contrepartie de
/// trésorerie en position 2.
fn equilibree(
    journal: &str,
    date: Date,
    compte: &str,
    sens: Sens,
    montant: i64,
    tiers: &str,
) -> EcritureEquilibree {
    let mut brouillon = EcritureBrouillon::nouvelle(
        CodeJournal::parse(journal).unwrap(),
        date,
        TexteCourt::parse("opération").unwrap(),
    );
    brouillon.ajouter_ligne(ligne_ecriture(compte, sens, montant, Some(tiers)));
    brouillon.ajouter_ligne(ligne_ecriture(CONTREPARTIE, sens.inverse(), montant, None));
    brouillon.justifier(
        Justification::pieces([ReferencePiece::parse(&"a".repeat(64)).unwrap()]).unwrap(),
    );
    brouillon.valider_equilibre().unwrap()
}

/// Écriture validée de rang de chaîne `rang` (numéro `VE-2026-rang`).
fn validee(
    rang: u64,
    date: Date,
    compte: &str,
    sens: Sens,
    montant: i64,
    tiers: &str,
) -> EcritureValidee {
    validee_journal("VE", rang, date, compte, sens, montant, tiers)
}

fn validee_journal(
    journal: &str,
    rang: u64,
    date: Date,
    compte: &str,
    sens: Sens,
    montant: i64,
    tiers: &str,
) -> EcritureValidee {
    EcritureValidee::reconstituer(
        equilibree(journal, date, compte, sens, montant, tiers),
        NumeroEcriture::parse(&format!("{journal}-2026-{rang:06}")).unwrap(),
        date,
        None,
    )
    .unwrap()
}

/// Ligne d'une opération de tiers : écriture validée, ligne d'édition et ligne lettrable.
struct Operation {
    ecriture: EcritureValidee,
    rang: u64,
    tiers: String,
}

impl Operation {
    fn nouvelle(rang: u64, j: i64, compte: &str, sens: Sens, montant: i64, tiers: &str) -> Self {
        Self {
            ecriture: validee(rang, jour(j), compte, sens, montant, tiers),
            rang,
            tiers: tiers.to_owned(),
        }
    }

    fn ligne(&self, echeance: Option<i64>) -> LigneTiers {
        let echeance = EcheanceLigne::nouvelle(self.ecriture.date(), echeance.map(jour));
        self.ligne_avec(echeance)
    }

    fn ligne_avec(&self, echeance: EcheanceLigne) -> LigneTiers {
        LigneTiers::validee(&self.ecriture, id(self.rang), t(&self.tiers), echeance).unwrap()
    }

    fn lettrable(&self) -> crate::lettrage::LigneLettrable {
        crate::lettrage::LigneLettrable::depuis_validee(
            &self.ecriture,
            self.rang,
            1,
            Some(t(&self.tiers)),
            None,
            None,
        )
        .unwrap()
    }
}

fn registre<'a>(compte: &str, ops: impl IntoIterator<Item = &'a Operation>) -> RegistreLettrage {
    let mut registre = RegistreLettrage::ouvrir(n(compte), true).unwrap();
    for op in ops {
        registre.ajouter_ligne(op.lettrable()).unwrap();
    }
    registre
}

fn exercices() -> Exercices {
    let mut exercices = Exercices::new();
    exercices
        .ouvrir_premier(date!(2026 - 01 - 01), FinPremierExercice::AnneeDeDebut)
        .unwrap();
    exercices
}

fn journaux() -> Journaux {
    let plan = PlanComptes::depuis_referentiel([]).unwrap();
    let mut journaux = Journaux::new();
    for (code, genre) in [("VE", TypeJournal::Ventes), ("AN", TypeJournal::ANouveaux)] {
        journaux
            .ajouter(
                Journal::nouveau(
                    CodeJournal::parse(code).unwrap(),
                    TexteCourt::parse(code).unwrap(),
                    genre,
                    None,
                    &plan,
                )
                .unwrap(),
            )
            .unwrap();
    }
    journaux
}

fn filtre(du: Date, au: Date) -> Filtre {
    let ex = exercices();
    let exercice = ex.exercice(&CodeExercice::parse("2026").unwrap()).unwrap();
    Filtre::nouveau(exercice, &journaux(), du, au).unwrap()
}

fn colonnes(agee: &TiersAgee) -> Vec<(Decimal, Decimal)> {
    agee.colonnes()
        .iter()
        .map(|c| (c.debit(), c.credit()))
        .collect()
}

fn tiers_agee<'a>(balance: &'a BalanceAgee, compte: &str, tiers: &str) -> Option<&'a TiersAgee> {
    balance
        .comptes()
        .iter()
        .find(|c| c.compte().as_str() == compte)?
        .tiers()
        .iter()
        .find(|x| x.tiers().as_str() == tiers)
}

// --- Tranches --------------------------------------------------------------------------------

#[test]
fn bornes_exactes_des_tranches_par_defaut() {
    let tranches = Tranches::default();
    assert_eq!(tranches.bornes(), &[30, 60, 90]);
    for (retard, indice) in [
        (-5, 0),
        (0, 0),
        (1, 1),
        (30, 1),
        (31, 2),
        (60, 2),
        (61, 3),
        (90, 3),
        (91, 4),
        (10_000, 4),
    ] {
        assert_eq!(tranches.indice(retard), indice, "retard {retard}");
    }
    assert_eq!(
        tranches.colonnes(),
        vec![
            Tranche::NonEchu,
            Tranche::Retard { de: 1, a: Some(30) },
            Tranche::Retard {
                de: 31,
                a: Some(60)
            },
            Tranche::Retard {
                de: 61,
                a: Some(90)
            },
            Tranche::Retard { de: 91, a: None },
        ]
    );
}

#[test]
fn bornes_parametrables_validees() {
    assert!(Tranches::nouvelles(&[]).is_ok());
    assert_eq!(Tranches::nouvelles(&[]).unwrap().colonnes().len(), 2);
    let quatre = Tranches::nouvelles(&[15, 30, 60, 120]).unwrap();
    assert_eq!(quatre.colonnes().len(), 6);
    assert_eq!(quatre.indice(120), 4);
    assert_eq!(quatre.indice(121), 5);
    // Borne nulle ou répétée : tranche vide, admise (Tryton et OCA ne contrôlent pas).
    let vides = Tranches::nouvelles(&[0, 30, 30]).unwrap();
    assert_eq!(vides.colonnes().len(), 5);
    assert_eq!(vides.colonnes()[1], Tranche::Retard { de: 1, a: Some(0) });
    for (retard, indice) in [(0, 0), (1, 2), (30, 2), (31, 4)] {
        assert_eq!(vides.indice(retard), indice, "retard {retard}");
    }
    for invalides in [&[1, 2, 3, 4, 5][..], &[60, 30]] {
        assert_eq!(
            Tranches::nouvelles(invalides),
            Err(ErreurEditionTiers::BornesInvalides),
            "{invalides:?}"
        );
    }
}

// --- Construction des lignes -----------------------------------------------------------------

#[test]
fn ligne_tiers_controle_position_tiers_et_echeance() {
    let op = Operation::nouvelle(1, 0, CLIENTS, Sens::Debit, 100, "C1");
    let ligne = op.ligne(Some(30));
    assert_eq!(ligne.id(), Some(id(1)));
    assert_eq!(
        ligne.numero().map(ToString::to_string).as_deref(),
        Some("VE-2026-000001")
    );
    assert_eq!((ligne.sens(), ligne.montant()), (Sens::Debit, d(100)));
    assert_eq!(ligne.compte().as_str(), CLIENTS);
    assert_eq!(ligne.echeance().effective(jour(0)), jour(30));
    // Position 2 : contrepartie sans tiers.
    assert_eq!(
        LigneTiers::validee(
            &op.ecriture,
            IdLigne::nouveau(1, 2).unwrap(),
            t("C1"),
            EcheanceLigne::nouvelle(jour(0), None)
        ),
        Err(ErreurEditionTiers::SansTiers { position: 2 })
    );
    assert_eq!(
        LigneTiers::validee(
            &op.ecriture,
            IdLigne::nouveau(1, 3).unwrap(),
            t("C1"),
            EcheanceLigne::nouvelle(jour(0), None)
        ),
        Err(ErreurEditionTiers::LigneInconnue { position: 3 })
    );
    assert_eq!(
        LigneTiers::validee(
            &op.ecriture,
            id(1),
            t("C1"),
            EcheanceLigne::nouvelle(jour(1), None)
        ),
        Err(ErreurEditionTiers::EcheanceIncoherente(id(1)))
    );
    let brouillard = equilibree("VE", jour(0), CLIENTS, Sens::Debit, 10, "C1");
    assert_eq!(
        LigneTiers::provisoire(&brouillard, 0, t("C1"), None),
        Err(ErreurEditionTiers::LigneInconnue { position: 0 })
    );
    let provisoire = LigneTiers::provisoire(&brouillard, 1, t("C1"), Some(jour(9))).unwrap();
    assert_eq!(provisoire.id(), None);
    assert_eq!(provisoire.numero(), None);
    assert_eq!(provisoire.echeance().effective(jour(0)), jour(9));
}

#[test]
fn ligne_en_double_et_registre_en_double_refuses() {
    let op = Operation::nouvelle(1, 0, CLIENTS, Sens::Debit, 100, "C1");
    let lignes = [op.ligne(None), op.ligne(None)];
    assert_eq!(
        balance_agee(
            &lignes,
            &Lettrages::aucun(),
            Mode::Definitif,
            jour(0),
            &Tranches::default()
        ),
        Err(ErreurEditionTiers::LigneEnDouble(id(1)))
    );
    let r = registre(CLIENTS, [&op]);
    assert_eq!(
        Lettrages::depuis_registres([&r, &r]),
        Err(ErreurEditionTiers::RegistreEnDouble(n(CLIENTS)))
    );
}

#[test]
fn lettrage_d_un_autre_compte_refuse() {
    // La ligne 1 est lettrée dans le registre de 4111 mais présentée sur 4011 : incohérent.
    let facture = Operation::nouvelle(1, 0, CLIENTS, Sens::Debit, 100, "C1");
    let reglement = Operation::nouvelle(2, 1, CLIENTS, Sens::Credit, 100, "C1");
    let mut r = registre(CLIENTS, [&facture, &reglement]);
    r.lettrer(&[id(1), id(2)], jour(2), Origine::Manuelle)
        .unwrap();
    let lettrages = Lettrages::depuis_registres([&r]).unwrap();
    let autre = Operation::nouvelle(1, 0, FOURNISSEURS, Sens::Debit, 100, "C1");
    assert_eq!(
        lettrages.residuel_a(&autre.ligne(None), jour(5)),
        Err(ErreurEditionTiers::LettrageIncoherent(id(1)))
    );
}

// --- Balance âgée : cas de référence ---------------------------------------------------------

/// Jeu de référence au 31 mars 2026 (jour 89), chiffres inventés :
///
/// | Rang | Tiers | Date | Échéance | Sens | Montant | Lettrage |
/// |---|---|---|---|---|---|---|
/// | 1 | C1 | 1er janv. (j0) | 31 janv. (j30) | D | 1 000 000 | 300 000 avec 2, le 20 févr. |
/// | 2 | C1 | 15 févr. (j45) | — | C | 300 000 | idem |
/// | 3 | C1 | 1er mars (j59) | 30 avr. (j119) | D | 500 000 | — |
/// | 4 | C1 | 10 mars (j68) | — | C | 50 000 (avoir) | — |
/// | 5 | C2 | 1er janv. (j0) | — | D | 100 000 | — |
/// | 6 | C2 | 5 mars (j63) | — | C | 200 000 (acompte) | — |
fn jeu_de_reference() -> (Vec<Operation>, RegistreLettrage) {
    let ops = vec![
        Operation::nouvelle(1, 0, CLIENTS, Sens::Debit, 1_000_000, "C1"),
        Operation::nouvelle(2, 45, CLIENTS, Sens::Credit, 300_000, "C1"),
        Operation::nouvelle(3, 59, CLIENTS, Sens::Debit, 500_000, "C1"),
        Operation::nouvelle(4, 68, CLIENTS, Sens::Credit, 50_000, "C1"),
        Operation::nouvelle(5, 0, CLIENTS, Sens::Debit, 100_000, "C2"),
        Operation::nouvelle(6, 63, CLIENTS, Sens::Credit, 200_000, "C2"),
    ];
    let mut r = registre(CLIENTS, &ops);
    r.lettrer(&[id(1), id(2)], jour(50), Origine::Manuelle)
        .unwrap();
    (ops, r)
}

fn lignes_de_reference(ops: &[Operation]) -> Vec<LigneTiers> {
    let echeances = [Some(30), None, Some(119), None, None, None];
    ops.iter()
        .zip(echeances)
        .map(|(op, e)| op.ligne(e))
        .collect()
}

#[test]
fn balance_agee_cas_de_reference() {
    let (ops, r) = jeu_de_reference();
    let lettrages = Lettrages::depuis_registres([&r]).unwrap();
    let balance = balance_agee(
        &lignes_de_reference(&ops),
        &lettrages,
        Mode::Definitif,
        date!(2026 - 03 - 31),
        &Tranches::default(),
    )
    .unwrap();
    let c1 = tiers_agee(&balance, CLIENTS, "C1").unwrap();
    // Facture 1 : 700 000 ouverts, 59 jours de retard (31-60) ; facture 3 non échue ; avoir
    // non lettré, 21 jours (1-30, au crédit).
    assert_eq!(
        colonnes(c1),
        vec![
            (d(500_000), d(0)),
            (d(0), d(50_000)),
            (d(700_000), d(0)),
            (d(0), d(0)),
            (d(0), d(0)),
        ]
    );
    assert_eq!(c1.solde(), d(1_150_000));
    let retards: Vec<(i64, usize, Decimal)> = c1
        .lignes()
        .iter()
        .map(|l| (l.retard_jours(), l.tranche(), l.residuel()))
        .collect();
    assert_eq!(
        retards,
        vec![
            (59, 2, d(700_000)),
            (-30, 0, d(500_000)),
            (21, 1, d(50_000))
        ]
    );
    let c2 = tiers_agee(&balance, CLIENTS, "C2").unwrap();
    // Facture sans échéance : repli sur la date comptable, 89 jours (61-90) ; acompte 26 jours.
    assert_eq!(
        colonnes(c2),
        vec![
            (d(0), d(0)),
            (d(0), d(200_000)),
            (d(0), d(0)),
            (d(100_000), d(0)),
            (d(0), d(0)),
        ]
    );
    assert_eq!(c2.solde(), d(-100_000));
    // Aucune compensation : soldes débiteurs et créditeurs séparés, colonnes en débit et crédit.
    let total = balance.total();
    assert_eq!(total.soldes_debiteurs(), d(1_150_000));
    assert_eq!(total.soldes_crediteurs(), d(100_000));
    let totaux: Vec<(Decimal, Decimal)> = total
        .colonnes()
        .iter()
        .map(|c| (c.debit(), c.credit()))
        .collect();
    assert_eq!(
        totaux,
        vec![
            (d(500_000), d(0)),
            (d(0), d(250_000)),
            (d(700_000), d(0)),
            (d(100_000), d(0)),
            (d(0), d(0)),
        ]
    );
    assert_eq!(balance.comptes()[0].total(), total);
    assert_eq!(balance.date(), date!(2026 - 03 - 31));
    assert_eq!(balance.tranches(), &Tranches::default());
}

#[test]
fn reglement_lettre_apres_la_date_solde_a_la_date() {
    // Facture du 2 janvier (j1), règlement du 30 janvier (j29) lettré le 3 février (j33).
    let facture = Operation::nouvelle(1, 1, CLIENTS, Sens::Debit, 1_000, "C1");
    let reglement = Operation::nouvelle(2, 29, CLIENTS, Sens::Credit, 1_000, "C1");
    let mut r = registre(CLIENTS, [&facture, &reglement]);
    r.lettrer(&[id(1), id(2)], jour(33), Origine::Manuelle)
        .unwrap();
    let lignes = [facture.ligne(None), reglement.ligne(None)];
    let lettrages = Lettrages::depuis_registres([&r]).unwrap();
    let au = |j| {
        balance_agee(
            &lignes,
            &lettrages,
            Mode::Definitif,
            jour(j),
            &Tranches::default(),
        )
        .unwrap()
    };
    // Au 31 janvier (j30) : le paiement a eu lieu, la facture est soldée (Tryton, Odoo, OCA :
    // lettrage daté par la plus haute date des lignes).
    assert_eq!(au(30).comptes(), &[]);
    // Au 28 janvier (j27) : le règlement n'est pas encore dans les livres, facture ouverte et
    // lettre marquée postérieure.
    let au_27 = au(27);
    let ouverte = &au_27.comptes()[0].tiers()[0].lignes()[0];
    assert_eq!((ouverte.residuel(), ouverte.retard_jours()), (d(1_000), 26));
    let lettres = lettrages.lettres_a(&lignes[0], jour(27)).unwrap();
    assert!(lettres[0].est_posterieure());
    assert!(!lettrages.lettres_a(&lignes[0], jour(30)).unwrap()[0].est_posterieure());
}

#[test]
fn rupture_retire_le_lettrage_sans_double_imputation() {
    // Facture a (j0), règlements b (j5) et c (j3). A = {a, b} le j10, rompu le j20 ; B = {a, c}
    // le j21. Dater la rupture compterait A et B au j15 : la facture serait imputée deux fois.
    let a = Operation::nouvelle(1, 0, CLIENTS, Sens::Debit, 100, "C1");
    let b = Operation::nouvelle(2, 5, CLIENTS, Sens::Credit, 100, "C1");
    let c = Operation::nouvelle(3, 3, CLIENTS, Sens::Credit, 100, "C1");
    let mut r = registre(CLIENTS, [&a, &b, &c]);
    let lettre_a = r
        .lettrer(&[id(1), id(2)], jour(10), Origine::Manuelle)
        .unwrap()
        .lettre();
    let lignes = [a.ligne(None), b.ligne(None), c.ligne(None)];
    let avant = Lettrages::depuis_registres([&r]).unwrap();
    assert!(
        avant
            .residuel_a(&lignes[0], jour(15))
            .unwrap()
            .residuel()
            .is_zero()
    );
    r.rompre(
        lettre_a,
        jour(20),
        MotifRupture::parse("erreur de règlement").unwrap(),
        &exercices(),
    )
    .unwrap();
    // La rupture vaut à toute date, même antérieure à son jour (comme chez les moteurs, qui
    // suppriment le lettrage défait) : la lettre A n'apparaît plus.
    let rompu = Lettrages::depuis_registres([&r]).unwrap();
    assert_eq!(
        rompu.residuel_a(&lignes[0], jour(15)).unwrap().residuel(),
        d(100)
    );
    assert_eq!(rompu.lettres_a(&lignes[0], jour(15)).unwrap(), vec![]);
    r.lettrer(&[id(1), id(3)], jour(21), Origine::Manuelle)
        .unwrap();
    let relettre = Lettrages::depuis_registres([&r]).unwrap();
    for j in [3, 4, 5, 15, 30] {
        let balance = balance_agee(
            &lignes,
            &relettre,
            Mode::Definitif,
            jour(j),
            &Tranches::default(),
        )
        .unwrap();
        // Seul le règlement b reste ouvert, dès sa date.
        let ouvertes: Vec<Decimal> = balance
            .comptes()
            .iter()
            .flat_map(CompteAgee::tiers)
            .flat_map(TiersAgee::lignes)
            .map(LigneAgee::residuel)
            .collect();
        let attendu = if j < 5 { vec![] } else { vec![d(100)] };
        assert_eq!(ouvertes, attendu, "jour {j}");
    }
}

#[test]
fn contrepartie_posterieure_au_lettrage_ne_solde_pas_avant_sa_date() {
    let facture = Operation::nouvelle(1, 0, CLIENTS, Sens::Debit, 100, "C1");
    let reglement = Operation::nouvelle(2, 20, CLIENTS, Sens::Credit, 100, "C1");
    let mut r = registre(CLIENTS, [&facture, &reglement]);
    // Lettrage du jour 10 avec un règlement daté du jour 20.
    r.lettrer(&[id(1), id(2)], jour(10), Origine::Manuelle)
        .unwrap();
    let lettrages = Lettrages::depuis_registres([&r]).unwrap();
    let ligne = facture.ligne(None);
    assert_eq!(
        lettrages.residuel_a(&ligne, jour(15)).unwrap().residuel(),
        d(100)
    );
    let lettres = lettrages.lettres_a(&ligne, jour(15)).unwrap();
    assert_eq!(lettres.len(), 1);
    assert!(lettres[0].est_posterieure());
    assert!(
        lettrages
            .residuel_a(&ligne, jour(20))
            .unwrap()
            .residuel()
            .is_zero()
    );
    assert!(!lettrages.lettres_a(&ligne, jour(20)).unwrap()[0].est_posterieure());
}

#[test]
fn revision_d_echeance_posterieure_ignoree() {
    let op = Operation::nouvelle(1, 0, CLIENTS, Sens::Debit, 100, "C1");
    let revision = Revision::nouvelle(
        1,
        jour(80),
        jour(70),
        MotifRevision::parse("report accordé").unwrap(),
    )
    .unwrap();
    let echeance = EcheanceLigne::reconstituer(jour(0), Some(jour(30)), vec![revision]).unwrap();
    let lignes = [op.ligne_avec(echeance)];
    let a = |j| {
        balance_agee(
            &lignes,
            &Lettrages::aucun(),
            Mode::Definitif,
            jour(j),
            &Tranches::default(),
        )
        .unwrap()
    };
    // Au jour 65 : échéance initiale (j30), 35 jours de retard.
    assert_eq!(a(65).comptes()[0].tiers()[0].lignes()[0].retard_jours(), 35);
    // Au jour 75 : la révision du jour 70 porte l'échéance au jour 80, non échu.
    let au_75 = a(75);
    let ligne = &au_75.comptes()[0].tiers()[0].lignes()[0];
    assert_eq!((ligne.echeance(), ligne.tranche()), (jour(80), 0));
}

#[test]
fn mode_provisoire_avec_le_brouillard() {
    let op = Operation::nouvelle(1, 0, CLIENTS, Sens::Debit, 100, "C1");
    let brouillard = equilibree("VE", jour(5), CLIENTS, Sens::Debit, 40, "C1");
    let lignes = [
        op.ligne(None),
        LigneTiers::provisoire(&brouillard, 1, t("C1"), None).unwrap(),
    ];
    let solde = |mode| {
        balance_agee(
            &lignes,
            &Lettrages::aucun(),
            mode,
            jour(10),
            &Tranches::default(),
        )
        .unwrap()
        .comptes()[0]
            .tiers()[0]
            .solde()
    };
    assert_eq!(solde(Mode::Definitif), d(100));
    assert_eq!(solde(Mode::Provisoire), d(140));
}

#[test]
fn tiers_entierement_lettre_absent() {
    let (ops, r) = jeu_de_reference();
    let facture = Operation::nouvelle(7, 0, FOURNISSEURS, Sens::Credit, 10, "F1");
    let reglement = Operation::nouvelle(8, 1, FOURNISSEURS, Sens::Debit, 10, "F1");
    let mut rf = registre(FOURNISSEURS, [&facture, &reglement]);
    rf.lettrer(&[id(7), id(8)], jour(2), Origine::Manuelle)
        .unwrap();
    let mut lignes = lignes_de_reference(&ops);
    lignes.extend([facture.ligne(None), reglement.ligne(None)]);
    let balance = balance_agee(
        &lignes,
        &Lettrages::depuis_registres([&rf, &r]).unwrap(),
        Mode::Definitif,
        jour(89),
        &Tranches::default(),
    )
    .unwrap();
    assert_eq!(balance.comptes().len(), 1);
    assert_eq!(balance.comptes()[0].compte().as_str(), CLIENTS);
}

// --- Grand livre, balance auxiliaire, relevé -------------------------------------------------

#[test]
fn grand_livre_auxiliaire_ouverture_solde_cumule_et_lettres() {
    let (ops, mut r) = jeu_de_reference();
    // À-nouveau de C2 au 1er janvier : toujours en ouverture.
    let an = Operation {
        ecriture: validee_journal("AN", 9, jour(0), CLIENTS, Sens::Debit, 5_000, "C2"),
        rang: 9,
        tiers: "C2".to_owned(),
    };
    r.ajouter_ligne(an.lettrable()).unwrap();
    // Facture 3 lettrée (le 25 mars, j83) avec un règlement du 5 avril (j94) : lettrage
    // postérieur au 31 mars au sens de Tryton et d'Odoo, marqué, sans effet sur le résiduel.
    let reglement = Operation::nouvelle(10, 94, CLIENTS, Sens::Credit, 500_000, "C1");
    r.ajouter_ligne(reglement.lettrable()).unwrap();
    r.lettrer(&[id(3), id(10)], jour(83), Origine::Manuelle)
        .unwrap();
    let mut lignes = lignes_de_reference(&ops);
    lignes.push(an.ligne(None));
    lignes.push(reglement.ligne(None));
    let brouillard = equilibree("VE", jour(70), CLIENTS, Sens::Debit, 1, "C1");
    lignes.push(LigneTiers::provisoire(&brouillard, 1, t("C1"), None).unwrap());
    let lettrages = Lettrages::depuis_registres([&r]).unwrap();
    let mars = filtre(date!(2026 - 03 - 01), date!(2026 - 03 - 31));
    let gl = grand_livre_auxiliaire(&lignes, &lettrages, Mode::Provisoire, &mars).unwrap();
    let cles: Vec<(&str, &str)> = gl
        .iter()
        .map(|b| (b.compte().as_str(), b.tiers().as_str()))
        .collect();
    assert_eq!(cles, vec![(CLIENTS, "C1"), (CLIENTS, "C2")]);
    let c1 = &gl[0];
    // Ouverture : 1 000 000 − 300 000.
    assert_eq!(c1.solde_ouverture(), d(700_000));
    let soldes: Vec<Decimal> = c1.lignes().iter().map(LigneAuxiliaire::solde).collect();
    assert_eq!(soldes, vec![d(1_200_000), d(1_150_000), d(1_150_001)]);
    assert_eq!(
        (c1.total_debit(), c1.total_credit(), c1.solde_final()),
        (d(500_001), d(50_000), d(1_150_001))
    );
    let facture3 = &c1.lignes()[0];
    assert_eq!(facture3.echeance(), jour(119));
    assert_eq!(facture3.residuel().statut(), StatutReglement::Ouverte);
    assert_eq!(facture3.lettres().len(), 1);
    assert!(facture3.lettres()[0].est_posterieure());
    assert_eq!(
        facture3.numero().map(ToString::to_string).as_deref(),
        Some("VE-2026-000003")
    );
    // Brouillard en fin de journée, sans numéro ni lettre.
    let dernier = &c1.lignes()[2];
    assert_eq!(dernier.numero(), None);
    assert_eq!(dernier.lettres(), &[]);
    assert_eq!(dernier.date_valeur_comptable(), None);
    assert_eq!(dernier.journal().as_str(), "VE");
    assert_eq!(dernier.libelle().as_str(), "opération");
    assert_eq!((dernier.debit(), dernier.credit()), (d(1), d(0)));
    let c2 = &gl[1];
    assert_eq!(c2.solde_ouverture(), d(105_000));
    assert_eq!(c2.solde_final(), d(-95_000));
    // Mode définitif : sans le brouillard.
    let definitif = grand_livre_auxiliaire(&lignes, &lettrages, Mode::Definitif, &mars).unwrap();
    assert_eq!(definitif[0].lignes().len(), 2);
    // Filtre de comptes.
    let fournisseurs = mars.clone().comptes([n(FOURNISSEURS)]);
    assert_eq!(
        grand_livre_auxiliaire(&lignes, &lettrages, Mode::Definitif, &fournisseurs),
        Ok(Vec::new())
    );
}

#[test]
fn balance_auxiliaire_sans_compensation() {
    let (ops, _) = jeu_de_reference();
    let lignes = lignes_de_reference(&ops);
    let mars = filtre(date!(2026 - 03 - 01), date!(2026 - 03 - 31));
    let balance = balance_auxiliaire(&lignes, Mode::Definitif, &mars).unwrap();
    let colonnes: Vec<(&str, Vec<Decimal>)> = balance
        .lignes()
        .iter()
        .map(|l| {
            (
                l.tiers().as_str(),
                l.balance().colonnes(FormatBalance::Six).unwrap(),
            )
        })
        .collect();
    assert_eq!(
        colonnes,
        vec![
            (
                "C1",
                vec![d(700_000), d(0), d(500_000), d(50_000), d(1_150_000), d(0)]
            ),
            (
                "C2",
                vec![d(100_000), d(0), d(0), d(200_000), d(0), d(100_000)]
            ),
        ]
    );
    assert_eq!(balance.lignes()[0].compte().as_str(), CLIENTS);
    // Totaux : soldes débiteurs et créditeurs des tiers, non compensés.
    assert_eq!(
        balance.totaux(FormatBalance::Deux).unwrap(),
        vec![d(1_150_000), d(100_000)]
    );
    assert_eq!(
        balance
            .totaux_compte(&n(CLIENTS), FormatBalance::Deux)
            .unwrap(),
        vec![d(1_150_000), d(100_000)]
    );
    assert_eq!(
        balance
            .totaux_compte(&n(FOURNISSEURS), FormatBalance::Deux)
            .unwrap(),
        vec![d(0), d(0)]
    );
}

#[test]
fn releve_de_compte_ouvert_et_complet() {
    let (ops, r) = jeu_de_reference();
    let lignes = lignes_de_reference(&ops);
    let lettrages = Lettrages::depuis_registres([&r]).unwrap();
    let ouvert = releve_tiers(
        &lignes,
        &lettrages,
        Mode::Definitif,
        &t("C1"),
        Releve::Ouvertes(jour(89)),
    )
    .unwrap();
    assert_eq!(ouvert.len(), 1);
    let bloc = &ouvert[0];
    assert_eq!(bloc.tiers().as_str(), "C1");
    assert_eq!(bloc.solde_ouverture(), d(0));
    let lignes_ouvertes: Vec<(Decimal, Decimal, StatutReglement)> = bloc
        .lignes()
        .iter()
        .map(|l| (l.residuel().residuel(), l.solde(), l.residuel().statut()))
        .collect();
    assert_eq!(
        lignes_ouvertes,
        vec![
            (d(700_000), d(700_000), StatutReglement::PartiellementReglee),
            (d(500_000), d(1_200_000), StatutReglement::Ouverte),
            (d(50_000), d(1_150_000), StatutReglement::Ouverte),
        ]
    );
    assert_eq!(
        (bloc.total_debit(), bloc.total_credit(), bloc.solde_final()),
        (d(1_200_000), d(50_000), d(1_150_000))
    );
    assert_eq!(bloc.lignes()[0].lettres().len(), 1);
    let complet = releve_tiers(
        &lignes,
        &lettrages,
        Mode::Definitif,
        &t("C1"),
        Releve::Toutes(&filtre(jour(0), jour(89))),
    )
    .unwrap();
    assert_eq!(complet[0].lignes().len(), 4);
    assert_eq!(complet[0].solde_final(), d(1_150_000));
    let reglement = &complet[0].lignes()[1];
    assert_eq!(reglement.residuel().statut(), StatutReglement::Soldee);
    assert!(matches!(reglement.source(), SourceLigne::Validee { .. }));
}

// --- Propriétés ------------------------------------------------------------------------------

/// Ligne générée : (compte 4111 ou 4011, tiers 0 ou 1, débit, montant, jour, échéance relative).
type Genere = (bool, u8, bool, i64, i64, Option<i64>);

fn lignes_generees() -> impl Strategy<Value = Vec<Genere>> {
    prop::collection::vec(
        (
            any::<bool>(),
            0u8..2,
            any::<bool>(),
            1i64..1_000,
            0i64..120,
            prop::option::of(-10i64..100),
        ),
        2..14,
    )
}

/// Opérations de lettrage : (masque de lignes, rupture d'une lettre), datées du jour donné.
type OperationGeneree = (u16, Option<u8>, i64);

fn operations_generees() -> impl Strategy<Value = Vec<OperationGeneree>> {
    prop::collection::vec(
        (
            any::<u16>(),
            prop::option::of(any::<u8>()).prop_map(|k| k.filter(|x| x % 3 == 0)),
            0i64..150,
        ),
        0..16,
    )
    .prop_map(|mut ops| {
        // Dates d'opération croissantes, comme le jour du dossier au fil des opérations.
        ops.sort_by_key(|(_, _, j)| *j);
        ops
    })
}

fn construire(generees: &[Genere]) -> Vec<(Operation, Option<i64>)> {
    (1u64..)
        .zip(generees)
        .map(|(rang, (client, tiers, debit, montant, j, echeance))| {
            let compte = if *client { CLIENTS } else { FOURNISSEURS };
            let sens = if *debit { Sens::Debit } else { Sens::Credit };
            (
                Operation::nouvelle(rang, *j, compte, sens, *montant, &format!("T{tiers}")),
                echeance.map(|e| j + e),
            )
        })
        .collect()
}

/// Registres des deux comptes après les opérations (une rupture vise la lettre d'indice donné
/// du registre lettré par l'opération).
fn registres(
    ops: &[(Operation, Option<i64>)],
    operations: &[OperationGeneree],
) -> Vec<RegistreLettrage> {
    registres_dates(ops, operations, None)
}

/// Comme [`registres`], toutes les opérations datées du jour `fixe` s'il est donné.
fn registres_dates(
    ops: &[(Operation, Option<i64>)],
    operations: &[OperationGeneree],
    fixe: Option<i64>,
) -> Vec<RegistreLettrage> {
    let ex = exercices();
    let mut registres: Vec<RegistreLettrage> = [CLIENTS, FOURNISSEURS]
        .iter()
        .map(|compte| {
            registre(
                compte,
                ops.iter()
                    .map(|(op, _)| op)
                    .filter(|op| op.lettrable().compte().as_str() == *compte),
            )
        })
        .collect();
    for (masque, rupture, j) in operations {
        let j = &fixe.unwrap_or(*j);
        // Bit 0 : compte ; bit 1 : tiers ; bit 2 : toutes les lignes du tiers ; bits
        // suivants : lignes du tiers retenues.
        let indice = usize::from(masque & 1);
        let tiers = t(&format!("T{}", (masque >> 1) & 1));
        let toutes = masque & 4 != 0;
        let Some(r) = registres.get_mut(indice) else {
            continue;
        };
        let selection: Vec<IdLigne> = r
            .lignes()
            .filter(|l| l.tiers() == Some(&tiers))
            .enumerate()
            .filter(|(i, _)| toutes || (masque >> 3) & (1 << i) != 0)
            .map(|(_, l)| l.id())
            .collect();
        let _ = r.lettrer(&selection, jour(*j), Origine::Manuelle);
        if let Some(k) = rupture {
            let lettres: Vec<Lettre> = r
                .lettrages()
                .map(crate::lettrage::Lettrage::lettre)
                .collect();
            if !lettres.is_empty() {
                let lettre = lettres[usize::from(*k) % lettres.len()];
                let _ = r.rompre(lettre, jour(*j), MotifRupture::parse("m").unwrap(), &ex);
            }
        }
    }
    registres
}

/// Oracle naïf du résiduel à `date`, lu directement dans les registres (sans l'index
/// [`Lettrages`]) : liens des lettrages non rompus dont les deux lignes sont datées au plus tard
/// `date` ; les dates d'opération sont ignorées.
fn residuel_naif(
    ligne: &LigneTiers,
    ops: &[(Operation, Option<i64>)],
    registres: &[RegistreLettrage],
    date: Date,
) -> Decimal {
    let Some(id) = ligne.id() else {
        return ligne.montant();
    };
    let date_de = |autre: IdLigne| {
        ops.iter()
            .find(|(op, _)| op.rang == autre.rang())
            .map(|(op, _)| op.ecriture.date())
            .unwrap()
    };
    let mut reste = ligne.montant();
    for r in registres {
        for lettrage in r.lettrages() {
            if r.rupture(lettrage.lettre()).is_some() {
                continue;
            }
            for lien in lettrage.liens() {
                if (lien.debit() == id && date_de(lien.credit()) <= date)
                    || (lien.credit() == id && date_de(lien.debit()) <= date)
                {
                    reste -= lien.montant();
                }
            }
        }
    }
    reste
}

/// Colonne naïve : non échu, puis la première borne atteinte, sinon la dernière colonne.
fn tranche_naive(retard: i64, bornes: &[u32]) -> usize {
    if retard <= 0 {
        return 0;
    }
    for (k, borne) in bornes.iter().enumerate() {
        if retard <= i64::from(*borne) {
            return k + 1;
        }
    }
    bornes.len() + 1
}

/// Bornes engendrées : zéro à quatre, croissantes, nulles ou répétées admises.
fn bornes_generees() -> impl Strategy<Value = Vec<u32>> {
    prop::collection::vec(0u32..120, 0..=4).prop_map(|mut b| {
        b.sort_unstable();
        b
    })
}

/// Balance âgée naïve : (compte, tiers) → colonnes (débit, crédit).
fn balance_naive(
    lignes: &[LigneTiers],
    ops: &[(Operation, Option<i64>)],
    registres: &[RegistreLettrage],
    date: Date,
    bornes: &[u32],
) -> BTreeMap<(String, String), Vec<(Decimal, Decimal)>> {
    let mut resultat: BTreeMap<(String, String), Vec<(Decimal, Decimal)>> = BTreeMap::new();
    for ligne in lignes.iter().filter(|l| l.date() <= date) {
        let reste = residuel_naif(ligne, ops, registres, date);
        if reste.is_zero() {
            continue;
        }
        let retard = (date - ligne.echeance().effective(date)).whole_days();
        let colonnes = resultat
            .entry((
                ligne.compte().to_string(),
                ligne.tiers().as_str().to_owned(),
            ))
            .or_insert_with(|| vec![(Decimal::ZERO, Decimal::ZERO); bornes.len() + 2]);
        let colonne = &mut colonnes[tranche_naive(retard, bornes)];
        match ligne.sens() {
            Sens::Debit => colonne.0 += reste,
            Sens::Credit => colonne.1 += reste,
        }
    }
    resultat
}

fn balance_calculee(balance: &BalanceAgee) -> BTreeMap<(String, String), Vec<(Decimal, Decimal)>> {
    balance
        .comptes()
        .iter()
        .flat_map(|c| {
            c.tiers().iter().map(|x| {
                (
                    (c.compte().to_string(), x.tiers().as_str().to_owned()),
                    colonnes(x),
                )
            })
        })
        .collect()
}

proptest! {
    #[test]
    fn balance_agee_egale_l_oracle_et_le_solde_auxiliaire(
        generees in lignes_generees(),
        operations in operations_generees(),
        j in 0i64..150,
        bornes in bornes_generees(),
    ) {
        let ops = construire(&generees);
        let registres = registres(&ops, &operations);
        let lignes: Vec<LigneTiers> = ops.iter().map(|(op, e)| op.ligne(*e)).collect();
        let lettrages = Lettrages::depuis_registres(&registres).unwrap();
        let date = jour(j);
        let tranches = Tranches::nouvelles(&bornes).unwrap();
        let balance =
            balance_agee(&lignes, &lettrages, Mode::Definitif, date, &tranches).unwrap();
        prop_assert_eq!(
            balance_calculee(&balance),
            balance_naive(&lignes, &ops, &registres, date, &bornes)
        );
        // Somme des tranches = solde ouvert = solde du compte auxiliaire (balance auxiliaire
        // de l'exercice jusqu'à la date) = solde du relevé des lignes ouvertes.
        let auxiliaire =
            balance_auxiliaire(&lignes, Mode::Definitif, &filtre(jour(0), date)).unwrap();
        for compte in balance.comptes() {
            for tiers in compte.tiers() {
                let mut somme_tranches = Decimal::ZERO;
                for c in tiers.colonnes() {
                    somme_tranches += c.solde().unwrap();
                }
                prop_assert_eq!(somme_tranches, tiers.solde());
                let aux = auxiliaire
                    .lignes()
                    .iter()
                    .find(|l| l.compte() == compte.compte() && l.tiers() == tiers.tiers())
                    .unwrap();
                let (sd, sc) = aux.balance().solde_final().unwrap();
                prop_assert_eq!(tiers.solde(), sd - sc);
                let releve = releve_tiers(
                    &lignes, &lettrages, Mode::Definitif, tiers.tiers(), Releve::Ouvertes(date),
                )
                .unwrap();
                let bloc = releve.iter().find(|b| b.compte() == compte.compte()).unwrap();
                prop_assert_eq!(bloc.solde_final(), tiers.solde());
            }
        }
        // Tout tiers absent de la balance âgée a un solde auxiliaire nul.
        for aux in auxiliaire.lignes() {
            let present = tiers_agee(&balance, aux.compte().as_str(), aux.tiers().as_str());
            if present.is_none() {
                let (sd, sc) = aux.balance().solde_final().unwrap();
                prop_assert_eq!(sd, sc);
            }
        }
        // Totaux sans compensation : colonnes = sommes des tiers, soldes séparés.
        let (mut debiteurs, mut crediteurs) = (Decimal::ZERO, Decimal::ZERO);
        for tiers in balance.comptes().iter().flat_map(CompteAgee::tiers) {
            if tiers.solde() > Decimal::ZERO {
                debiteurs += tiers.solde();
            } else {
                crediteurs -= tiers.solde();
            }
        }
        prop_assert_eq!(balance.total().soldes_debiteurs(), debiteurs);
        prop_assert_eq!(balance.total().soldes_crediteurs(), crediteurs);
    }

    #[test]
    fn independance_de_l_ordre_et_des_dates_d_operation(
        generees in lignes_generees(),
        operations in operations_generees(),
        j in 0i64..150,
    ) {
        let ops = construire(&generees);
        let date = jour(j);
        let registres_complets = registres(&ops, &operations);
        let lignes: Vec<LigneTiers> = ops.iter().map(|(op, e)| op.ligne(*e)).collect();
        let calculer = |lignes: &[LigneTiers], registres: &[&RegistreLettrage]| {
            let lettrages = Lettrages::depuis_registres(registres.iter().copied()).unwrap();
            let balance =
                balance_agee(lignes, &lettrages, Mode::Definitif, date, &Tranches::default())
                    .unwrap();
            let gl = grand_livre_auxiliaire(
                lignes, &lettrages, Mode::Definitif, &filtre(jour(0), date),
            )
            .unwrap();
            (balance, gl)
        };
        let reference = calculer(&lignes, &registres_complets.iter().collect::<Vec<_>>());
        // Ordre des registres et des lignes indifférent.
        let mut inverses: Vec<LigneTiers> = lignes.clone();
        inverses.reverse();
        let registres_inverses: Vec<&RegistreLettrage> = registres_complets.iter().rev().collect();
        prop_assert_eq!(&calculer(&inverses, &registres_inverses), &reference);
        // Registres rejoués depuis leur historique, relu dans l'ordre inverse.
        let rejoues: Vec<RegistreLettrage> = registres_complets
            .iter()
            .map(|r| {
                let mut copie = RegistreLettrage::ouvrir(r.compte().clone(), true).unwrap();
                for l in r.lignes() {
                    copie.ajouter_ligne(l.clone()).unwrap();
                }
                let mut historique = r.historique();
                historique.reverse();
                copie.rejouer(historique).unwrap();
                copie
            })
            .collect();
        prop_assert_eq!(&calculer(&lignes, &rejoues.iter().collect::<Vec<_>>()), &reference);
        // Dates des opérations de lettrage et de rupture sans effet : toutes au même jour,
        // mêmes éditions (lettres postérieures comprises).
        let meme_jour = registres_dates(&ops, &operations, Some(149));
        prop_assert_eq!(&calculer(&lignes, &meme_jour.iter().collect::<Vec<_>>()), &reference);
        // Lignes postérieures à la date ignorées par la balance âgée.
        let anciennes: Vec<LigneTiers> =
            lignes.iter().filter(|l| l.date() <= date).cloned().collect();
        let lettrages = Lettrages::depuis_registres(&registres_complets).unwrap();
        prop_assert_eq!(
            balance_agee(&anciennes, &lettrages, Mode::Definitif, date, &Tranches::default())
                .unwrap(),
            reference.0
        );
    }

    #[test]
    fn tranche_conforme_a_l_oracle(retard in -400i64..400, bornes in bornes_generees()) {
        prop_assert_eq!(
            Tranches::nouvelles(&bornes).unwrap().indice(retard),
            tranche_naive(retard, &bornes)
        );
        prop_assert_eq!(Tranches::default().indice(retard), tranche_naive(retard, &[30, 60, 90]));
    }
}
