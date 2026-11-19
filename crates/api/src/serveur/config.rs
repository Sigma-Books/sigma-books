//! Configuration du binaire, lue dans l'environnement. Incomplète ou invalide : le serveur
//! refuse de démarrer. Aucun message d'erreur ne recopie une valeur (l'URL de la base contient
//! un mot de passe) : seul le nom de la variable est cité.
//!
//! | Variable | Obligatoire | Contenu |
//! |---|---|---|
//! | `SIGMA_ADRESSE_ECOUTE` | oui | Adresse et port d'écoute (`127.0.0.1:8080`) |
//! | `SIGMA_URL_BASE_APPLICATION` | oui | URL PostgreSQL du **rôle applicatif** (`sigma_application`) |
//! | `SIGMA_URL_BASE_LECTURE_AUDIT` | oui | URL PostgreSQL du **rôle de lecture d'audit** (`sigma_lecture_audit`, ADR 0014) |
//! | `SIGMA_ORIGINE` | oui | Origine publique (`https://compta.exemple.sn`), contrôlée par l'anti-CSRF |
//! | `SIGMA_REPERTOIRE_WEB` | non | Répertoire de l'interface compilée ; doit exister |
//! | `SIGMA_EN_TETE_ADRESSE_CLIENT` | non | En-tête de confiance du mandataire inverse (L9) ; **absent par défaut** |
//! | `SIGMA_AUTORITES_HORODATAGE` | non | Autorités RFC 3161, séparées par des virgules, dans l'ordre d'essai (`http://` seul) ; par défaut DigiCert puis Sectigo (ADR 0010) |
//! | `SIGMA_CONSERVATION_SESSIONS_JOURS` | non | Jours de conservation d'une session close avant sa purge (1 à 3650 ; 30 par défaut, à confirmer par le mainteneur, loi 2008-12) |

use std::fmt;
use std::net::SocketAddr;
use std::path::PathBuf;

use axum::http::HeaderName;

use super::Reglages;
use crate::horodatage::Autorite;

/// Configuration refusée.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ErreurConfiguration {
    /// Variable obligatoire absente ou vide.
    #[error("variable d'environnement {0} absente ou vide")]
    Manquante(&'static str),
    /// Valeur invalide (la valeur n'est pas recopiée).
    #[error("variable d'environnement {0} invalide")]
    Invalide(&'static str),
}

/// Configuration du binaire.
#[derive(Clone, PartialEq, Eq)]
pub struct Configuration {
    /// Adresse d'écoute.
    pub adresse_ecoute: SocketAddr,
    /// URL de la base, rôle applicatif (secrète : jamais affichée).
    pub url_base: String,
    /// URL de la base, rôle de lecture d'audit (secrète : jamais affichée).
    pub url_base_lecture_audit: String,
    /// Réglages du serveur.
    pub reglages: Reglages,
    /// Autorités d'horodatage, dans l'ordre d'essai : les seules destinations du client
    /// d'horodatage (ADR 0010).
    pub autorites_horodatage: Vec<Autorite>,
}

impl fmt::Debug for Configuration {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Configuration")
            .field("adresse_ecoute", &self.adresse_ecoute)
            .field("url_base", &"<masquée>")
            .field("url_base_lecture_audit", &"<masquée>")
            .field("reglages", &self.reglages)
            .field("autorites_horodatage", &self.autorites_horodatage)
            .finish()
    }
}

impl Configuration {
    /// Lit la configuration dans l'environnement du processus.
    ///
    /// # Errors
    /// [`ErreurConfiguration`] si une variable obligatoire manque ou si une valeur est invalide.
    pub fn depuis_environnement() -> Result<Self, ErreurConfiguration> {
        Self::depuis(|nom| std::env::var(nom).ok())
    }

    /// Lit la configuration par la fonction `lire` (nom de variable → valeur).
    ///
    /// # Errors
    /// [`ErreurConfiguration`] si une variable obligatoire manque ou si une valeur est invalide.
    pub fn depuis(lire: impl Fn(&str) -> Option<String>) -> Result<Self, ErreurConfiguration> {
        let obligatoire = |nom: &'static str| {
            lire(nom)
                .filter(|v| !v.trim().is_empty())
                .ok_or(ErreurConfiguration::Manquante(nom))
        };
        let facultative = |nom: &'static str| lire(nom).filter(|v| !v.trim().is_empty());

        let adresse_ecoute = obligatoire("SIGMA_ADRESSE_ECOUTE")?
            .trim()
            .parse()
            .map_err(|_| ErreurConfiguration::Invalide("SIGMA_ADRESSE_ECOUTE"))?;
        let url_base = obligatoire("SIGMA_URL_BASE_APPLICATION")?;
        if !(url_base.starts_with("postgres://") || url_base.starts_with("postgresql://")) {
            return Err(ErreurConfiguration::Invalide("SIGMA_URL_BASE_APPLICATION"));
        }
        let url_base_lecture_audit = obligatoire("SIGMA_URL_BASE_LECTURE_AUDIT")?;
        if !(url_base_lecture_audit.starts_with("postgres://")
            || url_base_lecture_audit.starts_with("postgresql://"))
        {
            return Err(ErreurConfiguration::Invalide(
                "SIGMA_URL_BASE_LECTURE_AUDIT",
            ));
        }
        let origine = origine(&obligatoire("SIGMA_ORIGINE")?)
            .ok_or(ErreurConfiguration::Invalide("SIGMA_ORIGINE"))?;

        let mut reglages = Reglages::new(origine);
        if let Some(repertoire) = facultative("SIGMA_REPERTOIRE_WEB") {
            let repertoire = PathBuf::from(repertoire);
            if !repertoire.is_dir() {
                return Err(ErreurConfiguration::Invalide("SIGMA_REPERTOIRE_WEB"));
            }
            reglages.repertoire_web = Some(repertoire);
        }
        if let Some(nom) = facultative("SIGMA_EN_TETE_ADRESSE_CLIENT") {
            reglages.en_tete_adresse_client = Some(
                HeaderName::try_from(nom.trim())
                    .map_err(|_| ErreurConfiguration::Invalide("SIGMA_EN_TETE_ADRESSE_CLIENT"))?,
            );
        }
        let autorites_horodatage = match facultative("SIGMA_AUTORITES_HORODATAGE") {
            Some(liste) => Autorite::liste(&liste)
                .map_err(|_| ErreurConfiguration::Invalide("SIGMA_AUTORITES_HORODATAGE"))?,
            None => Autorite::par_defaut(),
        };
        if let Some(jours) = facultative("SIGMA_CONSERVATION_SESSIONS_JOURS") {
            let jours: i64 = jours
                .trim()
                .parse()
                .ok()
                .filter(|j| (1..=3650).contains(j))
                .ok_or(ErreurConfiguration::Invalide(
                    "SIGMA_CONSERVATION_SESSIONS_JOURS",
                ))?;
            reglages.sessions.conservation = time::Duration::days(jours);
        }
        Ok(Self {
            adresse_ecoute,
            url_base,
            url_base_lecture_audit,
            reglages,
            autorites_horodatage,
        })
    }
}

/// Forme sérialisée d'une origine (`schéma://hôte[:port]`, en minuscules, sans chemin), telle
/// qu'un navigateur l'envoie dans `Origin`. `None` si le texte n'en est pas une.
fn origine(texte: &str) -> Option<String> {
    let texte = texte.trim().to_ascii_lowercase();
    let reste = texte
        .strip_prefix("https://")
        .or_else(|| texte.strip_prefix("http://"))?;
    let valide = !reste.is_empty()
        && reste
            .bytes()
            .all(|o| o.is_ascii_alphanumeric() || matches!(o, b'.' | b'-' | b':' | b'[' | b']'));
    valide.then_some(texte)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    fn lire(paires: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let table: HashMap<String, String> = paires
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        move |nom| table.get(nom).cloned()
    }

    const COMPLETE: [(&str, &str); 4] = [
        ("SIGMA_ADRESSE_ECOUTE", "127.0.0.1:8080"),
        (
            "SIGMA_URL_BASE_APPLICATION",
            "postgres://sigma_application@localhost/sigma_books",
        ),
        (
            "SIGMA_URL_BASE_LECTURE_AUDIT",
            "postgres://sigma_lecture_audit@localhost/sigma_books",
        ),
        ("SIGMA_ORIGINE", "https://Compta.Exemple.invalid"),
    ];

    #[test]
    fn une_configuration_complete_est_acceptee() {
        let config = Configuration::depuis(lire(&COMPLETE)).unwrap();
        assert_eq!(config.reglages.origine, "https://compta.exemple.invalid");
        assert_eq!(config.reglages.en_tete_adresse_client, None);
        assert_eq!(config.reglages.repertoire_web, None);
        assert_eq!(config.autorites_horodatage, Autorite::par_defaut());
        assert_eq!(
            config.reglages.sessions.conservation,
            time::Duration::days(30)
        );
        assert!(!format!("{config:?}").contains("sigma_application@"));
        assert!(!format!("{config:?}").contains("sigma_lecture_audit@"));
    }

    #[test]
    fn la_conservation_des_sessions_est_configurable() {
        let mut paires = COMPLETE.to_vec();
        paires.push(("SIGMA_CONSERVATION_SESSIONS_JOURS", " 90 "));
        let config = Configuration::depuis(lire(&paires)).unwrap();
        assert_eq!(
            config.reglages.sessions.conservation,
            time::Duration::days(90)
        );
    }

    #[test]
    fn chaque_variable_obligatoire_manquante_empeche_le_demarrage() {
        for i in 0..COMPLETE.len() {
            let mut paires = COMPLETE.to_vec();
            let (nom, _) = paires.remove(i);
            assert_eq!(
                Configuration::depuis(lire(&paires)),
                Err(ErreurConfiguration::Manquante(nom))
            );
        }
    }

    #[test]
    fn les_valeurs_invalides_sont_refusees_sans_etre_recopiees() {
        for (nom, valeur) in [
            ("SIGMA_ORIGINE", "https://exemple.invalid/chemin"),
            ("SIGMA_ORIGINE", "exemple.invalid"),
            ("SIGMA_ADRESSE_ECOUTE", "pas-une-adresse"),
            ("SIGMA_URL_BASE_APPLICATION", "mysql://secret@hote"),
            ("SIGMA_URL_BASE_LECTURE_AUDIT", "mysql://secret@hote"),
            ("SIGMA_REPERTOIRE_WEB", "/repertoire/inexistant/sigma"),
            ("SIGMA_EN_TETE_ADRESSE_CLIENT", "en tête invalide"),
            ("SIGMA_AUTORITES_HORODATAGE", "ftp://tsa.invalid"),
            ("SIGMA_AUTORITES_HORODATAGE", "https://tsa.invalid"),
            (
                "SIGMA_AUTORITES_HORODATAGE",
                "http://cle-secrete:x@tsa.invalid",
            ),
            ("SIGMA_AUTORITES_HORODATAGE", ","),
            ("SIGMA_AUTORITES_HORODATAGE", "http://169.254.169.254"),
            ("SIGMA_AUTORITES_HORODATAGE", "http://tsa.invalid:99999"),
            ("SIGMA_CONSERVATION_SESSIONS_JOURS", "0"),
            ("SIGMA_CONSERVATION_SESSIONS_JOURS", "3651"),
            ("SIGMA_CONSERVATION_SESSIONS_JOURS", "trente"),
        ] {
            let mut paires = COMPLETE.to_vec();
            paires.retain(|(n, _)| *n != nom);
            paires.push((nom, valeur));
            let erreur = Configuration::depuis(lire(&paires)).unwrap_err();
            assert!(!erreur.to_string().contains(valeur), "{erreur}");
            assert!(matches!(erreur, ErreurConfiguration::Invalide(_)), "{nom}");
        }
    }

    #[test]
    fn la_liste_des_autorites_d_horodatage_est_configurable() {
        let mut paires = COMPLETE.to_vec();
        paires.push((
            "SIGMA_AUTORITES_HORODATAGE",
            "http://tsa-b.exemple.invalid:3161/cle-secrete, http://tsa-a.exemple.invalid",
        ));
        let config = Configuration::depuis(lire(&paires)).unwrap();
        assert_eq!(
            config
                .autorites_horodatage
                .iter()
                .map(Autorite::nom)
                .collect::<Vec<_>>(),
            ["tsa-b.exemple.invalid:3161", "tsa-a.exemple.invalid"]
        );
        // `Debug` de la configuration : le nom des autorités, jamais leur chemin.
        let debug = format!("{config:?}");
        assert!(debug.contains("tsa-b.exemple.invalid:3161"), "{debug}");
        assert!(!debug.contains("cle-secrete"), "{debug}");
    }
}
