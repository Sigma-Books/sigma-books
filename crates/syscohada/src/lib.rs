//! Référentiel normatif du SYSCOHADA révisé : liste officielle des comptes (spec §5.1).
//!
//! Les données vivent dans `data/comptes.toml`, embarqué à la compilation (`include_str!`) et
//! **validé à chaque chargement** : numéros bien formés et uniques, intitulés valides, parent
//! présent pour chaque compte, classe connue, page dans la plage de la source. Chaque compte
//! cite sa source (document et page PDF). Méthode d'extraction, coquilles corrigées et règles
//! du sens normal : `crates/syscohada/README.md`.
//!
//! Le référentiel alimente le plan de comptes d'un dossier ([`compta_core::PlanComptes`]) par
//! [`Referentiel::comptes_reference`] : `compta-core` ne connaît aucun numéro de compte.
//!
//! Le compte de trésorerie des journaux par défaut (CA : caisse) est aussi une donnée de ce
//! crate, `data/journaux.toml` : [`Referentiel::journaux_par_defaut`]. Les comptes de TVA (443,
//! 444, 445) sont désignés par `data/comptes-tva.toml` : [`Referentiel::comptes_de_tva`].

#![forbid(unsafe_code)]

use std::collections::BTreeMap;

use compta_core::{
    CodeJournal, CompteReference, ErreurJournal, ErreurNumeroCompte, ErreurPlan, ErreurTexte,
    NumeroCompte, PlanComptes, SensNormal, TexteCourt,
};
use serde::Deserialize;

mod journaux;
mod tva;

pub use journaux::CompteDeJournal;
pub use tva::{CompteTva, ComptesTva, RoleCompteTva};

/// Données embarquées : liste officielle des comptes.
const COMPTES_TOML: &str = include_str!("../data/comptes.toml");

/// Pages PDF de la liste des comptes dans la source (Titre VII, section 3).
const PAGE_MIN: u16 = 219;
const PAGE_MAX: u16 = 272;

/// Pages PDF des fiches « Contenu et fonctionnement des comptes » (Titre VII, ch. 3).
const PAGE_FONCTIONNEMENT_MIN: u16 = 274;
const PAGE_FONCTIONNEMENT_MAX: u16 = 539;

/// Nature d'un compte, déduite de l'intitulé officiel de sa classe.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Nature {
    /// Compte de bilan (classes 1 à 5).
    Bilan,
    /// Compte de charges (classe 6, charges de la classe 8).
    Charge,
    /// Compte de produits (classe 7, produits de la classe 8).
    Produit,
    /// Engagement hors bilan.
    HorsBilan,
    /// Comptabilité analytique de gestion.
    Analytique,
}

/// Erreurs de chargement ou de validation du référentiel.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ErreurReferentiel {
    /// Le fichier n'est pas un TOML conforme au format attendu.
    #[error("référentiel illisible : {0}")]
    Syntaxe(String),
    /// Numéro de compte mal formé.
    #[error("numéro de compte invalide « {numero} » : {erreur}")]
    Numero {
        /// Numéro tel qu'écrit dans les données.
        numero: String,
        /// Cause.
        erreur: ErreurNumeroCompte,
    },
    /// Un compte du référentiel porte un segment libre.
    #[error("le compte {0} n'est pas un numéro officiel (segment libre)")]
    NonOfficiel(NumeroCompte),
    /// Intitulé vide, trop long ou contenant un caractère de contrôle.
    #[error("intitulé invalide pour {numero} : {erreur}")]
    Intitule {
        /// Compte ou classe concerné.
        numero: String,
        /// Cause.
        erreur: ErreurTexte,
    },
    /// Le même numéro apparaît deux fois.
    #[error("compte en double : {0}")]
    Doublon(NumeroCompte),
    /// Le compte parent (numéro sans son dernier chiffre) est absent.
    #[error("parent absent pour le compte {0}")]
    ParentAbsent(NumeroCompte),
    /// Classe absente, en double ou hors de 1 à 9.
    #[error("classe invalide ou absente : {0}")]
    Classe(u8),
    /// Journal par défaut invalide, ou journaux par défaut non instanciables.
    #[error("journaux par défaut : {0}")]
    Journal(ErreurJournal),
    /// Désignation des comptes de TVA invalide (`data/comptes-tva.toml`).
    #[error("comptes de TVA : {0}")]
    ComptesTva(String),
    /// Le compte d'un journal par défaut ne cite pas sa source ou son motif.
    #[error("compte du journal {0} : source ou motif manquant")]
    SourceManquante(CodeJournal),
    /// Le compte d'un journal par défaut n'est pas dans la liste officielle.
    #[error("compte du journal {code} : {compte} n'est pas un compte de la liste officielle")]
    CompteDeJournalInconnu {
        /// Journal.
        code: CodeJournal,
        /// Compte cité.
        compte: NumeroCompte,
    },
    /// Le compte d'un journal par défaut n'est pas admis en journal de trésorerie
    /// ([`Referentiel::admis_en_journal_de_tresorerie`]).
    #[error("compte du journal {code} : {compte} n'est pas admis en journal de trésorerie")]
    CompteDeJournalNonAdmis {
        /// Journal.
        code: CodeJournal,
        /// Compte cité.
        compte: NumeroCompte,
    },
    /// Le compte d'un journal par défaut est subdivisé dans la liste officielle : on ne saisit
    /// que sur un compte sans subdivision.
    #[error("compte du journal {code} : {compte} est subdivisé dans la liste officielle")]
    CompteDeJournalSubdivise {
        /// Journal.
        code: CodeJournal,
        /// Compte cité.
        compte: NumeroCompte,
    },
    /// Marque `journal_tresorerie` sans `page_fonctionnement` dans la plage des fiches de
    /// fonctionnement des comptes, page sans marque, ou exclusion (`false`) d'un compte
    /// qu'aucun ascendant n'admet.
    #[error(
        "compte {0} : `journal_tresorerie` exige `page_fonctionnement` entre \
         {PAGE_FONCTIONNEMENT_MIN} et {PAGE_FONCTIONNEMENT_MAX}, et inversement ; une exclusion \
         exige un ascendant admis"
    )]
    MarqueJournalTresorerie(NumeroCompte),
    /// Marque `tiers` sans `page_tiers` dans la plage des fiches de fonctionnement, ou
    /// inversement ; liste vide, type inconnu ou en double.
    #[error(
        "compte {0} : `tiers` exige `page_tiers` entre {PAGE_FONCTIONNEMENT_MIN} et \
         {PAGE_FONCTIONNEMENT_MAX}, et inversement, avec des types connus et distincts"
    )]
    MarqueTiers(NumeroCompte),
    /// Page hors de la plage de la liste officielle.
    #[error("page {page} hors de la source ({PAGE_MIN} à {PAGE_MAX}) pour {numero}")]
    Page {
        /// Compte ou classe concerné.
        numero: String,
        /// Page indiquée.
        page: u16,
    },
}

/// Classe du plan (1 à 9).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Classe {
    numero: u8,
    intitule: TexteCourt,
    page: u16,
}

impl Classe {
    /// Numéro de la classe (1 à 9).
    #[must_use]
    pub fn numero(&self) -> u8 {
        self.numero
    }

    /// Intitulé officiel.
    #[must_use]
    pub fn intitule(&self) -> &TexteCourt {
        &self.intitule
    }

    /// Page PDF de la source.
    #[must_use]
    pub fn page(&self) -> u16 {
        self.page
    }
}

/// Compte de la liste officielle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompteOfficiel {
    numero: NumeroCompte,
    intitule: TexteCourt,
    nature: Nature,
    sens: SensNormal,
    collectif: bool,
    page: u16,
    note: Option<String>,
    marque_journal_tresorerie: Option<(bool, u16)>,
    marque_tiers: Option<(Vec<String>, u16)>,
}

/// Types de tiers que les données peuvent admettre sur un compte (spec §5.3).
pub const TYPES_TIERS: [&str; 5] = ["client", "fournisseur", "salarie", "associe", "autre"];

impl CompteOfficiel {
    /// Numéro officiel (2 à 4 chiffres).
    #[must_use]
    pub fn numero(&self) -> &NumeroCompte {
        &self.numero
    }

    /// Intitulé officiel.
    #[must_use]
    pub fn intitule(&self) -> &TexteCourt {
        &self.intitule
    }

    /// Classe (premier chiffre du numéro).
    #[must_use]
    pub fn classe(&self) -> u8 {
        self.numero.classe()
    }

    /// Nature (bilan, charge, produit, hors bilan, analytique).
    #[must_use]
    pub fn nature(&self) -> Nature {
        self.nature
    }

    /// Sens normal du solde (convention de Sigma Books, voir le README du crate).
    #[must_use]
    pub fn sens(&self) -> SensNormal {
        self.sens
    }

    /// Vrai pour un compte collectif de tiers déclaré dans les données (les sous-comptes en
    /// héritent dans le plan du dossier).
    #[must_use]
    pub fn est_collectif(&self) -> bool {
        self.collectif
    }

    /// Page PDF de la source où figure le compte.
    #[must_use]
    pub fn page(&self) -> u16 {
        self.page
    }

    /// Note sur l'établissement du numéro ou de l'intitulé (coquille du texte officiel).
    #[must_use]
    pub fn note(&self) -> Option<&str> {
        self.note.as_deref()
    }

    /// Marque `journal_tresorerie` portée par ce compte lui-même : admission (`true`) ou
    /// exclusion explicite (`false`), et page PDF de la fiche de fonctionnement qui la fonde.
    /// Les sous-comptes héritent de la marque la plus proche
    /// ([`Referentiel::admis_en_journal_de_tresorerie`]).
    #[must_use]
    pub fn marque_journal_tresorerie(&self) -> Option<(bool, u16)> {
        self.marque_journal_tresorerie
    }

    /// Marque `tiers` portée par ce compte lui-même : types de tiers admis sur ce compte et
    /// ses sous-comptes ([`TYPES_TIERS`]), et page PDF de la fiche de fonctionnement qui la
    /// fonde. Les sous-comptes héritent de la marque la plus proche
    /// ([`Referentiel::types_de_tiers_admis`]).
    #[must_use]
    pub fn marque_tiers(&self) -> Option<(&[String], u16)> {
        self.marque_tiers
            .as_ref()
            .map(|(types, page)| (types.as_slice(), *page))
    }
}

/// Liste officielle des comptes, validée.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Referentiel {
    document: String,
    version: String,
    classes: BTreeMap<u8, Classe>,
    comptes: BTreeMap<NumeroCompte, CompteOfficiel>,
}

/// Forme brute du fichier de données.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Brut {
    source: SourceBrute,
    classe: Vec<ClasseBrute>,
    compte: Vec<CompteBrut>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SourceBrute {
    document: String,
    version: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ClasseBrute {
    numero: u8,
    intitule: String,
    page: u16,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CompteBrut {
    numero: String,
    intitule: String,
    page: u16,
    nature: Nature,
    sens: SensBrut,
    #[serde(default)]
    collectif: bool,
    note: Option<String>,
    journal_tresorerie: Option<bool>,
    page_fonctionnement: Option<u16>,
    tiers: Option<Vec<String>>,
    page_tiers: Option<u16>,
}

#[derive(Deserialize, Clone, Copy)]
#[serde(rename_all = "snake_case")]
enum SensBrut {
    Debiteur,
    Crediteur,
    Variable,
}

impl From<SensBrut> for SensNormal {
    fn from(sens: SensBrut) -> Self {
        match sens {
            SensBrut::Debiteur => Self::Debiteur,
            SensBrut::Crediteur => Self::Crediteur,
            SensBrut::Variable => Self::Variable,
        }
    }
}

fn verifier_page(numero: &str, page: u16) -> Result<u16, ErreurReferentiel> {
    if (PAGE_MIN..=PAGE_MAX).contains(&page) {
        Ok(page)
    } else {
        Err(ErreurReferentiel::Page {
            numero: numero.to_owned(),
            page,
        })
    }
}

fn texte(numero: &str, intitule: &str) -> Result<TexteCourt, ErreurReferentiel> {
    TexteCourt::parse(intitule).map_err(|erreur| ErreurReferentiel::Intitule {
        numero: numero.to_owned(),
        erreur,
    })
}

impl Referentiel {
    /// Charge et valide la liste officielle embarquée.
    ///
    /// # Errors
    /// [`ErreurReferentiel`] si les données embarquées sont invalides (garanti par les tests).
    pub fn officiel() -> Result<Self, ErreurReferentiel> {
        Self::depuis_toml(COMPTES_TOML)
    }

    /// Charge et valide un référentiel au format de `data/comptes.toml`.
    ///
    /// # Errors
    /// [`ErreurReferentiel`] décrivant la première anomalie rencontrée.
    pub fn depuis_toml(texte_toml: &str) -> Result<Self, ErreurReferentiel> {
        let brut: Brut =
            toml::from_str(texte_toml).map_err(|e| ErreurReferentiel::Syntaxe(e.to_string()))?;

        let mut classes = BTreeMap::new();
        for c in brut.classe {
            if !(1..=9).contains(&c.numero) {
                return Err(ErreurReferentiel::Classe(c.numero));
            }
            let cle = c.numero.to_string();
            let classe = Classe {
                numero: c.numero,
                intitule: texte(&cle, &c.intitule)?,
                page: verifier_page(&cle, c.page)?,
            };
            if classes.insert(c.numero, classe).is_some() {
                return Err(ErreurReferentiel::Classe(c.numero));
            }
        }

        let mut comptes = BTreeMap::new();
        for c in brut.compte {
            let numero =
                NumeroCompte::parse(&c.numero).map_err(|erreur| ErreurReferentiel::Numero {
                    numero: c.numero.clone(),
                    erreur,
                })?;
            if !numero.est_officiel() {
                return Err(ErreurReferentiel::NonOfficiel(numero));
            }
            if !classes.contains_key(&numero.classe()) {
                return Err(ErreurReferentiel::Classe(numero.classe()));
            }
            let marque_journal_tresorerie = match (c.journal_tresorerie, c.page_fonctionnement) {
                (None, None) => None,
                (Some(admis), Some(page))
                    if (PAGE_FONCTIONNEMENT_MIN..=PAGE_FONCTIONNEMENT_MAX).contains(&page) =>
                {
                    Some((admis, page))
                }
                _ => return Err(ErreurReferentiel::MarqueJournalTresorerie(numero)),
            };
            let marque_tiers = match (c.tiers, c.page_tiers) {
                (None, None) => None,
                (Some(types), Some(page))
                    if (PAGE_FONCTIONNEMENT_MIN..=PAGE_FONCTIONNEMENT_MAX).contains(&page)
                        && !types.is_empty()
                        && types.iter().all(|t| TYPES_TIERS.contains(&t.as_str()))
                        && types
                            .iter()
                            .enumerate()
                            .all(|(i, t)| !types[..i].contains(t)) =>
                {
                    Some((types, page))
                }
                _ => return Err(ErreurReferentiel::MarqueTiers(numero)),
            };
            let compte = CompteOfficiel {
                marque_journal_tresorerie,
                marque_tiers,
                intitule: texte(&c.numero, &c.intitule)?,
                page: verifier_page(&c.numero, c.page)?,
                nature: c.nature,
                sens: c.sens.into(),
                collectif: c.collectif,
                note: c.note,
                numero: numero.clone(),
            };
            if comptes.insert(numero.clone(), compte).is_some() {
                return Err(ErreurReferentiel::Doublon(numero));
            }
        }

        for numero in comptes.keys() {
            if let Some(parent) = numero.parent()
                && !comptes.contains_key(&parent)
            {
                return Err(ErreurReferentiel::ParentAbsent(numero.clone()));
            }
        }

        let referentiel = Self {
            document: brut.source.document,
            version: brut.source.version,
            classes,
            comptes,
        };
        // Une exclusion explicite n'a de sens que sous un compte admis.
        for compte in referentiel.comptes.values() {
            if compte
                .marque_journal_tresorerie
                .is_some_and(|(admis, _)| !admis)
                && !compte
                    .numero
                    .parent()
                    .is_some_and(|p| referentiel.admis_en_journal_de_tresorerie(&p))
            {
                return Err(ErreurReferentiel::MarqueJournalTresorerie(
                    compte.numero.clone(),
                ));
            }
        }
        Ok(referentiel)
    }

    /// Document source (texte officiel).
    #[must_use]
    pub fn document(&self) -> &str {
        &self.document
    }

    /// Version du référentiel.
    #[must_use]
    pub fn version(&self) -> &str {
        &self.version
    }

    /// Classes 1 à 9.
    pub fn classes(&self) -> impl Iterator<Item = &Classe> {
        self.classes.values()
    }

    /// Classe, si elle existe.
    #[must_use]
    pub fn classe(&self, numero: u8) -> Option<&Classe> {
        self.classes.get(&numero)
    }

    /// Comptes, dans l'ordre des numéros.
    pub fn comptes(&self) -> impl Iterator<Item = &CompteOfficiel> {
        self.comptes.values()
    }

    /// Compte officiel, s'il existe.
    #[must_use]
    pub fn compte(&self, numero: &NumeroCompte) -> Option<&CompteOfficiel> {
        self.comptes.get(numero)
    }

    /// Vrai si le numéro peut être le compte d'un journal de trésorerie : la marque
    /// `journal_tresorerie` la plus proche, sur lui-même ou sur ses ascendants de la liste
    /// officielle, l'admet (une exclusion explicite l'emporte sur l'admission héritée).
    #[must_use]
    pub fn admis_en_journal_de_tresorerie(&self, numero: &NumeroCompte) -> bool {
        std::iter::successors(Some(numero.clone()), NumeroCompte::parent)
            .find_map(|n| {
                self.comptes
                    .get(&n)
                    .and_then(|c| c.marque_journal_tresorerie)
            })
            .is_some_and(|(admis, _)| admis)
    }

    /// Types de tiers admis sur le numéro comme compte collectif d'un tiers : ceux de la
    /// marque `tiers` la plus proche, sur lui-même ou sur ses ascendants de la liste officielle.
    /// Vide si aucune marque ne s'applique.
    #[must_use]
    pub fn types_de_tiers_admis(&self, numero: &NumeroCompte) -> &[String] {
        std::iter::successors(Some(numero.clone()), NumeroCompte::parent)
            .find_map(|n| self.comptes.get(&n).and_then(|c| c.marque_tiers.as_ref()))
            .map_or(&[], |(types, _)| types.as_slice())
    }

    /// Nombre de comptes.
    #[must_use]
    pub fn len(&self) -> usize {
        self.comptes.len()
    }

    /// Vrai si le référentiel ne contient aucun compte.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.comptes.is_empty()
    }

    /// Comptes sous la forme d'entrée du plan de comptes d'un dossier (`compta-core`). Les
    /// comptes admis en journal de trésorerie y sont marqués comme tels.
    ///
    /// # Errors
    /// [`ErreurPlan`] si un compte n'est pas officiel (impossible après validation).
    pub fn comptes_reference(&self) -> Result<Vec<CompteReference>, ErreurPlan> {
        self.comptes
            .values()
            .map(|c| {
                let reference = CompteReference::nouveau(
                    c.numero.clone(),
                    c.intitule.clone(),
                    c.sens,
                    c.collectif,
                )?;
                Ok(if self.admis_en_journal_de_tresorerie(&c.numero) {
                    reference.admettre_en_journal_de_tresorerie()
                } else {
                    reference
                })
            })
            .collect()
    }

    /// Plan de comptes initial d'un dossier, construit à partir de ce référentiel.
    ///
    /// # Errors
    /// [`ErreurPlan`] si le plan ne peut être construit (impossible après validation).
    pub fn plan_comptes(&self) -> Result<PlanComptes, ErreurPlan> {
        PlanComptes::depuis_referentiel(self.comptes_reference()?)
    }
}

#[cfg(test)]
mod tests;
