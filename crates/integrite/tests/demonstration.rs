//! Démonstration de bout en bout du lot L1 (`docs/lots/L1.md`, « Critères de fin »).
//!
//! Scénario : une PME de négoce dakaroise, fictive, tient son premier exercice (2026) avec
//! l'API publique de `compta-core`, puis remet l'export d'intégrité de son livre à un
//! auditeur, qui le vérifie hors ligne avec `sigma-verifier`.
//!
//! 1. **Création du dossier** à partir du référentiel officiel (`syscohada`, liste des
//!    comptes de l'AUDCIF) : journaux par défaut, la caisse CA tenue sur 5711 ; une banque BQ1
//!    tenue sur 5211, sous-compte officiel de 521 ; premier exercice, du 1er janvier au
//!    31 décembre 2026.
//! 2. **Saisie et validation** de quatorze écritures : apport en capital, achat avec TVA
//!    récupérable, ventes avec TVA facturée, encaissements et règlements en banque, opérations
//!    de caisse, paie, inventaire. Chacune est justifiée par une pièce et validée le jour de sa
//!    date. Les écritures non conformes aux règles de validation sont refusées.
//! 3. **Contre-passation** d'une facture saisie deux fois par erreur.
//! 4. **Balance générale** (format 6, AUDCIF art. 19) : équilibrée sur chaque paire de
//!    colonnes ; soldes identiques à ceux d'un dossier jumeau où l'erreur n'a jamais été
//!    saisie (effet nul de la contre-passation).
//! 5. **Export** d'intégrité (sans jeton d'horodatage) dans un fichier.
//! 6. **`sigma-verifier`** avec `--dossier` et `--tete` (témoin externe) : code 0 et
//!    « RÉSULTAT : INTÈGRE ».
//! 7. **Trois altérations** de l'export, chacune dans son propre fichier : un octet d'un
//!    montant, une écriture supprimée, deux écritures consécutives permutées. Chacune donne le
//!    code 1 et des anomalies qui désignent l'écriture touchée.
//!
//! Tous les numéros de comptes employés figurent dans la liste officielle
//! (`crates/syscohada/data/comptes.toml`) ; le test vérifie que ce sont des comptes de
//! mouvement du plan du dossier. Les montants sont illustratifs ; tiers et pièces sont fictifs
//! (une pièce est référencée par une empreinte SHA-256 factice, motif de deux caractères répété).

#![allow(clippy::unwrap_used)]

use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::BufWriter;
use std::path::{Path, PathBuf};
use std::process::Command;

use compta_core::{
    Balance, CodeExercice, CodeJournal, Compteur, Contexte, Devise, EcritureBrouillon,
    EcritureValidee, ErreurValidation, Exercices, Filtre, FinPremierExercice, FormatBalance,
    IdTiers, Journal, Journaux, Justification, Ligne, Livre, Mode, Montant, NumeroCompte,
    NumeroEcriture, PlanComptes, ReferencePiece, Regroupement, Sens, TexteCourt, TypeJournal,
    journaux_par_defaut,
};
use integrite::{Empreinte, IdDossier};
use rust_decimal::Decimal;
use syscohada::Referentiel;
use time::Date;
use time::macros::date;

/// Identifiant du dossier, inscrit dans l'export et attendu par l'auditeur.
const DOSSIER: &str = "pme-negoce-dakar";

// Comptes de la liste officielle employés par le scénario (intitulés officiels en commentaire).
/// 1013 Capital souscrit, appelé, versé, non amorti.
const CAPITAL: &str = "1013";
/// 3111 Marchandises A1.
const MARCHANDISES: &str = "3111";
/// 4011 Fournisseurs (sous le collectif 401 : tiers obligatoire).
const FOURNISSEURS: &str = "4011";
/// 4111 Clients (sous le collectif 411 : tiers obligatoire).
const CLIENTS: &str = "4111";
/// 422 Personnel, rémunérations dues.
const PERSONNEL: &str = "422";
/// 4311 Prestations familiales.
const PRESTATIONS_FAMILIALES: &str = "4311";
/// 4313 Caisse de retraite obligatoire.
const RETRAITE: &str = "4313";
/// 4431 T.V.A. facturée sur ventes.
const TVA_FACTUREE: &str = "4431";
/// 4452 T.V.A. récupérable sur achats.
const TVA_RECUPERABLE: &str = "4452";
/// 4472 Impôts sur salaires.
const IMPOTS_SUR_SALAIRES: &str = "4472";
/// 5211 Banques en monnaie nationale (sous 521 Banques locales) : compte du journal BQ1.
const BANQUE: &str = "5211";
/// 5711 Caisse en monnaie nationale : compte du journal CA.
const CAISSE: &str = "5711";
/// 6011 Achats de marchandises dans la Région.
const ACHATS: &str = "6011";
/// 6031 Variations des stocks de marchandises.
const VARIATION_DES_STOCKS: &str = "6031";
/// 6052 Fournitures non stockables - Électricité.
const ELECTRICITE: &str = "6052";
/// 6611 Appointements salaires et commissions.
const SALAIRES: &str = "6611";
/// 6641 Charges sociales sur rémunération du personnel national.
const CHARGES_SOCIALES: &str = "6641";
/// 7011 Ventes de marchandises dans la Région.
const VENTES: &str = "7011";

const COMPTES_EMPLOYES: [&str; 18] = [
    CAPITAL,
    MARCHANDISES,
    FOURNISSEURS,
    CLIENTS,
    PERSONNEL,
    PRESTATIONS_FAMILIALES,
    RETRAITE,
    TVA_FACTUREE,
    TVA_RECUPERABLE,
    IMPOTS_SUR_SALAIRES,
    BANQUE,
    CAISSE,
    ACHATS,
    VARIATION_DES_STOCKS,
    ELECTRICITE,
    SALAIRES,
    CHARGES_SOCIALES,
    VENTES,
];

fn n(numero: &str) -> NumeroCompte {
    NumeroCompte::parse(numero).unwrap()
}

fn t(texte: &str) -> TexteCourt {
    TexteCourt::parse(texte).unwrap()
}

fn j(code: &str) -> CodeJournal {
    CodeJournal::parse(code).unwrap()
}

fn tiers(code: &str) -> IdTiers {
    IdTiers::parse(code).unwrap()
}

fn xof(montant: u64) -> Decimal {
    Decimal::from(montant)
}

fn ligne(compte: &str, sens: Sens, montant: u64) -> Ligne {
    Ligne::en_xof(n(compte), sens, Montant::nouveau(xof(montant), Devise::XOF)).unwrap()
}

fn debit(compte: &str, montant: u64) -> Ligne {
    ligne(compte, Sens::Debit, montant)
}

fn credit(compte: &str, montant: u64) -> Ligne {
    ligne(compte, Sens::Credit, montant)
}

/// Pièce justificative : empreinte SHA-256 factice, `motif` (deux chiffres hexadécimaux)
/// répété 32 fois.
fn piece(motif: &str) -> Justification {
    Justification::pieces([ReferencePiece::parse(&motif.repeat(32)).unwrap()]).unwrap()
}

/// Dossier comptable tenu en mémoire : plan, journaux, exercices, livre et compteurs de
/// numérotation (le rôle de la base de données, lot L2).
struct Dossier {
    plan: PlanComptes,
    journaux: Journaux,
    exercices: Exercices,
    livre: Livre,
    compteurs: BTreeMap<(CodeJournal, CodeExercice), Compteur>,
}

impl Dossier {
    /// Étape 1 : crée le dossier à partir du référentiel officiel.
    fn creer() -> Self {
        let referentiel = Referentiel::officiel().unwrap();
        let plan = referentiel.plan_comptes().unwrap();
        assert_eq!(plan.comptes().count(), referentiel.len());
        for compte in COMPTES_EMPLOYES {
            plan.compte_de_mouvement_actif(&n(compte)).unwrap();
        }
        let mut journaux = Journaux::new();
        for modele in journaux_par_defaut().unwrap() {
            // Le seul journal de trésorerie par défaut est la caisse (CA).
            let compte = (modele.genre() == TypeJournal::Tresorerie).then(|| n(CAISSE));
            journaux
                .ajouter(modele.instancier(compte, &plan).unwrap())
                .unwrap();
        }
        assert_eq!(
            journaux.journal(&j("CA")).unwrap().compte_tresorerie(),
            Some(&n(CAISSE))
        );
        let banque = Journal::nouveau(
            j("BQ1"),
            t("Banque"),
            TypeJournal::Tresorerie,
            Some(n(BANQUE)),
            &plan,
        )
        .unwrap();
        journaux.ajouter(banque).unwrap();
        let mut exercices = Exercices::new();
        let exercice = exercices
            .ouvrir_premier(date!(2026 - 01 - 01), FinPremierExercice::AnneeDeDebut)
            .unwrap();
        assert_eq!(exercice.fin(), date!(2026 - 12 - 31));
        Self {
            plan,
            journaux,
            exercices,
            livre: Livre::new(),
            compteurs: BTreeMap::new(),
        }
    }

    /// Compteur du journal pour l'exercice qui contient `date`.
    fn compteur(&self, journal: &CodeJournal, date: Date) -> Compteur {
        let exercice = self.exercices.exercice_contenant(date).unwrap().code();
        self.compteurs
            .get(&(journal.clone(), exercice.clone()))
            .cloned()
            .unwrap_or_else(|| Compteur::nouveau(journal.clone(), exercice.clone()))
    }

    fn enregistrer(&mut self, suivant: Compteur) {
        self.compteurs.insert(
            (suivant.journal().clone(), suivant.exercice().clone()),
            suivant,
        );
    }

    /// Soumet un brouillon à la validation, le `validee_le`.
    fn soumettre(
        &mut self,
        journal: &str,
        date: Date,
        validee_le: Date,
        libelle: &str,
        justification: Option<Justification>,
        lignes: Vec<Ligne>,
    ) -> Result<NumeroEcriture, ErreurValidation> {
        let mut brouillon = EcritureBrouillon::nouvelle(j(journal), date, t(libelle));
        for l in lignes {
            brouillon.ajouter_ligne(l);
        }
        if let Some(justification) = justification {
            brouillon.justifier(justification);
        }
        let ecriture = brouillon.valider_equilibre().unwrap();
        let compteur = self.compteur(ecriture.journal(), date);
        let contexte = Contexte {
            plan: &self.plan,
            journaux: &self.journaux,
            exercices: &self.exercices,
        };
        let (numero, suivant) = self
            .livre
            .valider(ecriture, &contexte, &compteur, validee_le)
            .map_err(|refus| refus.decomposer().1)?;
        self.enregistrer(suivant);
        Ok(numero)
    }

    /// Valide une écriture justifiée par une pièce, le jour de sa date.
    fn valider(
        &mut self,
        journal: &str,
        date: Date,
        libelle: &str,
        motif_piece: &str,
        lignes: Vec<Ligne>,
    ) -> NumeroEcriture {
        self.soumettre(
            journal,
            date,
            date,
            libelle,
            Some(piece(motif_piece)),
            lignes,
        )
        .unwrap()
    }

    /// Contre-passe une écriture, le jour de la contre-passation.
    fn contre_passer(&mut self, origine: &NumeroEcriture, date: Date) -> NumeroEcriture {
        let compteur = self.compteur(origine.journal(), date);
        let contexte = Contexte {
            plan: &self.plan,
            journaux: &self.journaux,
            exercices: &self.exercices,
        };
        let (numero, suivant) = self
            .livre
            .contre_passer(
                origine,
                date,
                t("Annulation de la facture FV-0002 saisie en double"),
                &contexte,
                &compteur,
                date,
            )
            .unwrap();
        self.enregistrer(suivant);
        numero
    }

    /// Balance générale de l'exercice 2026 (du 1er janvier au 31 décembre), par compte.
    fn balance(&self) -> Balance {
        let exercice = self
            .exercices
            .exercice_contenant(date!(2026 - 01 - 01))
            .unwrap();
        let filtre = Filtre::exercice(exercice, &self.journaux, exercice.fin()).unwrap();
        Balance::calculer(
            self.livre.ecritures(),
            &[],
            Mode::Definitif,
            &filtre,
            Regroupement::ParCompte,
        )
        .unwrap()
    }
}

/// Étape 2 (et 3 si `doublon`) : les écritures de l'exercice 2026, dans l'ordre de leur
/// validation. Avec `doublon`, la facture FV-0002 est saisie une seconde fois par erreur puis
/// contre-passée le lendemain : la fonction renvoie alors le doublon et sa contre-passation.
fn tenir_l_exercice(
    dossier: &mut Dossier,
    doublon: bool,
) -> Option<(NumeroEcriture, NumeroEcriture)> {
    constituer_et_commercer(dossier);
    caisse_et_paie(dossier);
    d_avril_a_decembre(dossier, doublon)
}

/// Janvier et février : apport en capital, premier achat, première vente, règlements.
fn constituer_et_commercer(d: &mut Dossier) {
    d.valider(
        "BQ1",
        date!(2026 - 01 - 05),
        "Libération du capital social",
        "a1",
        vec![debit(BANQUE, 5_000_000), credit(CAPITAL, 5_000_000)],
    );
    d.valider(
        "AC",
        date!(2026 - 01 - 12),
        "Facture fournisseur FA-0001, marchandises",
        "a2",
        vec![
            debit(ACHATS, 2_000_000),
            debit(TVA_RECUPERABLE, 360_000),
            credit(FOURNISSEURS, 2_360_000).avec_tiers(tiers("FOUR-001")),
        ],
    );
    d.valider(
        "VT",
        date!(2026 - 01 - 20),
        "Facture FV-0001",
        "a3",
        vec![
            debit(CLIENTS, 1_770_000).avec_tiers(tiers("CLI-001")),
            credit(VENTES, 1_500_000),
            credit(TVA_FACTUREE, 270_000),
        ],
    );
    d.valider(
        "BQ1",
        date!(2026 - 02 - 05),
        "Virement reçu, facture FV-0001",
        "a4",
        vec![
            debit(BANQUE, 1_770_000),
            credit(CLIENTS, 1_770_000).avec_tiers(tiers("CLI-001")),
        ],
    );
    d.valider(
        "BQ1",
        date!(2026 - 02 - 10),
        "Virement émis, facture FA-0001",
        "a5",
        vec![
            debit(FOURNISSEURS, 2_360_000).avec_tiers(tiers("FOUR-001")),
            credit(BANQUE, 2_360_000),
        ],
    );
}

/// Mars et avril : vente au comptant et facture d'électricité en caisse, paie de mars.
fn caisse_et_paie(d: &mut Dossier) {
    d.valider(
        "CA",
        date!(2026 - 03 - 03),
        "Vente au comptant, ticket 0001",
        "b1",
        vec![
            debit(CAISSE, 236_000),
            credit(VENTES, 200_000),
            credit(TVA_FACTUREE, 36_000),
        ],
    );
    d.valider(
        "CA",
        date!(2026 - 03 - 10),
        "Facture d'électricité de février, payée en espèces",
        "b2",
        vec![
            debit(ELECTRICITE, 50_000),
            debit(TVA_RECUPERABLE, 9_000),
            credit(CAISSE, 59_000),
        ],
    );
    d.valider(
        "PA",
        date!(2026 - 03 - 31),
        "Salaires de mars",
        "b3",
        vec![
            debit(SALAIRES, 600_000),
            credit(PERSONNEL, 528_000),
            credit(RETRAITE, 33_600),
            credit(IMPOTS_SUR_SALAIRES, 38_400),
        ],
    );
    d.valider(
        "PA",
        date!(2026 - 03 - 31),
        "Charges sociales patronales de mars",
        "b4",
        vec![
            debit(CHARGES_SOCIALES, 96_000),
            credit(PRESTATIONS_FAMILIALES, 42_000),
            credit(RETRAITE, 54_000),
        ],
    );
    d.valider(
        "BQ1",
        date!(2026 - 04 - 05),
        "Virement des salaires nets de mars",
        "b5",
        vec![debit(PERSONNEL, 528_000), credit(BANQUE, 528_000)],
    );
}

/// Avril à décembre : deuxième facture (et, si `doublon`, sa saisie en double puis la
/// contre-passation du doublon), encaissement, inventaire de fin d'exercice.
fn d_avril_a_decembre(d: &mut Dossier, doublon: bool) -> Option<(NumeroEcriture, NumeroEcriture)> {
    let facture = || {
        vec![
            debit(CLIENTS, 590_000).avec_tiers(tiers("CLI-002")),
            credit(VENTES, 500_000),
            credit(TVA_FACTUREE, 90_000),
        ]
    };
    d.valider(
        "VT",
        date!(2026 - 04 - 15),
        "Facture FV-0002",
        "c1",
        facture(),
    );
    // Étape 3 : la même facture, saisie une seconde fois par erreur, est validée ; elle est
    // définitive et ne peut être corrigée que par contre-passation.
    let erreur = doublon.then(|| {
        let origine = d.valider(
            "VT",
            date!(2026 - 04 - 15),
            "Facture FV-0002",
            "c1",
            facture(),
        );
        let contre_passation = d.contre_passer(&origine, date!(2026 - 04 - 16));
        (origine, contre_passation)
    });
    d.valider(
        "BQ1",
        date!(2026 - 05 - 10),
        "Virement reçu, facture FV-0002",
        "c2",
        vec![
            debit(BANQUE, 590_000),
            credit(CLIENTS, 590_000).avec_tiers(tiers("CLI-002")),
        ],
    );
    d.valider(
        "IN",
        date!(2026 - 12 - 31),
        "Stock final de marchandises, inventaire physique",
        "c3",
        vec![
            debit(MARCHANDISES, 800_000),
            credit(VARIATION_DES_STOCKS, 800_000),
        ],
    );
    erreur
}

/// Les règles de validation refusent les écritures non conformes, sans toucher au livre.
fn refuser_les_ecritures_non_conformes(d: &mut Dossier) {
    let avant = d.livre.ecritures().len();
    let jour = date!(2026 - 06 - 02);
    let facture = || {
        vec![
            debit(CLIENTS, 118_000).avec_tiers(tiers("CLI-003")),
            credit(VENTES, 100_000),
            credit(TVA_FACTUREE, 18_000),
        ]
    };
    // Aucune écriture sans pièce justificative.
    assert_eq!(
        d.soumettre("VT", jour, jour, "Facture FV-0003", None, facture()),
        Err(ErreurValidation::JustificationManquante)
    );
    // Tiers obligatoire sur un compte collectif.
    let sans_tiers = vec![
        debit(CLIENTS, 118_000),
        credit(VENTES, 100_000),
        credit(TVA_FACTUREE, 18_000),
    ];
    assert_eq!(
        d.soumettre(
            "VT",
            jour,
            jour,
            "Facture FV-0003",
            Some(piece("d1")),
            sans_tiers
        ),
        Err(ErreurValidation::TiersManquant {
            ligne: 0,
            compte: n(CLIENTS)
        })
    );
    // Le journal d'à-nouveaux est généré, jamais saisi.
    assert_eq!(
        d.soumettre(
            "AN",
            jour,
            jour,
            "Saisie interdite",
            Some(piece("d1")),
            facture()
        ),
        Err(ErreurValidation::JournalSansSaisie(j("AN")))
    );
    // Une écriture du journal de banque mouvemente le compte de banque.
    let sans_banque = vec![
        debit(CAISSE, 118_000),
        credit(CLIENTS, 118_000).avec_tiers(tiers("CLI-003")),
    ];
    assert_eq!(
        d.soumettre(
            "BQ1",
            jour,
            jour,
            "Encaissement",
            Some(piece("d1")),
            sans_banque
        ),
        Err(ErreurValidation::CompteTresorerieAbsent {
            journal: j("BQ1"),
            compte: n(BANQUE)
        })
    );
    // Une écriture de janvier doit être validée au plus tard le 30 avril (fin du troisième
    // mois qui suit la période).
    assert_eq!(
        d.soumettre(
            "VT",
            date!(2026 - 01 - 31),
            date!(2026 - 05 - 01),
            "Facture FV-0003",
            Some(piece("d1")),
            facture()
        ),
        Err(ErreurValidation::EcheanceDepassee {
            periode: date!(2026 - 01 - 01),
            echeance: date!(2026 - 04 - 30)
        })
    );
    assert_eq!(d.livre.ecritures().len(), avant);
}

/// Solde final (débiteur, créditeur) de chaque compte de la balance.
fn soldes(balance: &Balance) -> BTreeMap<String, (Decimal, Decimal)> {
    balance
        .lignes()
        .iter()
        .map(|l| (l.cle().to_owned(), l.solde_final().unwrap()))
        .collect()
}

/// Étape 4 : balance générale équilibrée ; contre-passation sans effet sur les soldes.
fn controler_la_balance(
    dossier: &Dossier,
    doublon: &NumeroEcriture,
    contre_passation: &NumeroEcriture,
) {
    let balance = dossier.balance();
    // Colonnes : ouverture (D, C), mouvements (D, C), solde final (D, C).
    let totaux = balance.totaux(FormatBalance::Six).unwrap();
    for paire in totaux.chunks(2) {
        assert_eq!(paire[0], paire[1], "balance déséquilibrée : {totaux:?}");
    }
    // Premier exercice, sans à-nouveaux : aucune ouverture.
    assert_eq!(totaux[0], Decimal::ZERO);
    // Les mouvements reprennent toutes les écritures validées.
    let total: Decimal = dossier
        .livre
        .ecritures()
        .iter()
        .map(EcritureValidee::total)
        .sum();
    assert_eq!(totaux[2], total);
    let soldes_du_dossier = soldes(&balance);
    for (compte, attendu) in [
        (BANQUE, (xof(4_472_000), Decimal::ZERO)),
        (CAISSE, (xof(177_000), Decimal::ZERO)),
        (CLIENTS, (Decimal::ZERO, Decimal::ZERO)),
        (VENTES, (Decimal::ZERO, xof(2_200_000))),
        (TVA_FACTUREE, (Decimal::ZERO, xof(396_000))),
    ] {
        assert_eq!(
            soldes_du_dossier[compte], attendu,
            "solde du compte {compte}"
        );
    }

    // La contre-passation est liée à son origine, dans les deux sens.
    assert_eq!(
        dossier.livre.contre_passations(doublon),
        [contre_passation.clone()]
    );
    let miroir = dossier.livre.ecriture(contre_passation).unwrap();
    assert_eq!(miroir.origine(), Some(doublon));

    // Effet nul : mêmes soldes que le dossier jumeau, où l'erreur n'a jamais été saisie ;
    // seuls les mouvements gardent la trace du doublon et de son annulation.
    let mut jumeau = Dossier::creer();
    assert!(tenir_l_exercice(&mut jumeau, false).is_none());
    let balance_jumelle = jumeau.balance();
    assert_eq!(soldes_du_dossier, soldes(&balance_jumelle));
    let mouvements_jumeaux = balance_jumelle.totaux(FormatBalance::Six).unwrap()[2];
    assert_eq!(
        totaux[2] - mouvements_jumeaux,
        Decimal::TWO * miroir.total()
    );
}

/// Étape 5 : exporte le livre dans `chemin` ; renvoie la tête de chaîne.
fn exporter(dossier: &Dossier, chemin: &Path) -> Empreinte {
    let sortie = BufWriter::new(File::create(chemin).unwrap());
    integrite::exporter(
        &IdDossier::parse(DOSSIER).unwrap(),
        dossier.livre.ecritures(),
        &[],
        sortie,
    )
    .unwrap()
}

/// Lance `sigma-verifier` comme l'auditeur : dossier et tête attendus, issus d'une source
/// distincte de l'export. Renvoie le code de retour et le rapport.
fn sigma_verifier(chemin: &Path, tete: &Empreinte) -> (i32, String) {
    let sortie = Command::new(env!("CARGO_BIN_EXE_sigma-verifier"))
        .args(["--dossier", DOSSIER, "--tete", &tete.to_string()])
        .arg(chemin)
        .output()
        .unwrap();
    (
        sortie.status.code().unwrap(),
        String::from_utf8(sortie.stdout).unwrap(),
    )
}

/// Export découpé en lignes : en-tête, blocs d'écriture (de `ecriture v1` à `empreinte=`
/// inclus), pied (tête, nombre, `fin-export`).
struct Decoupe<'a> {
    entete: Vec<&'a str>,
    ecritures: Vec<Vec<&'a str>>,
    pied: Vec<&'a str>,
}

impl<'a> Decoupe<'a> {
    fn nouvelle(export: &'a str) -> Self {
        let mut decoupe = Self {
            entete: Vec::new(),
            ecritures: Vec::new(),
            pied: Vec::new(),
        };
        for ligne in export.lines() {
            if ligne == "ecriture v1" {
                decoupe.ecritures.push(vec![ligne]);
            } else if !decoupe.pied.is_empty() || ligne.starts_with("tete=") {
                decoupe.pied.push(ligne);
            } else if let Some(bloc) = decoupe.ecritures.last_mut() {
                bloc.push(ligne);
            } else {
                decoupe.entete.push(ligne);
            }
        }
        decoupe
    }

    /// Rang dans la chaîne de l'écriture `numero`.
    fn rang(&self, numero: &str) -> usize {
        let cle = format!("numero={numero}");
        self.ecritures
            .iter()
            .position(|bloc| bloc.contains(&cle.as_str()))
            .unwrap()
    }

    fn recoller(&self) -> String {
        let mut texte = String::new();
        for ligne in self
            .entete
            .iter()
            .chain(self.ecritures.iter().flatten())
            .chain(&self.pied)
        {
            texte.push_str(ligne);
            texte.push('\n');
        }
        texte
    }
}

/// Altération 1 : un seul octet change, le premier chiffre du premier montant de l'écriture
/// `numero`.
fn modifier_un_octet(export: &str, numero: &str) -> String {
    let debut = export.find(&format!("numero={numero}\n")).unwrap();
    let position = debut + export[debut..].find("\nmontant=").unwrap() + "\nmontant=".len();
    let mut octets = export.as_bytes().to_vec();
    assert!(octets[position].is_ascii_digit());
    octets[position] = if octets[position] == b'9' {
        b'8'
    } else {
        octets[position] + 1
    };
    String::from_utf8(octets).unwrap()
}

/// Écrit l'export altéré dans son propre fichier, le fait vérifier et constate l'altération
/// (code 1). Renvoie le rapport.
fn constater_l_alteration(repertoire: &Path, nom: &str, altere: &str, tete: &Empreinte) -> String {
    let chemin = repertoire.join(format!("export-{nom}.txt"));
    fs::write(&chemin, altere).unwrap();
    let (code, rapport) = sigma_verifier(&chemin, tete);
    assert_eq!(code, 1, "{nom} : {rapport}");
    assert!(
        rapport.contains("RÉSULTAT : ANOMALIES"),
        "{nom} : {rapport}"
    );
    rapport
}

/// Exige qu'une même ligne du rapport contienne `sujet` (en général le numéro de l'écriture
/// en cause) et `motif`.
fn exiger(rapport: &str, sujet: &str, motif: &str) {
    assert!(
        rapport
            .lines()
            .any(|l| l.contains(sujet) && l.contains(motif)),
        "« {motif} » attendu pour {sujet} :\n{rapport}"
    );
}

/// Altération 1 : un octet modifié dans un montant de la facture fournisseur. L'empreinte de
/// l'écriture ne correspond plus à son contenu, qui n'est plus équilibré ; la tête recalculée
/// ne correspond plus au témoin.
fn alterer_un_octet(repertoire: &Path, export: &str, tete: &Empreinte) {
    let altere = modifier_un_octet(export, "AC-2026-000001");
    assert_eq!(altere.len(), export.len());
    let differences = altere
        .bytes()
        .zip(export.bytes())
        .filter(|(a, b)| a != b)
        .count();
    assert_eq!(differences, 1);
    let rapport = constater_l_alteration(repertoire, "octet-modifie", &altere, tete);
    exiger(
        &rapport,
        "AC-2026-000001",
        "empreinte différente du contenu",
    );
    exiger(&rapport, "AC-2026-000001", "débits et crédits différents");
    exiger(&rapport, &tete.to_string(), "tête attendue");
}

/// Altération 2 : une écriture supprimée entière (le règlement fournisseur BQ1-2026-000003,
/// avec son empreinte), le reste inchangé. La suivante ne se chaîne plus, la numérotation de
/// BQ1 a un trou et le nombre d'écritures ne concorde plus.
fn supprimer_une_ecriture(repertoire: &Path, export: &str, tete: &Empreinte) {
    let mut decoupe = Decoupe::nouvelle(export);
    let rang = decoupe.rang("BQ1-2026-000003");
    decoupe.ecritures.remove(rang);
    let rapport =
        constater_l_alteration(repertoire, "ecriture-supprimee", &decoupe.recoller(), tete);
    exiger(
        &rapport,
        "CA-2026-000001",
        "empreinte différente du contenu",
    );
    exiger(
        &rapport,
        "BQ1-2026-000004",
        "numérotation discontinue, rang 000003 attendu",
    );
    exiger(&rapport, "15 écriture(s) déclarée(s)", "nombre différent");
}

/// Altération 3 : deux écritures consécutives permutées avec leurs empreintes (le doublon
/// VT-2026-000003 et sa contre-passation VT-2026-000004). Les empreintes ne se chaînent plus,
/// la numérotation de VT est désordonnée et la contre-passation précède son origine.
fn permuter_deux_ecritures(repertoire: &Path, export: &str, tete: &Empreinte) {
    let mut decoupe = Decoupe::nouvelle(export);
    let rang = decoupe.rang("VT-2026-000003");
    assert_eq!(decoupe.rang("VT-2026-000004"), rang + 1);
    decoupe.ecritures.swap(rang, rang + 1);
    let rapport =
        constater_l_alteration(repertoire, "ecritures-permutees", &decoupe.recoller(), tete);
    for numero in ["VT-2026-000004", "VT-2026-000003"] {
        exiger(&rapport, numero, "empreinte différente du contenu");
    }
    exiger(
        &rapport,
        "VT-2026-000004",
        "numérotation discontinue, rang 000003 attendu",
    );
    exiger(
        &rapport,
        "VT-2026-000004",
        "contre-passe VT-2026-000003, absente de la chaîne avant elle",
    );
}

#[test]
fn demonstration_de_bout_en_bout_du_lot_l1() {
    // 1. Dossier créé à partir du référentiel officiel.
    let mut dossier = Dossier::creer();

    // 2 et 3. Saisie, validation, refus des écritures non conformes, contre-passation.
    let (doublon, contre_passation) = tenir_l_exercice(&mut dossier, true).unwrap();
    refuser_les_ecritures_non_conformes(&mut dossier);
    let chaine: Vec<String> = dossier
        .livre
        .ecritures()
        .iter()
        .map(|e| e.numero().to_string())
        .collect();
    assert_eq!(
        chaine,
        [
            "BQ1-2026-000001",
            "AC-2026-000001",
            "VT-2026-000001",
            "BQ1-2026-000002",
            "BQ1-2026-000003",
            "CA-2026-000001",
            "CA-2026-000002",
            "PA-2026-000001",
            "PA-2026-000002",
            "BQ1-2026-000004",
            "VT-2026-000002",
            "VT-2026-000003", // doublon de la facture FV-0002
            "VT-2026-000004", // sa contre-passation
            "BQ1-2026-000005",
            "IN-2026-000001",
        ]
    );
    assert_eq!(doublon.to_string(), "VT-2026-000003");
    assert_eq!(contre_passation.to_string(), "VT-2026-000004");
    for ecriture in dossier.livre.ecritures() {
        assert_eq!(ecriture.date_validation(), ecriture.date());
        assert_eq!(ecriture.justification().references().len(), 1);
    }

    // 4. Balance générale équilibrée ; contre-passation sans effet sur les soldes.
    controler_la_balance(&dossier, &doublon, &contre_passation);

    // 5. Export d'intégrité.
    let repertoire = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("demonstration-l1");
    fs::create_dir_all(&repertoire).unwrap();
    let chemin = repertoire.join("export.txt");
    let tete = exporter(&dossier, &chemin);
    let export = fs::read_to_string(&chemin).unwrap();
    // En-tête de l'export, quelle que soit la version du format (docs/integrite/).
    assert!(export.starts_with("sigma-books export v"));
    assert!(export.contains(&format!("tete={tete}\nnombre=15\n")));
    let decoupe = Decoupe::nouvelle(&export);
    assert_eq!(decoupe.ecritures.len(), 15);
    assert_eq!(decoupe.recoller(), export);

    // 6. L'auditeur vérifie l'export : intègre.
    let (code, rapport) = sigma_verifier(&chemin, &tete);
    assert_eq!(code, 0, "{rapport}");
    assert!(rapport.contains("RÉSULTAT : INTÈGRE"), "{rapport}");
    assert!(rapport.contains(&tete.to_string()), "{rapport}");

    // 7. Trois altérations, chacune dans son propre fichier, chacune constatée.
    alterer_un_octet(&repertoire, &export, &tete);
    supprimer_une_ecriture(&repertoire, &export, &tete);
    permuter_deux_ecritures(&repertoire, &export, &tete);
}
