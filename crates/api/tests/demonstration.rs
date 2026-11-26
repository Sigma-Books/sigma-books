//! Démonstration de bout en bout du lot L2 (L2-37, critère de fin du lot) : un cabinet est créé
//! par la commande d'administration, ses membres sont invités et enrôlent une passkey (authentificateur
//! logiciel), puis un dossier est tenu à la main par l'API jusqu'à l'export d'intégrité vérifié par
//! `sigma-verifier`, avec un **second cabinet** dont l'administrateur ne voit rien du premier à
//! chaque étape, et le journal d'audit complet.
//!
//! Déterminisme et indépendance : horloge injectée (figée, avancée à la main), identifiants et
//! courriels aléatoires (aucune base partagée avec un autre test, aucun état global), autorité
//! d'horodatage locale tirée à chaque exécution (clé et certificat calculés, jamais écrits), tâche
//! d'horodatage **prise par ce test seul** (sa tâche est avancée au début de la file, le
//! travailleur n'en prend qu'une). Aucun accès réseau. L'application, la base et le stockage des
//! pièces (en mémoire) sont ceux des autres tests.
#![cfg(feature = "tests-postgres")]
// Fichier de test : les `expect`/`unwrap` y sont voulus.
#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::too_many_lines,
    clippy::similar_names,
    clippy::many_single_char_names
)]

mod commun;

#[path = "commun/autorite.rs"]
mod autorite;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use api::base::liens::{self, NatureLien};
use api::base::{ContexteTenant, IdCabinet, IdUtilisateur, ouvrir_transaction};
use api::horodatage::tache::{Reglages as ReglagesTache, Travailleur};
use api::instance::{Commande, executer};
use api::serveur::Horloge;
use authz::Autorisation;
use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use commun::brouillard::journal_de_saisie;
use commun::referentiel::comptes;
use commun::serveur::{
    HorlogeTest, ORIGINE, Reponse, appeler, appeler_session_corps, application_sessions_avec,
    brancher_pieces_memoire, reglages_de_test, routeur_de,
};
use commun::{Env, MIGRATEUR, env, variable, verifier_migrations};
use serde_json::{Value, json};
use sqlx::postgres::{PgConnectOptions, PgConnection, PgPoolOptions};
use sqlx::{Connection, Executor};
use std::str::FromStr;
use time::Duration;
use uuid::Uuid;
use webauthn_authenticator_rs::WebauthnAuthenticator;
use webauthn_authenticator_rs::softpasskey::SoftPasskey;
use webauthn_rs::prelude::{
    CreationChallengeResponse, PublicKeyCredential, RegisterPublicKeyCredential,
    RequestChallengeResponse, Url,
};

const CEREMONIE: &str = "__Host-sigma_ceremonie";
const SESSION: &str = "__Host-sigma_session";

// --- Décor -------------------------------------------------------------------------------------

struct Session {
    cookie: String,
    csrf: String,
}

/// Un utilisateur de la démonstration : identité, authentificateur logiciel, session courante.
struct Personne {
    courriel: String,
    id: Uuid,
    auth: SoftPasskey,
    session: Session,
    invitation: Uuid,
}

struct Monde {
    env: Env,
    horloge: HorlogeTest,
    routeur: Router,
}

fn origine() -> Url {
    Url::parse(ORIGINE).unwrap()
}

fn cookie_nomme(r: &Reponse, nom: &str) -> String {
    r.entetes
        .get_all("set-cookie")
        .iter()
        .filter_map(|v| v.to_str().ok())
        .find(|v| v.starts_with(&format!("{nom}=")) && !v.contains("Max-Age=0"))
        .unwrap_or_else(|| panic!("cookie {nom} absent : {:?}", r.entetes))
        .split(';')
        .next()
        .unwrap()
        .to_owned()
}

fn attendre(r: &Reponse, statut: StatusCode) -> Value {
    assert_eq!(r.statut, statut, "{}", r.texte());
    if r.corps.is_empty()
        || r.entetes
            .get("content-type")
            .is_none_or(|t| t != "application/json")
    {
        Value::Null
    } else {
        r.json()
    }
}

impl Monde {
    fn nouveau(env: Env) -> Self {
        let horloge = HorlogeTest::alignee_sur_un_pas();
        let mut reglages = reglages_de_test();
        // Débits larges : la démonstration enchaîne plus d'appels qu'un utilisateur réel.
        for debit in [
            &mut reglages.facteurs.debit_connexion_adresse,
            &mut reglages.facteurs.debit_connexion_courriel,
            &mut reglages.facteurs.debit_verification,
            &mut reglages.debit_adresse,
            &mut reglages.debit_utilisateur,
        ] {
            debit.capacite = 100_000;
        }
        let etat = application_sessions_avec(
            &env,
            &horloge,
            Arc::new(Autorisation::charger().unwrap()),
            reglages,
        );
        brancher_pieces_memoire(&etat);
        let routeur = routeur_de(&etat);
        Self {
            env,
            horloge,
            routeur,
        }
    }

    async fn appel(&self, s: &Session, methode: &str, chemin: &str, corps: &Value) -> Reponse {
        let corps = if corps.is_null() {
            String::new()
        } else {
            corps.to_string()
        };
        appeler_session_corps(
            &self.routeur,
            methode,
            chemin,
            Some(&s.cookie),
            Some(&s.csrf),
            &corps,
        )
        .await
    }

    async fn lire(&self, s: &Session, chemin: &str) -> Reponse {
        self.appel(s, "GET", chemin, &Value::Null).await
    }

    async fn poster(&self, s: &Session, chemin: &str, corps: &Value) -> Reponse {
        self.appel(s, "POST", chemin, corps).await
    }

    /// `POST` sans session (connexion, usage d'un lien), avec cookie de cérémonie éventuel.
    async fn anonyme(&self, chemin: &str, corps: &Value, cookie: Option<&str>) -> Reponse {
        let mut requete = Request::post(chemin)
            .header("origin", ORIGINE)
            .header("sec-fetch-site", "same-origin")
            .header("x-sigma-csrf", "1")
            .header("content-type", "application/json")
            .header("user-agent", "Navigateur de la démonstration");
        if let Some(c) = cookie {
            requete = requete.header("cookie", c);
        }
        appeler(
            &self.routeur,
            requete.body(Body::from(corps.to_string())).unwrap(),
        )
        .await
    }

    /// Dépôt d'une pièce (corps binaire).
    async fn deposer(&self, s: &Session, chemin: &str, contenu: Vec<u8>) -> Reponse {
        let requete = Request::post(chemin)
            .header("origin", ORIGINE)
            .header("sec-fetch-site", "same-origin")
            .header("x-sigma-csrf", &s.csrf)
            .header("content-type", "application/octet-stream")
            .header("cookie", &s.cookie)
            .body(Body::from(contenu))
            .unwrap();
        appeler(&self.routeur, requete).await
    }

    /// Enrôle une passkey au nom de la session donnée.
    async fn enroler(&self, auth: &mut SoftPasskey, s: &Session, nom: &str) {
        let d = self
            .poster(s, "/api/moi/facteurs/passkeys/defi", &Value::Null)
            .await;
        assert_eq!(d.statut, StatusCode::OK, "{}", d.texte());
        let ceremonie = cookie_nomme(&d, CEREMONIE);
        let options: CreationChallengeResponse =
            serde_json::from_value(d.json()["defi"].clone()).unwrap();
        let credential: RegisterPublicKeyCredential =
            auth.do_registration(origine(), options).unwrap();
        let r = self
            .appel_cookies(
                s,
                &ceremonie,
                "/api/moi/facteurs/passkeys",
                &json!({"nom": nom, "credential": credential}),
            )
            .await;
        assert_eq!(r.statut, StatusCode::CREATED, "{}", r.texte());
    }

    async fn appel_cookies(
        &self,
        s: &Session,
        ceremonie: &str,
        chemin: &str,
        c: &Value,
    ) -> Reponse {
        appeler_session_corps(
            &self.routeur,
            "POST",
            chemin,
            Some(&format!("{}; {ceremonie}", s.cookie)),
            Some(&s.csrf),
            &c.to_string(),
        )
        .await
    }

    /// Connexion par passkey : défi, signature par l'authentificateur logiciel, session complète.
    async fn connecter(&self, auth: &mut SoftPasskey, courriel: &str) -> Session {
        let d = self
            .anonyme(
                "/api/connexion/passkey/defi",
                &json!({"courriel": courriel}),
                None,
            )
            .await;
        assert_eq!(d.statut, StatusCode::OK, "{}", d.texte());
        let ceremonie = cookie_nomme(&d, CEREMONIE);
        let options: RequestChallengeResponse =
            serde_json::from_value(d.json()["defi"].clone()).unwrap();
        let credential: PublicKeyCredential = auth.do_authentication(origine(), options).unwrap();
        let r = self
            .anonyme(
                "/api/connexion/passkey",
                &json!({"courriel": courriel, "credential": credential}),
                Some(&ceremonie),
            )
            .await;
        assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
        assert_eq!(r.json()["nature"], "complete");
        assert_eq!(r.json()["methode"], "passkey");
        Session {
            cookie: cookie_nomme(&r, SESSION),
            csrf: r.json()["csrf"].as_str().unwrap().to_owned(),
        }
    }

    /// Ré-authentification par passkey : la session redevient fraîche (jeton tourné).
    async fn reauthentifier(&self, p: &mut Personne) {
        let d = self
            .poster(
                &p.session,
                "/api/session/reauthentification/passkey/defi",
                &Value::Null,
            )
            .await;
        assert_eq!(d.statut, StatusCode::OK, "{}", d.texte());
        let ceremonie = cookie_nomme(&d, CEREMONIE);
        let options: RequestChallengeResponse =
            serde_json::from_value(d.json()["defi"].clone()).unwrap();
        let credential = p.auth.do_authentication(origine(), options).unwrap();
        let r = self
            .appel_cookies(
                &p.session,
                &ceremonie,
                "/api/session/reauthentification/passkey",
                &json!({"credential": credential}),
            )
            .await;
        assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
        p.session = Session {
            cookie: cookie_nomme(&r, SESSION),
            csrf: r.json()["csrf"].as_str().unwrap().to_owned(),
        };
    }

    /// Utilise un lien à usage unique, enrôle une passkey avec la session d'enrôlement, puis se
    /// connecte par cette passkey : rend la personne avec sa session complète.
    async fn integrer(&self, jeton: &str, courriel: &str) -> Option<Personne> {
        let r = self
            .anonyme("/api/liens/utilisation", &json!({"jeton": jeton}), None)
            .await;
        if r.statut == StatusCode::UNAUTHORIZED {
            return None;
        }
        assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
        assert_eq!(r.json()["nature"], "enrolement");
        let enrolement = Session {
            cookie: cookie_nomme(&r, SESSION),
            csrf: r.json()["csrf"].as_str().unwrap().to_owned(),
        };
        let mut auth = SoftPasskey::new(true);
        self.enroler(&mut auth, &enrolement, "Téléphone").await;
        let session = self.connecter(&mut auth, courriel).await;
        let id: Uuid = sqlx::query_scalar("SELECT id FROM utilisateur WHERE courriel = $1")
            .bind(courriel)
            .fetch_one(&self.env.admin)
            .await
            .unwrap();
        Some(Personne {
            courriel: courriel.to_owned(),
            id,
            auth,
            session,
            invitation: Uuid::nil(),
        })
    }

    /// Crée un cabinet par la commande d'administration de l'instance et intègre son premier
    /// administrateur (lien d'invitation, passkey, connexion).
    async fn creer_cabinet(&self, nom: &str) -> (Uuid, Personne) {
        let courriel = format!("admin.{}@cabinet.example", Uuid::new_v4().simple());
        let sortie = executer(
            &self.env.application,
            &Commande::CreerCabinet {
                nom: nom.to_owned(),
                courriel: courriel.clone(),
                nom_administrateur: "Administrateur du test".to_owned(),
            },
            ORIGINE,
            self.horloge.maintenant(),
            false,
        )
        .await
        .unwrap();
        let cabinet: Uuid = sortie.lignes[0]
            .strip_prefix("cabinet : ")
            .unwrap()
            .parse()
            .unwrap();
        let jeton = sortie
            .lignes
            .last()
            .unwrap()
            .split_once("/lien#")
            .unwrap()
            .1;
        (cabinet, self.integrer(jeton, &courriel).await.unwrap())
    }

    /// Invite un membre (route d'administration), émet le lien comme le fera la file de tâches,
    /// et l'intègre. Un lien n'est jamais lu en base : il n'existe que le temps de l'appel.
    async fn inviter(&self, cabinet: Uuid, admin: &Personne, role: &str) -> Personne {
        let courriel = format!("{role}.{}@cabinet.example", Uuid::new_v4().simple());
        let r = self
            .poster(
                &admin.session,
                &format!("/api/cabinets/{cabinet}/invitations"),
                &json!({"courriel": courriel, "nom": format!("Personne {role}"), "role": role}),
            )
            .await;
        let invitation: Uuid = attendre(&r, StatusCode::CREATED)["id"]
            .as_str()
            .unwrap()
            .parse()
            .unwrap();
        let mut tx = ouvrir_transaction(
            &self.env.application,
            ContexteTenant::cabinet(
                IdCabinet::depuis_uuid(cabinet),
                IdUtilisateur::depuis_uuid(admin.id),
            ),
        )
        .await
        .unwrap();
        let jeton = liens::emettre(
            &mut tx,
            NatureLien::Invitation,
            invitation,
            self.horloge.maintenant(),
        )
        .await
        .unwrap();
        tx.confirmer().await.unwrap();
        let personne = self.integrer(&jeton.texte(), &courriel).await;
        let mut personne = personne.expect("lien d'invitation utilisable");
        personne.invitation = invitation;
        personne
    }
}

fn annee(m: &Monde) -> i32 {
    m.horloge.maintenant().year()
}

/// Date du `jour` du mois courant de l'horloge (`mois` = 1) ou des suivants (2, 3…) : le dossier
/// de A commence ce mois-ci, ses écritures sont dans sa première période, qu'une validation
/// d'aujourd'hui peut encore atteindre (échéance de clôture).
fn date(m: &Monde, mois: u8, jour: u8) -> String {
    let d = m.horloge.maintenant().date();
    let rang = i32::from(u8::from(d.month())) - 1 + i32::from(mois) - 1;
    format!("{:04}-{:02}-{jour:02}", d.year() + rang / 12, rang % 12 + 1)
}

fn ligne(compte: &str, sens: &str, montant: i64) -> Value {
    json!({"compte": compte, "sens": sens, "montant": montant.to_string()})
}

/// Écriture attendue d'un test : (compte, sens, montant), tenue **indépendamment** de l'API pour
/// recalculer la balance.
type Mouvement = (String, &'static str, i64);

fn contenu(m: &Monde, jour: (u8, u8), libelle: &str, lignes: &[Mouvement]) -> Value {
    json!({
        "journal": journal_de_saisie(),
        "date": date(m, jour.0, jour.1),
        "libelle": libelle,
        "lignes": lignes.iter().map(|(c, s, x)| ligne(c, s, *x)).collect::<Vec<_>>(),
    })
}

// --- Isolation à chaque étape --------------------------------------------------------------------

/// Ce que le cabinet A possède, vu du cabinet B : identifiants qui ne doivent jamais apparaître.
struct Secrets {
    cabinet: Uuid,
    dossier: Option<Uuid>,
    noms: Vec<String>,
    /// Vrais identifiants du cabinet A, par famille de ressource : (`membre`, `invitation`,
    /// `compte`, `periode`, `brouillon`, `piece`, `ecriture`).
    ressources: BTreeMap<&'static str, Uuid>,
}

/// Contrôle de l'étape `etape` : l'administrateur du cabinet B ne voit rien du cabinet A. Les
/// ressources d'autrui répondent comme des ressources absentes (404, jamais 200), ni lecture ni
/// mutation n'aboutit, et rien du cabinet A ne figure dans les réponses de B.
async fn isoler(m: &Monde, b: &(Uuid, Personne, Uuid), a: &Secrets, etape: &str) {
    let (cabinet_b, admin_b, dossier_b) = (b.0, &b.1, b.2);
    let ca = a.cabinet;
    let avant = instantane(m, ca).await;
    let mut chemins = vec![
        format!("/api/cabinets/{ca}"),
        format!("/api/cabinets/{ca}/dossiers"),
        format!("/api/cabinets/{ca}/membres"),
        format!("/api/cabinets/{ca}/invitations"),
        format!("/api/cabinets/{ca}/audit"),
        format!("/api/cabinets/{ca}/audit/verification"),
    ];
    let mut mutations = vec![
        (
            format!("/api/cabinets/{ca}/invitations"),
            json!({"courriel": "intrus@cabinet.example", "nom": "Intrus", "role": "auditeur"}),
        ),
        (
            format!("/api/cabinets/{ca}/dossiers"),
            json!({"nom": "Intrus", "pays": "SN", "nature": "reprise", "annee": annee(m)}),
        ),
    ];
    if let Some(d) = a.dossier {
        let base = format!("/api/cabinets/{ca}/dossiers/{d}");
        for suite in [
            "",
            "/brouillard",
            "/ecritures",
            "/comptes",
            "/journaux",
            "/tiers",
            "/exercices",
            "/pieces",
            "/editions/balance",
            "/editions/grand-livre",
            "/editions/journaux",
            "/editions/centralisateur",
            "/alertes",
            "/integrite",
            "/integrite/jetons",
            "/integrite/verification",
            "/integrite/temoin",
            "/audit",
            "/audit/verification",
        ] {
            chemins.push(format!("{base}{suite}"));
        }
        for (suite, corps) in [
            ("/brouillard", contenu(m, (1, 2), "Intrusion", &[])),
            ("/integrite/export", Value::Null),
            ("/audit/export", Value::Null),
            (
                "/periodes/00000000-0000-4000-8000-000000000001/verrouillage",
                json!({}),
            ),
        ] {
            mutations.push((format!("{base}{suite}"), corps));
        }
    }
    for chemin in &chemins {
        let r = m.lire(&admin_b.session, chemin).await;
        controle_refus(&r, a, etape, chemin);
    }
    for (chemin, corps) in &mutations {
        let r = m.poster(&admin_b.session, chemin, corps).await;
        controle_refus(&r, a, etape, chemin);
    }
    // Les VRAIS identifiants du cabinet A, présentés par B : sur chaque famille de ressource, le
    // refus est indiscernable de celui d'un identifiant inexistant (même statut, même corps),
    // chez A comme dans le propre cabinet de B (identifiant d'autrui cité dans son chemin).
    let familles = familles(m, a);
    assert_eq!(
        familles.len(),
        familles_attendues(a),
        "{etape} : toutes les familles connues sont éprouvées"
    );
    for f in familles {
        let reel = a.ressources[f.genre];
        let (dossier_a, aleatoire) = (a.dossier.unwrap_or_else(Uuid::nil), Uuid::new_v4());
        let chemin = |c: Uuid, d: Uuid, x: Uuid| {
            f.chemin
                .replace("{c}", &c.to_string())
                .replace("{d}", &d.to_string())
                .replace("{x}", &x.to_string())
        };
        let (methode, corps) = (f.methode, &f.corps);
        let appel = |c: Uuid, d: Uuid, x: Uuid| {
            let chemin = chemin(c, d, x);
            async move {
                (
                    m.appel(&admin_b.session, methode, &chemin, corps).await,
                    chemin,
                )
            }
        };
        let (vrai, chemin_vrai) = appel(ca, dossier_a, reel).await;
        let (inexistant, _) = appel(ca, dossier_a, aleatoire).await;
        controle_refus(&vrai, a, etape, &chemin_vrai);
        assert_eq!(
            (vrai.statut, &vrai.corps),
            (inexistant.statut, &inexistant.corps),
            "{etape} : {} {chemin_vrai} se distingue d'une ressource inexistante",
            f.methode
        );
        let (croise, chemin_croise) = appel(cabinet_b, dossier_b, reel).await;
        let (croise_ref, _) = appel(cabinet_b, dossier_b, aleatoire).await;
        assert!(!croise.statut.is_success(), "{etape} : {chemin_croise}");
        assert_eq!(
            (croise.statut, &croise.corps),
            (croise_ref.statut, &croise_ref.corps),
            "{etape} : {} {chemin_croise} se distingue d'une ressource inexistante",
            f.methode
        );
        controle_absent(&croise.texte(), a, etape, &chemin_croise);
    }
    // Dépôt d'une pièce chez autrui.
    if let Some(d) = a.dossier {
        let r = m
            .deposer(
                &admin_b.session,
                &format!("/api/cabinets/{ca}/dossiers/{d}/pieces?nom=intrus.pdf"),
                b"%PDF-1.7\n% intrus\n".to_vec(),
            )
            .await;
        controle_refus(&r, a, etape, "dépôt de pièce");
    }
    // Ce que B voit de lui-même : ses cabinets et ses dossiers, rien de A.
    let r = m.lire(&admin_b.session, "/api/moi/cabinets").await;
    let texte = r.texte();
    assert_eq!(r.statut, StatusCode::OK, "{etape} : {texte}");
    assert!(texte.contains(&cabinet_b.to_string()), "{etape}");
    assert!(
        !texte.contains(&ca.to_string()),
        "{etape} : cabinet d'autrui listé"
    );
    let r = m
        .lire(
            &admin_b.session,
            &format!("/api/cabinets/{cabinet_b}/dossiers"),
        )
        .await;
    let texte = r.texte();
    assert_eq!(r.statut, StatusCode::OK, "{etape} : {texte}");
    assert!(texte.contains(&dossier_b.to_string()), "{etape}");
    controle_absent(&texte, a, etape, "liste des dossiers de B");
    // Rien n'a bougé chez A : écritures, brouillons, périodes, chaîne, audit, droits.
    assert_eq!(
        instantane(m, ca).await,
        avant,
        "{etape} : l'état du cabinet A a changé sous les requêtes de B"
    );
}

/// Une requête de B contre une famille de ressources de A.
struct Famille {
    genre: &'static str,
    methode: &'static str,
    chemin: &'static str,
    corps: Value,
}

/// Requêtes de B sur les familles de ressources de A déjà connues à cette étape.
fn familles(m: &Monde, a: &Secrets) -> Vec<Famille> {
    let (caisse, banque) = (
        comptes().caisse.as_str().to_owned(),
        comptes().banque.as_str().to_owned(),
    );
    let mut corrige = contenu(
        m,
        (1, 1),
        "Intrusion",
        &[(caisse, "debit", 1), (banque, "credit", 1)],
    );
    corrige["version"] = json!(1);
    let d = "/api/cabinets/{c}/dossiers/{d}";
    let tous = [
        (
            "brouillon",
            "GET",
            format!("{d}/brouillard/{{x}}"),
            Value::Null,
        ),
        (
            "brouillon",
            "POST",
            format!("{d}/brouillard/{{x}}/modification"),
            corrige,
        ),
        (
            "brouillon",
            "POST",
            format!("{d}/brouillard/{{x}}/validation"),
            json!({"version": 1}),
        ),
        (
            "brouillon",
            "POST",
            format!("{d}/brouillard/{{x}}/suppression"),
            json!({"version": 1}),
        ),
        (
            "brouillon",
            "POST",
            format!("{d}/brouillard/{{x}}/a-revoir"),
            json!({}),
        ),
        (
            "ecriture",
            "GET",
            format!("{d}/ecritures/{{x}}"),
            Value::Null,
        ),
        (
            "ecriture",
            "POST",
            format!("{d}/ecritures/{{x}}/contre-passation"),
            json!({"date": date(m, 1, 1), "libelle": "Intrusion", "motif": "Intrusion"}),
        ),
        (
            "piece",
            "GET",
            format!("{d}/pieces/{{x}}/contenu"),
            Value::Null,
        ),
        (
            "piece",
            "POST",
            format!("{d}/pieces/{{x}}/ecart"),
            json!({"motif": "Intrusion"}),
        ),
        (
            "periode",
            "POST",
            format!("{d}/periodes/{{x}}/verrouillage"),
            json!({}),
        ),
        (
            "periode",
            "POST",
            format!("{d}/periodes/{{x}}/deverrouillage"),
            json!({"motif": "Intrusion"}),
        ),
        (
            "periode",
            "POST",
            format!("{d}/periodes/{{x}}/cloture"),
            json!({"confirmation": "irreversible"}),
        ),
        (
            "compte",
            "POST",
            format!("{d}/comptes/{{x}}/intitule"),
            json!({"intitule": "Intrus"}),
        ),
        (
            "compte",
            "POST",
            format!("{d}/comptes/{{x}}/desactivation"),
            json!({}),
        ),
        (
            "membre",
            "GET",
            "/api/cabinets/{c}/membres/{x}/sessions".to_owned(),
            Value::Null,
        ),
        (
            "membre",
            "POST",
            "/api/cabinets/{c}/membres/{x}/role".to_owned(),
            json!({"role": "auditeur"}),
        ),
        (
            "membre",
            "POST",
            "/api/cabinets/{c}/membres/{x}/suspension".to_owned(),
            json!({}),
        ),
        (
            "invitation",
            "POST",
            "/api/cabinets/{c}/invitations/{x}/renvoi".to_owned(),
            json!({}),
        ),
        (
            "invitation",
            "DELETE",
            "/api/cabinets/{c}/invitations/{x}".to_owned(),
            Value::Null,
        ),
    ];
    tous.into_iter()
        .filter(|(genre, ..)| a.ressources.contains_key(genre))
        .map(|(genre, methode, chemin, corps)| Famille {
            genre,
            methode,
            // Les gabarits vivent le temps du test : fuite volontaire de petites chaînes.
            chemin: Box::leak(chemin.into_boxed_str()),
            corps,
        })
        .collect()
}

/// Nombre de requêtes de [`familles`] attendu selon les ressources connues (relevé indépendant).
fn familles_attendues(a: &Secrets) -> usize {
    [
        ("brouillon", 5),
        ("ecriture", 2),
        ("piece", 2),
        ("periode", 3),
        ("compte", 2),
        ("membre", 3),
        ("invitation", 2),
    ]
    .iter()
    .filter(|(f, _)| a.ressources.contains_key(f))
    .map(|(_, n)| n)
    .sum()
}

/// État observable du cabinet A, lu par le super-utilisateur : ce que les requêtes de B ne
/// doivent jamais changer.
async fn instantane(m: &Monde, cabinet: Uuid) -> Vec<String> {
    let mut etat = Vec::new();
    for table in [
        "dossier",
        "appartenance",
        "acces_dossier",
        "invitation",
        "compte",
        "journal",
        "tiers",
        "exercice",
        "periode",
        "brouillon",
        "brouillon_ligne",
        "brouillon_piece",
        "piece",
        "ecriture_validee",
        "ecriture_validee_ligne",
        "ecriture_validee_piece",
        "chaine_tete",
        "compteur_ecriture",
        "jeton_horodatage",
        "audit_cabinet",
        "audit_dossier",
    ] {
        let n: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
            "SELECT count(*) FROM {table} WHERE cabinet_id = $1"
        )))
        .bind(cabinet)
        .fetch_one(&m.env.admin)
        .await
        .unwrap();
        etat.push(format!("{table}={n}"));
    }
    for (nom, requete) in [
        (
            "periodes",
            "SELECT coalesce(string_agg(id::text || ':' || statut, ',' ORDER BY id), '') \
             FROM periode WHERE cabinet_id = $1",
        ),
        (
            "chaine",
            "SELECT coalesce(string_agg(rang::text || ':' || encode(empreinte, 'hex'), ',' \
                 ORDER BY dossier_id, rang), '') FROM ecriture_validee WHERE cabinet_id = $1",
        ),
        (
            "brouillons",
            "SELECT coalesce(string_agg(id::text || ':' || version::text, ',' ORDER BY id), '') \
             FROM brouillon WHERE cabinet_id = $1",
        ),
        (
            "roles",
            "SELECT coalesce(string_agg(utilisateur_id::text || ':' || role || ':' || actif::text, \
                 ',' ORDER BY utilisateur_id), '') FROM appartenance WHERE cabinet_id = $1",
        ),
    ] {
        let v: String = sqlx::query_scalar(requete)
            .bind(cabinet)
            .fetch_one(&m.env.admin)
            .await
            .unwrap();
        etat.push(format!("{nom}={v}"));
    }
    etat
}

fn controle_refus(r: &Reponse, a: &Secrets, etape: &str, chemin: &str) {
    assert!(
        r.statut == StatusCode::NOT_FOUND || r.statut == StatusCode::FORBIDDEN,
        "{etape} : {chemin} répond {} au lieu d'un refus : {}",
        r.statut,
        r.texte()
    );
    controle_absent(&r.texte(), a, etape, chemin);
}

fn controle_absent(texte: &str, a: &Secrets, etape: &str, chemin: &str) {
    let mut interdits = vec![a.cabinet.to_string()];
    interdits.extend(a.dossier.map(|d| d.to_string()));
    interdits.extend(a.noms.iter().cloned());
    for interdit in interdits {
        assert!(
            !texte.contains(&interdit),
            "{etape} : {chemin} révèle une donnée du cabinet d'en face"
        );
    }
}

// --- La démonstration ------------------------------------------------------------------------------

/// Base **dédiée** à la démonstration : créée, migrée et supprimée par le test. La file `tache`
/// est globale à l'instance ; sur une base partagée, le travailleur d'un autre test pourrait
/// prendre nos tâches (et nous les siennes), et nos alertes et courriels fausseraient les
/// comptes des tests de la file. Ici, rien n'est partagé : ni tâches, ni lignes, ni compteurs.
async fn base_dediee() -> (Env, String) {
    let racine = env().await;
    let nom = format!("sigma_test_demo_{}", Uuid::new_v4().simple());
    racine
        .admin
        .execute(sqlx::AssertSqlSafe(format!(
            "CREATE DATABASE {nom} OWNER sigma_migration"
        )))
        .await
        .unwrap();
    racine
        .admin
        .execute(sqlx::AssertSqlSafe(format!(
            "REVOKE ALL ON DATABASE {nom} FROM PUBLIC; \
             GRANT CONNECT ON DATABASE {nom} TO sigma_migration, sigma_application, sigma_lecture_audit"
        )))
        .await
        .unwrap();
    let options = |url: &str| PgConnectOptions::from_str(url).unwrap().database(&nom);
    let url_admin = variable("SIGMA_TEST_ADMIN_URL");
    // Migrations sous le rôle de migration, comme `sqlx migrate run`.
    let mut migration = PgConnection::connect_with(&options(&url_admin))
        .await
        .unwrap();
    migration.execute("SET ROLE sigma_migration").await.unwrap();
    MIGRATEUR.run(&mut migration).await.unwrap();
    migration.close().await.unwrap();
    let admin = PgPoolOptions::new()
        .max_connections(8)
        .connect_with(options(&url_admin))
        .await
        .unwrap();
    let application = PgPoolOptions::new()
        .max_connections(8)
        .connect_with(options(&variable("SIGMA_TEST_APPLICATION_URL")))
        .await
        .unwrap();
    let lecture_audit = PgPoolOptions::new()
        .max_connections(4)
        .after_connect(|connexion, _| {
            Box::pin(async move {
                connexion.execute("SET ROLE sigma_lecture_audit").await?;
                Ok(())
            })
        })
        .connect_with(options(&url_admin))
        .await
        .unwrap();
    verifier_migrations(&admin).await;
    (
        Env {
            admin,
            application,
            lecture_audit,
        },
        nom,
    )
}

#[tokio::test]
async fn demonstration_de_bout_en_bout_du_lot_l2() {
    let (env, nom) = base_dediee().await;
    let racine = commun::env().await;
    let pools = (
        env.admin.clone(),
        env.application.clone(),
        env.lecture_audit.clone(),
    );
    // Le scénario tourne dans une tâche locale : un échec ne laisse pas la base dédiée derrière
    // lui (supprimée dans tous les cas), et l'échec est ensuite rejoué tel quel.
    let local = tokio::task::LocalSet::new();
    let issue = local
        .run_until(async { tokio::task::spawn_local(scenario(Monde::nouveau(env))).await })
        .await;
    for pool in [pools.0, pools.1, pools.2] {
        pool.close().await;
    }
    racine
        .admin
        .execute(sqlx::AssertSqlSafe(format!(
            "DROP DATABASE IF EXISTS {nom} WITH (FORCE)"
        )))
        .await
        .unwrap();
    if let Err(erreur) = issue {
        std::panic::resume_unwind(erreur.into_panic());
    }
}

async fn scenario(m: Monde) {
    let c = comptes();
    let (caisse, banque, base) = (
        c.caisse.as_str().to_owned(),
        c.banque.as_str().to_owned(),
        c.base.as_str().to_owned(),
    );

    // Étape 1 : deux cabinets créés par la commande d'instance ; l'administrateur du premier
    // (A) a enrôlé une passkey et s'est connecté, l'autre (B) aussi.
    let (cabinet_a, admin_a) = m.creer_cabinet("Cabinet A de la démonstration").await;
    let (cabinet_b, admin_b) = m.creer_cabinet("Cabinet B de la démonstration").await;
    assert_ne!(cabinet_a, cabinet_b);
    let r = m.lire(&admin_a.session, "/api/moi/cabinets").await;
    assert!(
        r.texte().contains(&cabinet_a.to_string()) && !r.texte().contains(&cabinet_b.to_string())
    );
    // B a un dossier à lui, créé depuis le référentiel officiel, avec une saisie.
    let r = m
        .poster(
            &admin_b.session,
            &format!("/api/cabinets/{cabinet_b}/dossiers"),
            &json!({"nom": "Dossier du cabinet B", "pays": "SN", "nature": "reprise", "annee": annee(&m)}),
        )
        .await;
    let dossier_b: Uuid = attendre(&r, StatusCode::CREATED)["id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    let r = m
        .poster(
            &admin_b.session,
            &format!("/api/cabinets/{cabinet_b}/dossiers/{dossier_b}/brouillard"),
            &contenu(
                &m,
                (1, 9),
                "Saisie confidentielle de B",
                &[
                    (caisse.clone(), "debit", 777),
                    (banque.clone(), "credit", 777),
                ],
            ),
        )
        .await;
    attendre(&r, StatusCode::CREATED);
    let b = (cabinet_b, admin_b, dossier_b);
    let mut a = Secrets {
        cabinet: cabinet_a,
        dossier: None,
        noms: vec![],
        ressources: BTreeMap::new(),
    };
    isoler(&m, &b, &a, "1 cabinets créés").await;

    // Étape 2 : l'administrateur invite un valideur, un collaborateur et un auditeur ; chacun
    // accepte son lien (usage unique), enrôle sa passkey et se connecte.
    let valideur = m.inviter(cabinet_a, &admin_a, "valideur").await;
    let collaborateur = m.inviter(cabinet_a, &admin_a, "collaborateur").await;
    let mut auditeur = m.inviter(cabinet_a, &admin_a, "auditeur").await;
    a.noms = vec![valideur.courriel.clone(), collaborateur.courriel.clone()];
    a.ressources.insert("membre", valideur.id);
    a.ressources.insert("invitation", valideur.invitation);
    isoler(&m, &b, &a, "2 membres invités").await;

    // Étape 3 : l'administrateur crée un dossier depuis le référentiel officiel et y affecte les
    // trois membres (les droits se changent à session fraîche).
    let r = m
        .poster(
            &admin_a.session,
            &format!("/api/cabinets/{cabinet_a}/dossiers"),
            &json!({
                "nom": "Dossier A de la démonstration",
                "pays": "SN",
                "nature": "creation",
                "debut_activite": date(&m, 1, 1),
                "fin_premier_exercice": "annee_suivante"
            }),
        )
        .await;
    let dossier: Uuid = attendre(&r, StatusCode::CREATED)["id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    a.dossier = Some(dossier);
    a.noms.push("Dossier A de la démonstration".to_owned());
    for p in [&valideur, &collaborateur, &auditeur] {
        let r = m
            .poster(
                &admin_a.session,
                &format!("/api/cabinets/{cabinet_a}/membres/{}/acces", p.id),
                &json!({"dossier": dossier}),
            )
            .await;
        assert!(r.statut.is_success(), "{}", r.texte());
    }
    let racine = format!("/api/cabinets/{cabinet_a}/dossiers/{dossier}");
    let r = m
        .lire(&collaborateur.session, &format!("{racine}/comptes"))
        .await;
    let comptes_a = attendre(&r, StatusCode::OK);
    let nb_comptes = comptes_a["comptes"].as_array().unwrap().len();
    a.ressources.insert(
        "compte",
        comptes_a["comptes"][0]["id"]
            .as_str()
            .unwrap()
            .parse()
            .unwrap(),
    );
    let r = m
        .lire(&collaborateur.session, &format!("{racine}/exercices"))
        .await;
    a.ressources.insert(
        "periode",
        attendre(&r, StatusCode::OK)["exercices"][0]["periodes"][0]["id"]
            .as_str()
            .unwrap()
            .parse()
            .unwrap(),
    );
    assert!(
        nb_comptes > 100,
        "plan officiel copié : {nb_comptes} comptes"
    );
    isoler(&m, &b, &a, "3 dossier créé").await;

    // Étape 4 : saisie au brouillard par le collaborateur, avec pièces ; modification journalisée.
    let mut attendues: Vec<Mouvement> = Vec::new();
    let mut brouillons: Vec<(Uuid, i64, Vec<Mouvement>)> = Vec::new();
    let mut premiere_piece = None;
    for (jour, libelle, montant, contre) in [
        ((1, 1), "Vente au comptant du mois", 100_000, &caisse),
        ((1, 1), "Encaissement bancaire", 250_000, &banque),
        ((1, 1), "Dépôt de caisse", 40_000, &base),
    ] {
        let lignes: Vec<Mouvement> = vec![
            (caisse.clone(), "debit", montant),
            (
                if contre == &caisse {
                    banque.clone()
                } else {
                    contre.clone()
                },
                "credit",
                montant,
            ),
        ];
        let r = m
            .poster(
                &collaborateur.session,
                &format!("{racine}/brouillard"),
                &contenu(&m, jour, libelle, &lignes),
            )
            .await;
        let id: Uuid = attendre(&r, StatusCode::CREATED)["id"]
            .as_str()
            .unwrap()
            .parse()
            .unwrap();
        let r = m
            .deposer(
                &collaborateur.session,
                &format!("{racine}/pieces?nom=justificatif.pdf"),
                format!("%PDF-1.7\n% justificatif {}\n", Uuid::new_v4()).into_bytes(),
            )
            .await;
        let piece = attendre(&r, StatusCode::CREATED)["id"]
            .as_str()
            .unwrap()
            .to_owned();
        premiere_piece.get_or_insert_with(|| piece.parse::<Uuid>().unwrap());
        let r = m
            .poster(
                &collaborateur.session,
                &format!("{racine}/brouillard/{id}/pieces"),
                &json!({"piece": piece, "role": "principale", "version": 1}),
            )
            .await;
        let version = attendre(&r, StatusCode::OK)["version"].as_i64().unwrap();
        brouillons.push((id, version, lignes));
    }
    // Modification de la deuxième saisie : montant corrigé (avant 250 000, après 260 000).
    {
        let (id, version, lignes) = &mut brouillons[1];
        *lignes = vec![
            (caisse.clone(), "debit", 260_000),
            (banque.clone(), "credit", 260_000),
        ];
        let mut corps = contenu(&m, (1, 1), "Encaissement bancaire corrigé", lignes);
        corps["version"] = json!(*version);
        let r = m
            .poster(
                &collaborateur.session,
                &format!("{racine}/brouillard/{id}/modification"),
                &corps,
            )
            .await;
        *version = attendre(&r, StatusCode::OK)["version"].as_i64().unwrap();
        let journal: Vec<String> = sqlx::query_scalar(
            "SELECT contenu FROM audit_dossier WHERE dossier_id = $1 ORDER BY rang",
        )
        .bind(dossier)
        .fetch_all(&m.env.admin)
        .await
        .unwrap();
        assert!(
            journal
                .iter()
                .any(|e| e.contains("Encaissement bancaire corrigé")
                    && e.contains("Encaissement bancaire")),
            "la modification est journalisée avec son avant et son après"
        );
        assert!(
            journal
                .iter()
                .any(|e| e.contains("250000") && e.contains("260000")),
            "montant avant et après journalisés"
        );
    }
    // Une quatrième saisie, en février, reste au brouillard (mode provisoire).
    let r = m
        .poster(
            &collaborateur.session,
            &format!("{racine}/brouillard"),
            &contenu(
                &m,
                (2, 1),
                "Saisie de février",
                &[
                    (caisse.clone(), "debit", 5_000),
                    (banque.clone(), "credit", 5_000),
                ],
            ),
        )
        .await;
    let fevrier: Uuid = attendre(&r, StatusCode::CREATED)["id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    // Le collaborateur voit ses brouillons, l'administrateur de B n'en voit aucun.
    let r = m
        .lire(&collaborateur.session, &format!("{racine}/brouillard"))
        .await;
    assert_eq!(
        attendre(&r, StatusCode::OK)["brouillons"]
            .as_array()
            .unwrap()
            .len(),
        4
    );
    a.ressources.insert("brouillon", fevrier);
    a.ressources.insert("piece", premiere_piece.unwrap());
    isoler(&m, &b, &a, "4 brouillard saisi").await;

    // Étape 5 : validation unitaire par le valideur (la première), puis par lot (ré-authentifiée)
    // des deux suivantes ; chaque validation numérote et chaîne.
    let mut valideur = valideur;
    let (id1, v1, l1) = brouillons[0].clone();
    let r = m
        .poster(
            &valideur.session,
            &format!("{racine}/brouillard/{id1}/validation"),
            &json!({"version": v1}),
        )
        .await;
    let validee1 = attendre(&r, StatusCode::OK);
    assert_eq!(validee1["rang"], 1, "{validee1}");
    attendues.extend(l1.iter().cloned());
    let ecriture1: Uuid = validee1["id"].as_str().unwrap().parse().unwrap();
    a.ressources.insert("ecriture", ecriture1);
    // La session du valideur vieillit : la validation par lot exige une session fraîche.
    m.horloge.avancer(Duration::minutes(6));
    let lot: Vec<Value> = brouillons[1..3]
        .iter()
        .map(|(id, v, _)| json!({"ecriture": id, "version": v}))
        .collect();
    let r = m
        .poster(
            &valideur.session,
            &format!("{racine}/brouillard/validation-par-lot"),
            &json!({"ecritures": lot}),
        )
        .await;
    assert_eq!(r.statut, StatusCode::FORBIDDEN, "{}", r.texte());
    m.reauthentifier(&mut valideur).await;
    let r = m
        .poster(
            &valideur.session,
            &format!("{racine}/brouillard/validation-par-lot"),
            &json!({"ecritures": lot}),
        )
        .await;
    let par_lot = attendre(&r, StatusCode::OK);
    let rangs: Vec<i64> = par_lot["ecritures"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["rang"].as_i64().unwrap())
        .collect();
    assert_eq!(rangs, [2, 3], "{par_lot}");
    for (_, _, l) in &brouillons[1..3] {
        attendues.extend(l.iter().cloned());
    }
    isoler(&m, &b, &a, "5 écritures validées").await;

    // Étape 6 : contre-passation de la première écriture, puis une seconde (ADR 0021 : plusieurs
    // contre-passations d'une même écriture sont permises, chacune double l'annulation).
    for motif in ["Erreur de compte", "Doublon constaté"] {
        let r = m
            .poster(
                &valideur.session,
                &format!("{racine}/ecritures/{ecriture1}/contre-passation"),
                &json!({"date": date(&m, 1, 1), "libelle": "Contre-passation de la vente", "motif": motif}),
            )
            .await;
        attendre(&r, StatusCode::CREATED);
        attendues.extend(l1.iter().map(|(c, s, x)| {
            (
                c.clone(),
                if *s == "debit" { "credit" } else { "debit" },
                *x,
            )
        }));
    }
    let r = m
        .lire(&valideur.session, &format!("{racine}/ecritures"))
        .await;
    let liste = attendre(&r, StatusCode::OK);
    assert_eq!(liste["ecritures"].as_array().unwrap().len(), 5, "{liste}");
    isoler(&m, &b, &a, "6 contre-passations").await;

    // Étape 7 : verrouillage de la première période (celle du mois courant ; qui déclenche l'horodatage), le jeton est
    // obtenu auprès de l'autorité locale, puis clôture informatique (ré-authentifiée).
    let r = m
        .lire(&valideur.session, &format!("{racine}/exercices"))
        .await;
    let exercices = attendre(&r, StatusCode::OK);
    let janvier = exercices["exercices"][0]["periodes"][0]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(
        exercices["exercices"][0]["periodes"][0]["debut"],
        date(&m, 1, 1)
    );
    let r = m
        .poster(
            &valideur.session,
            &format!("{racine}/periodes/{janvier}/verrouillage"),
            &json!({}),
        )
        .await;
    assert_eq!(attendre(&r, StatusCode::OK)["statut"], "verrouillee");
    let taches: Vec<(String, String)> = sqlx::query_as(
        "SELECT nature, sujet FROM tache WHERE dossier_cible = $1 AND nature = 'horodatage' \
         ORDER BY sujet",
    )
    .bind(dossier)
    .fetch_all(&m.env.admin)
    .await
    .unwrap();
    assert_eq!(
        taches,
        [
            ("horodatage".to_owned(), "quotidien".to_owned()),
            ("horodatage".to_owned(), "verrouillage".to_owned())
        ],
        "la validation planifie le jeton quotidien, le verrouillage le jeton immédiat"
    );
    // La démonstration a SA base : la file `tache` n'a que nos tâches, aucun travailleur d'un
    // autre test ne peut en prendre une, et nous n'en prenons aucune d'autrui.
    let autorite = Arc::new(autorite::AutoriteLocale::nouvelle(Arc::new(
        m.horloge.clone(),
    )));
    let travailleur = Travailleur::new(
        m.env.application.clone(),
        autorite.clone(),
        Arc::new(m.horloge.clone()),
        ReglagesTache::default(),
    );
    let bilan = travailleur.passer().await.unwrap();
    assert_eq!((bilan.prises, bilan.jetons), (1, 1), "{bilan:?}");
    let etat_tache: String = sqlx::query_scalar(
        "SELECT etat FROM tache WHERE dossier_cible = $1 AND sujet = 'verrouillage'",
    )
    .bind(dossier)
    .fetch_one(&m.env.admin)
    .await
    .unwrap();
    assert_ne!(
        etat_tache, "en_attente",
        "la tâche de verrouillage est traitée"
    );
    let temoins = autorite.temoins();
    assert_eq!(temoins.len(), 1);
    assert!(
        temoins[0].contenu().contains(&dossier.to_string()),
        "témoin de notre dossier"
    );
    let jetons: i64 =
        sqlx::query_scalar("SELECT count(*) FROM jeton_horodatage WHERE dossier_id = $1")
            .bind(dossier)
            .fetch_one(&m.env.admin)
            .await
            .unwrap();
    assert_eq!(jetons, 1);
    // Clôture : session vieillie, refusée ; ré-authentifiée, acceptée.
    m.horloge.avancer(Duration::minutes(6));
    let cloture = format!("{racine}/periodes/{janvier}/cloture");
    let r = m
        .poster(
            &valideur.session,
            &cloture,
            &json!({"confirmation": "irreversible"}),
        )
        .await;
    assert_eq!(r.statut, StatusCode::FORBIDDEN, "{}", r.texte());
    m.reauthentifier(&mut valideur).await;
    let r = m
        .poster(
            &valideur.session,
            &cloture,
            &json!({"confirmation": "irreversible"}),
        )
        .await;
    assert_eq!(attendre(&r, StatusCode::OK)["statut"], "cloturee");
    // Une saisie datée de la période close est refusée.
    let r = m
        .poster(
            &collaborateur.session,
            &format!("{racine}/brouillard"),
            &contenu(
                &m,
                (1, 1),
                "Saisie tardive",
                &[(caisse.clone(), "debit", 1), (banque.clone(), "credit", 1)],
            ),
        )
        .await;
    assert!(
        r.statut == StatusCode::CONFLICT,
        "{} {}",
        r.statut,
        r.texte()
    );
    isoler(&m, &b, &a, "7 période close").await;

    // Étape 8 : éditions, relues par l'auditeur ; la balance à 6 colonnes est équilibrée et
    // égale à une somme **indépendante de l'API** (les mouvements que ce test a lui-même saisis).
    let mut attendu: BTreeMap<String, (i128, i128)> = BTreeMap::new();
    for (compte, sens, montant) in &attendues {
        let e = attendu.entry(compte.clone()).or_default();
        if *sens == "debit" {
            e.0 += i128::from(*montant);
        } else {
            e.1 += i128::from(*montant);
        }
    }
    let total_debit: i128 = attendu.values().map(|v| v.0).sum();
    let total_credit: i128 = attendu.values().map(|v| v.1).sum();
    assert_eq!(
        total_debit, total_credit,
        "les mouvements saisis s'équilibrent"
    );
    let r = m
        .lire(&auditeur.session, &format!("{racine}/editions/balance"))
        .await;
    let balance = attendre(&r, StatusCode::OK);
    assert_eq!(balance["format"], 6);
    assert_eq!(balance["equilibree"], true, "{balance}");
    let mut colonnes = [0i128; 6];
    let mut vus = BTreeSet::new();
    for l in balance["lignes"].as_array().unwrap() {
        let valeurs: Vec<i128> = l["valeurs"]
            .as_array()
            .unwrap()
            .iter()
            .map(decimal)
            .collect();
        assert_eq!(valeurs.len(), 6);
        for (c, v) in colonnes.iter_mut().zip(&valeurs) {
            *c += v;
        }
        let compte = l["cle"].as_str().unwrap().to_owned();
        let (d, c) = attendu
            .get(&compte)
            .copied()
            .unwrap_or_else(|| panic!("compte {compte} inattendu"));
        assert_eq!((valeurs[2], valeurs[3]), (d, c), "mouvements de {compte}");
        assert_eq!(
            (valeurs[4], valeurs[5]),
            ((d - c).max(0), (c - d).max(0)),
            "soldes de {compte}"
        );
        vus.insert(compte);
    }
    assert_eq!(vus, attendu.keys().cloned().collect::<BTreeSet<_>>());
    // Somme indépendante : ouvertures, mouvements et soldes s'équilibrent deux à deux.
    assert_eq!(colonnes[0], colonnes[1], "ouvertures");
    assert_eq!(
        (colonnes[2], colonnes[3]),
        (total_debit, total_credit),
        "mouvements"
    );
    assert_eq!(colonnes[4], colonnes[5], "soldes");
    assert_eq!(decimal(&balance["totaux"][2]), total_debit);
    // Provisoire : le brouillon de la période suivante s'ajoute.
    let r = m
        .lire(
            &auditeur.session,
            &format!("{racine}/editions/balance?mode=provisoire"),
        )
        .await;
    let provisoire = attendre(&r, StatusCode::OK);
    assert_eq!(decimal(&provisoire["totaux"][2]), total_debit + 5_000);
    assert_eq!(provisoire["equilibree"], true);
    // Grand livre, journaux, centralisateur : mêmes totaux.
    let r = m
        .lire(&auditeur.session, &format!("{racine}/editions/grand-livre"))
        .await;
    let grand_livre = attendre(&r, StatusCode::OK);
    for bloc in grand_livre["comptes"].as_array().unwrap() {
        let compte = bloc["compte"].as_str().unwrap();
        let (d, c) = attendu[compte];
        assert_eq!(
            (
                decimal(&bloc["total_debit"]),
                decimal(&bloc["total_credit"])
            ),
            (d, c),
            "grand livre de {compte}"
        );
    }
    let r = m
        .lire(&auditeur.session, &format!("{racine}/editions/journaux"))
        .await;
    let journaux = attendre(&r, StatusCode::OK);
    let (mut jd, mut jc, mut nb) = (0, 0, 0);
    for j in journaux["journaux"].as_array().unwrap() {
        jd += decimal(&j["total_debit"]);
        jc += decimal(&j["total_credit"]);
        nb += j["ecritures"].as_array().unwrap().len();
    }
    assert_eq!((jd, jc, nb), (total_debit, total_credit, 5));
    let r = m
        .lire(
            &auditeur.session,
            &format!("{racine}/editions/centralisateur"),
        )
        .await;
    let centralisateur = attendre(&r, StatusCode::OK);
    assert_eq!(
        decimal(&centralisateur["mouvements"]["total_debit"]),
        total_debit
    );
    assert_eq!(
        decimal(&centralisateur["mouvements"]["total_credit"]),
        total_credit
    );
    isoler(&m, &b, &a, "8 éditions").await;

    // Étape 9 : export d'intégrité (session fraîche exigée) puis `sigma-verifier`, jeton compris.
    m.horloge.avancer(Duration::minutes(6));
    let export = format!("{racine}/integrite/export");
    let r = m.poster(&auditeur.session, &export, &Value::Null).await;
    assert_eq!(r.statut, StatusCode::FORBIDDEN, "{}", r.texte());
    m.reauthentifier(&mut auditeur).await;
    let r = m.poster(&auditeur.session, &export, &Value::Null).await;
    assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
    let texte = r.texte();
    let (rang, tete): (i64, Vec<u8>) =
        sqlx::query_as("SELECT rang, empreinte FROM ecriture_validee WHERE dossier_id = $1 ORDER BY rang DESC LIMIT 1")
            .bind(dossier)
            .fetch_one(&m.env.admin)
            .await
            .unwrap();
    assert_eq!(rang, 5);
    let tete = integrite::Empreinte::depuis_octets(tete.try_into().unwrap()).to_string();
    let dossier_texte = dossier.to_string();
    let repertoire = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"));
    let sigle = Uuid::new_v4();
    let chemin_export = repertoire.join(format!("demonstration-{sigle}.txt"));
    let chemin_ancre = repertoire.join(format!("demonstration-{sigle}.ancre"));
    std::fs::write(&chemin_export, &texte).unwrap();
    std::fs::write(&chemin_ancre, autorite.certificat_der()).unwrap();
    let verifier = |fichier: &std::path::Path| {
        std::process::Command::new(env!("CARGO"))
            .args([
                "run",
                "--quiet",
                "--locked",
                "-p",
                "integrite",
                "--bin",
                "sigma-verifier",
                "--",
            ])
            .args(["--dossier", &dossier_texte, "--tete", &tete, "--ancre"])
            .arg(&chemin_ancre)
            .arg(fichier)
            .output()
            .unwrap()
    };
    let sortie = verifier(&chemin_export);
    let rapport = format!(
        "{}{}",
        String::from_utf8_lossy(&sortie.stdout),
        String::from_utf8_lossy(&sortie.stderr)
    );
    assert_eq!(sortie.status.code(), Some(0), "{rapport}");
    // Le test n'est pas vacueux : chaque défaut d'un export est détecté, et nommé. Une seule
    // ligne change (le montant d'un débit de la deuxième écriture), l'export reste bien formé.
    let sortie_texte = |sortie: &std::process::Output| {
        format!(
            "{}{}",
            String::from_utf8_lossy(&sortie.stdout),
            String::from_utf8_lossy(&sortie.stderr)
        )
    };
    let lignes: Vec<&str> = texte.lines().collect();
    let rang_ligne = lignes
        .iter()
        .position(|l| *l == "montant=260000")
        .expect("montant à altérer");
    let numero_altere = lignes[..rang_ligne]
        .iter()
        .rev()
        .find_map(|l| l.strip_prefix("numero="))
        .unwrap()
        .to_owned();
    let mut modifie: Vec<String> = lignes.iter().map(|l| (*l).to_owned()).collect();
    "montant=260001".clone_into(&mut modifie[rang_ligne]);
    let altere = repertoire.join(format!("demonstration-{sigle}-altere.txt"));
    std::fs::write(&altere, modifie.join("\n") + "\n").unwrap();
    let differences = lignes
        .iter()
        .zip(&modifie)
        .filter(|(avant, apres)| *avant != apres)
        .count();
    assert_eq!(differences, 1, "une seule ligne altérée");
    let sortie = verifier(&altere);
    let rapport = sortie_texte(&sortie);
    assert_eq!(sortie.status.code(), Some(1), "{rapport}");
    assert!(
        rapport.contains(&format!("- {numero_altere} (ligne")),
        "l'anomalie désigne l'écriture altérée {numero_altere} : {rapport}"
    );
    assert!(
        rapport.contains("empreinte différente du contenu"),
        "{rapport}"
    );
    // Seule l'écriture altérée est désignée parmi les écritures.
    for l in rapport
        .lines()
        .filter(|l| l.trim_start().starts_with("- AC-"))
    {
        assert!(l.contains(&numero_altere), "écriture saine désignée : {l}");
    }
    // Export tronqué (dernière ligne coupée) : refusé comme illisible, sans plus de détail.
    let tronque = repertoire.join(format!("demonstration-{sigle}-tronque.txt"));
    std::fs::write(&tronque, lignes[..lignes.len() - 4].join("\n")).unwrap();
    let sortie = verifier(&tronque);
    let rapport = sortie_texte(&sortie);
    assert_eq!(sortie.status.code(), Some(1), "{rapport}");
    assert!(rapport.contains("export illisible"), "{rapport}");
    // Ce qui n'est pas un export : même code de sortie, motif distinct (en-tête attendu).
    let informe = repertoire.join(format!("demonstration-{sigle}-informe.txt"));
    std::fs::write(&informe, "pas un export\n").unwrap();
    let sortie = verifier(&informe);
    let rapport = sortie_texte(&sortie);
    assert_eq!(sortie.status.code(), Some(1), "{rapport}");
    assert!(
        rapport.contains("export v5") && rapport.contains("attendu"),
        "{rapport}"
    );
    for f in [&chemin_export, &chemin_ancre, &altere, &tronque, &informe] {
        let _ = std::fs::remove_file(f);
    }
    isoler(&m, &b, &a, "9 export d'intégrité").await;

    // Étape 10 : journal d'audit complet, chaînes vérifiées, aucune donnée personnelle.
    let r = m
        .lire(
            &admin_a.session,
            &format!("/api/cabinets/{cabinet_a}/audit?limite=100"),
        )
        .await;
    let audit_cabinet = attendre(&r, StatusCode::OK);
    let r = m
        .lire(&admin_a.session, &format!("{racine}/audit?limite=100"))
        .await;
    let audit_dossier = attendre(&r, StatusCode::OK);
    let actions = |v: &Value| -> BTreeSet<String> {
        v["entrees"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["action"].as_str().unwrap().to_owned())
            .collect()
    };
    for (portee, chemin, page) in [
        (
            "cabinet",
            format!("/api/cabinets/{cabinet_a}/audit"),
            &audit_cabinet,
        ),
        ("dossier", format!("{racine}/audit"), &audit_dossier),
    ] {
        let r = m
            .lire(&admin_a.session, &format!("{chemin}/verification"))
            .await;
        let v = attendre(&r, StatusCode::OK);
        assert_eq!(v["integre"], true, "{portee} : {v}");
        assert_eq!(
            v["nombre"].as_u64().unwrap(),
            page["entrees"].as_array().unwrap().len() as u64,
            "{portee} : toutes les entrées sont lues"
        );
    }
    let attendues_cabinet = [
        "CreerCabinet",
        "UtiliserLienInvitation",
        "EnrolerPasskey",
        "OuvrirSession",
        "CreerDossier",
        "ModifierDroits",
    ];
    let attendues_dossier = [
        "CreerDossier",
        "ValiderEcriture",
        "ValiderParLot",
        "ContrePasser",
        "VerrouillerPeriode",
        "CloturerPeriode",
        "ObtenirJetonHorodatage",
        "ExporterIntegrite",
        "SaisirBrouillard",
        "CorrigerBrouillard",
        "DeposerPiece",
    ];
    let (ac, ad) = (actions(&audit_cabinet), actions(&audit_dossier));
    for action in attendues_cabinet {
        assert!(
            ac.contains(action),
            "audit du cabinet : {action} manque dans {ac:?}"
        );
    }
    for action in attendues_dossier {
        assert!(
            ad.contains(action),
            "audit du dossier : {action} manque dans {ad:?}"
        );
    }
    let contenus: Vec<String> = sqlx::query_scalar(
        "SELECT contenu FROM audit_cabinet WHERE cabinet_id = $1 \
         UNION ALL SELECT contenu FROM audit_dossier WHERE cabinet_id = $1",
    )
    .bind(cabinet_a)
    .fetch_all(&m.env.admin)
    .await
    .unwrap();
    let personnes = [&admin_a, &valideur, &collaborateur, &auditeur];
    for contenu in &contenus {
        assert!(!contenu.contains('@'), "courriel dans l'audit : {contenu}");
        for p in personnes {
            assert!(!contenu.contains(&p.courriel), "courriel dans l'audit");
        }
        assert!(!contenu.contains("Personne ") && !contenu.contains("Administrateur du test"));
    }
    // Rien n'a bougé chez B : son brouillon est intact et son audit ne cite rien de A.
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM brouillon WHERE cabinet_id = $1")
        .bind(cabinet_b)
        .fetch_one(&m.env.admin)
        .await
        .unwrap();
    assert_eq!(n, 1);
    isoler(&m, &b, &a, "10 audit").await;
    let _ = fevrier;
}

/// Entier d'un montant décimal rendu par l'API (« 1500 », ou « 1500.00 » : fraction nulle).
fn decimal(v: &Value) -> i128 {
    let texte = v
        .as_str()
        .unwrap_or_else(|| panic!("montant attendu : {v}"));
    let (entier, fraction) = texte.split_once('.').unwrap_or((texte, ""));
    assert!(
        fraction.chars().all(|c| c == '0'),
        "montant non entier : {texte}"
    );
    entier.parse().unwrap()
}
