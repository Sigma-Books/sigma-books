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

#[test]
fn reprise_controlee() {
    let mut r = registre([
        ligne(1, Sens::Debit, 100, None, 0),
        ligne(2, Sens::Credit, 100, None, 0),
    ]);
    let c = Lettre::parse("C").unwrap();
    assert_eq!(
        r.reprendre(c, jour(1), Origine::Automatique, vec![]),
        Err(ErreurLettrage::SansLien)
    );
    let inverse = Lien::nouveau(id(2), id(1), d(10)).unwrap();
    assert_eq!(
        r.reprendre(c, jour(1), Origine::Automatique, vec![inverse]),
        Err(ErreurLettrage::SensContraire(id(2)))
    );
    let trop = Lien::nouveau(id(1), id(2), d(101)).unwrap();
    assert_eq!(
        r.reprendre(c, jour(1), Origine::Automatique, vec![trop]),
        Err(ErreurLettrage::ResiduelDepasse(id(1)))
    );
    assert_eq!(
        Lien::nouveau(id(1), id(2), Decimal::ZERO),
        Err(ErreurLettrage::MontantNonPositif(Decimal::ZERO))
    );
    let lien = Lien::nouveau(id(1), id(2), d(40)).unwrap();
    r.reprendre(c, jour(1), Origine::PropositionValidee, vec![lien])
        .unwrap();
    assert_eq!(
        r.reprendre(c, jour(1), Origine::Manuelle, vec![lien]),
        Err(ErreurLettrage::LettreDejaEmployee(c))
    );
    assert_eq!(r.prochaine_lettre().unwrap().to_string(), "D");
    assert_eq!(r.residuel(id(1)), Some(d(60)));
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
    assert!(r.reprendre_rupture(a, jour(400), motif()).is_ok());
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
            60,
            Some("T1"),
            5,
            None,
            Some("Règlement f-002"),
        ),
    ]);
    let p = r.proposer(&Bornes::default());
    assert_eq!(p.propositions().len(), 1);
    let prop = &p.propositions()[0];
    assert_eq!(prop.regle(), Regle::Reference);
    assert_eq!(prop.lignes(), &[id(2), id(3)]);
    assert_eq!(prop.liens(), &[Lien::nouveau(id(2), id(3), d(60)).unwrap()]);
    assert_eq!(prop.tiers().unwrap().as_str(), "T1");
    assert_eq!(p.groupes()[0].evaluations_reference(), 2);
}

#[test]
fn proposition_par_montant_exact_et_candidat_unique() {
    let r = registre([
        ligne(1, Sens::Debit, 100, Some("T1"), 0),
        ligne(2, Sens::Debit, 100, Some("T1"), 1),
        ligne(3, Sens::Credit, 100, Some("T1"), 5),
        ligne(4, Sens::Debit, 70, Some("T2"), 0),
        ligne(5, Sens::Credit, 70, Some("T3"), 0),
    ]);
    let p = r.proposer(&Bornes::default());
    // Le règlement a deux candidats, mais la facture la plus ancienne n'en a qu'un : elle
    // est rapprochée la première. Aucun rapprochement entre tiers différents.
    assert_eq!(p.propositions().len(), 1);
    assert_eq!(p.propositions()[0].regle(), Regle::MontantExact);
    assert_eq!(p.propositions()[0].lignes(), &[id(1), id(3)]);
    assert_eq!(p.groupes().len(), 3);
}

#[test]
fn proposition_par_combinaison_bornee() {
    let lignes = [
        ligne(1, Sens::Debit, 40, None, 0),
        ligne(2, Sens::Debit, 25, None, 1),
        ligne(3, Sens::Debit, 60, None, 2),
        ligne(4, Sens::Credit, 100, None, 5),
    ];
    let r = registre(lignes.clone());
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
    assert_eq!(p.groupes()[0].evaluations_combinaison(), 1);
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

/// Ligne générée : (débit ?, montant, tiers, jour, échéance).
type Genere = (bool, i64, u8, i64, Option<i64>);

fn lignes_generees() -> impl Strategy<Value = Vec<Genere>> {
    prop::collection::vec(
        (
            any::<bool>(),
            1i64..200,
            0u8..2,
            0i64..60,
            prop::option::of(0i64..90),
        ),
        1..12,
    )
}

fn construire(generees: &[Genere]) -> Vec<LigneLettrable> {
    generees
        .iter()
        .enumerate()
        .map(|(i, &(debit, montant, tiers, j, echeance))| {
            let tiers = format!("T{tiers}");
            let sens = if debit { Sens::Debit } else { Sens::Credit };
            ligne_complete(i as u64 + 1, sens, montant, Some(&tiers), j, echeance, None)
        })
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

/// Applique des opérations : lettrages par masque, ruptures de la lettre d'indice choisi.
fn appliquer(registre: &mut RegistreLettrage, operations: &[(u16, Option<u8>)]) {
    let ex = exercices_ouverts();
    for (n, (masque, rupture)) in operations.iter().enumerate() {
        let date = jour(100 + i64::try_from(n).unwrap());
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

proptest! {
    #[test]
    fn conservation_des_montants_et_bornes_du_residuel(
        generees in lignes_generees(),
        operations in prop::collection::vec((any::<u16>(), prop::option::of(any::<u8>())), 0..8),
    ) {
        let lignes = construire(&generees);
        let mut r = registre(lignes.clone());
        appliquer(&mut r, &operations);
        // Un lettrage ne change aucune ligne (ni donc aucun solde).
        prop_assert_eq!(r.lignes().cloned().collect::<Vec<_>>(), lignes);
        let mut impute: BTreeMap<IdLigne, Decimal> = BTreeMap::new();
        for lettrage in r.lettrages().filter(|l| r.rupture(l.lettre()).is_none()) {
            for lien in lettrage.liens() {
                prop_assert!(lien.montant() > Decimal::ZERO);
                *impute.entry(lien.debit()).or_default() += lien.montant();
                *impute.entry(lien.credit()).or_default() += lien.montant();
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
    }

    #[test]
    fn imputation_deterministe(generees in lignes_generees(), masque in any::<u16>()) {
        let r = registre(construire(&generees));
        let choisies = selection(&r, masque);
        let mut inverse = choisies.clone();
        inverse.reverse();
        prop_assert_eq!(r.imputer(&choisies), r.imputer(&inverse));
    }

    #[test]
    fn rupture_retablit_les_residuels_et_ne_supprime_rien(
        generees in lignes_generees(),
        operations in prop::collection::vec((any::<u16>(), Just(None)), 0..4),
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
        r.rompre(lettre, jour(201), motif(), &exercices_ouverts()).unwrap();
        prop_assert_eq!(residuels(&r), avant);
        prop_assert_eq!(r.lettrages().count(), nombre);
        prop_assert!(r.lettrage(lettre).is_some() && r.rupture(lettre).is_some());
        // Lettrer, rompre puis relettrer = lettrer une fois.
        let relettre = r.lettrer(&selection(&r, masque), jour(202), Origine::Manuelle).unwrap();
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
    fn propositions_bornees_deterministes_et_applicables(
        generees in lignes_generees(),
        operations in prop::collection::vec((any::<u16>(), Just(None)), 0..3),
        lignes_max in 2usize..8,
        budget in 1u64..400,
        ordre in any::<u64>(),
    ) {
        let lignes = construire(&generees);
        let mut r = registre(lignes.clone());
        appliquer(&mut r, &operations);
        let bornes = Bornes::nouvelles(lignes_max, budget).unwrap();
        let propositions = r.proposer(&bornes);
        for groupe in propositions.groupes() {
            prop_assert!(groupe.evaluations_reference() <= budget);
            prop_assert!(groupe.evaluations_combinaison() <= budget);
        }
        // Indépendant de l'ordre d'ajout des lignes.
        let mut melangees = lignes.clone();
        let decalage = usize::try_from(ordre % lignes.len() as u64).unwrap();
        melangees.rotate_left(decalage);
        melangees.reverse();
        let mut autre = registre(melangees);
        appliquer(&mut autre, &operations);
        if residuels(&autre) == residuels(&r) {
            prop_assert_eq!(autre.proposer(&bornes), propositions.clone());
        }
        // Lignes disjointes, combinaisons bornées, application par lettrage à l'identique.
        let mut vues = BTreeSet::new();
        for prop in propositions.propositions() {
            prop_assert!(prop.lignes().len() <= lignes_max.max(2));
            if prop.regle() != Regle::Combinaison {
                prop_assert_eq!(prop.lignes().len(), 2);
            }
            for id in prop.lignes() {
                prop_assert!(vues.insert(*id));
            }
            let lettrage = r.lettrer(prop.lignes(), jour(300), Origine::PropositionValidee).unwrap();
            prop_assert_eq!(lettrage.liens(), prop.liens());
            if prop.regle() != Regle::Reference {
                let lettre = lettrage.lettre();
                prop_assert!(r.est_total(lettre), "montant exact et combinaison soldent le groupe");
            }
        }
    }
}
