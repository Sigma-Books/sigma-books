//! Moteur comptable SYSCOHADA : types, invariants et règles de la partie double.
//!
//! Crate **pure** : aucune I/O, aucune base, aucun réseau, aucune horloge, aucun aléa.
//! Les dates, les identifiants et le référentiel des comptes sont passés en paramètre.

#![forbid(unsafe_code)]

pub mod alertes;
pub mod centralisateur;
pub mod compte;
pub mod ecriture;
pub mod editions;
pub mod etiquette;
pub mod exercice;
pub mod journal;
pub mod lettrage;
pub mod livre;
pub mod montant;
pub mod numerotation;
pub mod plan;
pub mod texte;
pub mod validation;

pub use alertes::{AlerteBrouillard, RetardValidation, brouillard_en_retard, retard_de_validation};
pub use centralisateur::{
    Centralisateur, LigneCentralisateur, LigneCentralisateurCompte, MoisCivil,
    RubriqueCentralisateur, TotalMois,
};
pub use compte::{ErreurNumeroCompte, NumeroCompte};
pub use ecriture::{
    EcritureBrouillon, EcritureEquilibree, ErreurEcriture, IdTiers, Justification, Ligne,
    MontantDevise, ReferencePiece, Refus, Sens, VersionParametres,
};
pub use editions::{
    Balance, CompteGrandLivre, EcritureJournal, EditionJournal, ErreurEdition, Filtre,
    FormatBalance, LigneBalance, LigneGrandLivre, Mode, Regroupement, grand_livre, journaux,
};
pub use etiquette::{
    CodeMotif, CodeRegime, EcartTaxe, ErreurEtiquette, EtiquetteFiscale, EtiquetteRelue,
    Exigibilite, NatureEtiquette, ProfilRegime, ReferentielFiscal, SensTva, TauxTva, TotauxTaxe,
    controler_taxes_et_comptes, totaux_taxe,
};
pub use exercice::{
    CodeExercice, DELAI_CLOTURE_PAR_DEFAUT, DelaiCloture, Deverrouillage, EcheanceCloture,
    ErreurExercice, ErreurReconstitutionExercices, Exercice, Exercices, FinPremierExercice,
    LigneExercice, LignePeriode, Periode, StatutExercice, StatutPeriode, echeance_de_cloture,
};
pub use journal::{
    CodeJournal, ErreurJournal, Journal, Journaux, ModeleJournal, TypeJournal, journaux_par_defaut,
};
pub use lettrage::{
    BilanGroupe, Bornes, ErreurLettrage, IdLigne, Lettrage, Lettre, Lien, LigneLettrable,
    MotifRupture, Origine, Proposition, Propositions, RegistreLettrage, Regle, Rupture,
};
pub use livre::{
    Avertissement, Contexte, EcritureValidee, ErreurReconstitution, ErreurValidation, Livre,
};
pub use montant::{Cours, Devise, DeviseTenue, ErreurMontant, Montant};
pub use numerotation::{Compteur, ErreurNumerotation, NumeroEcriture};
pub use plan::{Compte, CompteReference, ErreurPlan, GenreCompte, PlanComptes, SensNormal, Usage};
pub use texte::{ErreurTexte, TexteCourt};
pub use validation::{Validation, contre_passer, valider};
