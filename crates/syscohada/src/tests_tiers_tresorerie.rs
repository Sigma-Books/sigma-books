//! Tests des données de tiers et de trésorerie (L3-12) : marques `lettrable` et
//! `type_tresorerie` de `data/comptes.toml`, rôles de comptes de `data/roles-comptes.toml`.
//! Les pages citées sont celles des fiches « Contenu et fonctionnement des comptes » de l'AUDCIF
//! (JO OHADA 15/02/2017), relues sur le PDF.

use super::*;
use proptest::prelude::*;

fn n(numero: &str) -> NumeroCompte {
    NumeroCompte::parse(numero).unwrap()
}

fn officiel() -> Referentiel {
    Referentiel::officiel().expect("référentiel embarqué valide")
}

// --- Marque `lettrable` -----------------------------------------------------------------------

#[test]
fn comptes_marques_lettrables_avec_leur_page() {
    let r = officiel();
    let marques: Vec<(&str, u16)> = r
        .comptes()
        .filter_map(|c| c.marque_lettrable().map(|p| (c.numero().as_str(), p)))
        .collect();
    assert_eq!(
        marques,
        [
            ("40", 381),
            ("41", 386),
            ("421", 389),
            ("451", 397),
            ("452", 397),
            ("461", 399),
            ("462", 399),
            ("463", 399),
            ("465", 399),
            ("466", 399),
            ("485", 406),
            ("488", 406),
            ("585", 430),
            ("588", 430),
        ]
    );
}

#[test]
fn le_lettrage_suit_la_marque_la_plus_proche() {
    let r = officiel();
    for numero in [
        "401",
        "4011",
        "404",
        "4041",
        "409",
        "411",
        "413",
        "4131",
        "419",
        "4211",
        "4851",
        "585",
        "588", // segments libres du dossier : auxiliaire et sous-compte
        "411-SENELEC",
        "585-01",
    ] {
        assert!(r.est_lettrable(&n(numero)), "{numero}");
    }
    // Collectifs sans source (question ouverte) et comptes étrangers au lettrage.
    for numero in [
        "103", "109", "181", "185", "467", "481", "4811", "482", "422", "4711", "4712", "471",
        "521", "5711", "552", "581", "601",
    ] {
        assert!(!r.est_lettrable(&n(numero)), "{numero}");
    }
}

#[test]
fn un_compte_lettrable_est_collectif_sauf_les_virements_internes() {
    let r = officiel();
    let plan = r.plan_comptes().unwrap();
    let non_collectifs: Vec<&str> = r
        .comptes()
        .filter(|c| r.est_lettrable(c.numero()))
        .filter(|c| !plan.compte(c.numero()).unwrap().est_collectif())
        .map(|c| c.numero().as_str())
        .collect();
    // 40 et 41 portent la marque pour leurs sous-comptes collectifs : regroupements sans ligne.
    assert_eq!(non_collectifs, ["40", "41", "585", "588"]);
}

#[test]
fn aucun_compte_lettrable_n_est_un_compte_de_tresorerie() {
    let r = officiel();
    assert!(
        r.comptes()
            .all(|c| !(r.est_lettrable(c.numero()) && r.admis_en_journal_de_tresorerie(c.numero())))
    );
}

// --- Marque `type_tresorerie` -----------------------------------------------------------------

#[test]
fn compte_par_defaut_de_chaque_type_de_tresorerie() {
    // Spec §9.1 ; fiches 52 (p. PDF 416), 57 (p. 428), 55 (p. 423).
    let r = officiel();
    let attendu = [
        (TypeTresorerie::Banque, "521", 416, "BANQUES LOCALES"),
        (TypeTresorerie::Caisse, "571", 428, "CAISSE SIÈGE SOCIAL"),
        (
            TypeTresorerie::Portefeuille,
            "552",
            423,
            "MONNAIE ÉLECTRONIQUE - TÉLÉPHONE PORTABLE",
        ),
    ];
    for (type_tresorerie, numero, page, intitule) in attendu {
        let defaut = r.compte_par_defaut_tresorerie(type_tresorerie).unwrap();
        assert_eq!(defaut, &n(numero));
        let compte = r.compte(defaut).unwrap();
        assert_eq!(
            compte.marque_type_tresorerie(),
            Some((type_tresorerie, page))
        );
        assert_eq!(compte.intitule().as_str(), intitule);
        assert!(r.admis_en_journal_de_tresorerie(defaut));
    }
    let marques = r
        .comptes()
        .filter(|c| c.marque_type_tresorerie().is_some())
        .count();
    assert_eq!(marques, TypeTresorerie::TOUS.len());
}

#[test]
fn le_journal_de_caisse_par_defaut_est_sous_le_compte_par_defaut_de_la_caisse() {
    // `journaux.toml` : CA sur 5711, sous-compte de 571.
    let r = officiel();
    let caisse = r
        .compte_par_defaut_tresorerie(TypeTresorerie::Caisse)
        .unwrap();
    let comptes = r.comptes_des_journaux().unwrap();
    assert_ne!(comptes.len(), 0);
    assert!(
        comptes
            .iter()
            .all(|c| caisse.est_ascendant_ou_egal(c.compte()))
    );
}

#[test]
fn codes_des_types_de_tresorerie() {
    for t in TypeTresorerie::TOUS {
        assert_eq!(TypeTresorerie::depuis_code(t.code()), Some(t));
        assert_eq!(t.to_string(), t.code());
    }
    assert_eq!(TypeTresorerie::depuis_code("mobile_money"), None);
}

// --- Marques invalides --------------------------------------------------------------------------

const BASE: &str = r#"
[source]
document = "d"
version = "v"
[[classe]]
numero = 4
intitule = "TIERS"
page = 240
[[classe]]
numero = 5
intitule = "TRÉSORERIE"
page = 249
[[compte]]
numero = "41"
intitule = "CLIENTS"
page = 241
nature = "bilan"
sens = "debiteur"
[[compte]]
numero = "411"
intitule = "CLIENTS"
page = 241
nature = "bilan"
sens = "debiteur"
collectif = true
[[compte]]
numero = "57"
intitule = "CAISSE"
page = 252
nature = "bilan"
sens = "debiteur"
journal_tresorerie = true
page_fonctionnement = 428
[[compte]]
numero = "571"
intitule = "CAISSE SIÈGE"
page = 252
nature = "bilan"
sens = "debiteur"
[[compte]]
numero = "572"
intitule = "CAISSE B"
page = 252
nature = "bilan"
sens = "debiteur"
[[compte]]
numero = "58"
intitule = "VIREMENTS"
page = 252
nature = "bilan"
sens = "variable"
[[compte]]
numero = "585"
intitule = "VIREMENTS DE FONDS"
page = 252
nature = "bilan"
sens = "variable"
"#;

/// Ajoute `marque` au bloc du compte `numero` de [`BASE`].
fn base_avec(marques: &[(&str, &str)]) -> Result<Referentiel, ErreurReferentiel> {
    let mut texte = BASE.to_owned();
    for (numero, marque) in marques {
        let entete = format!("numero = \"{numero}\"\n");
        let debut = texte.find(&entete).unwrap();
        let fin = texte[debut..]
            .find("[[compte]]")
            .map_or(texte.len(), |i| debut + i);
        texte.insert_str(fin, &format!("{marque}\n"));
    }
    Referentiel::depuis_toml(&texte)
}

#[test]
fn la_base_de_test_est_valide() {
    let r = base_avec(&[
        ("41", "lettrable = true\npage_lettrable = 386"),
        ("585", "lettrable = true\npage_lettrable = 430"),
        (
            "571",
            "type_tresorerie = \"caisse\"\npage_type_tresorerie = 428",
        ),
    ])
    .unwrap();
    assert!(r.est_lettrable(&n("411-X")));
    assert!(!r.est_lettrable(&n("571")));
    assert_eq!(
        r.compte_par_defaut_tresorerie(TypeTresorerie::Caisse),
        Some(&n("571"))
    );
    assert_eq!(r.compte_par_defaut_tresorerie(TypeTresorerie::Banque), None);
}

#[test]
fn une_marque_lettrable_invalide_est_refusee() {
    for (numero, marque) in [
        ("41", "lettrable = true"),
        ("41", "page_lettrable = 386"),
        ("41", "lettrable = false\npage_lettrable = 386"),
        ("41", "lettrable = true\npage_lettrable = 250"),
        ("41", "lettrable = true\npage_lettrable = 540"),
        // Compte de trésorerie, lui-même ou par un sous-compte.
        ("571", "lettrable = true\npage_lettrable = 428"),
        ("57", "lettrable = true\npage_lettrable = 428"),
    ] {
        assert_eq!(
            base_avec(&[(numero, marque)]).unwrap_err(),
            ErreurReferentiel::MarqueLettrable(n(numero)),
            "{numero} : {marque}"
        );
    }
    // Doublon : marque sous un ascendant déjà lettrable.
    assert_eq!(
        base_avec(&[
            ("41", "lettrable = true\npage_lettrable = 386"),
            ("411", "lettrable = true\npage_lettrable = 386"),
        ])
        .unwrap_err(),
        ErreurReferentiel::MarqueLettrable(n("411"))
    );
    assert!(matches!(
        base_avec(&[("41", "lettrable = \"oui\"\npage_lettrable = 386")]).unwrap_err(),
        ErreurReferentiel::Syntaxe(_)
    ));
}

#[test]
fn une_marque_de_type_de_tresorerie_invalide_est_refusee() {
    for (numero, marque) in [
        ("571", "type_tresorerie = \"caisse\""),
        ("571", "page_type_tresorerie = 428"),
        (
            "571",
            "type_tresorerie = \"coffre\"\npage_type_tresorerie = 428",
        ),
        (
            "571",
            "type_tresorerie = \"caisse\"\npage_type_tresorerie = 252",
        ),
        // Non admis en journal de trésorerie.
        (
            "585",
            "type_tresorerie = \"caisse\"\npage_type_tresorerie = 430",
        ),
        (
            "411",
            "type_tresorerie = \"banque\"\npage_type_tresorerie = 386",
        ),
    ] {
        assert_eq!(
            base_avec(&[(numero, marque)]).unwrap_err(),
            ErreurReferentiel::MarqueTypeTresorerie(n(numero)),
            "{numero} : {marque}"
        );
    }
    // Doublon : deux comptes pour le même type.
    assert_eq!(
        base_avec(&[
            (
                "571",
                "type_tresorerie = \"caisse\"\npage_type_tresorerie = 428"
            ),
            (
                "572",
                "type_tresorerie = \"caisse\"\npage_type_tresorerie = 428"
            ),
        ])
        .unwrap_err(),
        ErreurReferentiel::MarqueTypeTresorerie(n("572"))
    );
}

// --- Rôles de comptes ---------------------------------------------------------------------------

fn roles() -> RolesComptes {
    officiel().roles_de_comptes().expect("rôles valides")
}

#[test]
fn compte_d_attente_des_releves_suggere_4711_et_4712() {
    let roles = roles();
    let attente = roles.definition(RoleCompte::AttenteReleve).unwrap();
    assert!(attente.designation_obligatoire());
    assert!(attente.lettrable());
    assert_eq!(attente.a_confirmer_par(), Confirmation::ExpertComptable);
    assert!(attente.source().contains("ADR 0034"));
    let vues: Vec<(&str, &str, u16)> = attente
        .suggestions()
        .iter()
        .map(|s| (s.numero().as_str(), s.intitule(), s.page_liste()))
        .collect();
    assert_eq!(
        vues,
        [
            ("4711", "Débiteurs divers", 245),
            ("4712", "Créditeurs divers", 245),
        ]
    );
    assert!(
        attente
            .suggestions()
            .iter()
            .all(|s| s.source().contains("p. PDF 245"))
    );
    // Lettrable par la désignation, non par une marque des données.
    let r = officiel();
    assert!(
        attente
            .suggestions()
            .iter()
            .all(|s| !r.est_lettrable(s.numero()))
    );
}

#[test]
fn roles_du_prelevement_et_de_la_taxe_sans_suggestion() {
    // ADR 0034 point 10 : aucune suggestion de numéro tant que la thèse n'est pas validée.
    let roles = roles();
    for role in [RoleCompte::ChargePrelevement, RoleCompte::DetteCollectee] {
        let d = roles.definition(role).unwrap();
        assert!(!d.designation_obligatoire(), "{role:?}");
        assert!(!d.lettrable(), "{role:?}");
        assert_eq!(d.a_confirmer_par(), Confirmation::Fiscaliste);
        assert!(d.suggestions().is_empty(), "{role:?}");
        assert!(d.source().contains("JO n° 7853"), "{role:?}");
        assert!(d.source().contains("220 ter"), "{role:?}");
    }
}

#[test]
fn chaque_role_est_defini_une_fois() {
    let roles = roles();
    assert_eq!(roles.definitions().len(), RoleCompte::TOUS.len());
    for role in RoleCompte::TOUS {
        assert_eq!(roles.definition(role).unwrap().role(), role);
        assert_eq!(RoleCompte::depuis_code(role.code()), Some(role));
    }
}

const ROLES_MINIMAUX: &str = r#"
[[role]]
code = "attente_releve"
designation_obligatoire = true
lettrable = true
a_confirmer_par = "expert_comptable"
source = "s"
[[role]]
code = "charge_prelevement"
designation_obligatoire = false
lettrable = false
a_confirmer_par = "fiscaliste"
source = "s"
[[role]]
code = "dette_collectee"
designation_obligatoire = false
lettrable = false
a_confirmer_par = "fiscaliste"
source = "s"
"#;

fn suggestion(numero: &str, intitule: &str, page: u16) -> String {
    format!(
        "[[role.suggestion]]\nnumero = \"{numero}\"\nintitule = \"{intitule}\"\npage_liste = {page}\nsource = \"p. PDF {page}\"\n"
    )
}

/// Insère `ajout` à la fin du premier rôle (`attente_releve`, lettrable).
fn roles_avec(ajout: &str) -> Result<RolesComptes, ErreurReferentiel> {
    let coupe = ROLES_MINIMAUX.find("[[role]]\ncode = \"charge").unwrap();
    let texte = format!(
        "{}{ajout}{}",
        &ROLES_MINIMAUX[..coupe],
        &ROLES_MINIMAUX[coupe..]
    );
    officiel().roles_de_comptes_depuis_toml(&texte)
}

#[test]
fn des_roles_minimaux_sont_acceptes() {
    let roles = roles_avec(&suggestion("4711", "Débiteurs divers", 245)).unwrap();
    assert_eq!(
        roles
            .definition(RoleCompte::AttenteReleve)
            .unwrap()
            .suggestions()
            .len(),
        1
    );
}

#[test]
fn des_roles_invalides_sont_refuses() {
    let r = officiel();
    let refuse = |texte: &str| {
        matches!(
            r.roles_de_comptes_depuis_toml(texte),
            Err(ErreurReferentiel::RolesComptes(_))
        )
    };
    // Rôle inconnu, en double, absent ; confirmation inconnue ; source vide.
    assert!(refuse(
        &ROLES_MINIMAUX.replace("\"dette_collectee\"", "\"taxe_inconnue\"")
    ));
    assert!(refuse(
        &ROLES_MINIMAUX.replace("\"dette_collectee\"", "\"charge_prelevement\"")
    ));
    let sans_dernier = &ROLES_MINIMAUX[..ROLES_MINIMAUX.rfind("[[role]]").unwrap()];
    assert!(refuse(sans_dernier));
    assert!(refuse(&ROLES_MINIMAUX.replacen(
        "\"expert_comptable\"",
        "\"comptable\"",
        1
    )));
    assert!(refuse(&ROLES_MINIMAUX.replacen(
        "source = \"s\"",
        "source = \" \"",
        1
    )));
    // Champ inconnu : refus de syntaxe.
    assert!(matches!(
        r.roles_de_comptes_depuis_toml(&format!("{ROLES_MINIMAUX}numero = \"4711\"\n")),
        Err(ErreurReferentiel::Syntaxe(_))
    ));
}

#[test]
fn des_suggestions_invalides_sont_refusees() {
    for ajout in [
        // Absent de la liste, intitulé ou page différents de comptes.toml.
        suggestion("4710", "Débiteurs divers", 245),
        suggestion("4711", "Débiteurs", 245),
        suggestion("4711", "Débiteurs divers", 246),
        // Source vide.
        suggestion("4711", "Débiteurs divers", 245).replace("p. PDF 245", " "),
        // En double.
        format!(
            "{}{}",
            suggestion("4711", "Débiteurs divers", 245),
            suggestion("4711", "Débiteurs divers", 245)
        ),
        // Compte de trésorerie.
        suggestion("5711", "Caisse en monnaie nationale", 252),
        // Collectif pour un rôle lettrable (les lignes d'attente n'ont pas de tiers).
        suggestion("4111", "Clients", 241),
    ] {
        assert!(
            matches!(roles_avec(&ajout), Err(ErreurReferentiel::RolesComptes(_))),
            "{ajout}"
        );
    }
    assert!(matches!(
        roles_avec(&suggestion("47-A", "X", 245)),
        Err(ErreurReferentiel::RolesComptes(_) | ErreurReferentiel::Numero { .. })
    ));
}

proptest! {
    /// Aucune entrée, même hostile, ne fait paniquer le chargeur des rôles.
    #[test]
    fn le_chargeur_des_roles_ne_panique_jamais(texte in ".{0,400}") {
        let _ = officiel().roles_de_comptes_depuis_toml(&texte);
    }

    /// La marque `lettrable` s'hérite : un sous-compte (officiel ou libre) d'un compte lettrable
    /// est lettrable.
    #[test]
    fn la_marque_lettrable_s_herite(i in 0usize..1401, segment in "[A-Z0-9]{1,6}") {
        let r = officiel();
        let c = r.comptes().nth(i).unwrap();
        let libre = n(&format!("{}-{segment}", c.numero()));
        prop_assert_eq!(r.est_lettrable(&libre), r.est_lettrable(c.numero()));
        if let Some(parent) = c.numero().parent() {
            prop_assert!(!r.est_lettrable(&parent) || r.est_lettrable(c.numero()));
        }
    }
}
