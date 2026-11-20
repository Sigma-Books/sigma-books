//! Tests du référentiel : validation des données embarquées, contrôles ponctuels contre la liste
//! officielle (AUDCIF + SYSCOHADA, JO OHADA 15/02/2017), rejet des données invalides.

use super::*;
use compta_core::{Journal, TypeJournal};
use proptest::prelude::*;

fn n(numero: &str) -> NumeroCompte {
    NumeroCompte::parse(numero).unwrap()
}

fn officiel() -> Referentiel {
    Referentiel::officiel().expect("référentiel embarqué valide")
}

#[test]
fn le_referentiel_embarque_est_valide() {
    let r = officiel();
    assert_eq!(r.len(), 1401);
    assert_eq!(r.classes().count(), 9);
    let par_longueur = |l: usize| {
        r.comptes()
            .filter(|c| c.numero().as_str().len() == l)
            .count()
    };
    assert_eq!(par_longueur(2), 85);
    assert_eq!(par_longueur(3), 430);
    assert_eq!(par_longueur(4), 886);
}

#[test]
fn chaque_compte_a_son_parent_sauf_les_comptes_a_deux_chiffres() {
    let r = officiel();
    for c in r.comptes() {
        match c.numero().parent() {
            Some(parent) => assert!(
                r.compte(&parent).is_some(),
                "parent absent : {}",
                c.numero()
            ),
            None => assert_eq!(c.numero().as_str().len(), 2),
        }
        assert!(r.classe(c.classe()).is_some());
    }
}

#[test]
fn chaque_compte_cite_une_page_de_la_source() {
    let r = officiel();
    assert!(r.document().contains("15/02/2017"));
    for c in r.comptes() {
        assert!((PAGE_MIN..=PAGE_MAX).contains(&c.page()), "{}", c.numero());
    }
    // Les pages sont croissantes avec les numéros : la liste est imprimée dans l'ordre.
    let pages: Vec<u16> = r.comptes().map(CompteOfficiel::page).collect();
    assert!(pages.windows(2).all(|p| p[0] <= p[1]));
}

#[test]
fn controles_ponctuels_contre_le_texte_officiel() {
    let r = officiel();
    let cas = [
        ("10", "CAPITAL", 219),
        ("1013", "Capital souscrit, appelé, versé, non amorti", 219),
        ("130", "RÉSULTAT EN INSTANCE D'AFFECTATION", 221),
        ("1661", "sur emprunts obligataires", 223),
        (
            "2183",
            "Fichiers clients, notices, titres de journaux et magazines",
            226,
        ),
        ("4011", "Fournisseurs", 240),
        ("4431", "T.V.A. facturée sur ventes", 243),
        ("4449", "État, crédit de T.V.A. à reporter", 244),
        ("5211", "Banques en monnaie nationale", 250),
        ("552", "MONNAIE ÉLECTRONIQUE - TÉLÉPHONE PORTABLE", 251),
        ("6583", "Mécénat", 258),
        ("7015", "sur internet", 262),
        ("895", "IMPÔT MINIMUM FORFAITAIRE (I.M.F.)", 269),
        ("99", "COMPTES DE LIAISONS INTERNES", 272),
    ];
    for (numero, intitule, page) in cas {
        let c = r
            .compte(&n(numero))
            .unwrap_or_else(|| panic!("{numero} absent"));
        assert_eq!(c.intitule().as_str(), intitule, "{numero}");
        assert_eq!(c.page(), page, "{numero}");
    }
}

#[test]
fn comptes_absents_de_la_liste_officielle() {
    // Présents dans d'autres plans (Tryton, Odoo) mais pas dans le JO OHADA du 15/02/2017.
    let r = officiel();
    for numero in ["611", "687", "1666", "2715", "4126", "4442", "6235"] {
        assert!(
            r.compte(&n(numero)).is_none(),
            "{numero} ne doit pas exister"
        );
    }
}

#[test]
fn numeros_restitues_portent_une_note() {
    let r = officiel();
    for numero in ["4125", "4794", "908"] {
        let c = r.compte(&n(numero)).unwrap();
        assert!(c.note().is_some(), "{numero} doit expliquer sa restitution");
    }
    assert_eq!(r.comptes().filter(|c| c.note().is_some()).count(), 3);
}

#[test]
fn nature_coherente_avec_la_classe() {
    let r = officiel();
    for c in r.comptes() {
        match c.classe() {
            1..=5 => assert_eq!(c.nature(), Nature::Bilan, "{}", c.numero()),
            6 => assert_eq!(c.nature(), Nature::Charge, "{}", c.numero()),
            7 => assert_eq!(c.nature(), Nature::Produit, "{}", c.numero()),
            8 => assert!(matches!(c.nature(), Nature::Charge | Nature::Produit)),
            _ => assert!(matches!(c.nature(), Nature::HorsBilan | Nature::Analytique)),
        }
    }
    // Un compte hérite la nature de son parent.
    for c in r.comptes() {
        if let Some(parent) = c.numero().parent() {
            assert_eq!(
                r.compte(&parent).map(CompteOfficiel::nature),
                Some(c.nature())
            );
        }
    }
}

#[test]
fn sens_normal_de_quelques_comptes() {
    let r = officiel();
    let sens = |numero: &str| r.compte(&n(numero)).unwrap().sens();
    assert_eq!(sens("101"), SensNormal::Crediteur);
    assert_eq!(sens("129"), SensNormal::Debiteur);
    assert_eq!(sens("12"), SensNormal::Variable);
    assert_eq!(sens("2811"), SensNormal::Crediteur);
    assert_eq!(sens("4011"), SensNormal::Crediteur);
    assert_eq!(sens("4091"), SensNormal::Debiteur);
    assert_eq!(sens("4111"), SensNormal::Debiteur);
    assert_eq!(sens("4452"), SensNormal::Debiteur);
    assert_eq!(sens("4431"), SensNormal::Crediteur);
    assert_eq!(sens("5211"), SensNormal::Debiteur);
    assert_eq!(sens("6011"), SensNormal::Debiteur);
    assert_eq!(sens("6019"), SensNormal::Crediteur);
    assert_eq!(sens("7011"), SensNormal::Crediteur);
    assert_eq!(sens("7019"), SensNormal::Debiteur);
}

/// Corrections tirées des fiches de fonctionnement des comptes (AUDCIF, Titre VII, ch. 3).
#[test]
fn sens_normal_selon_les_fiches_de_fonctionnement() {
    let r = officiel();
    let sens = |numero: &str| r.compte(&n(numero)).unwrap().sens();
    for numero in ["181", "1811", "182", "183", "905", "9061", "908", "9088"] {
        assert_eq!(sens(numero), SensNormal::Crediteur, "{numero}");
    }
    for numero in ["1962", "581", "582", "901", "9011", "904"] {
        assert_eq!(sens(numero), SensNormal::Debiteur, "{numero}");
    }
    for numero in [
        "104", "1041", "12", "13", "18", "184", "58", "585", "603", "73", "90",
    ] {
        assert_eq!(sens(numero), SensNormal::Variable, "{numero}");
    }
    // Regroupements : le sens des subdivisions 1 à 8 (AUDCIF p. PDF 216).
    for (numero, attendu) in [
        ("40", SensNormal::Crediteur),
        ("41", SensNormal::Debiteur),
        ("444", SensNormal::Crediteur),
        ("601", SensNormal::Debiteur),
        ("701", SensNormal::Crediteur),
    ] {
        assert_eq!(sens(numero), attendu, "{numero}");
    }
}

/// Un compte de regroupement prend le sens commun de ses subdivisions terminées par 1 à 8, ou
/// est variable si elles divergent ; exceptions du texte : 12, 13, 18, 58, 603, 73 variables.
#[test]
fn sens_des_regroupements_coherent() {
    let r = officiel();
    let numeros: Vec<String> = r
        .comptes()
        .map(|c| c.numero().as_str().to_owned())
        .collect();
    let parent = |numero: &str| {
        (1..numero.len())
            .rev()
            .map(|l| &numero[..l])
            .find(|p| numeros.iter().any(|x| x == p))
            .map(str::to_owned)
    };
    for p in &numeros {
        let enfants: Vec<&String> = numeros
            .iter()
            .filter(|x| parent(x).as_deref() == Some(p.as_str()))
            .collect();
        if enfants.is_empty() || ["12", "13", "18", "58", "603", "73"].contains(&p.as_str()) {
            continue;
        }
        let mut retenus: Vec<SensNormal> = enfants
            .iter()
            .filter(|e| !e.ends_with('9') || p.starts_with('2'))
            .map(|e| r.compte(&n(e)).unwrap().sens())
            .collect();
        if retenus.is_empty() {
            retenus = enfants
                .iter()
                .map(|e| r.compte(&n(e)).unwrap().sens())
                .collect();
        }
        let attendu = if retenus.iter().all(|s| *s == retenus[0]) {
            retenus[0]
        } else {
            SensNormal::Variable
        };
        assert_eq!(r.compte(&n(p)).unwrap().sens(), attendu, "regroupement {p}");
    }
}

#[test]
fn comptes_collectifs_de_tiers() {
    let r = officiel();
    let plan = r.plan_comptes().unwrap();
    for numero in ["4011", "4111", "4211", "4041", "4131", "4621", "488"] {
        assert!(plan.compte(&n(numero)).unwrap().est_collectif(), "{numero}");
    }
    for numero in ["6011", "5211", "4431", "101"] {
        assert!(
            !plan.compte(&n(numero)).unwrap().est_collectif(),
            "{numero}"
        );
    }
}

#[test]
fn le_plan_du_dossier_reprend_tout_le_referentiel() {
    let r = officiel();
    let plan = r.plan_comptes().unwrap();
    assert_eq!(plan.comptes().count(), r.len());
    // Les comptes de mouvement sont exactement les comptes sans sous-compte.
    let mouvement = plan
        .comptes()
        .filter(|c| !plan.a_des_enfants(c.numero()))
        .count();
    assert!(mouvement > 1000);
    assert!(plan.compte_de_mouvement_actif(&n("4011")).is_ok());
    assert!(plan.compte_de_mouvement_actif(&n("401")).is_err());
}

/// Comptes admis en journal de trésorerie, et leur fiche de fonctionnement (AUDCIF, Titre VII,
/// ch. 3) : 52 Banques, 53 Établissements financiers, 55 Monnaie électronique, 57 Caisse.
const ADMIS: [(&str, u16); 4] = [("52", 417), ("53", 419), ("55", 424), ("57", 428)];
/// Comptes de la classe 5 exclus (motifs dans data/comptes.toml) ; 581 et 582 en attente du
/// mainteneur.
const EXCLUS: [&str; 9] = ["50", "51", "54", "56", "581", "582", "585", "588", "59"];
/// Sous-comptes d'intérêts courus des comptes admis, exclus explicitement (ni encaisse ni avoir
/// disponible), avec la page de la fiche du compte parent.
const INTERETS_COURUS: [(&str, u16); 2] = [("526", 416), ("536", 418)];

/// Premier compte de mouvement actif du plan sous `racine` (ou `racine` elle-même).
fn compte_de_mouvement_sous(plan: &PlanComptes, racine: &str) -> NumeroCompte {
    let racine = n(racine);
    plan.comptes()
        .map(|c| c.numero().clone())
        .find(|numero| {
            racine.est_ascendant_ou_egal(numero) && plan.compte_de_mouvement_actif(numero).is_ok()
        })
        .unwrap()
}

#[test]
fn comptes_admis_en_journal_de_tresorerie_lus_des_donnees() {
    let r = officiel();
    // Les marques des données sont exactement celles instruites, avec leur page.
    let marques: Vec<(String, bool, u16)> = r
        .comptes()
        .filter_map(|c| {
            c.marque_journal_tresorerie()
                .map(|(admis, page)| (c.numero().to_string(), admis, page))
        })
        .collect();
    let mut attendu: Vec<(String, bool, u16)> = ADMIS
        .iter()
        .map(|(c, p)| ((*c).to_owned(), true, *p))
        .chain(
            INTERETS_COURUS
                .iter()
                .map(|(c, p)| ((*c).to_owned(), false, *p)),
        )
        .collect();
    attendu.sort();
    assert_eq!(marques, attendu);
    // Les exclusions explicites portent toutes sur des intérêts courus ; tout compte d'intérêts
    // courus sous un compte admis, repéré par son intitulé, est exclu.
    let interets_courus = |c: &CompteOfficiel| {
        c.intitule()
            .as_str()
            .to_uppercase()
            .contains("INTÉRÊTS COURUS")
    };
    for (exclu, _) in INTERETS_COURUS {
        assert!(interets_courus(r.compte(&n(exclu)).unwrap()), "{exclu}");
    }
    for c in r.comptes().filter(|c| {
        interets_courus(c)
            && ADMIS
                .iter()
                .any(|(admis, _)| n(admis).est_ascendant_ou_egal(c.numero()))
    }) {
        assert!(
            !r.admis_en_journal_de_tresorerie(c.numero()),
            "{}",
            c.numero()
        );
    }
    // Les comptes exclus existent dans les données, et ni eux ni leurs sous-comptes ne sont admis.
    for exclu in EXCLUS {
        let exclu = n(exclu);
        assert!(r.compte(&exclu).is_some(), "{exclu}");
        for c in r
            .comptes()
            .filter(|c| exclu.est_ascendant_ou_egal(c.numero()))
        {
            assert!(
                !r.admis_en_journal_de_tresorerie(c.numero()),
                "{}",
                c.numero()
            );
        }
    }
    // Le plan du dossier porte la même marque, héritée par les sous-comptes sauf exclusion.
    let mut plan = r.plan_comptes().unwrap();
    let sous = |liste: &[(&str, u16)], numero: &NumeroCompte| {
        liste
            .iter()
            .any(|(racine, _)| n(racine).est_ascendant_ou_egal(numero))
    };
    for compte in plan.comptes() {
        assert_eq!(
            compte.est_admis_en_journal_de_tresorerie(),
            sous(&ADMIS, compte.numero()) && !sous(&INTERETS_COURUS, compte.numero()),
            "{}",
            compte.numero()
        );
        assert_eq!(
            compte.est_admis_en_journal_de_tresorerie(),
            r.admis_en_journal_de_tresorerie(compte.numero())
        );
    }
    for (admis, _) in ADMIS {
        let feuille = compte_de_mouvement_sous(&plan, admis);
        let sous_compte = NumeroCompte::parse(&format!("{feuille}-A")).unwrap();
        let ajoute = plan
            .ajouter_sous_compte(sous_compte, TexteCourt::parse("A").unwrap())
            .unwrap();
        assert!(ajoute.est_admis_en_journal_de_tresorerie());
    }
}

#[test]
fn journal_de_tresorerie_sur_un_compte_admis_ou_exclu() {
    let r = officiel();
    let plan = r.plan_comptes().unwrap();
    let journal = |compte: NumeroCompte| {
        Journal::nouveau(
            CodeJournal::parse("BQ1").unwrap(),
            TexteCourt::parse("Trésorerie").unwrap(),
            TypeJournal::Tresorerie,
            Some(compte),
            &plan,
        )
    };
    for (admis, _) in ADMIS {
        let compte = compte_de_mouvement_sous(&plan, admis);
        assert!(journal(compte.clone()).is_ok(), "{compte}");
    }
    for exclu in ["54", "56", "585", "59", "526", "536"] {
        let compte = compte_de_mouvement_sous(&plan, exclu);
        assert_eq!(
            journal(compte.clone()),
            Err(ErreurJournal::CompteNonAdmisEnTresorerie(
                CodeJournal::parse("BQ1").unwrap(),
                compte
            ))
        );
    }
}

#[test]
fn rejette_une_marque_de_journal_de_tresorerie_incomplete() {
    let marque = |suite: &str| avec(&format!("{}{suite}\n", compte("10")));
    for suite in [
        "journal_tresorerie = true",
        "page_fonctionnement = 417",
        "journal_tresorerie = false\npage_fonctionnement = 417",
        "journal_tresorerie = true\npage_fonctionnement = 273",
        "journal_tresorerie = true\npage_fonctionnement = 540",
    ] {
        assert_eq!(
            marque(suite).unwrap_err(),
            ErreurReferentiel::MarqueJournalTresorerie(n("10")),
            "{suite}"
        );
    }
    let r = marque("journal_tresorerie = true\npage_fonctionnement = 274").unwrap();
    assert_eq!(
        r.compte(&n("10")).unwrap().marque_journal_tresorerie(),
        Some((true, 274))
    );
    assert!(r.admis_en_journal_de_tresorerie(&n("10-A")));
    assert!(!r.admis_en_journal_de_tresorerie(&n("11")));
    // Exclusion explicite sous un compte admis : elle l'emporte sur l'héritage.
    let r = avec(&format!(
        "{}journal_tresorerie = true\npage_fonctionnement = 274\n{}{}journal_tresorerie = \
         false\npage_fonctionnement = 275\n",
        compte("10"),
        compte("101"),
        compte("1011")
    ))
    .unwrap();
    assert!(r.admis_en_journal_de_tresorerie(&n("10")));
    assert!(r.admis_en_journal_de_tresorerie(&n("101")));
    assert!(!r.admis_en_journal_de_tresorerie(&n("1011")));
    assert!(!r.admis_en_journal_de_tresorerie(&n("1011-A")));
    let plan = r.plan_comptes().unwrap();
    assert!(
        !plan
            .compte(&n("1011"))
            .unwrap()
            .est_admis_en_journal_de_tresorerie()
    );
}

const ENTETE: &str = r#"
[source]
document = "doc"
version = "v"

[[classe]]
numero = 1
intitule = "CLASSE UN"
page = 219
"#;

fn avec(comptes: &str) -> Result<Referentiel, ErreurReferentiel> {
    Referentiel::depuis_toml(&format!("{ENTETE}{comptes}"))
}

fn compte(numero: &str) -> String {
    format!(
        "[[compte]]\nnumero = \"{numero}\"\nintitule = \"X\"\npage = 219\nnature = \"bilan\"\nsens = \"crediteur\"\n"
    )
}

#[test]
fn rejette_un_doublon() {
    let e = avec(&format!("{}{}", compte("10"), compte("10"))).unwrap_err();
    assert_eq!(e, ErreurReferentiel::Doublon(n("10")));
}

#[test]
fn rejette_un_parent_absent() {
    let e = avec(&format!("{}{}", compte("10"), compte("1011"))).unwrap_err();
    assert_eq!(e, ErreurReferentiel::ParentAbsent(n("1011")));
}

#[test]
fn rejette_une_classe_absente() {
    assert_eq!(
        avec(&compte("20")).unwrap_err(),
        ErreurReferentiel::Classe(2)
    );
}

#[test]
fn rejette_un_numero_non_officiel_ou_invalide() {
    assert!(matches!(
        avec(&format!("{}{}", compte("10"), compte("10-A"))).unwrap_err(),
        ErreurReferentiel::NonOfficiel(_)
    ));
    assert!(matches!(
        avec(&compte("1x")).unwrap_err(),
        ErreurReferentiel::Numero { .. }
    ));
}

#[test]
fn rejette_une_page_hors_source() {
    let texte = compte("10").replace("page = 219", "page = 12");
    assert!(matches!(
        avec(&texte).unwrap_err(),
        ErreurReferentiel::Page { page: 12, .. }
    ));
}

#[test]
fn rejette_un_intitule_vide_et_un_champ_inconnu() {
    let vide = compte("10").replace("intitule = \"X\"", "intitule = \" \"");
    assert!(matches!(
        avec(&vide).unwrap_err(),
        ErreurReferentiel::Intitule { .. }
    ));
    let inconnu = format!("{}taux = 3\n", compte("10"));
    assert!(matches!(
        avec(&inconnu).unwrap_err(),
        ErreurReferentiel::Syntaxe(_)
    ));
}

proptest! {
    /// Aucune entrée, même hostile, ne fait paniquer le chargeur.
    #[test]
    fn le_chargeur_ne_panique_jamais(texte in ".{0,400}") {
        let _ = Referentiel::depuis_toml(&texte);
    }

    /// Tout compte du référentiel, privé de chiffres finaux, remonte à un compte présent
    /// jusqu'au niveau à deux chiffres.
    #[test]
    fn chaque_ascendant_est_present(i in 0usize..1401) {
        let r = officiel();
        let c = r.comptes().nth(i).unwrap();
        let mut courant = c.numero().parent();
        while let Some(p) = courant {
            prop_assert!(r.compte(&p).is_some());
            courant = p.parent();
        }
    }
}

#[test]
fn types_de_tiers_admis_par_les_fiches_40_41_42_et_46() {
    // Fiches « Contenu et fonctionnement des comptes » : 40 p. PDF 380, 41 p. 384, 42 p. 389,
    // 46 p. 399. La marque la plus proche s'applique aux sous-comptes.
    let r = officiel();
    let admis = |numero: &str| r.types_de_tiers_admis(&n(numero)).to_vec();
    assert_eq!(admis("401"), ["fournisseur"]);
    assert_eq!(admis("4011"), ["fournisseur"]);
    assert_eq!(admis("411"), ["client"]);
    assert_eq!(admis("421"), ["salarie"]);
    assert_eq!(admis("462"), ["associe"]);
    assert_eq!(r.compte(&n("40")).unwrap().marque_tiers().unwrap().1, 380);
    assert_eq!(r.compte(&n("41")).unwrap().marque_tiers().unwrap().1, 384);
    // Aucun tiers sur les autres collectifs en L2 (Groupe, capital, investissements…).
    for numero in ["466", "103", "109", "181", "451", "481", "485", "488"] {
        assert!(admis(numero).is_empty(), "{numero}");
    }
    // Aucun compte n'admet le type « autre ».
    assert!(r.comptes().all(|c| {
        !r.types_de_tiers_admis(c.numero())
            .contains(&"autre".to_owned())
    }));
}

#[test]
fn une_marque_de_tiers_invalide_est_refusee() {
    let base = r#"
[source]
document = "d"
version = "v"
[[classe]]
numero = 4
intitule = "TIERS"
page = 238
[[compte]]
numero = "40"
intitule = "FOURNISSEURS"
page = 240
nature = "bilan"
sens = "crediteur"
"#;
    for marque in [
        "tiers = [\"fournisseur\"]",
        "page_tiers = 380",
        "tiers = []\npage_tiers = 380",
        "tiers = [\"inconnu\"]\npage_tiers = 380",
        "tiers = [\"client\", \"client\"]\npage_tiers = 380",
        "tiers = [\"client\"]\npage_tiers = 100",
    ] {
        assert_eq!(
            Referentiel::depuis_toml(&format!("{base}{marque}\n")),
            Err(ErreurReferentiel::MarqueTiers(n("40"))),
            "{marque}"
        );
    }
    assert!(
        Referentiel::depuis_toml(&format!("{base}tiers = [\"client\"]\npage_tiers = 380\n"))
            .is_ok()
    );
}
