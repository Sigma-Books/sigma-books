//! Éditions de tiers (spec §10.3 ; ADR 0033, points 4 et 5) : balance âgée à une date, grand
//! livre auxiliaire, balance auxiliaire, relevé de compte d'un tiers. Éditions de gestion et de
//! justification, facultatives (AUDCIF art. 19, p. PDF 22-23 : livres auxiliaires
//! facultatifs) ; le rendu PDF relève du lot L7.
//!
//! - **Lignes** : lignes de tiers ([`LigneTiers`]) d'écritures validées ou, en mode provisoire,
//!   du brouillard ([`Mode`], comme les éditions de L2), regroupées par **compte collectif puis
//!   par tiers résolu** au sens de la fusion (ADR 0031 : la résolution est faite par l'appelant,
//!   en un seul endroit). Une ligne du brouillard compte pour son montant entier : elle n'a pas
//!   d'identité stable et ne peut être lettrée (ADR 0032, point 2).
//! - **Lettrages datés** ([`Lettrages`]) : relus des registres de lettrage
//!   ([`RegistreLettrage`]) des comptes lettrables. Un lien compte à une date `D` si son
//!   lettrage est daté au plus tard de `D` (date de l'**opération**, ADR 0032, point 3), s'il
//!   n'était pas rompu à `D` (rupture datée au plus tard de `D`) et si ses deux lignes étaient
//!   comptabilisées à `D` (date comptable au plus tard `D`). Un lettrage postérieur à `D` ne
//!   solde donc pas la ligne à `D` : le résultat à une date ne dépend d'aucune opération
//!   postérieure (ADR 0033, point 4).
//! - **Échéance** : échéance effective à la date de l'édition ([`EcheanceLigne::effective`],
//!   ADR 0033, point 2), à défaut la date comptable.
//! - **Aucune compensation** entre tiers (AUDCIF fiches 40, 41, 42, 47 ; ADR 0033, point 4) :
//!   débits et crédits restent séparés dans les totaux.
//!
//! La troisième condition d'un lien (ses deux lignes comptabilisées à `D`) n'est pas écrite
//! dans l'ADR 0033 ; elle découle de la propriété qu'il exige (somme des tranches = solde du
//! compte auxiliaire) : sans elle, un lettrage daté avant la date comptable de sa contrepartie
//! solderait à `D` une ligne dont la contrepartie n'est pas encore dans les livres. OCA
//! `partner_statement` (`report_statement_common.py:84-95`) écarte de même les liens dont la
//! contrepartie est postérieure à la date. Tryton et Odoo y arrivent par la date du lettrage
//! (plus haute date des lignes).
//!
//! Tous les calculs sont exacts ; un dépassement de capacité est une erreur, jamais une
//! panique. Les sorties sont triées (compte, tiers, puis date et numéro) : même entrée, même
//! sortie, quel que soit l'ordre des lignes et des registres.

mod auxiliaire;
mod balance_agee;

use std::collections::{BTreeMap, BTreeSet};

use rust_decimal::Decimal;
use time::Date;

use crate::compte::NumeroCompte;
use crate::echeance::{EcheanceLigne, Residuel};
use crate::ecriture::{EcritureEquilibree, IdTiers, Ligne, Sens};
use crate::editions::Mode;
use crate::journal::CodeJournal;
use crate::lettrage::{IdLigne, Lettre, RegistreLettrage};
use crate::livre::EcritureValidee;
use crate::numerotation::NumeroEcriture;
use crate::texte::TexteCourt;

pub use auxiliaire::{
    BalanceAuxiliaire, CompteTiers, LigneAuxiliaire, LigneBalanceAuxiliaire, Releve,
    balance_auxiliaire, grand_livre_auxiliaire, releve_tiers,
};
pub use balance_agee::{
    BORNES_MAX, BORNES_PAR_DEFAUT, BalanceAgee, ColonneAgee, CompteAgee, LigneAgee, TiersAgee,
    TotalAgee, Tranche, Tranches, balance_agee,
};

/// Erreurs des éditions de tiers.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ErreurEditionTiers {
    /// Position de ligne absente de l'écriture.
    #[error("ligne {position} absente de l'écriture")]
    LigneInconnue {
        /// Position demandée (à partir de 1).
        position: u16,
    },
    /// Ligne sans tiers : elle n'entre pas dans une édition de tiers.
    #[error("ligne {position} sans tiers")]
    SansTiers {
        /// Position de la ligne (à partir de 1).
        position: u16,
    },
    /// Échéance reconstituée pour une autre date comptable que celle de l'écriture.
    #[error("échéance incohérente avec la date comptable de la ligne {0}")]
    EcheanceIncoherente(IdLigne),
    /// Même ligne validée fournie deux fois.
    #[error("ligne en double : {0}")]
    LigneEnDouble(IdLigne),
    /// Deux registres de lettrage pour le même compte.
    #[error("registre de lettrage en double pour le compte {0}")]
    RegistreEnDouble(NumeroCompte),
    /// Lien de lettrage qui cite la ligne sur un autre compte ou dans l'autre sens.
    #[error("lettrage incohérent avec la ligne {0}")]
    LettrageIncoherent(IdLigne),
    /// Liens actifs dont le total dépasse le montant de la ligne (dates d'opérations
    /// incohérentes).
    #[error("résiduel hors bornes pour la ligne {0}")]
    ResiduelHorsBornes(IdLigne),
    /// Bornes de tranches non strictement croissantes, nulles ou trop nombreuses.
    #[error("bornes de tranches invalides")]
    BornesInvalides,
    /// Dépassement de capacité d'un total.
    #[error("dépassement de capacité")]
    Depassement,
}

fn somme(a: Decimal, b: Decimal) -> Result<Decimal, ErreurEditionTiers> {
    a.checked_add(b).ok_or(ErreurEditionTiers::Depassement)
}

fn difference(a: Decimal, b: Decimal) -> Result<Decimal, ErreurEditionTiers> {
    a.checked_sub(b).ok_or(ErreurEditionTiers::Depassement)
}

/// Provenance d'une ligne de tiers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceLigne {
    /// Ligne d'une écriture validée.
    Validee {
        /// Identifiant (rang de chaîne, position).
        id: IdLigne,
        /// Numéro définitif de l'écriture.
        numero: NumeroEcriture,
    },
    /// Ligne d'une écriture équilibrée du brouillard (mode provisoire).
    Brouillard {
        /// Position de la ligne dans l'écriture (à partir de 1).
        position: u16,
    },
}

/// Ligne de tiers vue par les éditions : immuable, construite à partir d'une ligne d'écriture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LigneTiers {
    source: SourceLigne,
    journal: CodeJournal,
    date: Date,
    date_valeur_comptable: Option<Date>,
    libelle: TexteCourt,
    compte: NumeroCompte,
    tiers: IdTiers,
    sens: Sens,
    montant: Decimal,
    echeance: EcheanceLigne,
}

impl LigneTiers {
    /// Ligne `id.position()` d'une écriture validée de rang de chaîne `id.rang()`.
    ///
    /// **Contrat** (comme [`crate::lettrage::LigneLettrable::nouvelle`]) : `id` est bien
    /// l'identifiant de cette ligne ; `tiers_resolu` est son tiers **résolu** au sens de la
    /// fusion (ADR 0031) ; `echeance` est son échéance relue (initiale et révisions, ADR 0033).
    ///
    /// # Errors
    /// Position hors de l'écriture, ligne sans tiers, échéance d'une autre date comptable.
    pub fn validee(
        ecriture: &EcritureValidee,
        id: IdLigne,
        tiers_resolu: IdTiers,
        echeance: EcheanceLigne,
    ) -> Result<Self, ErreurEditionTiers> {
        if echeance.date_comptable() != ecriture.date() {
            return Err(ErreurEditionTiers::EcheanceIncoherente(id));
        }
        let ligne = ligne_a(ecriture.lignes(), id.position())?;
        Ok(Self {
            source: SourceLigne::Validee {
                id,
                numero: ecriture.numero().clone(),
            },
            journal: ecriture.journal().clone(),
            date: ecriture.date(),
            date_valeur_comptable: ecriture.date_valeur_comptable(),
            libelle: ecriture.libelle().clone(),
            compte: ligne.compte().clone(),
            tiers: tiers_resolu,
            sens: ligne.sens(),
            montant: ligne.montant().valeur(),
            echeance,
        })
    }

    /// Ligne `position` (à partir de 1) d'une écriture équilibrée du brouillard ; `echeance` est
    /// la date d'échéance saisie sur la ligne (`brouillon_ligne.date_echeance`), à défaut la
    /// date comptable.
    ///
    /// # Errors
    /// Position hors de l'écriture, ligne sans tiers.
    pub fn provisoire(
        ecriture: &EcritureEquilibree,
        position: u16,
        tiers_resolu: IdTiers,
        echeance: Option<Date>,
    ) -> Result<Self, ErreurEditionTiers> {
        let ligne = ligne_a(ecriture.lignes(), position)?;
        Ok(Self {
            source: SourceLigne::Brouillard { position },
            journal: ecriture.journal().clone(),
            date: ecriture.date(),
            date_valeur_comptable: ecriture.date_valeur_comptable(),
            libelle: ecriture.libelle().clone(),
            compte: ligne.compte().clone(),
            tiers: tiers_resolu,
            sens: ligne.sens(),
            montant: ligne.montant().valeur(),
            echeance: EcheanceLigne::nouvelle(ecriture.date(), echeance),
        })
    }

    /// Provenance.
    #[must_use]
    pub fn source(&self) -> &SourceLigne {
        &self.source
    }

    /// Identifiant d'une ligne validée ; `None` pour le brouillard.
    #[must_use]
    pub fn id(&self) -> Option<IdLigne> {
        match self.source {
            SourceLigne::Validee { id, .. } => Some(id),
            SourceLigne::Brouillard { .. } => None,
        }
    }

    /// Numéro définitif ; `None` pour le brouillard.
    #[must_use]
    pub fn numero(&self) -> Option<&NumeroEcriture> {
        match &self.source {
            SourceLigne::Validee { numero, .. } => Some(numero),
            SourceLigne::Brouillard { .. } => None,
        }
    }

    /// Journal.
    #[must_use]
    pub fn journal(&self) -> &CodeJournal {
        &self.journal
    }

    /// Date comptable.
    #[must_use]
    pub fn date(&self) -> Date {
        self.date
    }

    /// Date de valeur comptable d'une opération tardive (AUDCIF art. 22 4°).
    #[must_use]
    pub fn date_valeur_comptable(&self) -> Option<Date> {
        self.date_valeur_comptable
    }

    /// Libellé de l'écriture.
    #[must_use]
    pub fn libelle(&self) -> &TexteCourt {
        &self.libelle
    }

    /// Compte collectif.
    #[must_use]
    pub fn compte(&self) -> &NumeroCompte {
        &self.compte
    }

    /// Tiers résolu.
    #[must_use]
    pub fn tiers(&self) -> &IdTiers {
        &self.tiers
    }

    /// Sens.
    #[must_use]
    pub fn sens(&self) -> Sens {
        self.sens
    }

    /// Montant en devise de tenue, strictement positif.
    #[must_use]
    pub fn montant(&self) -> Decimal {
        self.montant
    }

    /// Échéance de la ligne (initiale et révisions).
    #[must_use]
    pub fn echeance(&self) -> &EcheanceLigne {
        &self.echeance
    }

    fn debit(&self) -> Decimal {
        match self.sens {
            Sens::Debit => self.montant,
            Sens::Credit => Decimal::ZERO,
        }
    }

    fn credit(&self) -> Decimal {
        match self.sens {
            Sens::Debit => Decimal::ZERO,
            Sens::Credit => self.montant,
        }
    }

    /// Valeur signée (débit positif, crédit négatif).
    fn signer(&self, valeur: Decimal) -> Decimal {
        match self.sens {
            Sens::Debit => valeur,
            Sens::Credit => -valeur,
        }
    }

    fn retenue(&self, mode: Mode) -> bool {
        mode == Mode::Provisoire || self.id().is_some()
    }

    /// Clé de tri des éditions : date, écritures numérotées avant le brouillard, numéro, puis
    /// position de la ligne.
    fn cle_tri(&self) -> (Date, bool, Option<&NumeroEcriture>, u16) {
        let position = match self.source {
            SourceLigne::Validee { id, .. } => id.position(),
            SourceLigne::Brouillard { position } => position,
        };
        (self.date, self.id().is_none(), self.numero(), position)
    }
}

fn ligne_a(lignes: &[Ligne], position: u16) -> Result<&Ligne, ErreurEditionTiers> {
    let ligne = usize::from(position)
        .checked_sub(1)
        .and_then(|i| lignes.get(i))
        .ok_or(ErreurEditionTiers::LigneInconnue { position })?;
    if ligne.tiers().is_none() {
        return Err(ErreurEditionTiers::SansTiers { position });
    }
    Ok(ligne)
}

/// Refuse une même ligne validée fournie deux fois.
fn controler_doublons(lignes: &[LigneTiers]) -> Result<(), ErreurEditionTiers> {
    let mut vues = BTreeSet::new();
    for id in lignes.iter().filter_map(LigneTiers::id) {
        if !vues.insert(id) {
            return Err(ErreurEditionTiers::LigneEnDouble(id));
        }
    }
    Ok(())
}

/// Part d'un lien de lettrage vue d'une de ses lignes.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Imputation {
    compte: NumeroCompte,
    lettre: Lettre,
    /// Sens de la ligne dans le lien.
    sens: Sens,
    montant: Decimal,
    /// Date de l'opération de lettrage.
    date: Date,
    /// Date de la rupture du lettrage, s'il est rompu.
    rupture: Option<Date>,
    /// Date comptable de l'autre ligne du lien.
    date_contrepartie: Date,
}

impl Imputation {
    /// Le lien compte à `date` : lettré et non rompu à cette date, contrepartie comptabilisée.
    fn active_a(&self, date: Date) -> bool {
        self.date <= date && self.date_contrepartie <= date && self.rupture.is_none_or(|r| r > date)
    }
}

/// Lettre d'une ligne dans une édition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct LettreEdition {
    lettre: Lettre,
    posterieure: bool,
}

impl LettreEdition {
    /// Lettre.
    #[must_use]
    pub fn lettre(&self) -> Lettre {
        self.lettre
    }

    /// Vrai pour un lettrage **postérieur** à la date de l'édition (non rompu à ce jour), marqué
    /// comme tel (ADR 0033, point 5) ; il ne solde pas la ligne à cette date.
    #[must_use]
    pub fn est_posterieure(&self) -> bool {
        self.posterieure
    }
}

/// Lettrages datés d'un dossier, indexés par ligne, relus des registres de ses comptes
/// lettrables. Construire l'index ne modifie aucun registre.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Lettrages {
    par_ligne: BTreeMap<IdLigne, Vec<Imputation>>,
}

impl Lettrages {
    /// Index sans lettrage (comptes non lettrables ou mode sans lettrage).
    #[must_use]
    pub fn aucun() -> Self {
        Self::default()
    }

    /// Index des lettrages (rompus compris) des registres donnés, un par compte, dans un
    /// ordre quelconque.
    ///
    /// # Errors
    /// [`ErreurEditionTiers::RegistreEnDouble`] si deux registres portent sur le même compte.
    pub fn depuis_registres<'a>(
        registres: impl IntoIterator<Item = &'a RegistreLettrage>,
    ) -> Result<Self, ErreurEditionTiers> {
        let mut comptes = BTreeSet::new();
        let mut par_ligne: BTreeMap<IdLigne, Vec<Imputation>> = BTreeMap::new();
        for registre in registres {
            if !comptes.insert(registre.compte().clone()) {
                return Err(ErreurEditionTiers::RegistreEnDouble(
                    registre.compte().clone(),
                ));
            }
            let dates: BTreeMap<IdLigne, Date> =
                registre.lignes().map(|l| (l.id(), l.date())).collect();
            for lettrage in registre.lettrages() {
                let rupture = registre
                    .rupture(lettrage.lettre())
                    .map(crate::lettrage::Rupture::date);
                for lien in lettrage.liens() {
                    for (id, sens, autre) in [
                        (lien.debit(), Sens::Debit, lien.credit()),
                        (lien.credit(), Sens::Credit, lien.debit()),
                    ] {
                        // Un lien cite toujours deux lignes du registre (contrôle du lettrage).
                        let Some(date_contrepartie) = dates.get(&autre).copied() else {
                            return Err(ErreurEditionTiers::LettrageIncoherent(autre));
                        };
                        par_ligne.entry(id).or_default().push(Imputation {
                            compte: registre.compte().clone(),
                            lettre: lettrage.lettre(),
                            sens,
                            montant: lien.montant(),
                            date: lettrage.date(),
                            rupture,
                            date_contrepartie,
                        });
                    }
                }
            }
        }
        Ok(Self { par_ligne })
    }

    fn imputations(
        &self,
        ligne: &LigneTiers,
    ) -> Result<(Option<IdLigne>, &[Imputation]), ErreurEditionTiers> {
        let Some(id) = ligne.id() else {
            return Ok((None, &[]));
        };
        let imputations = self.par_ligne.get(&id).map_or(&[][..], Vec::as_slice);
        if imputations
            .iter()
            .any(|i| i.compte != ligne.compte || i.sens != ligne.sens)
        {
            return Err(ErreurEditionTiers::LettrageIncoherent(id));
        }
        Ok((Some(id), imputations))
    }

    /// Résiduel de la ligne à `date` : son montant moins les liens qui comptent à cette date.
    /// Une ligne du brouillard n'est jamais lettrée.
    ///
    /// # Errors
    /// Lettrage incohérent avec la ligne, résiduel hors bornes, dépassement de capacité.
    pub fn residuel_a(
        &self,
        ligne: &LigneTiers,
        date: Date,
    ) -> Result<Residuel, ErreurEditionTiers> {
        let (id, imputations) = self.imputations(ligne)?;
        let mut impute = Decimal::ZERO;
        for imputation in imputations.iter().filter(|i| i.active_a(date)) {
            impute = somme(impute, imputation.montant)?;
        }
        let reste = difference(ligne.montant, impute)?;
        Residuel::nouveau(ligne.montant, reste).map_err(|_| match id {
            Some(id) => ErreurEditionTiers::ResiduelHorsBornes(id),
            None => ErreurEditionTiers::Depassement,
        })
    }

    /// Lettres de la ligne à `date`, dans l'ordre des lettres : lettrages qui comptent à cette
    /// date, puis lettrages postérieurs jamais rompus, marqués. Un lettrage rompu au plus tard
    /// à `date`, ou postérieur puis rompu, n'apparaît pas.
    ///
    /// # Errors
    /// Lettrage incohérent avec la ligne.
    pub fn lettres_a(
        &self,
        ligne: &LigneTiers,
        date: Date,
    ) -> Result<Vec<LettreEdition>, ErreurEditionTiers> {
        let (_, imputations) = self.imputations(ligne)?;
        let mut lettres: BTreeMap<Lettre, bool> = BTreeMap::new();
        for imputation in imputations {
            if imputation.active_a(date) {
                lettres.insert(imputation.lettre, false);
            } else if imputation.rupture.is_none() {
                lettres.entry(imputation.lettre).or_insert(true);
            }
        }
        Ok(lettres
            .into_iter()
            .map(|(lettre, posterieure)| LettreEdition {
                lettre,
                posterieure,
            })
            .collect())
    }
}

#[cfg(test)]
mod tests;
