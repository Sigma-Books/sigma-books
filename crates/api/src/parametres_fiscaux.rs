//! Paramètres fiscaux d'un dossier (ADR 0030) : la version applicable à une date, celle qu'une
//! écriture a enregistrée.
//!
//! Le registre ([`fiscal::Registre`]) est porté par l'application ; le pays vient du contexte du
//! dossier (ADR 0027). Aucun pays n'est nommé ici.

use fiscal::{ParametresFiscaux, Portee, Registre, VersionsDuDossier};
use integrite::Empreinte;
use time::Date;

use crate::base::parametres_fiscaux::{VersionPropre, lire_versions};
use crate::base::{ErreurBase, TransactionTenant};

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

/// Version propre rebâtie d'une ligne de la table, **contrôlée** : le contenu se relit par le
/// validateur commun, son identifiant, son pays, sa version de base et ses dates sont ceux des
/// colonnes, et son empreinte est celle du contenu (une ligne altérée est une donnée incohérente).
///
/// # Errors
/// [`ErreurBase::Donnee`] si la ligne n'est pas celle qu'a écrite le cas d'usage.
pub fn version_depuis_ligne(
    registre: &Registre,
    ligne: &VersionPropre,
) -> Result<ParametresFiscaux, ErreurBase> {
    let incoherent = || ErreurBase::VersionParametresIllisible;
    let numero = u32::try_from(ligne.numero).map_err(|_| incoherent())?;
    let p =
        ParametresFiscaux::depuis_texte(registre.socle(), Portee::Dossier(numero), &ligne.contenu)
            .map_err(|_| incoherent())?;
    let identifiant = format!(
        "{}-{}+d{numero}",
        ligne.pays.to_ascii_lowercase(),
        ligne.version
    );
    let empreinte = Empreinte::calculer(ligne.contenu.as_bytes());
    if p.pays() != ligne.pays
        || p.version() != ligne.version
        || p.identifiant() != identifiant
        || p.date_effet() != ligne.date_effet
        || p.date_fin() != ligne.date_fin
        || empreinte.octets().as_slice() != ligne.empreinte.as_slice()
    {
        return Err(incoherent());
    }
    Ok(p)
}

/// Versions propres du dossier du contexte, rebâties et contrôlées, par numéro croissant.
///
/// # Errors
/// [`ErreurBase::Donnee`] si une ligne est incohérente, erreur de PostgreSQL.
pub async fn lire_propres(
    tx: &mut TransactionTenant,
    registre: &Registre,
) -> Result<Vec<ParametresFiscaux>, ErreurBase> {
    lire_versions(tx)
        .await?
        .iter()
        .map(|l| version_depuis_ligne(registre, l))
        .collect()
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

    fn ligne_de_test(registre: &Registre) -> VersionPropre {
        let base = &registre.versions_du_pays("SN")[0];
        let taux = [("normal".to_owned(), "19".to_owned())];
        let d = base
            .deriver(
                registre.socle(),
                &fiscal::Surcharge {
                    numero: 1,
                    date_effet: date!(2027 - 01 - 01),
                    date_fin: None,
                    motif: "Convention",
                    taux: &taux,
                    regimes_retires: &[],
                },
            )
            .unwrap();
        let contenu = d.vers_texte();
        VersionPropre {
            id: uuid::Uuid::nil(),
            numero: 1,
            pays: "SN".to_owned(),
            version: "2025.1".to_owned(),
            date_effet: date!(2027 - 01 - 01),
            date_fin: None,
            empreinte: Empreinte::calculer(contenu.as_bytes()).octets().to_vec(),
            contenu,
            motif: "Convention".to_owned(),
            cree_par: uuid::Uuid::nil(),
            cree_le: time::OffsetDateTime::UNIX_EPOCH,
        }
    }

    #[test]
    fn une_ligne_intacte_se_relit_et_une_ligne_alteree_est_refusee() {
        let registre = Registre::officiel().unwrap();
        let ligne = ligne_de_test(&registre);
        let p = version_depuis_ligne(&registre, &ligne).unwrap();
        assert_eq!(p.identifiant(), "sn-2025.1+d1");
        let sn = ParametresDuDossier::nouveau(&registre, "SN", std::slice::from_ref(&p));
        assert_eq!(
            sn.applicable(date!(2027 - 06 - 01))
                .map(ParametresFiscaux::identifiant),
            Some("sn-2025.1+d1")
        );
        assert_eq!(
            sn.enregistree(Some("sn-2025.1+d1"), date!(2020 - 01 - 01))
                .unwrap()
                .map(ParametresFiscaux::identifiant),
            Some("sn-2025.1+d1")
        );
        // Contenu altéré (taux 19 -> 5) sans changer l'empreinte, ou colonnes qui divergent.
        let mut altere = ligne.clone();
        altere.contenu = altere.contenu.replace("valeur = \"19\"", "valeur = \"5\"");
        assert!(version_depuis_ligne(&registre, &altere).is_err());
        for modifie in [
            VersionPropre {
                pays: "CM".to_owned(),
                ..ligne.clone()
            },
            VersionPropre {
                version: "2025.2".to_owned(),
                ..ligne.clone()
            },
            VersionPropre {
                date_effet: date!(2027 - 02 - 01),
                ..ligne.clone()
            },
            VersionPropre {
                date_fin: Some(date!(2027 - 12 - 31)),
                ..ligne.clone()
            },
            VersionPropre {
                empreinte: vec![0; 32],
                ..ligne.clone()
            },
            VersionPropre {
                contenu: "pas du toml".to_owned(),
                ..ligne.clone()
            },
        ] {
            assert!(version_depuis_ligne(&registre, &modifie).is_err());
        }
    }
}
