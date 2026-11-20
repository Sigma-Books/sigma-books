//! Étiquette fiscale (ADR 0019) : aller-retour, copie par la contre-passation, contrôle des
//! taxes. Référentiel **fictif** : `compta-core` ne connaît aucun régime.

// Fichier de test : les `unwrap` y sont voulus (une donnée de test invalide fait échouer le test).
#![allow(clippy::unwrap_used)]
mod commun;

use commun::*;
use compta_core::{
    CodeMotif, CodeRegime, EtiquetteFiscale, EtiquetteRelue, Exigibilite, Ligne, NatureEtiquette,
    NumeroCompte, ProfilRegime, ReferentielFiscal, Sens, SensTva, TauxTva,
    controler_taxes_et_comptes, totaux_taxe,
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
                true,
                taux(&["18", "10"]),
                vec![Exigibilite::Debit, Exigibilite::Encaissement],
            )),
            "EXO" => Some(ProfilRegime::nouveau(true, false, vec![], vec![])),
            "HC" => Some(ProfilRegime::nouveau(false, false, vec![], vec![])),
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
    // Hors champ : pas de ligne de taxe, seule la base est étiquetée.
    let hors_champ = sens.prop_map(|s| etiquette(NatureEtiquette::Base, s, "HC", None, None, None));
    prop_oneof![imposable, exonere, hors_champ]
}

fn ligne_etiquetee(compte: &str, sens: Sens, montant: &str, e: EtiquetteFiscale) -> Ligne {
    ligne(compte, sens, montant).avec_etiquette(e)
}

proptest! {
    /// Aller-retour : les accesseurs d'une étiquette valide la reconstruisent à l'identique.
    #[test]
    fn aller_retour(e in etiquettes_valides()) {
        let relue = EtiquetteRelue::nouvelle(
            e.nature(),
            e.sens(),
            e.regime().clone(),
            e.taux(),
            e.exigibilite(),
            e.motif().cloned(),
        );
        prop_assert_eq!(relue, Ok(e.en_relue()));
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

/// Une étiquette incohérente n'est pas construisible par l'API publique : `nouvelle` refuse
/// chaque combinaison fautive, et le type « relu » ne se pose pas sur une ligne (doctest
/// `compile_fail` de `EtiquetteRelue`).
#[test]
fn aucune_etiquette_incoherente_sur_une_ligne() {
    let faire = |nature, regime: &str, taux: Option<&str>, exig, motif: Option<&str>| {
        EtiquetteFiscale::nouvelle(
            nature,
            SensTva::Collectee,
            CodeRegime::parse(regime).unwrap(),
            taux.map(|t| TauxTva::parse(t).unwrap()),
            exig,
            motif.map(|m| CodeMotif::parse(m).unwrap()),
            &Fictif,
        )
    };
    let debit = Some(Exigibilite::Debit);
    let incoherentes = [
        faire(NatureEtiquette::Base, "EXO", None, None, None),
        faire(
            NatureEtiquette::Base,
            "EXO",
            Some("18"),
            None,
            Some("EXO-1"),
        ),
        faire(NatureEtiquette::Taxe, "EXO", None, None, Some("EXO-1")),
        faire(
            NatureEtiquette::Base,
            "IMP",
            Some("18"),
            debit,
            Some("EXO-1"),
        ),
        faire(NatureEtiquette::Taxe, "HC", None, None, None),
        faire(NatureEtiquette::Base, "IMP", Some("20"), debit, None),
        faire(NatureEtiquette::Base, "IMP", None, debit, None),
        faire(NatureEtiquette::Base, "ABSENT", None, None, None),
    ];
    for e in incoherentes {
        assert!(e.is_err(), "{e:?}");
    }
    // La version « relue » accepte une combinaison que le référentiel refuse, mais sans pouvoir
    // la poser sur une ligne : `Ligne::avec_etiquette` ne prend qu'une `EtiquetteFiscale`.
    assert!(
        EtiquetteRelue::nouvelle(
            NatureEtiquette::Taxe,
            SensTva::Collectee,
            CodeRegime::parse("HC").unwrap(),
            None,
            None,
            None
        )
        .is_ok()
    );
}
