//! Tests du pointage, de l'état de rapprochement et des avertissements de solde : cas
//! unitaires, cas de référence chiffré et propriétés contre un oracle naïf (ADR 0034, point 9).

use std::collections::BTreeSet;

use proptest::prelude::*;
use rust_decimal::Decimal;
use time::macros::date;
use time::{Date, Duration};

use super::*;
use crate::ecriture::{EcritureBrouillon, Justification, ReferencePiece};
use crate::journal::CodeJournal;
use crate::montant::{Devise, DeviseTenue, Montant};
use crate::numerotation::NumeroEcriture;
use crate::texte::TexteCourt;

/// Compte de test (aucun numéro n'est codé hors des tests).
const COMPTE: &str = "5211";

fn xof() -> DeviseTenue {
    DeviseTenue::nouvelle(Devise::parse("XOF").unwrap(), 0).unwrap()
}

fn jour(n: i64) -> Date {
    date!(2026 - 01 - 01) + Duration::days(n)
}

fn d(n: i64) -> Decimal {
    Decimal::from(n)
}

fn compte() -> NumeroCompte {
    NumeroCompte::parse(COMPTE).unwrap()
}

fn ligne_ecriture(compte: &str, signe: i64) -> Ligne {
    let sens = if signe > 0 { Sens::Debit } else { Sens::Credit };
    let montant = Montant::nouveau(d(signe.abs()), Devise::parse("XOF").unwrap());
    Ligne::en_devise_tenue(NumeroCompte::parse(compte).unwrap(), sens, montant, &xof()).unwrap()
}

fn ide(rang: u64) -> IdLigneEcriture {
    IdLigneEcriture::nouveau(rang, 1).unwrap()
}

fn idr(n: u64) -> IdLigneReleve {
    IdLigneReleve::nouveau(n)
}

/// Ligne d'écriture de rang `rang`, montant signé (débit positif), à la date `date`.
fn lc(rang: u64, signe: i64, date: Date) -> LigneComptable {
    LigneComptable::nouvelle(ide(rang), &ligne_ecriture(COMPTE, signe), date)
}

fn lr(n: u64, montant: i64, date: Date) -> LigneReleve {
    LigneReleve::nouvelle(idr(n), date, d(montant))
}

fn registre(releve: Vec<LigneReleve>, comptables: Vec<LigneComptable>) -> RegistrePointage {
    let mut registre = RegistrePointage::ouvrir(compte());
    for l in releve {
        registre.ajouter_releve(l).unwrap();
    }
    for l in comptables {
        registre.ajouter_comptable(l).unwrap();
    }
    registre
}

fn ids_releve(etat: &EtatRapprochement) -> Vec<IdLigneReleve> {
    etat.releve_non_pointees()
        .iter()
        .map(LigneReleve::id)
        .collect()
}

fn ids_comptables(etat: &EtatRapprochement) -> Vec<IdLigneEcriture> {
    etat.comptables_non_pointees()
        .iter()
        .map(LigneComptable::id)
        .collect()
}

// ---------------------------------------------------------------------------------------------
// Lignes et identifiants
// ---------------------------------------------------------------------------------------------

#[test]
fn identifiant_de_ligne_d_ecriture() {
    assert!(IdLigneEcriture::nouveau(0, 1).is_err());
    assert!(IdLigneEcriture::nouveau(1, 0).is_err());
    let id = IdLigneEcriture::nouveau(7, 2).unwrap();
    assert_eq!((id.rang(), id.position()), (7, 2));
    assert_eq!(id.to_string(), "7/2");
    assert_eq!(idr(12).to_string(), "R12");
    assert_eq!(idr(12).valeur(), 12);
}

#[test]
fn montant_signe_selon_le_sens() {
    let debit = lc(1, 500, jour(0));
    let credit = lc(2, -300, jour(0));
    assert_eq!(debit.montant(), d(500));
    assert_eq!(credit.montant(), d(-300));
    assert_eq!(debit.compte(), &compte());
    assert_eq!(credit.date(), jour(0));
}

#[test]
fn ligne_depuis_une_ecriture_validee() {
    let mut brouillon = EcritureBrouillon::nouvelle(
        CodeJournal::parse("BQ1").unwrap(),
        jour(3),
        TexteCourt::parse("frais").unwrap(),
    );
    brouillon.ajouter_ligne(ligne_ecriture("6311", 2_500));
    brouillon.ajouter_ligne(ligne_ecriture(COMPTE, -2_500));
    brouillon.justifier(
        Justification::pieces([ReferencePiece::parse(&"a".repeat(64)).unwrap()]).unwrap(),
    );
    let validee = EcritureValidee::reconstituer(
        brouillon.valider_equilibre().unwrap(),
        NumeroEcriture::parse("BQ1-2026-000001").unwrap(),
        jour(3),
        None,
    )
    .unwrap();
    let l = LigneComptable::depuis_validee(&validee, 4, 2).unwrap();
    assert_eq!(
        (l.id(), l.montant(), l.date()),
        (IdLigneEcriture::nouveau(4, 2).unwrap(), d(-2_500), jour(3))
    );
    assert_eq!(
        LigneComptable::depuis_validee(&validee, 4, 3),
        Err(ErreurRapprochement::LigneComptableInconnue(
            IdLigneEcriture::nouveau(4, 3).unwrap()
        ))
    );
    assert!(LigneComptable::depuis_validee(&validee, 4, 0).is_err());
    // La ligne de charge n'est pas sur le compte de trésorerie.
    let charge = LigneComptable::depuis_validee(&validee, 4, 1).unwrap();
    let mut r = RegistrePointage::ouvrir(compte());
    assert_eq!(
        r.ajouter_comptable(charge),
        Err(ErreurRapprochement::AutreCompte(
            IdLigneEcriture::nouveau(4, 1).unwrap()
        ))
    );
}

#[test]
fn lignes_en_double_dans_le_registre() {
    let mut r = registre(vec![lr(1, 10, jour(0))], vec![lc(1, 10, jour(0))]);
    assert_eq!(
        r.ajouter_releve(lr(1, 20, jour(1))),
        Err(ErreurRapprochement::LigneReleveEnDouble(idr(1)))
    );
    assert_eq!(
        r.ajouter_comptable(lc(1, 20, jour(1))),
        Err(ErreurRapprochement::LigneComptableEnDouble(ide(1)))
    );
    assert_eq!(r.lignes_releve().count(), 1);
    assert_eq!(r.lignes_comptables().count(), 1);
}

// ---------------------------------------------------------------------------------------------
// Pointage
// ---------------------------------------------------------------------------------------------

#[test]
fn pointage_un_pour_un_et_n_n() {
    let mut r = registre(
        vec![
            lr(1, 500, jour(1)),
            lr(2, -10, jour(2)),
            lr(3, -10, jour(2)),
        ],
        vec![lc(1, 500, jour(0)), lc(2, -15, jour(2)), lc(3, -5, jour(2))],
    );
    let p = r.pointer(&[idr(1)], &[ide(1)], jour(5)).unwrap();
    assert_eq!((p.rang(), p.date(), p.montant()), (1, jour(5), d(500)));
    // N-N : deux retraits de 10 au relevé, 15 et 5 en comptabilité.
    let p = r
        .pointer(&[idr(3), idr(2)], &[ide(3), ide(2)], jour(5))
        .unwrap();
    assert_eq!(p.rang(), 2);
    assert_eq!(p.montant(), d(-20));
    assert_eq!(
        p.releve().iter().copied().collect::<Vec<_>>(),
        vec![idr(2), idr(3)]
    );
    assert_eq!(
        p.comptables().iter().copied().collect::<Vec<_>>(),
        vec![ide(2), ide(3)]
    );
    assert_eq!(r.pointage_releve(idr(2)).map(Pointage::rang), Some(2));
    assert_eq!(r.pointage_comptable(ide(1)).map(Pointage::rang), Some(1));
    assert_eq!(r.pointages().count(), 2);
}

#[test]
fn pointage_desequilibre_refuse_sans_tolerance() {
    let mut r = registre(
        vec![LigneReleve::nouvelle(
            idr(1),
            jour(0),
            Decimal::new(10_001, 2),
        )],
        vec![lc(1, 100, jour(0))],
    );
    assert_eq!(
        r.pointer(&[idr(1)], &[ide(1)], jour(1)),
        Err(ErreurRapprochement::Desequilibre {
            releve: Decimal::new(10_001, 2),
            comptable: d(100),
        })
    );
    // Sens opposés : +100 au relevé contre un crédit de 100.
    let mut r = registre(vec![lr(1, 100, jour(0))], vec![lc(1, -100, jour(0))]);
    assert!(matches!(
        r.pointer(&[idr(1)], &[ide(1)], jour(1)),
        Err(ErreurRapprochement::Desequilibre { .. })
    ));
    assert_eq!(r.pointages().count(), 0);
    assert_eq!(r.historique().len(), 0);
}

#[test]
fn echelles_differentes_meme_valeur() {
    let mut r = registre(
        vec![LigneReleve::nouvelle(
            idr(1),
            jour(0),
            Decimal::new(10_000, 2),
        )],
        vec![lc(1, 100, jour(0))],
    );
    assert!(r.pointer(&[idr(1)], &[ide(1)], jour(1)).is_ok());
}

#[test]
fn selections_invalides() {
    let mut r = registre(
        vec![lr(1, 100, jour(0)), lr(2, 100, jour(0))],
        vec![lc(1, 100, jour(0)), lc(2, 100, jour(0))],
    );
    assert_eq!(
        r.pointer(&[], &[ide(1)], jour(1)),
        Err(ErreurRapprochement::SansLigneReleve)
    );
    assert_eq!(
        r.pointer(&[idr(1)], &[], jour(1)),
        Err(ErreurRapprochement::SansLigneComptable)
    );
    assert_eq!(
        r.pointer(&[idr(9)], &[ide(1)], jour(1)),
        Err(ErreurRapprochement::LigneReleveInconnue(idr(9)))
    );
    assert_eq!(
        r.pointer(&[idr(1)], &[ide(9)], jour(1)),
        Err(ErreurRapprochement::LigneComptableInconnue(ide(9)))
    );
    assert_eq!(
        r.pointer(&[idr(1), idr(1)], &[ide(1), ide(2)], jour(1)),
        Err(ErreurRapprochement::LigneReleveEnDouble(idr(1)))
    );
    assert_eq!(
        r.pointer(&[idr(1), idr(2)], &[ide(1), ide(1)], jour(1)),
        Err(ErreurRapprochement::LigneComptableEnDouble(ide(1)))
    );
    assert_eq!(r.historique().len(), 0);
}

#[test]
fn une_ligne_dans_un_seul_pointage_non_rompu() {
    let mut r = registre(
        vec![lr(1, 100, jour(0)), lr(2, 100, jour(0))],
        vec![lc(1, 100, jour(0)), lc(2, 100, jour(0))],
    );
    r.pointer(&[idr(1)], &[ide(1)], jour(1)).unwrap();
    // Ni la ligne de relevé ni la ligne d'écriture ne peuvent entrer dans un second pointage.
    assert_eq!(
        r.pointer(&[idr(1)], &[ide(2)], jour(2)),
        Err(ErreurRapprochement::ReleveDejaPointee {
            ligne: idr(1),
            pointage: 1,
        })
    );
    assert_eq!(
        r.pointer(&[idr(2)], &[ide(1)], jour(2)),
        Err(ErreurRapprochement::ComptableDejaPointee {
            ligne: ide(1),
            pointage: 1,
        })
    );
    // Après rupture, la ligne se pointe à nouveau à partir de la date de rupture.
    r.rompre(1, jour(3)).unwrap();
    assert_eq!(r.pointage_releve(idr(1)), None);
    // Un pointage daté avant la rupture chevaucherait le pointage rompu.
    assert_eq!(
        r.pointer(&[idr(1)], &[ide(2)], jour(2)),
        Err(ErreurRapprochement::ReleveDejaPointee {
            ligne: idr(1),
            pointage: 1,
        })
    );
    let p = r.pointer(&[idr(1)], &[ide(2)], jour(3)).unwrap();
    assert_eq!(p.rang(), 3);
    assert_eq!(r.pointage_releve(idr(1)).map(Pointage::rang), Some(3));
}

#[test]
fn rupture_ajoutee_sans_rien_supprimer() {
    let mut r = registre(vec![lr(1, 100, jour(0))], vec![lc(1, 100, jour(0))]);
    r.pointer(&[idr(1)], &[ide(1)], jour(1)).unwrap();
    let rupture = *r.rompre(1, jour(4)).unwrap();
    assert_eq!(
        (rupture.rang(), rupture.pointage(), rupture.date()),
        (2, 1, jour(4))
    );
    assert_eq!(r.rupture(1), Some(&rupture));
    // Le pointage rompu reste dans le registre, avec ses lignes.
    assert_eq!(r.pointage(1).map(|p| p.releve().len()), Some(1));
    assert_eq!(r.pointages().count(), 1);
    assert_eq!(r.historique().len(), 2);
    assert_eq!(r.rompre(1, jour(5)), Err(ErreurRapprochement::DejaRompu(1)));
    assert_eq!(
        r.rompre(9, jour(5)),
        Err(ErreurRapprochement::PointageInconnu(9))
    );
    assert_eq!(r.historique().len(), 2);
}

#[test]
fn en_vigueur_selon_les_dates() {
    let mut r = registre(vec![lr(1, 100, jour(0))], vec![lc(1, 100, jour(0))]);
    r.pointer(&[idr(1)], &[ide(1)], jour(5)).unwrap();
    r.rompre(1, jour(9)).unwrap();
    assert!(!r.en_vigueur_a(1, jour(4)));
    assert!(r.en_vigueur_a(1, jour(5)));
    assert!(r.en_vigueur_a(1, jour(8)));
    assert!(!r.en_vigueur_a(1, jour(9)));
    assert!(!r.en_vigueur_a(2, jour(9)));
}

#[test]
fn rejouer_historique_tout_ou_rien() {
    let lignes_r = vec![lr(1, 100, jour(0)), lr(2, -40, jour(1))];
    let lignes_c = vec![lc(1, 100, jour(0)), lc(2, -40, jour(1))];
    let mut r = registre(lignes_r.clone(), lignes_c.clone());
    r.pointer(&[idr(1)], &[ide(1)], jour(2)).unwrap();
    r.rompre(1, jour(3)).unwrap();
    r.pointer(&[idr(1)], &[ide(1)], jour(3)).unwrap();
    r.pointer(&[idr(2)], &[ide(2)], jour(3)).unwrap();
    let mut historique = r.historique();
    historique.reverse();
    let mut relu = registre(lignes_r.clone(), lignes_c.clone());
    relu.rejouer(historique).unwrap();
    assert_eq!(relu, r);

    // Rang en double ou nul : refus, registre inchangé.
    let vide = registre(lignes_r.clone(), lignes_c.clone());
    let mut copie = vide.clone();
    let operation = Operation::Rupture {
        pointage: 1,
        date: jour(3),
    };
    assert_eq!(
        copie.rejouer([(0, operation.clone())]),
        Err(ErreurRapprochement::RangOperationInvalide(0))
    );
    let pointage = Operation::Pointage {
        date: jour(2),
        releve: vec![idr(1)],
        comptables: vec![ide(1)],
    };
    assert_eq!(
        copie.rejouer([(1, pointage.clone()), (1, operation.clone())]),
        Err(ErreurRapprochement::RangOperationInvalide(1))
    );
    // Opération invalide en fin de liste : rien n'est inscrit.
    let desequilibre = Operation::Pointage {
        date: jour(2),
        releve: vec![idr(2)],
        comptables: vec![ide(1)],
    };
    assert!(
        copie
            .rejouer([(1, pointage.clone()), (2, desequilibre)])
            .is_err()
    );
    assert_eq!(copie, vide);
    // Un rang déjà présent dans le registre est refusé.
    copie.rejouer([(1, pointage.clone())]).unwrap();
    assert_eq!(
        copie.rejouer([(1, operation)]),
        Err(ErreurRapprochement::RangOperationInvalide(1))
    );
}

#[test]
fn depassement_de_capacite_sans_panique() {
    let mut r = registre(
        vec![
            LigneReleve::nouvelle(idr(1), jour(0), Decimal::MAX),
            LigneReleve::nouvelle(idr(2), jour(0), Decimal::MAX),
        ],
        vec![lc(1, 1, jour(0))],
    );
    assert_eq!(
        r.pointer(&[idr(1), idr(2)], &[ide(1)], jour(1)),
        Err(ErreurRapprochement::Depassement)
    );
    assert_eq!(
        r.etat(jour(1), Ouverture::default()),
        Err(ErreurRapprochement::Depassement)
    );
    let r = registre(vec![lr(1, -1, jour(0))], vec![]);
    assert_eq!(
        r.etat(jour(1), Ouverture::nouvelle(Decimal::MIN, Decimal::ZERO)),
        Err(ErreurRapprochement::Depassement)
    );
}

// ---------------------------------------------------------------------------------------------
// État de rapprochement : cas de référence chiffré (critère de fin du lot, plan L3)
// ---------------------------------------------------------------------------------------------

/// Compte bancaire, janvier 2026, en XOF ; soldes d'ouverture concordants (1 000 000).
///
/// | Comptabilité | Relevé |
/// |---|---|
/// | C1 05/01 encaissement client +500 000 | R1 06/01 +500 000 |
/// | C2 10/01 chèque émis −200 000 (non encaissé) | — |
/// | C3 12/01 virement fournisseur −150 000 | R2 13/01 −150 000 |
/// | C4, C5 20/01 deux retraits de −10 000 (doublon légitime) | R3, R4 20/01 −10 000 ×2 |
/// | — | R5 25/01 frais de banque −5 000 (non comptabilisés) |
/// | — | R6 28/01 encaissement +75 000 (non comptabilisé) |
/// | C6 31/01 remise de chèques +300 000 | R7 02/02 +300 000 |
fn cas_de_reference() -> RegistrePointage {
    registre(
        vec![
            lr(1, 500_000, date!(2026 - 01 - 06)),
            lr(2, -150_000, date!(2026 - 01 - 13)),
            lr(3, -10_000, date!(2026 - 01 - 20)),
            lr(4, -10_000, date!(2026 - 01 - 20)),
            lr(5, -5_000, date!(2026 - 01 - 25)),
            lr(6, 75_000, date!(2026 - 01 - 28)),
            lr(7, 300_000, date!(2026 - 02 - 02)),
        ],
        vec![
            lc(1, 500_000, date!(2026 - 01 - 05)),
            lc(2, -200_000, date!(2026 - 01 - 10)),
            lc(3, -150_000, date!(2026 - 01 - 12)),
            lc(4, -10_000, date!(2026 - 01 - 20)),
            lc(5, -10_000, date!(2026 - 01 - 20)),
            lc(6, 300_000, date!(2026 - 01 - 31)),
        ],
    )
}

fn ouverture_concordante() -> Ouverture {
    Ouverture::nouvelle(d(1_000_000), d(1_000_000))
}

#[test]
fn reference_etat_au_31_janvier() {
    let mut r = cas_de_reference();
    let fin_janvier = date!(2026 - 01 - 31);
    r.pointer(&[idr(1)], &[ide(1)], fin_janvier).unwrap();
    r.pointer(&[idr(2)], &[ide(3)], fin_janvier).unwrap();
    r.pointer(&[idr(3), idr(4)], &[ide(4), ide(5)], fin_janvier)
        .unwrap();
    // La remise est pointée en février, à réception du relevé.
    r.pointer(&[idr(7)], &[ide(6)], date!(2026 - 02 - 03))
        .unwrap();

    let etat = r.etat(fin_janvier, ouverture_concordante()).unwrap();
    assert_eq!(etat.date(), fin_janvier);
    assert_eq!(etat.solde_comptable(), d(1_430_000));
    assert_eq!(etat.solde_releve(), d(1_400_000));
    assert_eq!(ids_comptables(&etat), vec![ide(2), ide(6)]);
    assert_eq!(etat.total_comptables_non_pointees(), d(100_000));
    assert_eq!(ids_releve(&etat), vec![idr(5), idr(6)]);
    assert_eq!(etat.total_releve_non_pointees(), d(70_000));
    assert_eq!(etat.pointages(), &[1, 2, 3]);
    // (1 430 000 − 100 000) − (1 400 000 − 70 000) = 1 330 000 − 1 330 000 = 0.
    assert_eq!(etat.ecart(), Decimal::ZERO);
    assert!(etat.ecart_nul());

    // Février : frais et encaissement comptabilisés (C7, C8) puis pointés ; il ne reste que le
    // chèque émis non encaissé.
    r.ajouter_comptable(lc(7, -5_000, date!(2026 - 01 - 25)))
        .unwrap();
    r.ajouter_comptable(lc(8, 75_000, date!(2026 - 01 - 28)))
        .unwrap();
    r.pointer(&[idr(5)], &[ide(7)], date!(2026 - 02 - 05))
        .unwrap();
    r.pointer(&[idr(6)], &[ide(8)], date!(2026 - 02 - 05))
        .unwrap();
    let fevrier = r
        .etat(date!(2026 - 02 - 28), ouverture_concordante())
        .unwrap();
    assert_eq!(fevrier.solde_comptable(), d(1_500_000));
    assert_eq!(fevrier.solde_releve(), d(1_700_000));
    assert_eq!(ids_comptables(&fevrier), vec![ide(2)]);
    assert_eq!(ids_releve(&fevrier).len(), 0);
    assert!(fevrier.ecart_nul());

    // L'état du 31 janvier recalculé après coup est inchangé, à ceci près que les lignes C7 et
    // C8, datées de janvier, y figurent désormais comme non pointées à cette date.
    let rejoue = r.etat(fin_janvier, ouverture_concordante()).unwrap();
    assert_eq!(ids_releve(&rejoue), vec![idr(5), idr(6)]);
    assert_eq!(
        ids_comptables(&rejoue),
        vec![ide(2), ide(7), ide(8), ide(6)]
    );
    assert!(rejoue.ecart_nul());
}

#[test]
fn ouvertures_discordantes_donnent_l_ecart() {
    let mut r = cas_de_reference();
    r.pointer(&[idr(1)], &[ide(1)], jour(30)).unwrap();
    let etat = r
        .etat(jour(30), Ouverture::nouvelle(d(990_000), d(1_000_000)))
        .unwrap();
    assert_eq!(etat.ecart(), d(10_000));
    assert!(!etat.ecart_nul());
    assert_eq!(etat.ouverture().releve(), d(990_000));
    assert_eq!(etat.ouverture().comptable(), d(1_000_000));
}

#[test]
fn pointage_qui_enjambe_la_date() {
    // Relevé du 30, écriture du 2 du mois suivant, pointées le 30 : à la fin du mois, la ligne
    // de relevé reste listée (chevauchement de dates).
    let mut r = registre(
        vec![lr(1, 100, date!(2026 - 01 - 30))],
        vec![lc(1, 100, date!(2026 - 02 - 02))],
    );
    r.pointer(&[idr(1)], &[ide(1)], date!(2026 - 01 - 30))
        .unwrap();
    let etat = r.etat(date!(2026 - 01 - 31), Ouverture::default()).unwrap();
    assert_eq!(ids_releve(&etat), vec![idr(1)]);
    assert_eq!(ids_comptables(&etat).len(), 0);
    assert_eq!(etat.pointages().len(), 0);
    assert!(etat.ecart_nul());
    let etat = r.etat(date!(2026 - 02 - 02), Ouverture::default()).unwrap();
    assert_eq!(ids_releve(&etat).len(), 0);
    assert_eq!(etat.pointages(), &[1]);
}

#[test]
fn reconstitution_par_dates_d_operation() {
    let mut r = registre(vec![lr(1, 100, jour(0))], vec![lc(1, 100, jour(0))]);
    r.pointer(&[idr(1)], &[ide(1)], jour(5)).unwrap();
    r.rompre(1, jour(9)).unwrap();
    let liste = |n| ids_releve(&r.etat(jour(n), Ouverture::default()).unwrap());
    assert_eq!(liste(4), vec![idr(1)]);
    assert_eq!(liste(5).len(), 0);
    assert_eq!(liste(8).len(), 0);
    assert_eq!(liste(9), vec![idr(1)]);
    // Les lignes postérieures à la date n'entrent pas dans l'état.
    let r = registre(vec![lr(1, 100, jour(3))], vec![lc(1, 50, jour(3))]);
    let etat = r.etat(jour(2), Ouverture::default()).unwrap();
    assert_eq!(etat.solde_releve(), Decimal::ZERO);
    assert_eq!(etat.solde_comptable(), Decimal::ZERO);
    assert_eq!(etat.releve_non_pointees().len(), 0);
}

// ---------------------------------------------------------------------------------------------
// Avertissements de solde (D2)
// ---------------------------------------------------------------------------------------------

#[test]
fn avertissements_de_solde() {
    use AvertissementSolde as A;
    use TypeTresorerie as T;
    assert_eq!(
        avertissements_solde(T::Caisse, Decimal::ZERO, None).len(),
        0
    );
    assert_eq!(avertissements_solde(T::Caisse, d(10), None).len(), 0);
    assert_eq!(
        avertissements_solde(T::Caisse, d(-1), None),
        vec![A::CaisseSoldeCrediteur { solde: d(-1) }]
    );
    assert_eq!(
        avertissements_solde(T::Portefeuille, d(-1), Some(d(5))),
        vec![A::PortefeuilleSoldeCrediteur { solde: d(-1) }]
    );
    assert_eq!(
        avertissements_solde(T::Portefeuille, d(50), Some(d(5))).len(),
        0
    );
    // Banque créditrice : découvert, aucun avertissement (fiche 52).
    assert_eq!(
        avertissements_solde(T::Banque, d(-1_000_000), Some(d(5))).len(),
        0
    );
    // Plafond de caisse : strictement dépassé.
    assert_eq!(
        avertissements_solde(T::Caisse, d(500), Some(d(500))).len(),
        0
    );
    assert_eq!(
        avertissements_solde(T::Caisse, d(501), Some(d(500))),
        vec![A::CaissePlafondDepasse {
            solde: d(501),
            plafond: d(500),
        }]
    );
    // Un plafond négatif (que rien n'interdit) et un solde créditeur : deux avertissements.
    let deux = avertissements_solde(T::Caisse, d(-1), Some(d(-10)));
    assert_eq!(
        deux.iter()
            .map(AvertissementSolde::code)
            .collect::<Vec<_>>(),
        vec!["caisse_solde_crediteur", "caisse_plafond_depasse"]
    );
    assert_eq!(
        A::PortefeuilleSoldeCrediteur { solde: d(-1) }.code(),
        "portefeuille_solde_crediteur"
    );
}

// ---------------------------------------------------------------------------------------------
// Propriétés contre un oracle naïf
// ---------------------------------------------------------------------------------------------

/// Groupe de lignes équilibré, pointé une fois, éventuellement rompu puis repointé.
#[derive(Debug, Clone)]
struct Groupe {
    /// (montant signé, jour) de chaque ligne de relevé.
    releve: Vec<(i64, i64)>,
    /// (montant signé non nul, jour) de chaque ligne d'écriture.
    comptables: Vec<(i64, i64)>,
    /// Jour du pointage.
    pointage: i64,
    /// Jour de la rupture et, s'il y a lieu, délai du repointage après la rupture.
    rupture: Option<(i64, Option<i64>)>,
}

#[derive(Debug, Clone)]
struct Scenario {
    groupes: Vec<Groupe>,
    /// Ordre d'application des groupes.
    ordre: Vec<usize>,
    seuls_releve: Vec<(i64, i64)>,
    seuls_comptables: Vec<(i64, i64)>,
    ouverture_releve: i64,
    ouverture_comptable: i64,
}

fn montant() -> impl Strategy<Value = i64> {
    prop_oneof![-500i64..=-1, 1i64..=500]
}

fn groupe() -> impl Strategy<Value = Groupe> {
    (
        prop::collection::vec((montant(), 0i64..40, 0i64..40), 1..4),
        0u8..3,
        0i64..40,
        0i64..50,
        prop::option::of((0i64..60, prop::option::of(0i64..10))),
    )
        .prop_map(|(parts, variante, jour_fusion, pointage, rupture)| {
            let somme: i64 = parts.iter().map(|p| p.0).sum();
            let detail_r: Vec<(i64, i64)> = parts.iter().map(|p| (p.0, p.1)).collect();
            let detail_c: Vec<(i64, i64)> = parts.iter().map(|p| (p.0, p.2)).collect();
            // Variantes N-N : mêmes montants des deux côtés, ou une seule ligne d'un côté.
            let (releve, comptables) = match variante {
                1 if somme != 0 => (detail_r, vec![(somme, jour_fusion)]),
                2 if somme != 0 => (vec![(somme, jour_fusion)], detail_c),
                _ => (detail_r, detail_c),
            };
            Groupe {
                releve,
                comptables,
                pointage,
                rupture,
            }
        })
}

fn scenario() -> impl Strategy<Value = Scenario> {
    (
        prop::collection::vec(groupe(), 0..6),
        prop::collection::vec((montant(), 0i64..40), 0..5),
        prop::collection::vec((montant(), 0i64..40), 0..5),
        -1_000i64..1_000,
        -1_000i64..1_000,
    )
        .prop_flat_map(|(groupes, seuls_releve, seuls_comptables, o_r, o_c)| {
            let n = groupes.len();
            (
                Just(groupes),
                Just((0..n).collect::<Vec<usize>>()).prop_shuffle(),
                Just(seuls_releve),
                Just(seuls_comptables),
                Just(o_r),
                Just(o_c),
            )
        })
        .prop_map(
            |(groupes, ordre, seuls_releve, seuls_comptables, o_r, o_c)| Scenario {
                groupes,
                ordre,
                seuls_releve,
                seuls_comptables,
                ouverture_releve: o_r,
                ouverture_comptable: o_c,
            },
        )
}

/// Lignes matérialisées : identifiants attribués dans l'ordre de génération.
struct Materiel {
    releve: Vec<LigneReleve>,
    comptables: Vec<LigneComptable>,
    /// Identifiants des lignes de chaque groupe.
    groupes: Vec<(Vec<IdLigneReleve>, Vec<IdLigneEcriture>)>,
}

fn materialiser(s: &Scenario) -> Materiel {
    let mut releve = Vec::new();
    let mut comptables = Vec::new();
    let mut ajouter_r = |(m, j): (i64, i64)| {
        let id = idr(u64::try_from(releve.len()).unwrap() + 1);
        releve.push(lr(id.valeur(), m, jour(j)));
        id
    };
    let mut groupes_r = Vec::new();
    for g in &s.groupes {
        groupes_r.push(g.releve.iter().map(|x| ajouter_r(*x)).collect::<Vec<_>>());
    }
    for x in &s.seuls_releve {
        ajouter_r(*x);
    }
    let mut ajouter_c = |(m, j): (i64, i64)| {
        let rang = u64::try_from(comptables.len()).unwrap() + 1;
        comptables.push(lc(rang, m, jour(j)));
        ide(rang)
    };
    let mut groupes_c = Vec::new();
    for g in &s.groupes {
        groupes_c.push(
            g.comptables
                .iter()
                .map(|x| ajouter_c(*x))
                .collect::<Vec<_>>(),
        );
    }
    for x in &s.seuls_comptables {
        ajouter_c(*x);
    }
    Materiel {
        releve,
        comptables,
        groupes: groupes_r.into_iter().zip(groupes_c).collect(),
    }
}

/// Registre construit en appliquant les groupes dans l'ordre donné ; les lignes sont ajoutées
/// dans l'ordre inverse si `inverser`.
fn construire(s: &Scenario, m: &Materiel, ordre: &[usize], inverser: bool) -> RegistrePointage {
    let mut r = RegistrePointage::ouvrir(compte());
    let mut lignes_r = m.releve.clone();
    let mut lignes_c = m.comptables.clone();
    if inverser {
        lignes_r.reverse();
        lignes_c.reverse();
    }
    for l in lignes_r {
        r.ajouter_releve(l).unwrap();
    }
    for l in lignes_c {
        r.ajouter_comptable(l).unwrap();
    }
    for &i in ordre {
        let g = &s.groupes[i];
        let (ids_r, ids_c) = &m.groupes[i];
        let rang = r.pointer(ids_r, ids_c, jour(g.pointage)).unwrap().rang();
        if let Some((f, repointage)) = g.rupture {
            r.rompre(rang, jour(f)).unwrap();
            if let Some(delai) = repointage {
                r.pointer(ids_r, ids_c, jour(f + delai)).unwrap();
            }
        }
    }
    r
}

/// Périodes de vigueur d'un groupe : `(début, fin exclue)`.
fn vigueurs(g: &Groupe) -> Vec<(i64, Option<i64>)> {
    match g.rupture {
        None => vec![(g.pointage, None)],
        Some((f, None)) => vec![(g.pointage, Some(f))],
        Some((f, Some(delai))) => vec![(g.pointage, Some(f)), (f + delai, None)],
    }
}

/// Oracle : état recalculé naïvement à partir du scénario, sans le registre.
struct EtatNaif {
    solde_releve: i64,
    solde_comptable: i64,
    releve_non_pointees: BTreeSet<IdLigneReleve>,
    comptables_non_pointees: BTreeSet<IdLigneEcriture>,
    ecart: i64,
}

fn oracle(s: &Scenario, m: &Materiel, n: i64) -> EtatNaif {
    let mut pointees_r = BTreeSet::new();
    let mut pointees_c = BTreeSet::new();
    for (g, (ids_r, ids_c)) in s.groupes.iter().zip(&m.groupes) {
        let lignes_avant =
            g.releve.iter().all(|x| x.1 <= n) && g.comptables.iter().all(|x| x.1 <= n);
        let actif = vigueurs(g)
            .iter()
            .any(|&(debut, fin)| debut <= n && fin.is_none_or(|f| f > n));
        if lignes_avant && actif {
            pointees_r.extend(ids_r.iter().copied());
            pointees_c.extend(ids_c.iter().copied());
        }
    }
    let mut solde_releve = s.ouverture_releve;
    let mut non_pointe_r = 0;
    let mut releve_non_pointees = BTreeSet::new();
    for l in &m.releve {
        if l.date() <= jour(n) {
            let v = i64::try_from(l.montant()).unwrap();
            solde_releve += v;
            if !pointees_r.contains(&l.id()) {
                non_pointe_r += v;
                releve_non_pointees.insert(l.id());
            }
        }
    }
    let mut solde_comptable = s.ouverture_comptable;
    let mut non_pointe_c = 0;
    let mut comptables_non_pointees = BTreeSet::new();
    for l in &m.comptables {
        if l.date() <= jour(n) {
            let v = i64::try_from(l.montant()).unwrap();
            solde_comptable += v;
            if !pointees_c.contains(&l.id()) {
                non_pointe_c += v;
                comptables_non_pointees.insert(l.id());
            }
        }
    }
    EtatNaif {
        solde_releve,
        solde_comptable,
        releve_non_pointees,
        comptables_non_pointees,
        ecart: (solde_comptable - non_pointe_c) - (solde_releve - non_pointe_r),
    }
}

fn ouverture(s: &Scenario) -> Ouverture {
    Ouverture::nouvelle(d(s.ouverture_releve), d(s.ouverture_comptable))
}

/// Vue d'un état sans les rangs de pointage (qui dépendent de l'ordre des opérations).
type Vue = (
    Decimal,
    Decimal,
    Vec<IdLigneReleve>,
    Vec<IdLigneEcriture>,
    Decimal,
);

fn vue(e: &EtatRapprochement) -> Vue {
    (
        e.solde_releve(),
        e.solde_comptable(),
        ids_releve(e),
        ids_comptables(e),
        e.ecart(),
    )
}

/// Jours d'observation : de part et d'autre de toutes les dates générées.
const JOURS: [i64; 10] = [0, 5, 12, 20, 27, 35, 42, 50, 60, 80];

proptest! {
    /// L'état du registre est celui de l'oracle naïf, à toute date ; l'écart suit la formule de
    /// l'ADR et vaut la différence des soldes d'ouverture.
    #[test]
    fn etat_conforme_a_l_oracle(s in scenario()) {
        let m = materialiser(&s);
        let r = construire(&s, &m, &s.ordre, false);
        for n in JOURS {
            let etat = r.etat(jour(n), ouverture(&s)).unwrap();
            let naif = oracle(&s, &m, n);
            prop_assert_eq!(etat.solde_releve(), d(naif.solde_releve));
            prop_assert_eq!(etat.solde_comptable(), d(naif.solde_comptable));
            prop_assert_eq!(
                ids_releve(&etat).into_iter().collect::<BTreeSet<_>>(),
                naif.releve_non_pointees
            );
            prop_assert_eq!(
                ids_comptables(&etat).into_iter().collect::<BTreeSet<_>>(),
                naif.comptables_non_pointees
            );
            prop_assert_eq!(etat.ecart(), d(naif.ecart));
            prop_assert_eq!(etat.ecart(), d(s.ouverture_comptable - s.ouverture_releve));
            prop_assert_eq!(
                etat.total_releve_non_pointees(),
                etat.releve_non_pointees().iter().map(LigneReleve::montant).sum::<Decimal>()
            );
            // Listes triées par date puis identifiant.
            let cles: Vec<_> = etat
                .releve_non_pointees()
                .iter()
                .map(|l| (l.date(), l.id()))
                .collect();
            let mut triees = cles.clone();
            triees.sort();
            prop_assert_eq!(cles, triees);
        }
    }

    /// Un pointage ne dépasse jamais une ligne : à toute date, une ligne est dans au plus un
    /// pointage en vigueur, et un repointage d'une ligne pointée est refusé.
    #[test]
    fn une_ligne_au_plus_un_pointage_en_vigueur(s in scenario(), j in 0i64..90) {
        let m = materialiser(&s);
        let mut r = construire(&s, &m, &s.ordre, false);
        for n in JOURS {
            for l in &m.releve {
                let nombre = r
                    .pointages()
                    .filter(|p| p.releve().contains(&l.id()) && r.en_vigueur_a(p.rang(), jour(n)))
                    .count();
                prop_assert!(nombre <= 1);
            }
            for l in &m.comptables {
                let nombre = r
                    .pointages()
                    .filter(|p| p.comptables().contains(&l.id()) && r.en_vigueur_a(p.rang(), jour(n)))
                    .count();
                prop_assert!(nombre <= 1);
            }
        }
        for (g, (ids_r, ids_c)) in s.groupes.iter().zip(&m.groupes) {
            let encore_pointe = !matches!(g.rupture, Some((_, None)));
            let avant = r.historique().len();
            let resultat = r.pointer(ids_r, ids_c, jour(j));
            if encore_pointe {
                let refuse = matches!(
                    resultat,
                    Err(ErreurRapprochement::ReleveDejaPointee { .. })
                );
                prop_assert!(refuse);
                prop_assert_eq!(r.historique().len(), avant);
            }
        }
    }

    /// Déterminisme : l'ordre des pointages, l'ordre d'ajout des lignes et la relecture de
    /// l'historique (dans n'importe quel ordre) ne changent pas l'état.
    #[test]
    fn determinisme_et_ordre_indifferent(s in scenario()) {
        let m = materialiser(&s);
        let r = construire(&s, &m, &s.ordre, false);
        let mut inverse = s.ordre.clone();
        inverse.reverse();
        let autre = construire(&s, &m, &inverse, true);
        let mut relu = construire(&s, &m, &[], true);
        let mut historique = r.historique();
        historique.reverse();
        relu.rejouer(historique).unwrap();
        prop_assert_eq!(&relu, &r);
        for n in JOURS {
            let etat = r.etat(jour(n), ouverture(&s)).unwrap();
            prop_assert_eq!(&r.etat(jour(n), ouverture(&s)).unwrap(), &etat);
            prop_assert_eq!(vue(&autre.etat(jour(n), ouverture(&s)).unwrap()), vue(&etat));
        }
    }

    /// La rupture ne supprime rien, et rompre puis repointer donne le même état que pointer
    /// une fois ; tout pointé avec des ouvertures concordantes, l'écart est nul.
    #[test]
    fn rupture_puis_repointage(s in scenario(), delai in 0i64..10) {
        let m = materialiser(&s);
        let nombre_lignes = (m.releve.len(), m.comptables.len());
        let mut simple = construire(&s, &m, &[], false);
        let mut double = construire(&s, &m, &[], false);
        for (i, (ids_r, ids_c)) in m.groupes.iter().enumerate() {
            let i = i64::try_from(i).unwrap();
            simple.pointer(ids_r, ids_c, jour(0)).unwrap();
            let rang = double.pointer(ids_r, ids_c, jour(0)).unwrap().rang();
            double.rompre(rang, jour(1 + i)).unwrap();
            double.pointer(ids_r, ids_c, jour(1 + i + delai)).unwrap();
        }
        // Rien n'est supprimé : chaque pointage rompu reste, avec ses lignes.
        prop_assert_eq!(double.pointages().count(), 2 * m.groupes.len());
        prop_assert_eq!(double.historique().len(), 3 * m.groupes.len());
        prop_assert_eq!(
            (double.lignes_releve().count(), double.lignes_comptables().count()),
            nombre_lignes
        );
        for p in double.pointages().filter(|p| double.rupture(p.rang()).is_some()) {
            let complet = !p.releve().is_empty() && !p.comptables().is_empty();
            prop_assert!(complet);
        }
        let fin = jour(100);
        let concordante = Ouverture::nouvelle(d(s.ouverture_releve), d(s.ouverture_releve));
        let etat_simple = simple.etat(fin, concordante).unwrap();
        prop_assert_eq!(vue(&double.etat(fin, concordante).unwrap()), vue(&etat_simple));
        prop_assert!(etat_simple.ecart_nul());
        // Seules les lignes hors groupe restent non pointées.
        prop_assert_eq!(etat_simple.releve_non_pointees().len(), s.seuls_releve.len());
        prop_assert_eq!(etat_simple.comptables_non_pointees().len(), s.seuls_comptables.len());
    }
}
