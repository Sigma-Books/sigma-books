//! Tests de l'échéancier : cas unitaires, cas de référence des moteurs et propriétés
//! (ADR 0033, point 3).
//!
//! Cas de référence repris de Tryton (`modules/account_invoice/tests/test_module.py`,
//! `test_payment_term` et `test_negative_amount`, commit
//! f5354ccd9231703470611d7ebf1c9cdc65e952bc, GPL-3.0-or-later, © 2008-2026 Cédric Krier,
//! © 2008-2026 B2CK SRL et autres titulaires du fichier COPYRIGHT ; voir
//! `docs/provenance/tryton.md#payment-term`) et d'Odoo (`addons/account/tests/test_payment_term.py`,
//! `test_payment_term_days_end_of_month_*`, commit fa175fb2b1dd69883188ddbd07d0d0b7a6cf9118,
//! LGPL-3.0, © Odoo S.A. ; voir `docs/provenance/odoo.md#delais-de-reglement`).

use proptest::prelude::*;
use rust_decimal::Decimal;
use time::macros::date;
use time::{Date, Duration, Month};

use super::*;
use crate::montant::{Devise, DeviseTenue, Montant};

fn tenue(decimales: u8) -> DeviseTenue {
    DeviseTenue::nouvelle(Devise::parse("ZZZ").unwrap(), decimales).unwrap()
}

fn xof() -> DeviseTenue {
    DeviseTenue::nouvelle(Devise::parse("XOF").unwrap(), 0).unwrap()
}

fn d(texte: &str) -> Decimal {
    texte.parse().unwrap()
}

fn pct(texte: &str, delai: Delai) -> Terme {
    Terme::nouveau(
        ValeurTerme::Pourcentage(Pourcentage::nouveau(d(texte)).unwrap()),
        delai,
    )
}

fn fixe(texte: &str, delai: Delai) -> Terme {
    Terme::nouveau(
        ValeurTerme::Fixe(MontantFixe::nouveau(d(texte)).unwrap()),
        delai,
    )
}

fn solde(delai: Delai) -> Terme {
    Terme::nouveau(ValeurTerme::Solde, delai)
}

fn condition(termes: Vec<Terme>) -> ConditionReglement {
    ConditionReglement::nouvelle(termes).unwrap()
}

fn repartir(
    c: &ConditionReglement,
    total: &str,
    t: DeviseTenue,
    reference: Date,
) -> Vec<(Date, String)> {
    c.repartir(Montant::nouveau(d(total), t.devise()), reference, &t)
        .unwrap()
        .iter()
        .map(|terme| (terme.date(), terme.montant().valeur().to_string()))
        .collect()
}

fn jour_du_mois(jour: u8) -> JourDuMois {
    JourDuMois::nouveau(jour).unwrap()
}

fn motif(texte: &str) -> MotifRevision {
    MotifRevision::parse(texte).unwrap()
}

fn residuel(montant: i64, reste: i64) -> Residuel {
    Residuel::nouveau(Decimal::from(montant), Decimal::from(reste)).unwrap()
}

// --- Répartition -------------------------------------------------------------------------

/// Tryton `test_payment_term` : 1 587,35 en 25 %, 25 %, 396,84 fixe et solde. Le premier terme
/// de Tryton est « 25 % du reste », ici « 25 % du total » (même montant : premier terme) ; les
/// délais `relativedelta` sont exprimés en jours (mêmes dates).
#[test]
fn reference_tryton_repartition() {
    let c = condition(vec![
        pct("25", Delai::Jours(30)),
        pct("25", Delai::Jours(31)),
        fixe("396.84", Delai::Jours(61)),
        solde(Delai::Jours(105)),
    ]);
    let attendu = [
        (date!(2011 - 10 - 31), "396.84"),
        (date!(2011 - 11 - 01), "396.84"),
        (date!(2011 - 12 - 01), "396.84"),
        (date!(2012 - 01 - 14), "396.83"),
    ];
    let obtenu = repartir(&c, "1587.35", tenue(2), date!(2011 - 10 - 01));
    assert_eq!(obtenu, attendu.map(|(j, m)| (j, m.to_owned())).to_vec());
}

/// Tryton `test_negative_amount` : le signe du total est reporté sur chaque terme.
#[test]
fn reference_tryton_montant_negatif() {
    let c = condition(vec![
        pct("25", Delai::Jours(30)),
        pct("25", Delai::Jours(31)),
        fixe("4", Delai::Jours(61)),
        solde(Delai::Jours(105)),
    ]);
    let montants: Vec<String> = repartir(&c, "-10.00", tenue(2), date!(2011 - 10 - 01))
        .into_iter()
        .map(|(_, m)| m)
        .collect();
    assert_eq!(montants, ["-2.50", "-2.50", "-4", "-1.00"]);
}

#[test]
fn total_nul_donne_un_terme_nul_a_la_reference() {
    let c = condition(vec![pct("25", Delai::Jours(1)), solde(Delai::Jours(30))]);
    assert_eq!(
        repartir(&c, "0", xof(), date!(2016 - 05 - 17)),
        [(date!(2016 - 05 - 17), "0".to_owned())]
    );
}

/// Un terme qui dépasse le reste est plafonné au reste ; les termes suivants, nuls, sont omis
/// (Tryton `payment_term.py:65-70` ; Odoo produirait un solde négatif de -200).
#[test]
fn terme_plafonne_au_reste() {
    let c = condition(vec![
        pct("70", Delai::Jours(0)),
        fixe("500", Delai::Jours(10)),
        pct("20", Delai::Jours(20)),
        solde(Delai::Jours(30)),
    ]);
    let r = date!(2016 - 01 - 01);
    assert_eq!(
        repartir(&c, "1000", xof(), r),
        [
            (r, "700".to_owned()),
            (date!(2016 - 01 - 11), "300".to_owned())
        ]
    );
}

#[test]
fn arrondi_xof_et_solde_absorbe_l_arrondi() {
    let c = condition(vec![
        pct("33.333333", Delai::Jours(0)),
        pct("33.333333", Delai::Jours(30)),
        solde(Delai::Jours(60)),
    ]);
    let montants: Vec<String> = repartir(&c, "100001", xof(), date!(2026 - 01 - 01))
        .into_iter()
        .map(|(_, m)| m)
        .collect();
    assert_eq!(montants, ["33334", "33334", "33333"]);
    // Demi éloigné de zéro : 50 % de 1 franc donne 1, le solde est nul et omis.
    let moitie = condition(vec![pct("50", Delai::Jours(0)), solde(Delai::Jours(30))]);
    assert_eq!(
        repartir(&moitie, "1", xof(), date!(2026 - 01 - 01)),
        [(date!(2026 - 01 - 01), "1".to_owned())]
    );
    // Montant fixe arrondi à l'échelle de la devise.
    let f = condition(vec![fixe("10.5", Delai::Jours(0)), solde(Delai::Jours(1))]);
    assert_eq!(repartir(&f, "100", xof(), date!(2026 - 01 - 01))[0].1, "11");
}

#[test]
fn resultat_trie_par_date_ordre_des_termes_a_date_egale() {
    let c = condition(vec![
        fixe("1", Delai::Jours(60)),
        fixe("2", Delai::Jours(0)),
        fixe("3", Delai::Jours(60)),
        solde(Delai::Jours(0)),
    ]);
    let r = date!(2026 - 03 - 01);
    let obtenu = repartir(&c, "10", xof(), r);
    let attendu = [
        (r, "2"),
        (r, "4"),
        (date!(2026 - 04 - 30), "1"),
        (date!(2026 - 04 - 30), "3"),
    ];
    assert_eq!(obtenu, attendu.map(|(j, m)| (j, m.to_owned())).to_vec());
}

#[test]
fn condition_invalide_refusee() {
    let j = Delai::Jours(0);
    assert_eq!(
        ConditionReglement::nouvelle(vec![]),
        Err(ErreurEcheance::ConditionSansTerme)
    );
    assert_eq!(
        ConditionReglement::nouvelle(vec![pct("50", j)]),
        Err(ErreurEcheance::DernierTermePasSolde)
    );
    assert_eq!(
        ConditionReglement::nouvelle(vec![pct("50", j), solde(j), solde(j)]),
        Err(ErreurEcheance::SoldeAvantLaFin(2))
    );
    assert_eq!(
        ConditionReglement::nouvelle(vec![pct("60", j), pct("40", j), solde(j)]),
        Err(ErreurEcheance::PourcentagesExcessifs)
    );
    assert!(
        ConditionReglement::nouvelle(vec![pct("60", j), pct("39.999999", j), solde(j)]).is_ok()
    );
    assert!(ConditionReglement::nouvelle(vec![solde(j)]).is_ok());
    for refuse in ["0", "-1", "100", "150", "1.0000001"] {
        assert_eq!(
            Pourcentage::nouveau(d(refuse)),
            Err(ErreurEcheance::PourcentageInvalide(d(refuse))),
            "{refuse}"
        );
    }
    assert!(Pourcentage::nouveau(d("1.0000010")).is_ok());
    assert!(MontantFixe::nouveau(d("0")).is_err());
    assert!(MontantFixe::nouveau(d("-5")).is_err());
    assert_eq!(
        JourDuMois::nouveau(32),
        Err(ErreurEcheance::JourDuMoisInvalide(32))
    );
    assert_eq!(jour_du_mois(31).jour(), 31);
}

/// Total plus précis que l'échelle de la devise : les termes autres que le solde sont arrondis,
/// le solde garde la précision du total et la somme reste exacte (Odoo ; aucune erreur).
#[test]
fn total_plus_precis_que_la_devise() {
    let c = condition(vec![pct("50", Delai::Jours(0)), solde(Delai::Jours(30))]);
    let r = date!(2026 - 01 - 01);
    assert_eq!(
        repartir(&c, "10.001", tenue(2), r),
        [
            (r, "5.00".to_owned()),
            (date!(2026 - 01 - 31), "5.001".to_owned())
        ]
    );
    assert_eq!(
        repartir(&c, "-10.5", xof(), r),
        [
            (r, "-5".to_owned()),
            (date!(2026 - 01 - 31), "-5.5".to_owned())
        ]
    );
}

#[test]
fn total_invalide_refuse() {
    let c = condition(vec![pct("50", Delai::Jours(0)), solde(Delai::Jours(30))]);
    let r = date!(2026 - 01 - 01);
    let eur = Devise::parse("EUR").unwrap();
    assert_eq!(
        c.repartir(Montant::nouveau(d("10"), eur), r, &xof()),
        Err(ErreurEcheance::PasEnDeviseTenue(eur))
    );
    let xof_devise = xof().devise();
    assert_eq!(
        c.repartir(Montant::nouveau(Decimal::MAX, xof_devise), r, &xof()),
        Err(ErreurEcheance::Depassement)
    );
    let loin = condition(vec![solde(Delai::Jours(10))]);
    assert_eq!(
        loin.repartir(Montant::nouveau(d("10"), xof_devise), Date::MAX, &xof()),
        Err(ErreurEcheance::DateHorsLimites)
    );
}

// --- Délais ------------------------------------------------------------------------------

#[test]
fn delais_fin_de_mois_fevrier_et_bissextiles() {
    let fin = Delai::JoursApresFinDeMois(0);
    assert_eq!(
        fin.echeance(date!(2024 - 02 - 10)),
        Ok(date!(2024 - 02 - 29))
    );
    assert_eq!(
        fin.echeance(date!(2026 - 02 - 10)),
        Ok(date!(2026 - 02 - 28))
    );
    assert_eq!(
        fin.echeance(date!(2100 - 02 - 01)),
        Ok(date!(2100 - 02 - 28))
    );
    assert_eq!(
        fin.echeance(date!(2000 - 02 - 01)),
        Ok(date!(2000 - 02 - 29))
    );
    assert_eq!(
        fin.echeance(date!(2026 - 12 - 31)),
        Ok(date!(2026 - 12 - 31))
    );
    // « 30 jours fin de mois » au sens d'Odoo : fin du mois, puis 30 jours.
    assert_eq!(
        Delai::JoursApresFinDeMois(30).echeance(date!(2026 - 01 - 15)),
        Ok(date!(2026 - 03 - 02))
    );
    assert_eq!(
        Delai::JoursApresFinDeMois(10).echeance(date!(2026 - 01 - 31)),
        Ok(date!(2026 - 02 - 10))
    );
    let suivant = Delai::JoursApresFinDuMoisSuivant(0);
    assert_eq!(
        suivant.echeance(date!(2026 - 01 - 31)),
        Ok(date!(2026 - 02 - 28))
    );
    assert_eq!(
        suivant.echeance(date!(2028 - 01 - 31)),
        Ok(date!(2028 - 02 - 29))
    );
    assert_eq!(
        suivant.echeance(date!(2026 - 12 - 15)),
        Ok(date!(2027 - 01 - 31))
    );
    assert_eq!(
        Delai::JoursApresFinDuMoisSuivant(5).echeance(date!(2026 - 11 - 30)),
        Ok(date!(2027 - 01 - 05))
    );
    assert_eq!(
        Delai::Jours(0).echeance(date!(2026 - 03 - 01)),
        Ok(date!(2026 - 03 - 01))
    );
    assert_eq!(
        Delai::Jours(60).echeance(date!(2026 - 01 - 01)),
        Ok(date!(2026 - 03 - 02))
    );
}

/// Cas d'Odoo (`test_payment_term_days_end_of_month_*`) pour « jours, puis le X du mois
/// suivant ».
#[test]
fn reference_odoo_jour_du_mois_suivant() {
    let delai = |jours, jour| Delai::JoursPuisJourDuMoisSuivant {
        jours,
        jour: jour_du_mois(jour),
    };
    let cas = [
        (30, 10, date!(2023 - 12 - 12), date!(2024 - 02 - 10)),
        (30, 31, date!(2023 - 12 - 12), date!(2024 - 02 - 29)),
        (0, 29, date!(2024 - 05 - 23), date!(2024 - 06 - 29)),
        (0, 31, date!(2024 - 05 - 23), date!(2024 - 06 - 30)),
        (15, 30, date!(2024 - 05 - 24), date!(2024 - 07 - 30)),
        (15, 31, date!(2024 - 05 - 23), date!(2024 - 07 - 31)),
        (30, 0, date!(2024 - 04 - 22), date!(2024 - 05 - 31)),
    ];
    for (jours, jour, reference, attendu) in cas {
        assert_eq!(
            delai(jours, jour).echeance(reference),
            Ok(attendu),
            "{reference}"
        );
    }
    // Février d'une année non bissextile, passage d'année.
    assert_eq!(
        delai(0, 31).echeance(date!(2027 - 01 - 15)),
        Ok(date!(2027 - 02 - 28))
    );
    assert_eq!(
        delai(0, 5).echeance(date!(2026 - 12 - 31)),
        Ok(date!(2027 - 01 - 05))
    );
}

/// Jours négatifs admis comme chez Tryton et Odoo (aucune contrainte de signe).
#[test]
fn delais_negatifs_admis() {
    assert_eq!(
        Delai::Jours(-10).echeance(date!(2026 - 03 - 05)),
        Ok(date!(2026 - 02 - 23))
    );
    assert_eq!(
        Delai::JoursApresFinDeMois(-1).echeance(date!(2026 - 03 - 10)),
        Ok(date!(2026 - 03 - 30))
    );
    assert_eq!(
        Delai::JoursApresFinDuMoisSuivant(-28).echeance(date!(2026 - 01 - 10)),
        Ok(date!(2026 - 01 - 31))
    );
    assert_eq!(
        Delai::JoursPuisJourDuMoisSuivant {
            jours: -40,
            jour: jour_du_mois(10)
        }
        .echeance(date!(2026 - 03 - 15)),
        Ok(date!(2026 - 03 - 10))
    );
    // Une échéance antérieure à la pièce reste répartie sans perte.
    let c = condition(vec![pct("50", Delai::Jours(-5)), solde(Delai::Jours(0))]);
    assert_eq!(
        repartir(&c, "100", xof(), date!(2026 - 01 - 10)),
        [
            (date!(2026 - 01 - 05), "50".to_owned()),
            (date!(2026 - 01 - 10), "50".to_owned())
        ]
    );
}

#[test]
fn date_hors_limites_sans_panique() {
    assert_eq!(
        Delai::Jours(-1).echeance(Date::MIN),
        Err(ErreurEcheance::DateHorsLimites)
    );
    assert_eq!(
        Delai::Jours(i32::MIN).echeance(date!(2026 - 01 - 01)),
        Err(ErreurEcheance::DateHorsLimites)
    );
    assert_eq!(
        Delai::JoursPuisJourDuMoisSuivant {
            jours: i32::MAX,
            jour: jour_du_mois(0)
        }
        .echeance(date!(2026 - 01 - 01)),
        Err(ErreurEcheance::DateHorsLimites)
    );
    let fin_9999 = Date::from_calendar_date(9999, Month::December, 15).unwrap();
    assert_eq!(
        Delai::Jours(i32::MAX).echeance(Date::MAX),
        Err(ErreurEcheance::DateHorsLimites)
    );
    assert_eq!(
        Delai::JoursApresFinDuMoisSuivant(0).echeance(fin_9999),
        Err(ErreurEcheance::DateHorsLimites)
    );
    assert_eq!(
        Delai::JoursPuisJourDuMoisSuivant {
            jours: 0,
            jour: jour_du_mois(1)
        }
        .echeance(fin_9999),
        Err(ErreurEcheance::DateHorsLimites)
    );
}

// --- Échéance effective, révisions, statut -------------------------------------------------

#[test]
fn echeance_effective_et_replis() {
    let comptable = date!(2026 - 01 - 10);
    assert_eq!(
        EcheanceLigne::nouvelle(comptable, None).effective(date!(2026 - 06 - 01)),
        comptable
    );
    let initiale = date!(2026 - 02 - 28);
    let mut ligne = EcheanceLigne::nouvelle(comptable, Some(initiale));
    assert_eq!(ligne.effective(date!(2026 - 01 - 01)), initiale);
    let r = residuel(1000, 1000);
    let rev = ligne
        .reviser(
            date!(2026 - 03 - 31),
            date!(2026 - 02 - 20),
            motif("report accordé"),
            &r,
        )
        .unwrap();
    assert_eq!(rev.rang(), 1);
    ligne
        .reviser(
            date!(2026 - 04 - 30),
            date!(2026 - 02 - 20),
            motif("second report"),
            &r,
        )
        .unwrap();
    ligne
        .reviser(
            date!(2026 - 03 - 15),
            date!(2026 - 03 - 01),
            motif("erreur"),
            &r,
        )
        .unwrap();
    assert_eq!(ligne.revisions().len(), 3);
    assert_eq!(ligne.revisions()[2].rang(), 3);
    // Avant toute révision : l'initiale ; le jour même : la dernière du jour ; ensuite : la
    // dernière révision.
    assert_eq!(ligne.effective(date!(2026 - 02 - 19)), initiale);
    assert_eq!(
        ligne.effective(date!(2026 - 02 - 20)),
        date!(2026 - 04 - 30)
    );
    assert_eq!(
        ligne.effective(date!(2026 - 12 - 31)),
        date!(2026 - 03 - 15)
    );
}

#[test]
fn revision_refusee_si_soldee_ou_hors_ordre() {
    let mut ligne = EcheanceLigne::nouvelle(date!(2026 - 01 - 10), None);
    assert_eq!(
        ligne.reviser(
            date!(2026 - 03 - 01),
            date!(2026 - 02 - 01),
            motif("m"),
            &residuel(100, 0)
        ),
        Err(ErreurEcheance::EcheanceSoldee)
    );
    // Partiellement réglée : permise.
    ligne
        .reviser(
            date!(2026 - 03 - 01),
            date!(2026 - 02 - 01),
            motif("m"),
            &residuel(100, 40),
        )
        .unwrap();
    assert_eq!(
        ligne.reviser(
            date!(2026 - 03 - 01),
            date!(2026 - 01 - 31),
            motif("m"),
            &residuel(100, 40)
        ),
        Err(ErreurEcheance::RevisionHorsOrdre(2))
    );
    assert_eq!(ligne.revisions().len(), 1);
}

#[test]
fn reconstitution_des_revisions_stockees() {
    let rev = |rang, nouvelle, operation| {
        Revision::nouvelle(rang, nouvelle, operation, motif("m")).unwrap()
    };
    let comptable = date!(2026 - 01 - 10);
    let ligne = EcheanceLigne::reconstituer(
        comptable,
        None,
        vec![
            rev(4, date!(2026 - 05 - 01), date!(2026 - 03 - 01)),
            rev(2, date!(2026 - 04 - 01), date!(2026 - 02 - 01)),
        ],
    )
    .unwrap();
    assert_eq!(
        ligne
            .revisions()
            .iter()
            .map(Revision::rang)
            .collect::<Vec<_>>(),
        [2, 4]
    );
    assert_eq!(
        ligne.effective(date!(2026 - 02 - 15)),
        date!(2026 - 04 - 01)
    );
    assert_eq!(ligne.date_comptable(), comptable);
    assert_eq!(ligne.initiale(), None);
    assert_eq!(
        EcheanceLigne::reconstituer(
            comptable,
            None,
            vec![rev(1, comptable, comptable), rev(1, comptable, comptable)]
        ),
        Err(ErreurEcheance::RevisionHorsOrdre(1))
    );
    assert_eq!(
        EcheanceLigne::reconstituer(
            comptable,
            None,
            vec![
                rev(1, comptable, date!(2026 - 02 - 02)),
                rev(2, comptable, date!(2026 - 02 - 01))
            ]
        ),
        Err(ErreurEcheance::RevisionHorsOrdre(2))
    );
    assert_eq!(
        Revision::nouvelle(0, comptable, comptable, motif("m")),
        Err(ErreurEcheance::RangInvalide)
    );
    let r = rev(3, date!(2026 - 05 - 01), date!(2026 - 03 - 01));
    assert_eq!(
        (
            r.nouvelle_echeance(),
            r.date_operation(),
            r.motif().as_str()
        ),
        (date!(2026 - 05 - 01), date!(2026 - 03 - 01), "m")
    );
}

#[test]
fn statut_et_situation_calcules() {
    assert_eq!(residuel(100, 100).statut(), StatutReglement::Ouverte);
    assert_eq!(
        residuel(100, 1).statut(),
        StatutReglement::PartiellementReglee
    );
    assert_eq!(residuel(100, 0).statut(), StatutReglement::Soldee);
    assert!(matches!(
        Residuel::nouveau(Decimal::from(100), Decimal::from(101)),
        Err(ErreurEcheance::ResiduelHorsBornes { .. })
    ));
    assert!(Residuel::nouveau(Decimal::from(100), Decimal::from(-1)).is_err());
    let r = residuel(100, 60);
    assert_eq!(
        (r.montant(), r.residuel()),
        (Decimal::from(100), Decimal::from(60))
    );

    let ligne = EcheanceLigne::nouvelle(date!(2026 - 01 - 10), Some(date!(2026 - 02 - 28)));
    assert_eq!(
        ligne.situation(date!(2026 - 02 - 28), &r),
        Situation::AEchoir
    );
    assert_eq!(
        ligne.situation(date!(2026 - 01 - 01), &r),
        Situation::AEchoir
    );
    // Borne : à échoir le jour de l'échéance, échu d'un jour le lendemain.
    assert_eq!(
        ligne.situation(date!(2026 - 03 - 01), &r),
        Situation::Echue { retard_jours: 1 }
    );
    assert_eq!(
        ligne.situation(date!(2026 - 03 - 31), &r),
        Situation::Echue { retard_jours: 31 }
    );
    assert_eq!(
        ligne.situation(date!(2026 - 03 - 31), &residuel(100, 0)),
        Situation::Soldee
    );
}

#[test]
fn motif_de_revision() {
    assert_eq!(
        motif("Report accordé au client").as_str(),
        "Report accordé au client"
    );
    assert!(MotifRevision::parse(&"é".repeat(200)).is_ok());
    for refuse in ["", "   ", "a\nb", "a\u{202E}b", "a\u{200B}b", "a\u{2028}b"] {
        assert_eq!(
            MotifRevision::parse(refuse),
            Err(ErreurEcheance::MotifInvalide),
            "{refuse:?}"
        );
    }
    assert!(MotifRevision::parse(&"a".repeat(MOTIF_OCTETS_MAX + 1)).is_err());
    assert!(MotifRevision::parse(&"é".repeat(201)).is_err());
}

// --- Propriétés ----------------------------------------------------------------------------

fn date_quelconque() -> impl Strategy<Value = Date> {
    (0i64..120_000).prop_map(|n| date!(1900 - 01 - 01) + Duration::days(n))
}

fn delai_quelconque() -> impl Strategy<Value = Delai> {
    prop_oneof![
        (-400i32..400).prop_map(Delai::Jours),
        (-400i32..400).prop_map(Delai::JoursApresFinDeMois),
        (-400i32..400).prop_map(Delai::JoursApresFinDuMoisSuivant),
        (-400i32..400, 0u8..=31).prop_map(|(jours, jour)| Delai::JoursPuisJourDuMoisSuivant {
            jours,
            jour: JourDuMois::nouveau(jour).unwrap(),
        }),
    ]
}

/// Terme autre que le solde : pourcentage jusqu'à 99,999999 % ou montant fixe jusqu'à
/// 10¹³ (souvent plus grand que le total : terme plafonné), à quatre décimales au plus.
fn valeur_quelconque() -> impl Strategy<Value = ValeurTerme> {
    prop_oneof![
        (1i64..=99_999_999).prop_map(|p| ValeurTerme::Pourcentage(
            Pourcentage::nouveau(Decimal::new(p, 6)).unwrap()
        )),
        (1i64..10_000_000_000_000, 0u32..=4).prop_map(|(m, e)| ValeurTerme::Fixe(
            MontantFixe::nouveau(Decimal::new(m, e)).unwrap()
        )),
    ]
}

/// Condition quelconque : un pourcentage qui porterait la somme à 100 ou plus devient un
/// montant fixe de même valeur numérique (la somme des pourcentages peut approcher 100).
fn condition_quelconque() -> impl Strategy<Value = ConditionReglement> {
    (
        prop::collection::vec((valeur_quelconque(), delai_quelconque()), 0..8),
        delai_quelconque(),
    )
        .prop_map(|(valeurs, delai)| {
            let mut somme = Decimal::ZERO;
            let mut termes: Vec<Terme> = valeurs
                .into_iter()
                .map(|(valeur, delai)| match valeur {
                    ValeurTerme::Pourcentage(p) if somme + p.valeur() >= Decimal::ONE_HUNDRED => {
                        let f = MontantFixe::nouveau(p.valeur()).unwrap();
                        Terme::nouveau(ValeurTerme::Fixe(f), delai)
                    }
                    ValeurTerme::Pourcentage(p) => {
                        somme += p.valeur();
                        Terme::nouveau(valeur, delai)
                    }
                    _ => Terme::nouveau(valeur, delai),
                })
                .collect();
            termes.push(solde(delai));
            ConditionReglement::nouvelle(termes).unwrap()
        })
}

/// Oracle naïf des dates : avance jour par jour, sans arithmétique de mois.
fn date_oracle(delai: Delai, reference: Date) -> Date {
    let plus = |d: Date, n: i32| d + Duration::days(i64::from(n));
    let fin_mois = |mut d: Date| {
        while d.next_day().unwrap().month() == d.month() {
            d = d.next_day().unwrap();
        }
        d
    };
    let debut_suivant = |d: Date| fin_mois(d).next_day().unwrap();
    match delai {
        Delai::Jours(n) => plus(reference, n),
        Delai::JoursApresFinDeMois(n) => plus(fin_mois(reference), n),
        Delai::JoursApresFinDuMoisSuivant(n) => plus(fin_mois(debut_suivant(reference)), n),
        Delai::JoursPuisJourDuMoisSuivant { jours, jour } => {
            let apres = plus(reference, jours);
            if jour.jour() == 0 {
                return fin_mois(apres);
            }
            let mut d = debut_suivant(apres);
            while d.day() < jour.jour() && d.next_day().unwrap().day() != 1 {
                d = d.next_day().unwrap();
            }
            d
        }
    }
}

/// Oracle naïf de la répartition, formulé par cumuls : le cumul des termes jusqu'au rang i est
/// min(|total|, somme des valeurs brutes arrondies jusqu'au rang i), le solde valant |total| ;
/// le terme i est la différence de deux cumuls, les termes nuls sont omis.
fn repartition_oracle(
    c: &ConditionReglement,
    total: Decimal,
    reference: Date,
    t: DeviseTenue,
) -> Vec<(Date, Decimal)> {
    let absolu = total.abs();
    if absolu.is_zero() {
        return vec![(reference, Decimal::ZERO)];
    }
    let (mut brut_cumule, mut cumul_precedent) = (Decimal::ZERO, Decimal::ZERO);
    let mut attendu = Vec::new();
    for terme in c.termes() {
        let brut = match terme.valeur() {
            ValeurTerme::Pourcentage(p) => t.arrondir(absolu * (p.valeur() / Decimal::ONE_HUNDRED)),
            ValeurTerme::Fixe(f) => t.arrondir(f.valeur()),
            ValeurTerme::Solde => absolu,
        };
        brut_cumule += brut;
        let cumul = brut_cumule.min(absolu);
        let v = cumul - cumul_precedent;
        cumul_precedent = cumul;
        if !v.is_zero() {
            let signe = if total.is_sign_negative() { -v } else { v };
            attendu.push((date_oracle(terme.delai(), reference), signe));
        }
    }
    attendu.sort_by_key(|(d, _)| *d);
    attendu
}

fn est_bissextile(annee: i32) -> bool {
    annee % 4 == 0 && (annee % 100 != 0 || annee % 400 == 0)
}

proptest! {
    /// Aucun centime perdu : la somme des termes est exactement le total (même plus précis que
    /// la devise) ; montants et dates égaux à ceux de l'oracle naïf ; chaque terme est non nul,
    /// du signe du total ; dates croissantes ; au plus un terme par terme de la condition ;
    /// calcul déterministe.
    #[test]
    fn repartition_exacte(
        c in condition_quelconque(),
        unites in -1_000_000_000_000i64..1_000_000_000_000,
        decimales in 0u8..=4,
        echelle_total in 0u32..=6,
        reference in date_quelconque(),
    ) {
        let t = tenue(decimales);
        let total = Montant::nouveau(Decimal::new(unites, echelle_total), t.devise());
        let termes = c.repartir(total, reference, &t).unwrap();
        prop_assert_eq!(&termes, &c.repartir(total, reference, &t).unwrap());
        let obtenu: Vec<(Date, Decimal)> =
            termes.iter().map(|x| (x.date(), x.montant().valeur())).collect();
        prop_assert_eq!(obtenu, repartition_oracle(&c, total.valeur(), reference, t));
        let somme: Decimal = termes.iter().map(|x| x.montant().valeur()).sum();
        prop_assert_eq!(somme, total.valeur());
        prop_assert!(!termes.is_empty() && termes.len() <= c.termes().len());
        for terme in &termes {
            let v = terme.montant().valeur();
            prop_assert_eq!(terme.montant().devise(), t.devise());
            if total.est_nul() {
                prop_assert!(v.is_zero());
            } else {
                prop_assert!(!v.is_zero());
                prop_assert_eq!(v.is_sign_negative(), total.valeur().is_sign_negative());
            }
        }
        prop_assert!(termes.windows(2).all(|p| p[0].date() <= p[1].date()));
        // Un avoir se répartit comme la facture, au signe près.
        let oppose = c.repartir(total.oppose(), reference, &t).unwrap();
        prop_assert_eq!(oppose.len(), termes.len());
        for (a, b) in oppose.iter().zip(&termes) {
            prop_assert_eq!(a.montant(), b.montant().oppose());
            prop_assert_eq!(a.date(), b.date());
        }
    }

    /// Fin de mois : dernier jour du mois de la date, 29 février les seules années bissextiles.
    #[test]
    fn fin_de_mois_correcte(date in date_quelconque(), jours in -400i32..400) {
        let fin = Delai::JoursApresFinDeMois(0).echeance(date).unwrap();
        prop_assert_eq!((fin.year(), fin.month()), (date.year(), date.month()));
        prop_assert_eq!(fin.next_day().unwrap().day(), 1);
        if fin.month() == Month::February {
            let attendu = if est_bissextile(fin.year()) { 29 } else { 28 };
            prop_assert_eq!(fin.day(), attendu);
        }
        prop_assert_eq!(
            Delai::JoursApresFinDeMois(jours).echeance(date).unwrap(),
            fin + Duration::days(i64::from(jours))
        );
        let suivant = Delai::JoursApresFinDuMoisSuivant(0).echeance(date).unwrap();
        prop_assert_eq!(suivant.next_day().unwrap().day(), 1);
        prop_assert_eq!(suivant.month(), date.month().next());
    }

    /// « Jours, puis le X du mois suivant » : jour borné au dernier jour de ce mois ; X nul :
    /// fin du mois atteint après les jours.
    #[test]
    fn jour_du_mois_suivant_borne(date in date_quelconque(), jours in -400i32..400, x in 0u8..=31) {
        let delai = Delai::JoursPuisJourDuMoisSuivant { jours, jour: JourDuMois::nouveau(x).unwrap() };
        let obtenu = delai.echeance(date).unwrap();
        let apres = date + Duration::days(i64::from(jours));
        prop_assert_eq!(obtenu, date_oracle(delai, date));
        if x == 0 {
            prop_assert_eq!((obtenu.year(), obtenu.month()), (apres.year(), apres.month()));
            prop_assert_eq!(obtenu.next_day().unwrap().day(), 1);
        } else {
            prop_assert_eq!(obtenu.month(), apres.month().next());
            prop_assert_eq!(obtenu.day(), x.min(obtenu.month().length(obtenu.year())));
            prop_assert!(obtenu > apres);
        }
    }

    /// Chaque type de délai égale l'oracle naïf (jour par jour).
    #[test]
    fn delai_egal_oracle(delai in delai_quelconque(), date in date_quelconque()) {
        prop_assert_eq!(delai.echeance(date).unwrap(), date_oracle(delai, date));
    }

    /// Reconstitution : l'échéance effective à une date ne dépend d'aucune révision dont
    /// l'opération est postérieure à cette date.
    #[test]
    fn effective_independante_des_revisions_posterieures(
        comptable in date_quelconque(),
        initiale in proptest::option::of(date_quelconque()),
        ecarts in prop::collection::vec((0i64..60, date_quelconque()), 0..8),
        decalage in 0i64..500,
    ) {
        let mut operation = comptable;
        let mut revisions = Vec::new();
        for (rang, (ecart, nouvelle)) in (1u32..).zip(ecarts) {
            operation += Duration::days(ecart);
            revisions.push(Revision::nouvelle(rang, nouvelle, operation, motif("m")).unwrap());
        }
        let date = comptable + Duration::days(decalage);
        let anterieures: Vec<Revision> =
            revisions.iter().filter(|r| r.date_operation() <= date).cloned().collect();
        let complete = EcheanceLigne::reconstituer(comptable, initiale, revisions).unwrap();
        let tronquee = EcheanceLigne::reconstituer(comptable, initiale, anterieures).unwrap();
        prop_assert_eq!(complete.effective(date), tronquee.effective(date));
        let attendu = tronquee
            .revisions()
            .last()
            .map_or(initiale.unwrap_or(comptable), Revision::nouvelle_echeance);
        prop_assert_eq!(complete.effective(date), attendu);
    }
}
