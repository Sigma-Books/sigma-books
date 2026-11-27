//! Versions datées : sélection par date, versions d'un dossier, validité, continuité des motifs,
//! immuabilité des versions publiées, pack factice d'un autre pays.
#![allow(
    clippy::unwrap_used,
    clippy::too_many_lines,
    clippy::items_after_statements
)]

use std::fmt::Write as _;
use std::fs;
use std::path::Path;

use compta_core::{
    CodeMotif, CodeRegime, EtiquetteFiscale, NatureEtiquette, ReferentielFiscal, SensTva, TauxTva,
};
use fiscal::{ErreurFiscale, ParametresFiscaux, Portee, Registre, Socle};
use proptest::prelude::*;
use sha2::{Digest, Sha256};
use time::macros::date;
use time::{Date, Duration};

fn fixture(version: &str) -> String {
    let lire = |f: &str| {
        fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/zz")
                .join(version)
                .join(f),
        )
        .unwrap()
    };
    [
        lire("version.toml"),
        lire("regimes-tva.toml"),
        lire("motifs-exoneration.toml"),
    ]
    .join("\n")
}

fn charger(portee: Portee, texte: &str) -> ParametresFiscaux {
    ParametresFiscaux::depuis_texte(&Socle::officiel().unwrap(), portee, texte).unwrap()
}

fn registre_factice() -> Registre {
    let socle = Socle::officiel().unwrap();
    Registre::depuis_versions(
        socle,
        vec![
            charger(Portee::Pack, &fixture("2024.1")),
            charger(Portee::Pack, &fixture("2026.1")),
        ],
    )
    .unwrap()
}

#[test]
fn un_autre_pays_se_charge_sans_toucher_au_socle() {
    let registre = registre_factice();
    assert_eq!(registre.pays().collect::<Vec<_>>(), ["ZZ"]);
    let v = &registre.versions_du_pays("ZZ")[1];
    assert_eq!(v.identifiant(), "zz-2026.1");
    // Taux et régimes différents de ceux du Sénégal, mêmes codes génériques.
    assert_eq!(v.taux_du_role("normal").unwrap().to_string(), "16");
    assert!(
        v.regime("TAXABLE_REDUIT").is_none(),
        "régime non offert dans ce pays"
    );
    assert_eq!(v.retention().annees_min, 12);
    let vue = v.a_la_date(date!(2026 - 08 - 01));
    let faire = |taux: &str| {
        EtiquetteFiscale::nouvelle(
            NatureEtiquette::Base,
            SensTva::Collectee,
            CodeRegime::parse("TAXABLE_NORMAL").unwrap(),
            Some(TauxTva::parse(taux).unwrap()),
            Some(compta_core::Exigibilite::Debit),
            None,
            &vue,
        )
    };
    assert!(faire("16").is_ok());
    assert!(
        faire("18").is_err(),
        "le taux du Sénégal n'est pas celui de ce pays"
    );
    // Un régime que le pays n'offre pas est inconnu.
    assert!(
        EtiquetteFiscale::nouvelle(
            NatureEtiquette::Base,
            SensTva::Collectee,
            CodeRegime::parse("TAXABLE_REDUIT").unwrap(),
            Some(TauxTva::parse("5").unwrap()),
            Some(compta_core::Exigibilite::Debit),
            None,
            &vue,
        )
        .is_err()
    );
    // Le même socle sert le Sénégal : ses codes sont ceux du pays factice.
    let officiel = Registre::officiel().unwrap();
    assert_eq!(officiel.socle(), registre.socle());
}

#[test]
fn selection_par_date() {
    let registre = registre_factice();
    let dossier = registre.versions_du_dossier("ZZ", &[]);
    let id = |d: Date| dossier.applicable(d).map(|v| v.identifiant().to_owned());
    assert_eq!(id(date!(2023 - 12 - 31)), None, "avant la première version");
    assert_eq!(id(date!(2024 - 01 - 01)).as_deref(), Some("zz-2024.1"));
    assert_eq!(id(date!(2026 - 06 - 30)).as_deref(), Some("zz-2024.1"));
    assert_eq!(id(date!(2026 - 07 - 01)).as_deref(), Some("zz-2026.1"));
    assert_eq!(id(date!(2040 - 01 - 01)).as_deref(), Some("zz-2026.1"));
    // Un pays sans paquet n'a aucune version.
    assert!(
        registre
            .versions_du_dossier("QQ", &[])
            .applicable(date!(2026 - 01 - 01))
            .is_none()
    );
    // Par identifiant enregistré.
    assert_eq!(
        dossier.par_identifiant("zz-2024.1").unwrap().version(),
        "2024.1"
    );
    assert!(dossier.par_identifiant("zz-1999.1").is_none());
}

#[test]
fn date_de_fin_sans_successeur() {
    let socle = Socle::officiel().unwrap();
    let texte = fixture("2024.1").replacen(
        "date_effet = \"2024-01-01\"",
        "date_effet = \"2024-01-01\"\ndate_fin = \"2024-12-31\"",
        1,
    );
    let registre = Registre::depuis_versions(socle, vec![charger(Portee::Pack, &texte)]).unwrap();
    let dossier = registre.versions_du_dossier("ZZ", &[]);
    assert!(dossier.applicable(date!(2024 - 12 - 31)).is_some());
    assert!(
        dossier.applicable(date!(2025 - 01 - 01)).is_none(),
        "abrogée sans successeur"
    );
}

#[test]
fn version_du_dossier_prioritaire_et_datee() {
    let registre = registre_factice();
    // Version du dossier : taux 20, du 2025-03-01 au 2026-12-31.
    let propre = fixture("2024.1")
        .replacen(
            "date_effet = \"2024-01-01\"",
            "date_effet = \"2025-03-01\"\ndate_fin = \"2026-12-31\"",
            1,
        )
        .replacen("valeur = \"15\"", "valeur = \"20\"", 1);
    let propres = vec![charger(Portee::Dossier(1), &propre)];
    assert_eq!(propres[0].identifiant(), "zz-2024.1+d1");
    let dossier = registre.versions_du_dossier("ZZ", &propres);
    let id = |d: Date| dossier.applicable(d).unwrap().identifiant().to_owned();
    assert_eq!(id(date!(2025 - 02 - 28)), "zz-2024.1");
    assert_eq!(id(date!(2025 - 03 - 01)), "zz-2024.1+d1");
    // Elle l'emporte sur la version du pays qui prend effet en 2026.
    assert_eq!(id(date!(2026 - 07 - 01)), "zz-2024.1+d1");
    // Sa date de fin rend la main au paquet.
    assert_eq!(id(date!(2027 - 01 - 01)), "zz-2026.1");
    assert_eq!(
        dossier
            .par_identifiant("zz-2024.1+d1")
            .unwrap()
            .taux_du_role("normal")
            .unwrap()
            .to_string(),
        "20"
    );
    // Une version d'un autre pays est ignorée.
    let autre = fixture("2024.1").replace("pays = \"ZZ\"", "pays = \"YY\"");
    let etrangere = vec![charger(Portee::Dossier(1), &autre)];
    let seul = registre.versions_du_dossier("ZZ", &etrangere);
    assert_eq!(
        seul.applicable(date!(2025 - 05 - 05))
            .unwrap()
            .identifiant(),
        "zz-2024.1"
    );
}

#[test]
fn deux_versions_le_meme_jour_sont_refusees() {
    let socle = Socle::officiel().unwrap();
    let bis = fixture("2026.1").replacen("2026-07-01", "2024-01-01", 1);
    let erreur = Registre::depuis_versions(
        socle,
        vec![
            charger(Portee::Pack, &fixture("2024.1")),
            charger(Portee::Pack, &bis),
        ],
    )
    .unwrap_err();
    assert_eq!(
        erreur,
        ErreurFiscale::DateEffetEnDouble {
            pays: "ZZ".to_owned()
        }
    );
}

#[test]
fn un_motif_ne_disparait_pas_et_ne_change_pas_de_sens() {
    let socle = Socle::officiel().unwrap();
    let deux = |suivante: String| {
        Registre::depuis_versions(
            Socle::officiel().unwrap(),
            vec![
                charger(Portee::Pack, &fixture("2024.1")),
                charger(Portee::Pack, &suivante),
            ],
        )
    };
    // Rien ne change : continuité respectée.
    assert!(deux(fixture("2026.1")).is_ok());
    // Le motif x2 est retiré de la version suivante.
    let retire = {
        let t = fixture("2026.1");
        let i = t
            .find("[[motif]]\ncode = \"EXO_SANS_DEDUCTION-x2\"")
            .unwrap();
        let j = t.find("[[chevauchement]]").unwrap();
        // Le chevauchement cible x2 : on le retire aussi pour que seule la continuité échoue.
        format!("{}{}", &t[..i], &t[j..]).replace(
            "motif = \"EXO_SANS_DEDUCTION-x2\"\nnote",
            "motif = \"EXO_SANS_DEDUCTION-x1\"\nnote",
        )
    };
    assert_eq!(
        deux(retire).unwrap_err(),
        ErreurFiscale::MotifRetire("EXO_SANS_DEDUCTION-x2".to_owned())
    );
    // Même code, mais autre article, autre alinéa ou autre droit à déduction : réaffecté.
    for (avant, apres) in [
        (
            "article = \"F21\"\nalinea = \"2\"",
            "article = \"F22\"\nalinea = \"2\"",
        ),
        (
            "alinea = \"2\"\npages = [20]",
            "alinea = \"9\"\npages = [20]",
        ),
        (
            "libelle = \"Motif factice 2\"\ndeduction = \"non\"",
            "libelle = \"Motif factice 2\"\ndeduction = \"oui\"",
        ),
    ] {
        let t = fixture("2026.1");
        assert!(t.contains(avant), "{avant}");
        let changee = t.replacen(avant, apres, 1);
        assert_eq!(
            deux(changee).unwrap_err(),
            ErreurFiscale::MotifRetire("EXO_SANS_DEDUCTION-x2".to_owned()),
            "{apres}"
        );
    }
    drop(socle);
}

#[test]
fn le_registre_refuse_une_version_de_dossier() {
    let propre = charger(Portee::Dossier(1), &fixture("2024.1"));
    assert!(matches!(
        Registre::depuis_versions(Socle::officiel().unwrap(), vec![propre]),
        Err(ErreurFiscale::PorteeInattendue(_))
    ));
}

#[test]
fn une_version_de_dossier_exige_une_base_et_la_continuite() {
    let registre = registre_factice();
    let propre = |n: u32, texte: &str| charger(Portee::Dossier(n), texte);
    let t = fixture("2024.1").replacen("2024-01-01", "2025-03-01", 1);
    let premiere = propre(1, &t);
    assert!(registre.verifier_version_de_dossier(&[], &premiere).is_ok());
    // Un paquet de pays est refusé comme version de dossier.
    let pack = charger(Portee::Pack, &fixture("2024.1"));
    assert!(matches!(
        registre.verifier_version_de_dossier(&[], &pack),
        Err(ErreurFiscale::PorteeInattendue(_))
    ));
    // Pays sans paquet : pas de base.
    let ailleurs = propre(1, &t.replace("pays = \"ZZ\"", "pays = \"YY\""));
    assert_eq!(
        registre.verifier_version_de_dossier(&[], &ailleurs),
        Err(ErreurFiscale::PaysSansPaquet("YY".to_owned()))
    );
    // Version de base inconnue du paquet.
    let inconnue = propre(
        1,
        &t.replace("version = \"2024.1\"", "version = \"2030.9\""),
    );
    assert!(matches!(
        registre.verifier_version_de_dossier(&[], &inconnue),
        Err(ErreurFiscale::Version(_))
    ));
    // Même date d'effet qu'une version de dossier existante.
    let bis = propre(2, &t);
    assert!(matches!(
        registre.verifier_version_de_dossier(std::slice::from_ref(&premiere), &bis),
        Err(ErreurFiscale::DateEffetEnDouble { .. })
    ));
    // Motif retiré par rapport à sa base, puis par rapport à la version de dossier précédente.
    let sans_x2 = {
        let i = t
            .find("[[motif]]\ncode = \"EXO_SANS_DEDUCTION-x2\"")
            .unwrap();
        let j = t.find("[[chevauchement]]").unwrap();
        format!("{}{}", &t[..i], &t[j..]).replace(
            "motif = \"EXO_SANS_DEDUCTION-x2\"\nnote",
            "motif = \"EXO_SANS_DEDUCTION-x1\"\nnote",
        )
    };
    let amputee = propre(2, &sans_x2.replace("2025-03-01", "2025-06-01"));
    assert_eq!(
        registre.verifier_version_de_dossier(std::slice::from_ref(&premiere), &amputee),
        Err(ErreurFiscale::MotifRetire(
            "EXO_SANS_DEDUCTION-x2".to_owned()
        ))
    );
}

#[test]
fn validite_d_un_motif_selon_la_date() {
    let p = charger(Portee::Pack, &fixture("2024.1"));
    let regime = CodeRegime::parse("EXO_SANS_DEDUCTION").unwrap();
    let x1 = CodeMotif::parse("EXO_SANS_DEDUCTION-x1").unwrap();
    assert!(p.a_la_date(date!(2025 - 12 - 31)).motif_admis(&regime, &x1));
    assert!(!p.a_la_date(date!(2026 - 01 - 01)).motif_admis(&regime, &x1));
    assert_eq!(p.a_la_date(date!(2026 - 01 - 01)).motifs().count(), 1);
}

/// Les versions publiées sont immuables : l'empreinte SHA-256 de chaque fichier est figée dans
/// `data/pays/<xx>/EMPREINTES`. Modifier un fichier publié échoue ici ; une correction de la loi
/// est une NOUVELLE version.
#[test]
fn les_versions_publiees_sont_immuables() {
    let racine = Path::new(env!("CARGO_MANIFEST_DIR")).join("data/pays");
    let mut pays: Vec<_> = fs::read_dir(&racine)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    pays.sort();
    let nombre = pays.len();
    assert!(nombre > 0);
    for dossier in pays {
        let mut attendues: Vec<String> = Vec::new();
        let mut versions: Vec<_> = fs::read_dir(&dossier)
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| p.is_dir())
            .collect();
        versions.sort();
        for v in versions {
            let mut fichiers: Vec<_> = fs::read_dir(&v)
                .unwrap()
                .map(|e| e.unwrap().path())
                .collect();
            fichiers.sort();
            for f in fichiers {
                let octets = fs::read(&f).unwrap();
                let somme = Sha256::digest(&octets)
                    .iter()
                    .fold(String::new(), |mut s, o| {
                        let _ = write!(s, "{o:02x}");
                        s
                    });
                let nom = format!(
                    "{}/{}",
                    v.file_name().unwrap().to_string_lossy(),
                    f.file_name().unwrap().to_string_lossy()
                );
                attendues.push(format!("{somme}  {nom}"));
            }
        }
        let figees = fs::read_to_string(dossier.join("EMPREINTES")).unwrap();
        let figees: Vec<&str> = figees.lines().filter(|l| !l.trim().is_empty()).collect();
        assert_eq!(
            figees,
            attendues.iter().map(String::as_str).collect::<Vec<_>>(),
            "version publiée modifiée ou ajoutée sans empreinte ({})",
            dossier.display()
        );
    }
}

proptest! {
    /// La version choisie est celle de plus grande date d'effet au plus égale à la date, et elle
    /// couvre la date ; sinon aucune version ne la couvre.
    #[test]
    fn la_selection_choisit_la_derniere_version_en_vigueur(
        decalages in proptest::collection::btree_set(0i64..4000, 1..6),
        jour in -100i64..4500,
        fins in proptest::collection::vec(proptest::option::of(1i64..800), 6),
    ) {
        let socle = Socle::officiel().unwrap();
        let origine = date!(2020 - 01 - 01);
        let mut versions = Vec::new();
        for (i, d) in decalages.iter().enumerate() {
            let effet = origine + Duration::days(*d);
            let mut texte = fixture("2024.1").replacen("2024-01-01", &effet.to_string(), 1);
            texte = texte.replacen("version = \"2024.1\"", &format!("version = \"2024.{}\"", i + 1), 1);
            if let Some(f) = fins[i] {
                let fin = effet + Duration::days(f);
                texte = texte.replacen(
                    &format!("date_effet = \"{effet}\""),
                    &format!("date_effet = \"{effet}\"\ndate_fin = \"{fin}\""),
                    1,
                );
            }
            versions.push(ParametresFiscaux::depuis_texte(&socle, Portee::Pack, &texte).unwrap());
        }
        let registre = Registre::depuis_versions(socle, versions.clone()).unwrap();
        let cible = origine + Duration::days(jour);
        let choisie = registre.versions_du_dossier("ZZ", &[]).applicable(cible);
        let derniere = versions.iter().filter(|v| v.date_effet() <= cible).max_by_key(|v| v.date_effet());
        match (choisie, derniere) {
            (Some(c), Some(d)) => {
                prop_assert_eq!(c.identifiant(), d.identifiant());
                prop_assert!(c.couvre(cible));
            }
            (None, Some(d)) => prop_assert!(!d.couvre(cible)),
            (None, None) => {}
            (Some(_), None) => prop_assert!(false, "version choisie avant toute date d'effet"),
        }
    }
}

/// Une version de dossier ne reprend pas la `date_fin` du paquet : elle survit à l'abrogation de
/// la version qu'elle dérive, et sans `date_fin` elle masque les paquets suivants (ADR 0030, la
/// priorité d'une version de dossier ne tient aucun compte de l'abrogation du paquet).
#[test]
fn une_version_de_dossier_survit_a_l_abrogation_du_paquet() {
    let socle = Socle::officiel().unwrap();
    // Paquet : v2024.1 abrogée le 2025-12-31 sans successeur immédiat, v2026.1 dès le 2026-07-01.
    let abrogee = fixture("2024.1").replacen(
        "date_effet = \"2024-01-01\"",
        "date_effet = \"2024-01-01\"\ndate_fin = \"2025-12-31\"",
        1,
    );
    let v1 = charger(Portee::Pack, &abrogee);
    assert_eq!(v1.date_fin(), Some(date!(2025 - 12 - 31)));
    let registre = Registre::depuis_versions(
        Socle::officiel().unwrap(),
        vec![v1.clone(), charger(Portee::Pack, &fixture("2026.1"))],
    )
    .unwrap();
    // Dérivée de v2024.1 le 2025-06-01, sans date de fin : le paquet (abrogé) n'est pas hérité.
    let taux = [("normal".to_owned(), "20".to_owned())];
    let d = v1
        .deriver(
            &socle,
            &fiscal::Surcharge {
                numero: 1,
                date_effet: date!(2025 - 06 - 01),
                date_fin: None,
                motif: "Convention",
                taux: &taux,
                regimes_retires: &[],
            },
        )
        .unwrap();
    assert_eq!(d.date_fin(), None);
    let propres = vec![d];
    let versions = registre.versions_du_dossier("ZZ", &propres);
    let id = |j: Date| versions.applicable(j).map(|v| v.identifiant().to_owned());
    // Le paquet est abrogé le 2025-12-31 (aucune version) : la version du dossier s'applique encore.
    assert_eq!(id(date!(2026 - 03 - 01)).as_deref(), Some("zz-2024.1+d1"));
    // Et, sans date de fin, elle masque le paquet suivant (2026.1) : à poser par l'administrateur.
    assert_eq!(id(date!(2026 - 08 - 01)).as_deref(), Some("zz-2024.1+d1"));
    // Avec une date de fin, la main revient au paquet.
    let avec_fin = v1
        .deriver(
            &socle,
            &fiscal::Surcharge {
                numero: 1,
                date_effet: date!(2025 - 06 - 01),
                date_fin: Some(date!(2026 - 06 - 30)),
                motif: "Convention",
                taux: &taux,
                regimes_retires: &[],
            },
        )
        .unwrap();
    let propres = vec![avec_fin];
    let versions = registre.versions_du_dossier("ZZ", &propres);
    assert_eq!(
        versions
            .applicable(date!(2026 - 08 - 01))
            .unwrap()
            .identifiant(),
        "zz-2026.1"
    );
}

proptest! {
    /// Avec des versions de pays et de dossier : la version choisie est celle du dossier de plus
    /// grande date d'effet <= date qui couvre la date, sinon celle du pays, sinon aucune.
    #[test]
    fn la_selection_avec_des_versions_de_dossier(
        pays in proptest::collection::btree_set(0i64..2000, 1..4),
        propres in proptest::collection::btree_set(0i64..2000, 0..4),
        fins in proptest::collection::vec(proptest::option::of(1i64..500), 8),
        jour in -50i64..2600,
    ) {
        let socle = Socle::officiel().unwrap();
        let origine = date!(2020 - 01 - 01);
        let texte = |effet: Date, version: &str, fin: Option<i64>| {
            let mut t = fixture("2024.1").replacen("2024-01-01", &effet.to_string(), 1)
                .replacen("version = \"2024.1\"", &format!("version = \"{version}\""), 1);
            if let Some(f) = fin {
                t = t.replacen(
                    &format!("date_effet = \"{effet}\""),
                    &format!("date_effet = \"{effet}\"\ndate_fin = \"{}\"", effet + Duration::days(f)),
                    1,
                );
            }
            t
        };
        let mut vp = Vec::new();
        for (i, d) in pays.iter().enumerate() {
            let effet = origine + Duration::days(*d);
            vp.push(ParametresFiscaux::depuis_texte(&socle, Portee::Pack, &texte(effet, &format!("2024.{}", i + 1), fins[i])).unwrap());
        }
        let registre = Registre::depuis_versions(Socle::officiel().unwrap(), vp.clone()).unwrap();
        let mut dossier = Vec::new();
        for (i, d) in propres.iter().enumerate() {
            let effet = origine + Duration::days(*d);
            let n = u32::try_from(i).unwrap() + 1;
            dossier.push(ParametresFiscaux::depuis_texte(&socle, Portee::Dossier(n), &texte(effet, "2024.1", fins[4 + i])).unwrap());
        }
        let cible = origine + Duration::days(jour);
        let versions = registre.versions_du_dossier("ZZ", &dossier);
        let choisie = versions.applicable(cible).map(|v| v.identifiant().to_owned());
        let en_vigueur = |liste: &[ParametresFiscaux]| {
            liste.iter().filter(|v| v.date_effet() <= cible).max_by_key(|v| v.date_effet())
                .filter(|v| v.couvre(cible)).map(|v| v.identifiant().to_owned())
        };
        let attendue = en_vigueur(&dossier).or_else(|| en_vigueur(&vp));
        prop_assert_eq!(choisie, attendue);
    }
}
