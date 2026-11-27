//! Tests du lettrage : cas unitaires et propriétés (ADR 0032, conséquences).

use proptest::prelude::*;
use rust_decimal::Decimal;
use time::macros::date;
use time::{Date, Duration};

use super::*;
use crate::ecriture::{EcritureBrouillon, Justification, ReferencePiece};
use crate::exercice::{CodeExercice, FinPremierExercice};
use crate::journal::CodeJournal;
use crate::montant::{Cours, Devise, DeviseTenue};
use crate::numerotation::NumeroEcriture;
use crate::texte::TexteCourt;

const COMPTE: &str = "4111";

fn xof() -> DeviseTenue {
    DeviseTenue::nouvelle(Devise::parse("XOF").unwrap(), 0).unwrap()
}

fn jour(n: i64) -> Date {
    date!(2026 - 01 - 01) + Duration::days(n)
}

fn id(rang: u64) -> IdLigne {
    IdLigne::nouveau(rang, 1).unwrap()
}

fn ligne_ecriture(sens: Sens, montant: i64, tiers: Option<&str>) -> Ligne {
    let montant = Montant::nouveau(Decimal::from(montant), Devise::parse("XOF").unwrap());
    let ligne = Ligne::en_devise_tenue(NumeroCompte::parse(COMPTE).unwrap(), sens, montant, &xof())
        .unwrap();
    match tiers {
        Some(t) => ligne.avec_tiers(IdTiers::parse(t).unwrap()),
        None => ligne,
    }
}

/// Ligne lettrable de rang `rang`, datée du jour `j`, sans échéance ni référence.
fn ligne(rang: u64, sens: Sens, montant: i64, tiers: Option<&str>, j: i64) -> LigneLettrable {
    ligne_complete(rang, sens, montant, tiers, j, None, None)
}

fn ligne_complete(
    rang: u64,
    sens: Sens,
    montant: i64,
    tiers: Option<&str>,
    j: i64,
    echeance: Option<i64>,
    reference: Option<&str>,
) -> LigneLettrable {
    LigneLettrable::nouvelle(
        id(rang),
        &ligne_ecriture(sens, montant, tiers),
        jour(j),
        tiers.map(|t| IdTiers::parse(t).unwrap()),
        echeance.map(jour),
        reference,
    )
    .unwrap()
}

fn registre(lignes: impl IntoIterator<Item = LigneLettrable>) -> RegistreLettrage {
    let mut registre =
        RegistreLettrage::ouvrir(NumeroCompte::parse(COMPTE).unwrap(), true).unwrap();
    for l in lignes {
        registre.ajouter_ligne(l).unwrap();
    }
    registre
}

fn d(n: i64) -> Decimal {
    Decimal::from(n)
}

fn motif() -> MotifRupture {
    MotifRupture::parse("erreur de saisie").unwrap()
}

fn exercices_ouverts() -> Exercices {
    let mut exercices = Exercices::new();
    exercices
        .ouvrir_premier(date!(2026 - 01 - 01), FinPremierExercice::AnneeDeDebut)
        .unwrap();
    exercices
}

fn residuels(registre: &RegistreLettrage) -> Vec<(IdLigne, Decimal)> {
    registre
        .lignes()
        .map(|l| (l.id(), registre.residuel(l.id()).unwrap()))
        .collect()
}

// --- Construction ----------------------------------------------------------------------------

#[test]
fn compte_non_lettrable_refuse() {
    let compte = NumeroCompte::parse(COMPTE).unwrap();
    assert_eq!(
        RegistreLettrage::ouvrir(compte.clone(), false),
        Err(ErreurLettrage::CompteNonLettrable(compte))
    );
}

#[test]
fn identifiant_de_ligne_non_nul() {
    assert!(IdLigne::nouveau(0, 1).is_err());
    assert!(IdLigne::nouveau(1, 0).is_err());
    assert_eq!(id(7).to_string(), "7/1");
}

#[test]
fn ligne_d_un_autre_compte_ou_en_double_refusee() {
    let mut r = registre([ligne(1, Sens::Debit, 100, None, 0)]);
    assert_eq!(
        r.ajouter_ligne(ligne(1, Sens::Credit, 100, None, 0)),
        Err(ErreurLettrage::LigneEnDouble(id(1)))
    );
    let autre = Ligne::en_devise_tenue(
        NumeroCompte::parse("4011").unwrap(),
        Sens::Credit,
        Montant::nouveau(d(5), Devise::parse("XOF").unwrap()),
        &xof(),
    )
    .unwrap();
    let autre = LigneLettrable::nouvelle(id(2), &autre, jour(0), None, None, None).unwrap();
    assert_eq!(
        r.ajouter_ligne(autre),
        Err(ErreurLettrage::AutreCompte(id(2)))
    );
}

#[test]
fn tiers_resolu_coherent_avec_la_ligne() {
    let avec = ligne_ecriture(Sens::Debit, 10, Some("T1"));
    let sans = ligne_ecriture(Sens::Debit, 10, None);
    let t = IdTiers::parse("T2").unwrap();
    assert!(LigneLettrable::nouvelle(id(1), &avec, jour(0), None, None, None).is_err());
    assert!(LigneLettrable::nouvelle(id(1), &sans, jour(0), Some(t.clone()), None, None).is_err());
    // Le tiers résolu (fusion) peut différer du tiers porté par la ligne.
    let l = LigneLettrable::nouvelle(id(1), &avec, jour(0), Some(t.clone()), None, None).unwrap();
    assert_eq!(l.tiers(), Some(&t));
}

#[test]
fn ligne_depuis_une_ecriture_validee() {
    let mut brouillon = EcritureBrouillon::nouvelle(
        CodeJournal::parse("VE").unwrap(),
        jour(3),
        TexteCourt::parse("facture").unwrap(),
    );
    brouillon.ajouter_ligne(ligne_ecriture(Sens::Debit, 100, Some("T1")));
    let produit = Ligne::en_devise_tenue(
        NumeroCompte::parse("7011").unwrap(),
        Sens::Credit,
        Montant::nouveau(d(100), Devise::parse("XOF").unwrap()),
        &xof(),
    )
    .unwrap();
    brouillon.ajouter_ligne(produit);
    brouillon.justifier(
        Justification::pieces([ReferencePiece::parse(&"a".repeat(64)).unwrap()]).unwrap(),
    );
    let validee = EcritureValidee::reconstituer(
        brouillon.valider_equilibre().unwrap(),
        NumeroEcriture::parse("VE-2026-000001").unwrap(),
        jour(3),
        None,
    )
    .unwrap();
    let tiers = Some(IdTiers::parse("T1").unwrap());
    let l =
        LigneLettrable::depuis_validee(&validee, 9, 1, tiers.clone(), None, Some(" F 1 ")).unwrap();
    assert_eq!(
        (l.id(), l.sens(), l.montant()),
        (IdLigne::nouveau(9, 1).unwrap(), Sens::Debit, d(100))
    );
    assert_eq!(l.reference(), Some("f1"));
    assert_eq!(l.compte().as_str(), COMPTE);
    assert!(l.devise().is_none());
    assert_eq!(
        LigneLettrable::depuis_validee(&validee, 9, 3, tiers, None, None),
        Err(ErreurLettrage::LigneInconnue(
            IdLigne::nouveau(9, 3).unwrap()
        ))
    );
    assert!(LigneLettrable::depuis_validee(&validee, 9, 0, None, None, None).is_err());
}

#[test]
fn reference_vide_ignoree() {
    assert_eq!(
        ligne_complete(1, Sens::Debit, 1, None, 0, None, Some(" \t")).reference(),
        None
    );
}

// --- Lettrage manuel -------------------------------------------------------------------------

#[test]
fn lettrage_total() {
    let mut r = registre([
        ligne(1, Sens::Debit, 100, Some("T1"), 0),
        ligne(2, Sens::Credit, 100, Some("T1"), 5),
    ]);
    let lettrage = r
        .lettrer(&[id(2), id(1)], jour(10), Origine::Manuelle)
        .unwrap();
    assert_eq!(lettrage.lettre().to_string(), "A");
    assert_eq!(lettrage.date(), jour(10));
    assert_eq!(lettrage.origine(), Origine::Manuelle);
    assert_eq!(lettrage.montant(), d(100));
    assert_eq!(
        lettrage.liens(),
        &[Lien::nouveau(id(1), id(2), d(100)).unwrap()]
    );
    let lettre = lettrage.lettre();
    assert_eq!(r.residuel(id(1)), Some(Decimal::ZERO));
    assert_eq!(r.residuel(id(2)), Some(Decimal::ZERO));
    assert!(r.est_total(lettre));
    assert_eq!(r.prochaine_lettre().unwrap().to_string(), "B");
    assert_eq!(r.lettrage(lettre).unwrap().lignes().len(), 2);
}

#[test]
fn lettrage_partiel_puis_complement() {
    let mut r = registre([
        ligne(1, Sens::Debit, 100, Some("T1"), 0),
        ligne(2, Sens::Credit, 30, Some("T1"), 5),
        ligne(3, Sens::Credit, 70, Some("T1"), 9),
    ]);
    let a = r
        .lettrer(&[id(1), id(2)], jour(10), Origine::Manuelle)
        .unwrap()
        .lettre();
    assert_eq!(r.residuel(id(1)), Some(d(70)));
    assert!(!r.est_total(a));
    let b = r
        .lettrer(&[id(1), id(3)], jour(11), Origine::Manuelle)
        .unwrap()
        .lettre();
    assert_eq!(r.lettres(id(1)), vec![a, b]);
    assert!(r.est_total(a) && r.est_total(b));
    assert_eq!(
        r.lettrer(&[id(1), id(3)], jour(12), Origine::Manuelle),
        Err(ErreurLettrage::LigneSoldee(id(1)))
    );
}

#[test]
fn imputation_au_kilometre_du_plus_ancien_au_plus_recent() {
    let mut r = registre([
        ligne(1, Sens::Debit, 100, Some("T1"), 0),
        ligne(2, Sens::Debit, 50, Some("T1"), 5),
        ligne(3, Sens::Credit, 120, Some("T1"), 9),
    ]);
    let attendu = vec![
        Lien::nouveau(id(1), id(3), d(100)).unwrap(),
        Lien::nouveau(id(2), id(3), d(20)).unwrap(),
    ];
    assert_eq!(r.imputer(&[id(3), id(2), id(1)]).unwrap(), attendu);
    r.lettrer(&[id(2), id(3), id(1)], jour(10), Origine::Manuelle)
        .unwrap();
    assert_eq!(r.residuel(id(2)), Some(d(30)));
}

#[test]
fn imputation_selon_l_echeance_avant_la_date() {
    let r = registre([
        ligne_complete(1, Sens::Debit, 100, None, 0, Some(60), None),
        ligne_complete(2, Sens::Debit, 50, None, 5, Some(30), None),
        ligne(3, Sens::Credit, 60, None, 9),
    ]);
    assert_eq!(
        r.imputer(&[id(1), id(2), id(3)]).unwrap(),
        vec![
            Lien::nouveau(id(2), id(3), d(50)).unwrap(),
            Lien::nouveau(id(1), id(3), d(10)).unwrap(),
        ]
    );
}

#[test]
fn selections_refusees() {
    let r = registre([
        ligne(1, Sens::Debit, 100, Some("T1"), 0),
        ligne(2, Sens::Credit, 100, Some("T2"), 0),
        ligne(3, Sens::Credit, 100, Some("T1"), 0),
    ]);
    assert_eq!(
        r.imputer(&[]),
        Err(ErreurLettrage::SensManquant(Sens::Debit))
    );
    assert_eq!(
        r.imputer(&[id(1)]),
        Err(ErreurLettrage::SensManquant(Sens::Credit))
    );
    assert_eq!(
        r.imputer(&[id(1), id(1)]),
        Err(ErreurLettrage::LigneEnDouble(id(1)))
    );
    assert_eq!(
        r.imputer(&[id(1), id(9)]),
        Err(ErreurLettrage::LigneInconnue(id(9)))
    );
    assert_eq!(
        r.imputer(&[id(1), id(2)]),
        Err(ErreurLettrage::TiersDifferents)
    );
    assert!(r.imputer(&[id(1), id(3)]).is_ok());
}

fn op_lettrage(lettre: &str, liens: Vec<Lien>) -> Operation {
    Operation::Lettrage {
        lettre: Lettre::parse(lettre).unwrap(),
        date: jour(1),
        origine: Origine::Automatique,
        liens,
    }
}

#[test]
fn rejeu_controle() {
    let mut r = registre([
        ligne(1, Sens::Debit, 100, None, 0),
        ligne(2, Sens::Credit, 100, None, 0),
    ]);
    let lien = |m| Lien::nouveau(id(1), id(2), d(m)).unwrap();
    let cas = [
        (1, op_lettrage("C", vec![]), ErreurLettrage::SansLien),
        (
            1,
            op_lettrage("C", vec![Lien::nouveau(id(2), id(1), d(10)).unwrap()]),
            ErreurLettrage::SensContraire(id(2)),
        ),
        (
            1,
            op_lettrage("C", vec![lien(101)]),
            ErreurLettrage::ResiduelDepasse(id(1)),
        ),
        (
            0,
            op_lettrage("C", vec![lien(1)]),
            ErreurLettrage::RangOperationInvalide(0),
        ),
    ];
    for (rang, operation, erreur) in cas {
        assert_eq!(r.rejouer([(rang, operation)]), Err(erreur));
    }
    assert_eq!(
        Lien::nouveau(id(1), id(2), Decimal::ZERO),
        Err(ErreurLettrage::MontantNonPositif(Decimal::ZERO))
    );
    r.rejouer([(5, op_lettrage("C", vec![lien(40)]))]).unwrap();
    assert_eq!(r.lettrage(Lettre::parse("C").unwrap()).unwrap().rang(), 5);
    assert_eq!(r.prochaine_lettre().unwrap().to_string(), "D");
    assert_eq!(r.residuel(id(1)), Some(d(60)));
    let avant = r.clone();
    // Rang déjà passé, lettre déjà employée, lettre hors de la suite : registre inchangé.
    let erreurs = [
        (
            5,
            op_lettrage("D", vec![lien(1)]),
            ErreurLettrage::RangOperationInvalide(5),
        ),
        (
            6,
            op_lettrage("C", vec![lien(1)]),
            ErreurLettrage::LettreDejaEmployee(Lettre::parse("C").unwrap()),
        ),
        (
            6,
            op_lettrage("B", vec![lien(1)]),
            ErreurLettrage::LettreHorsSuite(Lettre::parse("B").unwrap()),
        ),
    ];
    for (rang, operation, erreur) in erreurs {
        assert_eq!(r.rejouer([(rang, operation)]), Err(erreur));
    }
    // Tout ou rien : la seconde opération échoue, la première n'est pas gardée.
    assert_eq!(
        r.rejouer([
            (7, op_lettrage("D", vec![lien(1)])),
            (7, op_lettrage("E", vec![lien(1)]))
        ]),
        Err(ErreurLettrage::RangOperationInvalide(7))
    );
    assert_eq!(r, avant);
}

#[test]
fn rejeu_d_un_lettrage_rompu_puis_relettre_le_meme_jour() {
    let lignes = [
        ligne(1, Sens::Debit, 100, None, 0),
        ligne(2, Sens::Credit, 100, None, 0),
    ];
    let mut r = registre(lignes.clone());
    let ex = exercices_ouverts();
    let a = r
        .lettrer(&[id(1), id(2)], jour(10), Origine::Manuelle)
        .unwrap()
        .lettre();
    r.rompre(a, jour(10), motif(), &ex).unwrap();
    let b = r
        .lettrer(&[id(1), id(2)], jour(10), Origine::Manuelle)
        .unwrap();
    assert_eq!(b.rang(), 3);
    let mut historique = r.historique();
    assert_eq!(
        historique.iter().map(|(rang, _)| *rang).collect::<Vec<_>>(),
        [1, 2, 3]
    );
    // Même jour partout : seul le rang ordonne. L'ordre fourni est indifférent.
    historique.reverse();
    let mut relu = registre(lignes);
    relu.rejouer(historique).unwrap();
    assert_eq!(relu, r);
}

// --- Rupture ---------------------------------------------------------------------------------

#[test]
fn rupture_en_ajout_seul() {
    let mut r = registre([
        ligne(1, Sens::Debit, 100, None, 0),
        ligne(2, Sens::Credit, 100, None, 0),
    ]);
    let a = r
        .lettrer(&[id(1), id(2)], jour(10), Origine::Manuelle)
        .unwrap()
        .lettre();
    let ex = exercices_ouverts();
    assert_eq!(
        r.rompre(a, jour(9), motif(), &ex),
        Err(ErreurLettrage::RuptureAnterieure(a))
    );
    let rupture = r.rompre(a, jour(12), motif(), &ex).unwrap();
    assert_eq!((rupture.lettre(), rupture.date()), (a, jour(12)));
    assert_eq!(rupture.motif().as_str(), "erreur de saisie");
    assert_eq!(r.residuel(id(1)), Some(d(100)));
    assert!(r.lettrage(a).is_some(), "le lettrage rompu reste inscrit");
    assert!(r.rupture(a).is_some());
    assert!(!r.est_total(a));
    assert_eq!(r.lettres(id(1)), Vec::<Lettre>::new());
    assert_eq!(
        r.rompre(a, jour(13), motif(), &ex),
        Err(ErreurLettrage::DejaRompu(a))
    );
    let inconnue = Lettre::parse("Z").unwrap();
    assert_eq!(
        r.rompre(inconnue, jour(13), motif(), &ex),
        Err(ErreurLettrage::LettreInconnue(inconnue))
    );
    // Re-lettrer crée un nouveau lettrage : la lettre rompue n'est pas réutilisée.
    let b = r
        .lettrer(&[id(1), id(2)], jour(14), Origine::Manuelle)
        .unwrap()
        .lettre();
    assert_eq!(b.to_string(), "B");
}

#[test]
fn rupture_refusee_apres_cloture_de_l_exercice_du_lettrage() {
    let mut r = registre([
        ligne(1, Sens::Debit, 100, None, 0),
        ligne(2, Sens::Credit, 100, None, 0),
    ]);
    let a = r
        .lettrer(&[id(1), id(2)], jour(10), Origine::Manuelle)
        .unwrap()
        .lettre();
    let mut ex = exercices_ouverts();
    ex.ouvrir_suivant().unwrap();
    let code = CodeExercice::parse("2026").unwrap();
    ex.passer_en_cloture(&code).unwrap();
    assert!(
        r.clone().rompre(a, jour(400), motif(), &ex).is_ok(),
        "en clôture : admis"
    );
    ex.cloturer(&code, []).unwrap();
    assert_eq!(
        r.rompre(a, jour(400), motif(), &ex),
        Err(ErreurLettrage::ExerciceClos(a))
    );
    // La relecture d'une rupture historique ne contrôle pas l'exercice.
    let rupture = Operation::Rupture {
        lettre: a,
        date: jour(400),
        motif: motif(),
    };
    assert!(r.rejouer([(2, rupture)]).is_ok());
}

#[test]
fn motif_de_rupture_valide() {
    assert!(MotifRupture::parse(&"é".repeat(200)).is_ok());
    for invalide in [
        "",
        "  ",
        "a\nb",
        "a\u{202E}b",
        "a\u{200B}b",
        "é".repeat(201).as_str(),
    ] {
        assert_eq!(
            MotifRupture::parse(invalide),
            Err(ErreurLettrage::MotifInvalide)
        );
    }
}

// --- Devise étrangère ------------------------------------------------------------------------

fn ligne_eur(rang: u64, sens: Sens, montant: i64, cours: &str) -> LigneLettrable {
    let l = Ligne::en_devise(
        NumeroCompte::parse(COMPTE).unwrap(),
        sens,
        Montant::nouveau(d(montant), Devise::parse("EUR").unwrap()),
        Cours::parse(cours).unwrap(),
        &xof(),
    )
    .unwrap();
    LigneLettrable::nouvelle(id(rang), &l, jour(0), None, None, None).unwrap()
}

#[test]
fn devise_etrangere_lettree_seulement_si_totale_et_equilibree() {
    let r = registre([
        ligne_eur(1, Sens::Debit, 100, "655.957"),
        ligne_eur(2, Sens::Credit, 100, "655.957"),
        ligne_eur(3, Sens::Credit, 50, "655.957"),
        ligne(4, Sens::Credit, 65_596, None, 0),
    ]);
    assert_eq!(
        r.lignes().next().unwrap().devise().unwrap().valeur(),
        d(100)
    );
    assert!(r.imputer(&[id(1), id(2)]).is_ok());
    assert_eq!(
        r.imputer(&[id(1), id(3)]),
        Err(ErreurLettrage::DeviseHorsL3),
        "partiel"
    );
    assert_eq!(
        r.imputer(&[id(1), id(4)]),
        Err(ErreurLettrage::DeviseHorsL3),
        "mixte"
    );
    let change = registre([
        ligne_eur(1, Sens::Debit, 100, "655.957"),
        ligne_eur(2, Sens::Credit, 100, "650"),
        ligne(3, Sens::Credit, 596, None, 0),
    ]);
    assert_eq!(
        change.imputer(&[id(1), id(2), id(3)]),
        Err(ErreurLettrage::DeviseHorsL3)
    );
}

// --- Propositions ----------------------------------------------------------------------------

#[test]
fn proposition_par_reference() {
    let r = registre([
        ligne_complete(1, Sens::Debit, 100, Some("T1"), 0, None, Some("F-001")),
        ligne_complete(2, Sens::Debit, 100, Some("T1"), 1, None, Some("F-002")),
        ligne_complete(
            3,
            Sens::Credit,
            100,
            Some("T1"),
            5,
            None,
            Some("Règlement f-002"),
        ),
    ]);
    let p = r.proposer(&Bornes::default());
    // Le montant seul est ambigu ; la référence départage.
    assert_eq!(p.propositions().len(), 1);
    let prop = &p.propositions()[0];
    assert_eq!(prop.regle(), Regle::Reference);
    assert_eq!(prop.lignes(), &[id(2), id(3)]);
    assert_eq!(
        prop.liens(),
        &[Lien::nouveau(id(2), id(3), d(100)).unwrap()]
    );
    assert_eq!(prop.tiers().unwrap().as_str(), "T1");
    assert_eq!(p.ambiguites(), []);
    // 3 lignes en boucle externe, 1 candidat pour la ligne 1, 1 + 2 (réciproque) pour la 2.
    assert_eq!(p.groupes()[0].evaluations(Regle::Reference), 7);
}

#[test]
fn reference_courte_ou_montant_different_ignores() {
    let r = registre([
        ligne_complete(1, Sens::Debit, 100, None, 0, None, Some("F2")),
        ligne_complete(2, Sens::Credit, 100, None, 1, None, Some("F2")),
        ligne_complete(3, Sens::Debit, 70, None, 2, None, Some("FAC-9")),
        ligne_complete(4, Sens::Credit, 30, None, 3, None, Some("FAC-9")),
    ]);
    // « F2 » est trop courte : seul le montant exact rapproche 1 et 2. FAC-9 : déséquilibré.
    let p = r.proposer(&Bornes::default());
    assert_eq!(p.propositions().len(), 1);
    assert_eq!(p.propositions()[0].regle(), Regle::MontantExact);
    assert_eq!(p.propositions()[0].lignes(), &[id(1), id(2)]);
}

#[test]
fn montant_exact_a_candidat_unique_reciproque() {
    let r = registre([
        ligne(1, Sens::Debit, 100, Some("T1"), 0),
        ligne(2, Sens::Credit, 100, Some("T1"), 5),
        ligne(4, Sens::Debit, 70, Some("T2"), 0),
        ligne(5, Sens::Credit, 70, Some("T3"), 0),
    ]);
    let p = r.proposer(&Bornes::default());
    assert_eq!(
        p.propositions().len(),
        1,
        "aucun rapprochement entre tiers différents"
    );
    assert_eq!(p.propositions()[0].regle(), Regle::MontantExact);
    assert_eq!(p.propositions()[0].lignes(), &[id(1), id(2)]);
    assert_eq!(p.groupes().len(), 3);
    assert_eq!(
        p.groupes()[1].evaluations(Regle::MontantExact),
        0,
        "groupe unilatéral"
    );
}

#[test]
fn montant_exact_ambigu_sans_proposition() {
    // Deux factures, un règlement : ambigu du côté du règlement.
    let r = registre([
        ligne(1, Sens::Debit, 100, None, 0),
        ligne(2, Sens::Debit, 100, None, 1),
        ligne(3, Sens::Credit, 100, None, 5),
    ]);
    let p = r.proposer(&Bornes::default());
    assert_eq!(p.propositions(), []);
    assert_eq!(p.ambiguites().len(), 1);
    let ambiguite = &p.ambiguites()[0];
    assert_eq!(
        (ambiguite.regle(), ambiguite.ligne()),
        (Regle::MontantExact, id(3))
    );
    assert_eq!(ambiguite.candidats(), &[id(1), id(2)]);
    // Deux factures, deux règlements : ambigu des deux côtés.
    let r = registre([
        ligne(1, Sens::Debit, 100, None, 0),
        ligne(2, Sens::Debit, 100, None, 1),
        ligne(3, Sens::Credit, 100, None, 5),
        ligne(4, Sens::Credit, 100, None, 6),
    ]);
    let p = r.proposer(&Bornes::default());
    assert_eq!(p.propositions(), []);
    assert_eq!(p.ambiguites().len(), 4);
}

#[test]
fn proposition_par_combinaison_bornee() {
    let r = registre([
        ligne(1, Sens::Debit, 40, None, 0),
        ligne(2, Sens::Debit, 25, None, 1),
        ligne(3, Sens::Debit, 60, None, 2),
        ligne(4, Sens::Credit, 100, None, 5),
    ]);
    let p = r.proposer(&Bornes::default());
    assert_eq!(p.propositions().len(), 1);
    let prop = &p.propositions()[0];
    assert_eq!(prop.regle(), Regle::Combinaison);
    assert_eq!(prop.lignes(), &[id(1), id(3), id(4)]);
    assert!(!p.groupes()[0].epuise());
    // Deux lignes au plus : pas de combinaison.
    assert_eq!(
        r.proposer(&Bornes::nouvelles(2, 1000).unwrap())
            .propositions(),
        []
    );
    // Budget d'une évaluation : recherche interrompue et signalée.
    let p = r.proposer(&Bornes::nouvelles(6, 1).unwrap());
    assert_eq!(p.propositions(), []);
    assert!(p.groupes()[0].epuise());
    assert_eq!(p.groupes()[0].evaluations(Regle::Combinaison), 1);
}

#[test]
fn grand_groupe_borne() {
    // Non-régression : un groupe unilatéral n'est pas parcouru, quelle que soit sa taille.
    let n = 100_000u64;
    let unilateral = registre((1..=n).map(|k| ligne(k, Sens::Debit, 100, None, 0)));
    let p = unilateral.proposer(&Bornes::default());
    assert_eq!(p.groupes()[0].evaluations(Regle::Reference), 0);
    assert_eq!(p.groupes()[0].evaluations(Regle::Combinaison), 0);
    // Groupe bilatéral : chaque règle s'arrête au budget, parcours compris.
    let bilateral = registre((1..=20_000u64).map(|k| {
        let sens = if k % 2 == 0 {
            Sens::Debit
        } else {
            Sens::Credit
        };
        ligne_complete(
            k,
            sens,
            100 + i64::try_from(k % 7).unwrap(),
            None,
            0,
            None,
            Some("REF-COMMUNE"),
        )
    }));
    let bornes = Bornes::nouvelles(6, 5_000).unwrap();
    let p = bilateral.proposer(&bornes);
    let groupe = &p.groupes()[0];
    for regle in [Regle::Reference, Regle::MontantExact, Regle::Combinaison] {
        assert!(groupe.evaluations(regle) <= bornes.budget(), "{regle:?}");
    }
    assert!(groupe.epuise());
}

#[test]
fn proposition_appliquee_par_lettrage() {
    let mut r = registre([
        ligne(1, Sens::Debit, 40, None, 0),
        ligne(2, Sens::Debit, 60, None, 2),
        ligne(3, Sens::Credit, 100, None, 5),
    ]);
    let prop = r.proposer(&Bornes::default()).propositions()[0].clone();
    let lettrage = r
        .lettrer(prop.lignes(), jour(9), Origine::PropositionValidee)
        .unwrap();
    assert_eq!(lettrage.liens(), prop.liens());
    assert_eq!(r.proposer(&Bornes::default()).propositions(), []);
}

#[test]
fn bornes_invalides() {
    assert!(Bornes::nouvelles(1, 10).is_err());
    assert!(Bornes::nouvelles(Bornes::LIGNES_MAX_ADMIS + 1, 10).is_err());
    assert!(Bornes::nouvelles(6, 0).is_err());
    let b = Bornes::nouvelles(6, 100_000).unwrap();
    assert_eq!((b.lignes_max(), b.budget()), (6, 100_000));
    assert_eq!(b, Bornes::default());
}

// --- Propriétés ------------------------------------------------------------------------------

/// Références générées : contenues l'une dans l'autre, ou trop courte (« A1 »).
const REFERENCES: [&str; 5] = ["FAC-001", "FAC-002", "Règlement FAC-001", "fac 002", "A1"];

/// Ligne générée : (débit ?, centièmes, dix-millièmes, tiers, jour, échéance, référence,
/// devise étrangère ?).
type Genere = (bool, i64, i64, u8, i64, Option<i64>, Option<usize>, bool);

fn lignes_generees() -> impl Strategy<Value = Vec<Genere>> {
    prop::collection::vec(
        (
            any::<bool>(),
            1i64..60,
            prop_oneof![3 => Just(0i64), 1 => 1i64..100],
            0u8..2,
            0i64..60,
            prop::option::of(0i64..90),
            prop::option::of(0usize..REFERENCES.len()),
            prop::bool::weighted(0.25),
        ),
        1..12,
    )
}

/// Montants à 2 à 4 décimales en devise de tenue USD (4 décimales) ; une ligne sur quatre en
/// EUR au cours fixe 1,0837.
fn ligne_generee(rang: u64, genere: &Genere) -> LigneLettrable {
    let (debit, centiemes, dix_milliemes, tiers, j, echeance, reference, etrangere) = *genere;
    let valeur = Decimal::new(centiemes, 2) + Decimal::new(dix_milliemes, 4);
    let tenue = DeviseTenue::nouvelle(Devise::parse("USD").unwrap(), 4).unwrap();
    let compte = NumeroCompte::parse(COMPTE).unwrap();
    let sens = if debit { Sens::Debit } else { Sens::Credit };
    let ligne = if etrangere {
        let montant = Montant::nouveau(valeur, Devise::parse("EUR").unwrap());
        Ligne::en_devise(
            compte,
            sens,
            montant,
            Cours::parse("1.0837").unwrap(),
            &tenue,
        )
    } else {
        Ligne::en_devise_tenue(
            compte,
            sens,
            Montant::nouveau(valeur, tenue.devise()),
            &tenue,
        )
    };
    let tiers = IdTiers::parse(&format!("T{tiers}")).unwrap();
    LigneLettrable::nouvelle(
        id(rang),
        &ligne.unwrap().avec_tiers(tiers.clone()),
        jour(j),
        Some(tiers),
        echeance.map(jour),
        reference.map(|k| REFERENCES[k]),
    )
    .unwrap()
}

fn construire(generees: &[Genere]) -> Vec<LigneLettrable> {
    (1u64..)
        .zip(generees)
        .map(|(rang, g)| ligne_generee(rang, g))
        .collect()
}

/// Sélection d'après un masque de bits sur les lignes, restreinte au tiers de la première
/// ligne choisie (pour que la plupart des sélections soient lettrables).
fn selection(registre: &RegistreLettrage, masque: u16) -> Vec<IdLigne> {
    let choisies: Vec<&LigneLettrable> = registre
        .lignes()
        .enumerate()
        .filter(|(i, _)| masque & (1 << i) != 0)
        .map(|(_, l)| l)
        .collect();
    let tiers = choisies.first().map(|l| l.tiers());
    choisies
        .iter()
        .filter(|l| Some(l.tiers()) == tiers)
        .map(|l| l.id())
        .collect()
}

/// Applique des opérations : lettrages par masque, ruptures de la lettre d'indice choisi. Les
/// dates se répètent (deux opérations par jour) : seul le rang ordonne le rejeu.
fn appliquer(registre: &mut RegistreLettrage, operations: &[(u16, Option<u8>)]) {
    let ex = exercices_ouverts();
    for (n, (masque, rupture)) in operations.iter().enumerate() {
        let date = jour(100 + i64::try_from(n / 2).unwrap());
        let _ = registre.lettrer(&selection(registre, *masque), date, Origine::Manuelle);
        if let Some(k) = rupture {
            let lettres: Vec<Lettre> = registre.lettrages().map(Lettrage::lettre).collect();
            if !lettres.is_empty() {
                let lettre = lettres[usize::from(*k) % lettres.len()];
                let _ = registre.rompre(lettre, date, motif(), &ex);
            }
        }
    }
}

fn operations() -> impl Strategy<Value = Vec<(u16, Option<u8>)>> {
    prop::collection::vec((any::<u16>(), prop::option::of(any::<u8>())), 0..8)
}

proptest! {
    #[test]
    fn conservation_des_montants_et_bornes_du_residuel(
        generees in lignes_generees(),
        operations in operations(),
    ) {
        let lignes = construire(&generees);
        let mut r = registre(lignes.clone());
        appliquer(&mut r, &operations);
        // Un lettrage ne change aucune ligne (ni donc aucun solde).
        prop_assert_eq!(r.lignes().cloned().collect::<Vec<_>>(), lignes.clone());
        let mut impute: BTreeMap<IdLigne, Decimal> = BTreeMap::new();
        for lettrage in r.lettrages().filter(|l| r.rupture(l.lettre()).is_none()) {
            for lien in lettrage.liens() {
                prop_assert!(lien.montant() > Decimal::ZERO);
                *impute.entry(lien.debit()).or_default() += lien.montant();
                *impute.entry(lien.credit()).or_default() += lien.montant();
            }
            // Devise étrangère : lignes toutes en devise, soldées, montants en devise égaux.
            let cites: Vec<&LigneLettrable> =
                r.lignes().filter(|l| lettrage.lignes().contains(&l.id())).collect();
            if cites.iter().any(|l| l.devise().is_some()) {
                let mut solde = Decimal::ZERO;
                for l in &cites {
                    let devise = l.devise();
                    prop_assert!(devise.is_some());
                    prop_assert_eq!(r.residuel(l.id()), Some(Decimal::ZERO));
                    let valeur = devise.map_or_default(|m| m.valeur());
                    solde += if l.sens() == Sens::Debit { valeur } else { -valeur };
                }
                prop_assert_eq!(solde, Decimal::ZERO);
            }
        }
        let (mut debits, mut credits) = (Decimal::ZERO, Decimal::ZERO);
        for ligne in r.lignes() {
            let residuel = r.residuel(ligne.id()).unwrap();
            prop_assert!(residuel >= Decimal::ZERO && residuel <= ligne.montant());
            let lettre = ligne.montant() - residuel;
            prop_assert_eq!(lettre, impute.get(&ligne.id()).copied().unwrap_or_default());
            match ligne.sens() {
                Sens::Debit => debits += lettre,
                Sens::Credit => credits += lettre,
            }
        }
        prop_assert_eq!(debits, credits, "débit imputé = crédit imputé");
        // Le rejeu de l'historique, dans n'importe quel ordre, redonne le même registre.
        let mut historique = r.historique();
        historique.reverse();
        let mut relu = registre(lignes);
        relu.rejouer(historique).unwrap();
        prop_assert_eq!(relu, r);
    }

    #[test]
    fn imputation_deterministe(
        generees in lignes_generees(),
        masque in any::<u16>(),
        decalage in any::<usize>(),
    ) {
        let r = registre(construire(&generees));
        let choisies = selection(&r, masque);
        let mut permutee = choisies.clone();
        if !permutee.is_empty() {
            let k = decalage % permutee.len();
            permutee.rotate_left(k);
        }
        permutee.reverse();
        prop_assert_eq!(r.imputer(&choisies), r.imputer(&permutee));
    }

    #[test]
    fn rupture_retablit_les_residuels_et_ne_supprime_rien(
        generees in lignes_generees(),
        operations in operations(),
        masque in any::<u16>(),
    ) {
        let mut r = registre(construire(&generees));
        appliquer(&mut r, &operations);
        let avant = residuels(&r);
        let Ok(lettrage) = r.lettrer(&selection(&r, masque), jour(200), Origine::Manuelle) else {
            return Ok(());
        };
        let lettre = lettrage.lettre();
        let une_fois = residuels(&r);
        let nombre = r.lettrages().count();
        r.rompre(lettre, jour(200), motif(), &exercices_ouverts()).unwrap();
        prop_assert_eq!(residuels(&r), avant);
        prop_assert_eq!(r.lettrages().count(), nombre);
        prop_assert!(r.lettrage(lettre).is_some() && r.rupture(lettre).is_some());
        // Lettrer, rompre puis relettrer (le même jour) = lettrer une fois.
        let relettre = r.lettrer(&selection(&r, masque), jour(200), Origine::Manuelle).unwrap();
        prop_assert!(relettre.lettre() > lettre);
        prop_assert_eq!(residuels(&r), une_fois);
    }

    #[test]
    fn lettrage_total_implique_solde_nul(generees in lignes_generees(), masque in any::<u16>()) {
        let mut r = registre(construire(&generees));
        let choisies = selection(&r, masque);
        let Ok(lettrage) = r.lettrer(&choisies, jour(100), Origine::Manuelle) else {
            return Ok(());
        };
        let lettre = lettrage.lettre();
        if r.est_total(lettre) {
            let solde: Decimal = r
                .lignes()
                .filter(|l| r.lettrage(lettre).unwrap().lignes().contains(&l.id()))
                .map(|l| match l.sens() {
                    Sens::Debit => l.montant(),
                    Sens::Credit => -l.montant(),
                })
                .sum();
            prop_assert_eq!(solde, Decimal::ZERO);
        }
    }

    #[test]
    fn propositions_bornees_equilibrees_et_applicables(
        generees in lignes_generees(),
        operations in operations(),
        lignes_max in 2usize..8,
        budget in 1u64..400,
    ) {
        let mut r = registre(construire(&generees));
        appliquer(&mut r, &operations);
        let bornes = Bornes::nouvelles(lignes_max, budget).unwrap();
        let propositions = r.proposer(&bornes);
        for groupe in propositions.groupes() {
            for regle in [Regle::Reference, Regle::MontantExact, Regle::Combinaison] {
                prop_assert!(groupe.evaluations(regle) <= budget);
            }
        }
        for ambiguite in propositions.ambiguites() {
            prop_assert!(ambiguite.candidats().len() >= 2);
            prop_assert!(ambiguite.regle() != Regle::Combinaison);
        }
        // Lignes disjointes, combinaisons bornées, groupes soldés, application à l'identique.
        let mut vues = BTreeSet::new();
        for prop in propositions.propositions() {
            prop_assert!(prop.lignes().len() <= lignes_max);
            if prop.regle() != Regle::Combinaison {
                prop_assert_eq!(prop.lignes().len(), 2);
            }
            for id in prop.lignes() {
                prop_assert!(vues.insert(*id));
            }
            let lettrage = r.lettrer(prop.lignes(), jour(300), Origine::PropositionValidee).unwrap();
            prop_assert_eq!(lettrage.liens(), prop.liens());
            let lettre = lettrage.lettre();
            prop_assert!(r.est_total(lettre), "une proposition solde ses lignes");
        }
    }

    #[test]
    fn devise_etrangere_lettree_si_soldee_dans_les_deux_devises(
        valeurs in prop::collection::vec((1i64..5_000, 2u32..=4), 1..5),
        cours in prop::sample::select(vec!["1.0837", "655.957", "0.9213"]),
        retirer in any::<bool>(),
    ) {
        let tenue = DeviseTenue::nouvelle(Devise::parse("USD").unwrap(), 4).unwrap();
        let eur = Devise::parse("EUR").unwrap();
        let cours = Cours::parse(cours).unwrap();
        let compte = || NumeroCompte::parse(COMPTE).unwrap();
        let montants: Vec<Decimal> = valeurs.iter().map(|&(m, e)| Decimal::new(m, e)).collect();
        let total: Decimal = montants.iter().sum();
        let mut lignes: Vec<LigneLettrable> = (1u64..)
            .zip(&montants)
            .map(|(rang, m)| {
                let l = Ligne::en_devise(compte(), Sens::Debit, Montant::nouveau(*m, eur), cours, &tenue);
                LigneLettrable::nouvelle(id(rang), &l.unwrap(), jour(0), None, None, None).unwrap()
            })
            .collect();
        let reglement = Ligne::en_devise(compte(), Sens::Credit, Montant::nouveau(total, eur), cours, &tenue);
        lignes.push(LigneLettrable::nouvelle(id(99), &reglement.unwrap(), jour(1), None, None, None).unwrap());
        if retirer && lignes.len() > 2 {
            lignes.remove(0);
        }
        let somme = |sens| -> Decimal {
            lignes.iter().filter(|l| l.sens() == sens).map(LigneLettrable::montant).sum()
        };
        let soldee = somme(Sens::Debit) == somme(Sens::Credit)
            && !(retirer && montants.len() > 1);
        let mut r = registre(lignes.clone());
        let selection: Vec<IdLigne> = lignes.iter().map(LigneLettrable::id).collect();
        match r.lettrer(&selection, jour(2), Origine::Manuelle) {
            Ok(lettrage) => {
                let lettre = lettrage.lettre();
                prop_assert!(soldee);
                prop_assert!(r.est_total(lettre));
            }
            Err(erreur) => {
                prop_assert!(!soldee);
                prop_assert_eq!(erreur, ErreurLettrage::DeviseHorsL3);
            }
        }
        // Une ligne en devise de tenue imputée la première (plus ancienne) : toujours hors de L3.
        let mut mixte = registre(lignes);
        let tenue_seule = Ligne::en_devise_tenue(compte(), Sens::Debit, Montant::nouveau(Decimal::ONE, tenue.devise()), &tenue);
        mixte.ajouter_ligne(LigneLettrable::nouvelle(id(100), &tenue_seule.unwrap(), jour(-1), None, None, None).unwrap()).unwrap();
        let mut avec = selection;
        avec.push(id(100));
        prop_assert_eq!(mixte.imputer(&avec), Err(ErreurLettrage::DeviseHorsL3));
    }
}
