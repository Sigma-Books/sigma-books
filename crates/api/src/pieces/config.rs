//! Configuration des pièces, lue dans l'environnement par
//! [`crate::serveur::config::Configuration`]. Aucune valeur n'est recopiée dans une erreur.
//!
//! Le stockage objet est **obligatoire** (ADR 0015) : sans ses quatre variables, ou si le
//! compartiment n'a pas le versionnement et le verrouillage d'objet, le binaire **refuse de
//! démarrer**. Aucun autre stockage n'est sélectionnable par la configuration.
//!
//! | Variable | Obligatoire | Contenu |
//! |---|---|---|
//! | `SIGMA_PIECES_S3_POINT_ACCES` | oui | Point d'accès compatible S3, `http://hôte[:port]` (TLS : relais de l'hébergement, L9) |
//! | `SIGMA_PIECES_S3_COMPARTIMENT` | oui | Compartiment créé **avec** verrouillage d'objet (donc versionné) |
//! | `SIGMA_PIECES_S3_CLE` | oui | Identifiant d'accès du rôle applicatif (secret d'exécution) |
//! | `SIGMA_PIECES_S3_SECRET` | oui | Clé secrète du rôle applicatif (secret d'exécution) |
//! | `SIGMA_PIECES_S3_HTTP_ASSUME` | oui (point d'accès `http://`) | `1` : absence de TLS vers le stockage assumée. **L9 : TLS ou relais local** ; en L2 le client n'a pas TLS |
//! | `SIGMA_PIECES_S3_REGION` | non | Région de la signature ; `us-east-1` par défaut |
//! | `SIGMA_PIECES_S3_STYLE` | non | `chemin` (défaut, compartiment dans le chemin) ou `hote` (dans le nom d'hôte) |
//! | `SIGMA_PIECES_TAILLE_MAX` | non | Taille maximale d'une pièce, en octets : 20 Mio par défaut, de 1 Kio à 100 Mio |
//! | `SIGMA_PIECES_RETENTION_ANS` | non | Rétention posée sur chaque objet, en années : 10 par défaut, de 10 à 100 |
//! | `SIGMA_PIECES_DELAI_DEPOT` | non | Délai de traitement d'un dépôt, en secondes : 120 par défaut, de 30 à 900 |

use std::time::Duration;

use rusty_s3::Credentials;
use url::Url;

use super::s3::ConfigurationS3;
use super::{RETENTION_MIN_ANS, ReglagesPieces};
use crate::serveur::config::{Environnement, ErreurConfiguration};

/// Configuration des pièces : accès au compartiment et réglages.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigurationPieces {
    /// Accès au compartiment S3.
    pub s3: ConfigurationS3,
    /// Réglages du service.
    pub reglages: ReglagesPieces,
}

/// Lit la configuration des pièces.
///
/// # Errors
/// [`ErreurConfiguration`] : variable manquante ou invalide.
pub fn configuration(env: &Environnement<'_>) -> Result<ConfigurationPieces, ErreurConfiguration> {
    let mut reglages = ReglagesPieces::default();
    if let Some(taille) = entier(env, "SIGMA_PIECES_TAILLE_MAX", 1024, 100 * 1024 * 1024)? {
        reglages.taille_max = usize::try_from(taille)
            .map_err(|_| ErreurConfiguration::Invalide("SIGMA_PIECES_TAILLE_MAX"))?;
    }
    if let Some(ans) = entier(
        env,
        "SIGMA_PIECES_RETENTION_ANS",
        u64::from(RETENTION_MIN_ANS),
        100,
    )? {
        reglages.retention_ans = u16::try_from(ans)
            .map_err(|_| ErreurConfiguration::Invalide("SIGMA_PIECES_RETENTION_ANS"))?;
    }
    if let Some(secondes) = entier(env, "SIGMA_PIECES_DELAI_DEPOT", 30, 900)? {
        reglages.delai_depot = Duration::from_secs(secondes);
    }
    let point_acces = env.obligatoire("SIGMA_PIECES_S3_POINT_ACCES")?;
    let invalide = ErreurConfiguration::Invalide("SIGMA_PIECES_S3_POINT_ACCES");
    let point_acces: Url = point_acces.trim().parse().map_err(|_| invalide)?;
    let admis = point_acces.scheme() == "http"
        && point_acces.host_str().is_some()
        && point_acces.username().is_empty()
        && point_acces.password().is_none()
        && point_acces.query().is_none()
        && point_acces.fragment().is_none();
    if !admis {
        return Err(invalide);
    }
    // Transport en clair : seulement si l'exploitant l'assume explicitement (L9 : TLS ou relais
    // local). Aucun mode « production » n'existe encore : l'exigence vaut partout.
    match env
        .facultative("SIGMA_PIECES_S3_HTTP_ASSUME")
        .as_deref()
        .map(str::trim)
    {
        Some("1") => {}
        Some(_) => return Err(ErreurConfiguration::Invalide("SIGMA_PIECES_S3_HTTP_ASSUME")),
        None => {
            return Err(ErreurConfiguration::Manquante(
                "SIGMA_PIECES_S3_HTTP_ASSUME",
            ));
        }
    }
    let compartiment = env.obligatoire("SIGMA_PIECES_S3_COMPARTIMENT")?;
    let compartiment = compartiment.trim().to_owned();
    // Règles de nommage S3 : 3 à 63 caractères, minuscules, chiffres, `.` et `-`.
    let nom_admis = (3..=63).contains(&compartiment.len())
        && compartiment
            .bytes()
            .all(|o| o.is_ascii_lowercase() || o.is_ascii_digit() || o == b'.' || o == b'-');
    if !nom_admis {
        return Err(ErreurConfiguration::Invalide(
            "SIGMA_PIECES_S3_COMPARTIMENT",
        ));
    }
    let cle = env.obligatoire("SIGMA_PIECES_S3_CLE")?;
    let secret = env.obligatoire("SIGMA_PIECES_S3_SECRET")?;
    let region = env
        .facultative("SIGMA_PIECES_S3_REGION")
        .map_or_else(|| "us-east-1".to_owned(), |r| r.trim().to_owned());
    if !region
        .bytes()
        .all(|o| o.is_ascii_lowercase() || o.is_ascii_digit() || o == b'-')
    {
        return Err(ErreurConfiguration::Invalide("SIGMA_PIECES_S3_REGION"));
    }
    let style_chemin = match env.facultative("SIGMA_PIECES_S3_STYLE").as_deref() {
        None | Some("chemin") => true,
        Some("hote") => false,
        Some(_) => return Err(ErreurConfiguration::Invalide("SIGMA_PIECES_S3_STYLE")),
    };
    Ok(ConfigurationPieces {
        s3: ConfigurationS3 {
            point_acces,
            compartiment,
            region,
            style_chemin,
            identifiants: Credentials::new(cle.trim(), secret.trim()),
        },
        reglages,
    })
}

/// Entier facultatif dans `[min, max]`.
fn entier(
    env: &Environnement<'_>,
    nom: &'static str,
    min: u64,
    max: u64,
) -> Result<Option<u64>, ErreurConfiguration> {
    env.facultative(nom)
        .map(|texte| {
            texte
                .trim()
                .parse::<u64>()
                .ok()
                .filter(|n| (min..=max).contains(n))
                .ok_or(ErreurConfiguration::Invalide(nom))
        })
        .transpose()
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    fn lire(paires: &[(&str, &str)]) -> Result<ConfigurationPieces, ErreurConfiguration> {
        let table: HashMap<String, String> = paires
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        let f = move |nom: &str| table.get(nom).cloned();
        configuration(&Environnement::new(&f))
    }

    const S3: [(&str, &str); 5] = [
        ("SIGMA_PIECES_S3_POINT_ACCES", "http://127.0.0.1:9100"),
        ("SIGMA_PIECES_S3_COMPARTIMENT", "sigma-pieces"),
        ("SIGMA_PIECES_S3_CLE", "cle-exemple"),
        ("SIGMA_PIECES_S3_SECRET", "valeur-confidentielle"),
        ("SIGMA_PIECES_S3_HTTP_ASSUME", "1"),
    ];

    #[test]
    fn le_stockage_est_obligatoire_et_les_reglages_ont_leurs_defauts() {
        assert_eq!(
            lire(&[]),
            Err(ErreurConfiguration::Manquante(
                "SIGMA_PIECES_S3_POINT_ACCES"
            ))
        );
        let c = lire(&S3).unwrap();
        assert_eq!(c.reglages, ReglagesPieces::default());
        assert_eq!(c.reglages.taille_max, 20 * 1024 * 1024);
        assert_eq!(c.reglages.retention_ans, 10);
    }

    #[test]
    fn configuration_complete() {
        let mut paires = S3.to_vec();
        paires.extend([
            ("SIGMA_PIECES_TAILLE_MAX", "1048576"),
            ("SIGMA_PIECES_RETENTION_ANS", "12"),
            ("SIGMA_PIECES_DELAI_DEPOT", "300"),
            ("SIGMA_PIECES_S3_STYLE", "hote"),
        ]);
        let c = lire(&paires).unwrap();
        let s3 = c.s3.clone();
        assert_eq!(s3.compartiment, "sigma-pieces");
        assert_eq!(s3.region, "us-east-1");
        assert!(!s3.style_chemin);
        assert_eq!(c.reglages.taille_max, 1_048_576);
        assert_eq!(c.reglages.retention_ans, 12);
        assert_eq!(c.reglages.delai_depot, Duration::from_secs(300));
        assert!(!format!("{c:?}").contains("valeur-confidentielle"));
    }

    #[test]
    fn variables_manquantes_ou_invalides() {
        for i in 0..S3.len() {
            let mut paires = S3.to_vec();
            let (nom, _) = paires.remove(i);
            assert_eq!(lire(&paires), Err(ErreurConfiguration::Manquante(nom)));
        }
        for (nom, valeur) in [
            ("SIGMA_PIECES_S3_POINT_ACCES", "https://s3.exemple.invalid"),
            (
                "SIGMA_PIECES_S3_POINT_ACCES",
                "http://cle:secret@s3.invalid",
            ),
            ("SIGMA_PIECES_S3_POINT_ACCES", "http://s3.invalid/?x=1"),
            ("SIGMA_PIECES_S3_POINT_ACCES", "pas une url"),
            ("SIGMA_PIECES_S3_COMPARTIMENT", "Majuscules"),
            ("SIGMA_PIECES_S3_COMPARTIMENT", "xy"),
            ("SIGMA_PIECES_S3_REGION", "us east"),
            ("SIGMA_PIECES_S3_STYLE", "autre"),
            ("SIGMA_PIECES_S3_HTTP_ASSUME", "oui"),
            ("SIGMA_PIECES_S3_HTTP_ASSUME", "0"),
            ("SIGMA_PIECES_TAILLE_MAX", "0"),
            ("SIGMA_PIECES_TAILLE_MAX", "104857601"),
            ("SIGMA_PIECES_RETENTION_ANS", "9"),
            ("SIGMA_PIECES_DELAI_DEPOT", "dix"),
        ] {
            let mut paires = S3.to_vec();
            paires.retain(|(n, _)| *n != nom);
            paires.push((nom, valeur));
            let erreur = lire(&paires).unwrap_err();
            assert_eq!(erreur, ErreurConfiguration::Invalide(nom), "{nom}={valeur}");
            assert!(!erreur.to_string().contains(valeur));
        }
    }
}
