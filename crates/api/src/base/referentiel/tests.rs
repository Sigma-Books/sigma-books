//! Tests sans base du référentiel d'un dossier : propriétés du stockage à plat, contrôle
//! croisé, préparation d'un nouveau dossier. Aucun numéro de compte écrit en dur : tous sont
//! tirés du référentiel officiel (`syscohada`).
#![allow(clippy::unwrap_used, clippy::expect_used)]

use compta_core::{Compte, ErreurPlan};
use proptest::prelude::*;
use time::{Date, Month};

use super::*;

fn modele() -> &'static Modele {
    Modele::officiel().unwrap()
}

fn t(texte: &str) -> TexteCourt {
    TexteCourt::parse(texte).unwrap()
}

fn date(annee: i32, mois: Month, jour: u8) -> Date {
    Date::from_calendar_date(annee, mois, jour).unwrap()
}

/// Comptes du plan officiel qui vérifient `filtre`, par ordre de numéro.
fn comptes(plan: &PlanComptes, filtre: impl Fn(&Compte) -> bool) -> Vec<NumeroCompte> {
    plan.comptes()
        .filter(|c| filtre(c))
        .map(|c| c.numero().clone())
        .collect()
}

/// Compte du journal de caisse (CA), lu dans les données de `syscohada`.
fn compte_de_caisse(etat: &EtatReferentiel) -> NumeroCompte {
    etat.journaux
        .journaux()
        .find_map(|j| j.compte_tresorerie().cloned())
        .unwrap()
}

#[test]
fn le_modele_officiel_est_valide_et_son_empreinte_stable() {
    let m = modele();
    assert!(m.etat().plan.comptes().count() > 1000);
    assert_eq!(verifier(m.etat()), Ok(()));
    assert_eq!(empreinte(m.version(), m.etat()), m.empreinte());
    // Toute différence de contenu change l'empreinte.
    let mut autre = m.etat().clone();
    let numero = comptes(&autre.plan, Compte::est_actif)[0].clone();
    autre.plan.desactiver(&numero).unwrap();
    assert_ne!(empreinte(m.version(), &autre), m.empreinte());
    assert_ne!(empreinte("autre version", m.etat()), m.empreinte());
}

#[test]
fn aller_retour_du_modele_officiel() {
    let m = modele();
    let plan = reconstituer_plan(aplatir_plan(&m.etat().plan)).unwrap();
    assert_eq!(plan, m.etat().plan);
    let journaux = reconstituer_journaux(aplatir_journaux(&m.etat().journaux), &plan).unwrap();
    assert_eq!(journaux, m.etat().journaux);
    // Lignes en ordre inverse : même résultat.
    let mut lignes = aplatir_plan(&m.etat().plan);
    lignes.reverse();
    assert_eq!(reconstituer_plan(lignes).unwrap(), m.etat().plan);
}

#[test]
fn une_ligne_incoherente_est_refusee() {
    let mut etat = modele().etat().clone();
    let parent = comptes(&etat.plan, |c| !c.est_collectif())[5].clone();
    let numero = NumeroCompte::parse(&format!("{parent}-X")).unwrap();
    etat.plan
        .ajouter_sous_compte(numero.clone(), t("X"))
        .unwrap();
    let mut lignes = aplatir_plan(&etat.plan);
    let ligne = lignes.iter_mut().find(|l| l.numero == numero).unwrap();
    ligne.collectif = !ligne.collectif;
    assert_eq!(
        reconstituer_plan(lignes),
        Err(ErreurRelecture::Incoherence(numero))
    );
}

#[test]
fn le_controle_croise_refuse_ce_qui_rendrait_le_journal_de_caisse_illisible() {
    let mut etat = modele().etat().clone();
    let caisse = compte_de_caisse(&etat);
    let sous = NumeroCompte::parse(&format!("{caisse}-A")).unwrap();
    let mut candidat = etat.clone();
    candidat
        .plan
        .ajouter_sous_compte(sous, t("Caisse A"))
        .unwrap();
    assert!(matches!(
        verifier(&candidat),
        Err(RefusReferentiel::Journal(
            ErreurJournal::CompteTresorerieRetire(_, ErreurPlan::Regroupement(_))
        ))
    ));
    let mut candidat = etat.clone();
    candidat.plan.desactiver(&caisse).unwrap();
    assert!(verifier(&candidat).is_err());

    // Un second journal sur le compte de caisse est refusé.
    let journal = Journal::nouveau(
        CodeJournal::parse("CA2").unwrap(),
        t("Caisse bis"),
        TypeJournal::Tresorerie,
        Some(caisse.clone()),
        &etat.plan,
    )
    .unwrap();
    let mut candidat = etat.clone();
    candidat.journaux.ajouter(journal).unwrap();
    assert!(matches!(
        verifier(&candidat),
        Err(RefusReferentiel::CompteTresorerieDejaUtilise(..))
    ));

    // Tiers : compte collectif actif exigé tant que le tiers est actif.
    // Comptes collectifs de mouvement qui admettent un client, un fournisseur (données).
    let admet = |etat: &EtatReferentiel, genre: TypeTiers| {
        comptes(&etat.plan, |c| {
            c.est_collectif()
                && !etat.plan.a_des_enfants(c.numero())
                && types_de_tiers_admis(etat, c.numero()).is_some_and(|t| t.contains(&genre))
        })[0]
            .clone()
    };
    let collectif = admet(&etat, TypeTiers::Client);
    let non_collectif = comptes(&etat.plan, |c| !c.est_collectif())[0].clone();
    etat.tiers.insert(
        Uuid::new_v4(),
        Tiers::nouveau(t("Client"), TypeTiers::Client, collectif.clone()),
    );
    assert_eq!(verifier(&etat), Ok(()));
    let mut candidat = etat.clone();
    candidat.plan.desactiver(&collectif).unwrap();
    assert_eq!(
        verifier(&candidat),
        Err(RefusReferentiel::CompteDeTiers(collectif.clone()))
    );
    for tiers in candidat.tiers.values_mut() {
        tiers.definir_actif(false);
    }
    assert_eq!(verifier(&candidat), Ok(()));
    let mut candidat = etat.clone();
    candidat.tiers.insert(
        Uuid::new_v4(),
        Tiers::nouveau(t("Autre"), TypeTiers::Autre, non_collectif.clone()),
    );
    assert_eq!(
        verifier(&candidat),
        Err(RefusReferentiel::CompteDeTiers(non_collectif))
    );
}

#[test]
fn le_type_du_tiers_doit_etre_admis_sur_son_compte_collectif() {
    let etat = modele().etat().clone();
    // Comptes collectifs de mouvement qui admettent un client, un fournisseur (données).
    let admet = |etat: &EtatReferentiel, genre: TypeTiers| {
        comptes(&etat.plan, |c| {
            c.est_collectif()
                && !etat.plan.a_des_enfants(c.numero())
                && types_de_tiers_admis(etat, c.numero()).is_some_and(|t| t.contains(&genre))
        })[0]
            .clone()
    };
    let collectif = admet(&etat, TypeTiers::Client);
    let fournisseurs = admet(&etat, TypeTiers::Fournisseur);
    // Type du tiers contraire à la marque de son compte : refusé. « Autre » : nulle part.
    for (genre, compte) in [
        (TypeTiers::Fournisseur, collectif.clone()),
        (TypeTiers::Client, fournisseurs.clone()),
        (TypeTiers::Autre, collectif.clone()),
        (TypeTiers::Salarie, collectif.clone()),
    ] {
        let mut candidat = etat.clone();
        candidat.tiers.insert(
            Uuid::new_v4(),
            Tiers::nouveau(t("T"), genre, compte.clone()),
        );
        assert_eq!(
            verifier(&candidat),
            Err(RefusReferentiel::TypeDeTiersRefuse(genre, compte))
        );
    }
    let collectif_sans_marque = comptes(&etat.plan, |c| {
        c.est_collectif() && types_de_tiers_admis(&etat, c.numero()).is_none()
    })[0]
        .clone();
    for genre in TypeTiers::TOUS {
        let mut candidat = etat.clone();
        candidat.tiers.insert(
            Uuid::new_v4(),
            Tiers::nouveau(t("T"), genre, collectif_sans_marque.clone()),
        );
        assert!(verifier(&candidat).is_err(), "{genre:?}");
    }
    // Un sous-compte collectif hérite de la marque ; un compte auxiliaire est refusé.
    let mut candidat = etat.clone();
    let sous = NumeroCompte::parse(&format!("{collectif}-DAKAR")).unwrap();
    let auxiliaire = NumeroCompte::parse(&format!("{collectif}-SENELEC")).unwrap();
    candidat
        .plan
        .ajouter_sous_compte(sous.clone(), t("Dakar"))
        .unwrap();
    candidat
        .plan
        .ajouter_auxiliaire(auxiliaire.clone(), t("Senelec"))
        .unwrap();
    candidat.tiers.clear();
    candidat.tiers.insert(
        Uuid::new_v4(),
        Tiers::nouveau(t("A"), TypeTiers::Client, sous),
    );
    assert_eq!(verifier(&candidat), Ok(()));
    candidat.tiers.insert(
        Uuid::new_v4(),
        Tiers::nouveau(t("B"), TypeTiers::Client, auxiliaire.clone()),
    );
    assert_eq!(
        verifier(&candidat),
        Err(RefusReferentiel::CompteDeTiers(auxiliaire))
    );
}

#[test]
fn preparation_d_un_nouveau_dossier() {
    let m = modele();
    let aujourd_hui = date(2026, Month::November, 19);
    // Dossier repris : exercice civil choisi, jamais une année future.
    let i = Instanciation::preparer(m, Ouverture::Reprise { annee: 2026 }, aujourd_hui).unwrap();
    let e = &i.exercices().exercices()[0];
    assert_eq!(
        (e.code().as_str(), e.debut(), e.fin()),
        (
            "2026",
            date(2026, Month::January, 1),
            date(2026, Month::December, 31)
        )
    );
    assert_eq!(e.periodes().len(), 12);
    // Reprise : N ou N-1 (exercice précédent pas encore arrêté, AUDCIF art. 23).
    assert!(Instanciation::preparer(m, Ouverture::Reprise { annee: 2025 }, aujourd_hui).is_ok());
    for annee in [2027, 2024, 2000] {
        assert_eq!(
            Instanciation::preparer(m, Ouverture::Reprise { annee }, aujourd_hui),
            Err(RefusReferentiel::DateRefusee),
            "{annee}"
        );
    }
    // Entité nouvelle : AUDCIF art. 7, second semestre seulement pour la prolongation.
    let creation = |debut, fin| Ouverture::Creation {
        debut_activite: debut,
        fin,
    };
    let i = Instanciation::preparer(
        m,
        creation(
            date(2026, Month::September, 15),
            FinPremierExercice::AnneeSuivante,
        ),
        aujourd_hui,
    )
    .unwrap();
    assert_eq!(i.exercices().exercices()[0].code().as_str(), "2027");
    assert!(matches!(
        Instanciation::preparer(
            m,
            creation(
                date(2026, Month::March, 1),
                FinPremierExercice::AnneeSuivante
            ),
            aujourd_hui
        ),
        Err(RefusReferentiel::Exercice(
            ErreurExercice::ProlongationImpossible(_)
        ))
    ));
    assert!(
        Instanciation::preparer(
            m,
            creation(
                date(2027, Month::January, 1),
                FinPremierExercice::AnneeDeDebut
            ),
            aujourd_hui
        )
        .is_ok()
    );
    assert!(
        Instanciation::preparer(
            m,
            creation(
                date(2025, Month::January, 1),
                FinPremierExercice::AnneeDeDebut
            ),
            aujourd_hui
        )
        .is_ok()
    );
    // Création : du 1er janvier N-1 au 31 décembre N+1 ; une entité plus ancienne n'obtient
    // pas l'option « second semestre ».
    for debut in [
        date(2028, Month::January, 1),
        date(2024, Month::December, 31),
        date(2024, Month::September, 1),
    ] {
        assert_eq!(
            Instanciation::preparer(
                m,
                creation(debut, FinPremierExercice::AnneeSuivante),
                aujourd_hui
            ),
            Err(RefusReferentiel::DateRefusee),
            "{debut}"
        );
    }
    assert_eq!(i.etat(), m.etat());
    assert_eq!((i.version(), i.empreinte()), (m.version(), m.empreinte()));
}

/// Opération tirée sur le référentiel d'un dossier.
#[derive(Debug, Clone)]
enum Operation {
    SousCompte(usize, String),
    Auxiliaire(usize, String),
    Renommer(usize, String),
    Desactiver(usize),
    Reactiver(usize),
    JournalTresorerie(usize, u8),
}

fn operation() -> impl Strategy<Value = Operation> {
    let segment = "[A-Z0-9]{1,3}";
    prop_oneof![
        3 => (any::<usize>(), segment).prop_map(|(i, s)| Operation::SousCompte(i, s)),
        2 => (any::<usize>(), segment).prop_map(|(i, s)| Operation::Auxiliaire(i, s)),
        1 => (any::<usize>(), "[a-z]{1,8}").prop_map(|(i, s)| Operation::Renommer(i, s)),
        2 => any::<usize>().prop_map(Operation::Desactiver),
        1 => any::<usize>().prop_map(Operation::Reactiver),
        2 => (any::<usize>(), 0u8..20).prop_map(|(i, k)| Operation::JournalTresorerie(i, k)),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(48))]

    /// Pour toute suite de mutations du plan officiel et des journaux par défaut, chacune
    /// retenue seulement si le contrôle croisé l'accepte (comme dans `muter`) :
    /// `reconstituer(aplatir(x)) == x`, pour le plan puis pour les journaux.
    #[test]
    fn reconstituer_aplatir_est_l_identite(ops in proptest::collection::vec(operation(), 0..40)) {
        let mut etat = modele().etat().clone();
        // Comptes ciblés : un échantillon du plan (classes 4 et 5 surtout), pour que les
        // sous-comptes atteignent les collectifs et les comptes de trésorerie.
        for op in ops {
            let cibles: Vec<NumeroCompte> = comptes(&etat.plan, |c| {
                matches!(c.numero().classe(), 4..=6)
            });
            let numero = |i: usize| cibles[i % cibles.len()].clone();
            let mut candidat = etat.clone();
            let resultat: Result<(), String> = match &op {
                Operation::SousCompte(i, s) => NumeroCompte::parse(&format!("{}-{s}", numero(*i)))
                    .map_err(|e| e.to_string())
                    .and_then(|n| candidat.plan.ajouter_sous_compte(n, t("Sous-compte")).map(|_| ()).map_err(|e| e.to_string())),
                Operation::Auxiliaire(i, s) => NumeroCompte::parse(&format!("{}-{s}", numero(*i)))
                    .map_err(|e| e.to_string())
                    .and_then(|n| candidat.plan.ajouter_auxiliaire(n, t("Tiers")).map(|_| ()).map_err(|e| e.to_string())),
                Operation::Renommer(i, s) => candidat.plan.renommer(&numero(*i), t(s)).map_err(|e| e.to_string()),
                Operation::Desactiver(i) => candidat.plan.desactiver(&numero(*i)).map_err(|e| e.to_string()),
                Operation::Reactiver(i) => candidat.plan.reactiver(&numero(*i)).map_err(|e| e.to_string()),
                Operation::JournalTresorerie(i, k) => Journal::nouveau(
                    CodeJournal::parse(&format!("BQ{k}")).unwrap(),
                    t("Banque"),
                    TypeJournal::Tresorerie,
                    Some(numero(*i)),
                    &candidat.plan,
                )
                .and_then(|j| candidat.journaux.ajouter(j))
                .map_err(|e| e.to_string()),
            };
            if resultat.is_ok() && verifier(&candidat).is_ok() {
                etat = candidat;
            }
            prop_assert_eq!(verifier(&etat), Ok(()));
        }
        let plan = reconstituer_plan(aplatir_plan(&etat.plan)).unwrap();
        prop_assert_eq!(&plan, &etat.plan);
        let journaux = reconstituer_journaux(aplatir_journaux(&etat.journaux), &plan);
        prop_assert_eq!(journaux.as_ref(), Ok(&etat.journaux));
    }
}
