//! Paquet du Sénégal, version 2025.1 : présence de chaque entrée du CGI, unicité, refus des
//! données invalides, pilotage de l'étiquette fiscale.
#![allow(
    clippy::unwrap_used,
    clippy::too_many_lines,
    clippy::items_after_statements
)]

use compta_core::{
    CodeMotif, CodeRegime, EtiquetteFiscale, Exigibilite, NatureEtiquette, ReferentielFiscal,
    SensTva, TauxTva,
};
use fiscal::{Deduction, ErreurFiscale, ParametresFiscaux, Portee, Registre, Socle};
use time::macros::date;

const VERSION: &str = include_str!("../data/pays/sn/2025.1/version.toml");
const REGIMES: &str = include_str!("../data/pays/sn/2025.1/regimes-tva.toml");
const MOTIFS: &str = include_str!("../data/pays/sn/2025.1/motifs-exoneration.toml");

/// Codes de régime du socle : liste FERMÉE à ajout seul (ils entrent dans le contenu haché).
const CODES_REGIMES: [&str; 13] = [
    "TAXABLE_NORMAL",
    "TAXABLE_REDUIT",
    "TAXABLE_ENCAISSEMENT",
    "EXO_SANS_DEDUCTION",
    "EXO_AVEC_DEDUCTION",
    "HORS_CHAMP",
    "AUTOLIQUIDATION",
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

fn charger(version: &str, regimes: &str, motifs: &str) -> Result<ParametresFiscaux, ErreurFiscale> {
    ParametresFiscaux::depuis_texte(
        &Socle::officiel().unwrap(),
        Portee::Pack,
        &[version, regimes, motifs].join("\n"),
    )
}

fn sn() -> ParametresFiscaux {
    charger(VERSION, REGIMES, MOTIFS).unwrap()
}

#[test]
fn le_socle_est_ferme_et_stable() {
    let socle = Socle::officiel().unwrap();
    let codes: Vec<&str> = socle.regimes().iter().map(|r| r.code().as_str()).collect();
    assert_eq!(codes, CODES_REGIMES);
    // Aucun code du socle ne porte un taux ni une référence de loi.
    for code in codes {
        assert!(
            !code.chars().any(|c| c.is_ascii_digit()),
            "{code} contient un chiffre"
        );
    }
    assert_eq!(socle.plancher_retention(), 10);
}

#[test]
fn chaque_regime_du_cgi_est_present() {
    let r = sn();
    assert_eq!(r.regimes().len(), CODES_REGIMES.len());
    for code in CODES_REGIMES {
        assert!(r.regime(code).is_some(), "{code}");
    }
    assert!(r.regime("IMP18").is_none(), "ancien code retiré");
    let normal = r.regime("TAXABLE_NORMAL").unwrap();
    assert_eq!((normal.article(), normal.pages()), ("369", &[184][..]));
    assert_eq!(normal.taux().len(), 1);
    assert_eq!(normal.taux()[0].to_string(), "18");
    let reduit = r.regime("TAXABLE_REDUIT").unwrap();
    assert_eq!(reduit.alinea(), Some("2"));
    assert_eq!(reduit.taux()[0].to_string(), "10");
    assert_eq!(
        r.regime("EXO_SANS_DEDUCTION").unwrap().deduction(),
        Deduction::NonSaufExceptions
    );
    assert_eq!(
        r.regime("PRECOMPTE").unwrap().deduction(),
        Deduction::OuiSiDeclaree
    );
    assert!(r.regime("AUTOLIQUIDATION").unwrap().a_valider());
    // Le taux vient de la version, pas du code.
    assert_eq!(r.taux_du_role("normal").unwrap().to_string(), "18");
    assert_eq!(r.taux_du_role("reduit").unwrap().to_string(), "10");
    assert_eq!(r.regime("MARGE").unwrap().taux().len(), 2);
    assert_eq!(r.regime("REGUL").unwrap().taux().len(), 0);
    assert_eq!(r.retention().annees_min, 10);
}

#[test]
fn chaque_motif_du_cgi_est_present() {
    let r = sn();
    for n in ALINEAS_361 {
        let m = r.motif(&format!("EXO_SANS_DEDUCTION-{n}")).unwrap();
        assert_eq!((m.article(), m.alinea()), ("361", n.to_string().as_str()));
        assert_eq!(m.deduction(), Deduction::Non);
    }
    for k in ["a", "b", "c", "d", "e", "f"] {
        let m = r.motif(&format!("EXO_AVEC_DEDUCTION-{k}")).unwrap();
        assert_eq!((m.article(), m.pages()), ("380", &[189][..]));
        assert_eq!(m.deduction(), Deduction::Oui);
    }
    assert_eq!(r.motifs().len(), ALINEAS_361.len() + 6);
    // 361-14, 361-15 et 361-26 ne sont pas des codes distincts.
    for n in [14, 15, 26] {
        assert!(r.motif(&format!("EXO_SANS_DEDUCTION-{n}")).is_none());
    }
    let cibles: Vec<(&str, &str)> = r
        .chevauchements()
        .iter()
        .map(|c| (c.alinea.as_str(), c.motif.as_str()))
        .collect();
    assert_eq!(
        cibles,
        [
            ("14", "EXO_AVEC_DEDUCTION-a"),
            ("15", "EXO_AVEC_DEDUCTION-d"),
            ("26", "EXO_AVEC_DEDUCTION-e")
        ]
    );
}

#[test]
fn codes_uniques_et_pages_verifiees() {
    let r = sn();
    let pages = |n: &str| r.motif(&format!("EXO_SANS_DEDUCTION-{n}")).unwrap().pages()[0];
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
    // Aucun code de régime ne porte un taux ni un article du CGI.
    for regime in r.regimes() {
        assert!(
            !regime
                .code()
                .as_str()
                .contains(|c: char| c.is_ascii_digit())
        );
    }
}

#[test]
fn fin_de_validite_du_motif_agricole() {
    let r = sn();
    let e = r.motif("EXO_AVEC_DEDUCTION-e").unwrap();
    assert_eq!(e.fin(), Some(date!(2026 - 12 - 31)));
    let regime = CodeRegime::parse("EXO_AVEC_DEDUCTION").unwrap();
    let motif = CodeMotif::parse("EXO_AVEC_DEDUCTION-e").unwrap();
    assert!(
        r.a_la_date(date!(2026 - 12 - 31))
            .motif_admis(&regime, &motif)
    );
    assert!(
        !r.a_la_date(date!(2027 - 01 - 01))
            .motif_admis(&regime, &motif)
    );
    // Les autres motifs de l'art. 380 restent valides en 2027.
    let a = CodeMotif::parse("EXO_AVEC_DEDUCTION-a").unwrap();
    assert!(r.a_la_date(date!(2027 - 01 - 01)).motif_admis(&regime, &a));
}

#[test]
fn refus_des_donnees_invalides() {
    let rejet =
        |version: &str, regimes: &str, motifs: &str| charger(version, regimes, motifs).unwrap_err();
    // Code en double.
    let double = REGIMES.replacen("code = \"TAXABLE_REDUIT\"", "code = \"TAXABLE_NORMAL\"", 1);
    assert!(matches!(
        rejet(VERSION, &double, MOTIFS),
        ErreurFiscale::CodeEnDouble(_)
    ));
    // Régime hors socle : un paquet ne crée pas de code.
    let hors = REGIMES.replacen("code = \"MARGE\"", "code = \"NOUVEAU\"", 1);
    assert!(matches!(
        rejet(VERSION, &hors, MOTIFS),
        ErreurFiscale::HorsSocle(_)
    ));
    // Taux non décimal ou non canonique.
    for t in ["dix-huit", "18.0", "1e1", "-5"] {
        let faux = VERSION.replacen("valeur = \"18\"", &format!("valeur = \"{t}\""), 1);
        assert!(
            matches!(rejet(&faux, REGIMES, MOTIFS), ErreurFiscale::Taux { .. }),
            "{t}"
        );
    }
    // Rôle de taux inconnu du socle.
    let role = VERSION.replacen("[taux.reduit]", "[taux.exotique]", 1);
    assert!(matches!(
        rejet(&role, REGIMES, MOTIFS),
        ErreurFiscale::RoleInconnu(_)
    ));
    // Entrée sans source, ou page hors du chapitre du document.
    let sans_page = REGIMES.replacen("pages = [184]", "pages = []", 1);
    assert!(matches!(
        rejet(VERSION, &sans_page, MOTIFS),
        ErreurFiscale::SansSource(_)
    ));
    let hors_page = MOTIFS.replacen("pages = [176]", "pages = [3]", 1);
    assert!(matches!(
        rejet(VERSION, REGIMES, &hors_page),
        ErreurFiscale::Page { .. }
    ));
    // Doublon d'article et d'alinéa.
    let doublon = MOTIFS.replacen("alinea = \"2\"", "alinea = \"1\"", 1);
    assert!(matches!(
        rejet(VERSION, REGIMES, &doublon),
        ErreurFiscale::ArticleEnDouble(_)
    ));
    // Motif d'un régime absent ou non exonéré ; champ inconnu ; champ de taux dans une entrée
    // (le taux n'est plus dans l'entrée).
    let absent = MOTIFS.replacen("regime = \"EXO_SANS_DEDUCTION\"", "regime = \"EXO999\"", 1);
    assert!(matches!(
        rejet(VERSION, REGIMES, &absent),
        ErreurFiscale::RegimeAbsent { .. }
    ));
    let imposable = MOTIFS.replacen(
        "regime = \"EXO_SANS_DEDUCTION\"",
        "regime = \"TAXABLE_NORMAL\"",
        1,
    );
    assert!(matches!(
        rejet(VERSION, REGIMES, &imposable),
        ErreurFiscale::Incoherente { .. }
    ));
    let inconnu = REGIMES.replacen("[[regime]]", "[[regime]]\nsecret = 1", 1);
    assert!(matches!(
        rejet(VERSION, &inconnu, MOTIFS),
        ErreurFiscale::Syntaxe(_)
    ));
    let avec_taux = REGIMES.replacen("[[regime]]", "[[regime]]\ntaux = [\"18\"]", 1);
    assert!(matches!(
        rejet(VERSION, &avec_taux, MOTIFS),
        ErreurFiscale::Syntaxe(_)
    ));
    // Motif dont le code n'est pas préfixé du code de son régime.
    let prefixe = MOTIFS.replacen("code = \"EXO_SANS_DEDUCTION-1\"", "code = \"AUTRE-1\"", 1);
    assert!(matches!(
        rejet(VERSION, REGIMES, &prefixe),
        ErreurFiscale::Incoherente { .. }
    ));
    // Pays, version, dates, document, rétention.
    let pays = VERSION.replacen("pays = \"SN\"", "pays = \"sn\"", 1);
    assert!(matches!(
        rejet(&pays, REGIMES, MOTIFS),
        ErreurFiscale::Pays(_)
    ));
    let version = VERSION.replacen("version = \"2025.1\"", "version = \"2025\"", 1);
    assert!(matches!(
        rejet(&version, REGIMES, MOTIFS),
        ErreurFiscale::Version(_)
    ));
    let jour = VERSION.replacen("2025-01-01", "2025-02-30", 1);
    assert!(matches!(
        rejet(&jour, REGIMES, MOTIFS),
        ErreurFiscale::Date(_)
    ));
    let fin = VERSION.replacen(
        "date_effet = \"2025-01-01\"",
        "date_effet = \"2025-01-01\"\ndate_fin = \"2024-12-31\"",
        1,
    );
    assert!(matches!(
        rejet(&fin, REGIMES, MOTIFS),
        ErreurFiscale::Periode(_)
    ));
    let periode = MOTIFS.replacen(
        "fin = \"2026-12-31\"",
        "debut = \"2027-01-01\"\nfin = \"2026-12-31\"",
        1,
    );
    assert!(matches!(
        rejet(VERSION, REGIMES, &periode),
        ErreurFiscale::Periode(_)
    ));
    let doc = REGIMES.replacen("pages = [184]", "pages = [184]\ndocument = \"autre\"", 1);
    assert!(matches!(
        rejet(VERSION, &doc, MOTIFS),
        ErreurFiscale::Document(_)
    ));
    let retention = VERSION.replacen("annees_min = 10", "annees_min = 9", 1);
    assert!(matches!(
        rejet(&retention, REGIMES, MOTIFS),
        ErreurFiscale::RetentionSousPlancher {
            annees: 9,
            plancher: 10
        }
    ));
}

#[test]
fn le_referentiel_pilote_l_etiquette() {
    let p = sn();
    let r = p.a_la_date(date!(2026 - 06 - 30));
    let regime = |c: &str| CodeRegime::parse(c).unwrap();
    let motif = |c: &str| Some(CodeMotif::parse(c).unwrap());
    let taux = |t: &str| Some(TauxTva::parse(t).unwrap());
    let faire = |nature, reg: &str, t, e, m| {
        EtiquetteFiscale::nouvelle(nature, SensTva::Collectee, regime(reg), t, e, m, &r)
    };
    let ok = |nature, reg, t, e, m| assert!(faire(nature, reg, t, e, m).is_ok(), "{reg}");
    let ko = |nature, reg, t, e, m| assert!(faire(nature, reg, t, e, m).is_err(), "{reg}");
    use NatureEtiquette::{Base, Taxe};
    ok(
        Taxe,
        "TAXABLE_NORMAL",
        taux("18"),
        Some(Exigibilite::Debit),
        None,
    );
    ok(
        Base,
        "TAXABLE_REDUIT",
        taux("10"),
        Some(Exigibilite::Encaissement),
        None,
    );
    ko(
        Base,
        "TAXABLE_REDUIT",
        taux("18"),
        Some(Exigibilite::Debit),
        None,
    );
    ko(
        Base,
        "TAXABLE_NORMAL",
        taux("10"),
        Some(Exigibilite::Debit),
        None,
    );
    ko(
        Base,
        "TAXABLE_ENCAISSEMENT",
        taux("18"),
        Some(Exigibilite::Debit),
        None,
    );
    ok(
        Base,
        "TAXABLE_ENCAISSEMENT",
        taux("10"),
        Some(Exigibilite::Encaissement),
        None,
    );
    ok(
        Base,
        "EXO_SANS_DEDUCTION",
        None,
        None,
        motif("EXO_SANS_DEDUCTION-7"),
    );
    ok(
        Base,
        "EXO_AVEC_DEDUCTION",
        None,
        None,
        motif("EXO_AVEC_DEDUCTION-a"),
    );
    ko(
        Base,
        "EXO_SANS_DEDUCTION",
        None,
        None,
        motif("EXO_AVEC_DEDUCTION-a"),
    );
    ko(
        Base,
        "EXO_SANS_DEDUCTION",
        None,
        None,
        motif("EXO_SANS_DEDUCTION-14"),
    );
    ko(Base, "EXO_SANS_DEDUCTION", None, None, None);
    ok(Base, "HORS_CHAMP", None, None, None);
    ko(Base, "NOUVEAU", None, None, None);
    assert!(r.profil_regime(&regime("REGUL")).is_some());
    // Les anciens codes propres au Sénégal sont retirés, non aliasés.
    for ancien in [
        "IMP18",
        "IMP10",
        "IMP_ENC",
        "EXO361",
        "EXO380",
        "AUTOLIQ_REPFISC",
    ] {
        ko(Base, ancien, None, None, None);
    }
}

#[test]
fn ligne_de_taxe_selon_le_regime() {
    let p = sn();
    for code in [
        "EXO_SANS_DEDUCTION",
        "EXO_AVEC_DEDUCTION",
        "HORS_CHAMP",
        "SUSPENSION",
    ] {
        assert!(!p.regime(code).unwrap().porte_une_ligne_de_taxe(), "{code}");
    }
    for code in [
        "TAXABLE_NORMAL",
        "TAXABLE_REDUIT",
        "TAXABLE_ENCAISSEMENT",
        "AUTOLIQUIDATION",
        "COLLECTE_NUM",
        "PRECOMPTE",
        "MARGE",
        "REGUL",
        "REMB_ANNUL",
    ] {
        assert!(p.regime(code).unwrap().porte_une_ligne_de_taxe(), "{code}");
    }
    for regime in p.regimes() {
        assert!(
            regime.source_ligne_taxe().contains("art."),
            "{}",
            regime.code()
        );
    }
    let r = p.a_la_date(date!(2026 - 01 - 15));
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
    for code in ["HORS_CHAMP", "SUSPENSION"] {
        assert!(matches!(
            faire(NatureEtiquette::Taxe, code, None, None),
            Err(compta_core::ErreurEtiquette::LigneTaxeInterdite(_))
        ));
        assert!(faire(NatureEtiquette::Base, code, None, None).is_ok());
    }
    let enc = Some(Exigibilite::Encaissement);
    assert!(faire(NatureEtiquette::Taxe, "PRECOMPTE", Some("18"), enc).is_ok());
    assert!(faire(NatureEtiquette::Taxe, "COLLECTE_NUM", Some("18"), None).is_ok());
    assert!(faire(NatureEtiquette::Taxe, "AUTOLIQUIDATION", Some("18"), None).is_ok());
    assert!(faire(NatureEtiquette::Taxe, "REGUL", None, None).is_ok());
    assert!(faire(NatureEtiquette::Taxe, "REMB_ANNUL", None, None).is_ok());
}

#[test]
fn le_motif_b_de_l_exoneration_avec_deduction_mentionne_l_exclusion_des_telecommunications() {
    let p = sn();
    let b = p.motif("EXO_AVEC_DEDUCTION-b").unwrap();
    assert!(b.libelle().contains("télécommunications"));
    assert!(b.libelle().contains("358"));
}

#[test]
fn le_registre_officiel_charge_le_senegal() {
    let registre = Registre::officiel().unwrap();
    assert_eq!(registre.pays().collect::<Vec<_>>(), ["SN"]);
    let versions = registre.versions_du_pays("SN");
    let ids: Vec<&str> = versions
        .iter()
        .map(ParametresFiscaux::identifiant)
        .collect();
    assert_eq!(ids, ["sn-2025.1", "sn-2025.2"]);
    assert_eq!(registre.versions_du_pays("CM").len(), 0);
}
