//! Configuration du relais SMTP (ADR 0012, question 8a ; ADR 0024), lue dans l'environnement.
//!
//! | Variable | Obligatoire | Contenu |
//! |---|---|---|
//! | `SIGMA_SMTP_HOTE` | non | Nom d'hôte du relais. **Absente : aucun courriel n'est envoyé** (les envois attendent ; avertissement au démarrage). Les autres variables n'ont alors aucun effet. |
//! | `SIGMA_SMTP_EXPEDITEUR` | si l'hôte est donné | Adresse d'expédition (domaine d'envoi : SPF, DKIM, DMARC relèvent de l'exploitation, F12.1). |
//! | `SIGMA_SMTP_PORT` | non | Port (587 par défaut en `starttls`, 465 en `implicite`). |
//! | `SIGMA_SMTP_TLS` | non | `starttls` (défaut : le relais **doit** annoncer STARTTLS, sinon aucun envoi) ou `implicite` (TLS dès la connexion). **Aucune valeur n'admet l'absence de TLS.** |
//! | `SIGMA_SMTP_UTILISATEUR` | non | Identifiant d'authentification. |
//! | `SIGMA_SMTP_FICHIER_MOT_DE_PASSE` | avec l'utilisateur | **Chemin d'un fichier** de droits 0600 ou 0400 contenant le mot de passe (secret d'exécution : jamais dans l'environnement, jamais dans le dépôt). |
//! | `SIGMA_SMTP_NOM_EXPEDITEUR` | non | Nom d'affichage de l'expéditeur (« Sigma Books » par défaut). |
//! | `SIGMA_SMTP_NOM_HELO` | non | Nom annoncé à `EHLO` (par défaut, l'hôte de l'origine de l'instance). |
//! | `SIGMA_SMTP_DELAI` | non | Délai de chaque échange, en secondes (30 par défaut, 1 à 300). |
//!
//! Le mot de passe est effacé de la mémoire à la libération ; `Debug` ne montre aucun secret.

use std::fmt;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use zeroize::Zeroizing;

use crate::courriel;
use crate::serveur::config::{Environnement, ErreurConfiguration};

/// Mode du chiffrement de la liaison (jamais « aucun »).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModeTls {
    /// `STARTTLS` exigé : un relais qui ne l'annonce pas est refusé.
    Starttls,
    /// TLS dès la connexion.
    Implicite,
}

/// Identifiants d'authentification du relais.
#[derive(Clone, PartialEq, Eq)]
pub struct Identifiants {
    /// Identifiant.
    pub utilisateur: String,
    /// Mot de passe (effacé à la libération).
    pub mot_de_passe: Zeroizing<String>,
}

impl fmt::Debug for Identifiants {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Identifiants(<masqués>)")
    }
}

/// Relais SMTP configuré.
#[derive(Clone, PartialEq, Eq)]
pub struct ConfigurationSmtp {
    /// Nom d'hôte du relais.
    pub hote: String,
    /// Port.
    pub port: u16,
    /// Chiffrement de la liaison.
    pub tls: ModeTls,
    /// Authentification, si le relais l'exige.
    pub identifiants: Option<Identifiants>,
    /// Adresse d'expédition, normalisée.
    pub expediteur: String,
    /// Nom d'affichage de l'expéditeur.
    pub nom_expediteur: String,
    /// Nom annoncé à `EHLO`.
    pub nom_helo: Option<String>,
    /// Délai de chaque échange.
    pub delai: std::time::Duration,
}

impl fmt::Debug for ConfigurationSmtp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ConfigurationSmtp")
            .field("hote", &self.hote)
            .field("port", &self.port)
            .field("tls", &self.tls)
            .field("identifiants", &self.identifiants)
            .field("expediteur", &self.expediteur)
            .finish_non_exhaustive()
    }
}

fn hote_valide(hote: &str) -> bool {
    !hote.is_empty()
        && hote.len() <= 253
        && hote.split('.').all(|e| {
            !e.is_empty()
                && e.len() <= 63
                && !e.starts_with('-')
                && !e.ends_with('-')
                && e.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        })
}

/// Lit un secret dans un fichier de droits restreints (0600 ou 0400) : un fichier lisible par
/// d'autres, vide ou trop gros, est refusé. Un seul retour à la ligne final est retiré.
fn lire_secret(chemin: &Path) -> Option<Zeroizing<String>> {
    let meta = std::fs::metadata(chemin).ok()?;
    if !meta.is_file() || meta.permissions().mode() & 0o077 != 0 || meta.len() > 1024 {
        return None;
    }
    let texte = Zeroizing::new(std::fs::read_to_string(chemin).ok()?);
    let secret = texte.strip_suffix('\n').unwrap_or(&texte);
    let secret = secret.strip_suffix('\r').unwrap_or(secret);
    (!secret.is_empty() && !secret.chars().any(char::is_control))
        .then(|| Zeroizing::new(secret.to_owned()))
}

/// Lit la configuration du relais. `Ok(None)` : `SIGMA_SMTP_HOTE` absente, pas d'envoi.
///
/// # Errors
/// [`ErreurConfiguration`] : variable manquante ou invalide (le nom, jamais la valeur).
pub fn depuis(env: &Environnement<'_>) -> Result<Option<ConfigurationSmtp>, ErreurConfiguration> {
    let Some(hote) = env.facultative("SIGMA_SMTP_HOTE") else {
        return Ok(None);
    };
    let hote = hote.trim().to_ascii_lowercase();
    if !hote_valide(&hote) {
        return Err(ErreurConfiguration::Invalide("SIGMA_SMTP_HOTE"));
    }
    let tls = match env.facultative("SIGMA_SMTP_TLS").as_deref().map(str::trim) {
        None | Some("starttls") => ModeTls::Starttls,
        Some("implicite") => ModeTls::Implicite,
        Some(_) => return Err(ErreurConfiguration::Invalide("SIGMA_SMTP_TLS")),
    };
    let port = match env.facultative("SIGMA_SMTP_PORT") {
        None => match tls {
            ModeTls::Starttls => 587,
            ModeTls::Implicite => 465,
        },
        Some(p) => p
            .trim()
            .parse::<u16>()
            .ok()
            .filter(|p| *p > 0)
            .ok_or(ErreurConfiguration::Invalide("SIGMA_SMTP_PORT"))?,
    };
    let expediteur = courriel::adresse(&env.obligatoire("SIGMA_SMTP_EXPEDITEUR")?)
        .ok_or(ErreurConfiguration::Invalide("SIGMA_SMTP_EXPEDITEUR"))?;
    let nom_expediteur = match env.facultative("SIGMA_SMTP_NOM_EXPEDITEUR") {
        None => "Sigma Books".to_owned(),
        Some(n) => {
            courriel::nom(&n).ok_or(ErreurConfiguration::Invalide("SIGMA_SMTP_NOM_EXPEDITEUR"))?
        }
    };
    let nom_helo = match env.facultative("SIGMA_SMTP_NOM_HELO") {
        None => None,
        Some(n) => {
            let n = n.trim().to_ascii_lowercase();
            if !hote_valide(&n) {
                return Err(ErreurConfiguration::Invalide("SIGMA_SMTP_NOM_HELO"));
            }
            Some(n)
        }
    };
    let delai = match env.facultative("SIGMA_SMTP_DELAI") {
        None => 30,
        Some(d) => d
            .trim()
            .parse::<u64>()
            .ok()
            .filter(|d| (1..=300).contains(d))
            .ok_or(ErreurConfiguration::Invalide("SIGMA_SMTP_DELAI"))?,
    };
    let identifiants = match env.facultative("SIGMA_SMTP_UTILISATEUR") {
        None => None,
        Some(utilisateur) => {
            let utilisateur = utilisateur.trim().to_owned();
            if utilisateur.is_empty()
                || utilisateur.chars().any(char::is_control)
                || utilisateur.len() > 254
            {
                return Err(ErreurConfiguration::Invalide("SIGMA_SMTP_UTILISATEUR"));
            }
            let fichier = env.obligatoire("SIGMA_SMTP_FICHIER_MOT_DE_PASSE")?;
            let mot_de_passe = lire_secret(Path::new(fichier.trim())).ok_or(
                ErreurConfiguration::Invalide("SIGMA_SMTP_FICHIER_MOT_DE_PASSE"),
            )?;
            Some(Identifiants {
                utilisateur,
                mot_de_passe,
            })
        }
    };
    Ok(Some(ConfigurationSmtp {
        hote,
        port,
        tls,
        identifiants,
        expediteur,
        nom_expediteur,
        nom_helo,
        delai: std::time::Duration::from_secs(delai),
    }))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::io::Write;

    use super::*;

    fn lire(vars: &[(&str, &str)]) -> Result<Option<ConfigurationSmtp>, ErreurConfiguration> {
        let table: BTreeMap<String, String> = vars
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        let f = move |nom: &str| table.get(nom).cloned();
        depuis(&Environnement::new(&f))
    }

    #[test]
    fn sans_hote_aucun_envoi() {
        assert_eq!(lire(&[]), Ok(None));
        assert_eq!(lire(&[("SIGMA_SMTP_PORT", "25")]), Ok(None));
    }

    #[test]
    fn valeurs_par_defaut_et_tls_jamais_absent() {
        let c = lire(&[
            ("SIGMA_SMTP_HOTE", "Relais.Exemple.SN"),
            ("SIGMA_SMTP_EXPEDITEUR", "noreply@exemple.sn"),
        ])
        .unwrap()
        .unwrap();
        assert_eq!(
            (c.hote.as_str(), c.port, c.tls),
            ("relais.exemple.sn", 587, ModeTls::Starttls)
        );
        assert_eq!(c.nom_expediteur, "Sigma Books");
        let c = lire(&[
            ("SIGMA_SMTP_HOTE", "relais.exemple.sn"),
            ("SIGMA_SMTP_EXPEDITEUR", "noreply@exemple.sn"),
            ("SIGMA_SMTP_TLS", "implicite"),
        ])
        .unwrap()
        .unwrap();
        assert_eq!((c.port, c.tls), (465, ModeTls::Implicite));
        // Aucune valeur n'admet l'absence de TLS.
        for tls in ["aucun", "non", "plain", "", "STARTTLS "] {
            let r = lire(&[
                ("SIGMA_SMTP_HOTE", "relais.exemple.sn"),
                ("SIGMA_SMTP_EXPEDITEUR", "noreply@exemple.sn"),
                ("SIGMA_SMTP_TLS", tls),
            ]);
            if tls.trim() == "STARTTLS" || tls.is_empty() {
                // Vide : variable absente ; casse : refusée.
                assert_eq!(r.is_ok(), tls.is_empty(), "{tls:?}");
            } else {
                assert_eq!(
                    r,
                    Err(ErreurConfiguration::Invalide("SIGMA_SMTP_TLS")),
                    "{tls:?}"
                );
            }
        }
    }

    #[test]
    fn valeurs_invalides_refusees_sans_les_citer() {
        let base = [
            ("SIGMA_SMTP_HOTE", "relais.exemple.sn"),
            ("SIGMA_SMTP_EXPEDITEUR", "noreply@exemple.sn"),
        ];
        let avec = |extra: (&str, &str)| {
            let mut v = base.to_vec();
            v.push(extra);
            lire(&v)
        };
        assert!(avec(("SIGMA_SMTP_PORT", "0")).is_err());
        assert!(avec(("SIGMA_SMTP_PORT", "70000")).is_err());
        assert!(avec(("SIGMA_SMTP_DELAI", "0")).is_err());
        assert!(avec(("SIGMA_SMTP_DELAI", "301")).is_err());
        assert!(avec(("SIGMA_SMTP_NOM_EXPEDITEUR", "A\r\nBcc: x@y.sn")).is_err());
        assert!(avec(("SIGMA_SMTP_NOM_HELO", "mauvais nom")).is_err());
        assert!(
            lire(&[
                ("SIGMA_SMTP_HOTE", "rel ais"),
                ("SIGMA_SMTP_EXPEDITEUR", "a@b.sn")
            ])
            .is_err()
        );
        assert!(lire(&[("SIGMA_SMTP_HOTE", "relais.exemple.sn")]).is_err());
        assert!(
            lire(&[
                ("SIGMA_SMTP_HOTE", "relais.exemple.sn"),
                ("SIGMA_SMTP_EXPEDITEUR", "a@b.sn\nBcc: x@y.sn")
            ])
            .is_err()
        );
        let erreur = format!(
            "{:?}",
            avec(("SIGMA_SMTP_PORT", "secret-valeur")).unwrap_err()
        );
        assert!(!erreur.contains("secret-valeur"));
    }

    #[test]
    fn le_mot_de_passe_vient_d_un_fichier_aux_droits_restreints() {
        let repertoire = std::env::temp_dir().join(format!("sigma-smtp-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&repertoire).unwrap();
        let fichier = repertoire.join("secret");
        let ecrire = |mode: u32, contenu: &str| {
            let mut f = std::fs::File::create(&fichier).unwrap();
            f.write_all(contenu.as_bytes()).unwrap();
            std::fs::set_permissions(&fichier, std::fs::Permissions::from_mode(mode)).unwrap();
        };
        let vars = |chemin: &str| {
            lire(&[
                ("SIGMA_SMTP_HOTE", "relais.exemple.sn"),
                ("SIGMA_SMTP_EXPEDITEUR", "noreply@exemple.sn"),
                ("SIGMA_SMTP_UTILISATEUR", "envoi"),
                ("SIGMA_SMTP_FICHIER_MOT_DE_PASSE", chemin),
            ])
        };
        let chemin = fichier.to_str().unwrap().to_owned();
        ecrire(0o600, "valeur-de-test\n");
        let c = vars(&chemin).unwrap().unwrap();
        assert_eq!(
            c.identifiants.as_ref().unwrap().mot_de_passe.as_str(),
            "valeur-de-test"
        );
        // Jamais affiché.
        let debug = format!("{c:?}");
        assert!(!debug.contains("valeur-de-test"), "{debug}");
        // Lisible par d'autres, vide, ou absent : refusé.
        ecrire(0o644, "valeur-de-test");
        assert!(vars(&chemin).is_err());
        ecrire(0o600, "\n");
        assert!(vars(&chemin).is_err());
        assert!(vars("/inexistant/secret").is_err());
        // Utilisateur sans fichier : refusé.
        assert!(
            lire(&[
                ("SIGMA_SMTP_HOTE", "relais.exemple.sn"),
                ("SIGMA_SMTP_EXPEDITEUR", "noreply@exemple.sn"),
                ("SIGMA_SMTP_UTILISATEUR", "envoi"),
            ])
            .is_err()
        );
        std::fs::remove_dir_all(&repertoire).unwrap();
    }
}
