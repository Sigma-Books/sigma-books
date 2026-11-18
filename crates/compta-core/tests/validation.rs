//! Validation (spec §6.2, invariants 3 à 7), numérotation à la validation et contre-passation.

// Fichier de test : les `unwrap` y sont voulus (une donnée de test invalide fait échouer le test).
#![allow(clippy::unwrap_used)]
mod commun;

use commun::*;
use compta_core::{
    CodeExercice, Compteur, EcritureBrouillon, ErreurPlan, ErreurValidation, NumeroEcriture, Sens,
    Usage,
};
use time::macros::date;

fn achat() -> compta_core::EcritureEquilibree {
    equilibree(
        "AC",
        date!(2026 - 03 - 10),
        "Facture F-001",
        vec![
            ligne("6052", Sens::Debit, "118000"),
            ligne_tiers("4011", Sens::Credit, "118000", "FOURNISSEUR-1"),
        ],
    )
}

#[test]
fn validation_attribue_un_numero_continu() {
    let mut d = Dossier::nouveau();
    let a = d.valider(achat()).unwrap();
    let b = d.valider(achat()).unwrap();
    assert_eq!(a.to_string(), "AC-2026-000001");
    assert_eq!(b.to_string(), "AC-2026-000002");
    let e = d.livre.ecriture(&a).unwrap();
    assert_eq!(e.date(), date!(2026 - 03 - 10));
    assert_eq!(e.date_validation(), date!(2026 - 03 - 10));
    assert_eq!(e.libelle().as_str(), "Facture F-001");
    assert_eq!(e.total().to_string(), "118000");
    assert!(e.origine().is_none());
    assert_eq!(d.livre.ecritures().len(), 2);
}

#[test]
fn la_numerotation_suit_l_exercice_de_la_date() {
    let mut d = Dossier::nouveau();
    let mut e = achat().en_brouillon();
    e.changer_date(date!(2027 - 01 - 05));
    let numero = d.valider(e.valider_equilibre().unwrap()).unwrap();
    assert_eq!(numero.to_string(), "AC-2027-000001");
}

#[test]
fn invariant_3_compte_inconnu_inactif_ou_de_regroupement() {
    let mut d = Dossier::nouveau();
    let inconnu = equilibree(
        "OD",
        date!(2026 - 03 - 10),
        "x",
        vec![
            ligne("6052", Sens::Debit, "1"),
            ligne("7019", Sens::Credit, "1"),
        ],
    );
    assert!(matches!(
        d.valider(inconnu),
        Err(ErreurValidation::Compte {
            ligne: 1,
            erreur: ErreurPlan::Inconnu(_)
        })
    ));
    let regroupement = equilibree(
        "OD",
        date!(2026 - 03 - 10),
        "x",
        vec![
            ligne("605", Sens::Debit, "1"),
            ligne("7011", Sens::Credit, "1"),
        ],
    );
    assert!(matches!(
        d.valider(regroupement),
        Err(ErreurValidation::Compte {
            ligne: 0,
            erreur: ErreurPlan::Regroupement(_)
        })
    ));
    d.plan.desactiver(&n("7011")).unwrap();
    let inactif = equilibree(
        "OD",
        date!(2026 - 03 - 10),
        "x",
        vec![
            ligne("6052", Sens::Debit, "1"),
            ligne("7011", Sens::Credit, "1"),
        ],
    );
    assert!(matches!(
        d.valider(inactif),
        Err(ErreurValidation::Compte {
            ligne: 1,
            erreur: ErreurPlan::Inactif(_)
        })
    ));
    assert!(
        d.livre.ecritures().is_empty(),
        "rien n'est inscrit après un refus"
    );
}

#[test]
fn invariant_4_tiers_obligatoire_sur_compte_collectif() {
    let mut d = Dossier::nouveau();
    let sans_tiers = equilibree(
        "AC",
        date!(2026 - 03 - 10),
        "x",
        vec![
            ligne("6052", Sens::Debit, "10"),
            ligne("4011", Sens::Credit, "10"),
        ],
    );
    assert!(matches!(
        d.valider(sans_tiers),
        Err(ErreurValidation::TiersManquant { ligne: 1, .. })
    ));
}

#[test]
fn invariant_5_date_periode_et_exercice() {
    let mut d = Dossier::nouveau();
    let mut hors = achat().en_brouillon();
    hors.changer_date(date!(2025 - 12 - 31));
    assert_eq!(
        d.valider(hors.valider_equilibre().unwrap()),
        Err(ErreurValidation::DateHorsExercice(date!(2025 - 12 - 31)))
    );

    d.exercices
        .verrouiller_periode(date!(2026 - 03 - 01))
        .unwrap();
    assert_eq!(
        d.valider(achat()),
        Err(ErreurValidation::PeriodeFermee(date!(2026 - 03 - 10)))
    );

    // Exercice en clôture : OD et IN admis ; achats et paie (PA, pourtant d'opérations
    // diverses) refusés.
    let mut d = Dossier::nouveau();
    let code = CodeExercice::parse("2026").unwrap();
    d.exercices.passer_en_cloture(&code).unwrap();
    assert_eq!(
        d.valider(achat()),
        Err(ErreurValidation::ExerciceEnCloture(code.clone()))
    );
    let paie = equilibree(
        "PA",
        date!(2026 - 12 - 31),
        "Salaires de décembre",
        vec![
            ligne("6052", Sens::Debit, "50"),
            ligne("521", Sens::Credit, "50"),
        ],
    );
    assert_eq!(
        d.valider(paie),
        Err(ErreurValidation::ExerciceEnCloture(code.clone()))
    );
    let inventaire = equilibree(
        "IN",
        date!(2026 - 12 - 31),
        "Stock final",
        vec![
            ligne("6052", Sens::Credit, "50"),
            ligne("7011", Sens::Debit, "50"),
        ],
    );
    assert!(d.valider(inventaire).is_ok());

    d.exercices.cloturer(&code).unwrap();
    let od = equilibree(
        "OD",
        date!(2026 - 12 - 31),
        "x",
        vec![
            ligne("6052", Sens::Debit, "1"),
            ligne("7011", Sens::Credit, "1"),
        ],
    );
    assert_eq!(d.valider(od), Err(ErreurValidation::ExerciceCloture(code)));
}

#[test]
fn invariant_6_piece_obligatoire() {
    // AUDCIF art. 17 et 22 : aucune écriture sans pièce, quel que soit le journal.
    let mut d = Dossier::nouveau();
    for journal in ["OD", "IN", "AC"] {
        let mut sans_justification =
            EcritureBrouillon::nouvelle(j(journal), date!(2026 - 03 - 10), t("x"));
        sans_justification.ajouter_ligne(ligne("6052", Sens::Debit, "1"));
        sans_justification.ajouter_ligne(ligne("7011", Sens::Credit, "1"));
        assert_eq!(
            d.valider(sans_justification.clone().valider_equilibre().unwrap()),
            Err(ErreurValidation::JustificationManquante),
            "{journal}"
        );
        sans_justification.justifier(piece());
        assert!(
            d.valider(sans_justification.valider_equilibre().unwrap())
                .is_ok()
        );
    }
}

#[test]
fn invariant_7_journal_inconnu_a_nouveaux_et_tresorerie() {
    let mut d = Dossier::nouveau();
    let inconnu = equilibree(
        "ZZ",
        date!(2026 - 03 - 10),
        "x",
        vec![
            ligne("6052", Sens::Debit, "1"),
            ligne("7011", Sens::Credit, "1"),
        ],
    );
    assert_eq!(
        d.valider(inconnu),
        Err(ErreurValidation::JournalInconnu(j("ZZ")))
    );
    let a_nouveaux = equilibree(
        "AN",
        date!(2026 - 01 - 01),
        "x",
        vec![
            ligne("521", Sens::Debit, "1"),
            ligne("7011", Sens::Credit, "1"),
        ],
    );
    assert_eq!(
        d.valider(a_nouveaux),
        Err(ErreurValidation::JournalSansSaisie(j("AN")))
    );
    let banque_sans_banque = equilibree(
        "BQ1",
        date!(2026 - 03 - 10),
        "x",
        vec![
            ligne("571", Sens::Debit, "1"),
            ligne("7011", Sens::Credit, "1"),
        ],
    );
    assert!(matches!(
        d.valider(banque_sans_banque),
        Err(ErreurValidation::CompteTresorerieAbsent { .. })
    ));
    let banque = equilibree(
        "BQ1",
        date!(2026 - 03 - 10),
        "Règlement",
        vec![
            ligne_tiers("4011", Sens::Debit, "118000", "FOURNISSEUR-1"),
            ligne("521", Sens::Credit, "118000"),
        ],
    );
    assert!(d.valider(banque).is_ok());
}

#[test]
fn compteur_inadapte_ou_desynchronise() {
    let mut d = Dossier::nouveau();
    let contexte_compteur = |journal: &str, exercice: &str| {
        Compteur::nouveau(j(journal), CodeExercice::parse(exercice).unwrap())
    };
    let ctx_plan = d.plan.clone();
    let ctx_journaux = d.journaux.clone();
    let ctx_exercices = d.exercices.clone();
    let contexte = compta_core::Contexte {
        plan: &ctx_plan,
        journaux: &ctx_journaux,
        exercices: &ctx_exercices,
    };
    let refus = d
        .livre
        .valider(
            achat(),
            &contexte,
            &contexte_compteur("VT", "2026"),
            date!(2026 - 03 - 10),
        )
        .unwrap_err();
    assert!(matches!(
        refus.erreur(),
        ErreurValidation::CompteurInadapte { .. }
    ));
    let refus = d
        .livre
        .valider(
            achat(),
            &contexte,
            &contexte_compteur("AC", "2027"),
            date!(2026 - 03 - 10),
        )
        .unwrap_err();
    assert!(matches!(
        refus.erreur(),
        ErreurValidation::CompteurInadapte { .. }
    ));
    // Un compteur périmé (rejoué après une validation) est refusé : pas de doublon.
    let neuf = contexte_compteur("AC", "2026");
    let (_, suivant) = d
        .livre
        .valider(achat(), &contexte, &neuf, date!(2026 - 03 - 10))
        .unwrap();
    let refus = d
        .livre
        .valider(achat(), &contexte, &neuf, date!(2026 - 03 - 10))
        .unwrap_err();
    assert_eq!(
        refus.erreur(),
        &ErreurValidation::CompteurDesynchronise {
            fournie: 0,
            attendue: 1
        }
    );
    assert_eq!(
        refus.ecriture(),
        &achat(),
        "l'écriture refusée est rendue intacte"
    );
    assert!(
        d.livre
            .valider(achat(), &contexte, &suivant, date!(2026 - 03 - 10))
            .is_ok()
    );
}

#[test]
fn contre_passation_liee_dans_les_deux_sens_et_unique() {
    let mut d = Dossier::nouveau();
    let origine = d.valider(achat()).unwrap();
    let cp = d.contre_passer(&origine, date!(2026 - 04 - 02)).unwrap();
    assert_eq!(cp.to_string(), "AC-2026-000002");
    assert_eq!(d.livre.contre_passee_par(&origine), Some(&cp));
    let miroir = d.livre.ecriture(&cp).unwrap();
    assert_eq!(miroir.origine(), Some(&origine));
    let source = d.livre.ecriture(&origine).unwrap();
    for (a, b) in source.lignes().iter().zip(miroir.lignes()) {
        assert_eq!(a.compte(), b.compte());
        assert_eq!(a.montant(), b.montant());
        assert_eq!(a.tiers(), b.tiers());
        assert_eq!(a.sens(), b.sens().inverse());
    }
    assert_eq!(miroir.justification(), source.justification());
    // L'origine reste validée et inchangée.
    assert_eq!(source.lignes()[0].sens(), Sens::Debit);
    assert!(matches!(
        d.contre_passer(&origine, date!(2026 - 04 - 03)),
        Err(ErreurValidation::DejaContrePassee { .. })
    ));
}

#[test]
fn contre_passation_refusee_avant_l_origine_ou_inconnue() {
    let mut d = Dossier::nouveau();
    let origine = d.valider(achat()).unwrap();
    assert!(matches!(
        d.contre_passer(&origine, date!(2026 - 03 - 09)),
        Err(ErreurValidation::DateAnterieureAOrigine { .. })
    ));
    let inconnue: NumeroEcriture = "AC-2026-000099".parse().unwrap();
    assert_eq!(
        d.contre_passer(&inconnue, date!(2026 - 04 - 01)),
        Err(ErreurValidation::EcritureInconnue(inconnue))
    );
    // Contre-passation dans une période verrouillée : refusée comme toute validation.
    d.exercices
        .verrouiller_periode(date!(2026 - 05 - 01))
        .unwrap();
    assert_eq!(
        d.contre_passer(&origine, date!(2026 - 05 - 15)),
        Err(ErreurValidation::PeriodeFermee(date!(2026 - 05 - 15)))
    );
    assert!(d.livre.contre_passee_par(&origine).is_none());
}

#[test]
fn contre_passation_sur_l_exercice_suivant() {
    let mut d = Dossier::nouveau();
    let origine = d.valider(achat()).unwrap();
    let cp = d.contre_passer(&origine, date!(2027 - 01 - 02)).unwrap();
    assert_eq!(cp.to_string(), "AC-2027-000001");
}

#[test]
fn usage_des_comptes_et_journaux() {
    let mut d = Dossier::nouveau();
    assert_eq!(d.livre.usage_compte(&n("6052")), Usage::SansEcriture);
    d.valider(achat()).unwrap();
    assert_eq!(d.livre.usage_compte(&n("6052")), Usage::Mouvemente);
    assert_eq!(d.livre.usage_compte(&n("7011")), Usage::SansEcriture);
    assert_eq!(d.livre.usage_journal(&j("AC")), Usage::Mouvemente);
    assert_eq!(d.livre.usage_journal(&j("VT")), Usage::SansEcriture);
    // Le plan refuse alors la suppression d'un compte mouvementé (auxiliaire ici).
    d.plan
        .ajouter_auxiliaire(n("4111-C1"), t("Client 1"))
        .unwrap();
    let vente = equilibree(
        "VT",
        date!(2026 - 03 - 11),
        "Facture V-001",
        vec![
            ligne_tiers("4111-C1", Sens::Debit, "500", "CLIENT-1"),
            ligne("7011", Sens::Credit, "500"),
        ],
    );
    d.valider(vente).unwrap();
    let usage = d.livre.usage_compte(&n("4111-C1"));
    assert_eq!(
        d.plan.supprimer(&n("4111-C1"), usage),
        Err(ErreurPlan::CompteMouvemente(n("4111-C1")))
    );
}

/// AUDCIF art. 22 3° et 4° : l'ordre des dates n'est figé qu'à la clôture informatique. Dans
/// une période ouverte, une écriture peut précéder une écriture déjà validée (décision du
/// mainteneur du 2026-11-18) ; une période clôturée n'accepte plus rien.
#[test]
fn chronologie_figee_par_la_cloture() {
    let mut d = Dossier::nouveau();
    d.valider(achat()).unwrap(); // AC, 10 mars
    let mut avant = achat().en_brouillon();
    avant.changer_date(date!(2026 - 03 - 09));
    assert!(d.valider(avant.valider_equilibre().unwrap()).is_ok());
    // Clôture de janvier et février : février n'accepte plus rien, mars reste ouvert.
    d.exercices.cloturer_periode(date!(2026 - 01 - 15)).unwrap();
    d.exercices.cloturer_periode(date!(2026 - 02 - 15)).unwrap();
    let mut fevrier = achat().en_brouillon();
    fevrier.changer_date(date!(2026 - 02 - 27));
    assert_eq!(
        d.valider(fevrier.valider_equilibre().unwrap()),
        Err(ErreurValidation::PeriodeFermee(date!(2026 - 02 - 27)))
    );
    // Une contre-passation ne peut pas viser une période clôturée.
    let origine = d.valider(achat()).unwrap();
    assert!(d.contre_passer(&origine, date!(2026 - 02 - 28)).is_err());
    assert!(d.contre_passer(&origine, date!(2026 - 03 - 10)).is_ok());
}

/// Échéance de la clôture informatique (art. 22 3°) : fin du 3e mois qui suit la période ;
/// au-delà, plus aucune validation dans la période.
#[test]
fn validation_refusee_apres_l_echeance() {
    let mut d = Dossier::nouveau();
    assert!(d.valider_le(achat(), date!(2026 - 06 - 30)).is_ok());
    assert_eq!(
        d.valider_le(achat(), date!(2026 - 07 - 01)),
        Err(ErreurValidation::EcheanceDepassee {
            periode: date!(2026 - 03 - 01),
            echeance: date!(2026 - 06 - 30),
        })
    );
    // Décembre, dernière période : échéance au 30 avril suivant (art. 23).
    let mut decembre = achat().en_brouillon();
    decembre.changer_date(date!(2026 - 12 - 31));
    let decembre = decembre.valider_equilibre().unwrap();
    assert!(
        d.valider_le(decembre.clone(), date!(2027 - 04 - 30))
            .is_ok()
    );
    assert!(matches!(
        d.valider_le(decembre, date!(2027 - 05 - 01)),
        Err(ErreurValidation::EcheanceDepassee { .. })
    ));
}
