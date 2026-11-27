//! Lecture et écriture des formats d'échange. Toute entrée est hostile.
//!
//! - [`releve`] : relevés bancaires et mobile money au format CSV, lus par un profil (ADR 0034
//!   §6) ; analyseur pur sur `&[u8]`, limites appliquées pendant la lecture, empreintes de
//!   fichier et de ligne (ADR 0034 §7).
//! - [`motif`] : expression régulière en temps linéaire et de taille bornée, pour les règles de
//!   suggestion de relevé (ADR 0034 §8).

#![forbid(unsafe_code)]

pub mod motif;
pub mod releve;
