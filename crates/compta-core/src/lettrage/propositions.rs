//! Propositions de lettrage automatique (spec §10.1 ; ADR 0032, point 7).
//!
//! Règles **déterministes**, appliquées dans l'ordre de la spec, par groupe (compte, tiers
//! résolu), sur les lignes au résiduel non nul triées par échéance effective (à défaut date
//! comptable) puis rang de chaîne : le résultat ne dépend pas de l'ordre d'ajout des lignes.
//!
//! 1. **Référence** : la référence normalisée d'une ligne est contenue dans celle d'une ligne
//!    de sens opposé, **candidat unique** (OCA `account_bank_statement_line.py:980-1060`) ;
//! 2. **Montant exact** : résiduels égaux, sens opposés, **candidat unique** (OCA, `COUNT = 1`) ;
//! 3. **Combinaison bornée** : plusieurs lignes d'un sens dont les résiduels somment exactement
//!    le résiduel d'une ligne de l'autre sens (somme de factures = règlement). Fonctionnalité
//!    Sigma, absente des moteurs (Tryton n'a qu'une approximation gloutonne, `move.py:1828-1871`).
//!    Recherche en profondeur dans l'ordre fixé, bornée en **nombre de lignes** et en
//!    **budget d'évaluations** par groupe ; la première combinaison trouvée est proposée.
//!
//! Une ligne entre dans au plus une proposition. Chaque proposition est contrôlée comme un
//! lettrage manuel ([`RegistreLettrage::imputer`]) : elle ne mélange jamais deux comptes, deux
//! tiers ni un groupe déséquilibré. Rien n'est inscrit : la proposition est validée par un
//! utilisateur, ou appliquée automatiquement si le paramètre du dossier l'autorise (API).

use std::collections::BTreeMap;

use rust_decimal::Decimal;

use super::{ErreurLettrage, IdLigne, Lien, RegistreLettrage};
use crate::ecriture::{IdTiers, Sens};

/// Bornes du lettrage automatique (F2.16 : déni de service par combinaison exponentielle).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Bornes {
    lignes_max: usize,
    budget: u64,
}

impl Bornes {
    /// Plus grand nombre de lignes admis dans une combinaison.
    pub const LIGNES_MAX_ADMIS: usize = 32;

    /// Valeurs initiales de l'ADR 0032 (point 7) : 6 lignes, 100 000 évaluations par groupe.
    pub const PAR_DEFAUT: Self = Self {
        lignes_max: 6,
        budget: 100_000,
    };

    /// Bornes : `lignes_max` lignes au plus par combinaison (2 à [`Self::LIGNES_MAX_ADMIS`]),
    /// `budget` évaluations au plus par groupe et par règle (au moins 1).
    ///
    /// # Errors
    /// [`ErreurLettrage::BornesInvalides`].
    pub fn nouvelles(lignes_max: usize, budget: u64) -> Result<Self, ErreurLettrage> {
        if !(2..=Self::LIGNES_MAX_ADMIS).contains(&lignes_max) || budget == 0 {
            return Err(ErreurLettrage::BornesInvalides);
        }
        Ok(Self { lignes_max, budget })
    }

    /// Nombre maximal de lignes d'une combinaison (cible comprise).
    #[must_use]
    pub fn lignes_max(&self) -> usize {
        self.lignes_max
    }

    /// Budget d'évaluations par groupe et par règle.
    #[must_use]
    pub fn budget(&self) -> u64 {
        self.budget
    }
}

impl Default for Bornes {
    fn default() -> Self {
        Self::PAR_DEFAUT
    }
}

/// Règle qui a produit une proposition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Regle {
    /// Même référence.
    Reference,
    /// Même montant exact.
    MontantExact,
    /// Combinaison de montants.
    Combinaison,
}

/// Proposition de lettrage : lignes et liens qu'un lettrage manuel de ces lignes produirait.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Proposition {
    regle: Regle,
    tiers: Option<IdTiers>,
    lignes: Vec<IdLigne>,
    liens: Vec<Lien>,
}

impl Proposition {
    /// Règle appliquée.
    #[must_use]
    pub fn regle(&self) -> Regle {
        self.regle
    }

    /// Tiers résolu du groupe.
    #[must_use]
    pub fn tiers(&self) -> Option<&IdTiers> {
        self.tiers.as_ref()
    }

    /// Lignes, dans l'ordre des identifiants : la sélection à passer à
    /// [`RegistreLettrage::lettrer`].
    #[must_use]
    pub fn lignes(&self) -> &[IdLigne] {
        &self.lignes
    }

    /// Liens prévus.
    #[must_use]
    pub fn liens(&self) -> &[Lien] {
        &self.liens
    }
}

/// Bilan d'un groupe (compte, tiers) : évaluations consommées et épuisement du budget.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BilanGroupe {
    tiers: Option<IdTiers>,
    evaluations_reference: u64,
    evaluations_combinaison: u64,
    epuise: bool,
}

impl BilanGroupe {
    /// Tiers résolu du groupe.
    #[must_use]
    pub fn tiers(&self) -> Option<&IdTiers> {
        self.tiers.as_ref()
    }

    /// Évaluations de la règle de référence (au plus le budget).
    #[must_use]
    pub fn evaluations_reference(&self) -> u64 {
        self.evaluations_reference
    }

    /// Évaluations de la règle de combinaison (au plus le budget).
    #[must_use]
    pub fn evaluations_combinaison(&self) -> u64 {
        self.evaluations_combinaison
    }

    /// Vrai si une règle a épuisé son budget : la recherche du groupe est incomplète.
    #[must_use]
    pub fn epuise(&self) -> bool {
        self.epuise
    }
}

/// Résultat du lettrage automatique.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Propositions {
    propositions: Vec<Proposition>,
    groupes: Vec<BilanGroupe>,
}

impl Propositions {
    /// Propositions, par groupe puis par règle.
    #[must_use]
    pub fn propositions(&self) -> &[Proposition] {
        &self.propositions
    }

    /// Bilan de chaque groupe, dans l'ordre des tiers.
    #[must_use]
    pub fn groupes(&self) -> &[BilanGroupe] {
        &self.groupes
    }
}

/// Ligne ouverte d'un groupe.
struct Ouverte<'a> {
    id: IdLigne,
    sens: Sens,
    residuel: Decimal,
    reference: Option<&'a str>,
}

/// Budget d'évaluations épuisé.
struct Epuise;

/// Issue d'une recherche bornée.
enum Issue {
    Trouvee,
    Absente,
    Epuisee,
}

impl RegistreLettrage {
    /// Calcule les propositions de lettrage automatique, sans rien inscrire.
    #[must_use]
    pub fn proposer(&self, bornes: &Bornes) -> Propositions {
        let mut groupes: BTreeMap<Option<&IdTiers>, Vec<_>> = BTreeMap::new();
        for ligne in self.lignes.values() {
            let residuel = self.residuel(ligne.id).unwrap_or_default();
            if residuel > Decimal::ZERO {
                groupes.entry(ligne.tiers.as_ref()).or_default().push((
                    ligne.cle(),
                    Ouverte {
                        id: ligne.id,
                        sens: ligne.sens,
                        residuel,
                        reference: ligne.reference.as_deref(),
                    },
                ));
            }
        }
        let mut resultat = Propositions::default();
        for (tiers, mut lignes) in groupes {
            lignes.sort_by_key(|(cle, _)| *cle);
            let lignes: Vec<Ouverte<'_>> = lignes.into_iter().map(|(_, o)| o).collect();
            let mut groupe = Groupe {
                registre: self,
                tiers: tiers.cloned(),
                lignes: &lignes,
                prise: vec![false; lignes.len()],
                propositions: Vec::new(),
            };
            let (evaluations_reference, epuise_reference) = groupe.par_reference(bornes.budget);
            groupe.par_montant();
            let (evaluations_combinaison, epuise_combinaison) = groupe.par_combinaison(bornes);
            resultat.propositions.append(&mut groupe.propositions);
            resultat.groupes.push(BilanGroupe {
                tiers: tiers.cloned(),
                evaluations_reference,
                evaluations_combinaison,
                epuise: epuise_reference || epuise_combinaison,
            });
        }
        resultat
    }
}

/// Recherche dans un groupe (compte, tiers).
struct Groupe<'r, 'l> {
    registre: &'r RegistreLettrage,
    tiers: Option<IdTiers>,
    lignes: &'l [Ouverte<'l>],
    prise: Vec<bool>,
    propositions: Vec<Proposition>,
}

impl Groupe<'_, '_> {
    /// Retient la proposition si elle passe les contrôles d'un lettrage manuel.
    fn retenir(&mut self, regle: Regle, indices: &[usize]) -> bool {
        let mut lignes: Vec<IdLigne> = indices.iter().map(|&i| self.lignes[i].id).collect();
        lignes.sort_unstable();
        let Ok(liens) = self.registre.imputer(&lignes) else {
            return false;
        };
        for &i in indices {
            self.prise[i] = true;
        }
        self.propositions.push(Proposition {
            regle,
            tiers: self.tiers.clone(),
            lignes,
            liens,
        });
        true
    }

    /// Candidat libre, de sens opposé, qui satisfait `critere` : `Some` s'il est unique,
    /// `None` s'il n'y en a aucun ou plusieurs ; [`Epuise`] si `critere` épuise le budget.
    fn candidat_unique(
        &self,
        i: usize,
        candidats: impl Iterator<Item = usize>,
        mut critere: impl FnMut(&Ouverte<'_>) -> Result<bool, Epuise>,
    ) -> Result<Option<usize>, Epuise> {
        let mut trouve = None;
        for j in candidats {
            if j == i || self.prise[j] || self.lignes[j].sens == self.lignes[i].sens {
                continue;
            }
            if critere(&self.lignes[j])? {
                if trouve.is_some() {
                    return Ok(None);
                }
                trouve = Some(j);
            }
        }
        Ok(trouve)
    }

    /// Règle 1 : référence. Renvoie (évaluations, budget épuisé).
    fn par_reference(&mut self, budget: u64) -> (u64, bool) {
        let mut evaluations = 0u64;
        for i in 0..self.lignes.len() {
            let Some(reference) = self.lignes[i].reference else {
                continue;
            };
            if self.prise[i] {
                continue;
            }
            let critere = |autre: &Ouverte<'_>| {
                if evaluations >= budget {
                    return Err(Epuise);
                }
                evaluations += 1;
                Ok(autre.reference.is_some_and(|r| r.contains(reference)))
            };
            match self.candidat_unique(i, 0..self.lignes.len(), critere) {
                Err(Epuise) => return (evaluations, true),
                Ok(Some(j)) => {
                    self.retenir(Regle::Reference, &[i, j]);
                }
                Ok(None) => {}
            }
        }
        (evaluations, false)
    }

    /// Règle 2 : montant exact, par index des résiduels (coût quasi linéaire, sans budget).
    fn par_montant(&mut self) {
        let mut index: BTreeMap<(Sens, Decimal), Vec<usize>> = BTreeMap::new();
        for (j, ligne) in self.lignes.iter().enumerate() {
            index
                .entry((ligne.sens, ligne.residuel))
                .or_default()
                .push(j);
        }
        for i in 0..self.lignes.len() {
            if self.prise[i] {
                continue;
            }
            let cle = (self.lignes[i].sens.inverse(), self.lignes[i].residuel);
            let candidats = index.get(&cle).map_or_default(Vec::as_slice);
            if let Ok(Some(j)) = self.candidat_unique(i, candidats.iter().copied(), |_| Ok(true)) {
                self.retenir(Regle::MontantExact, &[i, j]);
            }
        }
    }

    /// Règle 3 : combinaison bornée. Renvoie (évaluations, budget épuisé).
    fn par_combinaison(&mut self, bornes: &Bornes) -> (u64, bool) {
        let mut restant = bornes.budget;
        for i in 0..self.lignes.len() {
            if self.prise[i] {
                continue;
            }
            let cible = &self.lignes[i];
            let candidats: Vec<usize> = (0..self.lignes.len())
                .filter(|&j| {
                    !self.prise[j]
                        && self.lignes[j].sens != cible.sens
                        && self.lignes[j].residuel < cible.residuel
                })
                .collect();
            let mut choisis = Vec::new();
            match chercher(
                self.lignes,
                &candidats,
                0,
                cible.residuel,
                bornes.lignes_max - 1,
                &mut restant,
                &mut choisis,
            ) {
                Issue::Epuisee => return (bornes.budget - restant, true),
                Issue::Trouvee => {
                    let indices: Vec<usize> = std::iter::once(i)
                        .chain(choisis.iter().map(|&k| candidats[k]))
                        .collect();
                    self.retenir(Regle::Combinaison, &indices);
                }
                Issue::Absente => {}
            }
        }
        (bornes.budget - restant, false)
    }
}

/// Recherche en profondeur d'au moins deux candidats (au plus `max`) dont les résiduels
/// somment exactement `reste`, dans l'ordre des candidats. Chaque candidat examiné consomme
/// une évaluation du budget ; la profondeur est bornée par `max`.
fn chercher(
    lignes: &[Ouverte<'_>],
    candidats: &[usize],
    debut: usize,
    reste: Decimal,
    max: usize,
    restant: &mut u64,
    choisis: &mut Vec<usize>,
) -> Issue {
    for k in debut..candidats.len() {
        if *restant == 0 {
            return Issue::Epuisee;
        }
        *restant -= 1;
        let valeur = lignes[candidats[k]].residuel;
        if valeur > reste {
            continue;
        }
        choisis.push(k);
        if valeur == reste {
            // Un seul candidat égal : c'est la règle du montant exact (ambiguë ici), pas une
            // combinaison.
            if choisis.len() >= 2 {
                return Issue::Trouvee;
            }
        } else if choisis.len() < max {
            match chercher(
                lignes,
                candidats,
                k + 1,
                reste - valeur,
                max,
                restant,
                choisis,
            ) {
                Issue::Absente => {}
                issue => return issue,
            }
        }
        choisis.pop();
    }
    Issue::Absente
}
