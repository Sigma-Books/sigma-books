//! Autorité d'horodatage : URL lue dans la **configuration** seulement (ADR 0010).
//!
//! Une [`Autorite`] ne se construit que par [`Autorite::parse`], appelée sur la configuration
//! du binaire : aucune URL issue d'une donnée (requête, base, réponse d'une autorité) ne peut
//! devenir une destination. Le client n'en suit aucune redirection.

use std::fmt;
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
}

/// Autorité d'horodatage : hôte, port et chemin, en HTTP.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Autorite {
    /// Hôte en minuscules ; une adresse IPv6 garde ses crochets.
    hote: String,
    port: u16,
    /// Chemin et requête (`/` par défaut).
    chemin: String,
}

impl Autorite {
    /// Lit une URL d'autorité : `http://hôte[:port][/chemin]`, sans identifiants ni fragment.
    ///
    /// # Errors
    /// [`ErreurAutorite`] si l'URL est invalide, d'un autre schéma que `http`, ou porte des
    /// identifiants.
    pub fn parse(texte: &str) -> Result<Self, ErreurAutorite> {
        let texte = texte.trim();
        if texte.contains('#') {
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
        let port = match autorite.port_u16() {
            Some(0) => return Err(ErreurAutorite::Invalide),
            Some(port) => port,
            None => 80,
        };
        let chemin = match uri
            .path_and_query()
            .map(axum::http::uri::PathAndQuery::as_str)
        {
            None | Some("") => "/".to_owned(),
            Some(c) => c.to_owned(),
        };
        Ok(Self { hote, port, chemin })
    }

    /// Lit une liste d'URL séparées par des virgules, dans l'ordre d'essai.
    ///
    /// # Errors
    /// [`ErreurAutorite`] si une URL est refusée ou si la liste est vide.
    pub fn liste(texte: &str) -> Result<Vec<Self>, ErreurAutorite> {
        let autorites = texte
            .split(',')
            .filter(|t| !t.trim().is_empty())
            .map(Self::parse)
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
        let a = Autorite::parse(" HTTP://Tsa.Exemple.invalid:8080/tsr?x=1 ").unwrap();
        assert_eq!(a.nom(), "tsa.exemple.invalid:8080");
        assert_eq!(a.chemin(), "/tsr?x=1");
        assert_eq!(a.to_string(), "tsa.exemple.invalid:8080");
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
}
