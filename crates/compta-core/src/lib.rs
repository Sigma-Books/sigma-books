//! Moteur comptable SYSCOHADA : types, invariants et règles de la partie double.
//!
//! Crate **pure** : aucune I/O, aucune base, aucun réseau, aucune horloge, aucun aléa.
//! Les dates, les identifiants et le référentiel des comptes sont passés en paramètre.

#![forbid(unsafe_code)]

pub mod compte;
pub mod ecriture;
pub mod editions;
pub mod exercice;
pub mod journal;
pub mod livre;
pub mod montant;
pub mod numerotation;
pub mod plan;
pub mod texte;

pub use compte::{ErreurNumeroCompte, NumeroCompte};
pub use ecriture::{
    EcritureBrouillon, EcritureEquilibree, ErreurEcriture, IdTiers, Justification, Ligne,
    MontantDevise, ReferencePiece, Refus, Sens, VersionParametres,
};
pub use editions::{
    Balance, CompteGrandLivre, EcritureJournal, EditionJournal, ErreurEdition, Filtre,
    FormatBalance, LigneBalance, LigneGrandLivre, Mode, Regroupement, grand_livre, journaux,
};
pub use exercice::{
    CodeExercice, Deverrouillage, ErreurExercice, Exercice, Exercices, FinPremierExercice, Periode,
    StatutExercice, StatutPeriode,
};
pub use journal::{
    CodeJournal, ErreurJournal, Journal, Journaux, ModeleJournal, TypeJournal, journaux_par_defaut,
};
pub use livre::{Contexte, EcritureValidee, ErreurValidation, Livre};
pub use montant::{Cours, Devise, ErreurMontant, Montant};
pub use numerotation::{Compteur, ErreurNumerotation, NumeroEcriture};
pub use plan::{Compte, CompteReference, ErreurPlan, GenreCompte, PlanComptes, SensNormal, Usage};
pub use texte::{ErreurTexte, TexteCourt};
