//! Client d'autorité d'horodatage RFC 3161 (ADR 0010 ; menaces F6 ; L2-30).
//!
//! Le client soumet l'empreinte d'un témoin ([`Temoin`]) aux autorités de la configuration,
//! **dans l'ordre**, et renvoie le premier jeton **vérifié**. La requête est construite et la
//! réponse vérifiée par `integrite` (statut, empreinte, nonce, signature, chaîne jusqu'à une
//! ancre) : ce module ne fait que le transport. Une autorité dont la réponse est invalide est
//! traitée comme une panne : on passe à la suivante.
//!
//! Sortie réseau limitée (F6.4) :
//! - destinations : la seule liste des autorités ([`Autorite`], lue dans la configuration) ;
//!   aucune redirection suivie (un 3xx est un échec), aucune URL issue d'une donnée ;
//! - HTTP/1.1 en clair, une connexion par requête, sans mandataire ni réutilisation : seule
//!   l'empreinte du témoin part (F6.3), et le jeton porte sa propre preuve (signature) ;
//! - délai de connexion (résolution du nom comprise) et délai total bornés, par autorité ;
//! - taille de réponse bornée : `Content-Length` excessif refusé avant lecture, corps coupé
//!   au-delà de la limite.
//!
//! Le **nonce** est tiré ici ([`tirer_nonce`], aléa du système), jamais dans `integrite`, qui
//! reste pur.
//!
//! Journaux en liste blanche (F2.11) : une ligne par autorité interrogée, avec `autorite` (hôte
//! et port, jamais le chemin), `statut` HTTP s'il est connu, `duree_ms` et `issue` (liste fermée,
//! [`EchecAutorite::issue`]). Jamais l'empreinte, ni le corps de la requête ou de la réponse.

mod autorite;

use std::time::{Duration, Instant};

use axum::http::{Request, StatusCode, header};
use http_body_util::{BodyExt, Full, LengthLimitError, Limited};
use hyper::body::Bytes;
use hyper::client::conn::http1;
use hyper_util::rt::TokioIo;
use integrite::horodatage::{Ancre, ErreurJeton, ErreurRequete, JetonVerifie, Temoin};
use tokio::net::TcpStream;
use tokio::task::JoinHandle;

pub use autorite::{AUTORITES_PAR_DEFAUT, Autorite, ErreurAutorite};

use crate::serveur::journal::CIBLE;

/// Type de contenu d'une requête RFC 3161 (§3.4).
pub const TYPE_REQUETE: &str = "application/timestamp-query";
/// Type de contenu attendu d'une réponse RFC 3161 (§3.4).
pub const TYPE_REPONSE: &str = "application/timestamp-reply";

/// Tampon de lecture de la connexion (en-têtes compris) : les en-têtes d'une réponse plus grands
/// sont refusés par `hyper`. Minimum admis par `hyper` : 8 Kio.
const TAMPON_MAX: usize = 16 * 1024;

/// Limites d'un échange avec une autorité.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limites {
    /// Délai d'établissement de la connexion TCP, résolution du nom comprise.
    pub connexion: Duration,
    /// Délai total de l'échange avec **une** autorité (connexion, envoi, lecture complète).
    pub total: Duration,
    /// Taille maximale du corps de la réponse, en octets. `integrite` refuse en outre toute
    /// réponse de plus de [`integrite::horodatage::JETON_MAX`] octets.
    pub taille_reponse: usize,
}

impl Default for Limites {
    /// 5 s pour la connexion, 20 s au total, [`integrite::horodatage::JETON_MAX`] (64 Kio) pour
    /// la réponse (les réponses réelles font 5 à 7 Kio).
    fn default() -> Self {
        Self {
            connexion: Duration::from_secs(5),
            total: Duration::from_secs(20),
            taille_reponse: integrite::horodatage::JETON_MAX,
        }
    }
}

/// Échec d'une autorité. Les messages ne citent ni l'empreinte ni le contenu échangé.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EchecAutorite {
    /// Connexion refusée, nom introuvable, échange interrompu ou réponse HTTP illisible.
    #[error("erreur réseau : {0}")]
    Reseau(String),
    /// Connexion non établie dans le délai.
    #[error("délai de connexion dépassé")]
    DelaiConnexion,
    /// Échange non terminé dans le délai total.
    #[error("délai total dépassé")]
    DelaiTotal,
    /// Statut HTTP autre que 200 (une redirection n'est jamais suivie).
    #[error("statut HTTP {0}")]
    StatutHttp(u16),
    /// Type de contenu autre que `application/timestamp-reply`.
    #[error("type de contenu de la réponse inattendu")]
    TypeDeContenu,
    /// Corps de réponse au-delà de la limite (en octets).
    #[error("réponse de plus de {0} octets")]
    ReponseTropGrande(usize),
    /// Réponse refusée par `integrite` (statut, empreinte, nonce, signature, ancre…).
    #[error("réponse invalide : {0}")]
    ReponseInvalide(#[from] ErreurJeton),
}

impl EchecAutorite {
    /// Issue journalisée (liste fermée).
    #[must_use]
    pub fn issue(&self) -> &'static str {
        match self {
            Self::Reseau(_) => "reseau",
            Self::DelaiConnexion => "delai_connexion",
            Self::DelaiTotal => "delai_total",
            Self::StatutHttp(_) => "statut_http",
            Self::TypeDeContenu => "type_de_contenu",
            Self::ReponseTropGrande(_) => "reponse_trop_grande",
            Self::ReponseInvalide(_) => "reponse_invalide",
        }
    }
}

/// Échec de l'horodatage.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ErreurHorodatage {
    /// Requête non encodable (non attendu).
    #[error(transparent)]
    Requete(#[from] ErreurRequete),
    /// Toutes les autorités ont échoué : chacune avec son motif, dans l'ordre d'essai.
    #[error("toutes les autorités d'horodatage ont échoué ({} essai(s))", .0.len())]
    ToutesEnEchec(Vec<(Autorite, EchecAutorite)>),
}

/// Client refusé à la construction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ErreurClient {
    /// Aucune autorité.
    #[error("aucune autorité d'horodatage configurée")]
    AucuneAutorite,
    /// Aucune ancre de confiance : aucun jeton ne pourrait être accepté.
    #[error("aucune ancre de confiance pour vérifier les jetons d'horodatage")]
    AucuneAncre,
}

/// Aléa du système indisponible.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("aléa du système indisponible pour le nonce d'horodatage")]
pub struct ErreurAlea;

/// Tire un nonce de 64 bits de l'aléa du système (`getrandom`).
///
/// # Errors
/// [`ErreurAlea`] si le système ne fournit pas d'aléa.
pub fn tirer_nonce() -> Result<u64, ErreurAlea> {
    getrandom::u64().map_err(|_| ErreurAlea)
}

/// Jeton obtenu et vérifié.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JetonObtenu {
    /// Autorité qui l'a délivré.
    pub autorite: Autorite,
    /// Jeton (`TimeStampToken`, DER), à conserver et à exporter.
    pub jeton: Vec<u8>,
    /// Ce qu'il atteste (date, autorité, politique, ancre).
    pub verifie: JetonVerifie,
}

/// Client des autorités d'horodatage : liste ordonnée, ancres de confiance, limites.
#[derive(Debug, Clone)]
pub struct ClientHorodatage {
    autorites: Vec<Autorite>,
    ancres: Vec<Ancre>,
    limites: Limites,
}

impl ClientHorodatage {
    /// Client pour ces autorités (dans l'ordre d'essai) et ces ancres.
    ///
    /// # Errors
    /// [`ErreurClient`] si la liste des autorités ou celle des ancres est vide.
    pub fn new(
        autorites: Vec<Autorite>,
        ancres: Vec<Ancre>,
        limites: Limites,
    ) -> Result<Self, ErreurClient> {
        if autorites.is_empty() {
            return Err(ErreurClient::AucuneAutorite);
        }
        if ancres.is_empty() {
            return Err(ErreurClient::AucuneAncre);
        }
        Ok(Self {
            autorites,
            ancres,
            limites,
        })
    }

    /// Autorités, dans l'ordre d'essai.
    #[must_use]
    pub fn autorites(&self) -> &[Autorite] {
        &self.autorites
    }

    /// Fait horodater le témoin avec ce nonce : la première autorité qui renvoie un jeton
    /// valide l'emporte. Durée au plus : nombre d'autorités × [`Limites::total`].
    ///
    /// # Errors
    /// [`ErreurHorodatage::ToutesEnEchec`] si aucune autorité n'a fourni de jeton valide.
    pub async fn horodater(
        &self,
        temoin: &Temoin,
        nonce: u64,
    ) -> Result<JetonObtenu, ErreurHorodatage> {
        let corps = Bytes::from(integrite::horodatage::requete(temoin, nonce)?);
        let mut echecs = Vec::with_capacity(self.autorites.len());
        for autorite in &self.autorites {
            let debut = Instant::now();
            let mut statut = None;
            let resultat = match tokio::time::timeout(
                self.limites.total,
                echanger(autorite, corps.clone(), &self.limites, &mut statut),
            )
            .await
            {
                Ok(Ok(reponse)) => {
                    integrite::horodatage::verifier_reponse(&reponse, temoin, nonce, &self.ancres)
                        .map_err(EchecAutorite::from)
                }
                Ok(Err(echec)) => Err(echec),
                Err(_) => Err(EchecAutorite::DelaiTotal),
            };
            journaliser(autorite, statut, debut, resultat.as_ref().err());
            match resultat {
                Ok((jeton, verifie)) => {
                    return Ok(JetonObtenu {
                        autorite: autorite.clone(),
                        jeton,
                        verifie,
                    });
                }
                Err(echec) => echecs.push((autorite.clone(), echec)),
            }
        }
        Err(ErreurHorodatage::ToutesEnEchec(echecs))
    }
}

/// Ligne de journal d'une autorité interrogée (liste blanche, `serveur::journal::CHAMPS`).
fn journaliser(
    autorite: &Autorite,
    statut: Option<u16>,
    debut: Instant,
    echec: Option<&EchecAutorite>,
) {
    let nom = autorite.nom();
    let duree_ms = u64::try_from(debut.elapsed().as_millis()).unwrap_or(u64::MAX);
    match echec {
        None => tracing::info!(
            target: CIBLE,
            autorite = nom.as_str(),
            statut,
            duree_ms,
            issue = "succes",
            "autorité d'horodatage interrogée"
        ),
        Some(echec) => tracing::warn!(
            target: CIBLE,
            autorite = nom.as_str(),
            statut,
            duree_ms,
            issue = echec.issue(),
            "autorité d'horodatage interrogée"
        ),
    }
}

/// Tâche qui pilote la connexion, interrompue quand l'échange se termine ou est abandonné
/// (délai dépassé) : aucune connexion ne survit à son échange.
struct Pilote(JoinHandle<()>);

impl Drop for Pilote {
    fn drop(&mut self) {
        self.0.abort();
    }
}

fn reseau(erreur: impl std::fmt::Display) -> EchecAutorite {
    EchecAutorite::Reseau(erreur.to_string())
}

/// Un échange HTTP avec une autorité : `POST` de la requête, réponse 200 de type
/// `application/timestamp-reply`, corps lu dans la limite. `statut` reçoit le statut HTTP dès
/// qu'il est connu (journal).
async fn echanger(
    autorite: &Autorite,
    corps: Bytes,
    limites: &Limites,
    statut: &mut Option<u16>,
) -> Result<Vec<u8>, EchecAutorite> {
    let flux = tokio::time::timeout(
        limites.connexion,
        TcpStream::connect((autorite.hote_connexion(), autorite.port())),
    )
    .await
    .map_err(|_| EchecAutorite::DelaiConnexion)?
    .map_err(reseau)?;
    let (mut envoi, connexion) = http1::Builder::new()
        .max_buf_size(TAMPON_MAX)
        .handshake::<_, Full<Bytes>>(TokioIo::new(flux))
        .await
        .map_err(reseau)?;
    let _pilote = Pilote(tokio::spawn(async move {
        // L'erreur de connexion éventuelle remonte par `send_request` ou la lecture du corps.
        let _ = connexion.await;
    }));

    let requete = Request::post(autorite.chemin())
        .header(header::HOST, autorite.nom())
        .header(header::CONTENT_TYPE, TYPE_REQUETE)
        .header(header::ACCEPT, TYPE_REPONSE)
        .header(header::USER_AGENT, "sigma-books")
        .body(Full::new(corps))
        .map_err(reseau)?;
    let reponse = envoi.send_request(requete).await.map_err(reseau)?;

    *statut = Some(reponse.status().as_u16());
    if reponse.status() != StatusCode::OK {
        return Err(EchecAutorite::StatutHttp(reponse.status().as_u16()));
    }
    if !type_reponse_attendu(reponse.headers().get(header::CONTENT_TYPE)) {
        return Err(EchecAutorite::TypeDeContenu);
    }
    let annoncee = reponse
        .headers()
        .get(header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse::<u64>().ok());
    if annoncee.is_some_and(|n| usize::try_from(n).map_or(true, |n| n > limites.taille_reponse)) {
        return Err(EchecAutorite::ReponseTropGrande(limites.taille_reponse));
    }
    let lu = Limited::new(reponse.into_body(), limites.taille_reponse)
        .collect()
        .await
        .map_err(|e| {
            if e.is::<LengthLimitError>() {
                EchecAutorite::ReponseTropGrande(limites.taille_reponse)
            } else {
                reseau(e)
            }
        })?;
    Ok(lu.to_bytes().to_vec())
}

/// `Content-Type` égal à [`TYPE_REPONSE`], à la casse et aux paramètres près.
fn type_reponse_attendu(valeur: Option<&header::HeaderValue>) -> bool {
    valeur
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(';').next())
        .is_some_and(|t| t.trim().eq_ignore_ascii_case(TYPE_REPONSE))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    #[test]
    fn type_de_contenu() {
        for (valeur, attendu) in [
            ("application/timestamp-reply", true),
            ("Application/TimeStamp-Reply; charset=binary", true),
            ("application/timestamp-query", false),
            ("application/octet-stream", false),
            ("text/html", false),
        ] {
            assert_eq!(
                type_reponse_attendu(Some(&HeaderValue::from_static(valeur))),
                attendu,
                "{valeur}"
            );
        }
        assert!(!type_reponse_attendu(None));
    }

    #[test]
    fn nonces_tires_distincts() {
        assert_ne!(tirer_nonce().unwrap(), tirer_nonce().unwrap());
    }

    #[test]
    fn client_sans_autorite_ni_ancre_refuse() {
        let ancres = integrite::horodatage::ancres_par_defaut().unwrap();
        assert_eq!(
            ClientHorodatage::new(Vec::new(), ancres, Limites::default()).map(|_| ()),
            Err(ErreurClient::AucuneAutorite)
        );
        assert_eq!(
            ClientHorodatage::new(Autorite::par_defaut(), Vec::new(), Limites::default())
                .map(|_| ()),
            Err(ErreurClient::AucuneAncre)
        );
    }

    #[test]
    fn issues_en_liste_fermee() {
        for echec in [
            EchecAutorite::Reseau(String::new()),
            EchecAutorite::DelaiConnexion,
            EchecAutorite::DelaiTotal,
            EchecAutorite::StatutHttp(500),
            EchecAutorite::TypeDeContenu,
            EchecAutorite::ReponseTropGrande(1),
            EchecAutorite::ReponseInvalide(ErreurJeton::NonceDifferent),
        ] {
            assert!(
                echec
                    .issue()
                    .bytes()
                    .all(|o| o.is_ascii_lowercase() || o == b'_')
            );
        }
    }
}
