//! Éditions et alertes par l'API (PR 14, L2-28 et L2-29) : balance générale (6 colonnes) et de
//! travail (2, 4, 8), grand livre, journaux, centralisateur, modes définitif et provisoire,
//! filtres, paramètres refusés, alertes. Les résultats de l'API sont comparés à ceux de
//! `compta-core` sur les **mêmes** écritures relues de la base.
//!
//! Isolation de chaque route : `tests/isolation/editions.rs`.
#![cfg(feature = "tests-postgres")]
// Fichier de test : les `expect`/`unwrap` y sont voulus.
#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::too_many_lines,
    clippy::similar_names,
    clippy::many_single_char_names,
    clippy::assertions_on_constants
)]

mod commun;

use std::sync::Arc;

use api::base::referentiel::charger;
use api::base::validation as base;
use api::base::{ContexteTenant, ouvrir_transaction};
use api::serveur::routes::brouillard::validation::reconstituer;
use api::serveur::{Application, routeur};
use authz::Autorisation;
use axum::Router;
use axum::http::StatusCode;
use commun::brouillard::{aujourd_hui, journal_de_saisie, ligne, texte_date};
use commun::referentiel::{comptes, doter};
use commun::serveur::{AuthentificationTest, HorlogeTest, Reponse, envoyer, lire};
use commun::{CabinetTest, Env, ajouter_affecte, creer_cabinet, env};
use compta_core::{
    Balance, Centralisateur, Filtre, FormatBalance, Mode, Regroupement, grand_livre, journaux,
};
use serde_json::Value;
use uuid::Uuid;

struct Banc {
    env: Env,
    c: CabinetTest,
    routeur: Router,
    horloge: HorlogeTest,
    valideur: api::base::IdUtilisateur,
    auditeur: api::base::IdUtilisateur,
}

async fn banc() -> Banc {
    banc_borne(None).await
}

/// Banc dont la borne des éditions est `borne` (réglage `editions_ecritures_max`).
async fn banc_borne(borne: Option<i64>) -> Banc {
    let env = env().await;
    let c = creer_cabinet(&env.admin, "Cabinet").await;
    doter(&env.application, c.cabinet, c.dossier, c.administrateur).await;
    let valideur = ajouter_affecte(&env.admin, &c, "valideur").await;
    let auditeur = ajouter_affecte(&env.admin, &c, "auditeur").await;
    let horloge = HorlogeTest::default();
    let mut reglages = commun::serveur::reglages_de_test();
    reglages.facteurs.cle_instance = Some(commun::serveur::cle_de_test());
    if let Some(borne) = borne {
        reglages.editions_ecritures_max = borne;
    }
    let etat = Application::avec_horloge(
        env.application.clone(),
        env.lecture_audit.clone(),
        Arc::new(Autorisation::charger().unwrap()),
        Arc::new(AuthentificationTest::default()),
        reglages,
        Arc::new(horloge.clone()),
    );
    commun::serveur::brancher_pieces_memoire(&etat);
    Banc {
        routeur: routeur(&etat),
        env,
        c,
        horloge,
        valideur,
        auditeur,
    }
}

fn caisse() -> &'static str {
    comptes().caisse.as_str()
}

fn banque() -> &'static str {
    comptes().banque.as_str()
}

impl Banc {
    fn racine(&self) -> String {
        format!(
            "/api/cabinets/{}/dossiers/{}",
            self.c.cabinet, self.c.dossier
        )
    }

    /// Brouillon (non validé) de `montant` XOF entre la caisse et la banque, daté du jour.
    async fn brouillon(&self, libelle: &str, montant: &str, debit: &str, credit: &str) -> Uuid {
        let corps = format!(
            r#"{{"journal":"{}","date":"{}","libelle":"{libelle}","lignes":[{},{}]}}"#,
            journal_de_saisie(),
            texte_date(aujourd_hui()),
            ligne(debit, "debit", montant),
            ligne(credit, "credit", montant),
        );
        let r = envoyer(
            &self.routeur,
            &format!("{}/brouillard", self.racine()),
            Some(self.c.collaborateur),
            &corps,
            None,
        )
        .await;
        assert_eq!(r.statut, StatusCode::CREATED, "{}", r.texte());
        r.json()["id"].as_str().unwrap().parse().unwrap()
    }

    /// Brouillon validé (pièce liée, validation par le valideur).
    async fn valider(&self, libelle: &str, montant: &str, debit: &str, credit: &str) -> Uuid {
        let id = self.brouillon(libelle, montant, debit, credit).await;
        let piece = commun::pieces::doter(
            &self.env.admin,
            self.c.cabinet,
            self.c.dossier,
            self.c.administrateur,
        )
        .await;
        let r = envoyer(
            &self.routeur,
            &format!("{}/brouillard/{id}/pieces", self.racine()),
            Some(self.c.collaborateur),
            &format!(r#"{{"piece":"{piece}","role":"principale","version":1}}"#),
            None,
        )
        .await;
        assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
        let version = r.json()["version"].as_i64().unwrap();
        let r = envoyer(
            &self.routeur,
            &format!("{}/brouillard/{id}/validation", self.racine()),
            Some(self.valideur),
            &format!(r#"{{"version":{version}}}"#),
            None,
        )
        .await;
        assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
        id
    }

    /// Contre-passe une écriture validée, aujourd'hui (valideur) ; rend l'identifiant de la
    /// contre-passation.
    async fn contre_passer(&self, id: Uuid) -> Uuid {
        let r = envoyer(
            &self.routeur,
            &format!("{}/ecritures/{id}/contre-passation", self.racine()),
            Some(self.valideur),
            &format!(
                r#"{{"date":"{}","libelle":"Contre-passation","motif":"Erreur de saisie"}}"#,
                texte_date(aujourd_hui())
            ),
            None,
        )
        .await;
        assert_eq!(r.statut, StatusCode::CREATED, "{}", r.texte());
        r.json()["id"].as_str().unwrap().parse().unwrap()
    }

    async fn editer(&self, suffixe: &str, utilisateur: api::base::IdUtilisateur) -> Reponse {
        lire(
            &self.routeur,
            &format!("{}/{suffixe}", self.racine()),
            Some(utilisateur),
        )
        .await
    }

    async fn ok(&self, suffixe: &str) -> Value {
        let r = self.editer(suffixe, self.c.administrateur).await;
        assert_eq!(r.statut, StatusCode::OK, "{suffixe} : {}", r.texte());
        r.json()
    }

    /// Les écritures validées, relues et reconstituées comme le fait l'API, et le filtre de
    /// l'exercice entier : de quoi calculer l'attendu avec `compta-core`.
    async fn attendu(&self) -> (Vec<compta_core::EcritureValidee>, Filtre) {
        let ids: Vec<Uuid> = sqlx::query_scalar(
            "SELECT id FROM ecriture_validee WHERE dossier_id = $1 ORDER BY rang",
        )
        .bind(self.c.dossier.uuid())
        .fetch_all(&self.env.admin)
        .await
        .unwrap();
        let mut tx = ouvrir_transaction(
            &self.env.application,
            ContexteTenant::dossier(self.c.cabinet, self.c.dossier, self.c.administrateur),
        )
        .await
        .unwrap();
        let mut validees = Vec::new();
        for id in ids {
            let e = base::lire(&mut tx, id).await.unwrap().unwrap();
            validees.push(reconstituer(&e, &commun::fiscal_sn(), &commun::tenue_xof()).unwrap());
        }
        let referentiel = charger(&mut tx).await.unwrap().unwrap();
        let exercice = referentiel.exercices().exercices().first().unwrap().clone();
        let filtre =
            Filtre::exercice(&exercice, &referentiel.etat().journaux, exercice.fin()).unwrap();
        tx.annuler().await.unwrap();
        (validees, filtre)
    }
}

fn decimal(v: &Value) -> String {
    v.as_str().unwrap().to_owned()
}

fn ligne_de<'a>(balance: &'a Value, cle: &str) -> &'a Value {
    balance["lignes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|l| l["cle"] == cle)
        .unwrap_or_else(|| panic!("ligne {cle} absente"))
}

/// Banc avec deux écritures validées (1 000 et 500 XOF, caisse au débit) et un brouillon
/// équilibré de 300.
async fn banc_garni() -> Banc {
    let b = banc().await;
    b.valider("Première", "1000", caisse(), banque()).await;
    b.valider("Seconde", "500", caisse(), banque()).await;
    b.brouillon("Brouillon équilibré", "300", caisse(), banque())
        .await;
    b
}

#[tokio::test]
async fn la_balance_generale_a_six_colonnes_egale_celle_de_compta_core() {
    let b = banc_garni().await;
    let v = b.ok("editions/balance").await;
    assert_eq!(v["format"], 6);
    assert_eq!(v["mode"], "definitif");
    assert_eq!(v["regroupement"], "compte");
    assert_eq!(
        v["colonnes"],
        serde_json::json!([
            "ouverture_debit",
            "ouverture_credit",
            "mouvements_debit",
            "mouvements_credit",
            "solde_debiteur",
            "solde_crediteur"
        ])
    );
    // Caisse : 1 500 au débit ; banque : 1 500 au crédit ; le brouillon n'y est pas.
    assert_eq!(
        v["lignes"].as_array().unwrap().len(),
        2,
        "{}",
        serde_json::to_string(&v["lignes"]).unwrap()
    );
    let attendu_caisse = ["0", "0", "1500", "0", "1500", "0"];
    let attendu_banque = ["0", "0", "0", "1500", "0", "1500"];
    for (cle, attendu) in [(caisse(), attendu_caisse), (banque(), attendu_banque)] {
        let valeurs: Vec<String> = ligne_de(&v, cle)["valeurs"]
            .as_array()
            .unwrap()
            .iter()
            .map(decimal)
            .collect();
        assert_eq!(valeurs, attendu, "{cle}");
    }
    assert_eq!(v["equilibree"], true);
    assert_eq!(
        v["totaux"],
        serde_json::json!(["0", "0", "1500", "1500", "1500", "1500"])
    );
    // Même résultat que `compta-core` sur les mêmes écritures.
    let (validees, filtre) = b.attendu().await;
    let attendue = Balance::calculer(
        &validees,
        &[],
        Mode::Definitif,
        &filtre,
        Regroupement::ParCompte,
    )
    .unwrap();
    assert_eq!(
        v["lignes"].as_array().unwrap().len(),
        attendue.lignes().len()
    );
    for l in attendue.lignes() {
        let colonnes: Vec<String> = l
            .colonnes(FormatBalance::Six)
            .unwrap()
            .iter()
            .map(|d| d.normalize().to_string())
            .collect();
        let relue: Vec<String> = ligne_de(&v, l.cle())["valeurs"]
            .as_array()
            .unwrap()
            .iter()
            .map(decimal)
            .collect();
        assert_eq!(relue, colonnes, "{}", l.cle());
    }
}

#[tokio::test]
async fn le_mode_provisoire_ajoute_le_brouillard_equilibre() {
    let b = banc_garni().await;
    let v = b.ok("editions/balance?mode=provisoire").await;
    assert_eq!(v["mode"], "provisoire");
    assert_eq!(
        decimal(&ligne_de(&v, caisse())["valeurs"][2]),
        "1800",
        "1 000 + 500 validés + 300 du brouillon"
    );
    assert_eq!(v["equilibree"], true);
    // Le mode définitif, lui, l'ignore.
    let d = b.ok("editions/balance?mode=definitif").await;
    assert_eq!(decimal(&ligne_de(&d, caisse())["valeurs"][2]), "1500");
}

#[tokio::test]
async fn les_balances_de_travail_ont_deux_quatre_et_huit_colonnes() {
    let b = banc_garni().await;
    for (format, noms) in [(2, 2usize), (4, 4), (8, 8)] {
        let v = b.ok(&format!("editions/balance?format={format}")).await;
        assert_eq!(v["format"], format);
        assert_eq!(v["colonnes"].as_array().unwrap().len(), noms);
        assert_eq!(v["totaux"].as_array().unwrap().len(), noms);
        for l in v["lignes"].as_array().unwrap() {
            assert_eq!(l["valeurs"].as_array().unwrap().len(), noms);
        }
        assert_eq!(v["equilibree"], true);
    }
    // Soldes (2 colonnes) : caisse débitrice, banque créditrice.
    let v = b.ok("editions/balance?format=2").await;
    assert_eq!(
        ligne_de(&v, caisse())["valeurs"],
        serde_json::json!(["1500", "0"])
    );
    assert_eq!(
        ligne_de(&v, banque())["valeurs"],
        serde_json::json!(["0", "1500"])
    );
}

#[tokio::test]
async fn le_regroupement_par_tronc_et_les_filtres() {
    let b = banc_garni().await;
    let (c, ba) = (caisse(), banque());
    // Par tronc : une clé de tronc officiel, jamais un numéro plus long que le compte.
    let v = b.ok("editions/balance?regroupement=tronc").await;
    for l in v["lignes"].as_array().unwrap() {
        let cle = l["cle"].as_str().unwrap();
        assert!(cle.len() <= c.len().max(ba.len()), "{cle}");
    }
    assert_eq!(v["regroupement"], "tronc");
    // Filtre de comptes : la caisse seule (la balance n'est plus équilibrée sans la banque).
    let v = b.ok(&format!("editions/balance?comptes={c}")).await;
    assert_eq!(v["lignes"].as_array().unwrap().len(), 1);
    assert_eq!(v["lignes"][0]["cle"], c);
    assert_eq!(v["equilibree"], false);
    // Filtre de journaux : le journal de saisie, puis un journal sans écriture.
    let j = journal_de_saisie();
    let v = b.ok(&format!("editions/balance?journaux={j}")).await;
    assert_eq!(v["lignes"].as_array().unwrap().len(), 2);
    // Périodes déterministes (les écritures sont datées d'aujourd'hui) : l'exercice a au moins
    // un jour avant ou après aujourd'hui. Un jour **après** aujourd'hui : les écritures sont
    // dans l'ouverture (1 500 de part et d'autre), aucun mouvement. Un jour **avant** : elles
    // sont postérieures à la période, la balance est vide.
    let tout = b.ok("editions/balance").await;
    let (debut, fin) = (
        tout["du"].as_str().unwrap().to_owned(),
        tout["au"].as_str().unwrap().to_owned(),
    );
    let exercice = tout["exercice"].as_str().unwrap().to_owned();
    let aujourdhui = texte_date(aujourd_hui());
    let mut verifie = 0;
    if fin > aujourdhui {
        let v = b
            .ok(&format!(
                "editions/balance?exercice={exercice}&du={fin}&au={fin}"
            ))
            .await;
        assert_eq!(
            ligne_de(&v, caisse())["valeurs"],
            serde_json::json!(["1500", "0", "0", "0", "1500", "0"]),
            "ouverture seule"
        );
        verifie += 1;
    }
    if debut < aujourdhui {
        let v = b
            .ok(&format!(
                "editions/balance?exercice={exercice}&du={debut}&au={debut}"
            ))
            .await;
        assert_eq!(v["lignes"].as_array().unwrap().len(), 0, "après la période");
        verifie += 1;
    }
    assert!(verifie >= 1, "exercice d'un seul jour, aujourd'hui");
}

#[tokio::test]
async fn les_parametres_invalides_sont_refuses() {
    let b = banc_garni().await;
    for mauvais in [
        "editions/balance?format=3",
        "editions/balance?mode=autre",
        "editions/balance?regroupement=autre",
        "editions/balance?inconnu=1",
        "editions/balance?mode=definitif&mode=provisoire",
        "editions/balance?du=2026-13-01",
        "editions/balance?du=2026-06-01&au=2026-01-01",
        "editions/balance?du=1999-01-01",
        "editions/balance?comptes=",
        "editions/balance?comptes=abc",
        "editions/balance?journaux=minuscule",
        "editions/balance?exercice=1900",
        // Le format et le regroupement ne concernent que la balance ; le filtre de comptes ne
        // concerne pas les journaux (une écriture y figure en entier).
        "editions/grand-livre?format=6",
        "editions/journaux?comptes=1",
        "editions/centralisateur?regroupement=tronc",
        "alertes?mode=provisoire",
    ] {
        let r = b.editer(mauvais, b.c.administrateur).await;
        assert_eq!(
            r.statut,
            StatusCode::BAD_REQUEST,
            "{mauvais} : {}",
            r.texte()
        );
    }
}

#[tokio::test]
async fn le_grand_livre_donne_les_soldes_progressifs_et_egale_compta_core() {
    let b = banc_garni().await;
    let v = b.ok("editions/grand-livre").await;
    let comptes_vus = v["comptes"].as_array().unwrap();
    let bloc = comptes_vus
        .iter()
        .find(|c| c["compte"] == caisse())
        .unwrap();
    assert_eq!(bloc["solde_ouverture"], "0");
    let lignes = bloc["lignes"].as_array().unwrap();
    assert_eq!(lignes.len(), 2);
    assert_eq!(lignes[0]["debit"], "1000");
    assert_eq!(lignes[0]["solde"], "1000");
    assert_eq!(lignes[1]["debit"], "500");
    assert_eq!(lignes[1]["solde"], "1500");
    assert!(
        lignes[0]["numero"].is_string(),
        "écriture validée numérotée"
    );
    assert_eq!(bloc["total_debit"], "1500");
    assert_eq!(bloc["solde_final"], "1500");
    let (validees, filtre) = b.attendu().await;
    let attendu = grand_livre(&validees, &[], Mode::Definitif, &filtre).unwrap();
    assert_eq!(comptes_vus.len(), attendu.len());
    for c in &attendu {
        let relu = comptes_vus
            .iter()
            .find(|x| x["compte"] == c.compte.as_str())
            .unwrap();
        assert_eq!(relu["solde_final"], c.solde_final.normalize().to_string());
        assert_eq!(relu["lignes"].as_array().unwrap().len(), c.lignes.len());
    }
    // Provisoire : le brouillon s'ajoute en fin de journée, sans numéro.
    let p = b.ok("editions/grand-livre?mode=provisoire").await;
    let bloc = p["comptes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["compte"] == caisse())
        .unwrap();
    let lignes = bloc["lignes"].as_array().unwrap();
    assert_eq!(lignes.len(), 3);
    assert!(lignes[2]["numero"].is_null());
    assert_eq!(bloc["solde_final"], "1800");
}

#[tokio::test]
async fn les_journaux_et_le_centralisateur_totalisent_les_ecritures() {
    let b = banc_garni().await;
    let v = b.ok("editions/journaux").await;
    let blocs = v["journaux"].as_array().unwrap();
    assert_eq!(blocs.len(), 1);
    assert_eq!(blocs[0]["journal"], journal_de_saisie());
    assert_eq!(blocs[0]["ecritures"].as_array().unwrap().len(), 2);
    assert_eq!(blocs[0]["total_debit"], "1500");
    assert_eq!(blocs[0]["total_credit"], "1500");
    let (validees, filtre) = b.attendu().await;
    assert_eq!(
        journaux(&validees, &[], Mode::Definitif, &filtre)
            .unwrap()
            .len(),
        blocs.len()
    );
    let p = b.ok("editions/journaux?mode=provisoire").await;
    assert_eq!(p["journaux"][0]["ecritures"].as_array().unwrap().len(), 3);

    let c = b.ok("editions/centralisateur").await;
    let m = &c["mouvements"];
    assert_eq!(m["par_journal"].as_array().unwrap().len(), 1);
    assert_eq!(m["par_journal"][0]["debit"], "1500");
    assert_eq!(m["par_journal"][0]["credit"], "1500");
    assert_eq!(m["total_debit"], m["total_credit"]);
    assert_eq!(c["a_nouveaux"]["par_journal"].as_array().unwrap().len(), 0);
    let attendu = Centralisateur::calculer(&validees, &[], Mode::Definitif, &filtre).unwrap();
    let (td, tc) = attendu.mouvements().total();
    assert_eq!(m["total_debit"], td.normalize().to_string());
    assert_eq!(m["total_credit"], tc.normalize().to_string());
}

#[tokio::test]
async fn tous_les_roles_du_cabinet_consultent_et_aucun_ne_modifie() {
    let b = banc_garni().await;
    for u in [
        b.c.administrateur,
        b.valideur,
        b.c.collaborateur,
        b.auditeur,
    ] {
        for chemin in [
            "editions/balance",
            "editions/grand-livre",
            "editions/journaux",
            "editions/centralisateur",
            "alertes",
        ] {
            let r = b.editer(chemin, u).await;
            assert_eq!(r.statut, StatusCode::OK, "{chemin} : {}", r.texte());
        }
    }
    // Lecture seule : une édition ne change rien (ni écriture ni compteur).
    let avant: i64 =
        sqlx::query_scalar("SELECT count(*) FROM ecriture_validee WHERE dossier_id = $1")
            .bind(b.c.dossier.uuid())
            .fetch_one(&b.env.admin)
            .await
            .unwrap();
    b.ok("editions/balance?mode=provisoire").await;
    let apres: i64 =
        sqlx::query_scalar("SELECT count(*) FROM ecriture_validee WHERE dossier_id = $1")
            .bind(b.c.dossier.uuid())
            .fetch_one(&b.env.admin)
            .await
            .unwrap();
    assert_eq!(avant, apres);
    // Les méthodes d'écriture n'existent pas sur ces ressources.
    let r = envoyer(
        &b.routeur,
        &format!("{}/editions/balance", b.racine()),
        Some(b.c.administrateur),
        "{}",
        None,
    )
    .await;
    assert_eq!(r.statut, StatusCode::METHOD_NOT_ALLOWED, "{}", r.texte());
}

#[tokio::test]
async fn les_alertes_signalent_le_brouillard_en_retard_et_l_echeance_de_cloture() {
    let b = banc_garni().await;
    // Aujourd'hui : aucune période n'est terminée, rien en retard.
    let v = b.ok("alertes").await;
    assert_eq!(v["brouillard_non_valide"].as_array().unwrap().len(), 0);
    // Quatre mois plus tard : la période du brouillon est terminée de longue date.
    let futur = aujourd_hui().replace_day(1).unwrap() + time::Duration::days(120);
    b.horloge
        .fixer(time::PrimitiveDateTime::new(futur, time::Time::MIDNIGHT).assume_utc());
    // Un second brouillon du même jour et du même journal : l'alerte les compte ensemble.
    b.brouillon("Brouillon bis", "50", caisse(), banque()).await;
    let v = b.ok("alertes").await;
    assert_eq!(v["date_du_jour"], texte_date(futur));
    let brouillard = v["brouillard_non_valide"].as_array().unwrap();
    assert_eq!(brouillard.len(), 1, "{v}");
    assert_eq!(brouillard[0]["journal"], journal_de_saisie());
    assert_eq!(brouillard[0]["nombre"], 2);
    assert!(brouillard[0]["jours_de_retard"].as_i64().unwrap() >= 1);
    let echeances = v["echeances_de_cloture"].as_array().unwrap();
    assert!(!echeances.is_empty(), "{v}");
    for e in echeances {
        assert!(e["jours_restants"].as_i64().unwrap() <= 30);
        assert_eq!(
            e["depassee"].as_bool().unwrap(),
            e["jours_restants"].as_i64().unwrap() < 0
        );
        assert_ne!(e["statut"], "cloturee");
    }
}

#[tokio::test]
async fn les_editions_simultanees_au_dela_du_pool_aboutissent_toutes() {
    // Le pool du rôle applicatif des tests a 4 connexions ; chaque requête en tient une (la
    // transaction du pipeline) et lit l'instantané sur le pool des lectures : 16 éditions
    // simultanées, quatre fois la taille du pool, doivent toutes réussir (aucun interblocage).
    let b = banc_garni().await;
    let mut taches = tokio::task::JoinSet::new();
    for i in 0..16 {
        let routeur = b.routeur.clone();
        let chemin = format!(
            "{}/{}",
            b.racine(),
            [
                "editions/balance?mode=provisoire",
                "editions/grand-livre",
                "alertes",
                "editions/journaux"
            ][i % 4]
        );
        let utilisateur = b.c.administrateur;
        taches.spawn(async move { lire(&routeur, &chemin, Some(utilisateur)).await });
    }
    let mut reussies = 0;
    while let Some(r) = taches.join_next().await {
        let r = r.unwrap();
        assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
        reussies += 1;
    }
    assert_eq!(reussies, 16);
}

#[tokio::test]
async fn une_edition_trop_volumineuse_est_refusee_en_409() {
    // Borne de 2 écritures : deux validées passent, la troisième fait refuser l'édition.
    let b = banc_borne(Some(2)).await;
    b.valider("Première", "1000", caisse(), banque()).await;
    b.valider("Seconde", "500", caisse(), banque()).await;
    let r = b.editer("editions/balance", b.c.administrateur).await;
    assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
    b.valider("Troisième", "100", caisse(), banque()).await;
    for chemin in [
        "editions/balance",
        "editions/grand-livre",
        "editions/journaux",
        "editions/centralisateur",
    ] {
        let r = b.editer(chemin, b.c.administrateur).await;
        assert_eq!(r.statut, StatusCode::CONFLICT, "{chemin} : {}", r.texte());
        assert_eq!(r.json()["erreur"], "edition_trop_volumineuse");
        assert_eq!(r.json()["nombre"], 2);
    }
    // Restreindre la période n'y change rien ici (tout est daté d'aujourd'hui) ; mais le brouillard
    // compte aussi, séparément, en mode provisoire.
    let b = banc_borne(Some(1)).await;
    b.valider("Une", "1000", caisse(), banque()).await;
    b.brouillon("Un", "300", caisse(), banque()).await;
    b.brouillon("Deux", "200", caisse(), banque()).await;
    let r = b
        .editer("editions/balance?mode=provisoire", b.c.administrateur)
        .await;
    assert_eq!(r.statut, StatusCode::CONFLICT, "{}", r.texte());
    let r = b.editer("editions/balance", b.c.administrateur).await;
    assert_eq!(
        r.statut,
        StatusCode::OK,
        "le définitif ignore le brouillard"
    );
}

#[tokio::test]
async fn le_brouillard_desequilibre_ou_illisible_est_exclu_et_signale() {
    let b = banc_garni().await;
    // Un brouillon déséquilibré : débit 300, crédit 200.
    let corps = format!(
        r#"{{"journal":"{}","date":"{}","libelle":"Déséquilibré","lignes":[{},{}]}}"#,
        journal_de_saisie(),
        texte_date(aujourd_hui()),
        ligne(caisse(), "debit", "300"),
        ligne(banque(), "credit", "200"),
    );
    let r = envoyer(
        &b.routeur,
        &format!("{}/brouillard", b.racine()),
        Some(b.c.collaborateur),
        &corps,
        None,
    )
    .await;
    assert_eq!(r.statut, StatusCode::CREATED, "{}", r.texte());
    // Un brouillon illisible : un régime fiscal que le référentiel ne connaît pas.
    let id = b.brouillon("Illisible", "100", caisse(), banque()).await;
    sqlx::query(
        "UPDATE brouillon_ligne SET fiscal_nature = 'base', fiscal_sens = 'collectee', \
         fiscal_regime = 'regime-retire' WHERE brouillon_id = $1 AND position = 1",
    )
    .bind(id)
    .execute(&b.env.admin)
    .await
    .unwrap();
    let v = b.ok("editions/balance?mode=provisoire").await;
    // Seul le brouillon équilibré et lisible de `banc_garni` (300) s'ajoute.
    assert_eq!(decimal(&ligne_de(&v, caisse())["valeurs"][2]), "1800");
    assert_eq!(v["brouillons_exclus"]["desequilibres"], 1);
    assert_eq!(v["brouillons_exclus"]["illisibles"], 1);
    // En définitif : rien à signaler.
    let d = b.ok("editions/balance").await;
    assert_eq!(d["brouillons_exclus"]["desequilibres"], 0);
    assert_eq!(d["brouillons_exclus"]["illisibles"], 0);
    // Les autres éditions provisoires ne tombent pas non plus.
    for chemin in [
        "editions/grand-livre?mode=provisoire",
        "editions/journaux?mode=provisoire",
        "editions/centralisateur?mode=provisoire",
    ] {
        let r = b.editer(chemin, b.c.administrateur).await;
        assert_eq!(r.statut, StatusCode::OK, "{chemin} : {}", r.texte());
    }
}

#[tokio::test]
async fn une_ecriture_anterieure_a_l_exercice_n_entre_pas_dans_ses_editions() {
    let b = banc_garni().await;
    let (exercice, journal, debut): (Uuid, Uuid, time::Date) = sqlx::query_as(
        "SELECT x.id, j.id, x.debut FROM exercice x, journal j \
         WHERE x.dossier_id = $1 AND j.dossier_id = $1 AND j.code = $2 \
         ORDER BY x.debut LIMIT 1",
    )
    .bind(b.c.dossier.uuid())
    .bind(journal_de_saisie())
    .fetch_one(&b.env.admin)
    .await
    .unwrap();
    // Écriture validée de 1 000 la veille du premier jour de l'exercice (insérée par le
    // super-utilisateur : aucune route ne date une écriture hors exercice).
    let veille = debut - time::Duration::days(1);
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO ecriture_validee (cabinet_id, dossier_id, id, journal_id, exercice_id, \
         sequence, numero, date_comptable, libelle, format, contenu_canonique, empreinte, \
         empreinte_precedente, rang, valide_par, date_validation, valide_le) \
         VALUES ($1, $2, $3, $4, $5, 900, 'AC-0000-000900', $6, 'Antérieure', 1, 'fictif', $7, \
                 $8, 900, $9, $6, now())",
    )
    .bind(b.c.cabinet.uuid())
    .bind(b.c.dossier.uuid())
    .bind(id)
    .bind(journal)
    .bind(exercice)
    .bind(veille)
    .bind(vec![9u8; 32])
    .bind(vec![8u8; 32])
    .bind(b.c.administrateur.uuid())
    .execute(&b.env.admin)
    .await
    .unwrap();
    for (position, compte, sens) in [(1i16, caisse(), "debit"), (2, banque(), "credit")] {
        sqlx::query(
            "INSERT INTO ecriture_validee_ligne (cabinet_id, dossier_id, ecriture_id, position, \
             compte_numero, sens, montant) VALUES ($1, $2, $3, $4, $5, $6, 1000)",
        )
        .bind(b.c.cabinet.uuid())
        .bind(b.c.dossier.uuid())
        .bind(id)
        .bind(position)
        .bind(compte)
        .bind(sens)
        .execute(&b.env.admin)
        .await
        .unwrap();
    }
    // Les éditions de l'exercice ne la voient ni en mouvements ni en ouverture : toujours 1 500.
    let v = b.ok("editions/balance").await;
    assert_eq!(
        ligne_de(&v, caisse())["valeurs"],
        serde_json::json!(["0", "0", "1500", "0", "1500", "0"])
    );
    let gl = b.ok("editions/grand-livre").await;
    let bloc = gl["comptes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["compte"] == caisse())
        .unwrap();
    assert_eq!(bloc["solde_ouverture"], "0");
    assert_eq!(bloc["solde_final"], "1500");
}

#[tokio::test]
async fn une_ecriture_et_sa_contre_passation_soldent_la_balance_dans_tous_les_modes() {
    let b = banc().await;
    let id = b.valider("À annuler", "1000", caisse(), banque()).await;
    let contre = b.contre_passer(id).await;
    assert_ne!(contre, id);
    // Une contre-passation est une écriture validée comme une autre : elle figure dans les
    // éditions, et l'origine se reconstitue (sinon `compta-core` la refuserait : 500).
    for suffixe in [
        "editions/balance",
        "editions/balance?mode=provisoire",
        "editions/balance?regroupement=tronc",
    ] {
        let v = b.ok(suffixe).await;
        let lignes = v["lignes"].as_array().unwrap();
        assert!(!lignes.is_empty(), "{suffixe}");
        for l in lignes {
            let valeurs: Vec<String> = l["valeurs"]
                .as_array()
                .unwrap()
                .iter()
                .map(decimal)
                .collect();
            // 6 colonnes : mouvements 1 000 / 1 000, soldes nuls.
            assert_eq!(valeurs[2], "1000", "{suffixe} {l}");
            assert_eq!(valeurs[3], "1000", "{suffixe} {l}");
            assert_eq!(valeurs[4], "0", "{suffixe} {l}");
            assert_eq!(valeurs[5], "0", "{suffixe} {l}");
        }
        assert_eq!(v["equilibree"], true);
        assert_eq!(
            v["totaux"],
            serde_json::json!(["0", "0", "2000", "2000", "0", "0"])
        );
    }
    // Grand livre, journaux, centralisateur : deux écritures, soldes finaux nuls.
    let gl = b.ok("editions/grand-livre").await;
    for c in gl["comptes"].as_array().unwrap() {
        assert_eq!(c["lignes"].as_array().unwrap().len(), 2, "{c}");
        assert_eq!(c["solde_final"], "0");
    }
    let j = b.ok("editions/journaux").await;
    assert_eq!(j["journaux"][0]["ecritures"].as_array().unwrap().len(), 2);
    assert_eq!(j["journaux"][0]["total_debit"], "2000");
    let c = b.ok("editions/centralisateur").await;
    assert_eq!(c["mouvements"]["total_debit"], "2000");
    assert_eq!(c["mouvements"]["total_credit"], "2000");
    // Contre-passer la contre-passation : l'origine est elle-même une contre-passation, les
    // éditions la relisent encore, et l'effet de l'écriture initiale est rétabli (soldes 1 000).
    b.contre_passer(contre).await;
    let v = b.ok("editions/balance").await;
    assert_eq!(
        v["totaux"],
        serde_json::json!(["0", "0", "3000", "3000", "1000", "1000"])
    );
}
