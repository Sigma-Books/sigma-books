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
//!
//! Le **pack pays** (`data/pays.toml`, [`PackPays`]) donne les valeurs par défaut d'un dossier
//! selon son pays : devise de tenue et échelle, fuseau, langue, types d'identifiants légaux
//! (ADR 0027). Le plan de comptes reste commun à tous les pays.
//!
//! Tiers et trésorerie (L3-12) : la marque `lettrable` dit où le lettrage est admis
//! ([`Referentiel::est_lettrable`]) ; la marque `type_tresorerie` donne le compte par défaut de
//! chaque type de compte de trésorerie ([`Referentiel::compte_par_defaut_tresorerie`]) ; les
//! rôles de comptes désignés par le dossier (compte d'attente des relevés, prélèvement et taxe
//! sur les transferts) sont lus de `data/roles-comptes.toml` ([`Referentiel::roles_de_comptes`]).

#![forbid(unsafe_code)]

use std::collections::BTreeMap;

use compta_core::{
    CodeJournal, CompteReference, ErreurJournal, ErreurNumeroCompte, ErreurPlan, ErreurTexte,
    NumeroCompte, PlanComptes, SensNormal, TexteCourt,
};
use serde::Deserialize;

mod journaux;
mod pays;
mod roles;
mod tresorerie;
mod tva;

pub use journaux::CompteDeJournal;
pub use pays::{
    ErreurPack, Fuseau, InfoDevise, LANGUE_LONGUEUR_MAX, PackPays, Pays, TypeIdentifiant,
    langue_valide,
};
pub use roles::{Confirmation, DefinitionRole, RoleCompte, RolesComptes, SuggestionCompte};
pub use tresorerie::TypeTresorerie;
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
    /// Marque `lettrable` sans `page_lettrable` dans la plage des fiches de fonctionnement, ou
    /// inversement ; valeur autre que `true` ; marque sous un ascendant déjà lettrable ; compte
    /// lettrable (ou l'un de ses sous-comptes) admis en journal de trésorerie.
    #[error(
        "compte {0} : `lettrable = true` exige `page_lettrable` entre {PAGE_FONCTIONNEMENT_MIN} \
         et {PAGE_FONCTIONNEMENT_MAX}, et inversement, sans ascendant déjà lettrable ni compte \
         admis en journal de trésorerie"
    )]
    MarqueLettrable(NumeroCompte),
    /// Marque `type_tresorerie` sans `page_type_tresorerie` dans la plage des fiches de
    /// fonctionnement, ou inversement ; type inconnu ou déjà attribué ; compte non admis en
    /// journal de trésorerie.
    #[error(
        "compte {0} : `type_tresorerie` exige un type connu et non encore attribué, \
         `page_type_tresorerie` entre {PAGE_FONCTIONNEMENT_MIN} et {PAGE_FONCTIONNEMENT_MAX}, \
         et un compte admis en journal de trésorerie"
    )]
    MarqueTypeTresorerie(NumeroCompte),
    /// Marque `virement_interne` sans `page_virement_interne` dans la plage des fiches de
    /// fonctionnement, ou inversement ; valeur autre que `true` ; compte non lettrable ou admis en
    /// journal de trésorerie.
    #[error(
        "compte {0} : `virement_interne = true` exige `page_virement_interne` entre \
         {PAGE_FONCTIONNEMENT_MIN} et {PAGE_FONCTIONNEMENT_MAX}, et inversement, sur un compte \
         lettrable hors des journaux de trésorerie"
    )]
    MarqueVirementInterne(NumeroCompte),
    /// Rôles de comptes invalides (`data/roles-comptes.toml`).
    #[error("rôles de comptes : {0}")]
    RolesComptes(String),
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
    marque_lettrable: Option<u16>,
    marque_type_tresorerie: Option<(TypeTresorerie, u16)>,
    marque_virement_interne: Option<u16>,
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

    /// Marque `lettrable` portée par ce compte lui-même : page PDF de la fiche de
    /// fonctionnement qui la fonde. Les sous-comptes en héritent
    /// ([`Referentiel::est_lettrable`]).
    #[must_use]
    pub fn marque_lettrable(&self) -> Option<u16> {
        self.marque_lettrable
    }

    /// Marque `virement_interne` portée par ce compte lui-même : page PDF de la fiche qui la
    /// fonde. Héritée par les sous-comptes ([`Referentiel::est_virement_interne`]).
    #[must_use]
    pub fn marque_virement_interne(&self) -> Option<u16> {
        self.marque_virement_interne
    }

    /// Marque `type_tresorerie` portée par ce compte : il est le compte par défaut de ce type
    /// de compte de trésorerie ; page PDF de la fiche de fonctionnement qui la fonde.
    #[must_use]
    pub fn marque_type_tresorerie(&self) -> Option<(TypeTresorerie, u16)> {
        self.marque_type_tresorerie
    }
}

/// Liste officielle des comptes, validée.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Referentiel {
    document: String,
    version: String,
    classes: BTreeMap<u8, Classe>,
    comptes: BTreeMap<NumeroCompte, CompteOfficiel>,
    comptes_par_defaut: BTreeMap<TypeTresorerie, NumeroCompte>,
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
    lettrable: Option<bool>,
    page_lettrable: Option<u16>,
    type_tresorerie: Option<String>,
    page_type_tresorerie: Option<u16>,
    virement_interne: Option<bool>,
    page_virement_interne: Option<u16>,
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

/// Compte officiel validé à partir de sa forme brute : numéro officiel d'une classe connue,
/// marques complètes (chacune avec la page de sa fiche de fonctionnement), intitulé et page.
fn compte_officiel(
    c: CompteBrut,
    classes: &BTreeMap<u8, Classe>,
) -> Result<CompteOfficiel, ErreurReferentiel> {
    let numero = NumeroCompte::parse(&c.numero).map_err(|erreur| ErreurReferentiel::Numero {
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
    let page_fiche =
        |page: u16| (PAGE_FONCTIONNEMENT_MIN..=PAGE_FONCTIONNEMENT_MAX).contains(&page);
    let marque_lettrable = match (c.lettrable, c.page_lettrable) {
        (None, None) => None,
        (Some(true), Some(page)) if page_fiche(page) => Some(page),
        _ => return Err(ErreurReferentiel::MarqueLettrable(numero)),
    };
    let marque_type_tresorerie = match (c.type_tresorerie, c.page_type_tresorerie) {
        (None, None) => None,
        (Some(code), Some(page)) if page_fiche(page) => match TypeTresorerie::depuis_code(&code) {
            Some(type_tresorerie) => Some((type_tresorerie, page)),
            None => return Err(ErreurReferentiel::MarqueTypeTresorerie(numero)),
        },
        _ => return Err(ErreurReferentiel::MarqueTypeTresorerie(numero)),
    };
    let marque_virement_interne = match (c.virement_interne, c.page_virement_interne) {
        (None, None) => None,
        (Some(true), Some(page)) if page_fiche(page) => Some(page),
        _ => return Err(ErreurReferentiel::MarqueVirementInterne(numero)),
    };
    Ok(CompteOfficiel {
        marque_journal_tresorerie,
        marque_tiers,
        marque_lettrable,
        marque_type_tresorerie,
        marque_virement_interne,
        intitule: texte(&c.numero, &c.intitule)?,
        page: verifier_page(&c.numero, c.page)?,
        nature: c.nature,
        sens: c.sens.into(),
        collectif: c.collectif,
        note: c.note,
        numero,
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
            let compte = compte_officiel(c, &classes)?;
            let numero = compte.numero.clone();
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

        let mut referentiel = Self {
            document: brut.source.document,
            version: brut.source.version,
            classes,
            comptes,
            comptes_par_defaut: BTreeMap::new(),
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
        referentiel.verifier_marque_lettrable()?;
        referentiel.verifier_marque_virement_interne()?;
        referentiel.comptes_par_defaut = referentiel.verifier_types_de_tresorerie()?;
        Ok(referentiel)
    }

    /// Une marque `lettrable` n'est pas redondante (aucun ascendant strict déjà lettrable) et ne
    /// couvre aucun compte admis en journal de trésorerie : le relevé se pointe, il ne se lettre
    /// pas (ADR 0034).
    fn verifier_marque_lettrable(&self) -> Result<(), ErreurReferentiel> {
        for compte in self.comptes.values() {
            if compte.marque_lettrable.is_none() {
                continue;
            }
            let redondante = compte
                .numero
                .parent()
                .is_some_and(|parent| self.est_lettrable(&parent));
            let couvre_la_tresorerie = self.comptes.keys().any(|n| {
                compte.numero.est_ascendant_ou_egal(n) && self.admis_en_journal_de_tresorerie(n)
            });
            if redondante || couvre_la_tresorerie {
                return Err(ErreurReferentiel::MarqueLettrable(compte.numero.clone()));
            }
        }
        Ok(())
    }

    /// Un compte de virements internes est lettrable (le retour à zéro passe par le lettrage,
    /// ADR 0034 point 2) et n'est pas admis en journal de trésorerie.
    fn verifier_marque_virement_interne(&self) -> Result<(), ErreurReferentiel> {
        for compte in self.comptes.values() {
            if compte.marque_virement_interne.is_some()
                && (!self.est_lettrable(&compte.numero)
                    || self.admis_en_journal_de_tresorerie(&compte.numero))
            {
                return Err(ErreurReferentiel::MarqueVirementInterne(
                    compte.numero.clone(),
                ));
            }
        }
        Ok(())
    }

    /// Un type de trésorerie n'a qu'un compte par défaut, admis en journal de trésorerie.
    fn verifier_types_de_tresorerie(
        &self,
    ) -> Result<BTreeMap<TypeTresorerie, NumeroCompte>, ErreurReferentiel> {
        let mut par_type = BTreeMap::new();
        for compte in self.comptes.values() {
            let Some((type_tresorerie, _)) = compte.marque_type_tresorerie else {
                continue;
            };
            if !self.admis_en_journal_de_tresorerie(&compte.numero)
                || par_type
                    .insert(type_tresorerie, compte.numero.clone())
                    .is_some()
            {
                return Err(ErreurReferentiel::MarqueTypeTresorerie(
                    compte.numero.clone(),
                ));
            }
        }
        Ok(par_type)
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

    /// Vrai si le lettrage est admis sur le numéro (officiel ou à segments libres) : lui-même
    /// ou l'un de ses ascendants de la liste officielle porte la marque `lettrable`. Le compte
    /// d'attente désigné par le dossier est lettrable par sa désignation
    /// ([`DefinitionRole::lettrable`]), non par cette marque.
    #[must_use]
    pub fn est_lettrable(&self, numero: &NumeroCompte) -> bool {
        std::iter::successors(Some(numero.clone()), NumeroCompte::parent).any(|n| {
            self.comptes
                .get(&n)
                .is_some_and(|c| c.marque_lettrable.is_some())
        })
    }

    /// Vrai si le numéro (officiel ou à segments libres) est un compte de virements internes :
    /// lui-même ou l'un de ses ascendants de la liste officielle porte la marque
    /// `virement_interne`. Un tel compte doit revenir à zéro (spec §9.3 et §15.1) : son solde non
    /// nul **avertit**, sans rien refuser (ADR 0034 point 2).
    #[must_use]
    pub fn est_virement_interne(&self, numero: &NumeroCompte) -> bool {
        std::iter::successors(Some(numero.clone()), NumeroCompte::parent).any(|n| {
            self.comptes
                .get(&n)
                .is_some_and(|c| c.marque_virement_interne.is_some())
        })
    }

    /// Compte par défaut d'un type de compte de trésorerie (marque `type_tresorerie`) : la
    /// création assistée d'un compte de trésorerie ouvre un sous-compte de ce compte. `None` si
    /// les données n'en désignent pas (le référentiel officiel les désigne tous, garanti par les
    /// tests).
    #[must_use]
    pub fn compte_par_defaut_tresorerie(
        &self,
        type_tresorerie: TypeTresorerie,
    ) -> Option<&NumeroCompte> {
        self.comptes_par_defaut.get(&type_tresorerie)
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
#[cfg(test)]
mod tests_tiers_tresorerie;
