//! Serveur applicatif de Sigma Books.
//!
//! Minimal : configuration lue dans l'environnement (refus de démarrer si elle est incomplète),
//! politiques Cedar chargées et validées, pool du rôle applicatif (refus d'un rôle trop
//! privilégié), puis service HTTP jusqu'à `Ctrl-C`. Authentification : [`RefusTotal`] jusqu'aux
//! sessions (PR 8) ; aucune autre n'est sélectionnable.

#![forbid(unsafe_code)]

use std::process::ExitCode;
use std::sync::Arc;

use api::base::{ReglagesPool, ouvrir_pool_application};
use api::serveur::config::Configuration;
use api::serveur::{Application, RefusTotal, journal, routeur, servir};
use authz::Autorisation;
use tokio::net::TcpListener;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

#[tokio::main]
async fn main() -> ExitCode {
    // Une panique (capturée par le serveur : 500 uniforme) ne recopie pas son message, qui
    // pourrait citer des données (F2.11) : seul son emplacement dans le code est écrit.
    std::panic::set_hook(Box::new(|info| {
        let lieu = info
            .location()
            .map_or_else(String::new, |l| format!(" ({}:{})", l.file(), l.line()));
        eprintln!("api : panique{lieu}");
    }));
    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer().with_target(false))
        .with(journal::filtre())
        .init();
    match demarrer().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("api : {message}");
            ExitCode::FAILURE
        }
    }
}

async fn demarrer() -> Result<(), String> {
    let config = Configuration::depuis_environnement().map_err(|e| e.to_string())?;
    let autorisation = Autorisation::charger().map_err(|e| e.to_string())?;
    // Le message d'une erreur de connexion ne cite pas l'URL (ni son mot de passe).
    let pool = ouvrir_pool_application(&config.url_base, ReglagesPool::default())
        .await
        .map_err(|e| format!("base de données : {e}"))?;
    let application = Application::new(
        pool,
        Arc::new(autorisation),
        Arc::new(RefusTotal),
        config.reglages.clone(),
    );
    let routeur = routeur(&application);
    let ecouteur = TcpListener::bind(config.adresse_ecoute)
        .await
        .map_err(|e| format!("écoute impossible : {e}"))?;
    tracing::info!(
        target: journal::CIBLE,
        adresse_ecoute = %config.adresse_ecoute,
        "serveur démarré"
    );
    servir(ecouteur, routeur, &config.reglages, async {
        // Sans gestionnaire de signal, on ne s'arrête que sur fermeture du processus.
        if tokio::signal::ctrl_c().await.is_err() {
            std::future::pending::<()>().await;
        }
    })
    .await
    .map_err(|e| format!("service interrompu : {e}"))
}
