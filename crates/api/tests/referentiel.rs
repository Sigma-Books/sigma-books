//! Référentiel d'un dossier contre un PostgreSQL réel (L2-22, PR 11a) : droits des tables,
//! création d'un dossier (entité nouvelle, dossier repris), atomicité, aller-retour du plan
//! officiel complet, contrôle croisé de chaque mutation par sa route, audit, verrou.
//!
//! Aucun numéro de compte écrit en dur : tous viennent du référentiel officiel
//! (`commun::referentiel::comptes`).
#![cfg(feature = "tests-postgres")]
#![allow(clippy::expect_used, clippy::unwrap_used)]

mod commun;

use api::base::organisation::creer_dossier;
use api::base::referentiel::{
    ErreurMutation, EtatReferentiel, Instanciation, Modele, Ouverture, RefusReferentiel, Tiers,
    TypeTiers, aplatir_plan, charger, instancier, muter, reconstituer_plan, verrouiller,
};
use api::base::{ContexteTenant, IdDossier, ouvrir_transaction};
use api::serveur::{Reglages, routeur};
use axum::Router;
use axum::http::StatusCode;
use commun::referentiel::{comptes, doter, sous_compte_fixe};
use commun::serveur::{ORIGINE, Reponse, application_base, envoyer, lire};
use commun::{CabinetTest, Env, code_sql, creer_cabinet, env};
use compta_core::{CodeJournal, Journal, NumeroCompte, TexteCourt, TypeJournal};
use time::{Date, Month};
use uuid::Uuid;

const TABLES: [&str; 6] = [
    "dossier_referentiel",
    "compte",
    "journal",
    "exercice",
    "periode",
    "tiers",
];

fn t(texte: &str) -> TexteCourt {
    TexteCourt::parse(texte).unwrap()
}

fn n(texte: &str) -> NumeroCompte {
    NumeroCompte::parse(texte).unwrap()
}

#[tokio::test]
async fn les_tables_du_referentiel_n_accordent_ni_delete_ni_update_de_table() {
    let env = env().await;
    // Colonnes modifiables attendues, table par table (note d'étude, §2 et §6).
    let attendues: [(&str, &[&str]); 6] = [
        ("dossier_referentiel", &[]),
        ("compte", &["actif", "intitule"]),
        ("journal", &[]),
        // Transitions de statut (PR 11b, `0201_periodes.sql`).
        ("exercice", &["statut"]),
        ("periode", &["motif", "statut", "statut_le", "statut_par"]),
        ("tiers", &["actif", "nom"]),
    ];
    for (table, colonnes) in attendues {
        let (rls, forcee): (bool, bool) = sqlx::query_as(
            "SELECT relrowsecurity, relforcerowsecurity FROM pg_class \
             WHERE relname = $1 AND relnamespace = 'public'::regnamespace",
        )
        .bind(table)
        .fetch_one(&env.admin)
        .await
        .unwrap();
        assert!(rls && forcee, "{table} : RLS");
        for role in ["sigma_application", "sigma_lecture_audit"] {
            let (delete, update, truncate): (bool, bool, bool) = sqlx::query_as(
                "SELECT has_table_privilege($1, $2, 'DELETE'), \
                        has_table_privilege($1, $2, 'UPDATE'), \
                        has_table_privilege($1, $2, 'TRUNCATE')",
            )
            .bind(role)
            .bind(table)
            .fetch_one(&env.admin)
            .await
            .unwrap();
            assert!(!delete && !update && !truncate, "{role} sur {table}");
        }
        let modifiables: Vec<String> = sqlx::query_scalar(
            "SELECT a.attname::text FROM pg_attribute a \
             WHERE a.attrelid = ('public.' || $1)::regclass AND a.attnum > 0 \
               AND NOT a.attisdropped \
               AND has_column_privilege('sigma_application', a.attrelid, a.attnum, 'UPDATE') \
             ORDER BY 1",
        )
        .bind(table)
        .fetch_all(&env.admin)
        .await
        .unwrap();
        assert_eq!(modifiables, colonnes, "{table} : UPDATE par colonne");
        let lecture: bool =
            sqlx::query_scalar("SELECT has_table_privilege('sigma_lecture_audit', $1, 'SELECT')")
                .bind(table)
                .fetch_one(&env.admin)
                .await
                .unwrap();
        assert!(!lecture, "{table} : rien pour le rôle de lecture d'audit");
    }
}

fn routeur_de(env: &Env) -> Router {
    routeur(&application_base(env, Reglages::new(ORIGINE.to_owned())))
}

fn base(c: &CabinetTest, dossier: IdDossier) -> String {
    format!("/api/cabinets/{}/dossiers/{dossier}", c.cabinet)
}

async fn compter(env: &Env, table: &str, dossier: Uuid) -> i64 {
    let sql = match table {
        "dossier" => "SELECT count(*) FROM dossier WHERE id = $1",
        "dossier_referentiel" => "SELECT count(*) FROM dossier_referentiel WHERE dossier_id = $1",
        "compte" => "SELECT count(*) FROM compte WHERE dossier_id = $1",
        "journal" => "SELECT count(*) FROM journal WHERE dossier_id = $1",
        "exercice" => "SELECT count(*) FROM exercice WHERE dossier_id = $1",
        "periode" => "SELECT count(*) FROM periode WHERE dossier_id = $1",
        "tiers" => "SELECT count(*) FROM tiers WHERE dossier_id = $1",
        "audit_dossier" => "SELECT count(*) FROM audit_dossier WHERE dossier_id = $1",
        autre => panic!("table inconnue : {autre}"),
    };
    sqlx::query_scalar(sql)
        .bind(dossier)
        .fetch_one(&env.admin)
        .await
        .unwrap()
}

async fn creer(routeur: &Router, c: &CabinetTest, corps: &str) -> Reponse {
    envoyer(
        routeur,
        &format!("/api/cabinets/{}/dossiers", c.cabinet),
        Some(c.administrateur),
        corps,
        None,
    )
    .await
}

fn id(reponse: &Reponse) -> Uuid {
    Uuid::parse_str(reponse.json()["id"].as_str().unwrap()).unwrap()
}

/// Contenu de la dernière entrée d'audit d'un dossier.
async fn derniere_entree(env: &Env, dossier: Uuid) -> String {
    sqlx::query_scalar(
        "SELECT contenu FROM audit_dossier WHERE dossier_id = $1 ORDER BY rang DESC LIMIT 1",
    )
    .bind(dossier)
    .fetch_one(&env.admin)
    .await
    .unwrap()
}

#[tokio::test]
async fn creation_d_une_entite_nouvelle_et_d_un_dossier_repris() {
    let env = env().await;
    let c = creer_cabinet(&env.admin, "Cabinet").await;
    let routeur = routeur_de(&env);
    let modele = Modele::officiel().unwrap();
    let nombre = i64::try_from(modele.etat().plan.comptes().count()).unwrap();

    // Entité nouvelle, second semestre, premier exercice prolongé (AUDCIF art. 7).
    let annee = time::OffsetDateTime::now_utc().year();
    let r = creer(
        &routeur,
        &c,
        &format!(
            r#"{{"nom":"Entité nouvelle","pays":"SN","nature":"creation","debut_activite":"{annee}-09-15","fin_premier_exercice":"annee_suivante"}}"#
        ),
    )
    .await;
    assert_eq!(r.statut, StatusCode::CREATED, "{}", r.texte());
    let d = id(&r);
    assert_eq!(compter(&env, "compte", d).await, nombre);
    assert_eq!(compter(&env, "journal", d).await, 7);
    assert_eq!(compter(&env, "exercice", d).await, 1);
    // Du 15 septembre au 31 décembre de l'année suivante : 4 + 12 périodes.
    assert_eq!(compter(&env, "periode", d).await, 16);
    let (nature, debut, version, empreinte): (String, Option<Date>, String, Vec<u8>) =
        sqlx::query_as(
            "SELECT nature, debut_activite, referentiel_version, referentiel_empreinte \
             FROM dossier_referentiel WHERE dossier_id = $1",
        )
        .bind(d)
        .fetch_one(&env.admin)
        .await
        .unwrap();
    assert_eq!(nature, "creation");
    assert_eq!(
        debut,
        Some(Date::from_calendar_date(annee, Month::September, 15).unwrap())
    );
    assert_eq!(version, modele.version());
    assert_eq!(empreinte, modele.empreinte().octets());
    // Première entrée de l'audit du dossier, enrichie.
    let entree = derniere_entree(&env, d).await;
    for attendu in [
        "rang=1\n".to_owned(),
        format!(
            "donnee=referentiel\nvaleur={} ; {}\n",
            modele.version(),
            modele.empreinte()
        ),
        format!("donnee=comptes\nvaleur={nombre}\n"),
        "donnee=journaux\nvaleur=AC,AN,CA,IN,OD,PA,VT\n".to_owned(),
        format!(
            "donnee=exercice\nvaleur={} ; {annee}-09-15 ; {}-12-31\n",
            annee + 1,
            annee + 1
        ),
        "donnee=nature\nvaleur=creation\n".to_owned(),
        format!("donnee=debut_activite\nvaleur={annee}-09-15\n"),
    ] {
        assert!(
            entree.contains(&attendu),
            "{attendu:?} absent de :\n{entree}"
        );
    }
    // Lecture des exercices : périodes et échéances.
    let r = lire(
        &routeur,
        &format!("{}/exercices", base(&c, IdDossier::depuis_uuid(d))),
        Some(c.administrateur),
    )
    .await;
    assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
    let e = &r.json()["exercices"][0];
    assert_eq!(e["code"], (annee + 1).to_string());
    assert_eq!(e["periodes"].as_array().unwrap().len(), 16);
    assert!(e["periodes"][0]["echeance_cloture"].is_string());
}

#[tokio::test]
async fn creation_d_un_dossier_repris_et_refus_avant_toute_ecriture() {
    let env = env().await;
    let c = creer_cabinet(&env.admin, "Cabinet").await;
    let routeur = routeur_de(&env);
    let annee = time::OffsetDateTime::now_utc().year();
    // Dossier repris : exercice civil choisi.
    let r = creer(
        &routeur,
        &c,
        &format!(r#"{{"nom":"Entité reprise","pays":"SN","nature":"reprise","annee":{annee}}}"#),
    )
    .await;
    assert_eq!(r.statut, StatusCode::CREATED, "{}", r.texte());
    let d = id(&r);
    assert_eq!(compter(&env, "periode", d).await, 12);
    let entree = derniere_entree(&env, d).await;
    assert!(entree.contains("donnee=nature\nvaleur=reprise\n"));
    assert!(!entree.contains("debut_activite"));

    // Refus avant toute écriture : année future, prolongation au premier semestre, nature
    // mêlée. Aucun dossier de plus.
    let dossiers = |env: &Env| {
        let admin = env.admin.clone();
        let cabinet = c.cabinet.uuid();
        async move {
            sqlx::query_scalar::<_, i64>("SELECT count(*) FROM dossier WHERE cabinet_id = $1")
                .bind(cabinet)
                .fetch_one(&admin)
                .await
                .unwrap()
        }
    };
    let avant = dossiers(&env).await;
    for corps in [
        format!(
            r#"{{"nom":"X","pays":"SN","nature":"reprise","annee":{}}}"#,
            annee + 1
        ),
        format!(
            r#"{{"nom":"X","pays":"SN","nature":"creation","debut_activite":"{annee}-03-01","fin_premier_exercice":"annee_suivante"}}"#
        ),
        format!(
            r#"{{"nom":"X","pays":"SN","nature":"reprise","annee":{annee},"debut_activite":"{annee}-01-01"}}"#
        ),
        r#"{"nom":"X"}"#.to_owned(),
    ] {
        let r = creer(&routeur, &c, &corps).await;
        assert_eq!(r.statut, StatusCode::BAD_REQUEST, "{corps} : {}", r.texte());
    }
    assert_eq!(dossiers(&env).await, avant);
}

#[tokio::test]
async fn une_transaction_en_echec_ne_laisse_rien() {
    let env = env().await;
    let c = creer_cabinet(&env.admin, "Cabinet").await;
    let aujourd_hui = time::OffsetDateTime::now_utc().date();
    let i = Instanciation::preparer(
        Modele::officiel().unwrap(),
        Ouverture::Reprise {
            annee: aujourd_hui.year(),
        },
        aujourd_hui,
    )
    .unwrap();
    let mut tx = ouvrir_transaction(
        &env.application,
        ContexteTenant::cabinet(c.cabinet, c.administrateur),
    )
    .await
    .unwrap();
    let dossier = creer_dossier(&mut tx, "Dossier", &commun::contexte_dossier_sn())
        .await
        .unwrap();
    instancier(&mut tx, dossier.id, &i).await.unwrap();
    // Une seconde instanciation échoue (unicité) : la transaction est perdue.
    let erreur = instancier(&mut tx, dossier.id, &i).await.unwrap_err();
    assert!(
        matches!(erreur, api::base::ErreurBase::Sqlx(ref e) if code_sql(e).as_deref() == Some("23505"))
    );
    drop(tx);
    for table in TABLES.into_iter().chain(["dossier"]) {
        assert_eq!(compter(&env, table, dossier.id.uuid()).await, 0, "{table}");
    }
}

#[tokio::test]
async fn aller_retour_du_plan_officiel_complet_enrichi() {
    let env = env().await;
    let c = creer_cabinet(&env.admin, "Cabinet").await;
    doter(&env.application, c.cabinet, c.dossier, c.administrateur).await;
    let k = comptes();

    // État attendu, calculé en pur à partir du modèle.
    let mut attendu: EtatReferentiel = Modele::officiel().unwrap().etat().clone();
    let tiers = Uuid::new_v4();
    let operations = |etat: &mut EtatReferentiel| -> Result<(), RefusReferentiel> {
        let auxiliaire = n(&format!("{}-SENELEC", k.collectif));
        let banque = n(&format!("{}-BICIS", k.banque));
        etat.plan
            .ajouter_auxiliaire(auxiliaire.clone(), t("Auxiliaire"))?;
        etat.plan.ajouter_sous_compte(banque.clone(), t("Banque"))?;
        etat.plan
            .ajouter_sous_compte(n(&format!("{banque}-EUR")), t("Banque, euros"))?;
        etat.plan.desactiver(&k.base)?;
        etat.journaux.ajouter(Journal::nouveau(
            CodeJournal::parse("BQ1").unwrap(),
            t("Banque"),
            TypeJournal::Tresorerie,
            Some(n(&format!("{banque}-EUR"))),
            &etat.plan,
        )?)?;
        etat.tiers.insert(
            tiers,
            Tiers::nouveau(
                t("Fournisseur"),
                TypeTiers::Fournisseur,
                k.fournisseur.clone(),
            ),
        );
        Ok(())
    };
    attendu
        .plan
        .ajouter_sous_compte(sous_compte_fixe(), t("Sous-compte fixe"))
        .unwrap();
    operations(&mut attendu).unwrap();

    let contexte = ContexteTenant::dossier(c.cabinet, c.dossier, c.administrateur);
    let mut tx = ouvrir_transaction(&env.application, contexte)
        .await
        .unwrap();
    muter(&mut tx, |_, etat| operations(etat)).await.unwrap();
    tx.confirmer().await.unwrap();

    let mut tx = ouvrir_transaction(&env.application, contexte)
        .await
        .unwrap();
    let relu = charger(&mut tx).await.unwrap().unwrap();
    assert_eq!(relu.etat().plan, attendu.plan);
    assert_eq!(relu.etat().journaux, attendu.journaux);
    assert_eq!(relu.etat().tiers.get(&tiers), attendu.tiers.get(&tiers));
    assert_eq!(relu.etat().tiers.len(), 2);
    let aujourd_hui = time::OffsetDateTime::now_utc().date();
    let mut exercices = compta_core::Exercices::new();
    exercices.ouvrir_repris(aujourd_hui.year()).unwrap();
    assert_eq!(relu.exercices(), &exercices);
    // Le dossier d'un autre contexte n'est pas lu.
    let mut tx = ouvrir_transaction(
        &env.application,
        ContexteTenant::dossier(c.cabinet, c.autre_dossier, c.administrateur),
    )
    .await
    .unwrap();
    assert!(charger(&mut tx).await.unwrap().is_none());
}

/// Mutation par sa route : statut attendu, et une entrée d'audit de plus si elle réussit.
async fn muter_par_route(
    env: &Env,
    routeur: &Router,
    c: &CabinetTest,
    suffixe: &str,
    corps: &str,
    statut: StatusCode,
) -> Reponse {
    let avant = compter(env, "audit_dossier", c.dossier.uuid()).await;
    let r = envoyer(
        routeur,
        &format!("{}/{suffixe}", base(c, c.dossier)),
        Some(c.administrateur),
        corps,
        None,
    )
    .await;
    assert_eq!(r.statut, statut, "{suffixe} {corps} : {}", r.texte());
    let apres = compter(env, "audit_dossier", c.dossier.uuid()).await;
    assert_eq!(apres - avant, i64::from(statut.is_success()), "{suffixe}");
    r
}

const OK: StatusCode = StatusCode::OK;
const CREE: StatusCode = StatusCode::CREATED;
const REFUS: StatusCode = StatusCode::BAD_REQUEST;

/// Dossier doté du référentiel, et de quoi l'appeler par ses routes.
struct Banc {
    env: Env,
    c: CabinetTest,
    f: commun::referentiel::Fixtures,
    routeur: Router,
}

/// Identifiant d'un compte du dossier, lu par `GET …/comptes`.
async fn id_compte(b: &Banc, numero: &NumeroCompte) -> String {
    let r = lire(
        &b.routeur,
        &format!("{}/comptes", base(&b.c, b.c.dossier)),
        Some(b.c.administrateur),
    )
    .await;
    r.json()["comptes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["numero"] == numero.as_str())
        .map(|x| x["id"].as_str().unwrap().to_owned())
        .unwrap()
}

#[tokio::test]
async fn chaque_mutation_passe_le_controle_croise() {
    let env = env().await;
    let c = creer_cabinet(&env.admin, "Cabinet").await;
    let f = doter(&env.application, c.cabinet, c.dossier, c.administrateur).await;
    let routeur = routeur_de(&env);
    let b = Banc { env, c, f, routeur };
    controle_du_plan(&b).await;
    controle_des_journaux(&b).await;
    controle_des_tiers(&b).await;
    controle_du_sous_compte(&b).await;
    let Banc { env, c, .. } = &b;
    // Le référentiel relu reste cohérent après toutes ces mutations.
    let mut tx = ouvrir_transaction(
        &env.application,
        ContexteTenant::dossier(c.cabinet, c.dossier, c.administrateur),
    )
    .await
    .unwrap();
    let relu = charger(&mut tx).await.unwrap().unwrap();
    assert_eq!(api::base::referentiel::verifier(relu.etat()), Ok(()));
}

/// Plan : sous-compte ou désactivation sous le compte du journal de caisse, intitulé officiel,
/// réactivation sans effet, auxiliaire hors collectif.
async fn controle_du_plan(b: &Banc) {
    let Banc { env, c, f, routeur } = b;
    let k = comptes();
    let caisse = id_compte(b, &k.caisse).await;
    // Sous-compte sous le compte du journal de caisse : refusé, CA reste lisible.
    muter_par_route(
        env,
        routeur,
        c,
        "comptes",
        &format!(
            r#"{{"numero":"{}-A","intitule":"Caisse A","genre":"sous_compte"}}"#,
            k.caisse
        ),
        REFUS,
    )
    .await;
    let r = lire(
        routeur,
        &format!("{}/journaux", base(c, c.dossier)),
        Some(c.administrateur),
    )
    .await;
    assert_eq!(r.statut, OK, "{}", r.texte());
    // Désactivation du compte de caisse : refusée.
    muter_par_route(
        env,
        routeur,
        c,
        &format!("comptes/{caisse}/desactivation"),
        "{}",
        REFUS,
    )
    .await;
    // Intitulé d'un compte officiel : immuable.
    muter_par_route(
        env,
        routeur,
        c,
        &format!("comptes/{caisse}/intitule"),
        r#"{"intitule":"Autre"}"#,
        REFUS,
    )
    .await;
    // Réactivation d'un compte actif : aucun changement.
    muter_par_route(
        env,
        routeur,
        c,
        &format!("comptes/{}/reactivation", f.compte),
        "{}",
        REFUS,
    )
    .await;
    // Auxiliaire sous un compte non collectif : refusé ; sous un collectif : admis.
    muter_par_route(
        env,
        routeur,
        c,
        "comptes",
        &format!(
            r#"{{"numero":"{}-AUX","intitule":"Aux","genre":"auxiliaire"}}"#,
            k.base
        ),
        REFUS,
    )
    .await;
    let r = muter_par_route(
        env,
        routeur,
        c,
        "comptes",
        &format!(
            r#"{{"numero":"{}-AUX","intitule":"Aux","genre":"auxiliaire"}}"#,
            k.collectif
        ),
        CREE,
    )
    .await;
    assert_eq!(r.json()["genre"], "auxiliaire");
    let entree = derniere_entree(env, c.dossier.uuid()).await;
    assert!(entree.contains("action=ParametrerDossier\n"), "{entree}");
    assert!(
        entree.contains(&format!("champ=numero\napres={}-AUX\n", k.collectif)),
        "{entree}"
    );
}

/// Journaux : compte de trésorerie déjà pris, de regroupement, à-nouveaux, doublon.
async fn controle_des_journaux(b: &Banc) {
    let Banc {
        env,
        c,
        f: _,
        routeur,
    } = b;
    let k = comptes();
    // Journaux : second journal sur le compte de caisse, compte de regroupement, type
    // à-nouveaux : refusés ; un compte de banque libre : admis.
    for corps in [
        format!(
            r#"{{"code":"BQ1","intitule":"B","type":"tresorerie","compte_tresorerie":"{}"}}"#,
            k.caisse
        ),
        format!(
            r#"{{"code":"BQ1","intitule":"B","type":"tresorerie","compte_tresorerie":"{}"}}"#,
            k.caisse.parent().unwrap()
        ),
        r#"{"code":"AN2","intitule":"B","type":"a_nouveaux"}"#.to_owned(),
        r#"{"code":"AC","intitule":"Doublon","type":"achats"}"#.to_owned(),
        r#"{"code":"VT2","intitule":"V","type":"ventes","admis_en_cloture":true}"#.to_owned(),
    ] {
        muter_par_route(env, routeur, c, "journaux", &corps, REFUS).await;
    }
    let r = muter_par_route(
        env,
        routeur,
        c,
        "journaux",
        &format!(
            r#"{{"code":"BQ1","intitule":"Banque","type":"tresorerie","compte_tresorerie":"{}"}}"#,
            k.banque
        ),
        CREE,
    )
    .await;
    assert_eq!(r.json()["compte_tresorerie"], k.banque.to_string());
    // Sous-compte sous le compte de la banque, désormais compte d'un journal : refusé.
    muter_par_route(
        env,
        routeur,
        c,
        "comptes",
        &format!(
            r#"{{"numero":"{}-X","intitule":"X","genre":"sous_compte"}}"#,
            k.banque
        ),
        REFUS,
    )
    .await;
}

/// Tiers : compte collectif exigé et gardé actif ; sous-compte renommé et désactivé.
async fn controle_des_tiers(b: &Banc) {
    let Banc { env, c, f, routeur } = b;
    let k = comptes();
    // Tiers : compte non collectif refusé ; le collectif d'un tiers actif ne se désactive
    // pas ; une fois le tiers désactivé, il le peut, et le tiers ne se réactive plus.
    muter_par_route(
        env,
        routeur,
        c,
        "tiers",
        &format!(
            r#"{{"nom":"T","type":"client","compte_collectif":"{}"}}"#,
            k.base
        ),
        REFUS,
    )
    .await;
    // Type contraire à la marque du compte (fiches 40 et 41), type « autre », compte
    // auxiliaire (individuel) : refusés. Un fournisseur sur un compte de la fiche 40 : admis.
    for (genre, compte) in [
        ("fournisseur", k.collectif.to_string()),
        ("client", k.fournisseur.to_string()),
        ("autre", k.collectif.to_string()),
        ("client", format!("{}-AUX", k.collectif)),
    ] {
        muter_par_route(
            env,
            routeur,
            c,
            "tiers",
            &format!(r#"{{"nom":"T","type":"{genre}","compte_collectif":"{compte}"}}"#),
            REFUS,
        )
        .await;
    }
    muter_par_route(
        env,
        routeur,
        c,
        "tiers",
        &format!(
            r#"{{"nom":"F","type":"fournisseur","compte_collectif":"{}"}}"#,
            k.fournisseur
        ),
        CREE,
    )
    .await;
    let collectif = id_compte(b, &k.collectif).await;
    muter_par_route(
        env,
        routeur,
        c,
        &format!("comptes/{collectif}/desactivation"),
        "{}",
        REFUS,
    )
    .await;
    muter_par_route(
        env,
        routeur,
        c,
        &format!("tiers/{}/modification", f.tiers),
        r#"{"actif":false}"#,
        OK,
    )
    .await;
    muter_par_route(
        env,
        routeur,
        c,
        &format!("comptes/{collectif}/desactivation"),
        "{}",
        OK,
    )
    .await;
    muter_par_route(
        env,
        routeur,
        c,
        &format!("tiers/{}/modification", f.tiers),
        r#"{"actif":true}"#,
        REFUS,
    )
    .await;
    muter_par_route(
        env,
        routeur,
        c,
        &format!("tiers/{}/modification", f.tiers),
        "{}",
        REFUS,
    )
    .await;
}

/// Sous-compte : renommé, désactivé, réactivé ; traces des seuls champs modifiés.
async fn controle_du_sous_compte(b: &Banc) {
    let Banc { env, c, f, routeur } = b;
    muter_par_route(
        env,
        routeur,
        c,
        &format!("comptes/{}/intitule", f.compte),
        r#"{"intitule":"Renommé"}"#,
        OK,
    )
    .await;
    let entree = derniere_entree(env, c.dossier.uuid()).await;
    assert!(
        entree.contains("champ=intitule\navant=Sous-compte fixe\napres=Renommé\n"),
        "{entree}"
    );
    assert!(!entree.contains("champ=actif"), "{entree}");
    muter_par_route(
        env,
        routeur,
        c,
        &format!("comptes/{}/desactivation", f.compte),
        "{}",
        OK,
    )
    .await;
    muter_par_route(
        env,
        routeur,
        c,
        &format!("comptes/{}/reactivation", f.compte),
        "{}",
        OK,
    )
    .await;

    // Pagination des tiers.
    let r = lire(
        routeur,
        &format!("{}/tiers?limite=1", base(c, c.dossier)),
        Some(c.administrateur),
    )
    .await;
    // Deux tiers (le tiers fixe et le fournisseur créé) : deux pages d'un tiers.
    assert_eq!(r.json()["tiers"].as_array().unwrap().len(), 1);
    let suite = r.json()["suite"].as_str().unwrap().to_owned();
    let r = lire(
        routeur,
        &format!("{}/tiers?limite=1&apres={suite}", base(c, c.dossier)),
        Some(c.administrateur),
    )
    .await;
    assert_eq!(r.json()["tiers"].as_array().unwrap().len(), 1);
    assert_ne!(r.json()["tiers"][0]["id"], suite.as_str());
    assert!(r.json()["suite"].is_null());
}

#[tokio::test]
async fn le_verrou_du_referentiel_serialise_les_mutations_sans_bloquer_les_insertions() {
    let env = env().await;
    let c = creer_cabinet(&env.admin, "Cabinet").await;
    doter(&env.application, c.cabinet, c.dossier, c.administrateur).await;
    let contexte = ContexteTenant::dossier(c.cabinet, c.dossier, c.administrateur);
    let mut tx1 = ouvrir_transaction(&env.application, contexte)
        .await
        .unwrap();
    assert!(verrouiller(&mut tx1).await.unwrap());

    // Une seconde mutation attend : refusée ici au bout de 100 ms.
    let mut tx2 = ouvrir_transaction(&env.application, contexte)
        .await
        .unwrap();
    sqlx::query("SET LOCAL lock_timeout = '100ms'")
        .execute(tx2.connexion())
        .await
        .unwrap();
    // Par `muter` lui-même : sans son verrou, cette mutation passerait (son insertion ne prend
    // que `FOR KEY SHARE` sur le dossier) et le test échouerait.
    let erreur = muter(&mut tx2, |_, etat| {
        etat.plan
            .ajouter_sous_compte(n(&format!("{}-VERROU", comptes().base)), t("Verrou"))?;
        Ok(())
    })
    .await
    .unwrap_err();
    assert!(
        matches!(erreur, ErreurMutation::Base(api::base::ErreurBase::Sqlx(ref e)) if code_sql(e).as_deref() == Some("55P03")),
        "{erreur:?}"
    );
    drop(tx2);

    // Une insertion d'une ligne enfant du dossier (`FOR KEY SHARE`) n'attend pas.
    let mut tx3 = ouvrir_transaction(&env.application, contexte)
        .await
        .unwrap();
    sqlx::query("SET LOCAL lock_timeout = '100ms'")
        .execute(tx3.connexion())
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO tiers (cabinet_id, dossier_id, id, nom, type, compte_collectif, actif) \
         VALUES ($1, $2, $3, 'Tiers', 'autre', $4, true)",
    )
    .bind(c.cabinet.uuid())
    .bind(c.dossier.uuid())
    .bind(Uuid::new_v4())
    .bind(comptes().collectif.to_string())
    .execute(tx3.connexion())
    .await
    .unwrap();
    drop(tx3);
    drop(tx1);
}

#[tokio::test]
async fn l_intitule_d_un_compte_officiel_ne_s_enregistre_jamais() {
    // Seconde barrière, après `compta-core` : un plan rebâti avec un intitulé officiel changé
    // (impossible par `renommer`) est refusé à l'enregistrement, et rien n'est écrit.
    let env = env().await;
    let c = creer_cabinet(&env.admin, "Cabinet").await;
    doter(&env.application, c.cabinet, c.dossier, c.administrateur).await;
    let contexte = ContexteTenant::dossier(c.cabinet, c.dossier, c.administrateur);
    let mut tx = ouvrir_transaction(&env.application, contexte)
        .await
        .unwrap();
    let base = comptes().base.clone();
    let erreur = muter(&mut tx, |_, etat| {
        let mut lignes = aplatir_plan(&etat.plan);
        let ligne = lignes.iter_mut().find(|l| l.numero == base).unwrap();
        ligne.intitule = t("Intitulé officiel altéré");
        etat.plan = reconstituer_plan(lignes).unwrap();
        Ok(())
    })
    .await
    .unwrap_err();
    assert!(
        matches!(
            erreur,
            ErreurMutation::Base(api::base::ErreurBase::Donnee(_))
        ),
        "{erreur:?}"
    );
    drop(tx);
    let mut tx = ouvrir_transaction(&env.application, contexte)
        .await
        .unwrap();
    let relu = charger(&mut tx).await.unwrap().unwrap();
    assert_eq!(
        relu.etat().plan.compte(&base).unwrap().intitule(),
        Modele::officiel()
            .unwrap()
            .etat()
            .plan
            .compte(&base)
            .unwrap()
            .intitule()
    );
}
