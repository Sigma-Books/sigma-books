//! Dossier de test : référentiel **fictif** (numéros arbitraires, sans valeur normative),
//! journaux, exercices 2026 et 2027, livre et compteurs. Utilise uniquement l'API publique.

// Aide de test : les `unwrap` y sont voulus (une donnée de test invalide doit faire échouer le test).
#![allow(dead_code, clippy::unwrap_used)]

use std::collections::BTreeMap;

use compta_core::{
    CodeExercice, CodeJournal, CompteReference, Compteur, Contexte, Devise, EcritureBrouillon,
    EcritureEquilibree, ErreurValidation, Exercices, FinPremierExercice, IdTiers, Journal,
    Journaux, Justification, Ligne, Livre, Montant, NumeroCompte, NumeroEcriture, PlanComptes,
    ReferencePiece, Sens, SensNormal, TexteCourt, TypeJournal,
};
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

pub fn xof(texte: &str) -> Montant {
    Montant::parse(texte, Devise::XOF).unwrap()
}

pub fn ligne(compte: &str, sens: Sens, montant: &str) -> Ligne {
    Ligne::en_xof(n(compte), sens, xof(montant)).unwrap()
}

pub fn ligne_tiers(compte: &str, sens: Sens, montant: &str, tiers: &str) -> Ligne {
    ligne(compte, sens, montant).avec_tiers(IdTiers::parse(tiers).unwrap())
}

pub fn piece() -> Justification {
    Justification::pieces([ReferencePiece::parse(&"ab".repeat(32)).unwrap()]).unwrap()
}

/// Brouillon justifié par une pièce.
pub fn brouillon(
    journal: &str,
    date: Date,
    libelle: &str,
    lignes: Vec<Ligne>,
) -> EcritureBrouillon {
    let mut e = EcritureBrouillon::nouvelle(j(journal), date, t(libelle));
    for l in lignes {
        e.ajouter_ligne(l);
    }
    e.justifier(piece());
    e
}

pub fn equilibree(
    journal: &str,
    date: Date,
    libelle: &str,
    lignes: Vec<Ligne>,
) -> EcritureEquilibree {
    brouillon(journal, date, libelle, lignes)
        .valider_equilibre()
        .unwrap()
}

pub struct Dossier {
    pub plan: PlanComptes,
    pub journaux: Journaux,
    pub exercices: Exercices,
    pub livre: Livre,
    pub compteurs: BTreeMap<(CodeJournal, CodeExercice), Compteur>,
}

impl Dossier {
    pub fn nouveau() -> Self {
        let references = [
            ("40", SensNormal::Crediteur, false, false),
            ("401", SensNormal::Crediteur, true, false),
            ("4011", SensNormal::Crediteur, false, false),
            ("41", SensNormal::Debiteur, false, false),
            ("411", SensNormal::Debiteur, true, false),
            ("4111", SensNormal::Debiteur, false, false),
            ("52", SensNormal::Debiteur, false, true),
            ("521", SensNormal::Debiteur, false, true),
            ("57", SensNormal::Debiteur, false, true),
            ("571", SensNormal::Debiteur, false, true),
            ("60", SensNormal::Debiteur, false, false),
            ("605", SensNormal::Debiteur, false, false),
            ("6052", SensNormal::Debiteur, false, false),
            ("70", SensNormal::Crediteur, false, false),
            ("701", SensNormal::Crediteur, false, false),
            ("7011", SensNormal::Crediteur, false, false),
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
        for (code, genre, compte, cloture) in [
            ("AC", TypeJournal::Achats, None, false),
            ("VT", TypeJournal::Ventes, None, false),
            ("BQ1", TypeJournal::Tresorerie, Some("521"), false),
            ("CA", TypeJournal::Tresorerie, Some("571"), false),
            ("PA", TypeJournal::OperationsDiverses, None, false),
            ("OD", TypeJournal::OperationsDiverses, None, true),
            ("IN", TypeJournal::OperationsDiverses, None, true),
            ("AN", TypeJournal::ANouveaux, None, false),
        ] {
            let mut journal =
                Journal::nouveau(j(code), t(code), genre, compte.map(n), &plan).unwrap();
            if cloture {
                journal = journal.admettre_en_cloture().unwrap();
            }
            journaux.ajouter(journal).unwrap();
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

    pub fn contexte(&self) -> Contexte<'_> {
        Contexte {
            plan: &self.plan,
            journaux: &self.journaux,
            exercices: &self.exercices,
        }
    }

    fn compteur(&self, journal: &CodeJournal, date: Date) -> Compteur {
        let exercice = self.exercices.exercice_contenant(date).map_or_else(
            || CodeExercice::parse("2026").unwrap(),
            |e| e.code().clone(),
        );
        self.compteurs
            .get(&(journal.clone(), exercice.clone()))
            .cloned()
            .unwrap_or_else(|| Compteur::nouveau(journal.clone(), exercice))
    }

    fn enregistrer(&mut self, compteur: Compteur) {
        self.compteurs.insert(
            (compteur.journal().clone(), compteur.exercice().clone()),
            compteur,
        );
    }

    /// Valide une écriture équilibrée avec le compteur courant de son journal et exercice,
    /// le jour même de sa date.
    pub fn valider(
        &mut self,
        ecriture: EcritureEquilibree,
    ) -> Result<NumeroEcriture, ErreurValidation> {
        let jour = ecriture.date();
        self.valider_le(ecriture, jour)
    }

    /// Valide une écriture à une date de validation donnée.
    pub fn valider_le(
        &mut self,
        ecriture: EcritureEquilibree,
        date_validation: Date,
    ) -> Result<NumeroEcriture, ErreurValidation> {
        let compteur = self.compteur(ecriture.journal(), ecriture.date());
        let contexte = Contexte {
            plan: &self.plan,
            journaux: &self.journaux,
            exercices: &self.exercices,
        };
        let (numero, suivant) = self
            .livre
            .valider(ecriture, &contexte, &compteur, date_validation)
            .map_err(|refus| refus.decomposer().1)?;
        self.enregistrer(suivant);
        Ok(numero)
    }

    /// Contre-passe une écriture validée.
    pub fn contre_passer(
        &mut self,
        origine: &NumeroEcriture,
        date: Date,
    ) -> Result<NumeroEcriture, ErreurValidation> {
        let compteur = self.compteur(origine.journal(), date);
        let contexte = Contexte {
            plan: &self.plan,
            journaux: &self.journaux,
            exercices: &self.exercices,
        };
        let (numero, suivant) = self.livre.contre_passer(
            origine,
            date,
            t("Contre-passation"),
            &contexte,
            &compteur,
            date,
        )?;
        self.enregistrer(suivant);
        Ok(numero)
    }
}
