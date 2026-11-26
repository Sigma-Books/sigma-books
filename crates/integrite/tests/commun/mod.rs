//! Dossier de test : référentiel **fictif** (numéros arbitraires, sans valeur normative),
//! journaux, exercices 2026 et 2027, écritures validées par l'API publique de `compta-core`.

// Aide de test : les `unwrap` y sont voulus (une donnée de test invalide doit faire échouer le test).
#![allow(dead_code, clippy::unwrap_used)]

use std::collections::BTreeMap;

use compta_core::{
    CodeExercice, CodeJournal, CodeMotif, CodeRegime, CompteReference, Compteur, Contexte, Cours,
    Devise, DeviseTenue, EcritureBrouillon, EcritureValidee, EtiquetteFiscale, Exercices,
    Exigibilite, FinPremierExercice, IdTiers, Journal, Journaux, Justification, Ligne, Livre,
    Montant, NatureEtiquette, NumeroCompte, NumeroEcriture, PlanComptes, ProfilRegime,
    ReferencePiece, ReferentielFiscal, Sens, SensNormal, SensTva, TauxTva, TexteCourt, TypeJournal,
    VersionParametres,
};
use integrite::IdDossier;
use time::Date;
use time::macros::date;

pub fn n(texte: &str) -> NumeroCompte {
    NumeroCompte::parse(texte).unwrap()
}

pub fn t(texte: &str) -> TexteCourt {
    TexteCourt::parse(texte).unwrap()
}

pub fn j(code: &str) -> CodeJournal {
    CodeJournal::parse(code).unwrap()
}

pub fn devise_xof() -> Devise {
    Devise::parse("XOF").unwrap()
}

/// Devise de tenue des dossiers de test : le franc CFA, sans décimale.
pub fn tenue_xof() -> DeviseTenue {
    DeviseTenue::nouvelle(devise_xof(), 0).unwrap()
}

pub fn xof(sens: Sens, compte: &str, montant: &str) -> Ligne {
    Ligne::en_devise_tenue(
        n(compte),
        sens,
        Montant::parse(montant, devise_xof()).unwrap(),
        &tenue_xof(),
    )
    .unwrap()
}

pub fn dossier_id() -> IdDossier {
    IdDossier::parse("dossier-test").unwrap()
}

pub struct Dossier {
    plan: PlanComptes,
    journaux: Journaux,
    exercices: Exercices,
    pub livre: Livre,
    compteurs: BTreeMap<(CodeJournal, CodeExercice), Compteur>,
    tenue: DeviseTenue,
}

impl Dossier {
    pub fn nouveau() -> Self {
        Self::nouveau_en(tenue_xof())
    }

    /// Dossier de test tenu dans la devise de tenue donnée.
    pub fn nouveau_en(tenue: DeviseTenue) -> Self {
        let references = [
            ("41", SensNormal::Debiteur, false, false),
            ("411", SensNormal::Debiteur, true, false),
            ("4111", SensNormal::Debiteur, false, false),
            ("52", SensNormal::Debiteur, false, true),
            ("521", SensNormal::Debiteur, false, true),
            ("60", SensNormal::Debiteur, false, false),
            ("605", SensNormal::Debiteur, false, false),
            ("70", SensNormal::Crediteur, false, false),
            ("701", SensNormal::Crediteur, false, false),
            ("44", SensNormal::Crediteur, false, false),
            ("443", SensNormal::Crediteur, false, false),
            ("4431", SensNormal::Crediteur, false, false),
            ("445", SensNormal::Debiteur, false, false),
            ("4452", SensNormal::Debiteur, false, false),
        ];
        let plan = PlanComptes::depuis_referentiel(references.into_iter().map(
            |(num, sens, coll, tres)| {
                let reference =
                    CompteReference::nouveau(n(num), t("Intitulé fictif"), sens, coll).unwrap();
                // Dernier champ : compte admis en journal de trésorerie.
                if tres {
                    reference.admettre_en_journal_de_tresorerie()
                } else {
                    reference
                }
            },
        ))
        .unwrap();
        let mut journaux = Journaux::new();
        for (code, genre, compte) in [
            ("VT", TypeJournal::Ventes, None),
            ("BQ1", TypeJournal::Tresorerie, Some("521")),
            ("OD", TypeJournal::OperationsDiverses, None),
        ] {
            journaux
                .ajouter(Journal::nouveau(j(code), t(code), genre, compte.map(n), &plan).unwrap())
                .unwrap();
        }
        let mut exercices = Exercices::new();
        exercices
            .ouvrir_premier(date!(2026 - 01 - 01), FinPremierExercice::AnneeDeDebut)
            .unwrap();
        exercices.ouvrir_suivant().unwrap();
        Self {
            plan,
            journaux,
            exercices,
            livre: Livre::new(),
            compteurs: BTreeMap::new(),
            tenue,
        }
    }

    fn compteur(&self, journal: &CodeJournal, date: Date) -> Compteur {
        let exercice = self
            .exercices
            .exercice_contenant(date)
            .unwrap()
            .code()
            .clone();
        self.compteurs
            .get(&(journal.clone(), exercice.clone()))
            .cloned()
            .unwrap_or_else(|| Compteur::nouveau(journal.clone(), exercice))
    }

    /// Valide un brouillon (journal, date, libellé, lignes, justification facultative).
    pub fn valider(
        &mut self,
        journal: &str,
        date: Date,
        libelle: &str,
        lignes: Vec<Ligne>,
        justification: Justification,
    ) -> NumeroEcriture {
        self.valider_avec(journal, date, libelle, lignes, justification, None)
    }

    /// Comme [`Dossier::valider`], avec une version de paramètres fiscaux.
    pub fn valider_avec(
        &mut self,
        journal: &str,
        date: Date,
        libelle: &str,
        lignes: Vec<Ligne>,
        justification: Justification,
        parametres: Option<&str>,
    ) -> NumeroEcriture {
        self.valider_complet(
            journal,
            (date, None),
            libelle,
            lignes,
            justification,
            parametres,
        )
    }

    /// Opération tardive (AUDCIF art. 22 4°) : écriture datée de `date`, date de valeur
    /// comptable `date_valeur`.
    pub fn valider_tardive(
        &mut self,
        journal: &str,
        date: Date,
        date_valeur: Date,
        libelle: &str,
        lignes: Vec<Ligne>,
        justification: Justification,
    ) -> NumeroEcriture {
        self.valider_complet(
            journal,
            (date, Some(date_valeur)),
            libelle,
            lignes,
            justification,
            None,
        )
    }

    /// Clôture informatique de la période contenant la date.
    pub fn cloturer_periode(&mut self, date: Date) {
        self.exercices.cloturer_periode(date, []).unwrap();
    }

    fn valider_complet(
        &mut self,
        journal: &str,
        (date, date_valeur): (Date, Option<Date>),
        libelle: &str,
        lignes: Vec<Ligne>,
        justification: Justification,
        parametres: Option<&str>,
    ) -> NumeroEcriture {
        let mut e = EcritureBrouillon::nouvelle(j(journal), date, t(libelle));
        e.mentionner_date_valeur_comptable(date_valeur);
        for l in lignes {
            e.ajouter_ligne(l);
        }
        e.justifier(justification);
        if let Some(v) = parametres {
            e.utiliser_parametres(VersionParametres::parse(v).unwrap());
        }
        let e = e.valider_equilibre().unwrap();
        let compteur = self.compteur(e.journal(), e.date());
        let contexte = Contexte {
            plan: &self.plan,
            journaux: &self.journaux,
            exercices: &self.exercices,
            delai_cloture: compta_core::DELAI_CLOTURE_PAR_DEFAUT,
            tenue: self.tenue,
        };
        let (numero, suivant) = self
            .livre
            .valider(e, &contexte, &compteur, date)
            .map_err(|r| r.decomposer().1)
            .unwrap();
        self.compteurs.insert(
            (suivant.journal().clone(), suivant.exercice().clone()),
            suivant,
        );
        numero
    }

    pub fn contre_passer(&mut self, origine: &NumeroEcriture, date: Date) -> NumeroEcriture {
        let compteur = self.compteur(origine.journal(), date);
        let contexte = Contexte {
            plan: &self.plan,
            journaux: &self.journaux,
            exercices: &self.exercices,
            delai_cloture: compta_core::DELAI_CLOTURE_PAR_DEFAUT,
            tenue: self.tenue,
        };
        let (numero, suivant) = self
            .livre
            .contre_passer(
                origine,
                date,
                t("Contre-passation"),
                &contexte,
                &compteur,
                date,
            )
            .unwrap();
        self.compteurs.insert(
            (suivant.journal().clone(), suivant.exercice().clone()),
            suivant,
        );
        numero
    }

    pub fn ecritures(&self) -> &[EcritureValidee] {
        self.livre.ecritures()
    }
}

pub fn piece(octet: &str) -> Justification {
    Justification::pieces([ReferencePiece::parse(&octet.repeat(32)).unwrap()]).unwrap()
}

/// Dossier de référence des vecteurs figés : deux exercices, trois journaux, devise, tiers,
/// pièces multiples, version de paramètres, contre-passation et opération tardive avec date de
/// valeur comptable.
pub fn dossier_de_reference() -> Dossier {
    let mut d = Dossier::nouveau();
    let vente = d.valider(
        "VT",
        date!(2026 - 01 - 15),
        "Facture 2026-001 = client « Ndiaye & fils »",
        vec![
            xof(Sens::Debit, "4111", "118000").avec_tiers(IdTiers::parse("C001").unwrap()),
            xof(Sens::Credit, "701", "118000"),
        ],
        Justification::pieces([
            ReferencePiece::parse(&"0a".repeat(32)).unwrap(),
            ReferencePiece::parse(&"1b".repeat(32)).unwrap(),
        ])
        .unwrap(),
    );
    let eur = Montant::parse("228.67", Devise::parse("EUR").unwrap()).unwrap();
    let cours = Cours::parse("655.957").unwrap();
    d.valider_avec(
        "BQ1",
        date!(2026 - 02 - 03),
        "Règlement",
        vec![
            Ligne::en_devise(n("521"), Sens::Debit, eur, cours, &tenue_xof()).unwrap(),
            Ligne::en_devise(n("4111"), Sens::Credit, eur, cours, &tenue_xof())
                .unwrap()
                .avec_tiers(IdTiers::parse("C001").unwrap()),
        ],
        piece("2c"),
        Some("sn-2026.1"),
    );
    d.valider(
        "OD",
        date!(2026 - 03 - 31),
        "Écriture d'ajustement",
        vec![
            xof(Sens::Debit, "605", "2500"),
            xof(Sens::Credit, "521", "2500"),
        ],
        piece("4e"),
    );
    d.contre_passer(&vente, date!(2026 - 04 - 01));
    d.valider(
        "VT",
        date!(2027 - 01 - 10),
        "Facture 2027-001",
        vec![
            xof(Sens::Debit, "4111", "50000").avec_tiers(IdTiers::parse("C002").unwrap()),
            xof(Sens::Credit, "701", "50000"),
        ],
        piece("3d"),
    );
    // Opération tardive (format canonique v2) : janvier 2026 clôturé, frais de janvier
    // enregistrés au 1er février avec leur date de valeur comptable.
    d.cloturer_periode(date!(2026 - 01 - 31));
    d.valider_tardive(
        "OD",
        date!(2026 - 02 - 01),
        date!(2026 - 01 - 28),
        "Frais bancaires de janvier, relevé reçu tardivement",
        vec![
            xof(Sens::Debit, "605", "1500"),
            xof(Sens::Credit, "521", "1500"),
        ],
        piece("5f"),
    );
    d
}

/// Référentiel fiscal **de test** : il admet les codes des vecteurs publiés (écriture v3, export v4
/// et v5 : `IMP18`, `IMP_ENC`, `EXO361`, `EXO361-n`), qui datent d'avant les codes génériques de l'ADR 0030.
/// Les vecteurs publiés ne changent pas (ADR 0030 point 6) et `integrite` ne dépend d'aucune crate
/// de paramètres fiscaux : le vérificateur n'a pas de liste de codes.
struct ReferentielDesVecteurs;

impl ReferentielFiscal for ReferentielDesVecteurs {
    fn profil_regime(&self, code: &CodeRegime) -> Option<ProfilRegime> {
        match code.as_str() {
            "IMP18" => Some(ProfilRegime::nouveau(
                false,
                true,
                vec![TauxTva::parse("18").unwrap()],
                vec![Exigibilite::Debit, Exigibilite::Encaissement],
            )),
            "IMP_ENC" => Some(ProfilRegime::nouveau(
                false,
                true,
                vec![TauxTva::parse("18").unwrap(), TauxTva::parse("10").unwrap()],
                vec![Exigibilite::Encaissement],
            )),
            "EXO361" => Some(ProfilRegime::nouveau(true, false, vec![], vec![])),
            _ => None,
        }
    }

    fn motif_admis(&self, regime: &CodeRegime, motif: &CodeMotif) -> bool {
        regime.as_str() == "EXO361" && motif.as_str().starts_with("EXO361-")
    }
}

/// Étiquette fiscale contrôlée par le référentiel des vecteurs (régime, taux, exigibilité, motif).
pub fn etiquette(
    nature: NatureEtiquette,
    sens: SensTva,
    regime: &str,
    taux: Option<&str>,
    exigibilite: Option<Exigibilite>,
    motif: Option<&str>,
) -> EtiquetteFiscale {
    EtiquetteFiscale::nouvelle(
        nature,
        sens,
        CodeRegime::parse(regime).unwrap(),
        taux.map(|t| TauxTva::parse(t).unwrap()),
        exigibilite,
        motif.map(|m| CodeMotif::parse(m).unwrap()),
        &ReferentielDesVecteurs,
    )
    .unwrap()
}

/// Dossier de référence du vecteur v4 : le dossier de référence (6 écritures, inchangées), puis
/// quatre écritures avec étiquette fiscale : vente taxée à 18 %, vente exonérée (art. 361-4),
/// contre-passation de la vente taxée, achat à l'encaissement avec date de valeur comptable.
pub fn dossier_de_reference_v4() -> Dossier {
    use NatureEtiquette::{Base, Taxe};
    let mut d = dossier_de_reference();
    let vente = d.valider(
        "VT",
        date!(2026 - 02 - 10),
        "Vente taxable à 18 %",
        vec![
            xof(Sens::Debit, "4111", "118000").avec_tiers(IdTiers::parse("C001").unwrap()),
            xof(Sens::Credit, "701", "100000").avec_etiquette(etiquette(
                Base,
                SensTva::Collectee,
                "IMP18",
                Some("18"),
                Some(Exigibilite::Debit),
                None,
            )),
            xof(Sens::Credit, "4431", "18000").avec_etiquette(etiquette(
                Taxe,
                SensTva::Collectee,
                "IMP18",
                Some("18"),
                Some(Exigibilite::Debit),
                None,
            )),
        ],
        piece("6a"),
    );
    d.valider(
        "VT",
        date!(2026 - 02 - 20),
        "Frais de scolarité = exonération",
        vec![
            xof(Sens::Debit, "4111", "50000").avec_tiers(IdTiers::parse("C002").unwrap()),
            xof(Sens::Credit, "701", "50000").avec_etiquette(etiquette(
                Base,
                SensTva::Collectee,
                "EXO361",
                None,
                None,
                Some("EXO361-4"),
            )),
        ],
        piece("7b"),
    );
    d.contre_passer(&vente, date!(2026 - 04 - 02));
    d.valider_tardive(
        "OD",
        date!(2026 - 02 - 01),
        date!(2026 - 01 - 30),
        "Achat payé sur fonds publics",
        vec![
            xof(Sens::Debit, "605", "1000").avec_etiquette(etiquette(
                Base,
                SensTva::Deductible,
                "IMP_ENC",
                Some("18"),
                Some(Exigibilite::Encaissement),
                None,
            )),
            xof(Sens::Debit, "4452", "180").avec_etiquette(etiquette(
                Taxe,
                SensTva::Deductible,
                "IMP_ENC",
                Some("18"),
                Some(Exigibilite::Encaissement),
                None,
            )),
            xof(Sens::Credit, "521", "1180"),
        ],
        piece("8c"),
    );
    d
}

/// Export (v4) du dossier de référence v4, sans jeton.
pub fn export_de_reference_v4() -> Vec<u8> {
    let d = dossier_de_reference_v4();
    let mut sortie = Vec::new();
    integrite::exporter(&dossier_id(), d.ecritures(), &[], &mut sortie).unwrap();
    sortie
}

/// Export (v3) du dossier de référence, sans jeton.
pub fn export_de_reference() -> Vec<u8> {
    export_avec_jetons(&[])
}

/// Export (v3) du dossier de référence, avec les jetons donnés.
pub fn export_avec_jetons(jetons: &[integrite::JetonExporte]) -> Vec<u8> {
    let d = dossier_de_reference();
    let mut sortie = Vec::new();
    integrite::exporter(&dossier_id(), d.ecritures(), jetons, &mut sortie).unwrap();
    sortie
}

/// Jetons réels du dossier de référence (obtenus le 2026-11-18) : `Sectigo` sur les 3 premières
/// écritures, `DigiCert` sur les 5, `Sectigo` sur les 6 (vecteur `export-v3-reference.txt`).
pub fn jetons_de_reference() -> Vec<integrite::JetonExporte> {
    let tete = |e: &str| integrite::Empreinte::parse(e).unwrap();
    vec![
        integrite::JetonExporte {
            nombre: 3,
            tete: tete("44c5d55be80588e6fe3616a3678ad23098dfb178a5ffe3dcaf94e5bdca0120e3"),
            der: include_bytes!("../vecteurs/horodatage/reference-sectigo-3.tst").to_vec(),
        },
        integrite::JetonExporte {
            nombre: 5,
            tete: tete("116649fdda079968b86eb09e4e5f165e5f6215da22ab559dc00060c95e6a3c35"),
            der: include_bytes!("../vecteurs/horodatage/reference-digicert-5.tst").to_vec(),
        },
        integrite::JetonExporte {
            nombre: 6,
            tete: tete("c55a13aabadadccb3b92249cc2241ddbc5f4c2040dc01cbbba542202844666f9"),
            der: include_bytes!("../vecteurs/horodatage/reference-sectigo-6.tst").to_vec(),
        },
    ]
}

/// Identifiant du dossier du vecteur v5 (tenu en CDF).
pub fn dossier_cdf_id() -> IdDossier {
    IdDossier::parse("dossier-cdf").unwrap()
}

/// Devise de tenue du vecteur v5 : le franc congolais (RDC), deux décimales.
pub fn tenue_cdf() -> DeviseTenue {
    DeviseTenue::nouvelle(Devise::parse("CDF").unwrap(), 2).unwrap()
}

fn cdf(sens: Sens, compte: &str, montant: &str) -> Ligne {
    Ligne::en_devise_tenue(
        n(compte),
        sens,
        Montant::parse(montant, Devise::parse("CDF").unwrap()).unwrap(),
        &tenue_cdf(),
    )
    .unwrap()
}

/// Dossier de référence du vecteur v5 (ADR 0027), tenu en CDF : une vente taxée avec centimes
/// et étiquettes fiscales, une opération en EUR dont la contre-valeur est arrondie au centime,
/// puis la contre-passation de la vente.
pub fn dossier_de_reference_v5() -> Dossier {
    use NatureEtiquette::{Base, Taxe};
    let mut d = Dossier::nouveau_en(tenue_cdf());
    let vente = d.valider(
        "VT",
        date!(2026 - 01 - 15),
        "Vente taxable à 18 %",
        vec![
            cdf(Sens::Debit, "4111", "118000.55").avec_tiers(IdTiers::parse("C001").unwrap()),
            cdf(Sens::Credit, "701", "100000.47").avec_etiquette(etiquette(
                Base,
                SensTva::Collectee,
                "IMP18",
                Some("18"),
                Some(Exigibilite::Debit),
                None,
            )),
            cdf(Sens::Credit, "4431", "18000.08").avec_etiquette(etiquette(
                Taxe,
                SensTva::Collectee,
                "IMP18",
                Some("18"),
                Some(Exigibilite::Debit),
                None,
            )),
        ],
        piece("9a"),
    );
    let eur = Montant::parse("100.55", Devise::parse("EUR").unwrap()).unwrap();
    let cours = Cours::parse("2827.3333").unwrap();
    d.valider(
        "OD",
        date!(2026 - 02 - 01),
        "Achat en euros payé depuis la banque",
        vec![
            Ligne::en_devise(n("605"), Sens::Debit, eur, cours, &tenue_cdf()).unwrap(),
            Ligne::en_devise(n("521"), Sens::Credit, eur, cours, &tenue_cdf()).unwrap(),
        ],
        piece("8b"),
    );
    d.contre_passer(&vente, date!(2026 - 03 - 05));
    d
}

/// Export (v5) du dossier de référence v5, sans jeton.
pub fn export_de_reference_v5() -> Vec<u8> {
    let d = dossier_de_reference_v5();
    let mut sortie = Vec::new();
    integrite::exporter(&dossier_cdf_id(), d.ecritures(), &[], &mut sortie).unwrap();
    sortie
}
