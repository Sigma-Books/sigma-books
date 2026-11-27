//! Rôles de comptes désignés par le dossier (L3-12, ADR 0034 points 8 et 10).
//!
//! Un rôle est un emplacement que chaque dossier remplit en désignant un compte de son plan :
//! compte d'attente des lignes de relevé (B4), charge et dette du prélèvement de 0,5 % et de la
//! taxe sur les transferts d'argent (B5). `data/roles-comptes.toml` dit comment chaque rôle se
//! comporte et, quand une source le permet, quels comptes de la liste officielle sont
//! **suggérés**, « à confirmer » par l'expert-comptable ou le fiscaliste. Aucun numéro de compte
//! n'est écrit dans le code (règle absolue 9).

use compta_core::NumeroCompte;
use serde::Deserialize;

use crate::{ErreurReferentiel, Referentiel};

/// Données embarquées : rôles de comptes.
const ROLES_COMPTES_TOML: &str = include_str!("../data/roles-comptes.toml");

/// Rôle de compte désigné par le dossier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RoleCompte {
    /// Compte d'attente des lignes de relevé sans contrepartie connue (décision B4).
    AttenteReleve,
    /// Charge du prélèvement sur paiements marchands ou de la taxe sur les transferts d'argent
    /// supportée par le dossier (décision B5).
    ChargePrelevement,
    /// Dette du dossier qui collecte le prélèvement ou la taxe et la reverse (décision B5).
    DetteCollectee,
}

impl RoleCompte {
    /// Tous les rôles, dans l'ordre.
    pub const TOUS: [Self; 3] = [
        Self::AttenteReleve,
        Self::ChargePrelevement,
        Self::DetteCollectee,
    ];

    /// Code stable du rôle, tel qu'écrit dans les données.
    #[must_use]
    pub fn code(self) -> &'static str {
        match self {
            Self::AttenteReleve => "attente_releve",
            Self::ChargePrelevement => "charge_prelevement",
            Self::DetteCollectee => "dette_collectee",
        }
    }

    /// Rôle correspondant à un code, `None` s'il est inconnu.
    #[must_use]
    pub fn depuis_code(code: &str) -> Option<Self> {
        Self::TOUS.into_iter().find(|r| r.code() == code)
    }
}

/// Personne qui doit confirmer la désignation (et les suggestions) d'un rôle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Confirmation {
    /// L'expert-comptable du dossier.
    ExpertComptable,
    /// Le fiscaliste (spec §26, point 2).
    Fiscaliste,
}

/// Compte de la liste officielle suggéré pour un rôle, avec sa source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SuggestionCompte {
    numero: NumeroCompte,
    intitule: String,
    page_liste: u16,
    source: String,
}

impl SuggestionCompte {
    /// Numéro officiel suggéré.
    #[must_use]
    pub fn numero(&self) -> &NumeroCompte {
        &self.numero
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

    /// Source (documents et pages).
    #[must_use]
    pub fn source(&self) -> &str {
        &self.source
    }
}

/// Définition d'un rôle : comportement, confirmation exigée, source et suggestions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DefinitionRole {
    role: RoleCompte,
    designation_obligatoire: bool,
    lettrable: bool,
    a_confirmer_par: Confirmation,
    source: String,
    suggestions: Vec<SuggestionCompte>,
}

impl DefinitionRole {
    /// Le rôle.
    #[must_use]
    pub fn role(&self) -> RoleCompte {
        self.role
    }

    /// Vrai si la désignation est exigée avant le premier usage du rôle ; sinon elle est
    /// facultative et ne bloque aucune validation (ADR 0026).
    #[must_use]
    pub fn designation_obligatoire(&self) -> bool {
        self.designation_obligatoire
    }

    /// Vrai si le compte désigné devient lettrable à sa désignation (ADR 0032 point 2).
    #[must_use]
    pub fn lettrable(&self) -> bool {
        self.lettrable
    }

    /// Qui doit confirmer la désignation et les suggestions.
    #[must_use]
    pub fn a_confirmer_par(&self) -> Confirmation {
        self.a_confirmer_par
    }

    /// Source (documents et pages) du rôle.
    #[must_use]
    pub fn source(&self) -> &str {
        &self.source
    }

    /// Comptes suggérés, jamais appliqués d'office (vide quand aucune source ne fonde de
    /// suggestion).
    #[must_use]
    pub fn suggestions(&self) -> &[SuggestionCompte] {
        &self.suggestions
    }
}

/// Rôles de comptes validés : chaque rôle de [`RoleCompte::TOUS`] est défini une fois.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RolesComptes {
    roles: Vec<DefinitionRole>,
}

impl RolesComptes {
    /// Les définitions, dans l'ordre des données.
    #[must_use]
    pub fn definitions(&self) -> &[DefinitionRole] {
        &self.roles
    }

    /// Définition d'un rôle (toujours présente après validation).
    #[must_use]
    pub fn definition(&self, role: RoleCompte) -> Option<&DefinitionRole> {
        self.roles.iter().find(|d| d.role == role)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Brut {
    role: Vec<RoleBrut>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RoleBrut {
    code: String,
    designation_obligatoire: bool,
    lettrable: bool,
    a_confirmer_par: String,
    source: String,
    #[serde(default)]
    suggestion: Vec<SuggestionBrute>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SuggestionBrute {
    numero: String,
    intitule: String,
    page_liste: u16,
    source: String,
}

fn invalide(sujet: &str, motif: &str) -> ErreurReferentiel {
    ErreurReferentiel::RolesComptes(format!("{sujet} : {motif}"))
}

impl Referentiel {
    /// Rôles de comptes, lus de `data/roles-comptes.toml` et validés contre ce référentiel.
    ///
    /// # Errors
    /// [`ErreurReferentiel`] si les données embarquées sont invalides (garanti par les tests).
    pub fn roles_de_comptes(&self) -> Result<RolesComptes, ErreurReferentiel> {
        self.roles_de_comptes_depuis_toml(ROLES_COMPTES_TOML)
    }

    /// Rôles de comptes au format de `data/roles-comptes.toml` : codes connus, chacun défini
    /// une fois ; confirmation connue ; source renseignée ; chaque suggestion est un compte de la
    /// liste officielle, d'intitulé et de page identiques, sourcée, non répétée, non admise en
    /// journal de trésorerie et, pour un rôle lettrable, non collective (les lignes d'attente
    /// n'ont pas de tiers).
    ///
    /// # Errors
    /// [`ErreurReferentiel::RolesComptes`] décrivant la première anomalie rencontrée.
    pub fn roles_de_comptes_depuis_toml(
        &self,
        texte_toml: &str,
    ) -> Result<RolesComptes, ErreurReferentiel> {
        let brut: Brut =
            toml::from_str(texte_toml).map_err(|e| ErreurReferentiel::Syntaxe(e.to_string()))?;
        let mut roles: Vec<DefinitionRole> = Vec::new();
        for r in brut.role {
            let role = RoleCompte::depuis_code(&r.code)
                .ok_or_else(|| invalide(&r.code, "rôle inconnu"))?;
            if roles.iter().any(|d| d.role == role) {
                return Err(invalide(&r.code, "rôle défini deux fois"));
            }
            let a_confirmer_par = match r.a_confirmer_par.as_str() {
                "expert_comptable" => Confirmation::ExpertComptable,
                "fiscaliste" => Confirmation::Fiscaliste,
                _ => return Err(invalide(&r.code, "confirmation inconnue")),
            };
            if r.source.trim().is_empty() {
                return Err(invalide(&r.code, "source manquante"));
            }
            let mut suggestions: Vec<SuggestionCompte> = Vec::new();
            for s in r.suggestion {
                let sujet = format!("{} (suggestion {})", r.code, s.numero);
                let numero =
                    NumeroCompte::parse(&s.numero).map_err(|erreur| ErreurReferentiel::Numero {
                        numero: s.numero.clone(),
                        erreur,
                    })?;
                let officiel = self
                    .compte(&numero)
                    .ok_or_else(|| invalide(&sujet, "absent de la liste officielle"))?;
                if officiel.intitule().as_str() != s.intitule {
                    return Err(invalide(
                        &sujet,
                        "intitulé différent de la liste officielle",
                    ));
                }
                if officiel.page() != s.page_liste {
                    return Err(invalide(
                        &sujet,
                        "page de la liste différente de comptes.toml",
                    ));
                }
                if s.source.trim().is_empty() {
                    return Err(invalide(&sujet, "source manquante"));
                }
                if suggestions.iter().any(|x| x.numero == numero) {
                    return Err(invalide(&sujet, "suggestion en double"));
                }
                if self.admis_en_journal_de_tresorerie(&numero) {
                    return Err(invalide(&sujet, "compte admis en journal de trésorerie"));
                }
                if r.lettrable && self.est_collectif(&numero) {
                    return Err(invalide(&sujet, "compte collectif pour un rôle lettrable"));
                }
                suggestions.push(SuggestionCompte {
                    numero,
                    intitule: s.intitule,
                    page_liste: s.page_liste,
                    source: s.source,
                });
            }
            roles.push(DefinitionRole {
                role,
                designation_obligatoire: r.designation_obligatoire,
                lettrable: r.lettrable,
                a_confirmer_par,
                source: r.source,
                suggestions,
            });
        }
        if let Some(absent) = RoleCompte::TOUS
            .into_iter()
            .find(|role| !roles.iter().any(|d| d.role == *role))
        {
            return Err(invalide(absent.code(), "rôle non défini"));
        }
        Ok(RolesComptes { roles })
    }

    /// Vrai si le numéro est collectif : lui-même ou l'un de ses ascendants officiels est
    /// déclaré collectif (même héritage que le plan du dossier).
    fn est_collectif(&self, numero: &NumeroCompte) -> bool {
        std::iter::successors(Some(numero.clone()), NumeroCompte::parent).any(|n| {
            self.compte(&n)
                .is_some_and(crate::CompteOfficiel::est_collectif)
        })
    }
}
