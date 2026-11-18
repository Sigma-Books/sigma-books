//! Validation et contre-passation sans livre en mémoire (lot L2, PR 3, issue L2-08) :
//! fonctions libres [`compta_core::valider`] et [`compta_core::contre_passer`], et
//! reconstitution d'une écriture relue de la base ([`EcritureValidee::reconstituer`]).
//!
//! Propriétés (`docs/lots/L2.md`, « Tests exigés ») :
//! - sur toute suite d'opérations (validations, contre-passations, refus compris, changements
//!   d'état des périodes et des exercices), les fonctions libres, avec compteurs et liens de
//!   contre-passation tenus à part comme le ferait la base, donnent exactement le résultat du
//!   [`Livre`] : mêmes écritures validées, mêmes refus, mêmes compteurs, mêmes liens ;
//! - une écriture validée, relue champ par champ et reconstituée, est égale à l'originale.

// Fichier de test : les `unwrap` y sont voulus (une donnée de test invalide fait échouer le test).
#![allow(clippy::unwrap_used)]
mod commun;

use std::collections::BTreeMap;

use commun::*;
use compta_core::{
    CodeExercice, CodeJournal, Compteur, EcritureBrouillon, EcritureEquilibree, EcritureValidee,
    ErreurReconstitution, ErreurValidation, Exercices, IdTiers, Livre, NumeroEcriture, Sens,
    VersionParametres, contre_passer, valider,
};
use proptest::prelude::*;
use rust_decimal::Decimal;
use time::macros::date;
use time::{Date, Duration};

fn achat() -> EcritureEquilibree {
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

fn exercice(code: &str) -> CodeExercice {
    CodeExercice::parse(code).unwrap()
}

/// Écriture équilibrée rebâtie à partir des champs d'une écriture validée, comme l'API la
/// relira de la base.
fn rebatir(e: &EcritureValidee) -> EcritureEquilibree {
    let mut b = EcritureBrouillon::nouvelle(e.journal().clone(), e.date(), e.libelle().clone());
    for l in e.lignes() {
        b.ajouter_ligne(l.clone());
    }
    b.justifier(e.justification().clone());
    if let Some(version) = e.version_parametres() {
        b.utiliser_parametres(version.clone());
    }
    b.mentionner_date_valeur_comptable(e.date_valeur_comptable());
    b.valider_equilibre().unwrap()
}

fn reconstituer(e: &EcritureValidee) -> Result<EcritureValidee, ErreurReconstitution> {
    EcritureValidee::reconstituer(
        rebatir(e),
        e.numero().clone(),
        e.date_validation(),
        e.origine().cloned(),
    )
}

// ---------------------------------------------------------------------------------------
// Fonctions libres
// ---------------------------------------------------------------------------------------

#[test]
fn valider_rend_l_ecriture_validee_et_le_compteur_suivant() {
    let d = Dossier::nouveau();
    let compteur = Compteur::nouveau(j("AC"), exercice("2026"));
    let (validee, suivant) =
        valider(achat(), &d.contexte(), &compteur, date!(2026 - 03 - 12)).unwrap();
    assert_eq!(validee.numero().to_string(), "AC-2026-000001");
    assert_eq!(suivant.dernier(), 1);
    assert_eq!(
        compteur.dernier(),
        0,
        "le compteur fourni n'est pas modifié"
    );
    assert_eq!(validee.date(), date!(2026 - 03 - 10));
    assert_eq!(validee.date_validation(), date!(2026 - 03 - 12));
    assert_eq!(validee.libelle().as_str(), "Facture F-001");
    assert_eq!(validee.lignes(), achat().lignes());
    assert_eq!(validee.justification(), &piece());
    assert!(validee.origine().is_none());
}

/// En base, le compteur lu `FOR UPDATE` fait foi : pas de contrôle de désynchronisation.
#[test]
fn le_compteur_fourni_fait_foi() {
    let d = Dossier::nouveau();
    let mut a_41 = Compteur::nouveau(j("AC"), exercice("2026"));
    for _ in 0..41 {
        a_41 = a_41.suivant().unwrap().1;
    }
    let (validee, suivant) = valider(achat(), &d.contexte(), &a_41, date!(2026 - 03 - 10)).unwrap();
    assert_eq!(validee.numero().to_string(), "AC-2026-000042");
    assert_eq!(suivant.dernier(), 42);
}

#[test]
fn valider_refuse_et_rend_l_ecriture_intacte() {
    let mut d = Dossier::nouveau();
    let refus = valider(
        achat(),
        &d.contexte(),
        &Compteur::nouveau(j("VT"), exercice("2026")),
        date!(2026 - 03 - 10),
    )
    .unwrap_err();
    assert!(matches!(
        refus.erreur(),
        ErreurValidation::CompteurInadapte { .. }
    ));
    assert_eq!(refus.ecriture(), &achat());
    let refus = valider(
        achat(),
        &d.contexte(),
        &Compteur::nouveau(j("AC"), exercice("2027")),
        date!(2026 - 03 - 10),
    )
    .unwrap_err();
    assert!(matches!(
        refus.erreur(),
        ErreurValidation::CompteurInadapte { .. }
    ));
    d.exercices
        .verrouiller_periode(date!(2026 - 03 - 01))
        .unwrap();
    let refus = valider(
        achat(),
        &d.contexte(),
        &Compteur::nouveau(j("AC"), exercice("2026")),
        date!(2026 - 03 - 10),
    )
    .unwrap_err();
    assert_eq!(
        refus.erreur(),
        &ErreurValidation::PeriodeFermee(date!(2026 - 03 - 10))
    );
    assert_eq!(refus.decomposer().0, achat());
}

#[test]
fn contre_passer_a_partir_de_l_origine_fournie() {
    let d = Dossier::nouveau();
    let mut b = achat().en_brouillon();
    b.utiliser_parametres(VersionParametres::parse("sn-2026.1").unwrap());
    let compteur = Compteur::nouveau(j("AC"), exercice("2026"));
    let (origine, compteur) = valider(
        b.valider_equilibre().unwrap(),
        &d.contexte(),
        &compteur,
        date!(2026 - 03 - 10),
    )
    .unwrap();
    let (miroir, suivant) = contre_passer(
        &origine,
        None,
        date!(2026 - 04 - 02),
        t("Annulation F-001"),
        &d.contexte(),
        &compteur,
        date!(2026 - 04 - 03),
    )
    .unwrap();
    assert_eq!(miroir.numero().to_string(), "AC-2026-000002");
    assert_eq!(suivant.dernier(), 2);
    assert_eq!(miroir.origine(), Some(origine.numero()));
    assert_eq!(miroir.date(), date!(2026 - 04 - 02));
    assert_eq!(miroir.date_validation(), date!(2026 - 04 - 03));
    assert_eq!(miroir.libelle().as_str(), "Annulation F-001");
    assert_eq!(miroir.justification(), origine.justification());
    assert_eq!(miroir.version_parametres(), origine.version_parametres());
    for (a, m) in origine.lignes().iter().zip(miroir.lignes()) {
        assert_eq!(a.compte(), m.compte());
        assert_eq!(a.montant(), m.montant());
        assert_eq!(a.tiers(), m.tiers());
        assert_eq!(a.sens(), m.sens().inverse());
    }
    assert_eq!(miroir.total(), origine.total());

    // Déjà contre-passée : le fait fourni par l'appelant suffit.
    assert_eq!(
        contre_passer(
            &origine,
            Some(miroir.numero()),
            date!(2026 - 04 - 02),
            t("Encore"),
            &d.contexte(),
            &suivant,
            date!(2026 - 04 - 03),
        ),
        Err(ErreurValidation::DejaContrePassee {
            origine: origine.numero().clone(),
            par: miroir.numero().clone(),
        })
    );
    // Datée avant l'origine.
    assert_eq!(
        contre_passer(
            &origine,
            None,
            date!(2026 - 03 - 09),
            t("Trop tôt"),
            &d.contexte(),
            &suivant,
            date!(2026 - 04 - 03),
        ),
        Err(ErreurValidation::DateAnterieureAOrigine {
            date: date!(2026 - 03 - 09),
            origine: date!(2026 - 03 - 10),
        })
    );
    // Compteur de l'exercice suivant pour une contre-passation de l'exercice en cours.
    assert!(matches!(
        contre_passer(
            &origine,
            None,
            date!(2026 - 04 - 02),
            t("x"),
            &d.contexte(),
            &Compteur::nouveau(j("AC"), exercice("2027")),
            date!(2026 - 04 - 03),
        ),
        Err(ErreurValidation::CompteurInadapte { .. })
    ));
}

// ---------------------------------------------------------------------------------------
// Reconstitution
// ---------------------------------------------------------------------------------------

/// Écritures validées variées : achat, opération tardive avec version de paramètres,
/// contre-passation.
fn ecritures_variees() -> (Dossier, Vec<NumeroEcriture>) {
    let mut d = Dossier::nouveau();
    let achat = d.valider(achat()).unwrap();
    d.exercices.cloturer_periode(date!(2026 - 01 - 10)).unwrap();
    let mut tardive = equilibree(
        "OD",
        date!(2026 - 02 - 01),
        "Frais de janvier",
        vec![
            ligne("6052", Sens::Debit, "1500"),
            ligne("521", Sens::Credit, "1500"),
        ],
    )
    .en_brouillon();
    tardive.mentionner_date_valeur_comptable(Some(date!(2026 - 01 - 28)));
    tardive.utiliser_parametres(VersionParametres::parse("sn-2026.1").unwrap());
    let tardive = d.valider(tardive.valider_equilibre().unwrap()).unwrap();
    let cp = d.contre_passer(&achat, date!(2026 - 03 - 20)).unwrap();
    (d, vec![achat, tardive, cp])
}

#[test]
fn reconstituer_redonne_l_ecriture_validee() {
    let (d, numeros) = ecritures_variees();
    for numero in &numeros {
        let e = d.livre.ecriture(numero).unwrap();
        assert_eq!(&reconstituer(e).unwrap(), e, "{numero}");
    }
    let tardive = d.livre.ecriture(&numeros[1]).unwrap();
    let relue = reconstituer(tardive).unwrap();
    assert_eq!(relue.date_valeur_comptable(), Some(date!(2026 - 01 - 28)));
    assert_eq!(relue.version_parametres().unwrap().as_str(), "sn-2026.1");
    let cp = reconstituer(d.livre.ecriture(&numeros[2]).unwrap()).unwrap();
    assert_eq!(cp.origine(), Some(&numeros[0]));
}

/// Les contrôles de contexte ne sont pas rejoués : la période peut avoir été clôturée, le
/// compte désactivé depuis la validation.
#[test]
fn reconstituer_ne_rejoue_pas_le_contexte() {
    let (mut d, numeros) = ecritures_variees();
    d.exercices.cloturer_periode(date!(2026 - 02 - 10)).unwrap();
    d.exercices.cloturer_periode(date!(2026 - 03 - 10)).unwrap();
    d.plan.desactiver(&n("6052")).unwrap();
    for numero in &numeros {
        let e = d.livre.ecriture(numero).unwrap();
        assert_eq!(&reconstituer(e).unwrap(), e);
    }
}

#[test]
fn reconstituer_refuse_une_forme_impossible() {
    let (d, numeros) = ecritures_variees();
    let achat = d.livre.ecriture(&numeros[0]).unwrap();
    let numero = achat.numero().clone();
    let jour = achat.date_validation();

    let autre_journal: NumeroEcriture = "VT-2026-000001".parse().unwrap();
    assert_eq!(
        EcritureValidee::reconstituer(rebatir(achat), autre_journal.clone(), jour, None),
        Err(ErreurReconstitution::JournalDifferent {
            numero: autre_journal,
            journal: j("AC"),
        })
    );

    let mut sans_piece =
        EcritureBrouillon::nouvelle(j("AC"), achat.date(), achat.libelle().clone());
    for l in achat.lignes() {
        sans_piece.ajouter_ligne(l.clone());
    }
    assert_eq!(
        EcritureValidee::reconstituer(
            sans_piece.valider_equilibre().unwrap(),
            numero.clone(),
            jour,
            None
        ),
        Err(ErreurReconstitution::JustificationManquante)
    );

    let mut valeur_posterieure = rebatir(achat).en_brouillon();
    valeur_posterieure.mentionner_date_valeur_comptable(Some(achat.date()));
    assert_eq!(
        EcritureValidee::reconstituer(
            valeur_posterieure.valider_equilibre().unwrap(),
            numero.clone(),
            jour,
            None
        ),
        Err(ErreurReconstitution::DateValeurNonAnterieure {
            date_valeur: achat.date(),
            date: achat.date(),
        })
    );

    let origine_ailleurs: NumeroEcriture = "OD-2026-000001".parse().unwrap();
    assert_eq!(
        EcritureValidee::reconstituer(
            rebatir(achat),
            numero.clone(),
            jour,
            Some(origine_ailleurs.clone())
        ),
        Err(ErreurReconstitution::OrigineAutreJournal {
            numero: numero.clone(),
            origine: origine_ailleurs,
        })
    );

    assert_eq!(
        EcritureValidee::reconstituer(rebatir(achat), numero.clone(), jour, Some(numero.clone())),
        Err(ErreurReconstitution::OrigineIdentique(numero.clone()))
    );
    assert!(
        ErreurReconstitution::OrigineIdentique(numero)
            .to_string()
            .contains("se cite elle-même")
    );
}

// ---------------------------------------------------------------------------------------
// Propriété : fonctions libres = Livre
// ---------------------------------------------------------------------------------------

/// Journaux du dossier de test, plus un journal inconnu ; `AN` refuse la saisie, `BQ1`
/// exige son compte de trésorerie, `PA` est fermé pendant la clôture de l'exercice.
const JOURNAUX: [&str; 6] = ["OD", "AC", "BQ1", "AN", "PA", "ZZ"];
/// Comptes du dossier de test, plus un compte de regroupement (`605`) et un compte inconnu.
const COMPTES: [&str; 8] = ["4011", "4111", "521", "571", "6052", "7011", "605", "7019"];

fn jour_de(jour: i64) -> Date {
    date!(2026 - 01 - 01) + Duration::days(jour)
}

#[derive(Debug, Clone)]
enum Op {
    Valider {
        journal: usize,
        jour: i64,
        delai: i64,
        lignes: Vec<(usize, usize, i64)>,
        tiers: bool,
        piece: bool,
        parametres: bool,
        recul_valeur: Option<i64>,
    },
    ContrePasser {
        cible: usize,
        jour: i64,
        delai: i64,
    },
    Verrouiller(i64),
    Deverrouiller(i64),
    CloturerPeriode(i64),
    PasserEnCloture,
    CloturerExercice,
}

/// Délai entre la date de l'écriture et sa validation : parfois au-delà de l'échéance de
/// clôture.
fn delai() -> impl Strategy<Value = i64> {
    prop_oneof![4 => 0i64..60, 1 => 60i64..200]
}

fn op() -> impl Strategy<Value = Op> {
    // Jours de -5 (avant le dossier) à 735 (après 2027) ; délai de validation jusqu'à 200
    // jours (échéance de clôture dépassée).
    // Le plus souvent un journal ouvert à la saisie et un compte de mouvement connu, pour
    // que des validations aboutissent ; parfois les autres, pour les refus.
    let journal = prop_oneof![6 => 0..3usize, 1 => 3..JOURNAUX.len()];
    let compte = || prop_oneof![12 => 0..6usize, 1 => 6..COMPTES.len()];
    let valider = (
        journal,
        -5i64..735,
        delai(),
        proptest::collection::vec((compte(), compte(), 1i64..1_000_000), 1..4),
        proptest::bool::weighted(0.9),
        proptest::bool::weighted(0.95),
        any::<bool>(),
        proptest::option::weighted(0.2, 1i64..90),
    )
        .prop_map(
            |(journal, jour, delai, lignes, tiers, piece, parametres, recul_valeur)| Op::Valider {
                journal,
                jour,
                delai,
                lignes,
                tiers,
                piece,
                parametres,
                recul_valeur,
            },
        );
    prop_oneof![
        12 => valider,
        4 => (any::<usize>(), -5i64..735, delai())
            .prop_map(|(cible, jour, delai)| Op::ContrePasser { cible, jour, delai }),
        1 => (0i64..730).prop_map(Op::Verrouiller),
        1 => (0i64..730).prop_map(Op::Deverrouiller),
        1 => (0i64..730).prop_map(Op::CloturerPeriode),
        1 => Just(Op::PasserEnCloture),
        1 => Just(Op::CloturerExercice),
    ]
}

fn ecriture_generee(
    journal: usize,
    date: Date,
    lignes: &[(usize, usize, i64)],
    tiers: bool,
    piece_jointe: bool,
    parametres: bool,
    date_valeur: Option<Date>,
) -> EcritureEquilibree {
    let mut b = EcritureBrouillon::nouvelle(j(JOURNAUX[journal]), date, t("Écriture générée"));
    for (debit, credit, montant) in lignes {
        let montant = Decimal::from(*montant).to_string();
        for (compte, sens) in [(debit, Sens::Debit), (credit, Sens::Credit)] {
            let mut l = ligne(COMPTES[*compte], sens, &montant);
            if tiers {
                l = l.avec_tiers(IdTiers::parse("T1").unwrap());
            }
            b.ajouter_ligne(l);
        }
    }
    if piece_jointe {
        b.justifier(piece());
    }
    if parametres {
        b.utiliser_parametres(VersionParametres::parse("sn-2026.1").unwrap());
    }
    b.mentionner_date_valeur_comptable(date_valeur);
    b.valider_equilibre().unwrap()
}

type Compteurs = BTreeMap<(CodeJournal, CodeExercice), Compteur>;

/// Compteur courant du journal pour l'exercice de la date, comme l'API le lira en base.
fn compteur(
    compteurs: &Compteurs,
    exercices: &Exercices,
    journal: &CodeJournal,
    d: Date,
) -> Compteur {
    let exercice = exercices
        .exercice_contenant(d)
        .map_or_else(|| exercice("2026"), |e| e.code().clone());
    compteurs
        .get(&(journal.clone(), exercice.clone()))
        .cloned()
        .unwrap_or_else(|| Compteur::nouveau(journal.clone(), exercice))
}

fn enregistrer(compteurs: &mut Compteurs, c: Compteur) {
    compteurs.insert((c.journal().clone(), c.exercice().clone()), c);
}

/// Deux mondes sur le même référentiel : le [`Livre`] et une « base » simulée (écritures,
/// compteurs, liens de contre-passation) alimentée par les fonctions libres.
struct Banc {
    dossier: Dossier,
    livre: Livre,
    compteurs_livre: Compteurs,
    base: Vec<EcritureValidee>,
    compteurs_base: Compteurs,
    liens_base: BTreeMap<NumeroEcriture, NumeroEcriture>,
    numeros: Vec<NumeroEcriture>,
}

impl Banc {
    fn nouveau() -> Self {
        Self {
            dossier: Dossier::nouveau(),
            livre: Livre::new(),
            compteurs_livre: Compteurs::new(),
            base: Vec::new(),
            compteurs_base: Compteurs::new(),
            liens_base: BTreeMap::new(),
            numeros: Vec::new(),
        }
    }

    /// Applique l'opération aux deux mondes et vérifie qu'ils répondent de même.
    fn appliquer(&mut self, op: &Op) -> Result<(), TestCaseError> {
        let code_2026 = exercice("2026");
        let exercices = &mut self.dossier.exercices;
        match op {
            Op::Verrouiller(jour) => {
                let _ = exercices.verrouiller_periode(jour_de(*jour));
            }
            Op::Deverrouiller(jour) => {
                let _ = exercices.deverrouiller_periode(jour_de(*jour), t("Motif"));
            }
            Op::CloturerPeriode(jour) => {
                let _ = exercices.cloturer_periode(jour_de(*jour));
            }
            Op::PasserEnCloture => {
                let _ = exercices.passer_en_cloture(&code_2026);
            }
            Op::CloturerExercice => {
                let _ = exercices.cloturer(&code_2026);
            }
            Op::Valider {
                journal,
                jour,
                delai,
                lignes,
                tiers,
                piece,
                parametres,
                recul_valeur,
            } => {
                let date = jour_de(*jour);
                let date_valeur = recul_valeur.map(|r| date - Duration::days(r));
                let e = ecriture_generee(
                    *journal,
                    date,
                    lignes,
                    *tiers,
                    *piece,
                    *parametres,
                    date_valeur,
                );
                self.valider(e, date + Duration::days(*delai))?;
            }
            Op::ContrePasser { cible, jour, delai } => {
                // Une cible sur six environ est inconnue.
                let origine = if self.numeros.is_empty() || cible % 6 == 0 {
                    "OD-2026-000999".parse().unwrap()
                } else {
                    self.numeros[cible % self.numeros.len()].clone()
                };
                let date = jour_de(*jour);
                self.contre_passer(&origine, date, date + Duration::days(*delai))?;
            }
        }
        Ok(())
    }

    fn valider(
        &mut self,
        e: EcritureEquilibree,
        date_validation: Date,
    ) -> Result<(), TestCaseError> {
        let exercices = &self.dossier.exercices;
        let c_livre = compteur(&self.compteurs_livre, exercices, e.journal(), e.date());
        let c_base = compteur(&self.compteurs_base, exercices, e.journal(), e.date());
        let contexte = self.dossier.contexte();
        let par_livre = self
            .livre
            .valider(e.clone(), &contexte, &c_livre, date_validation);
        let par_base = valider(e, &contexte, &c_base, date_validation);
        match (par_livre, par_base) {
            (Ok((numero, s_livre)), Ok((validee, s_base))) => {
                prop_assert_eq!(&numero, validee.numero());
                prop_assert_eq!(&s_livre, &s_base);
                prop_assert_eq!(self.livre.ecriture(&numero), Some(&validee));
                enregistrer(&mut self.compteurs_livre, s_livre);
                enregistrer(&mut self.compteurs_base, s_base);
                self.base.push(validee);
                self.numeros.push(numero);
            }
            (Err(a), Err(b)) => prop_assert_eq!(a, b),
            (a, b) => prop_assert!(false, "livre {:?}, base {:?}", a, b),
        }
        Ok(())
    }

    fn contre_passer(
        &mut self,
        origine: &NumeroEcriture,
        date: Date,
        date_validation: Date,
    ) -> Result<(), TestCaseError> {
        let exercices = &self.dossier.exercices;
        let c_livre = compteur(&self.compteurs_livre, exercices, origine.journal(), date);
        let c_base = compteur(&self.compteurs_base, exercices, origine.journal(), date);
        let contexte = self.dossier.contexte();
        let par_livre = self.livre.contre_passer(
            origine,
            date,
            t("Contre-passation"),
            &contexte,
            &c_livre,
            date_validation,
        );
        // Côté base : l'API relit l'origine et sa contre-passation éventuelle ; une origine
        // absente est son propre refus (`EcritureInconnue`).
        let par_base = match self.base.iter().find(|e| e.numero() == origine) {
            None => Err(ErreurValidation::EcritureInconnue(origine.clone())),
            Some(source) => contre_passer(
                source,
                self.liens_base.get(origine),
                date,
                t("Contre-passation"),
                &contexte,
                &c_base,
                date_validation,
            ),
        };
        match (par_livre, par_base) {
            (Ok((numero, s_livre)), Ok((validee, s_base))) => {
                prop_assert_eq!(&numero, validee.numero());
                prop_assert_eq!(validee.origine(), Some(origine));
                prop_assert_eq!(&s_livre, &s_base);
                prop_assert_eq!(self.livre.ecriture(&numero), Some(&validee));
                enregistrer(&mut self.compteurs_livre, s_livre);
                enregistrer(&mut self.compteurs_base, s_base);
                self.liens_base.insert(origine.clone(), numero.clone());
                self.base.push(validee);
                self.numeros.push(numero);
            }
            (Err(a), Err(b)) => prop_assert_eq!(a, b),
            (a, b) => prop_assert!(false, "livre {:?}, base {:?}", a, b),
        }
        Ok(())
    }

    /// État final identique : écritures, compteurs, liens dans les deux sens.
    fn comparer(&self) -> Result<(), TestCaseError> {
        prop_assert_eq!(self.livre.ecritures(), self.base.as_slice());
        prop_assert_eq!(&self.compteurs_livre, &self.compteurs_base);
        for e in &self.base {
            prop_assert_eq!(
                self.livre.contre_passee_par(e.numero()),
                self.liens_base.get(e.numero())
            );
        }
        Ok(())
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn fonctions_libres_et_livre_donnent_le_meme_resultat(
        ops in proptest::collection::vec(op(), 1..60),
    ) {
        let mut banc = Banc::nouveau();
        for op in &ops {
            banc.appliquer(op)?;
        }
        banc.comparer()?;
    }

    /// Toute écriture validée (validation, contre-passation, opération tardive, version de
    /// paramètres) se reconstitue à l'identique à partir de ses champs.
    #[test]
    fn reconstitution_identique(ops in proptest::collection::vec(op(), 1..60)) {
        let mut banc = Banc::nouveau();
        for op in &ops {
            banc.appliquer(op)?;
        }
        for e in &banc.base {
            prop_assert_eq!(&reconstituer(e).unwrap(), e);
        }
    }
}
