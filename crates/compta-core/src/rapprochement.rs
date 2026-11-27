//! Pointage et état de rapprochement bancaire (spec §9.5 ; ADR 0034, points 2 et 9 ;
//! décisions B3 et D2 du plan L3).
//!
//! - Un **pointage** relie, en **N-N**, des lignes de relevé importées à des lignes d'écriture
//!   **validées** du compte de trésorerie ; la **somme signée des deux côtés est égale**, sans
//!   tolérance (décimal exact : la tolérance de 0,01 en flottant de Bigcapital,
//!   `MatchTransactions.ts:111-118`, n'est pas reprise). Une ligne entre dans un pointage pour
//!   tout son montant et appartient à **un seul pointage non rompu** : un pointage ne dépasse
//!   jamais une ligne.
//! - Les pointages s'**ajoutent** ; la correction est une **rupture** ajoutée, jamais une
//!   suppression. Aucune ligne n'est modifiée : Dolibarr (`account.class.php:2719-2762`) et Odoo
//!   (`account_bank_statement_line.py:796-835`) mutent la ligne rapprochée, écart nécessaire
//!   (plan L3, P3) ; Sigma garde une table de liens, comme Bigcapital
//!   (`MatchedBankTransaction.ts:3-6`). Le pointage ne fait pas de lettrage (ADR 0032).
//! - L'**état de rapprochement** à une date est **calculé par reconstitution** à partir des
//!   dates des opérations de pointage et de rupture ([`RegistrePointage::etat`], module `etat`).
//! - Les **avertissements de solde** d'un compte de trésorerie (caisse ou portefeuille
//!   créditeur, plafond de caisse) ne refusent jamais rien (D2) : [`avertissements_solde`].
//!
//! **Signe.** Un montant de ligne de relevé est positif pour une entrée d'argent côté banque
//! (ADR 0034, point 5) ; une ligne d'écriture du compte de trésorerie compte positivement au
//! débit et négativement au crédit. Les deux côtés sont ainsi orientés de la même façon.
//!
//! **Opérations.** Chaque opération (pointage ou rupture) reçoit un **rang d'opération**
//! croissant par compte de trésorerie, qui identifie aussi le pointage ; la relecture de la
//! persistance ([`RegistrePointage::rejouer`]) suit ce rang. Les dates des opérations sont des
//! paramètres (crate pure) : la couche de persistance (PR 13a) y met le jour de l'opération.
//!
//! **Une ligne, un pointage à chaque date.** Un pointage est **en vigueur** de sa date jusqu'à
//! la veille de sa rupture. Une ligne ne peut entrer dans un nouveau pointage que si aucun
//! pointage qui la contient ne reste en vigueur à partir de la date du nouveau : la règle
//! « un seul pointage non rompu » de l'ADR tient ainsi à toute date de la reconstitution.
//!
//! Le registre ne vérifie pas que les lignes qu'on lui confie sont validées ni qu'elles sont du
//! même dossier : comme pour [`EcritureValidee::reconstituer`], c'est le contrat de l'appelant.

mod etat;
mod solde;

use std::collections::{BTreeMap, BTreeSet};

use rust_decimal::Decimal;
use time::Date;

use crate::compte::NumeroCompte;
use crate::ecriture::{Ligne, Sens};
use crate::livre::EcritureValidee;

pub use etat::{EtatRapprochement, Ouverture};
pub use solde::{AvertissementSolde, TypeTresorerie, avertissements_solde};

/// Erreurs du pointage et du rapprochement.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ErreurRapprochement {
    /// Rang de chaîne ou position de ligne nul.
    #[error("identifiant de ligne d'écriture invalide : rang {rang}, position {position}")]
    IdLigneInvalide {
        /// Rang de chaîne de l'écriture.
        rang: u64,
        /// Position de la ligne dans l'écriture.
        position: u16,
    },
    /// Ligne d'écriture d'un autre compte que celui du registre.
    #[error("la ligne {0} n'est pas sur le compte de trésorerie du registre")]
    AutreCompte(IdLigneEcriture),
    /// Ligne de relevé absente du registre.
    #[error("ligne de relevé inconnue : {0}")]
    LigneReleveInconnue(IdLigneReleve),
    /// Ligne d'écriture absente du registre ou de l'écriture.
    #[error("ligne d'écriture inconnue : {0}")]
    LigneComptableInconnue(IdLigneEcriture),
    /// Ligne de relevé citée deux fois (ou ajoutée deux fois au registre).
    #[error("ligne de relevé en double : {0}")]
    LigneReleveEnDouble(IdLigneReleve),
    /// Ligne d'écriture citée deux fois (ou ajoutée deux fois au registre).
    #[error("ligne d'écriture en double : {0}")]
    LigneComptableEnDouble(IdLigneEcriture),
    /// Pointage sans ligne de relevé (un lien relie une ligne de relevé à une ligne d'écriture).
    #[error("pointage sans ligne de relevé")]
    SansLigneReleve,
    /// Pointage sans ligne d'écriture.
    #[error("pointage sans ligne d'écriture")]
    SansLigneComptable,
    /// Ligne de relevé déjà dans un pointage en vigueur à la date du nouveau pointage.
    #[error("ligne de relevé {ligne} déjà pointée (pointage {pointage})")]
    ReleveDejaPointee {
        /// Ligne de relevé.
        ligne: IdLigneReleve,
        /// Rang du pointage qui la contient.
        pointage: u64,
    },
    /// Ligne d'écriture déjà dans un pointage en vigueur à la date du nouveau pointage.
    #[error("ligne d'écriture {ligne} déjà pointée (pointage {pointage})")]
    ComptableDejaPointee {
        /// Ligne d'écriture.
        ligne: IdLigneEcriture,
        /// Rang du pointage qui la contient.
        pointage: u64,
    },
    /// Les sommes signées des deux côtés diffèrent (ADR 0034, point 9).
    #[error("pointage déséquilibré : relevé {releve}, écritures {comptable}")]
    Desequilibre {
        /// Somme signée des lignes de relevé.
        releve: Decimal,
        /// Somme signée des lignes d'écriture.
        comptable: Decimal,
    },
    /// Rang de pointage inconnu.
    #[error("pointage inconnu : {0}")]
    PointageInconnu(u64),
    /// Pointage déjà rompu.
    #[error("pointage {0} déjà rompu")]
    DejaRompu(u64),
    /// Rang d'opération nul, en double ou qui ne suit pas le dernier rang du registre.
    #[error("rang d'opération invalide : {0}")]
    RangOperationInvalide(u64),
    /// Dépassement de capacité décimale.
    #[error("dépassement de capacité")]
    Depassement,
}

/// Identifiant d'une ligne de relevé importée (`ligne_releve`, ADR 0034, point 5), attribué par
/// la persistance et opaque pour le moteur.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct IdLigneReleve(u64);

impl IdLigneReleve {
    /// Construit un identifiant.
    #[must_use]
    pub fn nouveau(valeur: u64) -> Self {
        Self(valeur)
    }

    /// La valeur.
    #[must_use]
    pub fn valeur(&self) -> u64 {
        self.0
    }
}

impl std::fmt::Display for IdLigneReleve {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "R{}", self.0)
    }
}

/// Identifiant d'une ligne d'écriture **validée** : rang de chaîne de l'écriture (unique par
/// dossier, ADR 0020) et position de la ligne (à partir de 1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct IdLigneEcriture {
    rang: u64,
    position: u16,
}

impl IdLigneEcriture {
    /// Construit un identifiant.
    ///
    /// # Errors
    /// [`ErreurRapprochement::IdLigneInvalide`] si le rang ou la position est nul.
    pub fn nouveau(rang: u64, position: u16) -> Result<Self, ErreurRapprochement> {
        if rang == 0 || position == 0 {
            return Err(ErreurRapprochement::IdLigneInvalide { rang, position });
        }
        Ok(Self { rang, position })
    }

    /// Rang de chaîne de l'écriture.
    #[must_use]
    pub fn rang(&self) -> u64 {
        self.rang
    }

    /// Position de la ligne dans l'écriture.
    #[must_use]
    pub fn position(&self) -> u16 {
        self.position
    }
}

impl std::fmt::Display for IdLigneEcriture {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{}", self.rang, self.position)
    }
}

/// Ligne de relevé vue par le rapprochement : immuable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LigneReleve {
    id: IdLigneReleve,
    date: Date,
    montant: Decimal,
}

impl LigneReleve {
    /// Ligne relue de la persistance : `date` est la date d'opération du relevé, `montant` le
    /// montant signé (positif = entrée d'argent côté banque).
    #[must_use]
    pub fn nouvelle(id: IdLigneReleve, date: Date, montant: Decimal) -> Self {
        Self { id, date, montant }
    }

    /// Identifiant.
    #[must_use]
    pub fn id(&self) -> IdLigneReleve {
        self.id
    }

    /// Date d'opération.
    #[must_use]
    pub fn date(&self) -> Date {
        self.date
    }

    /// Montant signé (positif = entrée d'argent).
    #[must_use]
    pub fn montant(&self) -> Decimal {
        self.montant
    }
}

/// Ligne d'écriture validée du compte de trésorerie : immuable, construite à partir d'une
/// [`Ligne`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LigneComptable {
    id: IdLigneEcriture,
    compte: NumeroCompte,
    date: Date,
    montant: Decimal,
}

impl LigneComptable {
    /// Ligne relue d'une écriture validée : `ligne` est la ligne `id.position()` de l'écriture
    /// **validée** de rang de chaîne `id.rang()`, `date` sa date comptable (contrat de
    /// l'appelant, comme [`EcritureValidee::reconstituer`]). Le montant retenu est celui en
    /// devise de tenue, signé selon le sens (débit positif).
    #[must_use]
    pub fn nouvelle(id: IdLigneEcriture, ligne: &Ligne, date: Date) -> Self {
        let valeur = ligne.montant().valeur();
        let montant = match ligne.sens() {
            Sens::Debit => valeur,
            Sens::Credit => -valeur,
        };
        Self {
            id,
            compte: ligne.compte().clone(),
            date,
            montant,
        }
    }

    /// Ligne `position` (à partir de 1) d'une écriture validée de rang de chaîne `rang`.
    ///
    /// # Errors
    /// Identifiant invalide ou position hors de l'écriture.
    pub fn depuis_validee(
        ecriture: &EcritureValidee,
        rang: u64,
        position: u16,
    ) -> Result<Self, ErreurRapprochement> {
        let id = IdLigneEcriture::nouveau(rang, position)?;
        let ligne = ecriture
            .lignes()
            .get(usize::from(position) - 1)
            .ok_or(ErreurRapprochement::LigneComptableInconnue(id))?;
        Ok(Self::nouvelle(id, ligne, ecriture.date()))
    }

    /// Identifiant.
    #[must_use]
    pub fn id(&self) -> IdLigneEcriture {
        self.id
    }

    /// Compte.
    #[must_use]
    pub fn compte(&self) -> &NumeroCompte {
        &self.compte
    }

    /// Date comptable.
    #[must_use]
    pub fn date(&self) -> Date {
        self.date
    }

    /// Montant signé en devise de tenue (débit positif, crédit négatif).
    #[must_use]
    pub fn montant(&self) -> Decimal {
        self.montant
    }
}

/// Pointage : une opération, immuable une fois inscrite.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pointage {
    rang: u64,
    date: Date,
    releve: BTreeSet<IdLigneReleve>,
    comptables: BTreeSet<IdLigneEcriture>,
    montant: Decimal,
}

impl Pointage {
    /// Rang d'opération, qui identifie le pointage dans le compte de trésorerie.
    #[must_use]
    pub fn rang(&self) -> u64 {
        self.rang
    }

    /// Date de l'opération.
    #[must_use]
    pub fn date(&self) -> Date {
        self.date
    }

    /// Lignes de relevé, dans l'ordre des identifiants.
    #[must_use]
    pub fn releve(&self) -> &BTreeSet<IdLigneReleve> {
        &self.releve
    }

    /// Lignes d'écriture, dans l'ordre des identifiants.
    #[must_use]
    pub fn comptables(&self) -> &BTreeSet<IdLigneEcriture> {
        &self.comptables
    }

    /// Montant signé pointé (égal des deux côtés).
    #[must_use]
    pub fn montant(&self) -> Decimal {
        self.montant
    }
}

/// Rupture d'un pointage : ajoutée, jamais supprimée.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rupture {
    rang: u64,
    pointage: u64,
    date: Date,
}

impl Rupture {
    /// Rang d'opération.
    #[must_use]
    pub fn rang(&self) -> u64 {
        self.rang
    }

    /// Rang du pointage rompu.
    #[must_use]
    pub fn pointage(&self) -> u64 {
        self.pointage
    }

    /// Date de l'opération : le pointage n'est plus en vigueur à partir de ce jour.
    #[must_use]
    pub fn date(&self) -> Date {
        self.date
    }
}

/// Opération relue de la persistance, à rejouer par [`RegistrePointage::rejouer`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Operation {
    /// Pointage.
    Pointage {
        /// Date de l'opération.
        date: Date,
        /// Lignes de relevé.
        releve: Vec<IdLigneReleve>,
        /// Lignes d'écriture.
        comptables: Vec<IdLigneEcriture>,
    },
    /// Rupture d'un pointage.
    Rupture {
        /// Rang du pointage rompu.
        pointage: u64,
        /// Date de l'opération.
        date: Date,
    },
}

/// Pointages d'un compte de trésorerie : lignes, pointages et ruptures, en ajout seul.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegistrePointage {
    compte: NumeroCompte,
    releve: BTreeMap<IdLigneReleve, LigneReleve>,
    comptables: BTreeMap<IdLigneEcriture, LigneComptable>,
    pointages: BTreeMap<u64, Pointage>,
    /// Ruptures, par rang du pointage rompu (au plus une par pointage).
    ruptures: BTreeMap<u64, Rupture>,
    /// Index dérivé : pointages (rompus compris) qui citent chaque ligne.
    index_releve: BTreeMap<IdLigneReleve, Vec<u64>>,
    index_comptables: BTreeMap<IdLigneEcriture, Vec<u64>>,
    /// Dernier rang d'opération (0 : aucune).
    dernier_rang: u64,
}

impl RegistrePointage {
    /// Ouvre le registre du compte comptable d'un compte de trésorerie.
    #[must_use]
    pub fn ouvrir(compte: NumeroCompte) -> Self {
        Self {
            compte,
            releve: BTreeMap::new(),
            comptables: BTreeMap::new(),
            pointages: BTreeMap::new(),
            ruptures: BTreeMap::new(),
            index_releve: BTreeMap::new(),
            index_comptables: BTreeMap::new(),
            dernier_rang: 0,
        }
    }

    /// Compte du registre.
    #[must_use]
    pub fn compte(&self) -> &NumeroCompte {
        &self.compte
    }

    /// Ajoute une ligne de relevé du compte de trésorerie.
    ///
    /// # Errors
    /// [`ErreurRapprochement::LigneReleveEnDouble`].
    pub fn ajouter_releve(&mut self, ligne: LigneReleve) -> Result<(), ErreurRapprochement> {
        if self.releve.contains_key(&ligne.id) {
            return Err(ErreurRapprochement::LigneReleveEnDouble(ligne.id));
        }
        self.releve.insert(ligne.id, ligne);
        Ok(())
    }

    /// Ajoute une ligne d'écriture validée du compte.
    ///
    /// # Errors
    /// Ligne d'un autre compte ou déjà présente.
    pub fn ajouter_comptable(&mut self, ligne: LigneComptable) -> Result<(), ErreurRapprochement> {
        if ligne.compte != self.compte {
            return Err(ErreurRapprochement::AutreCompte(ligne.id));
        }
        if self.comptables.contains_key(&ligne.id) {
            return Err(ErreurRapprochement::LigneComptableEnDouble(ligne.id));
        }
        self.comptables.insert(ligne.id, ligne);
        Ok(())
    }

    /// Lignes de relevé, dans l'ordre des identifiants.
    pub fn lignes_releve(&self) -> impl Iterator<Item = &LigneReleve> {
        self.releve.values()
    }

    /// Lignes d'écriture, dans l'ordre des identifiants.
    pub fn lignes_comptables(&self) -> impl Iterator<Item = &LigneComptable> {
        self.comptables.values()
    }

    /// Pointages, rompus compris, dans l'ordre des rangs.
    pub fn pointages(&self) -> impl Iterator<Item = &Pointage> {
        self.pointages.values()
    }

    /// Pointage d'un rang.
    #[must_use]
    pub fn pointage(&self, rang: u64) -> Option<&Pointage> {
        self.pointages.get(&rang)
    }

    /// Rupture d'un pointage, s'il est rompu.
    #[must_use]
    pub fn rupture(&self, pointage: u64) -> Option<&Rupture> {
        self.ruptures.get(&pointage)
    }

    /// Pointage non rompu qui contient la ligne de relevé, s'il y en a un.
    #[must_use]
    pub fn pointage_releve(&self, id: IdLigneReleve) -> Option<&Pointage> {
        self.non_rompu(self.index_releve.get(&id))
    }

    /// Pointage non rompu qui contient la ligne d'écriture, s'il y en a un.
    #[must_use]
    pub fn pointage_comptable(&self, id: IdLigneEcriture) -> Option<&Pointage> {
        self.non_rompu(self.index_comptables.get(&id))
    }

    fn non_rompu(&self, rangs: Option<&Vec<u64>>) -> Option<&Pointage> {
        rangs?
            .iter()
            .filter(|r| !self.ruptures.contains_key(r))
            .find_map(|r| self.pointages.get(r))
    }

    /// Pointe des lignes de relevé avec des lignes d'écriture du compte (N-N) : les sommes
    /// signées des deux côtés doivent être égales. L'ordre des sélections est indifférent.
    ///
    /// # Errors
    /// Sélection vide d'un côté, ligne inconnue ou en double, ligne déjà pointée à cette date,
    /// sommes différentes ([`ErreurRapprochement::Desequilibre`]), dépassement de capacité.
    pub fn pointer(
        &mut self,
        releve: &[IdLigneReleve],
        comptables: &[IdLigneEcriture],
        date: Date,
    ) -> Result<&Pointage, ErreurRapprochement> {
        let rang = self.prochain_rang()?;
        self.inscrire(rang, date, releve, comptables)
    }

    /// Rompt un pointage : ajoute une rupture datée du jour de l'opération. Ses lignes peuvent
    /// être pointées à nouveau à partir de cette date. Rompre un pointage pris dans un état
    /// figé n'est pas refusé (ADR 0034, B3) : l'appelant avertit, à l'aide de
    /// [`Self::en_vigueur_a`] appliqué aux dates des états figés.
    ///
    /// # Errors
    /// Pointage inconnu ou déjà rompu.
    pub fn rompre(&mut self, pointage: u64, date: Date) -> Result<&Rupture, ErreurRapprochement> {
        let rang = self.prochain_rang()?;
        self.inscrire_rupture(rang, pointage, date)
    }

    /// Vrai si le pointage est **en vigueur** à la date : daté au plus tard ce jour-là et non
    /// rompu à cette date (rupture absente ou postérieure). C'est ce qui décide si un état
    /// figé à cette date le prend en compte (avec la condition sur les dates des lignes, voir
    /// [`Self::etat`]).
    #[must_use]
    pub fn en_vigueur_a(&self, pointage: u64, date: Date) -> bool {
        self.pointages
            .get(&pointage)
            .is_some_and(|p| self.en_vigueur(p, date))
    }

    fn en_vigueur(&self, pointage: &Pointage, date: Date) -> bool {
        pointage.date <= date
            && self
                .ruptures
                .get(&pointage.rang)
                .is_none_or(|r| r.date > date)
    }

    /// Rejoue des opérations relues de la persistance, **dans l'ordre de leur rang** (l'ordre
    /// de l'itérateur est indifférent), avec les contrôles de [`Self::pointer`] et de
    /// [`Self::rompre`]. Tout ou rien : en cas d'erreur, le registre est inchangé.
    ///
    /// # Errors
    /// [`ErreurRapprochement::RangOperationInvalide`] (rang nul, en double, ou pas au-delà du
    /// dernier rang du registre) ; contrôles de [`Self::pointer`] et de [`Self::rompre`].
    pub fn rejouer(
        &mut self,
        operations: impl IntoIterator<Item = (u64, Operation)>,
    ) -> Result<(), ErreurRapprochement> {
        let mut operations: Vec<(u64, Operation)> = operations.into_iter().collect();
        operations.sort_by_key(|(rang, _)| *rang);
        let mut copie = self.clone();
        for (rang, operation) in operations {
            if rang <= copie.dernier_rang {
                return Err(ErreurRapprochement::RangOperationInvalide(rang));
            }
            match operation {
                Operation::Pointage {
                    date,
                    releve,
                    comptables,
                } => {
                    copie.inscrire(rang, date, &releve, &comptables)?;
                }
                Operation::Rupture { pointage, date } => {
                    copie.inscrire_rupture(rang, pointage, date)?;
                }
            }
        }
        *self = copie;
        Ok(())
    }

    /// Historique des opérations, par rang : ce que [`Self::rejouer`] relit.
    #[must_use]
    pub fn historique(&self) -> Vec<(u64, Operation)> {
        let pointages = self.pointages.values().map(|p| {
            (
                p.rang,
                Operation::Pointage {
                    date: p.date,
                    releve: p.releve.iter().copied().collect(),
                    comptables: p.comptables.iter().copied().collect(),
                },
            )
        });
        let ruptures = self.ruptures.values().map(|r| {
            (
                r.rang,
                Operation::Rupture {
                    pointage: r.pointage,
                    date: r.date,
                },
            )
        });
        let mut historique: Vec<(u64, Operation)> = pointages.chain(ruptures).collect();
        historique.sort_by_key(|(rang, _)| *rang);
        historique
    }

    fn prochain_rang(&self) -> Result<u64, ErreurRapprochement> {
        self.dernier_rang
            .checked_add(1)
            .ok_or(ErreurRapprochement::RangOperationInvalide(
                self.dernier_rang,
            ))
    }

    /// Premier pointage de `rangs` qui reste en vigueur à une date postérieure ou égale à
    /// `date` : sa période de vigueur `[date du pointage, date de rupture)` rencontre
    /// `[date, ∞)`.
    fn conflit(&self, rangs: Option<&Vec<u64>>, date: Date) -> Option<u64> {
        rangs?.iter().copied().find(|r| {
            self.pointages.get(r).is_some_and(|p| {
                self.ruptures
                    .get(r)
                    .is_none_or(|rupture| p.date.max(date) < rupture.date)
            })
        })
    }

    fn inscrire(
        &mut self,
        rang: u64,
        date: Date,
        releve: &[IdLigneReleve],
        comptables: &[IdLigneEcriture],
    ) -> Result<&Pointage, ErreurRapprochement> {
        if releve.is_empty() {
            return Err(ErreurRapprochement::SansLigneReleve);
        }
        if comptables.is_empty() {
            return Err(ErreurRapprochement::SansLigneComptable);
        }
        let mut ids_releve = BTreeSet::new();
        let mut total_releve = Decimal::ZERO;
        for id in releve {
            if !ids_releve.insert(*id) {
                return Err(ErreurRapprochement::LigneReleveEnDouble(*id));
            }
            let ligne = self
                .releve
                .get(id)
                .ok_or(ErreurRapprochement::LigneReleveInconnue(*id))?;
            if let Some(pointage) = self.conflit(self.index_releve.get(id), date) {
                return Err(ErreurRapprochement::ReleveDejaPointee {
                    ligne: *id,
                    pointage,
                });
            }
            total_releve = ajouter(total_releve, ligne.montant)?;
        }
        let mut ids_comptables = BTreeSet::new();
        let mut total_comptable = Decimal::ZERO;
        for id in comptables {
            if !ids_comptables.insert(*id) {
                return Err(ErreurRapprochement::LigneComptableEnDouble(*id));
            }
            let ligne = self
                .comptables
                .get(id)
                .ok_or(ErreurRapprochement::LigneComptableInconnue(*id))?;
            if let Some(pointage) = self.conflit(self.index_comptables.get(id), date) {
                return Err(ErreurRapprochement::ComptableDejaPointee {
                    ligne: *id,
                    pointage,
                });
            }
            total_comptable = ajouter(total_comptable, ligne.montant)?;
        }
        if total_releve != total_comptable {
            return Err(ErreurRapprochement::Desequilibre {
                releve: total_releve,
                comptable: total_comptable,
            });
        }
        for id in &ids_releve {
            self.index_releve.entry(*id).or_default().push(rang);
        }
        for id in &ids_comptables {
            self.index_comptables.entry(*id).or_default().push(rang);
        }
        self.dernier_rang = rang;
        Ok(self.pointages.entry(rang).or_insert(Pointage {
            rang,
            date,
            releve: ids_releve,
            comptables: ids_comptables,
            montant: total_releve,
        }))
    }

    fn inscrire_rupture(
        &mut self,
        rang: u64,
        pointage: u64,
        date: Date,
    ) -> Result<&Rupture, ErreurRapprochement> {
        if !self.pointages.contains_key(&pointage) {
            return Err(ErreurRapprochement::PointageInconnu(pointage));
        }
        if self.ruptures.contains_key(&pointage) {
            return Err(ErreurRapprochement::DejaRompu(pointage));
        }
        self.dernier_rang = rang;
        Ok(self.ruptures.entry(pointage).or_insert(Rupture {
            rang,
            pointage,
            date,
        }))
    }
}

/// Addition contrôlée : erreur au lieu d'une panique en cas de dépassement.
fn ajouter(a: Decimal, b: Decimal) -> Result<Decimal, ErreurRapprochement> {
    a.checked_add(b).ok_or(ErreurRapprochement::Depassement)
}

#[cfg(test)]
mod tests;
