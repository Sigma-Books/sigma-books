//! Éditions de base calculées à partir des écritures (spec §17.1) : balance générale
//! (2, 4, 6 ou 8 colonnes), grand livre général, journaux. Le rendu PDF et Excel relève du
//! lot L7.
//!
//! - **Mode** : « définitif » (écritures validées seulement) ou « provisoire » (avec les
//!   écritures équilibrées du brouillard).
//! - **Filtre** : bornes de dates dans un exercice, comptes (un compte retenu englobe ses
//!   sous-comptes et auxiliaires), journaux.
//! - **Ouverture** : écritures des journaux d'à-nouveaux, toujours, et mouvements de
//!   l'exercice antérieurs à la date de début. Les à-nouveaux ne sont jamais des mouvements de
//!   la période, même si elle commence le premier jour de l'exercice (Guide d'application,
//!   Application 127 : colonne « SOLDE N-1 » de la balance N).
//!
//! Colonnes de la balance :
//!
//! | Format | Colonnes | Statut |
//! |---|---|---|
//! | 6 | solde d'ouverture D, C ; mouvements D, C ; solde final D, C | **balance générale** (AUDCIF art. 19, p. PDF 22) quand la période part du début de l'exercice ([`Filtre::exercice`]) |
//! | 2 | solde final D, C | balance de travail, à valider par l'expert-comptable |
//! | 4 | totaux D, C ; solde final D, C | balance de travail, à valider |
//! | 8 | solde d'ouverture D, C ; mouvements D, C ; totaux D, C ; solde final D, C | balance de travail, à valider |
//!
//! L'art. 19 décrit la balance générale par le solde à l'ouverture de l'exercice, « le cumul
//! depuis l'ouverture de l'exercice des mouvements débiteurs et […] créditeurs » et le solde à
//! la date considérée ; le modèle du Guide d'application (Application 127) a ces six colonnes.
//! « Totaux » = solde d'ouverture + mouvements, par colonne. Le mot « cumul » est réservé au
//! sens de l'art. 19 (mouvements depuis l'ouverture).
//! Tous les calculs sont exacts ; un dépassement de capacité est une erreur, jamais une panique.

use std::collections::{BTreeMap, BTreeSet};

use rust_decimal::Decimal;
use time::Date;

use crate::compte::NumeroCompte;
use crate::ecriture::{EcritureEquilibree, IdTiers, Ligne};
use crate::exercice::Exercice;
use crate::journal::{CodeJournal, Journaux, TypeJournal};
use crate::livre::Livre;
use crate::numerotation::NumeroEcriture;
use crate::texte::TexteCourt;

/// Erreurs de calcul d'une édition.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ErreurEdition {
    /// Date de début après la date de fin.
    #[error("période vide : du {du} au {au}")]
    BornesInversees {
        /// Début.
        du: Date,
        /// Fin.
        au: Date,
    },
    /// Borne hors de l'exercice choisi.
    #[error("la date {0} est hors de l'exercice")]
    HorsExercice(Date),
    /// Dépassement de capacité d'un total.
    #[error("dépassement de capacité")]
    Depassement,
}

/// Écritures prises en compte.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Écritures validées seulement.
    Definitif,
    /// Écritures validées et écritures équilibrées du brouillard.
    Provisoire,
}

/// Regroupement des lignes de la balance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Regroupement {
    /// Une ligne par compte mouvementé.
    ParCompte,
    /// Une ligne par tronc officiel (spec §5.1 : les états agrègent par tronc).
    ParTronc,
}

/// Nombre de colonnes de la balance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormatBalance {
    /// Soldes.
    Deux,
    /// Totaux et soldes.
    Quatre,
    /// Ouverture, mouvements, soldes : la balance générale de l'AUDCIF (art. 19).
    Six,
    /// Ouverture, mouvements, totaux, soldes.
    Huit,
}

/// Filtre d'une édition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Filtre {
    debut_exercice: Date,
    du: Date,
    au: Date,
    comptes: Vec<NumeroCompte>,
    journaux: BTreeSet<CodeJournal>,
    a_nouveaux: BTreeSet<CodeJournal>,
}

impl Filtre {
    /// Période `[du, au]` dans l'exercice ; tous les comptes et tous les journaux. Les
    /// journaux d'à-nouveaux du dossier alimentent toujours la colonne d'ouverture.
    ///
    /// # Errors
    /// Bornes inversées ou hors de l'exercice.
    pub fn nouveau(
        exercice: &Exercice,
        journaux: &Journaux,
        du: Date,
        au: Date,
    ) -> Result<Self, ErreurEdition> {
        if du > au {
            return Err(ErreurEdition::BornesInversees { du, au });
        }
        for date in [du, au] {
            if date < exercice.debut() || date > exercice.fin() {
                return Err(ErreurEdition::HorsExercice(date));
            }
        }
        Ok(Self {
            debut_exercice: exercice.debut(),
            du,
            au,
            comptes: Vec::new(),
            journaux: BTreeSet::new(),
            a_nouveaux: journaux
                .journaux()
                .filter(|j| j.genre() == TypeJournal::ANouveaux)
                .map(|j| j.code().clone())
                .collect(),
        })
    }

    /// Du début de l'exercice à `au` : avec le format 6, c'est la balance générale de
    /// l'AUDCIF (art. 19).
    ///
    /// # Errors
    /// Date hors de l'exercice.
    pub fn exercice(
        exercice: &Exercice,
        journaux: &Journaux,
        au: Date,
    ) -> Result<Self, ErreurEdition> {
        Self::nouveau(exercice, journaux, exercice.debut(), au)
    }

    /// Restreint aux comptes donnés et à leurs descendants.
    #[must_use]
    pub fn comptes(mut self, comptes: impl IntoIterator<Item = NumeroCompte>) -> Self {
        self.comptes = comptes.into_iter().collect();
        self
    }

    /// Restreint aux journaux donnés.
    #[must_use]
    pub fn journaux(mut self, journaux: impl IntoIterator<Item = CodeJournal>) -> Self {
        self.journaux = journaux.into_iter().collect();
        self
    }

    fn retient_journal(&self, journal: &CodeJournal) -> bool {
        self.journaux.is_empty() || self.journaux.contains(journal)
    }

    fn retient_compte(&self, compte: &NumeroCompte) -> bool {
        self.comptes.is_empty() || self.comptes.iter().any(|c| c.est_ascendant_ou_egal(compte))
    }

    fn dans_ouverture(&self, source: &Source<'_>) -> bool {
        if self.a_nouveaux.contains(source.journal) {
            self.debut_exercice <= source.date && source.date <= self.au
        } else {
            self.debut_exercice <= source.date && source.date < self.du
        }
    }

    fn dans_bornes(&self, date: Date) -> bool {
        self.du <= date && date <= self.au
    }

    fn dans_periode(&self, source: &Source<'_>) -> bool {
        !self.a_nouveaux.contains(source.journal) && self.dans_bornes(source.date)
    }
}

/// Écriture vue par les éditions : validée (avec numéro) ou provisoire (sans numéro).
#[derive(Debug, Clone, Copy)]
struct Source<'a> {
    numero: Option<&'a NumeroEcriture>,
    journal: &'a CodeJournal,
    date: Date,
    libelle: &'a TexteCourt,
    lignes: &'a [Ligne],
}

fn sources<'a>(
    livre: &'a Livre,
    brouillard: &'a [EcritureEquilibree],
    mode: Mode,
) -> impl Iterator<Item = Source<'a>> {
    let validees = livre.ecritures().iter().map(|e| Source {
        numero: Some(e.numero()),
        journal: e.journal(),
        date: e.date(),
        libelle: e.libelle(),
        lignes: e.lignes(),
    });
    let provisoires = brouillard
        .iter()
        .filter(move |_| mode == Mode::Provisoire)
        .map(|e| Source {
            numero: None,
            journal: e.journal(),
            date: e.date(),
            libelle: e.libelle(),
            lignes: e.lignes(),
        });
    validees.chain(provisoires)
}

fn somme(a: Decimal, b: Decimal) -> Result<Decimal, ErreurEdition> {
    a.checked_add(b).ok_or(ErreurEdition::Depassement)
}

fn difference(a: Decimal, b: Decimal) -> Result<Decimal, ErreurEdition> {
    a.checked_sub(b).ok_or(ErreurEdition::Depassement)
}

/// Solde signé (débit − crédit) réparti en (débiteur, créditeur).
fn repartir(solde: Decimal) -> (Decimal, Decimal) {
    if solde >= Decimal::ZERO {
        (solde, Decimal::ZERO)
    } else {
        (Decimal::ZERO, -solde)
    }
}

/// Ligne de balance : totaux bruts d'ouverture et de période d'un compte ou d'un tronc.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LigneBalance {
    cle: String,
    ouverture_debit: Decimal,
    ouverture_credit: Decimal,
    debit: Decimal,
    credit: Decimal,
}

impl LigneBalance {
    /// Numéro du compte, ou tronc officiel en regroupement par tronc.
    #[must_use]
    pub fn cle(&self) -> &str {
        &self.cle
    }

    /// Mouvements de la période (débit, crédit).
    #[must_use]
    pub fn mouvements(&self) -> (Decimal, Decimal) {
        (self.debit, self.credit)
    }

    /// Solde d'ouverture (débiteur, créditeur).
    ///
    /// # Errors
    /// Dépassement de capacité.
    pub fn solde_ouverture(&self) -> Result<(Decimal, Decimal), ErreurEdition> {
        Ok(repartir(difference(
            self.ouverture_debit,
            self.ouverture_credit,
        )?))
    }

    /// Totaux (débit, crédit) : solde d'ouverture + mouvements de la période.
    ///
    /// # Errors
    /// Dépassement de capacité.
    pub fn totaux(&self) -> Result<(Decimal, Decimal), ErreurEdition> {
        let (od, oc) = self.solde_ouverture()?;
        Ok((somme(od, self.debit)?, somme(oc, self.credit)?))
    }

    /// Solde final (débiteur, créditeur).
    ///
    /// # Errors
    /// Dépassement de capacité.
    pub fn solde_final(&self) -> Result<(Decimal, Decimal), ErreurEdition> {
        let (d, c) = self.totaux()?;
        Ok(repartir(difference(d, c)?))
    }

    /// Valeurs des colonnes, dans l'ordre du format (voir la documentation du module).
    ///
    /// # Errors
    /// Dépassement de capacité.
    pub fn colonnes(&self, format: FormatBalance) -> Result<Vec<Decimal>, ErreurEdition> {
        let (od, oc) = self.solde_ouverture()?;
        let (cd, cc) = self.totaux()?;
        let (sd, sc) = self.solde_final()?;
        Ok(match format {
            FormatBalance::Deux => vec![sd, sc],
            FormatBalance::Quatre => vec![cd, cc, sd, sc],
            FormatBalance::Six => vec![od, oc, self.debit, self.credit, sd, sc],
            FormatBalance::Huit => vec![od, oc, self.debit, self.credit, cd, cc, sd, sc],
        })
    }
}

/// Balance générale.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Balance {
    lignes: Vec<LigneBalance>,
}

impl Balance {
    /// Calcule la balance : une ligne par compte (ou tronc) mouvementé, triée par clé.
    ///
    /// # Errors
    /// Dépassement de capacité.
    pub fn calculer(
        livre: &Livre,
        brouillard: &[EcritureEquilibree],
        mode: Mode,
        filtre: &Filtre,
        regroupement: Regroupement,
    ) -> Result<Self, ErreurEdition> {
        let mut lignes: BTreeMap<String, LigneBalance> = BTreeMap::new();
        for source in sources(livre, brouillard, mode) {
            let ouverture = filtre.dans_ouverture(&source);
            if !(ouverture || filtre.dans_periode(&source))
                || !filtre.retient_journal(source.journal)
            {
                continue;
            }
            for ligne in source.lignes {
                if !filtre.retient_compte(ligne.compte()) {
                    continue;
                }
                let cle = match regroupement {
                    Regroupement::ParCompte => ligne.compte().as_str(),
                    Regroupement::ParTronc => ligne.compte().tronc(),
                };
                let entree = lignes
                    .entry(cle.to_owned())
                    .or_insert_with(|| LigneBalance {
                        cle: cle.to_owned(),
                        ouverture_debit: Decimal::ZERO,
                        ouverture_credit: Decimal::ZERO,
                        debit: Decimal::ZERO,
                        credit: Decimal::ZERO,
                    });
                if ouverture {
                    entree.ouverture_debit = somme(entree.ouverture_debit, ligne.debit())?;
                    entree.ouverture_credit = somme(entree.ouverture_credit, ligne.credit())?;
                } else {
                    entree.debit = somme(entree.debit, ligne.debit())?;
                    entree.credit = somme(entree.credit, ligne.credit())?;
                }
            }
        }
        Ok(Self {
            lignes: lignes.into_values().collect(),
        })
    }

    /// Lignes, triées par clé.
    #[must_use]
    pub fn lignes(&self) -> &[LigneBalance] {
        &self.lignes
    }

    /// Totaux de chaque colonne.
    ///
    /// # Errors
    /// Dépassement de capacité.
    pub fn totaux(&self, format: FormatBalance) -> Result<Vec<Decimal>, ErreurEdition> {
        let largeur = match format {
            FormatBalance::Deux => 2,
            FormatBalance::Quatre => 4,
            FormatBalance::Six => 6,
            FormatBalance::Huit => 8,
        };
        let mut totaux = vec![Decimal::ZERO; largeur];
        for ligne in &self.lignes {
            for (total, valeur) in totaux.iter_mut().zip(ligne.colonnes(format)?) {
                *total = somme(*total, valeur)?;
            }
        }
        Ok(totaux)
    }
}

/// Ligne du grand livre.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LigneGrandLivre {
    /// Date comptable.
    pub date: Date,
    /// Journal.
    pub journal: CodeJournal,
    /// Numéro définitif ; `None` pour une écriture du brouillard (mode provisoire).
    pub numero: Option<NumeroEcriture>,
    /// Libellé de l'écriture.
    pub libelle: TexteCourt,
    /// Tiers de la ligne.
    pub tiers: Option<IdTiers>,
    /// Débit.
    pub debit: Decimal,
    /// Crédit.
    pub credit: Decimal,
    /// Solde progressif signé (débit − crédit), ouverture comprise.
    pub solde: Decimal,
}

/// Compte du grand livre : solde d'ouverture, lignes de la période, totaux.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompteGrandLivre {
    /// Compte.
    pub compte: NumeroCompte,
    /// Solde d'ouverture signé (débit − crédit).
    pub solde_ouverture: Decimal,
    /// Lignes de la période, par date puis numéro (brouillard en fin de journée).
    pub lignes: Vec<LigneGrandLivre>,
    /// Total des débits de la période.
    pub total_debit: Decimal,
    /// Total des crédits de la période.
    pub total_credit: Decimal,
    /// Solde final signé.
    pub solde_final: Decimal,
}

/// Grand livre général : un bloc par compte mouvementé, trié par numéro.
///
/// # Errors
/// Dépassement de capacité.
pub fn grand_livre(
    livre: &Livre,
    brouillard: &[EcritureEquilibree],
    mode: Mode,
    filtre: &Filtre,
) -> Result<Vec<CompteGrandLivre>, ErreurEdition> {
    struct Brut {
        ouverture: Decimal,
        lignes: Vec<LigneGrandLivre>,
    }
    let mut comptes: BTreeMap<NumeroCompte, Brut> = BTreeMap::new();
    for source in sources(livre, brouillard, mode) {
        let ouverture = filtre.dans_ouverture(&source);
        if !(ouverture || filtre.dans_periode(&source)) || !filtre.retient_journal(source.journal) {
            continue;
        }
        for ligne in source.lignes {
            if !filtre.retient_compte(ligne.compte()) {
                continue;
            }
            let brut = comptes.entry(ligne.compte().clone()).or_insert(Brut {
                ouverture: Decimal::ZERO,
                lignes: Vec::new(),
            });
            if ouverture {
                brut.ouverture = somme(brut.ouverture, difference(ligne.debit(), ligne.credit())?)?;
            } else {
                brut.lignes.push(LigneGrandLivre {
                    date: source.date,
                    journal: source.journal.clone(),
                    numero: source.numero.cloned(),
                    libelle: source.libelle.clone(),
                    tiers: ligne.tiers().cloned(),
                    debit: ligne.debit(),
                    credit: ligne.credit(),
                    solde: Decimal::ZERO,
                });
            }
        }
    }
    let mut resultat = Vec::with_capacity(comptes.len());
    for (compte, mut brut) in comptes {
        // Tri stable : date, puis écritures numérotées avant le brouillard, puis numéro.
        brut.lignes.sort_by(|a, b| {
            (a.date, a.numero.is_none(), &a.numero).cmp(&(b.date, b.numero.is_none(), &b.numero))
        });
        let (mut solde, mut total_debit, mut total_credit) =
            (brut.ouverture, Decimal::ZERO, Decimal::ZERO);
        for ligne in &mut brut.lignes {
            solde = somme(solde, difference(ligne.debit, ligne.credit)?)?;
            total_debit = somme(total_debit, ligne.debit)?;
            total_credit = somme(total_credit, ligne.credit)?;
            ligne.solde = solde;
        }
        resultat.push(CompteGrandLivre {
            compte,
            solde_ouverture: brut.ouverture,
            lignes: brut.lignes,
            total_debit,
            total_credit,
            solde_final: solde,
        });
    }
    Ok(resultat)
}

/// Écriture telle qu'elle figure dans l'édition d'un journal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EcritureJournal {
    /// Numéro définitif ; `None` pour le brouillard.
    pub numero: Option<NumeroEcriture>,
    /// Date comptable.
    pub date: Date,
    /// Libellé.
    pub libelle: TexteCourt,
    /// Lignes, dans l'ordre de saisie.
    pub lignes: Vec<Ligne>,
}

/// Édition d'un journal sur la période : écritures et totaux.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditionJournal {
    /// Journal.
    pub journal: CodeJournal,
    /// Écritures, par date puis numéro (brouillard en fin de journée).
    pub ecritures: Vec<EcritureJournal>,
    /// Total des débits.
    pub total_debit: Decimal,
    /// Total des crédits (égal au total des débits).
    pub total_credit: Decimal,
}

/// Journaux de la période : un bloc par journal retenu et mouvementé, trié par code. Le filtre
/// de comptes ne s'applique pas : une écriture figure toujours en entier dans son journal.
///
/// # Errors
/// Dépassement de capacité.
pub fn journaux(
    livre: &Livre,
    brouillard: &[EcritureEquilibree],
    mode: Mode,
    filtre: &Filtre,
) -> Result<Vec<EditionJournal>, ErreurEdition> {
    let mut blocs: BTreeMap<CodeJournal, Vec<EcritureJournal>> = BTreeMap::new();
    for source in sources(livre, brouillard, mode) {
        // Édition d'un journal : les à-nouveaux figurent dans le journal AN à leur date.
        if !filtre.dans_bornes(source.date) || !filtre.retient_journal(source.journal) {
            continue;
        }
        blocs
            .entry(source.journal.clone())
            .or_default()
            .push(EcritureJournal {
                numero: source.numero.cloned(),
                date: source.date,
                libelle: source.libelle.clone(),
                lignes: source.lignes.to_vec(),
            });
    }
    let mut resultat = Vec::with_capacity(blocs.len());
    for (journal, mut ecritures) in blocs {
        ecritures.sort_by(|a, b| {
            (a.date, a.numero.is_none(), &a.numero).cmp(&(b.date, b.numero.is_none(), &b.numero))
        });
        let (mut total_debit, mut total_credit) = (Decimal::ZERO, Decimal::ZERO);
        for ligne in ecritures.iter().flat_map(|e| &e.lignes) {
            total_debit = somme(total_debit, ligne.debit())?;
            total_credit = somme(total_credit, ligne.credit())?;
        }
        resultat.push(EditionJournal {
            journal,
            ecritures,
            total_debit,
            total_credit,
        });
    }
    Ok(resultat)
}
