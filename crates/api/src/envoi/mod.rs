//! Envoi de courriels (L2-19, PR 9b ; ADR 0012, question 8a ; ADR 0024) : invitations et liens de
//! réinitialisation d'accès. Le courriel ne transporte **que** ce qu'il faut pour agir (lien à
//! usage unique) ; aucun montant, aucun nom de tiers, aucun mot de passe (F12.2).
//!
//! - [`config`] : relais SMTP, TLS obligatoire, mot de passe lu dans un fichier de droits restreints ;
//! - [`modele`] : modèles constants (sujet constant, adresse et nom validés) ;
//! - [`transport`] : interface et relais SMTP (erreurs réduites à des codes stables) ;
//! - [`tache`] : travailleur de la file, qui émet le lien au moment de l'envoi.

pub mod config;
pub mod modele;
pub mod tache;
pub mod transport;
