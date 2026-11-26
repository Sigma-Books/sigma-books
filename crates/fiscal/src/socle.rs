//! Socle commun : régimes génériques et ce que `compta-core` en suppose.

use std::collections::BTreeSet;

use compta_core::{CodeRegime, Exigibilite};
use serde::Deserialize;

use crate::erreur::ErreurFiscale;
use crate::types::RoleTaux;

const SOCLE_TOML: &str = include_str!("../data/socle/socle.toml");
const REGIMES_TOML: &str = include_str!("../data/socle/regimes-tva.toml");

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FichierSocle {
    roles_taux: Vec<String>,
    retention: RetentionSocle,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RetentionSocle {
    plancher_annees: u16,
    source: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FichierRegimes {
    regime: Vec<RegimeSocleBrut>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RegimeSocleBrut {
    code: String,
    nature: String,
    exonere: bool,
    ligne_taxe: bool,
    roles_taux: Vec<String>,
    exigibilites_etiquette: Vec<String>,
}

/// Régime générique du socle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegimeSocle {
    code: CodeRegime,
    nature: String,
    exonere: bool,
    ligne_taxe: bool,
    roles_taux: Vec<RoleTaux>,
    exigibilites_etiquette: Vec<Exigibilite>,
}

impl RegimeSocle {
    /// Code stable.
    #[must_use]
    pub fn code(&self) -> &CodeRegime {
        &self.code
    }
    /// Nature, en une phrase.
    #[must_use]
    pub fn nature(&self) -> &str {
        &self.nature
    }
    /// Régime d'exonération (motif obligatoire).
    #[must_use]
    pub fn est_exonere(&self) -> bool {
        self.exonere
    }
    /// Le régime comporte une ligne de taxe.
    #[must_use]
    pub fn porte_une_ligne_de_taxe(&self) -> bool {
        self.ligne_taxe
    }
    /// Rôles de taux admis (vide : pas de taux).
    #[must_use]
    pub fn roles_taux(&self) -> &[RoleTaux] {
        &self.roles_taux
    }
    /// Exigibilités que porte l'étiquette.
    #[must_use]
    pub fn exigibilites_etiquette(&self) -> &[Exigibilite] {
        &self.exigibilites_etiquette
    }
}

/// Socle : régimes génériques, rôles de taux, plancher de rétention.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Socle {
    roles: Vec<RoleTaux>,
    regimes: Vec<RegimeSocle>,
    plancher_retention: u16,
    source_plancher: String,
}

impl Socle {
    /// Charge et valide le socle embarqué.
    ///
    /// # Errors
    /// [`ErreurFiscale`] si les données embarquées sont invalides.
    pub fn officiel() -> Result<Self, ErreurFiscale> {
        Self::depuis_textes(SOCLE_TOML, REGIMES_TOML)
    }

    /// Charge depuis des textes TOML (tests de rejet).
    ///
    /// # Errors
    /// [`ErreurFiscale`] : syntaxe, code mal formé ou en double, rôle ou exigibilité inconnus.
    pub fn depuis_textes(socle: &str, regimes: &str) -> Result<Self, ErreurFiscale> {
        let socle: FichierSocle =
            toml::from_str(socle).map_err(|e| ErreurFiscale::Syntaxe(e.to_string()))?;
        let regimes: FichierRegimes =
            toml::from_str(regimes).map_err(|e| ErreurFiscale::Syntaxe(e.to_string()))?;
        let roles: Vec<RoleTaux> = socle.roles_taux.into_iter().map(RoleTaux).collect();
        if socle.retention.source.trim().is_empty() {
            return Err(ErreurFiscale::SansSource("retention".to_owned()));
        }
        let mut vus = BTreeSet::new();
        let mut liste = Vec::new();
        for r in regimes.regime {
            let code = CodeRegime::parse(&r.code).map_err(ErreurFiscale::Code)?;
            if !vus.insert(r.code.clone()) {
                return Err(ErreurFiscale::CodeEnDouble(r.code));
            }
            if r.nature.trim().is_empty() {
                return Err(ErreurFiscale::SansLibelle(r.code));
            }
            let roles_taux = r
                .roles_taux
                .into_iter()
                .map(|n| {
                    roles
                        .iter()
                        .find(|x| x.0 == n)
                        .cloned()
                        .ok_or(ErreurFiscale::RoleInconnu(n))
                })
                .collect::<Result<Vec<_>, _>>()?;
            let exigibilites_etiquette = r
                .exigibilites_etiquette
                .iter()
                .map(|e| {
                    Exigibilite::depuis_code(e).ok_or_else(|| ErreurFiscale::Incoherente {
                        code: r.code.clone(),
                        motif: "exigibilité d'étiquette inconnue",
                    })
                })
                .collect::<Result<Vec<_>, _>>()?;
            if r.exonere
                && (r.ligne_taxe || !roles_taux.is_empty() || !exigibilites_etiquette.is_empty())
            {
                return Err(ErreurFiscale::Incoherente {
                    code: r.code,
                    motif: "un régime exonéré n'a ni ligne de taxe, ni taux, ni exigibilité d'étiquette",
                });
            }
            liste.push(RegimeSocle {
                code,
                nature: r.nature,
                exonere: r.exonere,
                ligne_taxe: r.ligne_taxe,
                roles_taux,
                exigibilites_etiquette,
            });
        }
        Ok(Self {
            roles,
            regimes: liste,
            plancher_retention: socle.retention.plancher_annees,
            source_plancher: socle.retention.source,
        })
    }

    /// Régimes du socle, dans l'ordre du fichier.
    #[must_use]
    pub fn regimes(&self) -> &[RegimeSocle] {
        &self.regimes
    }

    /// Régime du socle par code.
    #[must_use]
    pub fn regime(&self, code: &str) -> Option<&RegimeSocle> {
        self.regimes.iter().find(|r| r.code.as_str() == code)
    }

    /// Rôle de taux du socle par nom.
    #[must_use]
    pub fn role(&self, nom: &str) -> Option<&RoleTaux> {
        self.roles.iter().find(|r| r.0 == nom)
    }

    /// Rôles de taux du socle.
    #[must_use]
    pub fn roles(&self) -> &[RoleTaux] {
        &self.roles
    }

    /// Plancher de rétention des pièces, en années.
    #[must_use]
    pub fn plancher_retention(&self) -> u16 {
        self.plancher_retention
    }

    /// Source du plancher.
    #[must_use]
    pub fn source_plancher(&self) -> &str {
        &self.source_plancher
    }
}
