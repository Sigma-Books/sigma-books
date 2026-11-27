//! Prélèvements et taxes perçus à la source (ADR 0034 point 10 ; plan L3, L3-13) : socle,
//! version 2025.2 du Sénégal (loi n° 2025-17), sélection par date, refus des données invalides,
//! calcul de contrôle avant arrondi, pack factice d'un autre pays.
#![allow(clippy::unwrap_used, clippy::too_many_lines)]

use std::fs;
use std::path::Path;

use compta_core::{Devise, Montant};
use fiscal::{
    Assiette, ErreurFiscale, ExigibiliteLegale, ParametresFiscaux, Portee, Registre, Socle,
    Surcharge,
};
use proptest::prelude::*;
use rust_decimal::Decimal;
use time::macros::date;

const VERSION: &str = include_str!("../data/pays/sn/2025.2/version.toml");
const REGIMES: &str = include_str!("../data/pays/sn/2025.2/regimes-tva.toml");
const MOTIFS: &str = include_str!("../data/pays/sn/2025.2/motifs-exoneration.toml");
const PRELEVEMENTS: &str = include_str!("../data/pays/sn/2025.2/prelevements.toml");

/// Codes de prélèvement du socle : liste FERMÉE à ajout seul.
const CODES_PRELEVEMENTS: [&str; 2] = ["PRELEVEMENT_PAIEMENT_RECU", "TAXE_TRANSFERT_ARGENT"];

const TTA: &str = "TAXE_TRANSFERT_ARGENT";
const MARCHAND: &str = "PRELEVEMENT_PAIEMENT_RECU";

fn socle() -> Socle {
    Socle::officiel().unwrap()
}

fn charger(prelevements: &str) -> Result<ParametresFiscaux, ErreurFiscale> {
    ParametresFiscaux::depuis_texte(
        &socle(),
        Portee::Pack,
        &[VERSION, REGIMES, MOTIFS, prelevements].join("\n"),
    )
}

fn sn() -> ParametresFiscaux {
    charger(PRELEVEMENTS).unwrap()
}

fn xof(texte: &str) -> Montant {
    Montant::parse(texte, Devise::parse("XOF").unwrap()).unwrap()
}

fn factice() -> ParametresFiscaux {
    let lire = |f: &str| {
        fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/zz/2026.1")
                .join(f),
        )
        .unwrap()
    };
    let texte = [
        "version.toml",
        "regimes-tva.toml",
        "motifs-exoneration.toml",
        "prelevements.toml",
    ]
    .map(lire)
    .join("\n");
    ParametresFiscaux::depuis_texte(&socle(), Portee::Pack, &texte).unwrap()
}

#[test]
fn le_socle_des_prelevements_est_ferme_et_sans_chiffre() {
    let socle = socle();
    let codes: Vec<&str> = socle
        .prelevements()
        .iter()
        .map(|p| p.code().as_str())
        .collect();
    assert_eq!(codes, CODES_PRELEVEMENTS);
    for p in socle.prelevements() {
        assert!(!p.code().as_str().chars().any(|c| c.is_ascii_digit()));
        assert_ne!(p.nature().trim(), "");
    }
}

/// 2025.2 reprend la TVA de 2025.1 sans changement et n'ajoute que les prélèvements ; elle
/// s'applique à partir de sa date d'effet (à confirmer par le fiscaliste).
#[test]
fn la_version_2025_2_ne_change_que_les_prelevements() {
    let registre = Registre::officiel().unwrap();
    let [v1, v2] = registre.versions_du_pays("SN") else {
        panic!("deux versions attendues");
    };
    assert_eq!(v2.identifiant(), "sn-2025.2");
    assert_eq!(v1.prelevements(), []);
    assert_eq!(v2.prelevements().len(), 2);
    assert_eq!(v1.regimes(), v2.regimes());
    assert_eq!(v1.motifs(), v2.motifs());
    assert_eq!(v1.chevauchements(), v2.chevauchements());
    assert_eq!(v1.taux(), v2.taux());
    assert_eq!(v1.retention(), v2.retention());
    assert_eq!(v2.date_effet(), date!(2025 - 10 - 02));
    let dossier = registre.versions_du_dossier("SN", &[]);
    assert_eq!(
        dossier
            .applicable(date!(2025 - 10 - 01))
            .unwrap()
            .identifiant(),
        "sn-2025.1"
    );
    assert_eq!(
        dossier
            .applicable(date!(2025 - 10 - 02))
            .unwrap()
            .identifiant(),
        "sn-2025.2"
    );
    let vue = dossier.applicable(date!(2026 - 03 - 15)).unwrap();
    assert!(
        vue.a_la_date(date!(2026 - 03 - 15))
            .prelevement(TTA)
            .is_some()
    );
    // Le paquet embarqué et le fichier relu ici sont les mêmes.
    assert_eq!(&sn(), v2);
}

/// Valeurs lues dans la loi n° 2025-17 (JO n° 7853, p. PDF 15 et 16).
#[test]
fn valeurs_sourcees_de_la_loi_2025_17() {
    let p = sn();
    let tta = p.prelevement(TTA).unwrap();
    assert_eq!(tta.taux().to_string(), "0.5");
    assert_eq!(tta.plafond(), Some(&xof("2000")));
    assert!(tta.source_plafond().unwrap().contains("399"));
    assert_eq!(tta.devise().code(), "XOF");
    assert_eq!(
        tta.assiette(),
        Assiette::MontantTransfereHorsCommissionEtTaxes
    );
    assert!(tta.source_assiette().contains("398"));
    assert!(tta.source_redevable().contains("396"));
    assert!(tta.source_fait_generateur().contains("399 bis"));
    assert_eq!(tta.exigibilite(), ExigibiliteLegale::DelaiApresEvenement);
    assert!(tta.source_exigibilite().contains("399 bis"));
    assert_eq!((tta.article(), tta.pages()), ("395", &[15u16][..]));
    let exos: Vec<(&str, &str)> = tta
        .exonerations()
        .iter()
        .map(|e| (e.article(), e.alinea()))
        .collect();
    assert_eq!(
        exos,
        [
            ("397", "1"),
            ("397", "2"),
            ("397", "3"),
            ("397", "4"),
            ("397", "5"),
            ("397", "6")
        ]
    );
    let retraits = &tta.exonerations()[1];
    assert_eq!(retraits.code(), "TAXE_TRANSFERT_ARGENT-2");
    assert_eq!(retraits.seuil(), Some(&xof("20000")));
    assert_eq!(retraits.fenetre_heures(), Some(24));
    assert!(
        tta.exonerations()
            .iter()
            .filter(|e| e.alinea() != "2")
            .all(|e| e.seuil().is_none())
    );

    let marchand = p.prelevement(MARCHAND).unwrap();
    assert_eq!(marchand.taux().to_string(), "0.5");
    assert_eq!(marchand.article(), "220 ter");
    assert_eq!(marchand.assiette(), Assiette::PaiementRecu);
    // Le texte ne fixe ni plafond ni exonération.
    assert!(marchand.plafond().is_none());
    assert_eq!(marchand.exonerations(), []);
    assert!(marchand.source_exigibilite().contains("15 premiers jours"));

    for x in p.prelevements() {
        // Paramètres fiscaux à faire valider (thèse comptable ouverte, B5).
        assert!(x.a_valider(), "{}", x.code().as_str());
        assert_eq!(x.document(), Some("jo-sn-7853"));
        assert!(x.pages().iter().all(|pg| (14..=16).contains(pg)));
        // Aucune validité propre : celle de la version (date d'entrée en vigueur non lue).
        assert!(x.debut().is_none() && x.fin().is_none());
        for source in [
            x.source_taux(),
            x.source_assiette(),
            x.source_redevable(),
            x.source_collecte(),
            x.source_fait_generateur(),
            x.source_exigibilite(),
        ] {
            assert!(source.contains("p. PDF"), "{source}");
        }
    }
}

#[test]
fn calcul_de_controle_avec_plafond_par_transaction() {
    let p = sn();
    let tta = p.prelevement(TTA).unwrap();
    assert_eq!(
        tta.montant_avant_arrondi(&xof("100000")).unwrap(),
        xof("500")
    );
    // 0,5 % de 1 000 000 = 5 000, plafonné à 2 000 par transaction (art. 399).
    assert_eq!(
        tta.montant_avant_arrondi(&xof("1000000")).unwrap(),
        xof("2000")
    );
    assert_eq!(
        tta.montant_avant_arrondi(&xof("400000")).unwrap(),
        xof("2000")
    );
    // Avant arrondi : la règle d'arrondi n'est pas dans le texte.
    assert_eq!(
        tta.montant_avant_arrondi(&xof("1001")).unwrap(),
        xof("5.005")
    );
    assert_eq!(tta.montant_avant_arrondi(&xof("0")).unwrap(), xof("0"));
    // Sans plafond (art. 220 ter).
    let marchand = p.prelevement(MARCHAND).unwrap();
    assert_eq!(
        marchand.montant_avant_arrondi(&xof("1000000")).unwrap(),
        xof("5000")
    );
    // Assiette négative ou dans une autre devise : refusée.
    for refus in [
        xof("-1"),
        Montant::parse("100", Devise::parse("EUR").unwrap()).unwrap(),
    ] {
        assert!(matches!(
            tta.montant_avant_arrondi(&refus),
            Err(ErreurFiscale::Montant(_))
        ));
    }
}

proptest! {
    /// Le montant de contrôle est positif, au plus le taux appliqué à l'assiette, au plus le
    /// plafond, et croît avec l'assiette.
    #[test]
    fn le_calcul_de_controle_respecte_taux_et_plafond(a in 0u64..10_000_000_000, b in 0u64..10_000_000_000) {
        let p = sn();
        let tta = p.prelevement(TTA).unwrap();
        let (petit, grand) = (a.min(b), a.max(b));
        let m_petit = tta.montant_avant_arrondi(&xof(&petit.to_string())).unwrap();
        let m_grand = tta.montant_avant_arrondi(&xof(&grand.to_string())).unwrap();
        prop_assert!(m_petit.valeur() >= Decimal::ZERO);
        prop_assert!(m_petit.valeur() <= m_grand.valeur());
        prop_assert!(m_grand.valeur() <= tta.plafond().unwrap().valeur());
        let brut = xof(&grand.to_string()).valeur() * tta.taux().valeur() / xof("100").valeur();
        prop_assert!(m_grand.valeur() <= brut);
    }
}

/// Les prélèvements suivent la version de dossier (contenu complet) et se relisent à l'identique.
#[test]
fn une_version_de_dossier_garde_les_prelevements() {
    let taux = [("normal".to_owned(), "19".to_owned())];
    let d = sn()
        .deriver(
            &socle(),
            &Surcharge {
                numero: 1,
                date_effet: date!(2027 - 01 - 01),
                date_fin: None,
                motif: "Convention",
                taux: &taux,
                regimes_retires: &[],
            },
        )
        .unwrap();
    assert_eq!(d.prelevements(), sn().prelevements());
    let relu =
        ParametresFiscaux::depuis_texte(&socle(), Portee::Dossier(1), &d.vers_texte()).unwrap();
    assert_eq!(relu, d);
}

/// Un autre pays (factice) se charge avec des paramètres différents, sans toucher au socle ni
/// au code : devise à deux décimales, validité propre, valeurs `non_trouve` signalées.
#[test]
fn le_pack_factice_charge_d_autres_prelevements() {
    let p = factice();
    let cdf = |t: &str| Montant::parse(t, Devise::parse("CDF").unwrap()).unwrap();
    let tta = p.prelevement(TTA).unwrap();
    assert_eq!(tta.taux().to_string(), "1.25");
    assert_eq!(tta.plafond(), Some(&cdf("1500.50")));
    assert_eq!(tta.exonerations()[0].seuil(), Some(&cdf("99.99")));
    assert_eq!(tta.exonerations()[0].fenetre_heures(), Some(48));
    assert_eq!(
        tta.montant_avant_arrondi(&cdf("1000")).unwrap(),
        cdf("12.5")
    );
    assert_eq!(
        tta.montant_avant_arrondi(&cdf("1000000")).unwrap(),
        cdf("1500.50")
    );
    let marchand = p.prelevement(MARCHAND).unwrap();
    assert_eq!(marchand.assiette(), Assiette::NonTrouve);
    assert!(marchand.a_valider());
    // Validité propre : la taxe factice finit le 2026-12-31.
    assert!(
        p.a_la_date(date!(2026 - 12 - 31))
            .prelevement(TTA)
            .is_some()
    );
    assert!(
        p.a_la_date(date!(2027 - 01 - 01))
            .prelevement(TTA)
            .is_none()
    );
    assert_eq!(p.a_la_date(date!(2027 - 01 - 01)).prelevements().count(), 1);
}

#[test]
fn refus_des_prelevements_invalides() {
    let remplacer = |de: &str, par: &str| {
        assert!(PRELEVEMENTS.contains(de), "{de}");
        charger(&PRELEVEMENTS.replacen(de, par, 1))
    };
    let cas: Vec<(&str, Result<ParametresFiscaux, ErreurFiscale>)> = vec![
        (
            "code hors socle",
            remplacer(
                "code = \"PRELEVEMENT_PAIEMENT_RECU\"",
                "code = \"TAXE_INCONNUE\"",
            ),
        ),
        (
            "code mal formé",
            remplacer(
                "code = \"PRELEVEMENT_PAIEMENT_RECU\"",
                "code = \"TAXE_220\"",
            ),
        ),
        (
            "code en double",
            remplacer(
                "code = \"PRELEVEMENT_PAIEMENT_RECU\"",
                "code = \"TAXE_TRANSFERT_ARGENT\"",
            ),
        ),
        (
            "source du taux vide",
            remplacer(
                "source_taux = \"art. 220 ter 3), p. PDF 15 (loi 2025-17, art. 4)\"",
                "source_taux = \" \"",
            ),
        ),
        (
            "article vide",
            remplacer("article = \"220 ter\"", "article = \"\""),
        ),
        (
            "page hors du document",
            remplacer(
                "pages = [15]\ndocument = \"jo-sn-7853\"\ntaux",
                "pages = [17]\ndocument = \"jo-sn-7853\"\ntaux",
            ),
        ),
        (
            "document inconnu",
            remplacer("document = \"jo-sn-7853\"", "document = \"autre\""),
        ),
        ("taux nul", remplacer("taux = \"0.5\"", "taux = \"0\"")),
        (
            "taux au-delà de 100",
            remplacer("taux = \"0.5\"", "taux = \"100.5\""),
        ),
        (
            "taux non canonique",
            remplacer("taux = \"0.5\"", "taux = \"0,5\""),
        ),
        (
            "plafond nul",
            remplacer("plafond = \"2000\"", "plafond = \"0\""),
        ),
        (
            "plafond négatif",
            remplacer("plafond = \"2000\"", "plafond = \"-2000\""),
        ),
        (
            "plafond illisible",
            remplacer("plafond = \"2000\"", "plafond = \"2 000\""),
        ),
        (
            "plafond sans source",
            remplacer(
                "source_plafond = \"art. 399 al. 2, p. PDF 15 : la taxe ne peut dépasser 2 000 francs par transaction\"\n",
                "",
            ),
        ),
        (
            "source de plafond sans plafond",
            remplacer("plafond = \"2000\"\n", ""),
        ),
        (
            "devise invalide",
            remplacer("devise = \"XOF\"", "devise = \"F\""),
        ),
        (
            "non trouvé sans validation",
            charger(
                &PRELEVEMENTS
                    .replacen(
                        "assiette = \"paiement_recu\"",
                        "assiette = \"non_trouve\"",
                        1,
                    )
                    .replacen(
                        "a_valider = true\nnote = \"Aucun plafond",
                        "a_valider = false\nnote = \"Aucun plafond",
                        1,
                    ),
            ),
        ),
        (
            "exonération mal préfixée",
            remplacer(
                "code = \"TAXE_TRANSFERT_ARGENT-1\"",
                "code = \"PRELEVEMENT_PAIEMENT_RECU-1\"",
            ),
        ),
        (
            "exonération en double",
            remplacer(
                "code = \"TAXE_TRANSFERT_ARGENT-2\"",
                "code = \"TAXE_TRANSFERT_ARGENT-1\"",
            ),
        ),
        (
            "alinéa d'exonération en double",
            remplacer(
                "alinea = \"2\"\npages = [15]",
                "alinea = \"1\"\npages = [15]",
            ),
        ),
        ("seuil nul", remplacer("seuil = \"20000\"", "seuil = \"0\"")),
        ("fenêtre sans seuil", remplacer("seuil = \"20000\"\n", "")),
        (
            "fenêtre nulle",
            remplacer("fenetre_heures = 24", "fenetre_heures = 0"),
        ),
        (
            "fin avant le début",
            remplacer(
                "a_valider = true\nnote = \"Aucun plafond",
                "a_valider = true\ndebut = \"2026-01-01\"\nfin = \"2025-12-31\"\nnote = \"Aucun plafond",
            ),
        ),
        (
            "champ inconnu",
            remplacer(
                "a_valider = true\nnote",
                "a_valider = true\nlibre = 1\nnote",
            ),
        ),
        (
            "collecteur vide",
            remplacer(
                "collecte_par = \"Opérateur de transfert d'argent, sous sa propre responsabilité\"",
                "collecte_par = \"\"",
            ),
        ),
    ];
    for (nom, resultat) in cas {
        assert!(resultat.is_err(), "{nom} accepté");
    }
    // Les erreurs attendues pour quelques cas typiques.
    assert!(matches!(
        remplacer(
            "code = \"PRELEVEMENT_PAIEMENT_RECU\"",
            "code = \"TAXE_INCONNUE\""
        ),
        Err(ErreurFiscale::HorsSocle(_))
    ));
    assert!(matches!(
        remplacer(
            "pages = [15]\ndocument = \"jo-sn-7853\"\ntaux",
            "pages = [17]\ndocument = \"jo-sn-7853\"\ntaux"
        ),
        Err(ErreurFiscale::Page { page: 17, .. })
    ));
    assert!(matches!(
        remplacer("plafond = \"2000\"", "plafond = \"0\""),
        Err(ErreurFiscale::Montant(_))
    ));
    assert!(matches!(
        remplacer("taux = \"0.5\"", "taux = \"0\""),
        Err(ErreurFiscale::Taux { .. })
    ));
    // Une valeur non trouvée, signalée à valider, est admise.
    assert!(remplacer("assiette = \"paiement_recu\"", "assiette = \"non_trouve\"").is_ok());
    // Sans le fichier, la version se charge (aucun prélèvement), comme 2025.1.
    assert_eq!(charger("").unwrap().prelevements(), []);
}

/// Le socle refuse un code de prélèvement mal formé, en double (y compris avec un régime) ou sans
/// nature.
#[test]
fn le_socle_refuse_les_prelevements_invalides() {
    const SOCLE: &str = include_str!("../data/socle/socle.toml");
    const REGIMES_SOCLE: &str = include_str!("../data/socle/regimes-tva.toml");
    assert!(Socle::depuis_textes(SOCLE, REGIMES_SOCLE).is_ok());
    for (de, par) in [
        (
            "code = \"TAXE_TRANSFERT_ARGENT\"",
            "code = \"PRELEVEMENT_PAIEMENT_RECU\"",
        ),
        ("code = \"TAXE_TRANSFERT_ARGENT\"", "code = \"MARGE\""),
        ("code = \"TAXE_TRANSFERT_ARGENT\"", "code = \"TAXE_395\""),
        ("code = \"TAXE_TRANSFERT_ARGENT\"", "code = \"taxe\""),
        (
            "nature = \"Taxe sur les opérations de transfert d'argent, collectée par l'intermédiaire qui les effectue\"",
            "nature = \" \"",
        ),
    ] {
        assert!(SOCLE.contains(de), "{de}");
        assert!(
            Socle::depuis_textes(&SOCLE.replacen(de, par, 1), REGIMES_SOCLE).is_err(),
            "{par}"
        );
    }
}
