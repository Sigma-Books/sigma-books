//! État de rapprochement à une date (spec §9.5 ; ADR 0034, point 9 et amendement du
//! 2026-11-27).
//!
//! **Formule de l'ADR**, les deux soldes à la date D :
//! `écart = (solde comptable − écritures non pointées) − (solde du relevé − lignes non pointées)`.
//!
//! - **Solde comptable** : solde d'ouverture comptable plus les lignes d'écriture validées du
//!   registre de date comptable ≤ D (débit positif).
//! - **Solde du relevé** : solde d'ouverture déclaré du premier relevé plus les lignes de
//!   relevé de date ≤ D (ADR 0034 : « la PR 13b affine »).
//! - **Pointée à D** : une ligne d'un pointage **non rompu** dans le registre fourni dont
//!   **toutes** les lignes sont datées au plus tard D. Les dates des opérations de pointage et
//!   de rupture n'interviennent pas : l'état est établi sur le registre de l'instant où il est
//!   calculé, puis figé (aucun moteur ne reconstitue un état passé) ; un état de fin de mois
//!   établi après coup compte donc les pointages faits après la fin du mois. Un pointage qui
//!   enjambe la date (une ligne au-delà de D) ne pointe rien à D : ses lignes antérieures
//!   restent listées, ce sont les « chevauchements de dates » de la fiche 52 (AUDCIF
//!   p. PDF 416).
//! - **Non pointées** (suspens) : les lignes de date ≤ D qui ne sont pas pointées à D, listées
//!   des deux côtés, triées par date puis identifiant.
//!
//! **Ouverture.** Les deux soldes d'ouverture sont des paramètres ([`Ouverture`]) : le solde
//! d'ouverture comptable couvre les lignes que l'appelant ne confie pas au registre (par
//! exemple l'écriture d'à-nouveaux, qui reprend un solde sans pouvoir se pointer).
//!
//! **Ce que dit l'écart.** Chaque pointage compté est équilibré et toutes ses lignes sont dans
//! les soldes : l'écart égale donc toujours `ouverture comptable − ouverture du relevé`
//! (identité du rapprochement bancaire : les suspens expliquent la différence des soldes).
//! **Écart nul** signifie : les soldes d'ouverture concordent et chaque différence entre le
//! solde comptable et le solde du relevé est un suspens listé. Il ne dit pas que les suspens
//! sont légitimes : un contrôle (clôture, L6) lit aussi
//! [`EtatRapprochement::releve_non_pointees`] et
//! [`EtatRapprochement::comptables_non_pointees`], pas [`EtatRapprochement::ecart_nul`] seul.
//!
//! Le calcul ne dépend ni de l'ordre d'ajout des lignes ni de l'ordre des pointages.

use std::collections::BTreeSet;

use rust_decimal::Decimal;
use time::Date;

use super::{
    ErreurRapprochement, IdLigneEcriture, IdLigneReleve, LigneComptable, LigneReleve, Pointage,
    RegistrePointage, ajouter,
};

/// Soldes d'ouverture du rapprochement d'un compte de trésorerie.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Ouverture {
    releve: Decimal,
    comptable: Decimal,
}

impl Ouverture {
    /// `releve` : solde d'ouverture déclaré du premier relevé du compte ; `comptable` : solde
    /// des lignes du compte qui ne sont pas confiées au registre (débit positif).
    #[must_use]
    pub fn nouvelle(releve: Decimal, comptable: Decimal) -> Self {
        Self { releve, comptable }
    }

    /// Solde d'ouverture du relevé.
    #[must_use]
    pub fn releve(&self) -> Decimal {
        self.releve
    }

    /// Solde d'ouverture comptable.
    #[must_use]
    pub fn comptable(&self) -> Decimal {
        self.comptable
    }
}

/// État de rapprochement calculé à une date : la matière de l'état figé (ADR 0034, point 9 ;
/// le format canonique `etat-rapprochement-v1` relève de la PR 13b).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EtatRapprochement {
    date: Date,
    ouverture: Ouverture,
    solde_releve: Decimal,
    solde_comptable: Decimal,
    releve_non_pointees: Vec<LigneReleve>,
    comptables_non_pointees: Vec<LigneComptable>,
    total_releve_non_pointees: Decimal,
    total_comptables_non_pointees: Decimal,
    pointages: Vec<u64>,
    ecart: Decimal,
}

impl EtatRapprochement {
    /// Date de l'état.
    #[must_use]
    pub fn date(&self) -> Date {
        self.date
    }

    /// Soldes d'ouverture retenus.
    #[must_use]
    pub fn ouverture(&self) -> Ouverture {
        self.ouverture
    }

    /// Solde du relevé à la date.
    #[must_use]
    pub fn solde_releve(&self) -> Decimal {
        self.solde_releve
    }

    /// Solde comptable à la date.
    #[must_use]
    pub fn solde_comptable(&self) -> Decimal {
        self.solde_comptable
    }

    /// Lignes de relevé non pointées, par date puis identifiant.
    #[must_use]
    pub fn releve_non_pointees(&self) -> &[LigneReleve] {
        &self.releve_non_pointees
    }

    /// Lignes d'écriture non pointées, par date puis identifiant.
    #[must_use]
    pub fn comptables_non_pointees(&self) -> &[LigneComptable] {
        &self.comptables_non_pointees
    }

    /// Somme signée des lignes de relevé non pointées.
    #[must_use]
    pub fn total_releve_non_pointees(&self) -> Decimal {
        self.total_releve_non_pointees
    }

    /// Somme signée des lignes d'écriture non pointées.
    #[must_use]
    pub fn total_comptables_non_pointees(&self) -> Decimal {
        self.total_comptables_non_pointees
    }

    /// Rangs des pointages pris en compte, croissants.
    #[must_use]
    pub fn pointages(&self) -> &[u64] {
        &self.pointages
    }

    /// Écart : `(solde comptable − non pointées) − (solde du relevé − non pointées)`, égal à
    /// `ouverture comptable − ouverture du relevé` (voir la documentation du module).
    #[must_use]
    pub fn ecart(&self) -> Decimal {
        self.ecart
    }

    /// Vrai si l'écart est nul (critère de la spec §9.5 et §15.1) : les ouvertures concordent
    /// et chaque différence de soldes est un suspens listé. À lire avec les listes de suspens.
    #[must_use]
    pub fn ecart_nul(&self) -> bool {
        self.ecart.is_zero()
    }
}

impl RegistrePointage {
    /// Calcule l'état de rapprochement à la date (voir la documentation du module `etat`).
    ///
    /// # Errors
    /// [`ErreurRapprochement::Depassement`] si une somme dépasse la capacité du décimal.
    pub fn etat(
        &self,
        date: Date,
        ouverture: Ouverture,
    ) -> Result<EtatRapprochement, ErreurRapprochement> {
        let pointages: Vec<&Pointage> = self
            .pointages
            .values()
            .filter(|p| self.pris_en_compte(p, date))
            .collect();
        let pointees_releve: BTreeSet<IdLigneReleve> = pointages
            .iter()
            .flat_map(|p| p.releve.iter().copied())
            .collect();
        let pointees_comptables: BTreeSet<IdLigneEcriture> = pointages
            .iter()
            .flat_map(|p| p.comptables.iter().copied())
            .collect();
        let est_pointee_r = |id: &IdLigneReleve| pointees_releve.contains(id);
        let est_pointee_c = |id: &IdLigneEcriture| pointees_comptables.contains(id);

        let mut solde_releve = ouverture.releve;
        let mut total_releve_non_pointees = Decimal::ZERO;
        let mut releve_non_pointees = Vec::new();
        for ligne in self.releve.values().filter(|l| l.date <= date) {
            solde_releve = ajouter(solde_releve, ligne.montant)?;
            if !est_pointee_r(&ligne.id) {
                total_releve_non_pointees = ajouter(total_releve_non_pointees, ligne.montant)?;
                releve_non_pointees.push(ligne.clone());
            }
        }
        let mut solde_comptable = ouverture.comptable;
        let mut total_comptables_non_pointees = Decimal::ZERO;
        let mut comptables_non_pointees = Vec::new();
        for ligne in self.comptables.values().filter(|l| l.date <= date) {
            solde_comptable = ajouter(solde_comptable, ligne.montant)?;
            if !est_pointee_c(&ligne.id) {
                total_comptables_non_pointees =
                    ajouter(total_comptables_non_pointees, ligne.montant)?;
                comptables_non_pointees.push(ligne.clone());
            }
        }
        releve_non_pointees.sort_by_key(|l| (l.date, l.id));
        comptables_non_pointees.sort_by_key(|l| (l.date, l.id));

        let comptable_rapproche = soustraire(solde_comptable, total_comptables_non_pointees)?;
        let releve_rapproche = soustraire(solde_releve, total_releve_non_pointees)?;
        let ecart = soustraire(comptable_rapproche, releve_rapproche)?;
        Ok(EtatRapprochement {
            date,
            ouverture,
            solde_releve,
            solde_comptable,
            releve_non_pointees,
            comptables_non_pointees,
            total_releve_non_pointees,
            total_comptables_non_pointees,
            pointages: pointages.iter().map(|p| p.rang).collect(),
            ecart,
        })
    }

    /// Pointage non rompu dont toutes les lignes sont datées au plus tard ce jour.
    fn pris_en_compte(&self, pointage: &Pointage, date: Date) -> bool {
        !self.ruptures.contains_key(&pointage.rang)
            && pointage
                .releve
                .iter()
                .all(|id| self.releve.get(id).is_some_and(|l| l.date <= date))
            && pointage
                .comptables
                .iter()
                .all(|id| self.comptables.get(id).is_some_and(|l| l.date <= date))
    }
}

/// Soustraction contrôlée : erreur au lieu d'une panique en cas de dépassement.
fn soustraire(a: Decimal, b: Decimal) -> Result<Decimal, ErreurRapprochement> {
    a.checked_sub(b).ok_or(ErreurRapprochement::Depassement)
}
