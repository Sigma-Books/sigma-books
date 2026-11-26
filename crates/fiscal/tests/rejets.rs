//! Refus des données invalides du socle, d'une version et du registre.
#![allow(
    clippy::unwrap_used,
    clippy::too_many_lines,
    clippy::items_after_statements
)]

use fiscal::{ErreurFiscale, ParametresFiscaux, Portee, Registre, Socle};

const SOCLE: &str = include_str!("../data/socle/socle.toml");
const REGIMES_SOCLE: &str = include_str!("../data/socle/regimes-tva.toml");
const VERSION: &str = include_str!("../data/pays/sn/2025.1/version.toml");
const REGIMES: &str = include_str!("../data/pays/sn/2025.1/regimes-tva.toml");
const MOTIFS: &str = include_str!("../data/pays/sn/2025.1/motifs-exoneration.toml");

fn socle(socle: &str, regimes: &str) -> Result<Socle, ErreurFiscale> {
    Socle::depuis_textes(socle, regimes)
}

#[test]
fn le_socle_refuse_les_donnees_invalides() {
    assert!(socle(SOCLE, REGIMES_SOCLE).is_ok());
    // Code en double.
    let double =
        REGIMES_SOCLE.replacen("code = \"TAXABLE_REDUIT\"", "code = \"TAXABLE_NORMAL\"", 1);
    assert!(matches!(
        socle(SOCLE, &double),
        Err(ErreurFiscale::CodeEnDouble(_))
    ));
    // Code mal formé.
    let mal = REGIMES_SOCLE.replacen("code = \"MARGE\"", "code = \"é !\"", 1);
    assert!(matches!(socle(SOCLE, &mal), Err(ErreurFiscale::Code(_))));
    // Rôle de taux inconnu.
    let role = REGIMES_SOCLE.replacen(
        "roles_taux = [\"reduit\"]",
        "roles_taux = [\"exotique\"]",
        1,
    );
    assert!(matches!(
        socle(SOCLE, &role),
        Err(ErreurFiscale::RoleInconnu(_))
    ));
    // Exigibilité d'étiquette inconnue.
    let exigibilite = REGIMES_SOCLE.replacen(
        "exigibilites_etiquette = [\"encaissement\"]\n\n[[regime]]\ncode = \"EXO_SANS",
        "exigibilites_etiquette = [\"jamais\"]\n\n[[regime]]\ncode = \"EXO_SANS",
        1,
    );
    assert!(matches!(
        socle(SOCLE, &exigibilite),
        Err(ErreurFiscale::Incoherente { .. })
    ));
    // Régime exonéré avec une ligne de taxe, un taux ou une exigibilité.
    let avec_taxe = REGIMES_SOCLE.replacen(
        "exonere = true\nligne_taxe = false",
        "exonere = true\nligne_taxe = true",
        1,
    );
    assert!(matches!(
        socle(SOCLE, &avec_taxe),
        Err(ErreurFiscale::Incoherente { .. })
    ));
    let avec_taux = REGIMES_SOCLE.replacen(
        "exonere = true\nligne_taxe = false\nroles_taux = []",
        "exonere = true\nligne_taxe = false\nroles_taux = [\"normal\"]",
        1,
    );
    assert!(matches!(
        socle(SOCLE, &avec_taux),
        Err(ErreurFiscale::Incoherente { .. })
    ));
    // Nature vide, source de rétention vide, plancher nul ou démesuré.
    let nature = REGIMES_SOCLE.replacen(
        "nature = \"Régularisation de déduction\"",
        "nature = \" \"",
        1,
    );
    assert!(matches!(
        socle(SOCLE, &nature),
        Err(ErreurFiscale::SansLibelle(_))
    ));
    let sans_source = SOCLE.replacen("source = \"AUDCIF", "source = \" \" # AUDCIF", 1);
    assert!(matches!(
        socle(&sans_source, REGIMES_SOCLE),
        Err(ErreurFiscale::SansSource(_))
    ));
    for plancher in ["0", "101"] {
        let faux = SOCLE.replacen(
            "plancher_annees = 10",
            &format!("plancher_annees = {plancher}"),
            1,
        );
        assert!(matches!(
            socle(&faux, REGIMES_SOCLE),
            Err(ErreurFiscale::PlancherInvalide(_))
        ));
    }
    // Champ inconnu, syntaxe.
    assert!(matches!(
        socle("pas du toml", REGIMES_SOCLE),
        Err(ErreurFiscale::Syntaxe(_))
    ));
}

fn charger(version: &str, regimes: &str, motifs: &str) -> Result<ParametresFiscaux, ErreurFiscale> {
    ParametresFiscaux::depuis_texte(
        &Socle::officiel().unwrap(),
        Portee::Pack,
        &[version, regimes, motifs].join("\n"),
    )
}

#[test]
fn une_version_refuse_un_regime_sans_taux_et_un_chevauchement_sans_cible() {
    // Seul le rôle `reduit` est défini : TAXABLE_NORMAL n'a aucun taux (TauxAbsent).
    let sans_normal = VERSION.replacen(
        "[taux.normal]\nvaleur = \"18\"\nsource = \"art. 369, p. 184\"\n",
        "",
        1,
    );
    assert_ne!(sans_normal, VERSION);
    assert!(matches!(
        charger(&sans_normal, REGIMES, MOTIFS),
        Err(ErreurFiscale::TauxAbsent(_))
    ));
    // Chevauchement vers un motif absent.
    let cible = MOTIFS.replacen(
        "motif = \"EXO_AVEC_DEDUCTION-a\"",
        "motif = \"EXO_AVEC_DEDUCTION-z\"",
        1,
    );
    assert!(matches!(
        charger(VERSION, REGIMES, &cible),
        Err(ErreurFiscale::CibleAbsente(_))
    ));
    // Source de version vide, document absent.
    let source = VERSION.replacen(
        "source = \"CGI du Sénégal",
        "source = \" \" # CGI du Sénégal",
        1,
    );
    assert!(matches!(
        charger(&source, REGIMES, MOTIFS),
        Err(ErreurFiscale::SansSource(_))
    ));
}

#[test]
fn le_registre_refuse_une_version_en_double() {
    let socle = Socle::officiel().unwrap();
    let a = charger(VERSION, REGIMES, MOTIFS).unwrap();
    // Même identifiant (sn-2025.1) à une autre date d'effet.
    let b = charger(
        &VERSION.replacen("2025-01-01", "2026-01-01", 1),
        REGIMES,
        MOTIFS,
    )
    .unwrap();
    assert_eq!(a.identifiant(), b.identifiant());
    assert_eq!(
        Registre::depuis_versions(socle, vec![a, b]).unwrap_err(),
        ErreurFiscale::VersionEnDouble("sn-2025.1".to_owned())
    );
}
