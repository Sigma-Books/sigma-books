//! Comptes de TVA désignés dans la liste officielle (issue #189, ADR 0026).
//!
//! `compta-core` ne connaît aucun numéro de compte (règle absolue 9) : la désignation des comptes
//! de TVA, lue de `data/comptes-tva.toml`, qui cite pour chaque entrée sa source (documents et
//! pages PDF), sert la déclaration de TVA (L5) et la répartition par étiquette. Elle ne bloque
//! aucune validation (ADR 0026).

use compta_core::NumeroCompte;
use serde::Deserialize;

use crate::{
    ErreurReferentiel, PAGE_FONCTIONNEMENT_MAX, PAGE_FONCTIONNEMENT_MIN, Referentiel, SensNormal,
};

/// Données embarquées : comptes de TVA désignés.
const COMPTES_TVA_TOML: &str = include_str!("../data/comptes-tva.toml");

/// Rôle d'un compte de TVA.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoleCompteTva {
    /// TVA facturée (famille 443) : taxe collectée.
    Facturee,
    /// TVA due ou crédit de TVA (famille 444) : solde de la période, où sont virés les comptes
    /// de TVA facturée et récupérable.
    Due,
    /// TVA récupérable (famille 445) : taxe déductible.
    Recuperable,
    /// TVA « probable » des factures à établir (4435) et non parvenues (4455) : provision de fin
    /// de période, ni exigible ni déductible tant que la facture n'existe pas.
    Provision,
}

/// Un compte de TVA désigné, avec sa source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompteTva {
    numero: NumeroCompte,
    role: RoleCompteTva,
    intitule: String,
    page_liste: u16,
    page_fonctionnement: u16,
    source: String,
}

impl CompteTva {
    /// Numéro officiel du compte (ses subdivisions ont le même rôle).
    #[must_use]
    pub fn numero(&self) -> &NumeroCompte {
        &self.numero
    }

    /// Rôle : TVA facturée, due ou récupérable.
    #[must_use]
    pub fn role(&self) -> RoleCompteTva {
        self.role
    }

    /// Intitulé officiel.
    #[must_use]
    pub fn intitule(&self) -> &str {
        &self.intitule
    }

    /// Page PDF de la liste des comptes.
    #[must_use]
    pub fn page_liste(&self) -> u16 {
        self.page_liste
    }

    /// Page PDF de la fiche de fonctionnement.
    #[must_use]
    pub fn page_fonctionnement(&self) -> u16 {
        self.page_fonctionnement
    }

    /// Source (documents et pages).
    #[must_use]
    pub fn source(&self) -> &str {
        &self.source
    }
}

/// Désignation des comptes de TVA, subdivisions comprises (la plus précise l'emporte : 4435 et
/// 4455 sont des provisions au sein de 443 et 445).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComptesTva {
    entrees: Vec<CompteTva>,
}

impl ComptesTva {
    /// Les comptes désignés, avec leur source.
    #[must_use]
    pub fn entrees(&self) -> &[CompteTva] {
        &self.entrees
    }

    /// Rôle d'un compte : celui de son plus proche ascendant (ou lui-même) désigné, `None` pour
    /// tout compte étranger à la TVA.
    #[must_use]
    pub fn role(&self, compte: &NumeroCompte) -> Option<RoleCompteTva> {
        let mut courant = Some(compte.clone());
        while let Some(numero) = courant {
            if let Some(e) = self.entrees.iter().find(|e| e.numero == numero) {
                return Some(e.role);
            }
            courant = numero.parent();
        }
        None
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Brut {
    compte_tva: Vec<EntreeBrute>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EntreeBrute {
    numero: String,
    intitule: String,
    page_liste: u16,
    role: String,
    sens_normal: String,
    page_fonctionnement: u16,
    source: String,
}

fn invalide(numero: &str, motif: &str) -> ErreurReferentiel {
    ErreurReferentiel::ComptesTva(format!("compte {numero} : {motif}"))
}

impl Referentiel {
    /// Comptes de TVA désignés, lus de `data/comptes-tva.toml` et validés contre ce référentiel.
    ///
    /// # Errors
    /// [`ErreurReferentiel`] si les données embarquées sont invalides (garanti par les tests).
    pub fn comptes_de_tva(&self) -> Result<ComptesTva, ErreurReferentiel> {
        self.comptes_de_tva_depuis_toml(COMPTES_TVA_TOML)
    }

    /// Comptes de TVA au format de `data/comptes-tva.toml` : chaque numéro est un compte de la
    /// liste officielle, d'intitulé, de page et de sens normal identiques ; un seul compte par
    /// rôle de base, les quatre rôles présents, un compte imbriqué d'un autre rôle que son
    /// ascendant ; page de la fiche de
    /// fonctionnement dans la plage des fiches ; source renseignée.
    ///
    /// # Errors
    /// [`ErreurReferentiel::ComptesTva`] décrivant la première anomalie rencontrée.
    pub fn comptes_de_tva_depuis_toml(
        &self,
        texte_toml: &str,
    ) -> Result<ComptesTva, ErreurReferentiel> {
        let brut: Brut =
            toml::from_str(texte_toml).map_err(|e| ErreurReferentiel::Syntaxe(e.to_string()))?;
        let mut entrees: Vec<CompteTva> = Vec::new();
        for e in brut.compte_tva {
            let numero =
                NumeroCompte::parse(&e.numero).map_err(|erreur| ErreurReferentiel::Numero {
                    numero: e.numero.clone(),
                    erreur,
                })?;
            let officiel = self
                .compte(&numero)
                .ok_or_else(|| invalide(&e.numero, "absent de la liste officielle"))?;
            if officiel.intitule().as_str() != e.intitule {
                return Err(invalide(
                    &e.numero,
                    "intitulé différent de la liste officielle",
                ));
            }
            if officiel.page() != e.page_liste {
                return Err(invalide(
                    &e.numero,
                    "page de la liste différente de comptes.toml",
                ));
            }
            let (role, sens) = match (e.role.as_str(), e.sens_normal.as_str()) {
                ("facturee", "crediteur") => (RoleCompteTva::Facturee, SensNormal::Crediteur),
                ("due", "crediteur") => (RoleCompteTva::Due, SensNormal::Crediteur),
                ("recuperable", "debiteur") => (RoleCompteTva::Recuperable, SensNormal::Debiteur),
                ("provision", "crediteur") => (RoleCompteTva::Provision, SensNormal::Crediteur),
                ("provision", "debiteur") => (RoleCompteTva::Provision, SensNormal::Debiteur),
                _ => {
                    return Err(invalide(
                        &e.numero,
                        "rôle ou sens normal inconnu ou incohérent",
                    ));
                }
            };
            if officiel.sens() != sens {
                return Err(invalide(&e.numero, "sens normal différent de comptes.toml"));
            }
            if !(PAGE_FONCTIONNEMENT_MIN..=PAGE_FONCTIONNEMENT_MAX).contains(&e.page_fonctionnement)
            {
                return Err(invalide(&e.numero, "page de fonctionnement hors plage"));
            }
            if e.source.trim().is_empty() {
                return Err(invalide(&e.numero, "source manquante"));
            }
            // Un compte n'est désigné qu'une fois ; un rôle de base (facturée, due, récupérable)
            // n'a qu'un compte ; un compte imbriqué dans un autre doit avoir un autre rôle (4435
            // provision dans le 443 facturée).
            let doublon = entrees.iter().any(|x| {
                x.numero == numero
                    || (role != RoleCompteTva::Provision && x.role == role)
                    || ((x.numero.est_ascendant_ou_egal(&numero)
                        || numero.est_ascendant_ou_egal(&x.numero))
                        && x.role == role)
            });
            if doublon {
                return Err(invalide(&e.numero, "rôle en double ou compte imbriqué"));
            }
            entrees.push(CompteTva {
                numero,
                role,
                intitule: e.intitule,
                page_liste: e.page_liste,
                page_fonctionnement: e.page_fonctionnement,
                source: e.source,
            });
        }
        for role in [
            RoleCompteTva::Facturee,
            RoleCompteTva::Due,
            RoleCompteTva::Recuperable,
            RoleCompteTva::Provision,
        ] {
            if !entrees.iter().any(|e| e.role == role) {
                return Err(ErreurReferentiel::ComptesTva(format!(
                    "aucun compte désigné pour le rôle {role:?}"
                )));
            }
        }
        Ok(ComptesTva { entrees })
    }
}
