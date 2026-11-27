//! Propositions de lettrage automatique (spec §10.1 ; ADR 0032, point 7).
//!
//! Règles **déterministes**, appliquées dans l'ordre de la spec, par groupe (compte, tiers
//! résolu), sur les lignes au résiduel non nul triées par échéance effective (à défaut date
//! comptable) puis rang de chaîne.
//!
//! 1. **Référence** : références normalisées d'au moins [`REFERENCE_LONGUEUR_MIN`] caractères,
//!    l'une contenue dans l'autre, **résiduels égaux**, sens opposés ;
//! 2. **Montant exact** : résiduels égaux, sens opposés ;
//! 3. **Combinaison bornée** : plusieurs lignes d'un sens dont les résiduels somment exactement
//!    le résiduel d'une ligne de l'autre sens (somme de factures = règlement). Fonctionnalité
//!    Sigma, absente des moteurs (Tryton n'a qu'une approximation gloutonne, `move.py:1828-1871`).
//!    Recherche en profondeur dans l'ordre fixé, bornée en nombre de lignes ; la première
//!    combinaison trouvée est proposée.
//!
//! **Candidat unique réciproque** (règles 1 et 2) : une paire n'est proposée que si chaque
//! ligne est l'unique candidat libre de l'autre. OCA compte le candidat du seul côté de la
//! ligne de relevé (`account_bank_statement_line.py:1044-1059`, `COUNT(*) = 1`) ; dans un
//! grand livre, aucun côté n'est privilégié : deux factures de 100 et un règlement de 100 sont
//! ambigus, et ne donnent **aucune** proposition automatique. L'ambiguïté est rendue
//! ([`Ambiguite`] : la ligne et ses candidats) pour que l'utilisateur tranche.
//!
//! **Équilibre** : une proposition solde toujours ses lignes (débit = crédit, ADR 0032 :
//! « ni un groupe déséquilibré ») ; le lettrage partiel reste manuel.
//!
//! **Budget** (F2.16) : chaque ligne examinée, en boucle externe comme parmi les candidats ou
//! dans la recherche de combinaison, consomme une évaluation ; le budget s'applique par groupe
//! et par règle. Un groupe sans ligne de l'un des deux sens n'est pas parcouru. Le coût d'un
//! appel est donc borné par le tri et l'indexation des lignes, et `3 × budget` évaluations par
//! groupe.
//!
//! Une ligne entre dans au plus une proposition. Chaque proposition est contrôlée comme un
//! lettrage manuel ([`RegistreLettrage::imputer`]). Rien n'est inscrit.

use std::collections::BTreeMap;

use rust_decimal::Decimal;

use super::{ErreurLettrage, IdLigne, Lien, RegistreLettrage};
use crate::ecriture::{IdTiers, Sens};

/// Longueur minimale (en caractères, après normalisation) d'une référence prise en compte par
/// la règle de référence. Une référence d'un ou deux caractères (« 1 », « A2 ») est contenue
/// dans presque toute autre et produirait de faux rapprochements ; OCA n'a pas de minimum
/// (une référence vide y correspond à tout).
pub const REFERENCE_LONGUEUR_MIN: usize = 3;

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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
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

/// Ligne qui a plusieurs candidats pour une règle : aucune proposition automatique.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ambiguite {
    regle: Regle,
    ligne: IdLigne,
    candidats: Vec<IdLigne>,
}

impl Ambiguite {
    /// Règle concernée.
    #[must_use]
    pub fn regle(&self) -> Regle {
        self.regle
    }

    /// Ligne examinée.
    #[must_use]
    pub fn ligne(&self) -> IdLigne {
        self.ligne
    }

    /// Ses candidats (au moins deux), dans l'ordre de recherche.
    #[must_use]
    pub fn candidats(&self) -> &[IdLigne] {
        &self.candidats
    }
}

/// Bilan d'un groupe (compte, tiers) : évaluations consommées et épuisement du budget.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BilanGroupe {
    tiers: Option<IdTiers>,
    evaluations: [u64; 3],
    epuise: bool,
}

impl BilanGroupe {
    /// Tiers résolu du groupe.
    #[must_use]
    pub fn tiers(&self) -> Option<&IdTiers> {
        self.tiers.as_ref()
    }

    /// Évaluations consommées par une règle (au plus le budget).
    #[must_use]
    pub fn evaluations(&self, regle: Regle) -> u64 {
        self.evaluations[regle as usize]
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
    retenues: Vec<Proposition>,
    ambiguites: Vec<Ambiguite>,
    groupes: Vec<BilanGroupe>,
}

impl Propositions {
    /// Propositions, par groupe puis par règle.
    #[must_use]
    pub fn propositions(&self) -> &[Proposition] {
        &self.retenues
    }

    /// Ambiguïtés relevées par les règles 1 et 2 (lignes laissées à l'utilisateur).
    #[must_use]
    pub fn ambiguites(&self) -> &[Ambiguite] {
        &self.ambiguites
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

/// Compteur d'évaluations d'une règle.
struct Compteur {
    budget: u64,
    consomme: u64,
}

impl Compteur {
    fn evaluer(&mut self) -> Result<(), Epuise> {
        if self.consomme >= self.budget {
            return Err(Epuise);
        }
        self.consomme += 1;
        Ok(())
    }
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
                        reference: ligne
                            .reference
                            .as_deref()
                            .filter(|r| r.chars().count() >= REFERENCE_LONGUEUR_MIN),
                    },
                ));
            }
        }
        let mut resultat = Propositions::default();
        for (tiers, mut lignes) in groupes {
            lignes.sort_by_key(|(cle, _)| *cle);
            let lignes: Vec<Ouverte<'_>> = lignes.into_iter().map(|(_, o)| o).collect();
            let (debits, credits): (Vec<usize>, Vec<usize>) =
                (0..lignes.len()).partition(|&i| lignes[i].sens == Sens::Debit);
            let mut index: BTreeMap<(Sens, Decimal), Vec<usize>> = BTreeMap::new();
            for (i, ligne) in lignes.iter().enumerate() {
                index
                    .entry((ligne.sens, ligne.residuel))
                    .or_default()
                    .push(i);
            }
            let mut groupe = Groupe {
                registre: self,
                tiers: tiers.cloned(),
                lignes: &lignes,
                debits,
                credits,
                index,
                prise: vec![false; lignes.len()],
                propositions: Vec::new(),
                ambiguites: Vec::new(),
            };
            let mut evaluations = [0; 3];
            let mut epuise = false;
            if !groupe.debits.is_empty() && !groupe.credits.is_empty() {
                for regle in [Regle::Reference, Regle::MontantExact, Regle::Combinaison] {
                    let mut compteur = Compteur {
                        budget: bornes.budget,
                        consomme: 0,
                    };
                    let issue = match regle {
                        Regle::Reference | Regle::MontantExact => {
                            groupe.apparier(regle, &mut compteur)
                        }
                        Regle::Combinaison => groupe.combiner(bornes, &mut compteur),
                    };
                    epuise |= issue.is_err();
                    evaluations[regle as usize] = compteur.consomme;
                }
            }
            resultat.retenues.append(&mut groupe.propositions);
            resultat.ambiguites.append(&mut groupe.ambiguites);
            resultat.groupes.push(BilanGroupe {
                tiers: tiers.cloned(),
                evaluations,
                epuise,
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
    debits: Vec<usize>,
    credits: Vec<usize>,
    /// Lignes par (sens, résiduel), dans l'ordre de recherche.
    index: BTreeMap<(Sens, Decimal), Vec<usize>>,
    prise: Vec<bool>,
    propositions: Vec<Proposition>,
    ambiguites: Vec<Ambiguite>,
}

impl Groupe<'_, '_> {
    /// Indices des lignes de sens opposé à la ligne `i`, dans l'ordre de recherche.
    fn opposees(&self, i: usize) -> &[usize] {
        match self.lignes[i].sens {
            Sens::Debit => &self.credits,
            Sens::Credit => &self.debits,
        }
    }

    /// Vrai si les lignes `a` et `b` (de sens opposés) se correspondent pour la règle.
    fn correspondent(&self, regle: Regle, a: usize, b: usize) -> bool {
        let (a, b) = (&self.lignes[a], &self.lignes[b]);
        a.residuel == b.residuel
            && match regle {
                Regle::Reference => match (a.reference, b.reference) {
                    (Some(x), Some(y)) => x.contains(y) || y.contains(x),
                    _ => false,
                },
                Regle::MontantExact | Regle::Combinaison => true,
            }
    }

    /// Candidats libres de la ligne `i` pour les règles 1 et 2, parmi les lignes de sens
    /// opposé au même résiduel (index) ; chaque ligne parcourue est évaluée.
    fn candidats(
        &self,
        regle: Regle,
        i: usize,
        compteur: &mut Compteur,
    ) -> Result<Vec<usize>, Epuise> {
        let cle = (self.lignes[i].sens.inverse(), self.lignes[i].residuel);
        let source = self.index.get(&cle).map_or_default(Vec::as_slice);
        let mut trouves = Vec::new();
        for &j in source {
            compteur.evaluer()?;
            if !self.prise[j] && self.correspondent(regle, i, j) {
                trouves.push(j);
            }
        }
        Ok(trouves)
    }

    /// Règles 1 et 2 : paires à candidat unique réciproque ; les ambiguïtés sont relevées.
    fn apparier(&mut self, regle: Regle, compteur: &mut Compteur) -> Result<(), Epuise> {
        let mut signalee = vec![false; self.lignes.len()];
        for i in 0..self.lignes.len() {
            compteur.evaluer()?;
            if self.prise[i] || (regle == Regle::Reference && self.lignes[i].reference.is_none()) {
                continue;
            }
            let candidats = self.candidats(regle, i, compteur)?;
            let (ambigue, candidats) = match candidats.as_slice() {
                [] => continue,
                [j] => {
                    let retour = self.candidats(regle, *j, compteur)?;
                    if retour == [i] {
                        self.retenir(regle, &[i, *j]);
                        continue;
                    }
                    (*j, retour)
                }
                _ => (i, candidats),
            };
            if !signalee[ambigue] {
                signalee[ambigue] = true;
                self.ambiguites.push(Ambiguite {
                    regle,
                    ligne: self.lignes[ambigue].id,
                    candidats: candidats.iter().map(|&k| self.lignes[k].id).collect(),
                });
            }
        }
        Ok(())
    }

    /// Règle 3 : combinaison bornée.
    fn combiner(&mut self, bornes: &Bornes, compteur: &mut Compteur) -> Result<(), Epuise> {
        for i in 0..self.lignes.len() {
            compteur.evaluer()?;
            if self.prise[i] {
                continue;
            }
            let cible = self.lignes[i].residuel;
            let mut candidats = Vec::new();
            for &j in self.opposees(i) {
                compteur.evaluer()?;
                if !self.prise[j] && self.lignes[j].residuel < cible {
                    candidats.push(j);
                }
            }
            let mut choisis = Vec::new();
            if chercher(
                self.lignes,
                &candidats,
                0,
                cible,
                bornes.lignes_max - 1,
                compteur,
                &mut choisis,
            )? {
                let indices: Vec<usize> = std::iter::once(i)
                    .chain(choisis.iter().map(|&k| candidats[k]))
                    .collect();
                self.retenir(Regle::Combinaison, &indices);
            }
        }
        Ok(())
    }

    /// Retient la proposition si elle passe les contrôles d'un lettrage manuel et solde
    /// toutes ses lignes.
    fn retenir(&mut self, regle: Regle, indices: &[usize]) {
        let mut lignes: Vec<IdLigne> = indices.iter().map(|&i| self.lignes[i].id).collect();
        lignes.sort_unstable();
        let Ok(liens) = self.registre.imputer(&lignes) else {
            return;
        };
        let lettre: Decimal = liens.iter().map(Lien::montant).sum();
        let debit: Decimal = indices
            .iter()
            .filter(|&&i| self.lignes[i].sens == Sens::Debit)
            .map(|&i| self.lignes[i].residuel)
            .sum();
        let credit: Decimal = indices
            .iter()
            .filter(|&&i| self.lignes[i].sens == Sens::Credit)
            .map(|&i| self.lignes[i].residuel)
            .sum();
        if lettre != debit || lettre != credit {
            return;
        }
        for &i in indices {
            self.prise[i] = true;
        }
        self.propositions.push(Proposition {
            regle,
            tiers: self.tiers.clone(),
            lignes,
            liens,
        });
    }
}

/// Recherche en profondeur d'au moins deux candidats (au plus `max`) dont les résiduels
/// somment exactement `reste`, dans l'ordre des candidats. Chaque candidat examiné consomme
/// une évaluation ; la profondeur est bornée par `max`. `Ok(true)` : `choisis` est rempli.
fn chercher(
    lignes: &[Ouverte<'_>],
    candidats: &[usize],
    debut: usize,
    reste: Decimal,
    max: usize,
    compteur: &mut Compteur,
    choisis: &mut Vec<usize>,
) -> Result<bool, Epuise> {
    for k in debut..candidats.len() {
        compteur.evaluer()?;
        let valeur = lignes[candidats[k]].residuel;
        if valeur > reste {
            continue;
        }
        choisis.push(k);
        if valeur == reste {
            // Un seul candidat égal relève de la règle du montant exact, pas d'une combinaison.
            if choisis.len() >= 2 {
                return Ok(true);
            }
        } else if choisis.len() < max
            && chercher(
                lignes,
                candidats,
                k + 1,
                reste - valeur,
                max,
                compteur,
                choisis,
            )?
        {
            return Ok(true);
        }
        choisis.pop();
    }
    Ok(false)
}
