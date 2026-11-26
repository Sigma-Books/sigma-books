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
    /// Plancher de rétention des pièces du socle (OHADA), à défaut de version applicable.
    plancher_retention: u16,
}

impl<'a> ParametresDuDossier<'a> {
    /// Versions d'un dossier du pays `pays` (ISO 3166-1 alpha-2, majuscules, tel que stocké),
    /// avec ses versions propres (`propres`).
    #[must_use]
    pub fn nouveau(registre: &'a Registre, pays: &str, propres: &'a [ParametresFiscaux]) -> Self {
        Self {
            versions: registre.versions_du_dossier(pays, propres),
            plancher_retention: registre.socle().plancher_retention(),
        }
    }

    /// Années de rétention des pièces déposées ce jour-là : celles de la version applicable, à
    /// défaut le plancher du socle.
    #[must_use]
    pub fn retention_des_pieces(&self, jour: Date) -> u16 {
        self.applicable(jour)
            .map_or(self.plancher_retention, |v| v.retention().annees_min)
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

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::date;

    #[test]
    fn le_pays_du_dossier_choisit_le_paquet() {
        let registre = Registre::officiel().unwrap();
        let sn = ParametresDuDossier::nouveau(&registre, "SN", &[]);
        assert_eq!(
            sn.applicable(date!(2026 - 03 - 15))
                .map(ParametresFiscaux::identifiant),
            Some("sn-2025.1")
        );
        // Avant la première version, ou pour un pays sans paquet : aucune version.
        assert!(sn.applicable(date!(2024 - 12 - 31)).is_none());
        let autre = ParametresDuDossier::nouveau(&registre, "CM", &[]);
        assert!(autre.applicable(date!(2026 - 03 - 15)).is_none());
    }

    #[test]
    fn la_version_enregistree_prime_sur_la_date() {
        let registre = Registre::officiel().unwrap();
        let sn = ParametresDuDossier::nouveau(&registre, "SN", &[]);
        let enregistree = sn
            .enregistree(Some("sn-2025.1"), date!(2030 - 01 - 01))
            .unwrap();
        assert_eq!(
            enregistree.map(ParametresFiscaux::identifiant),
            Some("sn-2025.1")
        );
        // Sans identifiant : celle de la date. Identifiant inconnu : donnée incohérente.
        assert!(
            sn.enregistree(None, date!(2026 - 01 - 01))
                .unwrap()
                .is_some()
        );
        assert!(
            sn.enregistree(None, date!(2020 - 01 - 01))
                .unwrap()
                .is_none()
        );
        assert!(
            sn.enregistree(Some("sn-1999.1"), date!(2026 - 01 - 01))
                .is_err()
        );
    }

    #[test]
    fn la_retention_suit_la_version_puis_le_plancher() {
        let registre = Registre::officiel().unwrap();
        let sn = ParametresDuDossier::nouveau(&registre, "SN", &[]);
        assert_eq!(sn.retention_des_pieces(date!(2026 - 03 - 15)), 10);
        // Pays sans paquet : plancher OHADA du socle.
        let autre = ParametresDuDossier::nouveau(&registre, "GA", &[]);
        assert_eq!(autre.retention_des_pieces(date!(2026 - 03 - 15)), 10);
    }
}
