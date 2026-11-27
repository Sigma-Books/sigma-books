//! Paramètres fiscaux par pays, versionnés et sourcés (ADR 0030).
//!
//! - le **socle** (`data/socle/`) fixe, pour tous les pays, les codes de régime génériques et ce
//!   que `compta-core` en suppose ;
//! - un **paquet de pays** (`data/pays/<xx>/<version>/`) apporte les valeurs sourcées du pays :
//!   taux, articles et pages, droit à déduction, exigibilité, validité dans le temps, motifs
//!   d'exonération, rétention des pièces. Une version publiée est **immuable** ;
//! - une [`ParametresFiscaux`] est une version chargée et validée ; sa [`VueAuJour`]
//!   implémente [`compta_core::ReferentielFiscal`] à la date d'une écriture ;
//! - le [`Registre`] associe un pays à ses versions et choisit celle qui s'applique à une date,
//!   avec les versions propres à un dossier ([`VersionsDuDossier`]).
//!
//! `compta-core` ne connaît aucune de ces valeurs.

#![forbid(unsafe_code)]

mod date;
mod erreur;
mod officiel;
mod registre;
mod socle;
mod types;
mod version;

pub use erreur::ErreurFiscale;
pub use registre::{Registre, VersionsDuDossier};
pub use socle::{RegimeSocle, Socle};
pub use types::{Deduction, ExigibiliteLegale, Portee, Retention, RoleTaux, TauxNomme};
pub use version::{
    Chevauchement, Document, MOTIF_MAX_OCTETS, MotifExoneration, ParametresFiscaux, RegimeTva,
    Surcharge, VueAuJour, motif_sur,
};
