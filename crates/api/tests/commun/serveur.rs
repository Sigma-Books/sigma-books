//! Aides des tests du serveur : authentification et décision **de test**, appel du routeur,
//! capture des journaux.
//!
//! [`AuthentificationTest`] n'existe que dans `crates/api/tests/` : elle n'est compilée que pour
//! les tests et le binaire ne peut pas la sélectionner. Elle lit l'utilisateur dans l'en-tête
//! [`EN_TETE_UTILISATEUR`], ce qu'aucune authentification réelle ne fera jamais.

use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use api::base::IdUtilisateur;
use api::serveur::authentification::{
    Authentification, ErreurAuthentification, Identite, ResultatAuthentification,
};
use api::serveur::{Application, Decideur, Etat, RefusTotal, Reglages, journal, routeur};
use authz::{Action, Autorisation, Contexte, Decision, Entites, Ressource};
use axum::Router;
use axum::body::Body;
use axum::http::request::Parts;
use axum::http::{HeaderMap, Request, StatusCode};
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use time::OffsetDateTime;
use tower::ServiceExt;
use tracing::field::{Field, Visit};
use tracing_subscriber::Layer;
use tracing_subscriber::layer::{Context, SubscriberExt};
use uuid::Uuid;

/// En-tête lu par [`AuthentificationTest`].
pub const EN_TETE_UTILISATEUR: &str = "x-test-utilisateur";

/// Origine des réglages de test.
pub const ORIGINE: &str = "https://sigma.exemple.invalid";

/// Authentification de test : l'utilisateur de l'en-tête [`EN_TETE_UTILISATEUR`], vérifié à
/// l'instant ; aucune identité sans l'en-tête. `delai` simule une authentification lente,
/// `panique` une authentification qui panique.
#[derive(Debug, Clone, Copy, Default)]
pub struct AuthentificationTest {
    pub delai: Option<Duration>,
    pub panique: bool,
}

impl Authentification for AuthentificationTest {
    fn authentifier<'a>(&'a self, requete: &'a Parts) -> ResultatAuthentification<'a> {
        Box::pin(async move {
            if let Some(delai) = self.delai {
                tokio::time::sleep(delai).await;
            }
            assert!(
                !self.panique,
                "panique de test : montant 1234567, libellé confidentiel"
            );
            let Some(valeur) = requete.headers.get(EN_TETE_UTILISATEUR) else {
                return Ok(None);
            };
            let texte = valeur.to_str().map_err(|_| ErreurAuthentification)?;
            let uuid = Uuid::try_parse(texte).map_err(|_| ErreurAuthentification)?;
            Ok(Some(Identite {
                utilisateur: IdUtilisateur::depuis_uuid(uuid),
                derniere_verification: OffsetDateTime::now_utc(),
            }))
        })
    }
}

/// Décision forcée à « autorisé » : Cedar se trompe. Sert à vérifier que la requête et la RLS
/// tiennent seules.
#[derive(Debug, Clone, Copy, Default)]
pub struct ToujoursAutorise;

impl Decideur for ToujoursAutorise {
    fn decider(
        &self,
        _principal: &authz::Appartenance,
        _action: Action,
        _ressource: Ressource,
        _entites: &Entites,
        _contexte: &Contexte,
    ) -> Decision {
        Decision::Autorise
    }
}

/// Pool qui ne se connecte jamais tant qu'il n'est pas utilisé : pour les tests sans base, dont
/// les requêtes sont refusées avant toute lecture.
pub fn pool_sans_base() -> PgPool {
    PgPoolOptions::new()
        .max_connections(1)
        .acquire_timeout(Duration::from_millis(200))
        .connect_lazy("postgres://sigma_inexistant@127.0.0.1:1/sigma_inexistante")
        .expect("pool paresseux")
}

/// Application de test : authentification de test, décision Cedar réelle.
pub fn application(pool: PgPool, reglages: Reglages) -> Etat {
    Application::new(
        pool,
        Arc::new(Autorisation::charger().expect("politiques Cedar")),
        Arc::new(AuthentificationTest::default()),
        reglages,
    )
}

/// Application telle que le binaire l'assemble : authentification [`RefusTotal`].
pub fn application_du_binaire(pool: PgPool) -> Etat {
    Application::new(
        pool,
        Arc::new(Autorisation::charger().expect("politiques Cedar")),
        Arc::new(RefusTotal),
        Reglages::new(ORIGINE.to_owned()),
    )
}

/// Réponse lue en entier.
#[derive(Debug, Clone)]
pub struct Reponse {
    pub statut: StatusCode,
    pub entetes: HeaderMap,
    pub corps: Vec<u8>,
}

impl Reponse {
    /// Le corps, en JSON.
    pub fn json(&self) -> serde_json::Value {
        serde_json::from_slice(&self.corps).expect("corps JSON")
    }

    /// Le corps, en texte.
    pub fn texte(&self) -> String {
        String::from_utf8_lossy(&self.corps).into_owned()
    }
}

/// Envoie une requête au routeur.
pub async fn appeler(routeur: &Router, requete: Request<Body>) -> Reponse {
    let reponse = routeur
        .clone()
        .oneshot(requete)
        .await
        .expect("routeur infaillible");
    let statut = reponse.status();
    let entetes = reponse.headers().clone();
    let corps = axum::body::to_bytes(reponse.into_body(), 1 << 20)
        .await
        .expect("corps")
        .to_vec();
    Reponse {
        statut,
        entetes,
        corps,
    }
}

/// `GET` au nom d'un utilisateur (ou sans authentification).
pub async fn lire(routeur: &Router, chemin: &str, utilisateur: Option<IdUtilisateur>) -> Reponse {
    let mut requete = Request::get(chemin);
    if let Some(u) = utilisateur {
        requete = requete.header(EN_TETE_UTILISATEUR, u.to_string());
    }
    appeler(routeur, requete.body(Body::empty()).expect("requête")).await
}

/// Routeur d'une application.
pub fn routeur_de(etat: &Etat) -> Router {
    routeur(etat)
}

/// Un événement de journal capturé : cible et champs (nom, valeur).
#[derive(Debug, Clone)]
pub struct Evenement {
    pub cible: String,
    pub champs: Vec<(String, String)>,
}

/// Couche `tracing` qui capture les événements.
#[derive(Debug, Clone, Default)]
pub struct Capture(pub Arc<Mutex<Vec<Evenement>>>);

struct Visiteur<'a>(&'a mut Vec<(String, String)>);

impl Visit for Visiteur<'_> {
    fn record_debug(&mut self, champ: &Field, valeur: &dyn std::fmt::Debug) {
        self.0
            .push((champ.name().to_owned(), format!("{valeur:?}")));
    }

    fn record_str(&mut self, champ: &Field, valeur: &str) {
        self.0.push((champ.name().to_owned(), valeur.to_owned()));
    }
}

impl<S: tracing::Subscriber> Layer<S> for Capture {
    fn on_event(&self, evenement: &tracing::Event<'_>, _: Context<'_, S>) {
        let mut champs = Vec::new();
        evenement.record(&mut Visiteur(&mut champs));
        self.0.lock().expect("capture").push(Evenement {
            cible: evenement.metadata().target().to_owned(),
            champs,
        });
    }
}

/// Capture globale du binaire de test : le même filtre que le binaire ([`journal::filtre`]),
/// puis la capture. Abonné **global**, installé une fois : un abonné par fil (`set_default`)
/// manquerait les événements dont l'intérêt a été calculé dans un autre fil. Les tests
/// distinguent leurs événements par l'identifiant de requête (en-tête `x-sigma-requete`).
pub fn capture_globale() -> &'static Capture {
    static CAPTURE: OnceLock<Capture> = OnceLock::new();
    CAPTURE.get_or_init(|| {
        let capture = Capture::default();
        let abonne = tracing_subscriber::registry()
            .with(journal::filtre())
            .with(capture.clone());
        tracing::subscriber::set_global_default(abonne).expect("abonné global unique");
        capture
    })
}

/// Identifiant de requête d'une réponse.
pub fn id_requete(reponse: &Reponse) -> String {
    reponse.entetes[journal::EN_TETE_REQUETE]
        .to_str()
        .expect("identifiant")
        .to_owned()
}

/// Événements capturés des requêtes données, dans l'ordre.
pub fn evenements_de(capture: &Capture, requetes: &[String]) -> Vec<Evenement> {
    let tous = capture.0.lock().expect("capture").clone();
    requetes
        .iter()
        .flat_map(|id| {
            tous.iter()
                .filter(|e| e.champs.iter().any(|(c, v)| c == "requete" && v == id))
                .cloned()
                .collect::<Vec<_>>()
        })
        .collect()
}

/// Vérifie que chaque événement n'a que des champs de la liste blanche, sous la cible de
/// Sigma Books, et qu'aucune valeur ne contient l'un des `interdits`.
pub fn verifier_liste_blanche(evenements: &[Evenement], interdits: &[String]) {
    for evenement in evenements {
        assert_eq!(evenement.cible, journal::CIBLE, "{evenement:?}");
        for (champ, valeur) in &evenement.champs {
            assert!(
                journal::CHAMPS.contains(&champ.as_str()),
                "champ hors liste blanche : {champ} ({evenement:?})"
            );
            for interdit in interdits {
                assert!(
                    !valeur.contains(interdit.as_str()),
                    "valeur interdite dans les journaux : {interdit} ({evenement:?})"
                );
            }
        }
    }
}

/// Valeur d'un champ d'un événement.
pub fn champ<'a>(evenement: &'a Evenement, nom: &str) -> Option<&'a str> {
    evenement
        .champs
        .iter()
        .find_map(|(c, v)| (c == nom).then_some(v.as_str()))
}
