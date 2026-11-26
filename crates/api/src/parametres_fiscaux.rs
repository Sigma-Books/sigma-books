//! Paramètres fiscaux d'un dossier (ADR 0030) : la version applicable à une date, celle qu'une
//! écriture a enregistrée.
//!
//! Le registre ([`fiscal::Registre`]) est porté par l'application ; le pays vient du contexte du
//! dossier (ADR 0027). Aucun pays n'est nommé ici.

use fiscal::{ParametresFiscaux, Registre, VersionsDuDossier};
use time::Date;

use crate::base::ErreurBase;

/// Versions de paramètres fiscaux que voit un dossier : le paquet de son pays et ses versions
/// propres.
#[derive(Debug, Clone)]
pub struct ParametresDuDossier<'a> {
    versions: VersionsDuDossier<'a>,
}

impl<'a> ParametresDuDossier<'a> {
    /// Versions d'un dossier du pays `pays` (ISO 3166-1 alpha-2, majuscules, tel que stocké),
    /// avec ses versions propres (`propres`).
    #[must_use]
    pub fn nouveau(registre: &'a Registre, pays: &str, propres: &'a [ParametresFiscaux]) -> Self {
        Self {
            versions: registre.versions_du_dossier(pays, propres),
        }
    }

    /// Version qui s'applique à une date comptable, s'il y en a une.
    #[must_use]
    pub fn applicable(&self, jour: Date) -> Option<&'a ParametresFiscaux> {
        self.versions.applicable(jour)
    }

    /// Version **enregistrée** par une écriture : celle de son identifiant ; sans identifiant
    /// (écriture sans version), celle qui s'applique à sa date.
    ///
    /// # Errors
    /// [`ErreurBase::Donnee`] si l'identifiant enregistré ne désigne aucune version connue.
    pub fn enregistree(
        &self,
        identifiant: Option<&str>,
        jour: Date,
    ) -> Result<Option<&'a ParametresFiscaux>, ErreurBase> {
        match identifiant {
            Some(id) => self
                .versions
                .par_identifiant(id)
                .map(Some)
                .ok_or(ErreurBase::Donnee("version de paramètres fiscaux inconnue")),
            None => Ok(self.applicable(jour)),
        }
    }
}
