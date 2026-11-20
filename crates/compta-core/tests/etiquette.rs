//! Étiquette fiscale (ADR 0019) : aller-retour, copie par la contre-passation, contrôle des
//! taxes. Référentiel **fictif** : `compta-core` ne connaît aucun régime.

// Fichier de test : les `unwrap` y sont voulus (une donnée de test invalide fait échouer le test).
#![allow(clippy::unwrap_used)]
mod commun;

use commun::*;
use compta_core::{
    CodeMotif, CodeRegime, EtiquetteFiscale, Exigibilite, Ligne, NatureEtiquette, NumeroCompte,
    ProfilRegime, ReferentielFiscal, Sens, SensTva, TauxTva, controler_taxes_et_comptes,
    totaux_taxe,
};
use proptest::prelude::*;
use rust_decimal::Decimal;
use time::macros::date;

struct Fictif;

impl ReferentielFiscal for Fictif {
    fn profil_regime(&self, code: &CodeRegime) -> Option<ProfilRegime> {
        let taux = |t: &[&str]| t.iter().map(|t| TauxTva::parse(t).unwrap()).collect();
        match code.as_str() {
            "IMP" => Some(ProfilRegime::nouveau(
                false,
                taux(&["18", "10"]),
                vec![Exigibilite::Debit, Exigibilite::Encaissement],
            )),
            "EXO" => Some(ProfilRegime::nouveau(true, vec![], vec![])),
            "HC" => Some(ProfilRegime::nouveau(false, vec![], vec![])),
            _ => None,
        }
    }

    fn motif_admis(&self, regime: &CodeRegime, motif: &CodeMotif) -> bool {
        regime.as_str() == "EXO" && motif.as_str().starts_with("EXO-")
    }
}

fn etiquette(
    nature: NatureEtiquette,
    sens: SensTva,
    regime: &str,
    taux: Option<&str>,
    exigibilite: Option<Exigibilite>,
    motif: Option<&str>,
) -> EtiquetteFiscale {
    EtiquetteFiscale::nouvelle(
        nature,
        sens,
        CodeRegime::parse(regime).unwrap(),
        taux.map(|t| TauxTva::parse(t).unwrap()),
        exigibilite,
        motif.map(|m| CodeMotif::parse(m).unwrap()),
        &Fictif,
    )
    .unwrap()
}

fn etiquettes_valides() -> impl Strategy<Value = EtiquetteFiscale> {
    let nature = prop_oneof![Just(NatureEtiquette::Base), Just(NatureEtiquette::Taxe)];
    let sens = prop_oneof![Just(SensTva::Collectee), Just(SensTva::Deductible)];
    let imposable = (
        nature,
        sens.clone(),
        prop_oneof![Just("18"), Just("10")],
        prop_oneof![Just(Exigibilite::Debit), Just(Exigibilite::Encaissement)],
    )
        .prop_map(|(n, s, t, e)| etiquette(n, s, "IMP", Some(t), Some(e), None));
    let exonere = (sens.clone(), "[0-9]{1,2}|[a-f]").prop_map(|(s, m)| {
        etiquette(
            NatureEtiquette::Base,
            s,
            "EXO",
            None,
            None,
            Some(&format!("EXO-{m}")),
        )
    });
    let hors_champ = (
        prop_oneof![Just(NatureEtiquette::Base), Just(NatureEtiquette::Taxe)],
        sens,
    )
        .prop_map(|(n, s)| etiquette(n, s, "HC", None, None, None));
    prop_oneof![imposable, exonere, hors_champ]
}

fn ligne_etiquetee(compte: &str, sens: Sens, montant: &str, e: EtiquetteFiscale) -> Ligne {
    ligne(compte, sens, montant).avec_etiquette(e)
}

proptest! {
    /// Aller-retour : les accesseurs d'une étiquette valide la reconstruisent à l'identique.
    #[test]
    fn aller_retour(e in etiquettes_valides()) {
        let relue = EtiquetteFiscale::relire(
            e.nature(),
            e.sens(),
            e.regime().clone(),
            e.taux(),
            e.exigibilite(),
            e.motif().cloned(),
        );
        prop_assert_eq!(relue, Ok(e.clone()));
        // Le taux s'écrit puis se relit exactement.
        if let Some(t) = e.taux() {
            prop_assert_eq!(TauxTva::parse(&t.to_string()), Ok(t));
        }
    }

    /// La contre-passation conserve l'étiquette de chaque ligne ; les totaux de taxe de
    /// l'écriture et de son miroir s'annulent.
    #[test]
    fn la_contre_passation_conserve_l_etiquette(e in etiquettes_valides(), taxe in 1u32..100_000) {
        let mut d = Dossier::nouveau();
        let ht = (u64::from(taxe) * 100).to_string();
        let tva = taxe.to_string();
        let ttc = (u64::from(taxe) * 100 + u64::from(taxe)).to_string();
        let lignes = vec![
            ligne_tiers("4111", Sens::Debit, &ttc, "C1"),
            ligne_etiquetee("7011", Sens::Credit, &ht, e.clone()),
            ligne("7011", Sens::Credit, &tva),
        ];
        let origine = d.valider(equilibree("VT", date!(2026 - 03 - 10), "Vente", lignes)).unwrap();
        let cp = d.contre_passer(&origine, date!(2026 - 04 - 02)).unwrap();
        let (a, b) = (
            d.livre.ecriture(&origine).unwrap(),
            d.livre.ecriture(&cp).unwrap(),
        );
        for (x, y) in a.lignes().iter().zip(b.lignes()) {
            prop_assert_eq!(x.etiquette(), y.etiquette());
        }
        prop_assert_eq!(b.lignes()[1].etiquette(), Some(&e));
        prop_assert_eq!(b.lignes()[2].etiquette(), None);
        let (ta, tb) = (totaux_taxe(a.lignes()), totaux_taxe(b.lignes()));
        prop_assert_eq!(ta.collectee + tb.collectee, Decimal::ZERO);
        prop_assert_eq!(ta.deductible + tb.deductible, Decimal::ZERO);
    }
}

#[test]
fn controle_des_taxes_et_des_comptes() {
    let collectee =
        |compte: &NumeroCompte| (compte.as_str() == "4431").then_some(SensTva::Collectee);
    let taxe = |s| {
        etiquette(
            NatureEtiquette::Taxe,
            s,
            "IMP",
            Some("18"),
            Some(Exigibilite::Debit),
            None,
        )
    };
    // 4431 n'existe pas dans le plan de test, mais la fonction est pure : seuls les numéros
    // comptent (le plan ne valide que l'écriture, pas ce contrôle).
    let lignes = vec![
        ligne("4111", Sens::Debit, "118"),
        ligne("7011", Sens::Credit, "100"),
        ligne_etiquetee("4431", Sens::Credit, "18", taxe(SensTva::Collectee)),
    ];
    assert_eq!(controler_taxes_et_comptes(&lignes, collectee), Ok(()));
    // Ligne sur le compte de TVA sans étiquette : écart.
    let sans_etiquette = vec![
        ligne("4111", Sens::Debit, "118"),
        ligne("7011", Sens::Credit, "100"),
        ligne("4431", Sens::Credit, "18"),
    ];
    let ecart = controler_taxes_et_comptes(&sans_etiquette, collectee).unwrap_err();
    assert_eq!(ecart.sens, SensTva::Collectee);
    assert_eq!(ecart.etiquettes, Decimal::ZERO);
    assert_eq!(ecart.comptes, Decimal::from(18));
}
