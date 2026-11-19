//! Autorité d'horodatage : URL lue dans la **configuration** seulement (ADR 0010).
//!
//! Une [`Autorite`] ne se construit que par [`Autorite::parse`], appelée sur la configuration
//! du binaire : aucune URL issue d'une donnée (requête, base, réponse d'une autorité) ne peut
//! devenir une destination. Le client n'en suit aucune redirection.
//!
//! La configuration ([`Autorite::liste`]) refuse en outre les hôtes **littéraux** de boucle
//! locale, de lien local, privés ou non spécifiés, et `localhost`. Ce n'est pas une garde par
//! adresse : un nom qui se résout vers une adresse interne n'est pas détecté. La limite réelle
//! de la sortie réseau est le pare-feu de l'hébergement (L9 ; menace F6.4).

use std::fmt;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::str::FromStr;

use axum::http::Uri;

/// Autorités par défaut, dans l'ordre d'essai (ADR 0010) : `DigiCert`, puis `Sectigo` en secours.
pub const AUTORITES_PAR_DEFAUT: [&str; 2] = [
    "http://timestamp.digicert.com",
    "http://timestamp.sectigo.com",
];

/// URL d'autorité refusée. Le message ne recopie pas l'URL : elle pourrait porter un secret.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ErreurAutorite {
    /// Texte qui n'est pas une URL absolue.
    #[error("URL d'autorité d'horodatage invalide")]
    Invalide,
    /// Schéma autre que `http` ou `https`.
    #[error("schéma d'URL refusé pour une autorité d'horodatage (http seul)")]
    SchemaRefuse,
    /// `https` : le client n'embarque pas TLS (aucune autorité de l'ADR 0010 ne l'exige).
    #[error("https n'est pas pris en charge par le client d'horodatage (ADR 0010 : http)")]
    HttpsNonPrisEnCharge,
    /// Identifiants dans l'URL (`utilisateur:mot-de-passe@`).
    #[error("identifiants refusés dans l'URL d'une autorité d'horodatage")]
    IdentifiantsRefuses,
    /// Liste vide.
    #[error("liste d'autorités d'horodatage vide")]
    ListeVide,
    /// Hôte littéral interne (boucle locale, lien local, privé, non spécifié) ou `localhost`.
    #[error("autorité d'horodatage sur une adresse interne refusée")]
    AdresseInterne,
}

/// Autorité d'horodatage : hôte, port et chemin, en HTTP.
#[derive(Clone, PartialEq, Eq)]
pub struct Autorite {
    /// Hôte en minuscules ; une adresse IPv6 garde ses crochets.
    hote: String,
    port: u16,
    /// Chemin, commençant par `/` (`/` par défaut), sans requête.
    chemin: String,
}

impl fmt::Debug for Autorite {
    /// Nom seul ([`Autorite::nom`]) : le chemin n'est jamais affiché.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("Autorite").field(&self.nom()).finish()
    }
}

impl Autorite {
    /// Lit une URL d'autorité : `http://hôte[:port][/chemin]`, port de 1 à 65535, sans
    /// identifiants, requête (`?`) ni fragment.
    ///
    /// # Errors
    /// [`ErreurAutorite`] si l'URL est invalide, d'un autre schéma que `http`, ou porte des
    /// identifiants.
    pub fn parse(texte: &str) -> Result<Self, ErreurAutorite> {
        let texte = texte.trim();
        if texte.contains(['#', '?']) {
            return Err(ErreurAutorite::Invalide);
        }
        let uri: Uri = texte.parse().map_err(|_| ErreurAutorite::Invalide)?;
        match uri.scheme_str() {
            Some(s) if s.eq_ignore_ascii_case("http") => {}
            Some(s) if s.eq_ignore_ascii_case("https") => {
                return Err(ErreurAutorite::HttpsNonPrisEnCharge);
            }
            Some(_) => return Err(ErreurAutorite::SchemaRefuse),
            None => return Err(ErreurAutorite::Invalide),
        }
        let autorite = uri.authority().ok_or(ErreurAutorite::Invalide)?;
        if autorite.as_str().contains('@') {
            return Err(ErreurAutorite::IdentifiantsRefuses);
        }
        let hote = autorite.host().to_ascii_lowercase();
        if hote.is_empty() || hote == "[]" {
            return Err(ErreurAutorite::Invalide);
        }
        // `port_u16` ne distingue pas un port absent d'un port illisible ou hors plage : on lit
        // ce qui suit l'hôte dans l'autorité (identifiants exclus ci-dessus).
        let port = match autorite.as_str().get(autorite.host().len()..) {
            Some("") => 80,
            Some(reste) => reste
                .strip_prefix(':')
                .filter(|p| !p.is_empty() && p.bytes().all(|o| o.is_ascii_digit()))
                .and_then(|p| p.parse::<u16>().ok())
                .filter(|&p| p != 0)
                .ok_or(ErreurAutorite::Invalide)?,
            None => return Err(ErreurAutorite::Invalide),
        };
        if uri.query().is_some() {
            return Err(ErreurAutorite::Invalide);
        }
        let chemin = match uri.path() {
            "" => "/".to_owned(),
            c if c.starts_with('/') => c.to_owned(),
            _ => return Err(ErreurAutorite::Invalide),
        };
        Ok(Self { hote, port, chemin })
    }

    /// Lit la liste de la configuration : URL séparées par des virgules, dans l'ordre d'essai.
    /// Refuse en outre les hôtes littéraux internes ([`ErreurAutorite::AdresseInterne`]).
    ///
    /// # Errors
    /// [`ErreurAutorite`] si une URL est refusée ou si la liste est vide.
    pub fn liste(texte: &str) -> Result<Vec<Self>, ErreurAutorite> {
        let autorites = texte
            .split(',')
            .filter(|t| !t.trim().is_empty())
            .map(|t| {
                let autorite = Self::parse(t)?;
                if autorite.hote_interne() {
                    return Err(ErreurAutorite::AdresseInterne);
                }
                Ok(autorite)
            })
            .collect::<Result<Vec<_>, _>>()?;
        if autorites.is_empty() {
            return Err(ErreurAutorite::ListeVide);
        }
        Ok(autorites)
    }

    /// Autorités par défaut de l'ADR 0010 ([`AUTORITES_PAR_DEFAUT`]).
    #[must_use]
    pub fn par_defaut() -> Vec<Self> {
        AUTORITES_PAR_DEFAUT
            .iter()
            .filter_map(|url| Self::parse(url).ok())
            .collect()
    }

    /// Nom de l'autorité dans les journaux et l'en-tête `Host` : hôte, et port s'il n'est pas 80.
    /// Jamais le chemin, qui pourrait porter une clé d'accès.
    #[must_use]
    pub fn nom(&self) -> String {
        if self.port == 80 {
            self.hote.clone()
        } else {
            format!("{}:{}", self.hote, self.port)
        }
    }

    /// `localhost`, ou adresse littérale de boucle locale, de lien local, privée, partagée
    /// (100.64.0.0/10), de diffusion ou non spécifiée. Les noms ne sont pas résolus.
    fn hote_interne(&self) -> bool {
        let hote = self.hote_connexion();
        if hote == "localhost" || hote.ends_with(".localhost") {
            return true;
        }
        match hote.parse::<IpAddr>() {
            Ok(IpAddr::V4(v4)) => v4_interne(v4),
            Ok(IpAddr::V6(v6)) => v6_interne(v6),
            Err(_) => false,
        }
    }

    /// Hôte à résoudre (sans les crochets d'une adresse IPv6).
    pub(super) fn hote_connexion(&self) -> &str {
        self.hote
            .strip_prefix('[')
            .and_then(|h| h.strip_suffix(']'))
            .unwrap_or(&self.hote)
    }

    /// Port TCP.
    #[must_use]
    pub fn port(&self) -> u16 {
        self.port
    }

    /// Chemin de la requête (`/` par défaut).
    #[must_use]
    pub fn chemin(&self) -> &str {
        &self.chemin
    }
}

fn v4_interne(ip: Ipv4Addr) -> bool {
    let [a, b, ..] = ip.octets();
    ip.is_loopback()
        || ip.is_private()
        || ip.is_link_local()
        || ip.is_unspecified()
        || ip.is_broadcast()
        || (a == 100 && (64..128).contains(&b))
}

fn v6_interne(ip: Ipv6Addr) -> bool {
    if let Some(v4) = ip.to_ipv4_mapped() {
        return v4_interne(v4);
    }
    ip.is_loopback() || ip.is_unspecified() || ip.is_unique_local() || ip.is_unicast_link_local()
}

impl FromStr for Autorite {
    type Err = ErreurAutorite;

    fn from_str(texte: &str) -> Result<Self, Self::Err> {
        Self::parse(texte)
    }
}

impl fmt::Display for Autorite {
    /// Nom seul ([`Autorite::nom`]) : le chemin n'est jamais affiché.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.nom())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn autorites_par_defaut_de_l_adr() {
        let autorites = Autorite::par_defaut();
        assert_eq!(autorites.len(), 2);
        assert_eq!(autorites[0].nom(), "timestamp.digicert.com");
        assert_eq!(autorites[1].nom(), "timestamp.sectigo.com");
        for a in &autorites {
            assert_eq!(a.port(), 80);
            assert_eq!(a.chemin(), "/");
        }
    }

    #[test]
    fn urls_admises() {
        let a = Autorite::parse(" HTTP://Tsa.Exemple.invalid:8080/tsr/cle-secrete ").unwrap();
        assert_eq!(a.nom(), "tsa.exemple.invalid:8080");
        assert_eq!(a.chemin(), "/tsr/cle-secrete");
        assert_eq!(a.to_string(), "tsa.exemple.invalid:8080");
        assert_eq!(format!("{a:?}"), r#"Autorite("tsa.exemple.invalid:8080")"#);
        assert_eq!(
            Autorite::parse("http://tsa.invalid:65535").unwrap().port(),
            65535
        );
        assert_eq!(Autorite::parse("http://tsa.invalid:1/").unwrap().port(), 1);
        let v6 = Autorite::parse("http://[::1]:3161").unwrap();
        assert_eq!(v6.hote_connexion(), "::1");
        assert_eq!(v6.nom(), "[::1]:3161");
        assert_eq!("http://127.0.0.1/".parse::<Autorite>().unwrap().port(), 80);
    }

    #[test]
    fn urls_refusees() {
        for (texte, attendu) in [
            (
                "https://tsa.exemple.invalid",
                ErreurAutorite::HttpsNonPrisEnCharge,
            ),
            ("ftp://tsa.exemple.invalid", ErreurAutorite::SchemaRefuse),
            ("file:///etc/passwd", ErreurAutorite::Invalide),
            (
                "gopher://tsa.exemple.invalid:70",
                ErreurAutorite::SchemaRefuse,
            ),
            ("tsa.exemple.invalid", ErreurAutorite::Invalide),
            ("/chemin", ErreurAutorite::Invalide),
            ("http://", ErreurAutorite::Invalide),
            ("http://tsa.exemple.invalid:0", ErreurAutorite::Invalide),
            ("http://tsa.exemple.invalid/#f", ErreurAutorite::Invalide),
            ("http://tsa.exemple.invalid:", ErreurAutorite::Invalide),
            ("http://tsa.exemple.invalid:65536", ErreurAutorite::Invalide),
            (
                "http://tsa.exemple.invalid:99999/",
                ErreurAutorite::Invalide,
            ),
            ("http://tsa.exemple.invalid:8o", ErreurAutorite::Invalide),
            ("http://tsa.exemple.invalid:+80", ErreurAutorite::Invalide),
            (
                "http://tsa.exemple.invalid/tsr?cle=1",
                ErreurAutorite::Invalide,
            ),
            ("http://tsa.exemple.invalid?", ErreurAutorite::Invalide),
            (
                "http://u:p@tsa.exemple.invalid",
                ErreurAutorite::IdentifiantsRefuses,
            ),
        ] {
            assert_eq!(Autorite::parse(texte), Err(attendu), "{texte}");
        }
    }

    #[test]
    fn listes() {
        let liste = Autorite::liste("http://a.invalid, http://b.invalid:81 ,").unwrap();
        assert_eq!(
            liste.iter().map(Autorite::nom).collect::<Vec<_>>(),
            ["a.invalid", "b.invalid:81"]
        );
        assert_eq!(Autorite::liste(" , "), Err(ErreurAutorite::ListeVide));
        assert_eq!(
            Autorite::liste("http://a.invalid,ftp://b.invalid"),
            Err(ErreurAutorite::SchemaRefuse)
        );
    }

    #[test]
    fn hotes_internes_refuses_par_la_configuration() {
        for hote in [
            "127.0.0.1",
            "127.1.2.3:8080",
            "10.0.0.1",
            "172.16.5.4",
            "192.168.1.1",
            "169.254.169.254",
            "100.64.0.1",
            "0.0.0.0",
            "255.255.255.255",
            "[::1]",
            "[::]",
            "[fe80::1]",
            "[fd00::1]",
            "[::ffff:127.0.0.1]",
            "[::ffff:10.1.2.3]",
            "localhost",
            "LOCALHOST:3161",
            "tsa.localhost",
        ] {
            let url = format!("http://{hote}");
            assert!(Autorite::parse(&url).is_ok(), "{url}");
            assert_eq!(
                Autorite::liste(&url),
                Err(ErreurAutorite::AdresseInterne),
                "{url}"
            );
        }
        for hote in [
            "93.184.215.14",
            "100.128.0.1",
            "[2606:4700::1111]",
            "tsa.invalid",
        ] {
            assert!(Autorite::liste(&format!("http://{hote}")).is_ok(), "{hote}");
        }
    }
}
