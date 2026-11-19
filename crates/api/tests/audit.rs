//! Journal d'audit chaîné (L2-15 ; ADR 0014, ADR 0016 ; L2, « Tests exigés », « Audit » ;
//! F1.6, F1.15, F2.6, F2.9).
//!
//! - toute action crée une entrée chaînée, au cabinet puis au dossier pour une création de
//!   dossier ; l'export est lui-même inscrit, avec la tête et le nombre exportés ;
//! - une entrée modifiée ou supprimée en base, par le rôle **propriétaire**, est détectée ;
//! - le rôle applicatif ne peut ni lire, ni modifier, ni supprimer une entrée ; le rôle de
//!   lecture ne peut rien écrire ;
//! - la RLS de l'audit isole les cabinets et les dossiers ; un export de dossier ne cite rien
//!   d'un autre dossier ;
//! - l'audit du cabinet est refusé (403) au valideur et à l'auditeur ; l'export exige une
//!   session fraîche ;
//! - un échec d'écriture de l'audit annule l'action ; des actions concurrentes donnent des
//!   rangs continus.
#![cfg(feature = "tests-postgres")]
// Fichier de test : les `expect`/`unwrap` y sont voulus.
#![allow(clippy::expect_used, clippy::unwrap_used)]

mod commun;

use api::base::{
    ContexteTenant, ErreurBase, IdCabinet, IdDossier, IdUtilisateur, ouvrir_transaction,
    verifier_role_lecture_audit,
};
use api::serveur::{Reglages, routeur};
use axum::Router;
use axum::http::StatusCode;
use commun::serveur::{ORIGINE, application_base, envoyer, lire};
use commun::{
    CabinetTest, Env, ROLE_MIGRATION, ajouter_affecte, code_sql, creer_cabinet, env, pool_sous_role,
};
use integrite::audit::verifier_export_audit;
use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

/// SQLSTATE `insufficient_privilege` : droit refusé, ou ligne refusée par `WITH CHECK`.
const REFUS: &str = "42501";

fn routeur_de(env: &Env) -> Router {
    routeur(&application_base(env, Reglages::new(ORIGINE.to_owned())))
}

async fn creer_dossier(
    routeur: &Router,
    c: &CabinetTest,
    par: IdUtilisateur,
    nom: &str,
) -> IdDossier {
    let reponse = envoyer(
        routeur,
        &format!("/api/cabinets/{}/dossiers", c.cabinet),
        Some(par),
        &serde_json::json!({ "nom": nom }).to_string(),
        None,
    )
    .await;
    assert_eq!(reponse.statut, StatusCode::CREATED, "{}", reponse.texte());
    IdDossier::depuis_uuid(Uuid::parse_str(reponse.json()["id"].as_str().unwrap()).unwrap())
}

fn chemin_audit(c: &CabinetTest, dossier: Option<IdDossier>) -> String {
    match dossier {
        None => format!("/api/cabinets/{}/audit", c.cabinet),
        Some(d) => format!("/api/cabinets/{}/dossiers/{d}/audit", c.cabinet),
    }
}

async fn verification(routeur: &Router, c: &CabinetTest, dossier: Option<IdDossier>) -> Value {
    let reponse = lire(
        routeur,
        &format!("{}/verification", chemin_audit(c, dossier)),
        Some(c.administrateur),
    )
    .await;
    assert_eq!(reponse.statut, StatusCode::OK, "{}", reponse.texte());
    reponse.json()
}

async fn page(routeur: &Router, c: &CabinetTest, dossier: Option<IdDossier>) -> Vec<Value> {
    let reponse = lire(
        routeur,
        &format!("{}?limite=100", chemin_audit(c, dossier)),
        Some(c.administrateur),
    )
    .await;
    assert_eq!(reponse.statut, StatusCode::OK, "{}", reponse.texte());
    reponse.json()["entrees"].as_array().unwrap().clone()
}

async fn exporter(
    routeur: &Router,
    c: &CabinetTest,
    dossier: Option<IdDossier>,
    par: IdUtilisateur,
    anciennete: Option<i64>,
) -> commun::serveur::Reponse {
    envoyer(
        routeur,
        &format!("{}/export", chemin_audit(c, dossier)),
        Some(par),
        "",
        anciennete,
    )
    .await
}

/// Exécute `sql` sous le rôle propriétaire des tables (`sigma_migration`), au contexte donné :
/// la RLS forcée s'applique aussi à lui.
async fn sous_proprietaire(
    cabinet: IdCabinet,
    dossier: Option<IdDossier>,
    sql: &'static str,
) -> u64 {
    let pool = pool_sous_role(ROLE_MIGRATION).await;
    let mut tx = pool.begin().await.unwrap();
    sqlx::query(
        "SELECT set_config('sigma.cabinet', $1, true), set_config('sigma.dossier', $2, true)",
    )
    .bind(cabinet.to_string())
    .bind(dossier.map_or_else(String::new, |d| d.to_string()))
    .execute(&mut *tx)
    .await
    .unwrap();
    let n = sqlx::query(sql)
        .execute(&mut *tx)
        .await
        .unwrap()
        .rows_affected();
    tx.commit().await.unwrap();
    n
}

#[tokio::test]
async fn toute_action_cree_une_entree_chainee() {
    let env = env().await;
    let c = creer_cabinet(&env.admin, "Cabinet audité").await;
    let valideur = ajouter_affecte(&env.admin, &c, "valideur").await;
    let routeur = routeur_de(&env);

    // Création par un valideur : il est affecté au dossier créé, qu'il voit donc.
    let dossier = creer_dossier(&routeur, &c, valideur, "SARL Téranga = Dakar").await;
    let reponse = lire(
        &routeur,
        &format!("/api/cabinets/{}/dossiers/{dossier}", c.cabinet),
        Some(valideur),
    )
    .await;
    assert_eq!(reponse.statut, StatusCode::OK, "{}", reponse.texte());

    // Au cabinet : une entrée, qui cite le dossier, son nom et l'affectation du créateur.
    let au_cabinet = page(&routeur, &c, None).await;
    assert_eq!(au_cabinet.len(), 1);
    let e = &au_cabinet[0];
    assert_eq!(e["rang"], 1);
    assert_eq!(e["action"], "CreerDossier");
    assert_eq!(e["ressource"], format!("dossier:{dossier}"));
    assert_eq!(e["acteur"], valideur.to_string());
    assert_eq!(e["acteur_nom"], "valideur affecté");
    assert_eq!(e["role"], "valideur");
    assert_eq!(e["champs"][0]["nom"], "nom");
    assert_eq!(e["champs"][0]["apres"], "SARL Téranga = Dakar");
    assert_eq!(e["champs"][1]["nom"], "affectation");
    assert_eq!(e["champs"][1]["apres"], valideur.to_string());
    // Au dossier : la première entrée de sa chaîne.
    let au_dossier = page(&routeur, &c, Some(dossier)).await;
    assert_eq!(au_dossier.len(), 1);
    assert_eq!(au_dossier[0]["action"], "CreerDossier");

    // Export du dossier par le valideur (session fraîche) : vérifiable seul, inscrit à l'audit.
    let reponse = exporter(&routeur, &c, Some(dossier), valideur, None).await;
    assert_eq!(reponse.statut, StatusCode::OK, "{}", reponse.texte());
    let rapport = verifier_export_audit(reponse.corps.as_slice(), None).unwrap();
    assert!(rapport.est_integre(), "{rapport}");
    assert_eq!(rapport.chaine.nombre, 1);
    assert_eq!(rapport.annexe.len(), 1);
    assert_eq!(
        rapport.annexe[0].1.as_ref().unwrap().as_str(),
        "valideur affecté"
    );
    let au_dossier = page(&routeur, &c, Some(dossier)).await;
    assert_eq!(au_dossier.len(), 2);
    let export = &au_dossier[1];
    assert_eq!(export["action"], "ExporterComplet");
    assert_eq!(export["reauthentification"], true);
    let donnees: Vec<(String, String)> = export["donnees"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| {
            (
                d["nom"].as_str().unwrap().to_owned(),
                d["valeur"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    assert_eq!(
        donnees,
        [
            ("export".to_owned(), "audit".to_owned()),
            ("tete".to_owned(), rapport.chaine.tete.to_string()),
            ("nombre".to_owned(), "1".to_owned()),
        ]
    );

    // Export du cabinet par l'administrateur : inscrit à la chaîne du cabinet.
    let reponse = exporter(&routeur, &c, None, c.administrateur, None).await;
    assert_eq!(reponse.statut, StatusCode::OK, "{}", reponse.texte());
    let au_cabinet = page(&routeur, &c, None).await;
    assert_eq!(au_cabinet.len(), 2);
    assert_eq!(au_cabinet[1]["action"], "ExporterAuditCabinet");

    // Les deux chaînes restent intègres.
    for portee in [None, Some(dossier)] {
        let v = verification(&routeur, &c, portee).await;
        assert_eq!(v["integre"], true, "{v}");
        assert_eq!(v["nombre"], 2, "{v}");
    }
}

#[tokio::test]
async fn une_entree_modifiee_ou_supprimee_par_le_proprietaire_est_detectee() {
    let env = env().await;
    let c = creer_cabinet(&env.admin, "Cabinet altéré").await;
    let routeur = routeur_de(&env);
    let dossier = creer_dossier(&routeur, &c, c.administrateur, "Dossier initial").await;
    for _ in 0..2 {
        let r = exporter(&routeur, &c, Some(dossier), c.administrateur, None).await;
        assert_eq!(r.statut, StatusCode::OK);
    }
    assert_eq!(
        verification(&routeur, &c, Some(dossier)).await["integre"],
        true
    );

    // Le propriétaire des tables (rôle de migration) réécrit le nom de l'entrée 1, sans
    // casser sa forme canonique.
    let n = sous_proprietaire(
        c.cabinet,
        Some(dossier),
        "UPDATE audit_dossier SET contenu = replace(contenu, 'Dossier initial', 'Dossier falsifié') \
         WHERE rang = 1",
    )
    .await;
    assert_eq!(n, 1);
    let v = verification(&routeur, &c, Some(dossier)).await;
    assert_eq!(v["integre"], false);
    assert_eq!(v["anomalies"][0]["code"], "empreinte_alteree");
    assert_eq!(v["anomalies"][0]["position"], 1);
    // L'export reflète la base : sa vérification hors ligne le détecte aussi.
    let reponse = exporter(&routeur, &c, Some(dossier), c.administrateur, None).await;
    let rapport = verifier_export_audit(reponse.corps.as_slice(), None).unwrap();
    assert!(!rapport.est_integre(), "{rapport}");

    // Suppression d'une entrée du milieu : rang et tête différents.
    let n = sous_proprietaire(
        c.cabinet,
        Some(dossier),
        "DELETE FROM audit_dossier WHERE rang = 2",
    )
    .await;
    assert_eq!(n, 1);
    let v = verification(&routeur, &c, Some(dossier)).await;
    let codes: Vec<&str> = v["anomalies"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["code"].as_str().unwrap())
        .collect();
    assert!(codes.contains(&"rang_different"), "{v}");
    assert!(codes.contains(&"nombre_different"), "{v}");
}

#[tokio::test]
async fn le_role_applicatif_ne_peut_ni_lire_ni_modifier_ni_supprimer_une_entree() {
    let env = env().await;
    let c = creer_cabinet(&env.admin, "Cabinet A").await;
    let routeur = routeur_de(&env);
    let dossier = creer_dossier(&routeur, &c, c.administrateur, "D").await;

    for (requete, dossier_en_contexte) in [
        ("SELECT count(*) FROM audit_cabinet", None),
        ("UPDATE audit_cabinet SET contenu = 'x'", None),
        ("DELETE FROM audit_cabinet", None),
        ("TRUNCATE audit_cabinet", None),
        ("SELECT count(*) FROM audit_dossier", Some(dossier)),
        ("UPDATE audit_dossier SET contenu = 'x'", Some(dossier)),
        ("DELETE FROM audit_dossier", Some(dossier)),
        ("DELETE FROM audit_tete_cabinet", None),
        ("DELETE FROM audit_tete_dossier", Some(dossier)),
        (
            "UPDATE audit_tete_cabinet SET cabinet_id = cabinet_id",
            None,
        ),
    ] {
        let contexte = match dossier_en_contexte {
            None => ContexteTenant::cabinet(c.cabinet, c.administrateur),
            Some(d) => ContexteTenant::dossier(c.cabinet, d, c.administrateur),
        };
        let mut tx = ouvrir_transaction(&env.application, contexte)
            .await
            .unwrap();
        let erreur = sqlx::query(requete)
            .execute(tx.connexion())
            .await
            .expect_err(requete);
        assert_eq!(code_sql(&erreur).as_deref(), Some(REFUS), "{requete}");
    }
    // Droits au catalogue : INSERT seul sur les entrées.
    for table in ["audit_cabinet", "audit_dossier"] {
        let droits: Vec<String> = sqlx::query_scalar(
            "SELECT d FROM unnest(ARRAY['SELECT', 'INSERT', 'UPDATE', 'DELETE', 'TRUNCATE']) AS d \
             WHERE has_table_privilege('sigma_application', $1::regclass, d) \
                OR (d IN ('SELECT', 'UPDATE') \
                    AND has_any_column_privilege('sigma_application', $1::regclass, d))",
        )
        .bind(table)
        .fetch_all(&env.admin)
        .await
        .unwrap();
        assert_eq!(droits, ["INSERT"], "{table}");
    }
}

#[tokio::test]
async fn le_role_de_lecture_ne_peut_rien_ecrire() {
    let env = env().await;
    let c = creer_cabinet(&env.admin, "Cabinet A").await;
    let routeur = routeur_de(&env);
    let dossier = creer_dossier(&routeur, &c, c.administrateur, "D").await;
    verifier_role_lecture_audit(&env.lecture_audit)
        .await
        .unwrap();
    // Le pool applicatif n'est pas un pool de lecture.
    assert!(matches!(
        verifier_role_lecture_audit(&env.application).await,
        Err(ErreurBase::RoleTropPrivilegie(_))
    ));

    for (requete, dossier_en_contexte) in [
        (
            "INSERT INTO audit_cabinet (cabinet_id, rang, contenu, empreinte) \
             SELECT cabinet_id, rang + 100, contenu, empreinte FROM audit_cabinet",
            None,
        ),
        ("UPDATE audit_cabinet SET contenu = 'x'", None),
        ("DELETE FROM audit_cabinet", None),
        ("UPDATE audit_tete_cabinet SET rang = 0", None),
        ("DELETE FROM audit_dossier", Some(dossier)),
        ("UPDATE audit_tete_dossier SET rang = 0", Some(dossier)),
        // Rien d'autre que l'audit, même en lecture.
        ("SELECT count(*) FROM dossier", None),
        ("SELECT count(*) FROM utilisateur", None),
    ] {
        let contexte = match dossier_en_contexte {
            None => ContexteTenant::cabinet(c.cabinet, c.administrateur),
            Some(d) => ContexteTenant::dossier(c.cabinet, d, c.administrateur),
        };
        let mut tx = ouvrir_transaction(&env.lecture_audit, contexte)
            .await
            .unwrap();
        let erreur = sqlx::query(requete)
            .execute(tx.connexion())
            .await
            .expect_err(requete);
        assert_eq!(code_sql(&erreur).as_deref(), Some(REFUS), "{requete}");
    }
}

/// Lignes d'audit visibles du rôle de lecture, au contexte donné : (entrées du cabinet,
/// entrées de dossiers, cabinets distincts, dossiers distincts).
async fn visibles(pool: &PgPool, contexte: ContexteTenant) -> (i64, i64, Vec<Uuid>, Vec<Uuid>) {
    let mut tx = ouvrir_transaction(pool, contexte).await.unwrap();
    let cabinet: i64 = sqlx::query_scalar("SELECT count(*) FROM audit_cabinet")
        .fetch_one(tx.connexion())
        .await
        .unwrap();
    let dossier: i64 = sqlx::query_scalar("SELECT count(*) FROM audit_dossier")
        .fetch_one(tx.connexion())
        .await
        .unwrap();
    let cabinets: Vec<Uuid> = sqlx::query_scalar(
        "SELECT DISTINCT cabinet_id FROM audit_cabinet UNION SELECT DISTINCT cabinet_id FROM audit_dossier",
    )
    .fetch_all(tx.connexion())
    .await
    .unwrap();
    let dossiers: Vec<Uuid> = sqlx::query_scalar("SELECT DISTINCT dossier_id FROM audit_dossier")
        .fetch_all(tx.connexion())
        .await
        .unwrap();
    (cabinet, dossier, cabinets, dossiers)
}

#[tokio::test]
async fn la_rls_de_l_audit_isole_les_cabinets_et_les_dossiers() {
    let env = env().await;
    let a = creer_cabinet(&env.admin, "A").await;
    let b = creer_cabinet(&env.admin, "B").await;
    let routeur = routeur_de(&env);
    let a1 = creer_dossier(&routeur, &a, a.administrateur, "A1").await;
    let a2 = creer_dossier(&routeur, &a, a.administrateur, "A2").await;
    let b1 = creer_dossier(&routeur, &b, b.administrateur, "B1").await;

    // Contexte du cabinet A, sans dossier : sa chaîne, aucune entrée de dossier.
    let (cab, dos, cabinets, _) = visibles(
        &env.lecture_audit,
        ContexteTenant::cabinet(a.cabinet, a.administrateur),
    )
    .await;
    assert_eq!((cab, dos), (2, 0));
    assert_eq!(cabinets, [a.cabinet.uuid()]);
    // Contexte du dossier A1 : la chaîne du cabinet A et celle de A1 seulement.
    let (cab, dos, cabinets, dossiers) = visibles(
        &env.lecture_audit,
        ContexteTenant::dossier(a.cabinet, a1, a.administrateur),
    )
    .await;
    assert_eq!((cab, dos), (2, 1));
    assert_eq!(cabinets, [a.cabinet.uuid()]);
    assert_eq!(dossiers, [a1.uuid()]);
    // Dossier de B cité sous le cabinet A : rien.
    let (cab, dos, _, _) = visibles(
        &env.lecture_audit,
        ContexteTenant::dossier(a.cabinet, b1, a.administrateur),
    )
    .await;
    assert_eq!((cab, dos), (2, 0));

    // Rôle applicatif, contexte de A : écrire dans la chaîne de B, ou dans celle d'un autre
    // dossier que celui du contexte, est refusé par `WITH CHECK`.
    for (contexte, requete, cabinet, dossier) in [
        (
            ContexteTenant::cabinet(a.cabinet, a.administrateur),
            "INSERT INTO audit_cabinet (cabinet_id, rang, contenu, empreinte) \
             VALUES ($1, 99, 'x', decode(repeat('00', 32), 'hex'))",
            b.cabinet.uuid(),
            None,
        ),
        (
            ContexteTenant::dossier(a.cabinet, a1, a.administrateur),
            "INSERT INTO audit_dossier (cabinet_id, dossier_id, rang, contenu, empreinte) \
             VALUES ($1, $2, 99, 'x', decode(repeat('00', 32), 'hex'))",
            a.cabinet.uuid(),
            Some(a2.uuid()),
        ),
    ] {
        let mut tx = ouvrir_transaction(&env.application, contexte)
            .await
            .unwrap();
        let mut q = sqlx::query(requete).bind(cabinet);
        if let Some(d) = dossier {
            q = q.bind(d);
        }
        let erreur = q.execute(tx.connexion()).await.expect_err(requete);
        assert_eq!(code_sql(&erreur).as_deref(), Some(REFUS), "{requete}");
    }
}

#[tokio::test]
async fn un_export_de_dossier_ne_contient_rien_d_un_autre_dossier_ni_du_cabinet() {
    let env = env().await;
    let a = creer_cabinet(&env.admin, "A").await;
    let b = creer_cabinet(&env.admin, "B").await;
    let routeur = routeur_de(&env);
    let a1 = creer_dossier(&routeur, &a, a.administrateur, "Premier").await;
    let a2 = creer_dossier(&routeur, &a, a.administrateur, "Second").await;
    let b1 = creer_dossier(&routeur, &b, b.administrateur, "Autre cabinet").await;
    let texte = exporter(&routeur, &a, Some(a1), a.administrateur, None)
        .await
        .texte();
    assert!(texte.contains(&a1.to_string()));
    for etranger in [
        a2.to_string(),
        b1.to_string(),
        b.cabinet.to_string(),
        "portee=cabinet".to_owned(),
        "Second".to_owned(),
    ] {
        assert!(!texte.contains(&etranger), "{etranger}");
    }
}

#[tokio::test]
async fn l_audit_du_cabinet_est_refuse_au_valideur_et_a_l_auditeur() {
    let env = env().await;
    let c = creer_cabinet(&env.admin, "Cabinet").await;
    let valideur = ajouter_affecte(&env.admin, &c, "valideur").await;
    let auditeur = ajouter_affecte(&env.admin, &c, "auditeur").await;
    let routeur = routeur_de(&env);
    creer_dossier(&routeur, &c, c.administrateur, "D").await;
    for membre in [valideur, auditeur] {
        for suffixe in ["", "/verification"] {
            let r = lire(
                &routeur,
                &format!("{}{suffixe}", chemin_audit(&c, None)),
                Some(membre),
            )
            .await;
            assert_eq!(r.statut, StatusCode::FORBIDDEN, "{suffixe}");
        }
        let r = exporter(&routeur, &c, None, membre, None).await;
        assert_eq!(r.statut, StatusCode::FORBIDDEN);
        // Leur dossier affecté leur reste ouvert.
        let r = lire(&routeur, &chemin_audit(&c, Some(c.dossier)), Some(membre)).await;
        assert_eq!(r.statut, StatusCode::OK);
    }
    // L'export exige une session fraîche (ré-authentification depuis moins de 5 minutes),
    // administrateur compris ; un refus n'inscrit rien.
    let r = exporter(&routeur, &c, None, c.administrateur, Some(600)).await;
    assert_eq!(r.statut, StatusCode::FORBIDDEN);
    let r = exporter(&routeur, &c, Some(c.dossier), valideur, Some(600)).await;
    assert_eq!(r.statut, StatusCode::FORBIDDEN);
    assert_eq!(page(&routeur, &c, None).await.len(), 1);
    assert_eq!(page(&routeur, &c, Some(c.dossier)).await.len(), 0);
}

#[tokio::test]
async fn un_echec_d_ecriture_de_l_audit_annule_l_action() {
    let env = env().await;
    let c = creer_cabinet(&env.admin, "Cabinet piégé").await;
    let routeur = routeur_de(&env);
    // Le super-utilisateur occupe le rang 1 de la chaîne du cabinet, sans tête : l'inscription
    // de la prochaine action heurte l'unicité (portée, identifiant, rang).
    sqlx::query(
        "INSERT INTO audit_cabinet (cabinet_id, rang, contenu, empreinte) \
         VALUES ($1, 1, 'intrus', decode(repeat('00', 32), 'hex'))",
    )
    .bind(c.cabinet.uuid())
    .execute(&env.admin)
    .await
    .unwrap();
    let compter = |sql: &'static str| {
        let admin = env.admin.clone();
        async move {
            sqlx::query_scalar::<_, i64>(sql)
                .bind(c.cabinet.uuid())
                .fetch_one(&admin)
                .await
                .unwrap()
        }
    };
    let avant = (
        compter("SELECT count(*) FROM dossier WHERE cabinet_id = $1").await,
        compter("SELECT count(*) FROM acces_dossier WHERE cabinet_id = $1").await,
    );
    let reponse = envoyer(
        &routeur,
        &format!("/api/cabinets/{}/dossiers", c.cabinet),
        Some(c.administrateur),
        r#"{"nom":"Jamais créé"}"#,
        None,
    )
    .await;
    assert_eq!(reponse.statut, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(reponse.json()["erreur"], "erreur_interne");
    // Ni dossier, ni affectation, ni tête, ni entrée de dossier : tout est annulé.
    assert_eq!(
        (
            compter("SELECT count(*) FROM dossier WHERE cabinet_id = $1").await,
            compter("SELECT count(*) FROM acces_dossier WHERE cabinet_id = $1").await,
        ),
        avant
    );
    for sql in [
        "SELECT count(*) FROM audit_tete_cabinet WHERE cabinet_id = $1",
        "SELECT count(*) FROM audit_dossier WHERE cabinet_id = $1",
        "SELECT count(*) FROM audit_tete_dossier WHERE cabinet_id = $1",
    ] {
        assert_eq!(compter(sql).await, 0, "{sql}");
    }
    assert_eq!(
        compter("SELECT count(*) FROM audit_cabinet WHERE cabinet_id = $1").await,
        1
    );
}

#[tokio::test]
async fn des_actions_concurrentes_donnent_des_rangs_continus() {
    let env = env().await;
    let c = creer_cabinet(&env.admin, "Cabinet concurrent").await;
    let routeur = routeur_de(&env);
    let taches: Vec<_> = (0..8)
        .map(|i| {
            let routeur = routeur.clone();
            let chemin = format!("/api/cabinets/{}/dossiers", c.cabinet);
            let admin = c.administrateur;
            tokio::spawn(async move {
                envoyer(
                    &routeur,
                    &chemin,
                    Some(admin),
                    &format!(r#"{{"nom":"D{i}"}}"#),
                    None,
                )
                .await
                .statut
            })
        })
        .collect();
    for t in taches {
        assert_eq!(t.await.unwrap(), StatusCode::CREATED);
    }
    let v = verification(&routeur, &c, None).await;
    assert_eq!(v["integre"], true, "{v}");
    assert_eq!(v["nombre"], 8);
    let rangs: Vec<u64> = page(&routeur, &c, None)
        .await
        .iter()
        .map(|e| e["rang"].as_u64().unwrap())
        .collect();
    assert_eq!(rangs, (1..=8).collect::<Vec<_>>());
}

#[tokio::test]
async fn la_consultation_est_paginee() {
    let env = env().await;
    let c = creer_cabinet(&env.admin, "Cabinet paginé").await;
    let routeur = routeur_de(&env);
    for i in 0..3 {
        creer_dossier(&routeur, &c, c.administrateur, &format!("D{i}")).await;
    }
    let base = chemin_audit(&c, None);
    let r = lire(
        &routeur,
        &format!("{base}?limite=2"),
        Some(c.administrateur),
    )
    .await;
    let page1 = r.json();
    assert_eq!(page1["entrees"].as_array().unwrap().len(), 2);
    assert_eq!(page1["suivant"], 2);
    let r = lire(
        &routeur,
        &format!("{base}?limite=2&apres=2"),
        Some(c.administrateur),
    )
    .await;
    let page2 = r.json();
    assert_eq!(page2["entrees"][0]["rang"], 3);
    assert_eq!(page2["suivant"], Value::Null);
    for invalide in ["?limite=101", "?limite=0", "?apres=-1", "?tri=rang"] {
        let r = lire(
            &routeur,
            &format!("{base}{invalide}"),
            Some(c.administrateur),
        )
        .await;
        assert_eq!(r.statut, StatusCode::BAD_REQUEST, "{invalide}");
    }
}
