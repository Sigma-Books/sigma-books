//! Cas de référence : balance générale à six colonnes de l'**Application 127** du Guide
//! d'application du SYSCOHADA révisé (AUDCIF art. 19).
//!
//! **Source** : Guide d'application du SYSCOHADA révisé (version du 1er mai 2017), Application
//! 127, tableau « BALANCE N », pages PDF 347 à 350 (références locales :
//! `references/officiel/Guide_application_SYSCOHADA_revise_2017-05.pdf`). Les 162 comptes et la
//! ligne TOTAL sont relevés dans `tests/donnees/guide-application-127-balance-n.tsv` : numéros
//! et montants seulement, sans les intitulés (règle absolue 10).
//!
//! **Ce que le cas prouve** : un dossier construit sur la liste officielle des comptes, qui
//! reçoit les soldes N-1 du Guide en à-nouveaux et ses mouvements N en écriture validée, donne
//! par [`Balance::calculer`] (format 6, du début de l'exercice au 31 décembre N) exactement la
//! balance imprimée : les six colonnes de chaque compte (solde N-1 débiteur et créditeur,
//! mouvements N débit et crédit, solde N débiteur et créditeur) et la ligne TOTAL. Le
//! regroupement par tronc officiel conserve ces totaux.
//!
//! **Choix** :
//! - **Plan du dossier** : la liste officielle ([`Referentiel::officiel`]), plus un sous-compte
//!   par compte du Guide. Le Guide numérote sur six chiffres (`101100`), la liste officielle sur
//!   deux à quatre : chaque compte du Guide devient un sous-compte du plus long compte officiel
//!   qui préfixe son numéro (`1011-101100`, `462-462001`). Les sous-comptes d'un compte
//!   collectif le sont aussi : leurs lignes portent un tiers fictif.
//! - **`120000` « Report à nouveau »** : son seul préfixe officiel est `12`, compte de
//!   regroupement de 121 « Report à nouveau créditeur » et 129 « Report à nouveau débiteur ». Il
//!   est rattaché à **121** : son solde est créditeur à l'ouverture (47 450 317) comme à la
//!   clôture (76 069 991) de l'exercice N.
//! - **À-nouveaux** : une écriture du journal AN datée du 1er janvier N reprend les soldes N-1.
//!   Elle reste **au brouillard** : le moteur refuse toute saisie dans un journal d'à-nouveaux,
//!   car ces écritures seront générées par la clôture de l'exercice N-1 (spec §6.2, invariant
//!   7 ; lot L6). La balance est donc calculée en mode provisoire, qui ajoute le brouillard aux
//!   écritures validées ; les journaux d'à-nouveaux alimentent toujours la colonne d'ouverture.
//! - **Mouvements N** : le Guide ne donne que leurs cumuls par compte. Une écriture
//!   récapitulative du journal OD, datée du 31 décembre N, porte pour chaque compte une ligne
//!   au débit et une au crédit de ses mouvements (sauf montant nul). Elle est validée par
//!   [`Livre::valider`] le jour même, avec une pièce justificative fictive : la période de
//!   décembre est ouverte et son échéance de clôture (30 avril N+1) n'est pas atteinte.
//! - **Exercices** : le Guide ne date pas l'exemple ; N-1 = 2025 et N = 2026.

// Fichier de test : les `unwrap` y sont voulus (une donnée de test invalide fait échouer le test).
#![allow(clippy::unwrap_used)]

use std::collections::{BTreeMap, BTreeSet};

use compta_core::{
    Balance, CodeExercice, CodeJournal, Compteur, Contexte, Devise, EcritureBrouillon,
    EcritureEquilibree, Exercices, Filtre, FinPremierExercice, FormatBalance, IdTiers, Journaux,
    Justification, Ligne, LigneBalance, Livre, Mode, Montant, NumeroCompte, PlanComptes,
    ReferencePiece, Regroupement, Sens, TexteCourt, TypeJournal, journaux_par_defaut,
};
use rust_decimal::Decimal;
use syscohada::Referentiel;
use time::Date;
use time::macros::date;

/// Balance N du Guide, relevée sur le PDF (voir l'en-tête du fichier).
const BALANCE_N: &str = include_str!("donnees/guide-application-127-balance-n.tsv");

/// Rattachements qui ne suivent pas la règle du plus long préfixe officiel (voir la doc du
/// module) : compte du Guide, compte officiel retenu.
const RATTACHEMENTS_CHOISIS: [(&str, &str); 1] = [("120000", "121")];

/// Ligne de la balance du Guide : compte (ou « TOTAL ») et six colonnes, dans l'ordre imprimé.
struct LigneGuide {
    compte: String,
    colonnes: [Montant; 6],
}

/// Lit le fichier de données : les 162 comptes, puis la ligne TOTAL.
fn guide() -> (Vec<LigneGuide>, LigneGuide) {
    let mut lignes: Vec<LigneGuide> = BALANCE_N
        .lines()
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| {
            let champs: Vec<&str> = l.split('\t').collect();
            assert_eq!(champs.len(), 7, "ligne mal formée : {l}");
            LigneGuide {
                compte: champs[0].to_owned(),
                colonnes: std::array::from_fn(|i| xof(champs[i + 1])),
            }
        })
        .collect();
    let total = lignes.pop().unwrap();
    assert_eq!(total.compte, "TOTAL");
    (lignes, total)
}

fn xof(texte: &str) -> Montant {
    Montant::parse(texte, Devise::XOF).unwrap()
}

fn n(texte: &str) -> NumeroCompte {
    NumeroCompte::parse(texte).unwrap()
}

fn t(texte: &str) -> TexteCourt {
    TexteCourt::parse(texte).unwrap()
}

fn j(code: &str) -> CodeJournal {
    CodeJournal::parse(code).unwrap()
}

/// Colonnes calculées par le moteur, exprimées en montants XOF pour les comparer au Guide.
fn en_xof(valeurs: impl IntoIterator<Item = Decimal>) -> Vec<Montant> {
    valeurs
        .into_iter()
        .map(|v| Montant::nouveau(v, Devise::XOF))
        .collect()
}

/// Plus long compte de la liste officielle dont le numéro préfixe celui du Guide.
fn plus_long_prefixe_officiel(referentiel: &Referentiel, compte_guide: &str) -> NumeroCompte {
    (2..=4)
        .rev()
        .map(|longueur| n(&compte_guide[..longueur]))
        .find(|prefixe| referentiel.compte(prefixe).is_some())
        .unwrap_or_else(|| panic!("aucun compte officiel ne préfixe {compte_guide}"))
}

/// Compte officiel de rattachement d'un compte du Guide (voir la documentation du module).
fn rattachement(referentiel: &Referentiel, compte_guide: &str) -> NumeroCompte {
    if let Some((_, choisi)) = RATTACHEMENTS_CHOISIS
        .iter()
        .find(|(compte, _)| *compte == compte_guide)
    {
        let choisi = n(choisi);
        assert!(referentiel.compte(&choisi).is_some(), "{choisi} officiel");
        assert!(
            plus_long_prefixe_officiel(referentiel, compte_guide).est_ascendant_ou_egal(&choisi),
            "{choisi} doit descendre du préfixe officiel de {compte_guide}"
        );
        return choisi;
    }
    let officiel = plus_long_prefixe_officiel(referentiel, compte_guide);
    // Un rattachement à un compte à deux chiffres serait un regroupement : à trancher et
    // documenter dans RATTACHEMENTS_CHOISIS, comme 120000.
    assert!(
        officiel.as_str().len() >= 3,
        "{compte_guide} ne se rattache qu'à {officiel}"
    );
    officiel
}

/// Sous-compte du dossier qui porte un compte du Guide : `1011-101100`.
fn sous_compte(referentiel: &Referentiel, compte_guide: &str) -> NumeroCompte {
    n(&format!(
        "{}-{compte_guide}",
        rattachement(referentiel, compte_guide)
    ))
}

/// Ligne d'écriture sur le sous-compte ; tiers fictif sur un compte collectif.
fn ligne(plan: &PlanComptes, compte: &NumeroCompte, sens: Sens, montant: Montant) -> Ligne {
    let ligne = Ligne::en_xof(compte.clone(), sens, montant).unwrap();
    if plan.compte(compte).unwrap().est_collectif() {
        ligne.avec_tiers(IdTiers::parse("TIERS-FICTIF").unwrap())
    } else {
        ligne
    }
}

/// Écriture d'un journal, une ligne par montant non nul, justifiée par une pièce fictive.
fn ecriture(
    plan: &PlanComptes,
    journal: &str,
    date: Date,
    libelle: &str,
    lignes: impl IntoIterator<Item = (NumeroCompte, Montant, Montant)>,
) -> EcritureEquilibree {
    let mut brouillon = EcritureBrouillon::nouvelle(j(journal), date, t(libelle));
    for (compte, debit, credit) in lignes {
        for (sens, montant) in [(Sens::Debit, debit), (Sens::Credit, credit)] {
            if !montant.est_nul() {
                brouillon.ajouter_ligne(ligne(plan, &compte, sens, montant));
            }
        }
    }
    brouillon.justifier(
        Justification::pieces([ReferencePiece::parse(&"ab".repeat(32)).unwrap()]).unwrap(),
    );
    brouillon.valider_equilibre().unwrap()
}

/// Dossier de l'Application 127 et balance générale au 31 décembre N.
struct Cas {
    referentiel: Referentiel,
    guide: Vec<LigneGuide>,
    total: LigneGuide,
    livre: Livre,
    brouillard: Vec<EcritureEquilibree>,
    journaux: Journaux,
    exercices: Exercices,
}

impl Cas {
    fn construire() -> Self {
        let referentiel = Referentiel::officiel().unwrap();
        let (guide, total) = guide();

        // Plan : liste officielle + un sous-compte par compte du Guide.
        let mut plan = referentiel.plan_comptes().unwrap();
        for ligne_guide in &guide {
            plan.ajouter_sous_compte(
                sous_compte(&referentiel, &ligne_guide.compte),
                t("Compte du Guide d'application, Application 127"),
            )
            .unwrap();
        }

        // Journaux par défaut ; ceux de trésorerie, qui exigent leur compte, ne servent pas.
        let mut journaux = Journaux::new();
        for modele in journaux_par_defaut().unwrap() {
            if modele.genre() != TypeJournal::Tresorerie {
                journaux
                    .ajouter(modele.instancier(None, &plan).unwrap())
                    .unwrap();
            }
        }

        // Exercices N-1 (2025) et N (2026), ouverts.
        let mut exercices = Exercices::new();
        exercices
            .ouvrir_premier(date!(2025 - 01 - 01), FinPremierExercice::AnneeDeDebut)
            .unwrap();
        exercices.ouvrir_suivant().unwrap();
        let exercice = exercices
            .exercice(&CodeExercice::parse("2026").unwrap())
            .unwrap();

        // À-nouveaux au 1er janvier N, au brouillard : le journal AN refuse la saisie.
        let a_nouveaux = ecriture(
            &plan,
            "AN",
            exercice.debut(),
            "À-nouveaux : soldes N-1 de l'Application 127",
            guide.iter().map(|l| {
                (
                    sous_compte(&referentiel, &l.compte),
                    l.colonnes[0],
                    l.colonnes[1],
                )
            }),
        );

        // Mouvements N : écriture récapitulative d'OD au 31 décembre N, validée le jour même.
        let mouvements = ecriture(
            &plan,
            "OD",
            exercice.fin(),
            "Mouvements N de l'Application 127 (cumuls par compte)",
            guide.iter().map(|l| {
                (
                    sous_compte(&referentiel, &l.compte),
                    l.colonnes[2],
                    l.colonnes[3],
                )
            }),
        );
        let mut livre = Livre::new();
        let contexte = Contexte {
            plan: &plan,
            journaux: &journaux,
            exercices: &exercices,
        };
        let compteur = Compteur::nouveau(j("OD"), exercice.code().clone());
        let (numero, _) = livre
            .valider(mouvements, &contexte, &compteur, exercice.fin())
            .map_err(|refus| refus.decomposer().1)
            .unwrap();
        assert_eq!(numero.to_string(), "OD-2026-000001");

        Self {
            referentiel,
            guide,
            total,
            livre,
            brouillard: vec![a_nouveaux],
            journaux,
            exercices,
        }
    }

    fn balance(&self, mode: Mode, regroupement: Regroupement) -> Balance {
        let exercice = self
            .exercices
            .exercice(&CodeExercice::parse("2026").unwrap())
            .unwrap();
        let filtre = Filtre::exercice(exercice, &self.journaux, exercice.fin()).unwrap();
        Balance::calculer(
            self.livre.ecritures(),
            &self.brouillard,
            mode,
            &filtre,
            regroupement,
        )
        .unwrap()
    }
}

/// Le relevé est cohérent : solde N-1 + mouvements N = solde N pour chaque compte, au plus un
/// côté non nul par solde, colonnes égales à la ligne TOTAL, comptes uniques. L'ordre imprimé
/// est conservé : le Guide place 488000 avant 485100.
#[test]
fn le_releve_du_guide_est_coherent() {
    let (guide, total) = guide();
    assert_eq!(guide.len(), 162);
    let comptes: BTreeSet<&str> = guide.iter().map(|l| l.compte.as_str()).collect();
    assert_eq!(comptes.len(), guide.len(), "comptes en double");
    assert!(
        guide
            .iter()
            .all(|l| l.compte.len() == 6 && l.compte.bytes().all(|b| b.is_ascii_digit()))
    );
    for l in &guide {
        let [od, oc, md, mc, sd, sc] = l.colonnes.map(|m| Montant::valeur(&m));
        assert_eq!(od - oc + md - mc, sd - sc, "{}", l.compte);
        assert!(od.is_zero() || oc.is_zero(), "{}", l.compte);
        assert!(sd.is_zero() || sc.is_zero(), "{}", l.compte);
    }
    for colonne in 0..6 {
        let somme = guide
            .iter()
            .map(|l| l.colonnes[colonne])
            .try_fold(Montant::zero(Devise::XOF), |a, b| a.ajouter(&b))
            .unwrap();
        assert_eq!(somme, total.colonnes[colonne], "colonne {colonne}");
    }
}

/// `120000` n'a que le regroupement `12` pour préfixe officiel : il est rattaché à 121.
#[test]
fn rattachement_du_report_a_nouveau() {
    let referentiel = Referentiel::officiel().unwrap();
    assert_eq!(plus_long_prefixe_officiel(&referentiel, "120000"), n("12"));
    assert_eq!(sous_compte(&referentiel, "120000"), n("121-120000"));
    assert_eq!(sous_compte(&referentiel, "101100"), n("1011-101100"));
    assert_eq!(sous_compte(&referentiel, "462001"), n("462-462001"));
}

/// AUDCIF art. 19 : la balance générale à six colonnes au 31 décembre N égale, compte par
/// compte et colonne par colonne, la balance N du Guide, totaux compris.
#[test]
fn balance_generale_conforme_a_l_application_127() {
    let cas = Cas::construire();
    let balance = cas.balance(Mode::Provisoire, Regroupement::ParCompte);
    let calculees: BTreeMap<&str, &LigneBalance> =
        balance.lignes().iter().map(|l| (l.cle(), l)).collect();
    assert_eq!(
        calculees.len(),
        cas.guide.len(),
        "une ligne par compte du Guide"
    );
    for attendue in &cas.guide {
        let cle = sous_compte(&cas.referentiel, &attendue.compte);
        let calculee = calculees
            .get(cle.as_str())
            .unwrap_or_else(|| panic!("{cle} absent de la balance"));
        assert_eq!(
            en_xof(calculee.colonnes(FormatBalance::Six).unwrap()),
            attendue.colonnes,
            "compte {}",
            attendue.compte
        );
    }
    assert_eq!(
        en_xof(balance.totaux(FormatBalance::Six).unwrap()),
        cas.total.colonnes
    );
}

/// Sans le brouillard (mode définitif), seuls les mouvements validés restent : colonne
/// d'ouverture vide, mouvements N inchangés.
#[test]
fn les_mouvements_n_sont_valides_et_les_a_nouveaux_au_brouillard() {
    let cas = Cas::construire();
    let totaux = en_xof(
        cas.balance(Mode::Definitif, Regroupement::ParCompte)
            .totaux(FormatBalance::Six)
            .unwrap(),
    );
    let zero = Montant::zero(Devise::XOF);
    assert_eq!(&totaux[..2], [zero, zero]);
    assert_eq!(&totaux[2..4], &cas.total.colonnes[2..4]);
}

/// Regroupement par tronc officiel (spec §5.1) : une ligne par compte de rattachement, mêmes
/// totaux que la balance du Guide (aucun tronc ne mêle des soldes de sens contraires).
#[test]
fn regroupement_par_tronc_memes_totaux() {
    let cas = Cas::construire();
    let balance = cas.balance(Mode::Provisoire, Regroupement::ParTronc);
    let mut troncs: Vec<NumeroCompte> = cas
        .guide
        .iter()
        .map(|l| rattachement(&cas.referentiel, &l.compte))
        .collect();
    troncs.sort();
    troncs.dedup();
    let cles: Vec<&str> = balance.lignes().iter().map(LigneBalance::cle).collect();
    let attendues: Vec<&str> = troncs.iter().map(NumeroCompte::as_str).collect();
    assert_eq!(cles, attendues);
    assert_eq!(
        en_xof(balance.totaux(FormatBalance::Six).unwrap()),
        cas.total.colonnes
    );
}
