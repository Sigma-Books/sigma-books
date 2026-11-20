//! Configuration du binaire, lue dans l'environnement. Incomplète ou invalide : le serveur
//! refuse de démarrer. Aucun message d'erreur ne recopie une valeur (l'URL de la base contient
//! un mot de passe) : seul le nom de la variable est cité.
//!
//! # Une lecture par module
//! Chaque module qui a des variables d'environnement a sa structure, sa lecture et la table de
//! ses variables dans sa documentation ; [`Configuration`] les compose. Index de toutes les
//! variables : `crates/api/README.md`, « Variables d'environnement ».
//!
//! | Module | Lecture | Variables |
//! |---|---|---|
//! | serveur (ce module) | [`Configuration::depuis`] | `SIGMA_ADRESSE_ECOUTE`, `SIGMA_ORIGINE`, `SIGMA_REPERTOIRE_WEB`, `SIGMA_EN_TETE_ADRESSE_CLIENT` |
//! | base | [`crate::base::config`] | `SIGMA_URL_BASE_APPLICATION`, `SIGMA_URL_BASE_LECTURE_AUDIT` |
//! | horodatage | [`crate::horodatage::config`] | `SIGMA_AUTORITES_HORODATAGE` |
//! | sessions | [`super::session::configurer`] | `SIGMA_CONSERVATION_SESSIONS_JOURS` |
//! | facteurs | [`super::facteurs::configurer`] | `SIGMA_FICHIER_CLE_INSTANCE` |
//! | pièces | [`crate::pieces::config`] | `SIGMA_PIECES_*` |
//!
//! Variables du serveur :
//!
//! | Variable | Obligatoire | Contenu |
//! |---|---|---|
//! | `SIGMA_ADRESSE_ECOUTE` | oui | Adresse et port d'écoute (`127.0.0.1:8080`) |
//! | `SIGMA_ORIGINE` | oui | Origine publique (`https://compta.exemple.sn`), contrôlée par l'anti-CSRF |
//! | `SIGMA_REPERTOIRE_WEB` | non | Répertoire de l'interface compilée ; doit exister |
//! | `SIGMA_EN_TETE_ADRESSE_CLIENT` | non | En-tête de confiance du mandataire inverse (L9) ; **absent par défaut** |

use std::net::SocketAddr;
use std::path::PathBuf;

use axum::http::HeaderName;

use super::Reglages;
use crate::base::config::ConfigurationBase;
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

/// Accès aux variables d'environnement, commun aux lectures de chaque module : une variable vide
/// (ou faite d'espaces) vaut une variable absente.
pub struct Environnement<'a> {
    lire: &'a dyn Fn(&str) -> Option<String>,
}

impl<'a> Environnement<'a> {
    /// Variables lues par la fonction `lire` (nom de variable → valeur).
    #[must_use]
    pub fn new(lire: &'a dyn Fn(&str) -> Option<String>) -> Self {
        Self { lire }
    }

    /// Variable obligatoire.
    ///
    /// # Errors
    /// [`ErreurConfiguration::Manquante`] si elle est absente ou vide.
    pub fn obligatoire(&self, nom: &'static str) -> Result<String, ErreurConfiguration> {
        self.facultative(nom)
            .ok_or(ErreurConfiguration::Manquante(nom))
    }

    /// Variable facultative : `None` si elle est absente ou vide.
    #[must_use]
    pub fn facultative(&self, nom: &'static str) -> Option<String> {
        (self.lire)(nom).filter(|v| !v.trim().is_empty())
    }
}

/// Configuration du binaire : la composition des configurations des modules. `Debug` ne montre
/// aucun secret (chaque module masque les siens).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Configuration {
    /// Adresse d'écoute.
    pub adresse_ecoute: SocketAddr,
    /// Accès à la base ([`crate::base::config`]).
    pub base: ConfigurationBase,
    /// Réglages du serveur, durées des sessions comprises ([`super::session::configurer`]).
    pub reglages: Reglages,
    /// Autorités d'horodatage, dans l'ordre d'essai : les seules destinations du client
    /// d'horodatage (ADR 0010 ; [`crate::horodatage::config`]).
    pub autorites_horodatage: Vec<Autorite>,
    // Voie authentification (PR 8b, 8c, 9) : configuration de ses modules.
    //
    // Voie comptabilité (PR 11, 13, 14) : configuration de ses modules.
    //
    // Voie pièces (PR 12) : configuration de ses modules.
    /// Pièces justificatives : stockage objet et réglages ([`crate::pieces::config`]).
    pub pieces: crate::pieces::config::ConfigurationPieces,
    // Voie intégrité et horodatage (PR 15) : configuration de ses modules.
}

impl Configuration {
    /// Lit la configuration dans l'environnement du processus.
    ///
    /// # Errors
    /// [`ErreurConfiguration`] si une variable obligatoire manque ou si une valeur est invalide.
    pub fn depuis_environnement() -> Result<Self, ErreurConfiguration> {
        Self::depuis(|nom| std::env::var(nom).ok())
    }

    /// Lit la configuration par la fonction `lire` (nom de variable → valeur), module par
    /// module.
    ///
    /// # Errors
    /// [`ErreurConfiguration`] si une variable obligatoire manque ou si une valeur est invalide.
    pub fn depuis(lire: impl Fn(&str) -> Option<String>) -> Result<Self, ErreurConfiguration> {
        let env = Environnement::new(&lire);
        let adresse_ecoute = adresse_ecoute(&env)?;
        let base = ConfigurationBase::depuis(&env)?;
        let mut reglages = reglages(&env)?;
        let autorites_horodatage = crate::horodatage::config::autorites(&env)?;
        super::session::configurer(&env, &mut reglages.sessions)?;
        // Voie authentification (PR 8b, 8c, 9).
        super::facteurs::configurer(&env, &mut reglages.facteurs)?;
        //
        // Voie comptabilité (PR 11, 13, 14).
        //
        // Voie pièces (PR 12).
        let pieces = crate::pieces::config::configuration(&env)?;
        // Voie intégrité et horodatage (PR 15).
        Ok(Self {
            adresse_ecoute,
            base,
            reglages,
            autorites_horodatage,
            pieces,
        })
    }
}

/// Adresse d'écoute du serveur.
fn adresse_ecoute(env: &Environnement<'_>) -> Result<SocketAddr, ErreurConfiguration> {
    env.obligatoire("SIGMA_ADRESSE_ECOUTE")?
        .trim()
        .parse()
        .map_err(|_| ErreurConfiguration::Invalide("SIGMA_ADRESSE_ECOUTE"))
}

/// Réglages du serveur : origine, interface compilée, en-tête du mandataire.
fn reglages(env: &Environnement<'_>) -> Result<Reglages, ErreurConfiguration> {
    let origine = origine(&env.obligatoire("SIGMA_ORIGINE")?)
        .ok_or(ErreurConfiguration::Invalide("SIGMA_ORIGINE"))?;
    let mut reglages = Reglages::new(origine);
    if let Some(repertoire) = env.facultative("SIGMA_REPERTOIRE_WEB") {
        let repertoire = PathBuf::from(repertoire);
        if !repertoire.is_dir() {
            return Err(ErreurConfiguration::Invalide("SIGMA_REPERTOIRE_WEB"));
        }
        reglages.repertoire_web = Some(repertoire);
    }
    if let Some(nom) = env.facultative("SIGMA_EN_TETE_ADRESSE_CLIENT") {
        reglages.en_tete_adresse_client = Some(
            HeaderName::try_from(nom.trim())
                .map_err(|_| ErreurConfiguration::Invalide("SIGMA_EN_TETE_ADRESSE_CLIENT"))?,
        );
    }
    Ok(reglages)
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

    /// Fichier de clé de l'instance de ces tests (droits 0600), écrit une fois dans le
    /// répertoire temporaire, et un fichier aux droits trop larges.
    fn fichiers_de_cle() -> &'static (String, String) {
        use std::os::unix::fs::PermissionsExt;
        static FICHIERS: std::sync::OnceLock<(String, String)> = std::sync::OnceLock::new();
        FICHIERS.get_or_init(|| {
            let repertoire =
                std::env::temp_dir().join(format!("sigma-config-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&repertoire).unwrap();
            let ecrire = |nom: &str, mode: u32| {
                let chemin = repertoire.join(nom);
                std::fs::write(&chemin, "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=").unwrap();
                std::fs::set_permissions(&chemin, std::fs::Permissions::from_mode(mode)).unwrap();
                chemin.display().to_string()
            };
            (ecrire("cle", 0o600), ecrire("cle-ouverte", 0o644))
        })
    }

    fn complete() -> Vec<(&'static str, &'static str)> {
        vec![
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
            ("SIGMA_FICHIER_CLE_INSTANCE", fichiers_de_cle().0.as_str()),
        ]
    }

    #[test]
    fn une_configuration_complete_est_acceptee() {
        let config = Configuration::depuis(lire(&complete())).unwrap();
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
        // La clé de l'instance est lue ; `Debug` n'en montre que l'identifiant.
        assert!(config.reglages.facteurs.cle_instance.is_some());
        assert!(!format!("{config:?}").contains("AAAAAAAA"));
    }

    #[test]
    fn la_conservation_des_sessions_est_configurable() {
        let mut paires = complete();
        paires.push(("SIGMA_CONSERVATION_SESSIONS_JOURS", " 90 "));
        let config = Configuration::depuis(lire(&paires)).unwrap();
        assert_eq!(
            config.reglages.sessions.conservation,
            time::Duration::days(90)
        );
    }

    #[test]
    fn chaque_variable_obligatoire_manquante_empeche_le_demarrage() {
        for i in 0..complete().len() {
            let mut paires = complete();
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
            ("SIGMA_FICHIER_CLE_INSTANCE", "/fichier/inexistant/sigma"),
            ("SIGMA_FICHIER_CLE_INSTANCE", fichiers_de_cle().1.as_str()),
        ] {
            let mut paires = complete();
            paires.retain(|(n, _)| *n != nom);
            paires.push((nom, valeur));
            let erreur = Configuration::depuis(lire(&paires)).unwrap_err();
            assert!(!erreur.to_string().contains(valeur), "{erreur}");
            assert!(matches!(erreur, ErreurConfiguration::Invalide(_)), "{nom}");
        }
    }

    #[test]
    fn la_liste_des_autorites_d_horodatage_est_configurable() {
        let mut paires = complete();
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
