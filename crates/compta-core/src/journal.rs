//! Journaux d'un dossier (spec §5.2 ; §6.2, invariant 7).
//!
//! Les journaux par défaut sont des **données** (`data/journaux-par-defaut.txt`), pas du
//! code. Un journal de trésorerie est associé à son compte de trésorerie, qui doit être un
//! compte de mouvement actif du plan ; les autres journaux n'ont pas de compte associé.
//!
//! Le **type** d'un journal et son compte de trésorerie ne sont modifiables par aucune
//! fonction ; seul le code peut changer, tant qu'aucune écriture n'utilise le journal (il
//! figure dans les numéros définitifs).

use std::collections::BTreeMap;
use std::fmt;

use crate::compte::NumeroCompte;
use crate::plan::{ErreurPlan, PlanComptes, Usage};
use crate::texte::{ErreurTexte, TexteCourt};

/// Longueur maximale d'un code de journal.
const CODE_MAX: usize = 8;

/// Données des journaux par défaut.
const JOURNAUX_PAR_DEFAUT: &str = include_str!("../data/journaux-par-defaut.txt");

/// Type d'un journal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TypeJournal {
    /// Achats.
    Achats,
    /// Ventes.
    Ventes,
    /// Trésorerie (banque, mobile money, caisse) : associé à un compte de trésorerie.
    Tresorerie,
    /// Opérations diverses (dont paie et inventaire).
    OperationsDiverses,
    /// À-nouveaux : généré uniquement, jamais saisi.
    ANouveaux,
}

impl TypeJournal {
    /// Analyse le nom d'un type, tel qu'écrit dans les données.
    ///
    /// # Errors
    /// [`ErreurJournal::TypeInconnu`].
    pub fn parse(texte: &str) -> Result<Self, ErreurJournal> {
        match texte {
            "achats" => Ok(Self::Achats),
            "ventes" => Ok(Self::Ventes),
            "tresorerie" => Ok(Self::Tresorerie),
            "operations_diverses" => Ok(Self::OperationsDiverses),
            "a_nouveaux" => Ok(Self::ANouveaux),
            _ => Err(ErreurJournal::TypeInconnu(texte.to_owned())),
        }
    }

    /// Vrai si une écriture peut y être saisie (tous les types sauf les à-nouveaux).
    #[must_use]
    pub fn accepte_la_saisie(self) -> bool {
        self != Self::ANouveaux
    }
}

/// Erreurs sur les journaux.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ErreurJournal {
    /// Code de journal invalide.
    #[error("code de journal invalide : « {0} » (1 à {CODE_MAX} caractères parmi A-Z et 0-9)")]
    CodeInvalide(String),
    /// Type de journal inconnu.
    #[error("type de journal inconnu : « {0} »")]
    TypeInconnu(String),
    /// Intitulé invalide.
    #[error("intitulé de journal invalide : {0}")]
    Intitule(#[from] ErreurTexte),
    /// Un journal de trésorerie doit avoir un compte de trésorerie.
    #[error("le journal de trésorerie {0} n'a pas de compte de trésorerie")]
    CompteTresorerieManquant(CodeJournal),
    /// Seuls les journaux de trésorerie ont un compte associé.
    #[error("le journal {0} n'est pas un journal de trésorerie : pas de compte associé")]
    CompteNonAttendu(CodeJournal),
    /// Le compte de trésorerie n'est pas utilisable.
    #[error("compte de trésorerie du journal {0} : {1}")]
    CompteTresorerie(CodeJournal, ErreurPlan),
    /// Le code est déjà utilisé.
    #[error("le journal {0} existe déjà")]
    Doublon(CodeJournal),
    /// Journal inconnu.
    #[error("le journal {0} n'existe pas")]
    Inconnu(CodeJournal),
    /// Le code d'un journal qui a des écritures ne change plus (il figure dans leurs numéros).
    #[error("le journal {0} a des écritures : son code ne peut plus changer")]
    JournalMouvemente(CodeJournal),
    /// Ligne invalide dans les données des journaux par défaut.
    #[error("journaux par défaut, ligne {0} : {1}")]
    Donnees(usize, String),
}

/// Code de journal (`AC`, `BQ1`) : 1 à 8 caractères parmi `A-Z` et `0-9`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CodeJournal(String);

impl CodeJournal {
    /// Analyse un code de journal.
    ///
    /// # Errors
    /// [`ErreurJournal::CodeInvalide`].
    pub fn parse(texte: &str) -> Result<Self, ErreurJournal> {
        let valide = (1..=CODE_MAX).contains(&texte.len())
            && texte
                .bytes()
                .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit());
        if valide {
            Ok(Self(texte.to_owned()))
        } else {
            Err(ErreurJournal::CodeInvalide(texte.to_owned()))
        }
    }

    /// Le code.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for CodeJournal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Modèle de journal (données par défaut), sans compte de trésorerie.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModeleJournal {
    code: CodeJournal,
    genre: TypeJournal,
    intitule: TexteCourt,
}

impl ModeleJournal {
    /// Code du modèle.
    #[must_use]
    pub fn code(&self) -> &CodeJournal {
        &self.code
    }

    /// Type du modèle.
    #[must_use]
    pub fn genre(&self) -> TypeJournal {
        self.genre
    }

    /// Crée le journal du dossier. Un journal de trésorerie reçoit ici son compte.
    ///
    /// # Errors
    /// Voir [`Journal::nouveau`].
    pub fn instancier(
        &self,
        compte_tresorerie: Option<NumeroCompte>,
        plan: &PlanComptes,
    ) -> Result<Journal, ErreurJournal> {
        Journal::nouveau(
            self.code.clone(),
            self.intitule.clone(),
            self.genre,
            compte_tresorerie,
            plan,
        )
    }
}

/// Modèles des journaux par défaut (spec §5.2), lus depuis les données du crate.
///
/// # Errors
/// [`ErreurJournal::Donnees`] si le fichier de données est mal formé (vérifié par les tests).
pub fn journaux_par_defaut() -> Result<Vec<ModeleJournal>, ErreurJournal> {
    analyser_modeles(JOURNAUX_PAR_DEFAUT)
}

fn analyser_modeles(donnees: &str) -> Result<Vec<ModeleJournal>, ErreurJournal> {
    let mut modeles: Vec<ModeleJournal> = Vec::new();
    for (index, ligne) in donnees.lines().enumerate() {
        let numero = index + 1;
        let ligne = ligne.trim();
        if ligne.is_empty() || ligne.starts_with('#') {
            continue;
        }
        let erreur = |e: &dyn fmt::Display| ErreurJournal::Donnees(numero, e.to_string());
        let mut champs = ligne.splitn(3, ';');
        let (Some(code), Some(genre), Some(intitule)) =
            (champs.next(), champs.next(), champs.next())
        else {
            return Err(erreur(&"trois champs attendus : code;type;intitulé"));
        };
        let modele = ModeleJournal {
            code: CodeJournal::parse(code).map_err(|e| erreur(&e))?,
            genre: TypeJournal::parse(genre).map_err(|e| erreur(&e))?,
            intitule: TexteCourt::parse(intitule).map_err(|e| erreur(&e))?,
        };
        if modeles.iter().any(|m| m.code == modele.code) {
            return Err(erreur(&ErreurJournal::Doublon(modele.code)));
        }
        modeles.push(modele);
    }
    Ok(modeles)
}

/// Journal d'un dossier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Journal {
    code: CodeJournal,
    intitule: TexteCourt,
    genre: TypeJournal,
    compte_tresorerie: Option<NumeroCompte>,
}

impl Journal {
    /// Crée un journal. Un journal de trésorerie exige un compte de mouvement actif du plan ;
    /// les autres n'ont pas de compte associé.
    ///
    /// # Errors
    /// Compte manquant, non attendu ou inutilisable.
    pub fn nouveau(
        code: CodeJournal,
        intitule: TexteCourt,
        genre: TypeJournal,
        compte_tresorerie: Option<NumeroCompte>,
        plan: &PlanComptes,
    ) -> Result<Self, ErreurJournal> {
        match (genre, &compte_tresorerie) {
            (TypeJournal::Tresorerie, None) => {
                return Err(ErreurJournal::CompteTresorerieManquant(code));
            }
            (TypeJournal::Tresorerie, Some(compte)) => {
                plan.compte_de_mouvement_actif(compte)
                    .map_err(|e| ErreurJournal::CompteTresorerie(code.clone(), e))?;
            }
            (_, Some(_)) => return Err(ErreurJournal::CompteNonAttendu(code)),
            (_, None) => {}
        }
        Ok(Self {
            code,
            intitule,
            genre,
            compte_tresorerie,
        })
    }

    /// Code du journal.
    #[must_use]
    pub fn code(&self) -> &CodeJournal {
        &self.code
    }

    /// Intitulé.
    #[must_use]
    pub fn intitule(&self) -> &TexteCourt {
        &self.intitule
    }

    /// Type du journal.
    #[must_use]
    pub fn genre(&self) -> TypeJournal {
        self.genre
    }

    /// Compte de trésorerie associé (journaux de trésorerie uniquement).
    #[must_use]
    pub fn compte_tresorerie(&self) -> Option<&NumeroCompte> {
        self.compte_tresorerie.as_ref()
    }
}

/// Journaux d'un dossier, par code.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Journaux {
    journaux: BTreeMap<CodeJournal, Journal>,
}

impl Journaux {
    /// Ensemble vide.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Ajoute un journal.
    ///
    /// # Errors
    /// [`ErreurJournal::Doublon`].
    pub fn ajouter(&mut self, journal: Journal) -> Result<(), ErreurJournal> {
        if self.journaux.contains_key(&journal.code) {
            return Err(ErreurJournal::Doublon(journal.code));
        }
        self.journaux.insert(journal.code.clone(), journal);
        Ok(())
    }

    /// Journal par son code.
    #[must_use]
    pub fn journal(&self, code: &CodeJournal) -> Option<&Journal> {
        self.journaux.get(code)
    }

    /// Tous les journaux, par ordre de code.
    pub fn journaux(&self) -> impl Iterator<Item = &Journal> {
        self.journaux.values()
    }

    /// Change le code d'un journal qui n'a encore aucune écriture.
    ///
    /// # Errors
    /// Journal inconnu, nouveau code déjà pris, ou journal mouvementé.
    pub fn changer_code(
        &mut self,
        ancien: &CodeJournal,
        nouveau: CodeJournal,
        usage: Usage,
    ) -> Result<(), ErreurJournal> {
        if !self.journaux.contains_key(ancien) {
            return Err(ErreurJournal::Inconnu(ancien.clone()));
        }
        if usage == Usage::Mouvemente {
            return Err(ErreurJournal::JournalMouvemente(ancien.clone()));
        }
        if self.journaux.contains_key(&nouveau) {
            return Err(ErreurJournal::Doublon(nouveau));
        }
        if let Some(mut journal) = self.journaux.remove(ancien) {
            journal.code = nouveau.clone();
            self.journaux.insert(nouveau, journal);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan::tests::{n, plan_de_test, t};

    fn c(texte: &str) -> CodeJournal {
        CodeJournal::parse(texte).unwrap()
    }

    #[test]
    fn donnees_par_defaut_conformes_a_la_spec() {
        let modeles = journaux_par_defaut().unwrap();
        let codes: Vec<&str> = modeles.iter().map(|m| m.code().as_str()).collect();
        assert_eq!(codes, ["AC", "VT", "CA", "PA", "OD", "IN", "AN"]);
        let an = modeles.iter().find(|m| m.code().as_str() == "AN").unwrap();
        assert!(!an.genre().accepte_la_saisie());
        let ca = modeles.iter().find(|m| m.code().as_str() == "CA").unwrap();
        assert_eq!(ca.genre(), TypeJournal::Tresorerie);
    }

    #[test]
    fn donnees_mal_formees() {
        assert!(matches!(
            analyser_modeles("AC;achats"),
            Err(ErreurJournal::Donnees(1, _))
        ));
        assert!(matches!(
            analyser_modeles("ac;achats;A"),
            Err(ErreurJournal::Donnees(1, _))
        ));
        assert!(matches!(
            analyser_modeles("AC;autre;A"),
            Err(ErreurJournal::Donnees(1, _))
        ));
        assert!(matches!(
            analyser_modeles("# c\nAC;achats;A\nAC;ventes;B"),
            Err(ErreurJournal::Donnees(3, _))
        ));
        assert_eq!(
            analyser_modeles("AC;achats;Achats; divers").unwrap().len(),
            1
        );
    }

    #[test]
    fn codes() {
        for code in ["AC", "BQ1", "MM12", "ABCDEFGH"] {
            assert!(CodeJournal::parse(code).is_ok(), "{code}");
        }
        for code in ["", "ac", "A-C", "ABCDEFGHI", "É"] {
            assert!(CodeJournal::parse(code).is_err(), "{code}");
        }
    }

    #[test]
    fn journal_de_tresorerie_et_son_compte() {
        let mut plan = plan_de_test();
        plan.ajouter_sous_compte(n("521-BICIS"), t("Banque"))
            .unwrap();
        let bq = Journal::nouveau(
            c("BQ1"),
            t("Banque"),
            TypeJournal::Tresorerie,
            Some(n("521-BICIS")),
            &plan,
        )
        .unwrap();
        assert_eq!(bq.compte_tresorerie(), Some(&n("521-BICIS")));

        let sans = Journal::nouveau(c("BQ2"), t("B"), TypeJournal::Tresorerie, None, &plan);
        assert_eq!(sans, Err(ErreurJournal::CompteTresorerieManquant(c("BQ2"))));
        let regroupement = Journal::nouveau(
            c("BQ3"),
            t("B"),
            TypeJournal::Tresorerie,
            Some(n("521")),
            &plan,
        );
        assert_eq!(
            regroupement,
            Err(ErreurJournal::CompteTresorerie(
                c("BQ3"),
                ErreurPlan::Regroupement(n("521"))
            ))
        );
        let achats = Journal::nouveau(c("AC"), t("A"), TypeJournal::Achats, Some(n("6052")), &plan);
        assert_eq!(achats, Err(ErreurJournal::CompteNonAttendu(c("AC"))));
    }

    #[test]
    fn instanciation_et_changement_de_code() {
        let plan = plan_de_test();
        let mut journaux = Journaux::new();
        for modele in journaux_par_defaut().unwrap() {
            let compte = (modele.genre() == TypeJournal::Tresorerie).then(|| n("6052"));
            journaux
                .ajouter(modele.instancier(compte, &plan).unwrap())
                .unwrap();
        }
        assert_eq!(journaux.journaux().count(), 7);
        let ac = journaux.journal(&c("AC")).unwrap().clone();
        assert_eq!(journaux.ajouter(ac), Err(ErreurJournal::Doublon(c("AC"))));

        assert_eq!(
            journaux.changer_code(&c("AC"), c("ACH"), Usage::Mouvemente),
            Err(ErreurJournal::JournalMouvemente(c("AC")))
        );
        assert_eq!(
            journaux.changer_code(&c("AC"), c("VT"), Usage::SansEcriture),
            Err(ErreurJournal::Doublon(c("VT")))
        );
        assert_eq!(
            journaux.changer_code(&c("ZZ"), c("ZY"), Usage::SansEcriture),
            Err(ErreurJournal::Inconnu(c("ZZ")))
        );
        journaux
            .changer_code(&c("AC"), c("ACH"), Usage::SansEcriture)
            .unwrap();
        assert!(journaux.journal(&c("AC")).is_none());
        assert_eq!(journaux.journal(&c("ACH")).unwrap().code(), &c("ACH"));
    }
}
