//! Configuration de l'accès à PostgreSQL, lue dans l'environnement par
//! [`crate::serveur::config::Configuration`]. Les URL contiennent un mot de passe : elles ne
//! sont jamais affichées (`Debug` masqué) ni recopiées dans un message d'erreur.
//!
//! | Variable | Obligatoire | Contenu |
//! |---|---|---|
//! | `SIGMA_URL_BASE_APPLICATION` | oui | URL PostgreSQL du **rôle applicatif** (`sigma_application`) |
//! | `SIGMA_URL_BASE_LECTURE_AUDIT` | oui | URL PostgreSQL du **rôle de lecture d'audit** (`sigma_lecture_audit`, ADR 0014) |

use std::fmt;

use crate::serveur::config::{Environnement, ErreurConfiguration};

/// URL des deux pools de la base.
#[derive(Clone, PartialEq, Eq)]
pub struct ConfigurationBase {
    /// URL de la base, rôle applicatif (secrète : jamais affichée).
    pub url_application: String,
    /// URL de la base, rôle de lecture d'audit (secrète : jamais affichée).
    pub url_lecture_audit: String,
}

impl fmt::Debug for ConfigurationBase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ConfigurationBase")
            .field("url_application", &"<masquée>")
            .field("url_lecture_audit", &"<masquée>")
            .finish()
    }
}

impl ConfigurationBase {
    /// Lit les variables du module.
    ///
    /// # Errors
    /// [`ErreurConfiguration`] si une URL manque ou n'est pas une URL PostgreSQL.
    pub fn depuis(env: &Environnement<'_>) -> Result<Self, ErreurConfiguration> {
        Ok(Self {
            url_application: url_postgres(env, "SIGMA_URL_BASE_APPLICATION")?,
            url_lecture_audit: url_postgres(env, "SIGMA_URL_BASE_LECTURE_AUDIT")?,
        })
    }
}

fn url_postgres(env: &Environnement<'_>, nom: &'static str) -> Result<String, ErreurConfiguration> {
    let url = env.obligatoire(nom)?;
    if url.starts_with("postgres://") || url.starts_with("postgresql://") {
        Ok(url)
    } else {
        Err(ErreurConfiguration::Invalide(nom))
    }
}
