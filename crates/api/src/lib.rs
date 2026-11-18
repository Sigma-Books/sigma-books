//! Serveur applicatif de Sigma Books : bibliothèque du binaire `api`.
//!
//! - [`base`] : accès à PostgreSQL sous le rôle applicatif, transaction au contexte de tenant
//!   (ADR 0011) et fonctions d'accès qui filtrent elles aussi par tenant.

#![forbid(unsafe_code)]

pub mod base;
