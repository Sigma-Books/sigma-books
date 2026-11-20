//! Paramètres fiscaux du Sénégal, versionnés et sourcés.
//!
//! Aujourd'hui : les régimes de TVA et les motifs d'exonération de l'étiquette fiscale d'une
//! ligne d'écriture (L2, question 7.2 ; ADR 0019). Les données vivent dans `data/*.toml`,
//! embarquées à la compilation et **validées à chaque chargement**. `compta-core` ne connaît
//! aucune de ces valeurs : [`ReferentielTva`] implémente son trait
//! [`compta_core::ReferentielFiscal`].

#![forbid(unsafe_code)]

mod tva;

pub use tva::{
    Chevauchement, Deduction, ErreurFiscale, ExigibiliteCgi, MotifExoneration, ReferentielTva,
    RegimeTva,
};
