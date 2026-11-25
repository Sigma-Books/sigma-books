//! Transport des courriels (ADR 0012, question 8a ; ADR 0024) : une interface étroite
//! ([`Transport`]) et son unique implémentation du binaire, le relais SMTP ([`TransportSmtp`]).
//!
//! - **TLS obligatoire** : `STARTTLS` exigé (la session est refusée si le relais ne l'annonce
//!   pas) ou TLS implicite ; certificat du relais vérifié contre le magasin du système. Aucun mode
//!   sans TLS n'existe, ni par la configuration ni par une fonctionnalité.
//! - **Aucun secret dans les erreurs** : une erreur du transport est réduite à un **code
//!   stable** ([`ErreurEnvoi`]) ; le texte du relais (qui peut citer une adresse) et celui de la
//!   bibliothèque ne sont jamais conservés ni journalisés.
//! - **Hors transaction** : l'envoi se fait après la validation de la transaction qui a émis le
//!   lien (file de tâches, [`super::tache`]).
//!
//! Les tests remplacent le transport par le leur (le trait est public) ; le binaire n'en
//! connaît qu'un (test de structure).

use std::future::Future;
use std::pin::Pin;

use lettre::message::header::ContentType;
use lettre::message::{Mailbox, Message as MessageSmtp};
use lettre::transport::smtp::authentication::Credentials;
use lettre::transport::smtp::extension::ClientId;
use lettre::{AsyncSmtpTransport, AsyncTransport, Tokio1Executor};

use super::config::{ConfigurationSmtp, ModeTls};
use super::modele::Message;

/// Échec d'un envoi : code stable, jamais le texte du relais.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ErreurEnvoi {
    /// Échec provisoire (relais injoignable, délai dépassé, réponse 4xx, coupure) : à rejouer.
    #[error("échec provisoire de l'envoi")]
    Temporaire,
    /// Le TLS n'a pas pu être établi ou vérifié (relais sans STARTTLS, certificat refusé) : rien
    /// n'est envoyé en clair ; à rejouer, l'exploitation doit corriger le relais.
    #[error("TLS indisponible ou refusé")]
    Tls,
    /// Refus définitif (réponse 5xx, authentification refusée, adresse refusée) : sans rejeu.
    #[error("refus définitif de l'envoi")]
    Refus,
    /// Message ou configuration invalide : sans rejeu.
    #[error("message invalide")]
    Invalide,
}

impl ErreurEnvoi {
    /// Code stable inscrit comme issue de la tâche (jamais de texte libre).
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::Temporaire => "smtp_temporaire",
            Self::Tls => "smtp_tls",
            Self::Refus => "smtp_refus",
            Self::Invalide => "message_invalide",
        }
    }

    /// Vrai si l'envoi est à rejouer plus tard.
    #[must_use]
    pub const fn a_rejouer(self) -> bool {
        matches!(self, Self::Temporaire | Self::Tls)
    }
}

/// Futur boxé d'un envoi.
pub type FuturEnvoi<'a> = Pin<Box<dyn Future<Output = Result<(), ErreurEnvoi>> + Send + 'a>>;

/// Remise d'un courriel.
pub trait Transport: Send + Sync + std::fmt::Debug {
    /// Envoie le message.
    fn envoyer<'a>(&'a self, message: &'a Message) -> FuturEnvoi<'a>;
}

/// Relais SMTP, TLS obligatoire.
pub struct TransportSmtp {
    transport: AsyncSmtpTransport<Tokio1Executor>,
    expediteur: Mailbox,
}

impl std::fmt::Debug for TransportSmtp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TransportSmtp(<relais>)")
    }
}

impl TransportSmtp {
    /// Construit le transport d'un relais configuré (aucune connexion à ce stade).
    ///
    /// # Errors
    /// [`ErreurEnvoi::Invalide`] : hôte, expéditeur ou nom hors des formes admises.
    pub fn new(config: &ConfigurationSmtp) -> Result<Self, ErreurEnvoi> {
        let constructeur = match config.tls {
            ModeTls::Starttls => AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&config.hote),
            ModeTls::Implicite => AsyncSmtpTransport::<Tokio1Executor>::relay(&config.hote),
        }
        .map_err(|_| ErreurEnvoi::Invalide)?;
        let mut constructeur = constructeur.port(config.port).timeout(Some(config.delai));
        if let Some(helo) = &config.nom_helo {
            constructeur = constructeur.hello_name(ClientId::Domain(helo.clone()));
        }
        // Limite connue : `Credentials::new` prend des `String` que `lettre` conserve pour la
        // durée du processus, sans les effacer. La copie de `ConfigurationSmtp` est, elle,
        // effacée à la libération ; la copie de la bibliothèque ne peut pas l'être (ADR 0024).
        if let Some(i) = &config.identifiants {
            constructeur = constructeur.credentials(Credentials::new(
                i.utilisateur.clone(),
                i.mot_de_passe.as_str().to_owned(),
            ));
        }
        let adresse = config
            .expediteur
            .parse()
            .map_err(|_| ErreurEnvoi::Invalide)?;
        Ok(Self {
            transport: constructeur.build(),
            expediteur: Mailbox::new(Some(config.nom_expediteur.clone()), adresse),
        })
    }

    fn construire(&self, message: &Message) -> Result<MessageSmtp, ErreurEnvoi> {
        let destinataire = message
            .destinataire
            .parse()
            .map_err(|_| ErreurEnvoi::Invalide)?;
        MessageSmtp::builder()
            .from(self.expediteur.clone())
            .to(Mailbox::new(None, destinataire))
            .subject(message.sujet)
            .header(ContentType::TEXT_PLAIN)
            .body(message.texte.clone())
            .map_err(|_| ErreurEnvoi::Invalide)
    }
}

impl Transport for TransportSmtp {
    fn envoyer<'a>(&'a self, message: &'a Message) -> FuturEnvoi<'a> {
        Box::pin(async move {
            let courriel = self.construire(message)?;
            match self.transport.send(courriel).await {
                Ok(_) => Ok(()),
                Err(e) if e.is_tls() => Err(ErreurEnvoi::Tls),
                Err(e) if e.is_permanent() => Err(ErreurEnvoi::Refus),
                // « STARTTLS non pris en charge » est une erreur du client : rien n'a été envoyé.
                Err(e) if e.is_client() => Err(ErreurEnvoi::Tls),
                Err(_) => Err(ErreurEnvoi::Temporaire),
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn les_codes_sont_stables_et_sans_texte_libre() {
        for e in [
            ErreurEnvoi::Temporaire,
            ErreurEnvoi::Tls,
            ErreurEnvoi::Refus,
            ErreurEnvoi::Invalide,
        ] {
            assert!(e.code().chars().all(|c| c.is_ascii_lowercase() || c == '_'));
            assert!(e.code().len() <= 40);
        }
        assert!(ErreurEnvoi::Temporaire.a_rejouer() && ErreurEnvoi::Tls.a_rejouer());
        assert!(!ErreurEnvoi::Refus.a_rejouer() && !ErreurEnvoi::Invalide.a_rejouer());
    }
}
