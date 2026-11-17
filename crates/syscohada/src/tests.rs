//! Tests du référentiel : validation des données embarquées, contrôles ponctuels contre la liste
//! officielle (AUDCIF + SYSCOHADA, JO OHADA 15/02/2017), rejet des données invalides.

use super::*;
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

#[test]
fn comptes_collectifs_de_tiers() {
    let r = officiel();
    let plan = r.plan_comptes().unwrap();
    for numero in ["4011", "4111", "4211", "4041", "4131", "4621"] {
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
