//! Serveur applicatif de Sigma Books : bibliothèque du binaire `api`.
//!
//! - [`base`] : accès à PostgreSQL sous le rôle applicatif, transaction au contexte de tenant
//!   (ADR 0011) et fonctions d'accès qui filtrent elles aussi par tenant ;
//! - [`horodatage`] : client des autorités d'horodatage RFC 3161 (ADR 0010) ;
//! - [`serveur`] : serveur HTTP, pipeline authentification → contexte → Cedar → transaction →
//!   cas d'usage, en-têtes, anti-CSRF, erreurs uniformes, journaux en liste blanche, limites.

#![forbid(unsafe_code)]

pub mod base;
pub mod horodatage;
pub mod serveur;
