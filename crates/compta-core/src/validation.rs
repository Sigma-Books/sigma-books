//! Validation et contre-passation **sans livre en mémoire**, à partir de faits fournis par
//! l'appelant (spec §6.2, §6.3 ; ADR 0007, ADR 0011 ; lot L2, PR 3).
//!
//! Au lot L2, les écritures validées sont en PostgreSQL : l'API ne charge pas tout le livre.
//! [`valider`] et [`contre_passer`] font exactement ce que font [`Livre::valider`] et
//! [`Livre::contre_passer`], qui leur délèguent ; le résultat est le même sur toute suite
//! d'opérations (propriété `tests/validation_sans_livre.rs`). Ces fonctions sont pures :
//! elles renvoient une [`Validation`] (l'écriture validée et le compteur à réécrire), que
//! l'appelant enregistre.
//!
//! # Preuve de validation
//! Une [`EcritureValidee`] peut venir d'une **relecture** de la base
//! ([`EcritureValidee::reconstituer`]) : la détenir ne prouve pas qu'elle a passé les
//! contrôles du §6.2. Seule une [`Validation`] le prouve : elle ne se construit que par
//! [`valider`] et [`contre_passer`]. La couche d'insertion (API, PR 13) n'accepte **que** ce
//! type pour inscrire une écriture validée ; une écriture reconstituée sert à la lecture
//! seulement (éditions, vérification de la chaîne, origine à contre-passer).
//!
//! # Faits que l'appelant fournit, et verrous à prendre
//! Tout est lu **dans la transaction de la validation** (`READ COMMITTED`), dans l'ordre de
//! verrouillage de l'ADR 0011 (période, tête de chaîne, compteur, tête d'audit) :
//! - **exercices et périodes** ([`Contexte::exercices`]) en lecture partagée (`FOR SHARE`) :
//!   un verrouillage ou une clôture concurrente, qui prend le verrou exclusif, attend la fin
//!   de la validation. Ce verrou n'est possible que parce que le rôle applicatif a le droit
//!   `UPDATE` sur ces tables (changement d'état des périodes) : PostgreSQL l'exige pour tout
//!   verrou de ligne ;
//! - **plan de comptes** ([`Contexte::plan`]) : au moins les comptes des lignes, avec leurs
//!   descendants (un compte qui en a est de regroupement) ; en pratique, le plan du dossier ;
//!   **journaux** ([`Contexte::journaux`]) ;
//! - **compteur** du journal pour l'exercice qui contient la date de l'écriture, lu
//!   `FOR UPDATE`. Il fait foi : aucun contrôle de désynchronisation ici (le [`Livre`] en a
//!   un, qui n'a pas de sens en base). La ligne du compteur doit exister avant la lecture
//!   (créée avec le journal ou l'exercice, ou `INSERT … ON CONFLICT DO NOTHING` puis
//!   `SELECT … FOR UPDATE`) : sinon deux transactions partiraient de zéro. Un index unique
//!   (journal, exercice, séquence) sert de filet (PR 13) ;
//! - **date de validation** : le jour lu par l'appelant (le moteur ne lit pas l'horloge) ;
//! - pour une contre-passation, l'**écriture d'origine**, relue par
//!   [`EcritureValidee::reconstituer`], lue après la prise des verrous. Une origine peut être
//!   contre-passée plusieurs fois (ADR 0021 point 4, révisé le 2026-11-25) : aucun état de
//!   contre-passation n'est un fait de la décision. La sérialisation de deux contre-passations
//!   concurrentes (numéros, chaîne) est celle du verrou `FOR UPDATE` de la tête de chaîne du
//!   dossier. **Aucun verrou de ligne n'est possible sur les écritures validées** : tous les
//!   verrous de ligne exigent le droit `UPDATE`, refusé au rôle applicatif (ADR 0011, ajout
//!   seul).
//!
//! # Moteurs de référence étudiés
//! - Tryton `modules/account/move.py` : `post` numérote à la validation par une séquence
//!   stricte, incrémentée sous verrou ; `cancel(reversal=True)` inverse débit et crédit et
//!   garde l'origine, et n'empêche pas d'annuler plusieurs fois la même pièce : suivi (ADR 0021).
//! - Odoo `sequence_mixin.py` : pas de compteur, numéro déduit du dernier enregistré, unicité
//!   par index unique, trous admis : écarté (spec §6.3, numérotation sans trou).
//! - Bigcapital `PublishManualJournal` : relit la pièce sous verrou de ligne, puis vérifie
//!   qu'elle n'est pas déjà publiée : verrouiller, puis vérifier, comme ici.
//! - Dolibarr : numéro `MAX + 1` sans verrou, doublons possibles sous concurrence : à éviter.

use time::Date;

use crate::ecriture::{EcritureBrouillon, EcritureEquilibree, Refus};
#[cfg(doc)]
use crate::livre::Livre;
use crate::livre::{Avertissement, Contexte, EcritureValidee, ErreurValidation, controler};
use crate::numerotation::{Compteur, NumeroEcriture};
use crate::texte::TexteCourt;

/// Résultat d'une validation ou d'une contre-passation : l'écriture validée et le compteur à
/// enregistrer à la place de l'ancien, dans la même transaction.
///
/// **Preuve de validation** : ce type ne se construit que par [`valider`] et
/// [`contre_passer`], après les contrôles du §6.2. La couche d'insertion (API, PR 13)
/// n'accepte que lui ; une [`EcritureValidee`] seule peut venir d'une relecture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Validation {
    ecriture: EcritureValidee,
    compteur: Compteur,
    avertissements: Vec<Avertissement>,
}

impl Validation {
    /// Écriture validée, à inscrire.
    #[must_use]
    pub fn ecriture(&self) -> &EcritureValidee {
        &self.ecriture
    }

    /// Compteur à enregistrer à la place de celui qui a été fourni.
    #[must_use]
    pub fn compteur(&self) -> &Compteur {
        &self.compteur
    }

    /// Avertissements de la validation (échéance de clôture dépassée, compte de trésorerie non
    /// mouvementé) : l'écriture est validée, l'utilisateur en est informé.
    #[must_use]
    pub fn avertissements(&self) -> &[Avertissement] {
        &self.avertissements
    }

    /// Récupère l'écriture et le compteur, au moment de les inscrire.
    #[must_use]
    pub fn decomposer(self) -> (EcritureValidee, Compteur) {
        (self.ecriture, self.compteur)
    }
}

/// Valide une écriture équilibrée à partir de faits fournis : contrôle les invariants du
/// §6.2 dans le `contexte`, attribue le numéro suivant du `compteur`.
///
/// # Errors
/// Invariant non respecté ou compteur d'un autre journal ou d'un autre exercice que
/// l'écriture ([`ErreurValidation::CompteurInadapte`]). L'écriture est rendue intacte avec le
/// motif.
pub fn valider(
    ecriture: EcritureEquilibree,
    contexte: &Contexte<'_>,
    compteur: &Compteur,
    date_validation: Date,
) -> Result<Validation, Refus<EcritureEquilibree, ErreurValidation>> {
    let (ecriture, compteur, avertissements) =
        valider_avec(ecriture, None, contexte, compteur, date_validation, |_| {
            Ok(())
        })?;
    Ok(Validation {
        ecriture,
        compteur,
        avertissements,
    })
}

/// Contre-passe l'écriture `origine` à partir de faits fournis : écriture miroir dans le même
/// journal (sens inversés ; mêmes comptes, montants, devises, tiers, justification et version
/// de paramètres), à la `date` donnée, validée selon les mêmes contrôles. Une origine peut être
/// contre-passée plusieurs fois (ADR 0021 point 4). La contre-passation ne
/// reprend pas la date de valeur comptable de l'origine : c'est une opération nouvelle,
/// datée dans une période ouverte.
///
/// `origine` doit être relue de la base **du même dossier**, dans la transaction de la
/// contre-passation, après la prise des verrous (voir la documentation du module) : le moteur
/// ne peut pas vérifier qu'elle est à jour.
///
/// # Errors
/// Invariant non respecté à la date choisie (qui peut précéder celle de l'origine : aucun moteur
/// ne l'interdit), compteur inadapté.
pub fn contre_passer(
    origine: &EcritureValidee,
    date: Date,
    libelle: TexteCourt,
    contexte: &Contexte<'_>,
    compteur: &Compteur,
    date_validation: Date,
) -> Result<Validation, ErreurValidation> {
    let miroir = miroir(origine, date, libelle)?;
    let (ecriture, compteur, avertissements) = valider_avec(
        miroir,
        Some(origine.numero().clone()),
        contexte,
        compteur,
        date_validation,
        |_| Ok(()),
    )
    .map_err(|refus| refus.decomposer().1)?;
    Ok(Validation {
        ecriture,
        compteur,
        avertissements,
    })
}

/// Écriture miroir de `origine`. La date n'est pas bornée par celle de l'origine : Tryton
/// (`move.py:399-445`, `_cancel_default`) et Odoo (`account_move_reversal.py:17` et `93-100`,
/// date de contre-passation libre) ne le font pas.
pub(crate) fn miroir(
    origine: &EcritureValidee,
    date: Date,
    libelle: TexteCourt,
) -> Result<EcritureEquilibree, ErreurValidation> {
    let mut miroir = EcritureBrouillon::nouvelle(origine.journal().clone(), date, libelle);
    for ligne in origine.lignes() {
        miroir.ajouter_ligne(ligne.miroir());
    }
    miroir.justifier(origine.justification().clone());
    if let Some(version) = origine.version_parametres() {
        miroir.utiliser_parametres(version.clone());
    }
    // Le miroir d'une écriture équilibrée l'est aussi : l'échec est impossible, mais il est
    // traité comme une erreur plutôt que par un `expect`.
    miroir
        .valider_equilibre()
        .map_err(|refus| ErreurValidation::Equilibre(refus.decomposer().1))
}

/// Validation commune : contrôles du §6.2, compteur adapté à l'écriture, puis
/// `controle_compteur` (le [`Livre`] y vérifie la synchronisation de ses séquences), puis
/// numéro suivant.
pub(crate) fn valider_avec(
    ecriture: EcritureEquilibree,
    origine: Option<NumeroEcriture>,
    contexte: &Contexte<'_>,
    compteur: &Compteur,
    date_validation: Date,
    controle_compteur: impl FnOnce(&Compteur) -> Result<(), ErreurValidation>,
) -> Result<
    (EcritureValidee, Compteur, Vec<Avertissement>),
    Refus<EcritureEquilibree, ErreurValidation>,
> {
    let numeroter = || {
        let (exercice, avertissements) = controler(&ecriture, contexte, date_validation)?;
        let justification = ecriture
            .justification()
            .cloned()
            .ok_or(ErreurValidation::JustificationManquante)?;
        if compteur.journal() != ecriture.journal() || compteur.exercice() != &exercice {
            return Err(ErreurValidation::CompteurInadapte {
                compteur_journal: compteur.journal().clone(),
                compteur_exercice: compteur.exercice().clone(),
                journal: ecriture.journal().clone(),
                exercice,
            });
        }
        controle_compteur(compteur)?;
        let (numero, suivant) = compteur.suivant()?;
        Ok((numero, justification, suivant, avertissements))
    };
    match numeroter() {
        Ok((numero, justification, suivant, avertissements)) => Ok((
            EcritureValidee::assembler(ecriture, numero, justification, date_validation, origine),
            suivant,
            avertissements,
        )),
        Err(erreur) => Err(Refus::nouveau(ecriture, erreur)),
    }
}
