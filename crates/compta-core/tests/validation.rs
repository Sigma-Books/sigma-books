//! Validation (spec §6.2, invariants 3 à 7), numérotation à la validation et contre-passation.

// Fichier de test : les `unwrap` y sont voulus (une donnée de test invalide fait échouer le test).
#![allow(clippy::unwrap_used)]
mod commun;

use commun::*;
use compta_core::{
    Avertissement, CodeExercice, CodeJournal, Compteur, Contexte, EcritureBrouillon, ErreurPlan,
    ErreurValidation, NumeroEcriture, Sens, Usage,
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

    d.exercices.cloturer(&code, []).unwrap();
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
    // Invariant 7 assoupli : validée, avec un avertissement (ni Tryton `move.py:449-538` ni
    // Odoo `account_move.py:2863-2893` n'imposent le compte de trésorerie).
    let (_, avertissements) = d
        .valider_le_avec_avertissements(banque_sans_banque, date!(2026 - 03 - 10))
        .unwrap();
    assert_eq!(
        avertissements,
        vec![Avertissement::CompteTresorerieAbsent {
            journal: j("BQ1"),
            compte: n("521"),
        }]
    );
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
        delai_cloture: compta_core::DELAI_CLOTURE_PAR_DEFAUT,
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
fn contre_passation_liee_dans_les_deux_sens_et_repetable() {
    let mut d = Dossier::nouveau();
    let origine = d.valider(achat()).unwrap();
    let cp = d.contre_passer(&origine, date!(2026 - 04 - 02)).unwrap();
    assert_eq!(cp.to_string(), "AC-2026-000002");
    assert_eq!(
        d.livre.contre_passations(&origine),
        std::slice::from_ref(&cp)
    );
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
    // Une origine peut être contre-passée plusieurs fois (ADR 0021 point 4, révisé le
    // 2026-11-25) : chaque contre-passation a son numéro et cite l'origine.
    let seconde = d.contre_passer(&origine, date!(2026 - 04 - 03)).unwrap();
    assert_eq!(seconde.to_string(), "AC-2026-000003");
    assert_eq!(
        d.livre.ecriture(&seconde).unwrap().origine(),
        Some(&origine)
    );
    assert_eq!(d.livre.contre_passations(&origine), [cp, seconde]);
}

/// Une contre-passation peut être datée avant l'origine (Tryton `move.py:399-445`, Odoo
/// `account_move_reversal.py:93-100` : aucune borne) ; elle prend le numéro suivant du journal.
#[test]
fn contre_passation_acceptee_avant_l_origine() {
    let mut d = Dossier::nouveau();
    let origine = d.valider(achat()).unwrap();
    let cp = d.contre_passer(&origine, date!(2026 - 03 - 09)).unwrap();
    assert_eq!(cp.to_string(), "AC-2026-000002");
    let validee = d.livre.ecriture(&cp).unwrap();
    assert_eq!(validee.date(), date!(2026 - 03 - 09));
    assert_eq!(validee.origine(), Some(&origine));
    // Elle relit comme toute contre-passation (contrôles structurels de la reconstitution).
    assert_eq!(d.livre.contre_passations(&origine), [cp]);
}

#[test]
fn contre_passation_refusee_si_l_origine_est_inconnue() {
    let mut d = Dossier::nouveau();
    let origine = d.valider(achat()).unwrap();
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
    assert_eq!(d.livre.contre_passations(&origine), []);
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
    d.exercices
        .cloturer_periode(date!(2026 - 01 - 15), [])
        .unwrap();
    d.exercices
        .cloturer_periode(date!(2026 - 02 - 15), [])
        .unwrap();
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
/// au-delà, la validation reste possible et **avertit** (alerte, pas de blocage : ni Tryton
/// `period.py:387-441` ni Odoo `company.py:76-100` n'ont de délai automatique).
#[test]
fn validation_avertie_apres_l_echeance() {
    let mut d = Dossier::nouveau();
    let (_, avertissements) = d
        .valider_le_avec_avertissements(achat(), date!(2026 - 06 - 30))
        .unwrap();
    assert!(avertissements.is_empty());
    let (_, avertissements) = d
        .valider_le_avec_avertissements(achat(), date!(2026 - 07 - 01))
        .unwrap();
    assert_eq!(
        avertissements,
        vec![Avertissement::EcheanceDeClotureDepassee {
            periode: date!(2026 - 03 - 01),
            echeance: date!(2026 - 06 - 30),
        }]
    );
    // Décembre, dernière période : échéance au 30 avril suivant (art. 23).
    let mut decembre = achat().en_brouillon();
    decembre.changer_date(date!(2026 - 12 - 31));
    let decembre = decembre.valider_equilibre().unwrap();
    let (_, avertissements) = d
        .valider_le_avec_avertissements(decembre.clone(), date!(2027 - 04 - 30))
        .unwrap();
    assert!(avertissements.is_empty());
    let (_, avertissements) = d
        .valider_le_avec_avertissements(decembre, date!(2027 - 05 - 01))
        .unwrap();
    assert!(matches!(
        avertissements.as_slice(),
        [Avertissement::EcheanceDeClotureDepassee { .. }]
    ));
}

/// Le verrouillage reste bloquant, échéance dépassée ou non : seul un humain ferme la période.
#[test]
fn le_verrouillage_bloque_meme_apres_l_echeance() {
    let mut d = Dossier::nouveau();
    d.exercices
        .verrouiller_periode(date!(2026 - 03 - 10))
        .unwrap();
    assert_eq!(
        d.valider_le(achat(), date!(2026 - 07 - 01)),
        Err(ErreurValidation::PeriodeFermee(date!(2026 - 03 - 10)))
    );
}

/// Le délai de clôture est un paramètre du contexte : l'avertissement le suit.
#[test]
fn l_avertissement_suit_le_delai_du_contexte() {
    let d = Dossier::nouveau();
    let compteur = Compteur::nouveau(
        CodeJournal::parse("AC").unwrap(),
        CodeExercice::parse("2026").unwrap(),
    );
    let contexte = Contexte {
        delai_cloture: compta_core::DelaiCloture {
            mois_periode: 1,
            mois_derniere_periode: 6,
        },
        ..d.contexte()
    };
    // Un mois de délai : l'échéance de mars est le 30 avril.
    let v = compta_core::valider(achat(), &contexte, &compteur, date!(2026 - 05 - 01)).unwrap();
    assert_eq!(
        v.avertissements(),
        [Avertissement::EcheanceDeClotureDepassee {
            periode: date!(2026 - 03 - 01),
            echeance: date!(2026 - 04 - 30),
        }]
    );
    // Avec le délai par défaut, le même jour n'avertit pas.
    let v = compta_core::valider(achat(), &d.contexte(), &compteur, date!(2026 - 05 - 01)).unwrap();
    assert!(v.avertissements().is_empty());
}

fn achat_tardif(date: time::Date, date_valeur: time::Date) -> compta_core::EcritureEquilibree {
    let mut b = achat().en_brouillon();
    b.changer_date(date);
    b.mentionner_date_valeur_comptable(Some(date_valeur));
    b.valider_equilibre().unwrap()
}

/// La date de valeur comptable figure distinctement dans les éditions (art. 22 4°).
#[test]
fn date_de_valeur_comptable_editee() {
    let mut d = Dossier::nouveau();
    d.exercices
        .cloturer_periode(date!(2026 - 01 - 10), [])
        .unwrap();
    d.valider(achat_tardif(date!(2026 - 02 - 01), date!(2026 - 01 - 31)))
        .unwrap();
    let exercice = d
        .exercices
        .exercice(&CodeExercice::parse("2026").unwrap())
        .unwrap();
    let filtre =
        compta_core::Filtre::exercice(exercice, &d.journaux, date!(2026 - 12 - 31)).unwrap();
    let journaux = compta_core::journaux(
        d.livre.ecritures(),
        &[],
        compta_core::Mode::Definitif,
        &filtre,
    )
    .unwrap();
    assert_eq!(
        journaux[0].ecritures[0].date_valeur_comptable,
        Some(date!(2026 - 01 - 31))
    );
    let grand_livre = compta_core::grand_livre(
        d.livre.ecritures(),
        &[],
        compta_core::Mode::Definitif,
        &filtre,
    )
    .unwrap();
    assert!(
        grand_livre
            .iter()
            .flat_map(|c| c.lignes.iter())
            .all(|l| l.date_valeur_comptable == Some(date!(2026 - 01 - 31)))
    );
}

/// AUDCIF art. 22 4° (décision E a) : date de valeur dans une période **clôturée** ⇒
/// l'opération est datée du premier jour de la première période non clôturée ; sa date de
/// valeur comptable est conservée.
#[test]
fn operation_tardive_reportee_apres_une_periode_cloturee() {
    let mut d = Dossier::nouveau();
    d.exercices
        .cloturer_periode(date!(2026 - 01 - 10), [])
        .unwrap();
    d.exercices
        .cloturer_periode(date!(2026 - 02 - 10), [])
        .unwrap();
    for date_valeur in [date!(2026 - 01 - 20), date!(2026 - 02 - 25)] {
        assert_eq!(
            d.valider(achat_tardif(date!(2026 - 03 - 05), date_valeur)),
            Err(ErreurValidation::DateComptableTardive {
                date: date!(2026 - 03 - 05),
                attendue: Some(date!(2026 - 03 - 01)),
            })
        );
        let numero = d
            .valider(achat_tardif(date!(2026 - 03 - 01), date_valeur))
            .unwrap();
        let e = d.livre.ecriture(&numero).unwrap();
        assert_eq!(e.date(), date!(2026 - 03 - 01));
        assert_eq!(e.date_valeur_comptable(), Some(date_valeur));
    }
    // La date de valeur précède toujours la date comptable.
    assert_eq!(
        d.valider(achat_tardif(date!(2026 - 03 - 01), date!(2026 - 03 - 01))),
        Err(ErreurValidation::DateValeurNonAnterieure {
            date_valeur: date!(2026 - 03 - 01),
            date: date!(2026 - 03 - 01),
        })
    );
}

/// Une période seulement verrouillée (réversible) n'est pas clôturée : pas de report, il faut
/// la déverrouiller et dater l'écriture de sa date de valeur (AUDCIF art. 16 al. 3, 22 4°).
/// Même chose pour une date de valeur dans une période ouverte.
#[test]
fn pas_de_report_hors_periode_cloturee() {
    let mut d = Dossier::nouveau();
    d.exercices
        .verrouiller_periode(date!(2026 - 02 - 10))
        .unwrap();
    assert_eq!(
        d.valider(achat_tardif(date!(2026 - 03 - 01), date!(2026 - 02 - 20))),
        Err(ErreurValidation::DateValeurDansPeriodeVerrouillee(date!(
            2026 - 02 - 20
        )))
    );
    assert!(
        ErreurValidation::DateValeurDansPeriodeVerrouillee(date!(2026 - 02 - 20))
            .to_string()
            .contains("déverrouillez-la")
    );
    assert_eq!(
        d.valider(achat_tardif(date!(2026 - 03 - 20), date!(2026 - 03 - 02))),
        Err(ErreurValidation::DateValeurDansPeriodeOuverte(date!(
            2026 - 03 - 02
        )))
    );
    // Après déverrouillage motivé, l'écriture est datée de sa date de valeur, sans mention.
    d.exercices
        .deverrouiller_periode(date!(2026 - 02 - 10), t("Facture reçue en retard"))
        .unwrap();
    let mut b = achat().en_brouillon();
    b.changer_date(date!(2026 - 02 - 20));
    assert!(d.valider(b.valider_equilibre().unwrap()).is_ok());
}

/// La période d'accueil (première non clôturée) est verrouillée : refus, il faut la
/// déverrouiller, jamais sauter au mois suivant.
#[test]
fn periode_d_accueil_verrouillee() {
    let mut d = Dossier::nouveau();
    d.exercices
        .cloturer_periode(date!(2026 - 01 - 10), [])
        .unwrap();
    d.exercices
        .verrouiller_periode(date!(2026 - 02 - 10))
        .unwrap();
    assert_eq!(
        d.valider(achat_tardif(date!(2026 - 03 - 01), date!(2026 - 01 - 20))),
        Err(ErreurValidation::PeriodeAccueilVerrouillee(date!(
            2026 - 02 - 01
        )))
    );
    d.exercices
        .deverrouiller_periode(date!(2026 - 02 - 10), t("Opération tardive"))
        .unwrap();
    assert!(
        d.valider(achat_tardif(date!(2026 - 02 - 01), date!(2026 - 01 - 20)))
            .is_ok()
    );
}

/// Date de valeur dans une période ouverte dont l'échéance de clôture est passée : la période
/// n'est pas clôturée, donc pas de report (l'échéance alerte, elle ne bloque pas) ; l'écriture
/// doit être datée de ce jour.
#[test]
fn date_de_valeur_dans_une_periode_echue() {
    let mut d = Dossier::nouveau();
    assert_eq!(
        d.valider_le(
            achat_tardif(date!(2026 - 04 - 01), date!(2026 - 03 - 10)),
            date!(2026 - 07 - 02)
        ),
        Err(ErreurValidation::DateValeurDansPeriodeOuverte(date!(
            2026 - 03 - 10
        )))
    );
    // Janvier à mars clôturés : l'opération va au 1er avril.
    for jour in [
        date!(2026 - 01 - 10),
        date!(2026 - 02 - 10),
        date!(2026 - 03 - 10),
    ] {
        d.exercices.cloturer_periode(jour, []).unwrap();
    }
    assert!(
        d.valider_le(
            achat_tardif(date!(2026 - 04 - 01), date!(2026 - 03 - 10)),
            date!(2026 - 07 - 02)
        )
        .is_ok()
    );
}

/// Exercices : exercice clôturé (art. 61 ou 20) et exercice en clôture qui n'admet pas le
/// journal (décision F a) ⇒ report au premier jour non clôturé qui admet le journal ; un
/// journal admis en clôture (OD) reste dans l'exercice en clôture. Date antérieure au
/// dossier : premier jour du dossier.
#[test]
fn operation_tardive_et_exercices() {
    // Exercice en clôture : l'achat va au 1er janvier 2027.
    let mut d = Dossier::nouveau();
    let code = CodeExercice::parse("2026").unwrap();
    d.exercices.passer_en_cloture(&code).unwrap();
    assert!(
        d.valider(achat_tardif(date!(2027 - 01 - 01), date!(2026 - 11 - 15)))
            .is_ok()
    );
    // OD admis en clôture : une date de valeur de novembre 2026 (ouvert) n'est pas reportée.
    let mut od = achat().en_brouillon();
    od.changer_journal(j("OD"));
    od.changer_date(date!(2026 - 12 - 01));
    od.mentionner_date_valeur_comptable(Some(date!(2026 - 11 - 15)));
    assert_eq!(
        d.valider(od.clone().valider_equilibre().unwrap()),
        Err(ErreurValidation::DateValeurDansPeriodeOuverte(date!(
            2026 - 11 - 15
        )))
    );
    // Date antérieure au dossier : premier jour du dossier pour OD.
    od.changer_date(date!(2026 - 01 - 01));
    od.mentionner_date_valeur_comptable(Some(date!(2025 - 12 - 20)));
    assert!(d.valider(od.valider_equilibre().unwrap()).is_ok());

    // Exercice clôturé : l'achat va au premier jour de l'exercice suivant.
    let mut d = Dossier::nouveau();
    d.exercices.passer_en_cloture(&code).unwrap();
    d.exercices.cloturer(&code, []).unwrap();
    assert!(
        d.valider(achat_tardif(date!(2027 - 01 - 01), date!(2026 - 06 - 10)))
            .is_ok()
    );
}

/// La contre-passation d'une opération tardive est une nouvelle opération, datée dans une
/// période ouverte : elle ne reprend pas la date de valeur comptable.
#[test]
fn contre_passation_d_une_operation_tardive() {
    let mut d = Dossier::nouveau();
    d.exercices
        .cloturer_periode(date!(2026 - 01 - 10), [])
        .unwrap();
    let origine = d
        .valider(achat_tardif(date!(2026 - 02 - 01), date!(2026 - 01 - 25)))
        .unwrap();
    let miroir = d.contre_passer(&origine, date!(2026 - 02 - 05)).unwrap();
    let miroir = d.livre.ecriture(&miroir).unwrap();
    assert_eq!(miroir.date_valeur_comptable(), None);
    assert_eq!(miroir.origine(), Some(&origine));
}

#[test]
fn message_de_l_operation_tardive() {
    let avec = ErreurValidation::DateComptableTardive {
        date: date!(2026 - 03 - 05),
        attendue: Some(date!(2026 - 03 - 01)),
    };
    assert!(
        avec.to_string()
            .contains("datez-la du 2026-03-01, premier jour de la première période non clôturée"),
        "{avec}"
    );
    let sans = ErreurValidation::DateComptableTardive {
        date: date!(2026 - 03 - 05),
        attendue: None,
    };
    assert!(
        sans.to_string().contains("aucune période non clôturée"),
        "{sans}"
    );
    assert!(!sans.to_string().contains("None"));
}
