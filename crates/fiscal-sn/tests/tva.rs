//! Données de TVA : présence de chaque entrée du CGI, unicité, refus des données invalides.
#![allow(clippy::unwrap_used)]

use compta_core::{
    CodeMotif, CodeRegime, EtiquetteFiscale, Exigibilite, NatureEtiquette, ReferentielFiscal,
    SensTva, TauxTva,
};
use fiscal_sn::{Deduction, ErreurFiscale, ReferentielTva};

const REGIMES: &str = include_str!("../data/regimes-tva.toml");
const MOTIFS: &str = include_str!("../data/motifs-exoneration.toml");

/// Codes de régime : liste FERMÉE à ajout seul (ils entrent dans le contenu haché).
const CODES_REGIMES: [&str; 13] = [
    "IMP18",
    "IMP10",
    "IMP_ENC",
    "EXO361",
    "EXO380",
    "HORS_CHAMP",
    "AUTOLIQ_REPFISC",
    "COLLECTE_NUM",
    "PRECOMPTE",
    "MARGE",
    "SUSPENSION",
    "REGUL",
    "REMB_ANNUL",
];

/// Alinéas de l'art. 361 avec un code propre (14, 15 et 26 sont portés par l'art. 380).
const ALINEAS_361: [u8; 25] = [
    1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 27, 28,
];

#[test]
fn chaque_regime_du_cgi_est_present() {
    let r = ReferentielTva::charger().unwrap();
    assert_eq!(r.regimes().len(), CODES_REGIMES.len());
    for code in CODES_REGIMES {
        assert!(r.regime(code).is_some(), "{code}");
    }
    assert!(r.regime("INCONNU").is_none());
    let imp18 = r.regime("IMP18").unwrap();
    assert_eq!((imp18.article(), imp18.pages()), ("369", &[184][..]));
    assert_eq!(imp18.taux()[0].to_string(), "18");
    assert_eq!(r.regime("IMP10").unwrap().alinea(), Some("2"));
    assert_eq!(
        r.regime("EXO361").unwrap().deduction(),
        Deduction::NonSaufChevauchement
    );
    assert_eq!(
        r.regime("PRECOMPTE").unwrap().deduction(),
        Deduction::OuiSiDeclaree
    );
    assert!(r.regime("AUTOLIQ_REPFISC").unwrap().a_valider());
}

#[test]
fn chaque_motif_du_cgi_est_present() {
    let r = ReferentielTva::charger().unwrap();
    for n in ALINEAS_361 {
        let m = r.motif(&format!("EXO361-{n}")).unwrap();
        assert_eq!((m.article(), m.alinea()), ("361", n.to_string().as_str()));
        assert_eq!(m.deduction(), Deduction::Non);
    }
    for k in ["a", "b", "c", "d", "e", "f"] {
        let m = r.motif(&format!("EXO380-{k}")).unwrap();
        assert_eq!((m.article(), m.pages()), ("380", &[189][..]));
        assert_eq!(m.deduction(), Deduction::Oui);
    }
    assert_eq!(r.motifs().len(), ALINEAS_361.len() + 6);
    // 361-14, 361-15 et 361-26 ne sont pas des codes distincts.
    for n in [14, 15, 26] {
        assert!(r.motif(&format!("EXO361-{n}")).is_none());
    }
    let cibles: Vec<(&str, &str)> = r
        .chevauchements()
        .iter()
        .map(|c| (c.alinea.as_str(), c.motif.as_str()))
        .collect();
    assert_eq!(
        cibles,
        [("14", "EXO380-a"), ("15", "EXO380-d"), ("26", "EXO380-e")]
    );
}

#[test]
fn codes_uniques_et_pages_verifiees() {
    let r = ReferentielTva::charger().unwrap();
    // 361-n : page vérifiée sur le PDF, folio = page.
    let pages = |n: &str| r.motif(&format!("EXO361-{n}")).unwrap().pages()[0];
    assert_eq!(
        [
            pages("1"),
            pages("4"),
            pages("9"),
            pages("18"),
            pages("25"),
            pages("28")
        ],
        [176, 177, 178, 179, 180, 181]
    );
    let mut codes: Vec<&str> = r.regimes().iter().map(|x| x.code().as_str()).collect();
    codes.extend(r.motifs().iter().map(|m| m.code().as_str()));
    let n = codes.len();
    codes.sort_unstable();
    codes.dedup();
    assert_eq!(codes.len(), n);
}

#[test]
fn refus_des_donnees_invalides() {
    let rejet =
        |regimes: &str, motifs: &str| ReferentielTva::depuis_textes(regimes, motifs).unwrap_err();
    // Code en double.
    let double = REGIMES.replacen("code = \"IMP10\"", "code = \"IMP18\"", 1);
    assert!(matches!(
        rejet(&double, MOTIFS),
        ErreurFiscale::CodeEnDouble(_)
    ));
    // Taux non décimal ou non canonique.
    for t in ["\"dix-huit\"", "\"18.0\"", "\"1e1\"", "\"-5\""] {
        let faux = REGIMES.replacen("taux = [\"18\"]", &format!("taux = [{t}]"), 1);
        assert!(
            matches!(rejet(&faux, MOTIFS), ErreurFiscale::Taux { .. }),
            "{t}"
        );
    }
    // Entrée sans source.
    let sans_page = REGIMES.replacen(
        "pages = [184]\nexonere = false\ndeduction = \"oui\"",
        "pages = []\nexonere = false\ndeduction = \"oui\"",
        1,
    );
    assert!(matches!(
        rejet(&sans_page, MOTIFS),
        ErreurFiscale::SansSource(_)
    ));
    let hors = MOTIFS.replacen("pages = [176]", "pages = [3]", 1);
    assert!(matches!(rejet(REGIMES, &hors), ErreurFiscale::Page { .. }));
    // Doublon d'article et d'alinéa.
    let doublon = MOTIFS.replacen("alinea = \"2\"", "alinea = \"1\"", 1);
    assert!(matches!(
        rejet(REGIMES, &doublon),
        ErreurFiscale::ArticleEnDouble(_)
    ));
    // Motif d'un régime absent ou non exonéré ; champ inconnu.
    let absent = MOTIFS.replacen("regime = \"EXO361\"", "regime = \"EXO999\"", 1);
    assert!(matches!(
        rejet(REGIMES, &absent),
        ErreurFiscale::RegimeAbsent { .. }
    ));
    let imposable = MOTIFS.replacen("regime = \"EXO361\"", "regime = \"IMP18\"", 1);
    assert!(matches!(
        rejet(REGIMES, &imposable),
        ErreurFiscale::Incoherente { .. }
    ));
    let inconnu = REGIMES.replacen("[[regime]]", "[[regime]]\nsecret = 1", 1);
    assert!(matches!(rejet(&inconnu, MOTIFS), ErreurFiscale::Syntaxe(_)));
    // Régime exonéré avec un taux.
    let exo = REGIMES
        .replacen("code = \"EXO380\"", "code = \"EXO380\"\n", 1)
        .replace(
            "taux = []\nexigibilites_etiquette = []\na_valider = true\nnote = \"Ouvre",
            "taux = [\"18\"]\nexigibilites_etiquette = []\na_valider = true\nnote = \"Ouvre",
        );
    assert!(matches!(
        rejet(&exo, MOTIFS),
        ErreurFiscale::Incoherente { .. }
    ));
}

#[test]
fn le_referentiel_pilote_l_etiquette() {
    let r = ReferentielTva::charger().unwrap();
    let regime = |c: &str| CodeRegime::parse(c).unwrap();
    let motif = |c: &str| Some(CodeMotif::parse(c).unwrap());
    let taux = |t: &str| Some(TauxTva::parse(t).unwrap());
    let faire = |nature, reg: &str, t, e, m| {
        EtiquetteFiscale::nouvelle(nature, SensTva::Collectee, regime(reg), t, e, m, &r)
    };
    assert!(
        faire(
            NatureEtiquette::Taxe,
            "IMP18",
            taux("18"),
            Some(Exigibilite::Debit),
            None
        )
        .is_ok()
    );
    assert!(
        faire(
            NatureEtiquette::Base,
            "IMP10",
            taux("10"),
            Some(Exigibilite::Encaissement),
            None
        )
        .is_ok()
    );
    assert!(
        faire(
            NatureEtiquette::Base,
            "IMP10",
            taux("18"),
            Some(Exigibilite::Debit),
            None
        )
        .is_err()
    );
    assert!(
        faire(
            NatureEtiquette::Base,
            "IMP_ENC",
            taux("18"),
            Some(Exigibilite::Debit),
            None
        )
        .is_err()
    );
    assert!(
        faire(
            NatureEtiquette::Base,
            "EXO361",
            None,
            None,
            motif("EXO361-7")
        )
        .is_ok()
    );
    assert!(
        faire(
            NatureEtiquette::Base,
            "EXO380",
            None,
            None,
            motif("EXO380-a")
        )
        .is_ok()
    );
    assert!(
        faire(
            NatureEtiquette::Base,
            "EXO361",
            None,
            None,
            motif("EXO380-a")
        )
        .is_err()
    );
    assert!(
        faire(
            NatureEtiquette::Base,
            "EXO361",
            None,
            None,
            motif("EXO361-14")
        )
        .is_err()
    );
    assert!(faire(NatureEtiquette::Base, "EXO361", None, None, None).is_err());
    assert!(faire(NatureEtiquette::Base, "HORS_CHAMP", None, None, None).is_ok());
    assert!(faire(NatureEtiquette::Base, "NOUVEAU", None, None, None).is_err());
    assert!(r.profil_regime(&regime("REGUL")).is_some());
}

#[test]
fn ligne_de_taxe_selon_le_regime() {
    let r = ReferentielTva::charger().unwrap();
    // Sans ligne de taxe : exonérés, hors champ, suspension. Chaque indicateur cite sa source.
    for code in ["EXO361", "EXO380", "HORS_CHAMP", "SUSPENSION"] {
        assert!(!r.regime(code).unwrap().porte_une_ligne_de_taxe(), "{code}");
    }
    for code in [
        "IMP18",
        "IMP10",
        "IMP_ENC",
        "AUTOLIQ_REPFISC",
        "COLLECTE_NUM",
        "PRECOMPTE",
        "MARGE",
        "REGUL",
        "REMB_ANNUL",
    ] {
        assert!(r.regime(code).unwrap().porte_une_ligne_de_taxe(), "{code}");
    }
    for regime in r.regimes() {
        assert!(
            regime.source_ligne_taxe().contains("art."),
            "{}",
            regime.code()
        );
    }
    let faire = |nature, reg: &str, t: Option<&str>, e| {
        EtiquetteFiscale::nouvelle(
            nature,
            SensTva::Collectee,
            CodeRegime::parse(reg).unwrap(),
            t.map(|t| TauxTva::parse(t).unwrap()),
            e,
            None,
            &r,
        )
    };
    // Refus : aucune ligne `taxe` sous HORS_CHAMP ni SUSPENSION.
    for code in ["HORS_CHAMP", "SUSPENSION"] {
        assert!(matches!(
            faire(NatureEtiquette::Taxe, code, None, None),
            Err(compta_core::ErreurEtiquette::LigneTaxeInterdite(_))
        ));
        assert!(faire(NatureEtiquette::Base, code, None, None).is_ok());
    }
    // Acceptation : la taxe du précompte (372-2), de la collecte numérique (355 bis), de
    // l'autoliquidation (355-3), de la régularisation (387-1) et du remboursement (389-1).
    let enc = Some(Exigibilite::Encaissement);
    assert!(faire(NatureEtiquette::Taxe, "PRECOMPTE", Some("18"), enc).is_ok());
    assert!(faire(NatureEtiquette::Taxe, "COLLECTE_NUM", Some("18"), None).is_ok());
    assert!(faire(NatureEtiquette::Taxe, "AUTOLIQ_REPFISC", Some("18"), None).is_ok());
    assert!(faire(NatureEtiquette::Taxe, "REGUL", None, None).is_ok());
    assert!(faire(NatureEtiquette::Taxe, "REMB_ANNUL", None, None).is_ok());
}

#[test]
fn exo380_b_mentionne_l_exclusion_des_telecommunications() {
    let r = ReferentielTva::charger().unwrap();
    let b = r.motif("EXO380-b").unwrap();
    assert!(b.libelle().contains("télécommunications"));
    assert!(b.libelle().contains("358"));
}
