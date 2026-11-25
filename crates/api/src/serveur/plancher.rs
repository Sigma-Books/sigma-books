//! Attente de la durée plancher des échecs de connexion (anti-énumération, ADR 0017).
//!
//! La réponse d'un échec part au plus tôt à `arrivée + durée plancher`, quel que soit le chemin
//! suivi. L'attente est derrière un trait pour que les tests vérifient **sans mesurer le temps**
//! que chaque chemin d'échec passe par elle, avec la même durée : une comparaison de durées
//! réelles sous charge n'est pas déterministe (le chemin le plus long peut dépasser le plancher
//! sur un exécuteur saturé). Le binaire n'utilise que [`AttenteTokio`].

use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use tokio::time::Instant;

/// Futur rendu par [`AttentePlancher::attendre`].
pub type FuturAttente<'a> = Pin<Box<dyn Future<Output = ()> + Send + 'a>>;

/// Attend que la durée plancher écoulée depuis `debut` soit atteinte.
pub trait AttentePlancher: fmt::Debug + Send + Sync + 'static {
    /// Rend la main à `debut + plancher` au plus tôt.
    fn attendre(&self, debut: Instant, plancher: Duration) -> FuturAttente<'_>;
}

/// Attente réelle (horloge de tokio) : celle du binaire.
#[derive(Debug, Clone, Copy, Default)]
pub struct AttenteTokio;

impl AttentePlancher for AttenteTokio {
    fn attendre(&self, debut: Instant, plancher: Duration) -> FuturAttente<'_> {
        Box::pin(tokio::time::sleep_until(debut + plancher))
    }
}

/// Attente partagée des réglages ; égalité par identité de l'instance.
#[derive(Clone)]
pub struct Plancher(Arc<dyn AttentePlancher>);

impl Plancher {
    /// Enveloppe une attente (tests : attente enregistrée, sans délai).
    pub fn new(attente: Arc<dyn AttentePlancher>) -> Self {
        Self(attente)
    }

    pub(crate) fn attendre(&self, debut: Instant, plancher: Duration) -> FuturAttente<'_> {
        self.0.attendre(debut, plancher)
    }
}

impl Default for Plancher {
    fn default() -> Self {
        Self(Arc::new(AttenteTokio))
    }
}

impl fmt::Debug for Plancher {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl PartialEq for Plancher {
    fn eq(&self, autre: &Self) -> bool {
        std::ptr::addr_eq(Arc::as_ptr(&self.0), Arc::as_ptr(&autre.0))
    }
}

impl Eq for Plancher {}
