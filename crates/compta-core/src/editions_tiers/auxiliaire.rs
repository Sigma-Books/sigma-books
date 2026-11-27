//! Grand livre auxiliaire, balance auxiliaire et relevé de compte d'un tiers (ADR 0033,
//! point 5). Mêmes filtres que les éditions de L2 ([`Filtre`] : période, comptes, journaux,
//! à-nouveaux en ouverture) ; regroupement par compte collectif puis par tiers résolu, comme
//! Tryton (`account.general_ledger.account.party`, `account.py:2306-2346` : lignes par compte
//! et tiers, triées par date) et OCA (`general_ledger.py:236-260`, regroupement par tiers).

use std::collections::BTreeMap;

use rust_decimal::Decimal;
use time::Date;

use super::{
    ErreurEditionTiers, Lettrages, LettreEdition, LigneTiers, SourceLigne, controler_doublons,
    somme,
};
use crate::compte::NumeroCompte;
use crate::echeance::Residuel;
use crate::ecriture::{IdTiers, Sens};
use crate::editions::{ErreurEdition, Filtre, FormatBalance, LigneBalance, Mode, totaliser};
use crate::journal::CodeJournal;
use crate::numerotation::NumeroEcriture;
use crate::texte::TexteCourt;

/// Ligne du grand livre auxiliaire ou du relevé de compte.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LigneAuxiliaire {
    source: SourceLigne,
    date: Date,
    journal: CodeJournal,
    date_valeur_comptable: Option<Date>,
    libelle: TexteCourt,
    debit: Decimal,
    credit: Decimal,
    solde: Decimal,
    echeance: Date,
    residuel: Residuel,
    lettres: Vec<LettreEdition>,
}

impl LigneAuxiliaire {
    /// Provenance (ligne validée avec son numéro, ou brouillard).
    #[must_use]
    pub fn source(&self) -> &SourceLigne {
        &self.source
    }

    /// Numéro définitif ; `None` pour le brouillard.
    #[must_use]
    pub fn numero(&self) -> Option<&NumeroEcriture> {
        match &self.source {
            SourceLigne::Validee { numero, .. } => Some(numero),
            SourceLigne::Brouillard { .. } => None,
        }
    }

    /// Date comptable.
    #[must_use]
    pub fn date(&self) -> Date {
        self.date
    }

    /// Journal.
    #[must_use]
    pub fn journal(&self) -> &CodeJournal {
        &self.journal
    }

    /// Date de valeur comptable d'une opération tardive, mentionnée distinctement (AUDCIF
    /// art. 22 4°).
    #[must_use]
    pub fn date_valeur_comptable(&self) -> Option<Date> {
        self.date_valeur_comptable
    }

    /// Libellé de l'écriture.
    #[must_use]
    pub fn libelle(&self) -> &TexteCourt {
        &self.libelle
    }

    /// Débit.
    #[must_use]
    pub fn debit(&self) -> Decimal {
        self.debit
    }

    /// Crédit.
    #[must_use]
    pub fn credit(&self) -> Decimal {
        self.credit
    }

    /// Solde cumulé signé (débit − crédit), ouverture comprise ; pour un relevé des lignes
    /// ouvertes, cumul des résiduels signés.
    #[must_use]
    pub fn solde(&self) -> Decimal {
        self.solde
    }

    /// Échéance effective à la date de l'édition.
    #[must_use]
    pub fn echeance(&self) -> Date {
        self.echeance
    }

    /// Résiduel à la date de l'édition (montant et part non lettrée, statut calculé).
    #[must_use]
    pub fn residuel(&self) -> &Residuel {
        &self.residuel
    }

    /// Lettres à la date de l'édition (lettrages postérieurs marqués).
    #[must_use]
    pub fn lettres(&self) -> &[LettreEdition] {
        &self.lettres
    }
}

/// Bloc d'un tiers sur un compte collectif : ouverture, lignes, totaux, solde final.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompteTiers {
    compte: NumeroCompte,
    tiers: IdTiers,
    solde_ouverture: Decimal,
    lignes: Vec<LigneAuxiliaire>,
    total_debit: Decimal,
    total_credit: Decimal,
    solde_final: Decimal,
}

impl CompteTiers {
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

    /// Solde d'ouverture signé (débit − crédit) ; nul pour un relevé des lignes ouvertes.
    #[must_use]
    pub fn solde_ouverture(&self) -> Decimal {
        self.solde_ouverture
    }

    /// Lignes, par date puis numéro (brouillard en fin de journée).
    #[must_use]
    pub fn lignes(&self) -> &[LigneAuxiliaire] {
        &self.lignes
    }

    /// Total des débits des lignes ; pour un relevé des lignes ouvertes, des résiduels au débit.
    #[must_use]
    pub fn total_debit(&self) -> Decimal {
        self.total_debit
    }

    /// Total des crédits des lignes ; pour un relevé des lignes ouvertes, des résiduels au
    /// crédit.
    #[must_use]
    pub fn total_credit(&self) -> Decimal {
        self.total_credit
    }

    /// Solde final signé (débit − crédit).
    #[must_use]
    pub fn solde_final(&self) -> Decimal {
        self.solde_final
    }
}

/// Ce que montre un relevé de compte.
#[derive(Debug, Clone, Copy)]
pub enum Releve<'a> {
    /// Toutes les lignes de la période du filtre, avec le solde d'ouverture (relevé d'activité,
    /// OCA `partner_statement/report/activity_statement.py:54-101`).
    Toutes(&'a Filtre),
    /// Lignes ouvertes à la date (résiduel non nul), quelle que soit leur date comptable
    /// antérieure (relevé des encours, OCA `report_statement_common.py:61-120`).
    Ouvertes(Date),
}

type Cle = (NumeroCompte, IdTiers);

/// Construit une ligne d'édition à la date `date` ; `solde` est complété par l'appelant.
fn ligne_edition(
    ligne: &LigneTiers,
    lettrages: &Lettrages,
    date: Date,
    residuel: Residuel,
) -> Result<LigneAuxiliaire, ErreurEditionTiers> {
    Ok(LigneAuxiliaire {
        source: ligne.source.clone(),
        date: ligne.date,
        journal: ligne.journal.clone(),
        date_valeur_comptable: ligne.date_valeur_comptable,
        libelle: ligne.libelle.clone(),
        debit: ligne.debit(),
        credit: ligne.credit(),
        solde: Decimal::ZERO,
        echeance: ligne.echeance.effective(date),
        residuel,
        lettres: lettrages.lettres_a(ligne, date)?,
    })
}

/// Grand livre auxiliaire : un bloc par (compte collectif, tiers résolu) mouvementé, trié par
/// compte puis tiers ; solde cumulé, échéance effective, résiduel et lettres à la date de fin du
/// filtre.
///
/// `lignes` : lignes de tiers d'**un seul dossier**, chacune une fois, dans un ordre
/// quelconque ; comme pour [`crate::editions::grand_livre`], l'API passe celles de l'exercice du
/// filtre jusqu'à sa date de fin.
///
/// # Errors
/// Ligne en double, lettrage incohérent, dépassement de capacité.
pub fn grand_livre_auxiliaire(
    lignes: &[LigneTiers],
    lettrages: &Lettrages,
    mode: Mode,
    filtre: &Filtre,
) -> Result<Vec<CompteTiers>, ErreurEditionTiers> {
    controler_doublons(lignes)?;
    grand_livre_filtre(lignes.iter(), lettrages, mode, filtre)
}

fn grand_livre_filtre<'a>(
    lignes: impl Iterator<Item = &'a LigneTiers>,
    lettrages: &Lettrages,
    mode: Mode,
    filtre: &Filtre,
) -> Result<Vec<CompteTiers>, ErreurEditionTiers> {
    let (_, au) = filtre.bornes();
    let mut blocs: BTreeMap<Cle, (Decimal, Vec<&LigneTiers>)> = BTreeMap::new();
    for ligne in lignes {
        if !ligne.retenue(mode)
            || !filtre.retient_journal(&ligne.journal)
            || !filtre.retient_compte(&ligne.compte)
        {
            continue;
        }
        let ouverture = filtre.en_ouverture(&ligne.journal, ligne.date);
        if !(ouverture || filtre.en_periode(&ligne.journal, ligne.date)) {
            continue;
        }
        let bloc = blocs
            .entry((ligne.compte.clone(), ligne.tiers.clone()))
            .or_insert((Decimal::ZERO, Vec::new()));
        if ouverture {
            bloc.0 = somme(bloc.0, ligne.signer(ligne.montant))?;
        } else {
            bloc.1.push(ligne);
        }
    }
    let mut resultat = Vec::with_capacity(blocs.len());
    for ((compte, tiers), (ouverture, mut retenues)) in blocs {
        retenues.sort_by(|a, b| a.cle_tri().cmp(&b.cle_tri()));
        let (mut solde, mut total_debit, mut total_credit) =
            (ouverture, Decimal::ZERO, Decimal::ZERO);
        let mut lignes_bloc = Vec::with_capacity(retenues.len());
        for ligne in retenues {
            let residuel = lettrages.residuel_a(ligne, au)?;
            let mut edition = ligne_edition(ligne, lettrages, au, residuel)?;
            solde = somme(solde, ligne.signer(ligne.montant))?;
            total_debit = somme(total_debit, edition.debit)?;
            total_credit = somme(total_credit, edition.credit)?;
            edition.solde = solde;
            lignes_bloc.push(edition);
        }
        resultat.push(CompteTiers {
            compte,
            tiers,
            solde_ouverture: ouverture,
            lignes: lignes_bloc,
            total_debit,
            total_credit,
            solde_final: solde,
        });
    }
    Ok(resultat)
}

/// Relevé de compte d'un tiers (ADR 0033, point 5) : un bloc par compte collectif du tiers,
/// trié par compte ; lignes avec lettres, échéance effective et résiduel, solde cumulé.
///
/// - [`Releve::Toutes`] : grand livre auxiliaire restreint au tiers.
/// - [`Releve::Ouvertes`] : lignes de date comptable au plus tard `D` dont le résiduel à `D`
///   n'est pas nul ; le solde cumule les résiduels signés, et le solde final de chaque bloc est
///   le solde ouvert du tiers sur ce compte (égal à celui de la balance âgée).
///
/// # Errors
/// Ligne en double, lettrage incohérent, dépassement de capacité.
pub fn releve_tiers(
    lignes: &[LigneTiers],
    lettrages: &Lettrages,
    mode: Mode,
    tiers: &IdTiers,
    releve: Releve<'_>,
) -> Result<Vec<CompteTiers>, ErreurEditionTiers> {
    controler_doublons(lignes)?;
    let du_tiers = lignes.iter().filter(|l| &l.tiers == tiers);
    let date = match releve {
        Releve::Toutes(filtre) => return grand_livre_filtre(du_tiers, lettrages, mode, filtre),
        Releve::Ouvertes(date) => date,
    };
    let mut blocs: BTreeMap<NumeroCompte, Vec<(&LigneTiers, Residuel)>> = BTreeMap::new();
    for ligne in du_tiers {
        if !ligne.retenue(mode) || ligne.date > date {
            continue;
        }
        let residuel = lettrages.residuel_a(ligne, date)?;
        if !residuel.residuel().is_zero() {
            blocs
                .entry(ligne.compte.clone())
                .or_default()
                .push((ligne, residuel));
        }
    }
    let mut resultat = Vec::with_capacity(blocs.len());
    for (compte, mut ouvertes) in blocs {
        ouvertes.sort_by(|a, b| a.0.cle_tri().cmp(&b.0.cle_tri()));
        let (mut solde, mut total_debit, mut total_credit) =
            (Decimal::ZERO, Decimal::ZERO, Decimal::ZERO);
        let mut lignes_bloc = Vec::with_capacity(ouvertes.len());
        for (ligne, residuel) in ouvertes {
            let reste = residuel.residuel();
            let mut edition = ligne_edition(ligne, lettrages, date, residuel)?;
            solde = somme(solde, ligne.signer(reste))?;
            match ligne.sens {
                Sens::Debit => total_debit = somme(total_debit, reste)?,
                Sens::Credit => total_credit = somme(total_credit, reste)?,
            }
            edition.solde = solde;
            lignes_bloc.push(edition);
        }
        resultat.push(CompteTiers {
            compte,
            tiers: tiers.clone(),
            solde_ouverture: Decimal::ZERO,
            lignes: lignes_bloc,
            total_debit,
            total_credit,
            solde_final: solde,
        });
    }
    Ok(resultat)
}

/// Ligne de la balance auxiliaire : un tiers sur un compte collectif.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LigneBalanceAuxiliaire {
    compte: NumeroCompte,
    tiers: IdTiers,
    balance: LigneBalance,
}

impl LigneBalanceAuxiliaire {
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

    /// Report (ouverture), mouvements et soldes, aux formats de la balance générale
    /// ([`LigneBalance::colonnes`]) ; la clé est l'identifiant du tiers.
    #[must_use]
    pub fn balance(&self) -> &LigneBalance {
        &self.balance
    }
}

/// Balance auxiliaire : report, mouvements et solde par tiers, triée par compte puis tiers.
/// Chaque solde est celui d'un tiers : débiteurs et créditeurs ne se compensent pas dans les
/// totaux (AUDCIF fiches 40 et 41, p. PDF 382 et 386).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BalanceAuxiliaire {
    lignes: Vec<LigneBalanceAuxiliaire>,
}

impl BalanceAuxiliaire {
    /// Lignes, triées par compte puis tiers.
    #[must_use]
    pub fn lignes(&self) -> &[LigneBalanceAuxiliaire] {
        &self.lignes
    }

    /// Totaux de chaque colonne, tous comptes.
    ///
    /// # Errors
    /// Dépassement de capacité.
    pub fn totaux(&self, format: FormatBalance) -> Result<Vec<Decimal>, ErreurEdition> {
        totaliser(self.lignes.iter().map(|l| &l.balance), format)
    }

    /// Totaux de chaque colonne pour un compte collectif.
    ///
    /// # Errors
    /// Dépassement de capacité.
    pub fn totaux_compte(
        &self,
        compte: &NumeroCompte,
        format: FormatBalance,
    ) -> Result<Vec<Decimal>, ErreurEdition> {
        totaliser(
            self.lignes
                .iter()
                .filter(|l| &l.compte == compte)
                .map(|l| &l.balance),
            format,
        )
    }
}

/// Balance auxiliaire (ADR 0033, point 5) : mêmes règles d'ouverture et de période que
/// [`crate::editions::Balance::calculer`].
///
/// # Errors
/// Ligne en double, dépassement de capacité.
pub fn balance_auxiliaire(
    lignes: &[LigneTiers],
    mode: Mode,
    filtre: &Filtre,
) -> Result<BalanceAuxiliaire, ErreurEditionTiers> {
    controler_doublons(lignes)?;
    // (ouverture débit, ouverture crédit, débit, crédit)
    let mut cumuls: BTreeMap<Cle, [Decimal; 4]> = BTreeMap::new();
    for ligne in lignes {
        if !ligne.retenue(mode)
            || !filtre.retient_journal(&ligne.journal)
            || !filtre.retient_compte(&ligne.compte)
        {
            continue;
        }
        let ouverture = filtre.en_ouverture(&ligne.journal, ligne.date);
        if !(ouverture || filtre.en_periode(&ligne.journal, ligne.date)) {
            continue;
        }
        let cumul = cumuls
            .entry((ligne.compte.clone(), ligne.tiers.clone()))
            .or_insert([Decimal::ZERO; 4]);
        let (i_debit, i_credit) = if ouverture { (0, 1) } else { (2, 3) };
        for (indice, valeur) in [(i_debit, ligne.debit()), (i_credit, ligne.credit())] {
            if let Some(total) = cumul.get_mut(indice) {
                *total = somme(*total, valeur)?;
            }
        }
    }
    let lignes = cumuls
        .into_iter()
        .map(|((compte, tiers), [od, oc, d, c])| LigneBalanceAuxiliaire {
            balance: LigneBalance::nouvelle(tiers.as_str().to_owned(), (od, oc), (d, c)),
            compte,
            tiers,
        })
        .collect();
    Ok(BalanceAuxiliaire { lignes })
}
