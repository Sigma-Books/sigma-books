//! Paquets de pays embarqués : **une ligne par version publiée**.
//!
//! Ajouter un pays ou une loi de finances = déposer un répertoire `data/pays/<xx>/<version>/`
//! (jamais modifier un répertoire publié) et ajouter une ligne ici (ADR 0030 point 1).

/// Textes d'une version publiée d'un paquet : en-tête, régimes, motifs.
pub(crate) struct VersionEmbarquee {
    pub(crate) version: &'static str,
    pub(crate) fichiers: [&'static str; 3],
}

macro_rules! version {
    ($pays:literal, $version:literal) => {
        VersionEmbarquee {
            version: $version,
            fichiers: [
                include_str!(concat!(
                    "../data/pays/",
                    $pays,
                    "/",
                    $version,
                    "/version.toml"
                )),
                include_str!(concat!(
                    "../data/pays/",
                    $pays,
                    "/",
                    $version,
                    "/regimes-tva.toml"
                )),
                include_str!(concat!(
                    "../data/pays/",
                    $pays,
                    "/",
                    $version,
                    "/motifs-exoneration.toml"
                )),
            ],
        }
    };
}

/// Paquets officiels : (pays en minuscules, versions).
pub(crate) const PAQUETS: &[(&str, &[VersionEmbarquee])] = &[("sn", &[version!("sn", "2025.1")])];
