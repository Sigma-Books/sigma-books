//! Stockage des pièces sur un service objet **compatible S3** (ADR 0015).
//!
//! - Signature AWS v4 par `rusty-s3` (sans E/S) : chaque requête est une URL présignée à durée
//!   très courte ([`VALIDITE_SIGNATURE`]), émise par le serveur seul vers le service ; aucune URL
//!   n'est jamais remise à un client (F3.2). Transport : le client HTTP/1.1 de `hyper`, une
//!   connexion par requête, sans redirection, réponse lue dans une limite.
//! - **WORM** : [`StockageS3::verifier_worm`] exige, au démarrage, le versionnement **et** le
//!   verrouillage d'objet du compartiment ; le binaire refuse de démarrer sinon. Chaque dépôt
//!   pose une rétention en mode **gouvernance** (`x-amz-object-lock-mode: GOVERNANCE`), refuse
//!   d'écraser une clé existante (`If-None-Match: *`) et fait vérifier par le service la SHA-256
//!   calculée par le serveur (`x-amz-checksum-sha256`). La lecture vise la **version** écrite
//!   (`versionId`) : une version déposée plus tard sous la même clé n'est jamais servie.
//! - **HTTP seul** en L2, comme le client d'horodatage : le service est local ou joint par un
//!   relais TLS de l'hébergement. Un client TLS sera choisi avec l'hébergement (L9).
//!
//! Les identifiants viennent de l'environnement d'exécution ([`super::config`]) ; ni eux ni les
//! URL signées n'apparaissent dans un message d'erreur ou un journal.

use std::fmt;
use std::time::Duration;

use axum::body::Bytes;
use axum::http::{HeaderMap, Method, Request, StatusCode, header};
use base64ct::{Base64, Encoding};
use http_body_util::{BodyExt, Full, LengthLimitError, Limited};
use hyper::client::conn::http1;
use hyper_util::rt::TokioIo;
use rusty_s3::actions::{DeleteObject, GetObject, HeadObject, PutObject};
use rusty_s3::{Bucket, Credentials, S3Action, UrlStyle};
use time::OffsetDateTime;
use tokio::net::TcpStream;
use url::Url;

use super::stockage::{CleObjet, Depot, ErreurStockage, Futur, ObjetADeposer, Stockage};

/// Durée de validité d'une signature : la requête part aussitôt.
pub const VALIDITE_SIGNATURE: Duration = Duration::from_secs(60);
/// Délai d'établissement de la connexion.
const DELAI_CONNEXION: Duration = Duration::from_secs(10);
/// Délai d'un échange complet (20 Mio sur un réseau local ou d'hébergement).
const DELAI_ECHANGE: Duration = Duration::from_secs(120);
/// Taille maximale d'une réponse de configuration du compartiment.
const TAILLE_REPONSE_CONFIGURATION: usize = 64 * 1024;

/// Accès au compartiment : point d'accès, nom, région, style d'URL, identifiants.
#[derive(Clone, PartialEq, Eq)]
pub struct ConfigurationS3 {
    /// Point d'accès `http://hôte[:port][/préfixe]`.
    pub point_acces: Url,
    /// Nom du compartiment.
    pub compartiment: String,
    /// Région (signature).
    pub region: String,
    /// Compartiment dans le chemin (`true`, `MinIO`) ou dans le nom d'hôte.
    pub style_chemin: bool,
    /// Identifiants du rôle applicatif (sans droit de lever un verrou).
    pub identifiants: Credentials,
}

impl fmt::Debug for ConfigurationS3 {
    /// Hôte, compartiment et région seulement : ni chemin, ni identifiants.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ConfigurationS3")
            .field("hote", &self.point_acces.host_str())
            .field("port", &self.point_acces.port_or_known_default())
            .field("compartiment", &self.compartiment)
            .field("region", &self.region)
            .field("style_chemin", &self.style_chemin)
            .finish_non_exhaustive()
    }
}

/// Stockage compatible S3.
pub struct StockageS3 {
    compartiment: Bucket,
    identifiants: Credentials,
}

impl fmt::Debug for StockageS3 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StockageS3")
            .field("compartiment", &self.compartiment.name())
            .finish_non_exhaustive()
    }
}

/// Configuration de compartiment refusée.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("configuration du stockage objet invalide")]
pub struct ErreurConfigurationS3;

impl StockageS3 {
    /// Stockage sur ce compartiment. Le point d'accès doit être en `http` (voir le module).
    ///
    /// # Errors
    /// [`ErreurConfigurationS3`] si le point d'accès ou le nom du compartiment sont refusés.
    pub fn new(config: ConfigurationS3) -> Result<Self, ErreurConfigurationS3> {
        if config.point_acces.scheme() != "http" || config.point_acces.host_str().is_none() {
            return Err(ErreurConfigurationS3);
        }
        let style = if config.style_chemin {
            UrlStyle::Path
        } else {
            UrlStyle::VirtualHost
        };
        let compartiment = Bucket::new(
            config.point_acces,
            style,
            config.compartiment,
            config.region,
        )
        .map_err(|_| ErreurConfigurationS3)?;
        Ok(Self {
            compartiment,
            identifiants: config.identifiants,
        })
    }

    /// Vérifie que le compartiment a le **versionnement activé** et le **verrouillage
    /// d'objet** (ADR 0015) ; le binaire refuse de démarrer sinon.
    ///
    /// # Errors
    /// [`ErreurStockage::SansVersionnement`], [`ErreurStockage::SansVerrouillage`], ou échec
    /// de l'échange.
    pub async fn verifier_worm(&self) -> Result<(), ErreurStockage> {
        let versionnement = self.configuration_du_compartiment("versioning").await?;
        if !versionnement.contains("<Status>Enabled</Status>") {
            return Err(ErreurStockage::SansVersionnement);
        }
        let verrou = match self.configuration_du_compartiment("object-lock").await {
            Ok(texte) => texte,
            // Compartiment créé sans verrouillage : `ObjectLockConfigurationNotFoundError`.
            Err(ErreurStockage::Refus(404) | ErreurStockage::Absent) => {
                return Err(ErreurStockage::SansVerrouillage);
            }
            Err(e) => return Err(e),
        };
        if !verrou.contains("<ObjectLockEnabled>Enabled</ObjectLockEnabled>") {
            return Err(ErreurStockage::SansVerrouillage);
        }
        self.verifier_sans_contournement().await
    }

    /// Dépose un objet **sentinelle** (`sentinelle/<UUID>`, quelques octets, rétention de
    /// gouvernance d'un jour), puis tente de supprimer sa version en **contournant** le mode
    /// gouvernance. Le service doit refuser : sinon les identifiants du serveur ont
    /// `s3:BypassGovernanceRetention` (ou sont ceux de l'administrateur) et le binaire refuse de
    /// démarrer. Chaque démarrage laisse donc une sentinelle, effaçable après un jour.
    ///
    /// # Errors
    /// [`ErreurStockage::ContournementPossible`], ou échec du dépôt de la sentinelle.
    pub async fn verifier_sans_contournement(&self) -> Result<(), ErreurStockage> {
        let cle = CleObjet::sentinelle();
        let contenu: &[u8] = b"sigma-books sentinelle du verrou\n";
        let depot = self
            .deposer(ObjetADeposer {
                cle: &cle,
                contenu,
                empreinte: integrite::Empreinte::calculer(contenu),
                type_contenu: "text/plain; charset=utf-8",
                retenir_jusqu_au: OffsetDateTime::now_utc() + time::Duration::DAY,
            })
            .await?;
        let entetes = [("x-amz-bypass-governance-retention", "true".to_owned())];
        let mut action =
            DeleteObject::new(&self.compartiment, Some(&self.identifiants), cle.as_str());
        action
            .query_mut()
            .insert("versionId", depot.version.as_str());
        for (nom, valeur) in &entetes {
            action.headers_mut().insert(*nom, valeur.clone());
        }
        let url = action.sign(VALIDITE_SIGNATURE);
        let reponse = echanger(
            Method::DELETE,
            &url,
            &entetes,
            Bytes::new(),
            TAILLE_REPONSE_CONFIGURATION,
        )
        .await?;
        if reponse.statut.is_success() {
            Err(ErreurStockage::ContournementPossible)
        } else if reponse.statut.is_client_error() {
            Ok(())
        } else {
            Err(ErreurStockage::Refus(reponse.statut.as_u16()))
        }
    }

    /// Lit une sous-ressource de configuration du compartiment (`?versioning`,
    /// `?object-lock`) : un `GET` signé sur la racine du compartiment. Seule la présence d'un
    /// élément attendu est recherchée dans la réponse (texte du service configuré, borné).
    async fn configuration_du_compartiment(
        &self,
        sous_ressource: &'static str,
    ) -> Result<String, ErreurStockage> {
        let mut action = GetObject::new(&self.compartiment, Some(&self.identifiants), "");
        action.query_mut().insert(sous_ressource, "");
        let url = action.sign(VALIDITE_SIGNATURE);
        let reponse = echanger(
            Method::GET,
            &url,
            &[],
            Bytes::new(),
            TAILLE_REPONSE_CONFIGURATION,
        )
        .await?;
        match reponse.statut {
            StatusCode::OK => {
                String::from_utf8(reponse.corps).map_err(|_| ErreurStockage::ReponseInattendue)
            }
            statut => Err(ErreurStockage::Refus(statut.as_u16())),
        }
    }
}

/// Contrôle les en-têtes de verrou d'une version relue (`HEAD`) : mode `GOVERNANCE` et fin de
/// rétention au moins égale à `attendu` (à la seconde).
///
/// # Errors
/// [`ErreurStockage::RetentionNonPosee`].
pub fn verifier_retention(
    entetes: &HeaderMap,
    attendu: OffsetDateTime,
) -> Result<(), ErreurStockage> {
    let texte = |nom: &str| entetes.get(nom).and_then(|v| v.to_str().ok());
    let mode = texte("x-amz-object-lock-mode");
    let fin = texte("x-amz-object-lock-retain-until-date").and_then(lire_instant_s3);
    let attendu = attendu.replace_nanosecond(0).unwrap_or(attendu);
    match (mode, fin) {
        (Some("GOVERNANCE"), Some(fin)) if fin >= attendu => Ok(()),
        _ => Err(ErreurStockage::RetentionNonPosee),
    }
}

/// Lit un instant S3 `AAAA-MM-JJTHH:MM:SS[.fff…]Z` (UTC), à la seconde.
fn lire_instant_s3(texte: &str) -> Option<OffsetDateTime> {
    let o = texte.as_bytes();
    let valide = o.len() >= 20
        && o[10] == b'T'
        && o[13] == b':'
        && o[16] == b':'
        && texte.ends_with('Z')
        && (o.len() == 20 || o[19] == b'.');
    if !valide {
        return None;
    }
    let date = super::interne::lire_date(texte.get(..10)?).ok()?;
    let nombre = |a: usize| -> Option<u8> {
        let t = texte.get(a..a + 2)?;
        t.bytes()
            .all(|b| b.is_ascii_digit())
            .then(|| t.parse().ok())?
    };
    let heure = time::Time::from_hms(nombre(11)?, nombre(14)?, nombre(17)?).ok()?;
    Some(date.with_time(heure).assume_utc())
}

/// Instant de fin de rétention au format attendu par S3 (`AAAA-MM-JJTHH:MM:SSZ`, UTC).
#[must_use]
pub fn instant_s3(instant: OffsetDateTime) -> String {
    let i = instant.to_offset(time::UtcOffset::UTC);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        i.year(),
        u8::from(i.month()),
        i.day(),
        i.hour(),
        i.minute(),
        i.second()
    )
}

impl Stockage for StockageS3 {
    fn deposer<'a>(&'a self, objet: ObjetADeposer<'a>) -> Futur<'a, Result<Depot, ErreurStockage>> {
        Box::pin(async move {
            let somme = Base64::encode_string(objet.empreinte.octets());
            let retenir = instant_s3(objet.retenir_jusqu_au);
            // En-têtes signés (triés par nom) : le service refuse une requête qui les altère.
            let entetes = [
                ("content-type", objet.type_contenu.to_owned()),
                ("if-none-match", "*".to_owned()),
                ("x-amz-checksum-sha256", somme),
                ("x-amz-object-lock-mode", "GOVERNANCE".to_owned()),
                ("x-amz-object-lock-retain-until-date", retenir),
            ];
            let mut action = PutObject::new(
                &self.compartiment,
                Some(&self.identifiants),
                objet.cle.as_str(),
            );
            for (nom, valeur) in &entetes {
                action.headers_mut().insert(*nom, valeur.clone());
            }
            let url = action.sign(VALIDITE_SIGNATURE);
            let reponse = echanger(
                Method::PUT,
                &url,
                &entetes,
                Bytes::copy_from_slice(objet.contenu),
                TAILLE_REPONSE_CONFIGURATION,
            )
            .await?;
            if reponse.statut != StatusCode::OK {
                return Err(ErreurStockage::Refus(reponse.statut.as_u16()));
            }
            // Versionnement obligatoire : sans version rendue, l'objet n'est pas protégé.
            let version = reponse
                .entetes
                .get("x-amz-version-id")
                .and_then(|v| v.to_str().ok())
                .filter(|v| !v.is_empty() && *v != "null" && v.len() <= 1024)
                .ok_or(ErreurStockage::ReponseInattendue)?
                .to_owned();
            // Rétention relue sur la version écrite : mode gouvernance et fin au moins égale à
            // celle demandée ; sinon l'objet n'est pas inscrit en base (ADR 0015).
            let mut action = HeadObject::new(
                &self.compartiment,
                Some(&self.identifiants),
                objet.cle.as_str(),
            );
            action.query_mut().insert("versionId", version.as_str());
            let url = action.sign(VALIDITE_SIGNATURE);
            let tete = echanger(Method::HEAD, &url, &[], Bytes::new(), objet.contenu.len()).await?;
            if tete.statut != StatusCode::OK {
                return Err(ErreurStockage::Refus(tete.statut.as_u16()));
            }
            verifier_retention(&tete.entetes, objet.retenir_jusqu_au)?;
            Ok(Depot { version })
        })
    }

    fn lire<'a>(
        &'a self,
        cle: &'a CleObjet,
        version: &'a str,
        taille_max: usize,
    ) -> Futur<'a, Result<Vec<u8>, ErreurStockage>> {
        Box::pin(async move {
            let mut action =
                GetObject::new(&self.compartiment, Some(&self.identifiants), cle.as_str());
            action.query_mut().insert("versionId", version);
            let url = action.sign(VALIDITE_SIGNATURE);
            let reponse = echanger(Method::GET, &url, &[], Bytes::new(), taille_max).await?;
            match reponse.statut {
                StatusCode::OK => Ok(reponse.corps),
                StatusCode::NOT_FOUND => Err(ErreurStockage::Absent),
                statut => Err(ErreurStockage::Refus(statut.as_u16())),
            }
        })
    }
}

/// Réponse lue d'un échange.
struct Reponse {
    statut: StatusCode,
    entetes: HeaderMap,
    corps: Vec<u8>,
}

/// Arrête la tâche qui pilote la connexion quand l'échange se termine ou est abandonné.
struct Pilote(tokio::task::JoinHandle<()>);

impl Drop for Pilote {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// Un échange HTTP/1.1 avec le service, dans le délai [`DELAI_ECHANGE`] ; corps de la
/// réponse lu dans la limite `taille_max` (au-delà : [`ErreurStockage::TropGrand`]).
async fn echanger(
    methode: Method,
    url: &Url,
    entetes: &[(&str, String)],
    corps: Bytes,
    taille_max: usize,
) -> Result<Reponse, ErreurStockage> {
    tokio::time::timeout(
        DELAI_ECHANGE,
        echanger_sans_delai(methode, url, entetes, corps, taille_max),
    )
    .await
    .map_err(|_| ErreurStockage::Injoignable)?
}

async fn echanger_sans_delai(
    methode: Method,
    url: &Url,
    entetes: &[(&str, String)],
    corps: Bytes,
    taille_max: usize,
) -> Result<Reponse, ErreurStockage> {
    let hote = url.host_str().ok_or(ErreurStockage::Injoignable)?;
    let port = url
        .port_or_known_default()
        .ok_or(ErreurStockage::Injoignable)?;
    let hote_connexion = hote
        .strip_prefix('[')
        .and_then(|h| h.strip_suffix(']'))
        .unwrap_or(hote);
    let flux = tokio::time::timeout(DELAI_CONNEXION, TcpStream::connect((hote_connexion, port)))
        .await
        .map_err(|_| ErreurStockage::Injoignable)?
        .map_err(|_| ErreurStockage::Injoignable)?;
    let (mut envoi, connexion) = http1::Builder::new()
        .handshake::<_, Full<Bytes>>(TokioIo::new(flux))
        .await
        .map_err(|_| ErreurStockage::Injoignable)?;
    let _pilote = Pilote(tokio::spawn(async move {
        let _ = connexion.await;
    }));
    // En-tête `Host` : celui que la signature a couvert (port seulement s'il n'est pas 80).
    let entete_hote = match url.port() {
        Some(p) => format!("{hote}:{p}"),
        None => hote.to_owned(),
    };
    let cible = match url.query() {
        Some(q) => format!("{}?{q}", url.path()),
        None => url.path().to_owned(),
    };
    let mut requete = Request::builder()
        .method(methode)
        .uri(cible)
        .header(header::HOST, entete_hote)
        .header(header::USER_AGENT, "sigma-books");
    for (nom, valeur) in entetes {
        requete = requete.header(*nom, valeur);
    }
    let requete = requete
        .body(Full::new(corps))
        .map_err(|_| ErreurStockage::ReponseInattendue)?;
    let reponse = envoi
        .send_request(requete)
        .await
        .map_err(|_| ErreurStockage::Injoignable)?;
    let statut = reponse.status();
    let entetes = reponse.headers().clone();
    let annoncee = entetes
        .get(header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse::<u64>().ok());
    if statut == StatusCode::OK
        && annoncee.is_some_and(|n| usize::try_from(n).map_or(true, |n| n > taille_max))
    {
        return Err(ErreurStockage::TropGrand);
    }
    // Une réponse d'erreur n'est lue que dans une petite limite : son texte n'est pas utilisé.
    let limite = if statut == StatusCode::OK {
        taille_max
    } else {
        TAILLE_REPONSE_CONFIGURATION
    };
    let lu = Limited::new(reponse.into_body(), limite)
        .collect()
        .await
        .map_err(|e| {
            if e.is::<LengthLimitError>() {
                ErreurStockage::TropGrand
            } else {
                ErreurStockage::Injoignable
            }
        })?;
    Ok(Reponse {
        statut,
        entetes,
        corps: lu.to_bytes().to_vec(),
    })
}

#[cfg(test)]
mod tests {
    use time::macros::datetime;

    use super::*;

    fn config(point_acces: &str) -> ConfigurationS3 {
        ConfigurationS3 {
            point_acces: point_acces.parse().unwrap(),
            compartiment: "sigma-pieces".to_owned(),
            region: "us-east-1".to_owned(),
            style_chemin: true,
            identifiants: Credentials::new("cle-exemple", "secret-exemple"),
        }
    }

    #[test]
    fn seul_http_est_admis() {
        assert!(StockageS3::new(config("http://127.0.0.1:9100")).is_ok());
        assert!(StockageS3::new(config("https://s3.exemple.invalid")).is_err());
        assert!(StockageS3::new(config("ftp://s3.exemple.invalid")).is_err());
    }

    #[test]
    fn le_debug_ne_montre_aucun_secret() {
        let c = config("http://127.0.0.1:9100/prefixe-secret");
        let texte = format!("{c:?} {:?}", StockageS3::new(c.clone()).unwrap());
        assert!(!texte.contains("secret"), "{texte}");
        assert!(texte.contains("sigma-pieces"), "{texte}");
    }

    #[test]
    fn retention_relue() {
        let entetes = |mode: &str, fin: &str| {
            let mut h = HeaderMap::new();
            h.insert("x-amz-object-lock-mode", mode.parse().unwrap());
            h.insert("x-amz-object-lock-retain-until-date", fin.parse().unwrap());
            h
        };
        let attendu = datetime!(2036-11-19 08:05:03.4 UTC);
        assert_eq!(
            verifier_retention(&entetes("GOVERNANCE", "2036-11-19T08:05:03Z"), attendu),
            Ok(())
        );
        assert_eq!(
            verifier_retention(&entetes("GOVERNANCE", "2036-11-19T08:05:04.000Z"), attendu),
            Ok(())
        );
        for (mode, fin) in [
            ("COMPLIANCE", "2036-11-19T08:05:03Z"),
            ("GOVERNANCE", "2036-11-19T08:05:02Z"),
            ("GOVERNANCE", "2026-11-19T08:05:03Z"),
            ("GOVERNANCE", "pas une date"),
        ] {
            assert_eq!(
                verifier_retention(&entetes(mode, fin), attendu),
                Err(ErreurStockage::RetentionNonPosee),
                "{mode} {fin}"
            );
        }
        assert_eq!(
            verifier_retention(&HeaderMap::new(), attendu),
            Err(ErreurStockage::RetentionNonPosee)
        );
    }

    #[test]
    fn instant_au_format_s3() {
        assert_eq!(
            instant_s3(datetime!(2036-11-19 08:05:03 UTC)),
            "2036-11-19T08:05:03Z"
        );
    }
}
