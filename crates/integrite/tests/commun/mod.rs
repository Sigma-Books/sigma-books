//! Dossier de test : référentiel **fictif** (numéros arbitraires, sans valeur normative),
//! journaux, exercices 2026 et 2027, écritures validées par l'API publique de `compta-core`.

// Aide de test : les `unwrap` y sont voulus (une donnée de test invalide doit faire échouer le test).
#![allow(dead_code, clippy::unwrap_used)]

use std::collections::BTreeMap;

use compta_core::{
    CodeExercice, CodeJournal, CompteReference, Compteur, Contexte, Cours, Devise,
    EcritureBrouillon, EcritureValidee, Exercices, FinPremierExercice, IdTiers, Journal, Journaux,
    Justification, Ligne, Livre, Montant, NumeroCompte, NumeroEcriture, PlanComptes,
    ReferencePiece, Sens, SensNormal, TexteCourt, TypeJournal, VersionParametres,
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

pub fn xof(sens: Sens, compte: &str, montant: &str) -> Ligne {
    Ligne::en_xof(
        n(compte),
        sens,
        Montant::parse(montant, Devise::XOF).unwrap(),
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
}

impl Dossier {
    pub fn nouveau() -> Self {
        let references = [
            ("41", SensNormal::Debiteur, false),
            ("411", SensNormal::Debiteur, true),
            ("4111", SensNormal::Debiteur, false),
            ("52", SensNormal::Debiteur, false),
            ("521", SensNormal::Debiteur, false),
            ("60", SensNormal::Debiteur, false),
            ("605", SensNormal::Debiteur, false),
            ("70", SensNormal::Crediteur, false),
            ("701", SensNormal::Crediteur, false),
        ];
        let plan =
            PlanComptes::depuis_referentiel(references.into_iter().map(|(num, sens, coll)| {
                CompteReference::nouveau(n(num), t("Intitulé fictif"), sens, coll).unwrap()
            }))
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
        self.exercices.cloturer_periode(date).unwrap();
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
            Ligne::en_devise(n("521"), Sens::Debit, eur, cours).unwrap(),
            Ligne::en_devise(n("4111"), Sens::Credit, eur, cours)
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
