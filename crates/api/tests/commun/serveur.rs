//! Aides des tests du serveur : authentification et décision **de test**, sessions réelles
//! (horloge injectée), appel du routeur, capture des journaux.
//!
//! [`AuthentificationTest`] n'existe que dans `crates/api/tests/` : elle n'est compilée que pour
//! les tests et le binaire ne peut pas la sélectionner. Elle lit l'utilisateur dans l'en-tête
//! [`EN_TETE_UTILISATEUR`], ce qu'aucune authentification réelle ne fera jamais.

use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use api::base::session::{MethodeSession, NatureSession, NouvelleSession};
use api::base::{IdSession, IdUtilisateur};
use api::facteurs::cle::CleInstance;
use api::serveur::authentification::{
    Authentification, ErreurAuthentification, Identite, ResultatAuthentification,
};
use api::serveur::routes::{CasUsage, Methode, Route, organisation, parametres_fiscaux, pieces};
use api::serveur::session::{SessionOuverte, ouvrir_session};
use api::serveur::{
    Application, AuthentificationSession, Decideur, Etat, Horloge, HorlogeSysteme, Reglages,
    journal, routeur,
};
use authz::{
    Action, ActionFacteur, ActionUtilisateur, Autorisation, Contexte, ContexteFacteur, Decision,
    Entites, Ressource, RessourceUtilisateur, Verdict,
};
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

/// En-tête lu par [`AuthentificationTest`] : ancienneté, en secondes, de la dernière
/// vérification de facteur (0 par défaut : session « fraîche »).
pub const EN_TETE_ANCIENNETE: &str = "x-test-verifie-il-y-a";

/// Origine des réglages de test.
pub const ORIGINE: &str = "https://sigma.exemple.invalid";

/// Authentification de test : l'utilisateur de l'en-tête [`EN_TETE_UTILISATEUR`], vérifié à
/// l'instant (ou il y a [`EN_TETE_ANCIENNETE`] secondes), avec une session neuve ; aucune
/// identité sans l'en-tête. `delai` simule une authentification lente,
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
            let anciennete = match requete.headers.get(EN_TETE_ANCIENNETE) {
                Some(v) => v
                    .to_str()
                    .ok()
                    .and_then(|v| v.parse::<i64>().ok())
                    .ok_or(ErreurAuthentification)?,
                None => 0,
            };
            // Session complète, et jeton anti-CSRF tenu pour lié : l'anti-CSRF lié à la session
            // est éprouvé avec les sessions réelles (`tests/sessions.rs`).
            Ok(Some(Identite {
                utilisateur: IdUtilisateur::depuis_uuid(uuid),
                session: IdSession::nouveau(),
                derniere_verification: OffsetDateTime::now_utc()
                    - time::Duration::seconds(anciennete),
                nature: NatureSession::Complete,
                csrf_lie: true,
                derniere_activite: OffsetDateTime::now_utc(),
            }))
        })
    }
}

/// Décision forcée à « autorisé » : Cedar se trompe. Sert à vérifier que la requête et la RLS
/// tiennent seules.
#[derive(Debug, Clone, Copy, Default)]
pub struct ToujoursAutorise;

impl Decideur for ToujoursAutorise {
    fn verdict(
        &self,
        _principal: &authz::Appartenance,
        _action: Action,
        _ressource: Ressource,
        _entites: &Entites,
        _contexte: &Contexte,
    ) -> Verdict {
        Verdict {
            decision: Decision::Autorise,
            politiques: vec!["test-toujours-autorise".to_owned()],
        }
    }

    fn verdict_utilisateur(
        &self,
        _principal: authz::IdUtilisateur,
        _action: ActionUtilisateur,
        _ressource: RessourceUtilisateur,
    ) -> Verdict {
        Verdict {
            decision: Decision::Autorise,
            politiques: vec!["test-toujours-autorise".to_owned()],
        }
    }

    fn verdict_connexion(&self) -> Verdict {
        Verdict {
            decision: Decision::Autorise,
            politiques: vec!["test-toujours-autorise".to_owned()],
        }
    }

    fn verdict_facteur(
        &self,
        _principal: authz::IdUtilisateur,
        _action: ActionFacteur,
        _ressource: authz::IdUtilisateur,
        _contexte: ContexteFacteur,
    ) -> Verdict {
        Verdict {
            decision: Decision::Autorise,
            politiques: vec!["test-toujours-autorise".to_owned()],
        }
    }
}

/// Clé de l'instance des tests : tirée une fois par binaire de test, jamais écrite.
pub fn cle_de_test() -> Arc<CleInstance> {
    static CLE: OnceLock<Arc<CleInstance>> = OnceLock::new();
    Arc::clone(CLE.get_or_init(|| {
        let mut octets = [0u8; 32];
        getrandom::fill(&mut octets).expect("aléa");
        Arc::new(CleInstance::depuis_octets(octets))
    }))
}

/// Réglages de test : ceux par défaut, avec la clé de l'instance des tests et une durée
/// plancher des échecs de connexion courte (les tests de la durée la règlent eux-mêmes).
pub fn reglages_de_test() -> Reglages {
    let mut reglages = Reglages::new(ORIGINE.to_owned());
    reglages.facteurs.cle_instance = Some(cle_de_test());
    reglages.facteurs.duree_plancher_connexion = Duration::from_millis(10);
    reglages
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

/// Application de test : authentification de test, décision Cedar réelle. Le même pool sert
/// à la lecture d'audit : pour les tests sans base, ou qui ne lisent pas l'audit.
pub fn application(pool: PgPool, reglages: Reglages) -> Etat {
    application_avec(pool.clone(), pool, reglages)
}

/// Application de test sur la base : pool du rôle applicatif et pool de lecture d'audit.
pub fn application_base(env: &super::Env, reglages: Reglages) -> Etat {
    application_avec(env.application.clone(), env.lecture_audit.clone(), reglages)
}

fn application_avec(pool: PgPool, lecture_audit: PgPool, mut reglages: Reglages) -> Etat {
    if reglages.facteurs.cle_instance.is_none() {
        reglages.facteurs.cle_instance = Some(cle_de_test());
    }
    let etat = Application::new(
        pool,
        lecture_audit,
        Arc::new(Autorisation::charger().expect("politiques Cedar")),
        Arc::new(AuthentificationTest::default()),
        reglages,
    );
    brancher_pieces_memoire(&etat);
    etat
}

/// Voie pièces (PR 12) : branche le stockage des pièces en mémoire partagé du binaire de test.
pub fn brancher_pieces_memoire(etat: &Etat) -> super::pieces::StockageMemoire {
    let stockage = super::pieces::StockageMemoire::partage();
    etat.brancher_pieces(super::pieces::pieces_memoire(&stockage))
        .expect("service des pièces branché une seule fois");
    stockage
}

/// Application telle que le binaire l'assemble : authentification par les sessions de
/// PostgreSQL ([`AuthentificationSession`]), horloge du système.
pub fn application_du_binaire(pool: PgPool) -> Etat {
    let reglages = Reglages::new(ORIGINE.to_owned());
    Application::new(
        pool.clone(),
        pool.clone(),
        Arc::new(Autorisation::charger().expect("politiques Cedar")),
        Arc::new(AuthentificationSession::new(
            pool,
            Arc::new(HorlogeSysteme),
            reglages.sessions,
        )),
        reglages,
    )
}

/// Horloge de test : l'instant du système, ou un instant **fixe** ([`HorlogeTest::fixer`]),
/// décalé d'un nombre de secondes réglable.
#[derive(Debug, Clone, Default)]
pub struct HorlogeTest {
    decalage: Arc<AtomicI64>,
    fixe: Arc<std::sync::Mutex<Option<OffsetDateTime>>>,
}

impl HorlogeTest {
    /// Horloge **figée** au milieu d'un pas TOTP de 30 secondes, près de l'instant du système.
    ///
    /// Pour tout test qui calcule un code TOTP à `maintenant()` puis le fait vérifier plus tard :
    /// avec l'instant du système, le test peut chevaucher deux pas (un code calculé avant la
    /// frontière, vérifié après ; deux codes « du même pas » en réalité de pas voisins), d'où
    /// des échecs rares et non reproductibles. Figée, l'horloge ne change qu'avec
    /// [`HorlogeTest::avancer`], dont les multiples de 30 s gardent le milieu du pas.
    pub fn alignee_sur_un_pas() -> Self {
        let horloge = Self::default();
        let pas = api::facteurs::totp::PAS_SECONDES;
        let maintenant = OffsetDateTime::now_utc().unix_timestamp();
        let milieu = maintenant - maintenant.rem_euclid(pas) + pas / 2;
        horloge.fixer(OffsetDateTime::from_unix_timestamp(milieu).expect("instant valide"));
        horloge
    }

    /// Avance l'horloge.
    pub fn avancer(&self, duree: time::Duration) {
        self.decalage
            .fetch_add(duree.whole_seconds(), Ordering::SeqCst);
    }

    /// Règle l'horloge sur un instant absolu : elle ne suit plus l'horloge du système (le
    /// décalage de [`HorlogeTest::avancer`] s'y ajoute encore).
    pub fn fixer(&self, instant: OffsetDateTime) {
        *self.fixe.lock().expect("horloge") = Some(instant);
    }
}

impl Horloge for HorlogeTest {
    fn maintenant(&self) -> OffsetDateTime {
        let base = (*self.fixe.lock().expect("horloge")).unwrap_or_else(OffsetDateTime::now_utc);
        base + time::Duration::seconds(self.decalage.load(Ordering::SeqCst))
    }
}

/// Application sur la base avec l'authentification **réelle** des sessions, son horloge
/// (partagée avec l'authentification) et un décideur donné.
pub fn application_sessions(
    env: &super::Env,
    horloge: &HorlogeTest,
    decideur: Arc<dyn Decideur>,
) -> Etat {
    application_sessions_avec(env, horloge, decideur, reglages_de_test())
}

/// Comme [`application_sessions`], avec des réglages donnés (débits des connexions…).
pub fn application_sessions_avec(
    env: &super::Env,
    horloge: &HorlogeTest,
    decideur: Arc<dyn Decideur>,
    reglages: Reglages,
) -> Etat {
    let horloge: Arc<dyn Horloge> = Arc::new(horloge.clone());
    Application::avec_horloge(
        env.application.clone(),
        env.lecture_audit.clone(),
        decideur,
        Arc::new(AuthentificationSession::new(
            env.application.clone(),
            Arc::clone(&horloge),
            reglages.sessions,
        )),
        reglages,
        horloge,
    )
}

/// Ouvre une session (fonction interne de l'API, comme le feront les connexions des PR 8b et
/// 8c) pour `utilisateur`, de nature `nature`, à l'instant de l'horloge.
pub async fn ouvrir(
    env: &super::Env,
    horloge: &HorlogeTest,
    utilisateur: IdUtilisateur,
    nature: NatureSession,
) -> SessionOuverte {
    let nouvelle = NouvelleSession {
        utilisateur,
        nature,
        methode: match nature {
            NatureSession::Complete => MethodeSession::Passkey,
            NatureSession::Restreinte => MethodeSession::CodeRecuperation,
            NatureSession::Enrolement => MethodeSession::LienEnrolement,
        },
        instant: horloge.maintenant(),
        appareil: Some("Navigateur de test".to_owned()),
        adresse: Some(std::net::IpAddr::from([192, 0, 2, 7])),
    };
    ouvrir_session(
        &env.application,
        &nouvelle,
        &Reglages::new(ORIGINE.to_owned()).sessions,
    )
    .await
    .expect("ouverture de session")
}

/// Valeur de l'en-tête `Cookie` d'une session.
pub fn cookie_de(session: &SessionOuverte) -> String {
    let set_cookie = session.cookie(OffsetDateTime::now_utc()).expect("cookie");
    let texte = set_cookie.to_str().expect("ASCII");
    texte.split(';').next().expect("paire").to_owned()
}

/// Appelle `methode chemin` avec le cookie de session donné (ou aucun) et, pour une mutation,
/// les marques de la même origine et l'en-tête anti-CSRF `csrf` (rien si `None`).
pub async fn appeler_session(
    routeur: &Router,
    methode: &str,
    chemin: &str,
    cookie: Option<&str>,
    csrf: Option<&str>,
) -> Reponse {
    appeler_session_corps(routeur, methode, chemin, cookie, csrf, "").await
}

/// Comme [`appeler_session`], avec un corps JSON.
pub async fn appeler_session_corps(
    routeur: &Router,
    methode: &str,
    chemin: &str,
    cookie: Option<&str>,
    csrf: Option<&str>,
    corps: &str,
) -> Reponse {
    let mut requete = Request::builder().method(methode).uri(chemin);
    if methode != "GET" {
        requete = requete
            .header("origin", ORIGINE)
            .header("sec-fetch-site", "same-origin")
            .header("content-type", "application/json");
    }
    if let Some(c) = cookie {
        requete = requete.header("cookie", c);
    }
    if let Some(c) = csrf {
        requete = requete.header("x-sigma-csrf", c);
    }
    appeler(
        routeur,
        requete.body(Body::from(corps.to_owned())).expect("requête"),
    )
    .await
}

/// Appelle une route de la table au nom d'une session réelle (cookie et jeton anti-CSRF).
pub async fn appeler_route_session(
    routeur: &Router,
    route: &Route,
    chemin: &str,
    session: &SessionOuverte,
) -> Reponse {
    let csrf = session.jeton.csrf();
    let mut requete = Request::builder()
        .method(route.methode.nom())
        .uri(chemin)
        .header("cookie", cookie_de(session));
    if route.methode != Methode::Get {
        requete = requete
            .header("origin", ORIGINE)
            .header("sec-fetch-site", "same-origin")
            .header("x-sigma-csrf", csrf)
            .header("content-type", "application/json");
    }
    appeler(
        routeur,
        requete
            .body(Body::from(corps_de(route).to_owned()))
            .expect("requête"),
    )
    .await
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

/// Mutation (`POST`) de la même origine (en-têtes anti-CSRF), au nom d'un utilisateur, avec un
/// corps JSON et, en option, l'ancienneté de la dernière vérification de facteur.
pub async fn envoyer(
    routeur: &Router,
    chemin: &str,
    utilisateur: Option<IdUtilisateur>,
    corps: &str,
    anciennete: Option<i64>,
) -> Reponse {
    let mut requete = Request::post(chemin)
        .header("origin", ORIGINE)
        .header("sec-fetch-site", "same-origin")
        .header("x-sigma-csrf", "1")
        .header("content-type", "application/json");
    if let Some(u) = utilisateur {
        requete = requete.header(EN_TETE_UTILISATEUR, u.to_string());
    }
    if let Some(secondes) = anciennete {
        requete = requete.header(EN_TETE_ANCIENNETE, secondes.to_string());
    }
    appeler(
        routeur,
        requete.body(Body::from(corps.to_owned())).expect("requête"),
    )
    .await
}

/// Corps d'exemple d'une route : celui de la création d'un dossier, vide sinon.
pub fn corps_de(route: &Route) -> &'static str {
    match route.cas_usage {
        CasUsage::Organisation(organisation::Cas::CreerDossier) => {
            static CORPS: OnceLock<String> = OnceLock::new();
            CORPS.get_or_init(|| super::referentiel::corps_creation("Dossier créé par le test"))
        }
        CasUsage::Organisation(organisation::Cas::ModifierContexte) => r#"{"langue":"fr"}"#,
        // Paramètres fiscaux du dossier (ADR 0030) : une version sans aucun changement, refusée
        // (422) pour qui a le droit, sans rien créer.
        CasUsage::ParametresFiscaux(parametres_fiscaux::Cas::Creer) => {
            r#"{"date_effet":"2099-01-01","motif":"Test d'isolation"}"#
        }
        // Voie pièces (PR 12) : un PDF minimal (signature seule), une note d'OD, un motif.
        CasUsage::Pieces(pieces::Cas::Deposer) => "%PDF-1.7\n% pièce de test\n",
        CasUsage::Pieces(pieces::Cas::CreerInterne) => {
            r#"{"date":"2026-11-19","motif":"Régularisation de test"}"#
        }
        CasUsage::Pieces(pieces::Cas::Ecarter) => r#"{"motif":"Dépôt erroné"}"#,
        // Voie administration (PR 9a).
        CasUsage::Administration(cas) => super::administration::corps(cas),
        // Voie comptabilité (PR 11, 13, 14).
        CasUsage::Referentiel(cas) => super::referentiel::corps(cas),
        CasUsage::Brouillard(cas) => super::brouillard::corps(cas),
        // Lot L3 : chaque module donne ses corps dans `commun/<module>.rs` (branché par la PR 6).
        CasUsage::Tiers(cas) => super::tiers::corps(cas),
        CasUsage::Echeancier(cas) => super::echeancier::corps(cas),
        CasUsage::Lettrage(cas) => super::lettrage::corps(cas),
        CasUsage::EditionsTiers(cas) => super::editions_tiers::corps(cas),
        CasUsage::Tresorerie(cas) => super::tresorerie::corps(cas),
        CasUsage::Releves(cas) => super::releves::corps(cas),
        CasUsage::Rapprochement(cas) => super::rapprochement::corps(cas),
        CasUsage::Prelevements(cas) => super::prelevements::corps(cas),
        _ => "",
    }
}

/// Appelle une route de la table avec sa méthode : `GET`, ou mutation de la même origine avec
/// le corps de [`corps_de`].
pub async fn appeler_route(
    routeur: &Router,
    route: &Route,
    chemin: &str,
    utilisateur: Option<IdUtilisateur>,
) -> Reponse {
    match route.methode {
        Methode::Get => lire(routeur, chemin, utilisateur).await,
        Methode::Post => envoyer(routeur, chemin, utilisateur, corps_de(route), None).await,
        Methode::Delete => {
            let mut requete = Request::delete(chemin)
                .header("origin", ORIGINE)
                .header("sec-fetch-site", "same-origin")
                .header("x-sigma-csrf", "1");
            if let Some(u) = utilisateur {
                requete = requete.header(EN_TETE_UTILISATEUR, u.to_string());
            }
            appeler(routeur, requete.body(Body::empty()).expect("requête")).await
        }
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
            // Le message d'échec ne reprend ni la valeur interdite (un jeton, par exemple) ni
            // l'événement qui la contient : seulement son rang et le champ fautif.
            for (rang, interdit) in interdits.iter().enumerate() {
                assert!(
                    !valeur.contains(interdit.as_str()),
                    "valeur interdite n° {rang} dans le champ « {champ} » des journaux"
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
