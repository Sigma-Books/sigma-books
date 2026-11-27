//! Paquets de pays embarqués : **une ligne par version publiée**.
//!
//! Ajouter un pays ou une loi de finances = déposer un répertoire `data/pays/<xx>/<version>/`
//! (jamais modifier un répertoire publié) et ajouter une ligne ici (ADR 0030 point 1).

/// Textes d'une version publiée : (nom du fichier, contenu), dans l'ordre de concaténation
/// (en-tête, régimes, motifs, puis les fichiers facultatifs comme `prelevements.toml`).
#[derive(Clone, Copy)]
pub(crate) struct VersionEmbarquee {
    pub(crate) version: &'static str,
    pub(crate) fichiers: &'static [(&'static str, &'static str)],
}

macro_rules! version {
    ($pays:literal, $version:literal $(, $fichier:literal)*) => {
        VersionEmbarquee {
            version: $version,
            fichiers: &[
                version!(@ $pays, $version, "version.toml"),
                version!(@ $pays, $version, "regimes-tva.toml"),
                version!(@ $pays, $version, "motifs-exoneration.toml"),
                $(version!(@ $pays, $version, $fichier),)*
            ],
        }
    };
    (@ $pays:literal, $version:literal, $fichier:literal) => {
        (
            $fichier,
            include_str!(concat!("../data/pays/", $pays, "/", $version, "/", $fichier)),
        )
    };
}

/// Paquets officiels : (pays en minuscules, versions).
pub(crate) const PAQUETS: &[(&str, &[VersionEmbarquee])] = &[(
    "sn",
    &[
        version!("sn", "2025.1"),
        // Loi n° 2025-17 : prélèvement (art. 220 ter) et taxe sur les transferts d'argent
        // (art. 395 s.), ADR 0034 point 10.
        version!("sn", "2025.2", "prelevements.toml"),
    ],
)];

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::fs;
    use std::path::Path;

    use super::*;
    use crate::{ErreurFiscale, Registre};

    /// Chaque répertoire `data/pays/<xx>/<version>` est enregistré, et chaque enregistrement a son
    /// répertoire : un paquet oublié dans `PAQUETS` (ou une ligne sans données) échoue ici.
    #[test]
    fn les_paquets_enregistres_sont_exactement_les_repertoires() {
        let racine = Path::new(env!("CARGO_MANIFEST_DIR")).join("data/pays");
        let mut sur_disque = BTreeSet::new();
        for pays in fs::read_dir(&racine).unwrap() {
            let pays = pays.unwrap().path();
            if !pays.is_dir() {
                continue;
            }
            for v in fs::read_dir(&pays).unwrap() {
                let v = v.unwrap().path();
                if v.is_dir() {
                    sur_disque.insert((
                        pays.file_name().unwrap().to_string_lossy().into_owned(),
                        v.file_name().unwrap().to_string_lossy().into_owned(),
                    ));
                }
            }
        }
        let enregistres: BTreeSet<(String, String)> = PAQUETS
            .iter()
            .flat_map(|(p, vs)| {
                vs.iter()
                    .map(move |v| ((*p).to_owned(), v.version.to_owned()))
            })
            .collect();
        assert_eq!(enregistres, sur_disque);
    }

    /// Chaque fichier d'un répertoire de version est embarqué (un `prelevements.toml` oublié dans
    /// la macro ne serait jamais chargé), et rien d'autre.
    #[test]
    fn les_fichiers_embarques_sont_exactement_ceux_du_repertoire() {
        let racine = Path::new(env!("CARGO_MANIFEST_DIR")).join("data/pays");
        for (pays, versions) in PAQUETS {
            for v in *versions {
                let sur_disque: BTreeSet<String> = fs::read_dir(racine.join(pays).join(v.version))
                    .unwrap()
                    .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                    .collect();
                let embarques: BTreeSet<String> = v
                    .fichiers
                    .iter()
                    .map(|(nom, _)| (*nom).to_owned())
                    .collect();
                assert_eq!(embarques, sur_disque, "{pays}/{}", v.version);
                for (nom, texte) in v.fichiers {
                    let lu =
                        fs::read_to_string(racine.join(pays).join(v.version).join(nom)).unwrap();
                    assert_eq!(*texte, lu);
                }
            }
        }
    }

    #[test]
    fn le_repertoire_et_l_en_tete_doivent_concorder() {
        let sn = &PAQUETS[0].1[0];
        let faux_pays: &[(&str, &[VersionEmbarquee])] = &[(
            "cm",
            &[VersionEmbarquee {
                version: sn.version,
                fichiers: sn.fichiers,
            }],
        )];
        assert!(matches!(
            Registre::depuis_embarques(faux_pays),
            Err(ErreurFiscale::Version(_))
        ));
        let faux_version: &[(&str, &[VersionEmbarquee])] = &[(
            "sn",
            &[VersionEmbarquee {
                version: "2025.9",
                fichiers: sn.fichiers,
            }],
        )];
        assert!(matches!(
            Registre::depuis_embarques(faux_version),
            Err(ErreurFiscale::Version(_))
        ));
        assert!(Registre::depuis_embarques(PAQUETS).is_ok());
    }
}
