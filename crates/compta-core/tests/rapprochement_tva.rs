//! Rapprochement des étiquettes fiscales et des comptes de TVA (ADR 0026). Référentiel et
//! désignation des comptes **fictifs** : `compta-core` ne connaît ni régime ni numéro de TVA ;
//! la désignation réelle est testée dans `syscohada` et dans l'API.

// Fichier de test : les `unwrap` y sont voulus (une donnée de test invalide fait échouer le test).
#![allow(clippy::unwrap_used)]
mod commun;

use commun::*;
use compta_core::{
    CodeRegime, ErreurRapprochementTva, EtiquetteFiscale, Exigibilite, Ligne, NatureEtiquette,
    NumeroCompte, ProfilRegime, ReferentielFiscal, RoleCompteTva, Sens, SensTva, TauxTva,
    rapprocher_etiquettes_et_comptes,
};
use proptest::prelude::*;
use rust_decimal::Decimal;

struct Fictif;

impl ReferentielFiscal for Fictif {
    fn profil_regime(&self, code: &CodeRegime) -> Option<ProfilRegime> {
        (code.as_str() == "IMP").then(|| {
            ProfilRegime::nouveau(
                false,
                true,
                vec![TauxTva::parse("18").unwrap()],
                vec![Exigibilite::Debit],
            )
        })
    }

    fn motif_admis(&self, _: &CodeRegime, _: &compta_core::CodeMotif) -> bool {
        false
    }
}

/// Désignation fictive : 443x facturée, 444x due, 445x récupérable (par préfixe).
fn role(compte: &NumeroCompte) -> Option<RoleCompteTva> {
    match compte.tronc().get(..3) {
        Some("443") => Some(RoleCompteTva::Facturee),
        Some("444") => Some(RoleCompteTva::Due),
        Some("445") => Some(RoleCompteTva::Recuperable),
        _ => None,
    }
}

fn etiquette(nature: NatureEtiquette, sens: SensTva) -> EtiquetteFiscale {
    EtiquetteFiscale::nouvelle(
        nature,
        sens,
        CodeRegime::parse("IMP").unwrap(),
        Some(TauxTva::parse("18").unwrap()),
        Some(Exigibilite::Debit),
        None,
        &Fictif,
    )
    .unwrap()
}

fn etiquetee(compte: &str, sens: Sens, montant: &str, e: EtiquetteFiscale) -> Ligne {
    ligne(compte, sens, montant).avec_etiquette(e)
}

fn taxe(s: SensTva) -> EtiquetteFiscale {
    etiquette(NatureEtiquette::Taxe, s)
}

fn base(s: SensTva) -> EtiquetteFiscale {
    etiquette(NatureEtiquette::Base, s)
}

#[test]
fn vente_et_achat_coherents() {
    let vente = vec![
        ligne("4111", Sens::Debit, "118"),
        etiquetee("7011", Sens::Credit, "100", base(SensTva::Collectee)),
        etiquetee("4431", Sens::Credit, "18", taxe(SensTva::Collectee)),
    ];
    assert_eq!(rapprocher_etiquettes_et_comptes(&vente, role), Ok(()));
    let achat = vec![
        etiquetee("6011", Sens::Debit, "100", base(SensTva::Deductible)),
        etiquetee("4452", Sens::Debit, "18", taxe(SensTva::Deductible)),
        ligne("4011", Sens::Credit, "118"),
    ];
    assert_eq!(rapprocher_etiquettes_et_comptes(&achat, role), Ok(()));
    // Avoir de vente : la TVA facturée est débitée, l'étiquette reste « collectée ».
    let avoir = vec![
        etiquetee("7011", Sens::Debit, "100", base(SensTva::Collectee)),
        etiquetee("4431", Sens::Debit, "18", taxe(SensTva::Collectee)),
        ligne("4111", Sens::Credit, "118"),
    ];
    assert_eq!(rapprocher_etiquettes_et_comptes(&avoir, role), Ok(()));
}

#[test]
fn taxe_collectee_sur_un_compte_de_charge_est_refusee() {
    // Le total des étiquettes (18) égale le solde des comptes de TVA (18) : seul le
    // rapprochement ligne à ligne voit la faute.
    let lignes = vec![
        ligne("4111", Sens::Debit, "136"),
        ligne("7011", Sens::Credit, "100"),
        etiquetee("6411", Sens::Credit, "18", taxe(SensTva::Collectee)),
        ligne("4431", Sens::Credit, "18"),
    ];
    assert_eq!(
        rapprocher_etiquettes_et_comptes(&lignes, role),
        Err(ErreurRapprochementTva::TaxeSurUnCompteEtranger {
            ligne: 2,
            sens: SensTva::Collectee
        })
    );
}

#[test]
fn taxe_du_mauvais_sens_ou_sur_le_compte_du_solde_est_refusee() {
    for (compte, sens) in [
        ("4452", SensTva::Collectee),
        ("4431", SensTva::Deductible),
        ("4441", SensTva::Collectee),
        ("4441", SensTva::Deductible),
    ] {
        let lignes = vec![
            ligne("4111", Sens::Debit, "18"),
            etiquetee(compte, Sens::Credit, "18", taxe(sens)),
        ];
        assert!(matches!(
            rapprocher_etiquettes_et_comptes(&lignes, role),
            Err(ErreurRapprochementTva::TaxeSurUnCompteEtranger { ligne: 1, .. })
        ));
    }
}

#[test]
fn base_sur_un_compte_de_tva_est_refusee() {
    let lignes = vec![
        ligne("4111", Sens::Debit, "100"),
        etiquetee("4431", Sens::Credit, "100", base(SensTva::Collectee)),
    ];
    assert_eq!(
        rapprocher_etiquettes_et_comptes(&lignes, role),
        Err(ErreurRapprochementTva::BaseSurUnCompteDeTva { ligne: 1 })
    );
}

#[test]
fn ligne_de_tva_sans_etiquette_ou_montant_en_ecart_est_refusee() {
    let sans = vec![
        ligne("4111", Sens::Debit, "118"),
        ligne("7011", Sens::Credit, "100"),
        ligne("4431", Sens::Credit, "18"),
    ];
    let Err(ErreurRapprochementTva::Ecart(e)) = rapprocher_etiquettes_et_comptes(&sans, role)
    else {
        panic!("écart attendu");
    };
    assert_eq!(
        (e.etiquettes, e.comptes),
        (Decimal::ZERO, Decimal::from(18))
    );
    let ecart = vec![
        ligne("4111", Sens::Debit, "118"),
        ligne("7011", Sens::Credit, "100"),
        etiquetee("4431", Sens::Credit, "10", taxe(SensTva::Collectee)),
        ligne("4432", Sens::Credit, "8"),
    ];
    assert!(matches!(
        rapprocher_etiquettes_et_comptes(&ecart, role),
        Err(ErreurRapprochementTva::Ecart(_))
    ));
}

#[test]
fn ecriture_sans_tva_ni_etiquette_passe() {
    let lignes = vec![
        ligne("6011", Sens::Debit, "100"),
        ligne("5211", Sens::Credit, "100"),
    ];
    assert_eq!(rapprocher_etiquettes_et_comptes(&lignes, role), Ok(()));
}

#[test]
fn centralisation_de_la_periode_passe_et_elle_seule() {
    // 443 et 445 virés au 444 : aucune étiquette, que des comptes de TVA.
    let centralisation = vec![
        ligne("4431", Sens::Debit, "60"),
        ligne("4452", Sens::Credit, "44"),
        ligne("4441", Sens::Credit, "16"),
    ];
    assert_eq!(
        rapprocher_etiquettes_et_comptes(&centralisation, role),
        Ok(())
    );
    // La même écriture avec un compte étranger (une charge) n'est plus une centralisation.
    let melange = vec![
        ligne("4431", Sens::Debit, "60"),
        ligne("6411", Sens::Credit, "60"),
    ];
    assert!(matches!(
        rapprocher_etiquettes_et_comptes(&melange, role),
        Err(ErreurRapprochementTva::Ecart(_))
    ));
    // Le règlement de la TVA due (444 et trésorerie) ne touche ni 443 ni 445 : sans écart.
    let reglement = vec![
        ligne("4441", Sens::Debit, "16"),
        ligne("5211", Sens::Credit, "16"),
    ];
    assert_eq!(rapprocher_etiquettes_et_comptes(&reglement, role), Ok(()));
}

const COMPTES: [&str; 8] = [
    "4111", "7011", "6411", "4431", "4435", "4441", "4452", "4455",
];

fn ligne_aleatoire() -> impl Strategy<Value = Ligne> {
    (
        0..COMPTES.len(),
        any::<bool>(),
        1u32..500,
        0u8..5,
        any::<bool>(),
    )
        .prop_map(|(compte, debit, montant, etiquette_choisie, collectee)| {
            let sens = if debit { Sens::Debit } else { Sens::Credit };
            let ligne = ligne(COMPTES[compte], sens, &montant.to_string());
            let s = if collectee {
                SensTva::Collectee
            } else {
                SensTva::Deductible
            };
            match etiquette_choisie {
                0 => ligne.avec_etiquette(taxe(s)),
                1 => ligne.avec_etiquette(base(s)),
                _ => ligne,
            }
        })
}

/// Oracle indépendant de la règle, écrit à partir de son énoncé.
fn attendu_ok(lignes: &[Ligne]) -> bool {
    let mut collectee_e = Decimal::ZERO;
    let mut deductible_e = Decimal::ZERO;
    let mut collectee_c = Decimal::ZERO;
    let mut deductible_c = Decimal::ZERO;
    let mut etiquetee = false;
    let mut tout_en_tva = true;
    for l in lignes {
        let r = role(l.compte());
        tout_en_tva &= r.is_some();
        let net_credit = l.credit() - l.debit();
        match r {
            Some(RoleCompteTva::Facturee) => collectee_c += net_credit,
            Some(RoleCompteTva::Recuperable) => deductible_c -= net_credit,
            _ => {}
        }
        if let Some(e) = l.etiquette() {
            etiquetee = true;
            match (e.nature(), e.sens(), r) {
                (NatureEtiquette::Taxe, SensTva::Collectee, Some(RoleCompteTva::Facturee)) => {
                    collectee_e += net_credit;
                }
                (NatureEtiquette::Taxe, SensTva::Deductible, Some(RoleCompteTva::Recuperable)) => {
                    deductible_e -= net_credit;
                }
                (NatureEtiquette::Base, _, None) => {}
                _ => return false,
            }
        }
    }
    (!etiquetee && tout_en_tva) || (collectee_e == collectee_c && deductible_e == deductible_c)
}

proptest! {
    /// La règle accepte exactement ce que son énoncé accepte.
    #[test]
    fn la_regle_est_celle_de_son_enonce(lignes in prop::collection::vec(ligne_aleatoire(), 1..7)) {
        prop_assert_eq!(
            rapprocher_etiquettes_et_comptes(&lignes, role).is_ok(),
            attendu_ok(&lignes)
        );
    }

    /// Une écriture acceptée n'a jamais de taxe étiquetée sur un compte étranger à la TVA, ni
    /// de base sur un compte de TVA.
    #[test]
    fn jamais_de_taxe_sur_un_compte_etranger(lignes in prop::collection::vec(ligne_aleatoire(), 1..7)) {
        if rapprocher_etiquettes_et_comptes(&lignes, role).is_ok() {
            for l in &lignes {
                if let Some(e) = l.etiquette() {
                    let r = role(l.compte());
                    match e.nature() {
                        NatureEtiquette::Taxe => prop_assert!(r.is_some_and(|r| r.sens() == Some(e.sens()))),
                        NatureEtiquette::Base => prop_assert!(r.is_none()),
                    }
                }
            }
        }
    }

    /// Une vente ou un achat bien formé est toujours accepté, et ne l'est plus quand on retire
    /// l'étiquette de la ligne de taxe.
    #[test]
    fn facture_bien_formee_acceptee_sans_etiquette_refusee(ht in 1u32..100_000, collectee in any::<bool>()) {
        let tva = (u64::from(ht) * 18 / 100).max(1).to_string();
        let ht_s = ht.to_string();
        let ttc = (u64::from(ht) + tva.parse::<u64>().unwrap()).to_string();
        let (compte_taxe, compte_base, s, sens_base, sens_taxe) = if collectee {
            ("4431", "7011", SensTva::Collectee, Sens::Credit, Sens::Credit)
        } else {
            ("4452", "6011", SensTva::Deductible, Sens::Debit, Sens::Debit)
        };
        let tiers = if collectee { ligne("4111", Sens::Debit, &ttc) } else { ligne("4011", Sens::Credit, &ttc) };
        let bien = vec![
            tiers.clone(),
            etiquetee(compte_base, sens_base, &ht_s, base(s)),
            etiquetee(compte_taxe, sens_taxe, &tva, taxe(s)),
        ];
        prop_assert!(rapprocher_etiquettes_et_comptes(&bien, role).is_ok());
        let sans = vec![tiers, ligne(compte_base, sens_base, &ht_s), ligne(compte_taxe, sens_taxe, &tva)];
        prop_assert!(rapprocher_etiquettes_et_comptes(&sans, role).is_err());
    }
}
