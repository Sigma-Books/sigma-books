//! Surcharge d'une version par un dossier : sérialisation relue à l'identique, dérivation,
//! refus, propriété sur les taux.
#![allow(clippy::unwrap_used, clippy::too_many_lines)]

use std::fs;
use std::path::Path;

use fiscal::{
    ErreurFiscale, MOTIF_MAX_OCTETS, ParametresFiscaux, Portee, Registre, Socle, Surcharge,
};
use proptest::prelude::*;
use time::Date;
use time::macros::date;

fn socle() -> Socle {
    Socle::officiel().unwrap()
}

fn sn() -> ParametresFiscaux {
    Registre::officiel().unwrap().versions_du_pays("SN")[0].clone()
}

fn factice(version: &str) -> ParametresFiscaux {
    let lire = |f: &str| {
        fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/zz")
                .join(version)
                .join(f),
        )
        .unwrap()
    };
    let texte = [
        lire("version.toml"),
        lire("regimes-tva.toml"),
        lire("motifs-exoneration.toml"),
    ]
    .join("\n");
    ParametresFiscaux::depuis_texte(&socle(), Portee::Pack, &texte).unwrap()
}

fn surcharge<'a>(
    effet: Date,
    taux: &'a [(String, String)],
    retires: &'a [String],
) -> Surcharge<'a> {
    Surcharge {
        numero: 1,
        date_effet: effet,
        date_fin: None,
        motif: "Convention particulière du client",
        taux,
        regimes_retires: retires,
    }
}

#[test]
fn le_contenu_se_relit_a_l_identique() {
    for p in [sn(), factice("2024.1"), factice("2026.1")] {
        let relu =
            ParametresFiscaux::depuis_texte(&socle(), Portee::Pack, &p.vers_texte()).unwrap();
        assert_eq!(relu, p, "{}", p.identifiant());
    }
}

#[test]
fn une_surcharge_remplace_un_taux_et_garde_le_reste() {
    let base = sn();
    let taux = [("normal".to_owned(), "19".to_owned())];
    let d = base
        .deriver(&socle(), &surcharge(date!(2027 - 01 - 01), &taux, &[]))
        .unwrap();
    assert_eq!(d.identifiant(), "sn-2025.1+d1");
    assert_eq!(d.date_effet(), date!(2027 - 01 - 01));
    assert_eq!(d.taux_du_role("normal").unwrap().to_string(), "19");
    assert_eq!(d.taux_du_role("reduit").unwrap().to_string(), "10");
    // Les régimes taxables au taux normal admettent le nouveau taux, pas l'ancien.
    let normal = d.regime("TAXABLE_NORMAL").unwrap();
    assert_eq!(
        normal
            .taux()
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>(),
        ["19"]
    );
    // Motifs, régimes et rétention sont ceux de la base ; la base n'a pas bougé.
    assert_eq!(d.motifs().len(), base.motifs().len());
    assert_eq!(d.regimes().len(), base.regimes().len());
    assert_eq!(d.retention(), base.retention());
    assert_eq!(base.taux_du_role("normal").unwrap().to_string(), "18");
    // Le contenu dérivé se relit : c'est ce qui est stocké.
    let relu =
        ParametresFiscaux::depuis_texte(&socle(), Portee::Dossier(1), &d.vers_texte()).unwrap();
    assert_eq!(relu, d);
}

#[test]
fn une_surcharge_retire_un_regime_sans_motifs() {
    let base = sn();
    let retires = ["MARGE".to_owned()];
    let d = base
        .deriver(&socle(), &surcharge(date!(2027 - 01 - 01), &[], &retires))
        .unwrap();
    assert!(d.regime("MARGE").is_none());
    assert_eq!(d.regimes().len(), base.regimes().len() - 1);
}

#[test]
fn les_surcharges_invalides_sont_refusees() {
    let base = sn();
    let jour = date!(2027 - 01 - 01);
    let essai = |taux: &[(String, String)], retires: &[String]| {
        base.deriver(&socle(), &surcharge(jour, taux, retires))
            .unwrap_err()
    };
    let t = |r: &str, v: &str| [(r.to_owned(), v.to_owned())];
    // Rien ne change.
    assert_eq!(essai(&[], &[]), ErreurFiscale::SurchargeVide);
    // Taux hors des bornes ou mal formé : 0, négatif, au-delà de 100, texte, exposant, zéro final.
    for v in ["0", "-5", "100.01", "101", "dix", "1e1", "18.0", ""] {
        assert!(
            matches!(essai(&t("normal", v), &[]), ErreurFiscale::Taux { .. }),
            "{v}"
        );
    }
    // Rôle inconnu de la base.
    assert!(matches!(
        essai(&t("exotique", "5"), &[]),
        ErreurFiscale::RoleInconnu(_)
    ));
    // Régime inconnu, hors socle, exonéré (a des motifs).
    assert!(matches!(
        essai(&[], &["NOUVEAU".to_owned()]),
        ErreurFiscale::RegimeNonOffert(_)
    ));
    assert!(matches!(
        essai(&[], &["EXO_SANS_DEDUCTION".to_owned()]),
        ErreurFiscale::RegimeNonRetirable(_)
    ));
    // Motif vide, trop long, avec contrôle ; fin avant le début.
    let dix_neuf = t("normal", "19");
    for motif in [" ", &"é".repeat(MOTIF_MAX_OCTETS / 2 + 1), "a\nb"] {
        let s = Surcharge {
            motif,
            ..surcharge(jour, &dix_neuf, &[])
        };
        assert_eq!(
            base.deriver(&socle(), &s).unwrap_err(),
            ErreurFiscale::MotifInvalide
        );
    }
    let s = Surcharge {
        date_fin: Some(date!(2026 - 12 - 31)),
        ..surcharge(jour, &dix_neuf, &[])
    };
    assert!(matches!(
        base.deriver(&socle(), &s),
        Err(ErreurFiscale::Periode(_))
    ));
    // Un motif de 400 octets exactement est admis.
    let juste = "a".repeat(MOTIF_MAX_OCTETS);
    let s = Surcharge {
        motif: &juste,
        ..surcharge(jour, &dix_neuf, &[])
    };
    assert!(base.deriver(&socle(), &s).is_ok());
}

#[test]
fn le_motif_ne_casse_pas_le_contenu() {
    // Guillemets, barres obliques et accents : le contenu dérivé reste du TOML valide.
    let base = sn();
    let taux = [("normal".to_owned(), "19".to_owned())];
    let s = Surcharge {
        motif: "Décision « du client » \"spéciale\" \\ fin",
        ..surcharge(date!(2027 - 01 - 01), &taux, &[])
    };
    let d = base.deriver(&socle(), &s).unwrap();
    assert!(ParametresFiscaux::depuis_texte(&socle(), Portee::Dossier(1), &d.vers_texte()).is_ok());
}

proptest! {
    /// Tout taux décimal canonique dans ]0 ; 100] est admis, relu tel quel et propagé aux régimes.
    #[test]
    fn un_taux_valide_se_propage(entier in 0u32..100, centiemes in 0u32..100) {
        let valeur = if centiemes == 0 {
            if entier == 0 { "100".to_owned() } else { entier.to_string() }
        } else if centiemes % 10 == 0 {
            format!("{entier}.{}", centiemes / 10)
        } else {
            format!("{entier}.{centiemes:02}")
        };
        let taux = [("normal".to_owned(), valeur.clone())];
        let d = sn().deriver(&socle(), &surcharge(date!(2027 - 01 - 01), &taux, &[])).unwrap();
        prop_assert_eq!(d.taux_du_role("normal").unwrap().to_string(), valeur.clone());
        let regime = d.regime("TAXABLE_NORMAL").unwrap();
        prop_assert_eq!(regime.taux()[0].to_string(), valeur);
    }

    /// Un taux hors de ]0 ; 100] est refusé.
    #[test]
    fn un_taux_hors_bornes_est_refuse(entier in 101u32..100_000) {
        let taux = [("normal".to_owned(), entier.to_string())];
        let erreur = sn().deriver(&socle(), &surcharge(date!(2027 - 01 - 01), &taux, &[])).unwrap_err();
        let est_taux = matches!(erreur, ErreurFiscale::Taux { .. });
        prop_assert!(est_taux);
    }
}
