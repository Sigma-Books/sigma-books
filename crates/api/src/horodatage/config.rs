//! Configuration du client d'horodatage, lue dans l'environnement par
//! [`crate::serveur::config::Configuration`].
//!
//! | Variable | Obligatoire | Contenu |
//! |---|---|---|
//! | `SIGMA_AUTORITES_HORODATAGE` | non | Autorités RFC 3161, séparées par des virgules, dans l'ordre d'essai (`http://` seul) ; par défaut DigiCert puis Sectigo (ADR 0010) |

use super::Autorite;
use crate::serveur::config::{Environnement, ErreurConfiguration};

/// Autorités d'horodatage, dans l'ordre d'essai : les seules destinations du client
/// (ADR 0010). La valeur n'est pas recopiée dans l'erreur (le chemin peut porter une clé).
///
/// # Errors
/// [`ErreurConfiguration::Invalide`] si la liste est refusée ([`Autorite::liste`]).
pub fn autorites(env: &Environnement<'_>) -> Result<Vec<Autorite>, ErreurConfiguration> {
    match env.facultative("SIGMA_AUTORITES_HORODATAGE") {
        Some(liste) => Autorite::liste(&liste)
            .map_err(|_| ErreurConfiguration::Invalide("SIGMA_AUTORITES_HORODATAGE")),
        None => Ok(Autorite::par_defaut()),
    }
}
