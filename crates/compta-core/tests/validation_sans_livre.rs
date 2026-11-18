//! Validation et contre-passation sans livre en mémoire (lot L2, PR 3, issue L2-08) :
//! fonctions libres [`compta_core::valider`] et [`compta_core::contre_passer`], et
//! reconstitution d'une écriture relue de la base ([`EcritureValidee::reconstituer`]).
//!
//! Propriétés (`docs/lots/L2.md`, « Tests exigés ») :
//! - sur toute suite d'opérations (validations, contre-passations, refus compris, changements
//!   d'état des périodes, ouverture et clôture d'exercices), les fonctions libres, avec compteurs et liens de
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
    ErreurReconstitution, ErreurValidation, Exercice, Exercices, IdTiers, Livre, NumeroEcriture,
    Periode, Sens, StatutExercice, StatutPeriode, Validation, VersionParametres, contre_passer,
    valider,
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
    let validation = valider(achat(), &d.contexte(), &compteur, date!(2026 - 03 - 12)).unwrap();
    assert_eq!(validation.compteur().dernier(), 1);
    let (validee, suivant) = validation.clone().decomposer();
    assert_eq!(validation.ecriture(), &validee);
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
    let (validee, suivant) = valider(achat(), &d.contexte(), &a_41, date!(2026 - 03 - 10))
        .unwrap()
        .decomposer();
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
    .unwrap()
    .decomposer();
    let (miroir, suivant) = contre_passer(
        &origine,
        None,
        date!(2026 - 04 - 02),
        t("Annulation F-001"),
        &d.contexte(),
        &compteur,
        date!(2026 - 04 - 03),
    )
    .unwrap()
    .decomposer();
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
    assert!(reconstituer(&miroir).is_ok());

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
    d.exercices
        .cloturer_periode(date!(2026 - 01 - 10), [])
        .unwrap();
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

/// Le miroir d'une opération tardive ne reprend pas sa date de valeur comptable : il se
/// reconstitue donc sans refus `DateValeurSurContrePassation`.
#[test]
fn la_contre_passation_n_a_pas_de_date_de_valeur() {
    let (d, numeros) = ecritures_variees();
    let tardive = d.livre.ecriture(&numeros[1]).unwrap();
    let (miroir, _) = contre_passer(
        tardive,
        None,
        date!(2026 - 02 - 05),
        t("Annulation"),
        &d.contexte(),
        &Compteur::nouveau(j("OD"), exercice("2026"))
            .suivant()
            .unwrap()
            .1,
        date!(2026 - 02 - 05),
    )
    .unwrap()
    .decomposer();
    assert_eq!(miroir.date_valeur_comptable(), None);
    assert_eq!(reconstituer(&miroir).unwrap(), miroir);
}

/// Les contrôles de contexte ne sont pas rejoués : la période peut avoir été clôturée, le
/// compte désactivé depuis la validation.
#[test]
fn reconstituer_ne_rejoue_pas_le_contexte() {
    let (mut d, numeros) = ecritures_variees();
    d.exercices
        .cloturer_periode(date!(2026 - 02 - 10), [])
        .unwrap();
    d.exercices
        .cloturer_periode(date!(2026 - 03 - 10), [])
        .unwrap();
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
        ErreurReconstitution::OrigineIdentique(numero.clone())
            .to_string()
            .contains("se cite elle-même")
    );

    // Origine du même journal et du même exercice, numérotée après la contre-passation.
    let posterieure: NumeroEcriture = "AC-2026-000002".parse().unwrap();
    assert_eq!(
        EcritureValidee::reconstituer(
            rebatir(achat),
            numero.clone(),
            jour,
            Some(posterieure.clone())
        ),
        Err(ErreurReconstitution::OrigineNonAnterieure {
            numero: numero.clone(),
            origine: posterieure,
        })
    );
    // Une origine d'un exercice antérieur peut avoir une séquence plus grande.
    let cp = d.livre.ecriture(&numeros[2]).unwrap();
    let mut cp_2027 = rebatir(cp).en_brouillon();
    cp_2027.changer_date(date!(2027 - 01 - 05));
    assert!(
        EcritureValidee::reconstituer(
            cp_2027.valider_equilibre().unwrap(),
            "AC-2027-000001".parse().unwrap(),
            date!(2027 - 01 - 05),
            Some("AC-2026-000007".parse().unwrap()),
        )
        .is_ok()
    );

    // Contre-passation qui porterait une date de valeur comptable.
    let mut cp_tardive = rebatir(cp).en_brouillon();
    cp_tardive.mentionner_date_valeur_comptable(Some(date!(2026 - 01 - 05)));
    assert_eq!(
        EcritureValidee::reconstituer(
            cp_tardive.valider_equilibre().unwrap(),
            cp.numero().clone(),
            cp.date_validation(),
            cp.origine().cloned(),
        ),
        Err(ErreurReconstitution::DateValeurSurContrePassation(
            cp.numero().clone()
        ))
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
/// Marge, en jours, avant le premier exercice et après le dernier.
const MARGE: i64 = 5;

/// Instant en millièmes de la plage couverte par les exercices **existants** (marge
/// comprise) : la plage s'étend quand un exercice s'ouvre.
fn jour_de(exercices: &Exercices, millieme: u16) -> Date {
    let liste = exercices.exercices();
    let debut = liste.first().unwrap().debut() - Duration::days(MARGE);
    let fin = liste.last().unwrap().fin() + Duration::days(MARGE);
    debut + Duration::days((fin - debut).whole_days() * i64::from(millieme) / 1000)
}

#[derive(Debug, Clone)]
enum Op {
    Valider {
        journal: usize,
        instant: u16,
        delai: i64,
        lignes: Vec<(usize, usize, i64)>,
        tiers: bool,
        piece: bool,
        parametres: bool,
        recul_valeur: Option<i64>,
        debut_de_periode: bool,
        compteur_decale: bool,
    },
    ContrePasser {
        cible: usize,
        instant: u16,
        delai: i64,
        compteur_decale: bool,
    },
    Verrouiller(u16),
    Deverrouiller(u16),
    /// Clôture de la plus ancienne période non clôturée (les clôtures se font dans l'ordre).
    CloturerPeriode,
    /// Le plus ancien exercice ouvert passe en clôture.
    PasserEnCloture,
    /// Le plus ancien exercice en clôture est clôturé.
    CloturerExercice,
    /// Ouvre l'exercice suivant (refusé tant que deux exercices ne sont pas clôturés).
    OuvrirSuivant,
}

/// Délai entre la date de l'écriture et sa validation : parfois au-delà de l'échéance de
/// clôture.
fn delai() -> impl Strategy<Value = i64> {
    prop_oneof![4 => 0i64..60, 1 => 60i64..200]
}

fn op() -> impl Strategy<Value = Op> {
    // Instants répartis sur les exercices existants, marge comprise ; date de valeur jusqu'à
    // 120 jours avant la date (exercice précédent compris), l'écriture étant alors souvent
    // datée du premier jour de sa période (opération tardive reportée) ; un compteur sur vingt
    // est celui d'un autre exercice (`CompteurInadapte`).
    // Le plus souvent un journal ouvert à la saisie et un compte de mouvement connu, pour
    // que des validations aboutissent ; parfois les autres, pour les refus.
    let journal = prop_oneof![6 => 0..3usize, 1 => 3..JOURNAUX.len()];
    let compte = || prop_oneof![12 => 0..6usize, 1 => 6..COMPTES.len()];
    let valider = (
        (journal, 0u16..=1000, delai()),
        proptest::collection::vec((compte(), compte(), 1i64..1_000_000), 1..4),
        (
            proptest::bool::weighted(0.9),
            proptest::bool::weighted(0.95),
            any::<bool>(),
        ),
        (
            proptest::option::weighted(0.3, 1i64..120),
            proptest::bool::weighted(0.3),
        ),
        proptest::bool::weighted(0.05),
    )
        .prop_map(
            |(
                (journal, instant, delai),
                lignes,
                (tiers, piece, parametres),
                (recul_valeur, debut_de_periode),
                compteur_decale,
            )| Op::Valider {
                journal,
                instant,
                delai,
                lignes,
                tiers,
                piece,
                parametres,
                recul_valeur,
                debut_de_periode,
                compteur_decale,
            },
        );
    let contre_passer = (
        any::<usize>(),
        0u16..=1000,
        delai(),
        proptest::bool::weighted(0.05),
    )
        .prop_map(
            |(cible, instant, delai, compteur_decale)| Op::ContrePasser {
                cible,
                instant,
                delai,
                compteur_decale,
            },
        );
    prop_oneof![
        12 => valider,
        5 => contre_passer,
        1 => (0u16..=1000).prop_map(Op::Verrouiller),
        1 => (0u16..=1000).prop_map(Op::Deverrouiller),
        2 => Just(Op::CloturerPeriode),
        1 => Just(Op::PasserEnCloture),
        1 => Just(Op::CloturerExercice),
        1 => Just(Op::OuvrirSuivant),
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

/// Compteur courant du journal pour l'exercice de la date, comme l'API le lira en base ;
/// `decale` : celui d'un autre exercice, comme le fournirait un appelant fautif.
fn compteur(
    compteurs: &Compteurs,
    exercices: &Exercices,
    journal: &CodeJournal,
    d: Date,
    decale: bool,
) -> Compteur {
    let premier = exercices.exercices()[0].code().clone();
    let mut exercice = exercices
        .exercice_contenant(d)
        .map_or_else(|| premier.clone(), |e| e.code().clone());
    if decale
        && let Some(autre) = exercices
            .exercices()
            .iter()
            .map(Exercice::code)
            .find(|c| **c != exercice)
    {
        exercice = autre.clone();
    }
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

    fn plus_ancien(&self, statut: StatutExercice) -> Option<CodeExercice> {
        self.dossier
            .exercices
            .exercices()
            .iter()
            .find(|e| e.statut() == statut)
            .map(|e| e.code().clone())
    }

    /// Applique l'opération aux deux mondes et vérifie qu'ils répondent de même.
    fn appliquer(&mut self, op: &Op) -> Result<(), TestCaseError> {
        let jour = |millieme: u16| jour_de(&self.dossier.exercices, millieme);
        match op {
            Op::Verrouiller(m) => {
                let d = jour(*m);
                let _ = self.dossier.exercices.verrouiller_periode(d);
            }
            Op::Deverrouiller(m) => {
                let d = jour(*m);
                let _ = self.dossier.exercices.deverrouiller_periode(d, t("Motif"));
            }
            Op::CloturerPeriode => {
                let premiere = self
                    .dossier
                    .exercices
                    .exercices()
                    .iter()
                    .flat_map(Exercice::periodes)
                    .find(|p| p.statut() != StatutPeriode::Cloturee)
                    .map(Periode::debut);
                if let Some(d) = premiere {
                    let _ = self.dossier.exercices.cloturer_periode(d, []);
                }
            }
            Op::PasserEnCloture => {
                if let Some(code) = self.plus_ancien(StatutExercice::Ouvert) {
                    let _ = self.dossier.exercices.passer_en_cloture(&code);
                }
            }
            Op::CloturerExercice => {
                if let Some(code) = self.plus_ancien(StatutExercice::EnCloture) {
                    let _ = self.dossier.exercices.cloturer(&code, []);
                }
            }
            Op::OuvrirSuivant => {
                let _ = self.dossier.exercices.ouvrir_suivant();
            }
            Op::Valider {
                journal,
                instant,
                delai,
                lignes,
                tiers,
                piece,
                parametres,
                recul_valeur,
                debut_de_periode,
                compteur_decale,
            } => {
                let mut date = jour(*instant);
                if *debut_de_periode
                    && let Some(p) = self
                        .dossier
                        .exercices
                        .exercice_contenant(date)
                        .and_then(|e| e.periode_contenant(date))
                {
                    date = p.debut();
                }
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
                self.valider(e, date + Duration::days(*delai), *compteur_decale)?;
            }
            Op::ContrePasser {
                cible,
                instant,
                delai,
                compteur_decale,
            } => {
                let date = jour(*instant);
                // Une cible sur six environ est inconnue.
                let origine = if self.numeros.is_empty() || cible % 6 == 0 {
                    "OD-2026-000999".parse().unwrap()
                } else {
                    self.numeros[cible % self.numeros.len()].clone()
                };
                self.contre_passer(
                    &origine,
                    date,
                    date + Duration::days(*delai),
                    *compteur_decale,
                )?;
            }
        }
        Ok(())
    }

    fn valider(
        &mut self,
        e: EcritureEquilibree,
        date_validation: Date,
        decale: bool,
    ) -> Result<(), TestCaseError> {
        let exercices = &self.dossier.exercices;
        let c_livre = compteur(
            &self.compteurs_livre,
            exercices,
            e.journal(),
            e.date(),
            decale,
        );
        let c_base = compteur(
            &self.compteurs_base,
            exercices,
            e.journal(),
            e.date(),
            decale,
        );
        let contexte = self.dossier.contexte();
        let par_livre = self
            .livre
            .valider(e.clone(), &contexte, &c_livre, date_validation);
        let par_base = valider(e, &contexte, &c_base, date_validation).map(Validation::decomposer);
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
        decale: bool,
    ) -> Result<(), TestCaseError> {
        let exercices = &self.dossier.exercices;
        let c_livre = compteur(
            &self.compteurs_livre,
            exercices,
            origine.journal(),
            date,
            decale,
        );
        let c_base = compteur(
            &self.compteurs_base,
            exercices,
            origine.journal(),
            date,
            decale,
        );
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
            )
            .map(Validation::decomposer),
        };
        match (par_livre, par_base) {
            (Ok((numero, s_livre)), Ok((validee, s_base))) => {
                prop_assert_eq!(&numero, validee.numero());
                prop_assert_eq!(validee.origine(), Some(origine));
                prop_assert_eq!(validee.date_valeur_comptable(), None);
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
        ops in proptest::collection::vec(op(), 1..80),
    ) {
        let mut banc = Banc::nouveau();
        for op in &ops {
            banc.appliquer(op)?;
        }
        banc.comparer()?;
    }

    /// Toute écriture validée (validation, contre-passation, opération tardive, version de
    /// paramètres, plusieurs exercices) se reconstitue à l'identique à partir de ses champs.
    #[test]
    fn reconstitution_identique(ops in proptest::collection::vec(op(), 1..80)) {
        let mut banc = Banc::nouveau();
        for op in &ops {
            banc.appliquer(op)?;
        }
        for e in &banc.base {
            prop_assert_eq!(&reconstituer(e).unwrap(), e);
        }
    }
}
