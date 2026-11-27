//! Registre pays → versions de paramètres, et sélection de la version applicable à une date.

use std::collections::BTreeMap;

use time::Date;

use crate::erreur::ErreurFiscale;
use crate::officiel::{PAQUETS, VersionEmbarquee};
use crate::socle::Socle;
use crate::types::Portee;
use crate::version::ParametresFiscaux;

/// Registre des paquets de pays, validé une fois (le serveur refuse de démarrer s'il est
/// invalide).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Registre {
    socle: Socle,
    /// Versions de chaque pays (clé : pays en majuscules), triées par date d'effet.
    pays: BTreeMap<String, Vec<ParametresFiscaux>>,
}

impl Registre {
    /// Registre des paquets embarqués.
    ///
    /// # Errors
    /// [`ErreurFiscale`] si le socle ou un paquet est invalide.
    pub fn officiel() -> Result<Self, ErreurFiscale> {
        Self::depuis_embarques(PAQUETS)
    }

    /// Charge des paquets embarqués : le répertoire (pays, version) et l'en-tête de chaque version
    /// doivent dire la même chose.
    pub(crate) fn depuis_embarques(
        paquets: &[(&str, &[VersionEmbarquee])],
    ) -> Result<Self, ErreurFiscale> {
        let socle = Socle::officiel()?;
        let mut versions = Vec::new();
        for (pays, liste) in paquets {
            for v in *liste {
                let texte = v
                    .fichiers
                    .iter()
                    .map(|(_, t)| *t)
                    .collect::<Vec<_>>()
                    .join("\n");
                let p = ParametresFiscaux::depuis_texte(&socle, Portee::Pack, &texte)?;
                if !p.pays().eq_ignore_ascii_case(pays) || p.version() != v.version {
                    return Err(ErreurFiscale::Version(format!("{pays}/{}", v.version)));
                }
                versions.push(p);
            }
        }
        Self::depuis_versions(socle, versions)
    }

    /// Registre de versions déjà chargées (paquets de test compris).
    ///
    /// # Errors
    /// [`ErreurFiscale`] : version de portée dossier (le registre ne contient que des paquets de
    /// pays), version ou date d'effet en double dans un pays, motif retiré d'une version à la
    /// suivante.
    pub fn depuis_versions(
        socle: Socle,
        versions: Vec<ParametresFiscaux>,
    ) -> Result<Self, ErreurFiscale> {
        let mut pays: BTreeMap<String, Vec<ParametresFiscaux>> = BTreeMap::new();
        for v in versions {
            if v.portee() != Portee::Pack {
                return Err(ErreurFiscale::PorteeInattendue(v.identifiant().to_owned()));
            }
            pays.entry(v.pays().to_owned()).or_default().push(v);
        }
        for (code, liste) in &mut pays {
            liste.sort_by_key(ParametresFiscaux::date_effet);
            for paire in liste.windows(2) {
                if paire[0].date_effet() == paire[1].date_effet() {
                    return Err(ErreurFiscale::DateEffetEnDouble { pays: code.clone() });
                }
                paire[0].verifier_continuite_avec(&paire[1])?;
            }
            let mut ids: Vec<&str> = liste.iter().map(ParametresFiscaux::identifiant).collect();
            ids.sort_unstable();
            if let Some(w) = ids.windows(2).find(|w| w[0] == w[1]) {
                return Err(ErreurFiscale::VersionEnDouble(w[0].to_owned()));
            }
        }
        Ok(Self { socle, pays })
    }

    /// Socle commun.
    #[must_use]
    pub fn socle(&self) -> &Socle {
        &self.socle
    }

    /// Pays qui ont au moins une version, en majuscules.
    pub fn pays(&self) -> impl Iterator<Item = &str> {
        self.pays.keys().map(String::as_str)
    }

    /// Versions du paquet d'un pays, par date d'effet croissante (vide si le pays n'en a pas).
    #[must_use]
    pub fn versions_du_pays(&self, pays: &str) -> &[ParametresFiscaux] {
        self.pays.get(pays).map_or(&[], Vec::as_slice)
    }

    /// Contrôle qu'une version **de dossier** peut s'ajouter à celles qu'il a déjà (`propres`) :
    /// portée dossier, pays qui a un paquet, version de base du paquet existante, date d'effet
    /// libre, et continuité des motifs avec sa base et avec la version de dossier qui précède.
    ///
    /// # Errors
    /// [`ErreurFiscale::PorteeInattendue`], [`ErreurFiscale::PaysSansPaquet`],
    /// [`ErreurFiscale::Version`] (base absente), [`ErreurFiscale::DateEffetEnDouble`],
    /// [`ErreurFiscale::MotifRetire`].
    pub fn verifier_version_de_dossier(
        &self,
        propres: &[ParametresFiscaux],
        nouvelle: &ParametresFiscaux,
    ) -> Result<(), ErreurFiscale> {
        if nouvelle.portee() == Portee::Pack {
            return Err(ErreurFiscale::PorteeInattendue(
                nouvelle.identifiant().to_owned(),
            ));
        }
        let pack = self.versions_du_pays(nouvelle.pays());
        if pack.is_empty() {
            return Err(ErreurFiscale::PaysSansPaquet(nouvelle.pays().to_owned()));
        }
        let base = pack
            .iter()
            .find(|v| v.version() == nouvelle.version())
            .ok_or_else(|| ErreurFiscale::Version(nouvelle.identifiant().to_owned()))?;
        let memes: Vec<&ParametresFiscaux> = propres
            .iter()
            .filter(|p| p.pays() == nouvelle.pays())
            .collect();
        if memes
            .iter()
            .any(|p| p.date_effet() == nouvelle.date_effet())
        {
            return Err(ErreurFiscale::DateEffetEnDouble {
                pays: nouvelle.pays().to_owned(),
            });
        }
        base.verifier_continuite_avec(nouvelle)?;
        if let Some(precedente) = memes
            .iter()
            .filter(|p| p.date_effet() < nouvelle.date_effet())
            .max_by_key(|p| p.date_effet())
        {
            precedente.verifier_continuite_avec(nouvelle)?;
        }
        Ok(())
    }

    /// Versions applicables à un dossier : le paquet de son pays et ses versions propres
    /// (`propres`, de ce pays ; les autres sont ignorées par prudence).
    #[must_use]
    pub fn versions_du_dossier<'a>(
        &'a self,
        pays: &str,
        propres: &'a [ParametresFiscaux],
    ) -> VersionsDuDossier<'a> {
        let mut propres: Vec<&ParametresFiscaux> = propres
            .iter()
            .filter(|p| p.pays() == pays && p.portee() != Portee::Pack)
            .collect();
        propres.sort_by_key(|p| p.date_effet());
        VersionsDuDossier {
            pack: self.versions_du_pays(pays),
            propres,
        }
    }
}

/// Dans une portée, version qui s'applique à `jour` : celle de plus grande date d'effet au plus
/// égale à `jour`, si `jour` ne dépasse pas sa date de fin. `versions` est trié par date d'effet.
fn dans_la_portee<'a, I>(versions: I, jour: Date) -> Option<&'a ParametresFiscaux>
where
    I: IntoIterator<Item = &'a ParametresFiscaux>,
{
    versions
        .into_iter()
        .filter(|v| v.date_effet() <= jour)
        .max_by_key(|v| v.date_effet())
        .filter(|v| v.couvre(jour))
}

/// Versions d'un dossier : paquet de son pays et versions propres.
#[derive(Debug, Clone)]
pub struct VersionsDuDossier<'a> {
    pack: &'a [ParametresFiscaux],
    propres: Vec<&'a ParametresFiscaux>,
}

impl<'a> VersionsDuDossier<'a> {
    /// Version applicable à une date : celle du dossier si une version du dossier s'applique ce
    /// jour-là, sinon celle du paquet (ADR 0030 point 3) ; `None` s'il n'y en a aucune.
    #[must_use]
    pub fn applicable(&self, jour: Date) -> Option<&'a ParametresFiscaux> {
        dans_la_portee(self.propres.iter().copied(), jour)
            .or_else(|| dans_la_portee(self.pack, jour))
    }

    /// Version par son identifiant enregistré (`sn-2025.1`, `sn-2025.1+d2`).
    #[must_use]
    pub fn par_identifiant(&self, identifiant: &str) -> Option<&'a ParametresFiscaux> {
        self.propres
            .iter()
            .copied()
            .chain(self.pack)
            .find(|v| v.identifiant() == identifiant)
    }
}
