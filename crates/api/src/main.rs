//! Serveur applicatif de Sigma Books.
//!
//! Minimal : configuration lue dans l'environnement (refus de démarrer si elle est incomplète),
//! politiques Cedar chargées et validées, pools du rôle applicatif et du rôle de lecture
//! d'audit (refus d'un rôle trop privilégié), puis service HTTP jusqu'à `Ctrl-C`.
//! Authentification : sessions lues dans PostgreSQL ([`AuthentificationSession`], ADR 0012) ;
//! aucune autre n'est sélectionnable.

#![forbid(unsafe_code)]

use std::process::ExitCode;
use std::sync::Arc;

use api::base::{ReglagesPool, ouvrir_pool_application, ouvrir_pool_lecture_audit};
use api::horodatage::ClientHorodatage;
use api::horodatage::tache::Travailleur;
use api::pieces::Pieces;
use api::pieces::s3::StockageS3;
use api::serveur::config::Configuration;
use api::serveur::{
    Application, AuthentificationSession, HorlogeSysteme, journal, routeur, servir,
};
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

/// Commande d'administration de l'instance (`api creer-cabinet`, `api reinitialiser-acces`) :
/// elle ne lit que la base et l'origine publique, puis rend la main sans démarrer le serveur.
async fn commande(commande: api::instance::Commande) -> Result<(), String> {
    let env = |nom: &str| std::env::var(nom).ok().filter(|v| !v.trim().is_empty());
    let url = env("SIGMA_URL_BASE_APPLICATION").ok_or("SIGMA_URL_BASE_APPLICATION absente")?;
    let origine = env("SIGMA_ORIGINE").ok_or("SIGMA_ORIGINE absente")?;
    let pool = ouvrir_pool_application(&url, ReglagesPool::default())
        .await
        .map_err(|e| format!("base de données : {e}"))?;
    let sortie =
        api::instance::executer(&pool, &commande, &origine, time::OffsetDateTime::now_utc())
            .await
            .map_err(|e| e.to_string())?;
    for ligne in sortie.lignes {
        println!("{ligne}");
    }
    Ok(())
}

async fn demarrer() -> Result<(), String> {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    if let Some(c) = api::instance::lire_commande(&arguments).map_err(|e| e.to_string())? {
        return commande(c).await;
    }
    let config = Configuration::depuis_environnement().map_err(|e| e.to_string())?;
    let autorisation = Autorisation::charger().map_err(|e| e.to_string())?;
    // Référentiel officiel copié dans chaque nouveau dossier (voie comptabilité, PR 11) :
    // invalide, le serveur ne démarre pas.
    api::base::referentiel::Modele::officiel().map_err(|e| e.to_string())?;
    // Régimes et motifs de TVA de `fiscal-sn` (étiquette fiscale du brouillard, PR 13a).
    api::serveur::routes::brouillard::referentiel_tva().map_err(|e| e.to_string())?;
    // Le message d'une erreur de connexion ne cite pas l'URL (ni son mot de passe).
    let pool = ouvrir_pool_application(&config.base.url_application, ReglagesPool::default())
        .await
        .map_err(|e| format!("base de données : {e}"))?;
    let pool_lecture_audit =
        ouvrir_pool_lecture_audit(&config.base.url_lecture_audit, ReglagesPool::default())
            .await
            .map_err(|e| format!("base de données (lecture d'audit) : {e}"))?;
    // Voie intégrité et horodatage (PR 15) : tâche quotidienne de la file, dans ce processus
    // (ADR 0010). Les ancres de confiance sont intégrées ; refus de démarrer si elles sont
    // illisibles ou si aucune autorité n'est configurée.
    let ancres = integrite::horodatage::ancres_par_defaut().map_err(|e| e.to_string())?;
    let client = ClientHorodatage::new(
        config.autorites_horodatage.clone(),
        ancres,
        api::horodatage::Limites::default(),
    )
    .map_err(|e| e.to_string())?;
    let travailleur = Travailleur::new(
        pool.clone(),
        Arc::new(client),
        Arc::new(HorlogeSysteme),
        api::horodatage::tache::Reglages::default(),
    );
    let authentification = AuthentificationSession::new(
        pool.clone(),
        Arc::new(HorlogeSysteme),
        config.reglages.sessions,
    );
    let application = Application::new(
        pool,
        pool_lecture_audit,
        Arc::new(autorisation),
        Arc::new(authentification),
        config.reglages.clone(),
    );
    // Voie pièces (PR 12) : stockage objet WORM obligatoire, vérifié avant de servir
    // (ADR 0015). Le binaire ne connaît que le stockage S3.
    let stockage = StockageS3::new(config.pieces.s3.clone()).map_err(|e| e.to_string())?;
    stockage
        .verifier_worm()
        .await
        .map_err(|e| format!("pièces : {e} ; le serveur refuse de démarrer (ADR 0015)"))?;
    if application
        .brancher_pieces(Pieces::new(Arc::new(stockage), config.pieces.reglages))
        .is_err()
    {
        return Err("pièces : service déjà branché".to_owned());
    }
    let routeur = routeur(&application);
    let ecouteur = TcpListener::bind(config.adresse_ecoute)
        .await
        .map_err(|e| format!("écoute impossible : {e}"))?;
    tracing::info!(
        target: journal::CIBLE,
        adresse_ecoute = %config.adresse_ecoute,
        "serveur démarré"
    );
    let boucle = tokio::spawn(async move { travailleur.superviser().await });
    let service = servir(ecouteur, routeur, &config.reglages, async {
        // Sans gestionnaire de signal, on ne s'arrête que sur fermeture du processus.
        if tokio::signal::ctrl_c().await.is_err() {
            std::future::pending::<()>().await;
        }
    })
    .await;
    boucle.abort();
    service.map_err(|e| format!("service interrompu : {e}"))
}
