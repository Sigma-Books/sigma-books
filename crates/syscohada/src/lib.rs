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

#![forbid(unsafe_code)]

use std::collections::BTreeMap;

use compta_core::{
    CompteReference, ErreurNumeroCompte, ErreurPlan, ErreurTexte, NumeroCompte, PlanComptes,
    SensNormal, TexteCourt,
};
use serde::Deserialize;

/// Données embarquées : liste officielle des comptes.
const COMPTES_TOML: &str = include_str!("../data/comptes.toml");

/// Pages PDF de la liste des comptes dans la source (Titre VII, section 3).
const PAGE_MIN: u16 = 219;
const PAGE_MAX: u16 = 272;

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
}

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
            let compte = CompteOfficiel {
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

        Ok(Self {
            document: brut.source.document,
            version: brut.source.version,
            classes,
            comptes,
        })
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

    /// Comptes sous la forme d'entrée du plan de comptes d'un dossier (`compta-core`).
    ///
    /// # Errors
    /// [`ErreurPlan`] si un compte n'est pas officiel (impossible après validation).
    pub fn comptes_reference(&self) -> Result<Vec<CompteReference>, ErreurPlan> {
        self.comptes
            .values()
            .map(|c| {
                CompteReference::nouveau(c.numero.clone(), c.intitule.clone(), c.sens, c.collectif)
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
