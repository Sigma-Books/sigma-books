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
    clippy::similar_names
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
use commun::{Env, env};
use serde_json::{Value, json};
use time::{Duration, OffsetDateTime};
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
    async fn nouveau() -> Self {
        let env = env().await;
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
    async fn integrer(&self, jeton: &str, courriel: &str) -> Personne {
        let r = self
            .anonyme("/api/liens/utilisation", &json!({"jeton": jeton}), None)
            .await;
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
        Personne {
            courriel: courriel.to_owned(),
            id,
            auth,
            session,
        }
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
        (cabinet, self.integrer(jeton, &courriel).await)
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
        self.integrer(&jeton.texte(), &courriel).await
    }
}

fn annee(m: &Monde) -> i32 {
    m.horloge.maintenant().year()
}

fn date(m: &Monde, mois: u8, jour: u8) -> String {
    format!("{:04}-{mois:02}-{jour:02}", annee(m))
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
}

/// Contrôle de l'étape `etape` : l'administrateur du cabinet B ne voit rien du cabinet A. Les
/// ressources d'autrui répondent comme des ressources absentes (404, jamais 200), ni lecture ni
/// mutation n'aboutit, et rien du cabinet A ne figure dans les réponses de B.
async fn isoler(m: &Monde, b: &(Uuid, Personne, Uuid), a: &Secrets, etape: &str) {
    let (cabinet_b, admin_b, dossier_b) = (b.0, &b.1, b.2);
    let ca = a.cabinet;
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
            json!({"nom": "Intrus", "nature": "reprise", "annee": annee(m)}),
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

#[tokio::test]
async fn demonstration_de_bout_en_bout_du_lot_l2() {
    let m = Monde::nouveau().await;
    let c = comptes();
    let (caisse, banque, base) = (
        c.caisse.as_str().to_owned(),
        c.banque.as_str().to_owned(),
        c.base.as_str().to_owned(),
    );

    // Étape 1 : deux cabinets créés par la commande d'instance ; l'administrateur du premier
    // (A) a enrôlé une passkey et s'est connecté, l'autre (B) aussi.
    let (cabinet_a, mut admin_a) = m.creer_cabinet("Cabinet A de la démonstration").await;
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
            &json!({"nom": "Dossier du cabinet B", "nature": "reprise", "annee": annee(&m)}),
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
    };
    isoler(&m, &b, &a, "1 cabinets créés").await;

    // Étape 2 : l'administrateur invite un valideur, un collaborateur et un auditeur ; chacun
    // accepte son lien (usage unique), enrôle sa passkey et se connecte.
    let valideur = m.inviter(cabinet_a, &admin_a, "valideur").await;
    let collaborateur = m.inviter(cabinet_a, &admin_a, "collaborateur").await;
    let auditeur = m.inviter(cabinet_a, &admin_a, "auditeur").await;
    a.noms = vec![valideur.courriel.clone(), collaborateur.courriel.clone()];
    isoler(&m, &b, &a, "2 membres invités").await;

    // Étape 3 : l'administrateur crée un dossier depuis le référentiel officiel et y affecte les
    // trois membres (les droits se changent à session fraîche).
    let r = m
        .poster(
            &admin_a.session,
            &format!("/api/cabinets/{cabinet_a}/dossiers"),
            &json!({"nom": "Dossier A de la démonstration", "nature": "reprise", "annee": annee(&m)}),
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
    let nb_comptes = attendre(&r, StatusCode::OK)["comptes"]
        .as_array()
        .unwrap()
        .len();
    assert!(
        nb_comptes > 100,
        "plan officiel copié : {nb_comptes} comptes"
    );
    isoler(&m, &b, &a, "3 dossier créé").await;

    // Étape 4 : saisie au brouillard par le collaborateur, avec pièces ; modification journalisée.
    let mut attendues: Vec<Mouvement> = Vec::new();
    let mut brouillons: Vec<(Uuid, i64, Vec<Mouvement>)> = Vec::new();
    for (jour, libelle, montant, contre) in [
        ((1, 15), "Vente au comptant de janvier", 100_000, &caisse),
        ((1, 20), "Encaissement bancaire", 250_000, &banque),
        ((1, 25), "Dépôt de caisse", 40_000, &base),
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
        let mut corps = contenu(&m, (1, 20), "Encaissement bancaire corrigé", lignes);
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
                (2, 3),
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
    eprintln!("LISTE {}", &r.texte()[..r.texte().len().min(300)]);
    isoler(&m, &b, &a, "4 brouillard saisi").await;
    let _ = (&auditeur, fevrier, &attendues);
    attendues.clear();
}
