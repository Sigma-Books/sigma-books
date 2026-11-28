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
//! - un échec d'écriture de l'audit annule l'action ; chaque mutation laisse une trace ; des
//!   actions concurrentes, au cabinet comme dans un dossier, donnent des rangs continus, et
//!   aucune vérification concurrente ne donne de fausse alerte ;
//! - une entrée réordonnée est détectée ; un rang hors limites dans la pagination donne 400.
#![cfg(feature = "tests-postgres")]
// Fichier de test : les `expect`/`unwrap` y sont voulus.
#![allow(clippy::expect_used, clippy::unwrap_used)]

mod commun;

use api::base::{
    ContexteTenant, ErreurBase, IdCabinet, IdDossier, IdUtilisateur, ouvrir_transaction,
    verifier_role_lecture_audit,
};
use api::serveur::routes::{CasUsage, Methode, ROUTES};
use api::serveur::{Reglages, routeur};
use axum::Router;
use axum::http::StatusCode;
use commun::serveur::{ORIGINE, appeler_route, application_base, envoyer, lire};
use commun::{
    CabinetTest, Env, ROLE_MIGRATION, ajouter_affecte, code_sql, creer_cabinet, env, pool_sous_role,
};
use integrite::audit::{AttentesAudit, verifier_export_audit};
use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

/// SQLSTATE `insufficient_privilege` : droit refusé, ou ligne refusée par `WITH CHECK`.
const REFUS: &str = "42501";

fn routeur_de(env: &Env) -> Router {
    routeur(&application_base(env, Reglages::new(ORIGINE.to_owned())))
}

/// Routeur aux limites de débit relevées, pour les tests de charge concurrente.
fn routeur_sans_limite(env: &Env) -> Router {
    let mut reglages = Reglages::new(ORIGINE.to_owned());
    reglages.debit_utilisateur.capacite = 100_000;
    reglages.debit_adresse.capacite = 100_000;
    routeur(&application_base(env, reglages))
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
        &commun::referentiel::corps_creation(nom),
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
    let rapport =
        verifier_export_audit(reponse.corps.as_slice(), &AttentesAudit::default()).unwrap();
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
    let rapport =
        verifier_export_audit(reponse.corps.as_slice(), &AttentesAudit::default()).unwrap();
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
    // Le message d'échec ne reprend pas la valeur cherchée (identifiants) : seulement son rang.
    for (rang, etranger) in [
        a2.to_string(),
        b1.to_string(),
        b.cabinet.to_string(),
        "portee=cabinet".to_owned(),
        "Second".to_owned(),
    ]
    .iter()
    .enumerate()
    {
        assert!(
            !texte.contains(etranger.as_str()),
            "valeur étrangère n° {rang} présente dans l'export"
        );
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
        &commun::referentiel::corps_creation("Jamais créé"),
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
                    &commun::referentiel::corps_creation(&format!("D{i}")),
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

#[tokio::test]
async fn aucune_fausse_alerte_de_verification_pendant_des_actions_concurrentes() {
    let env = env().await;
    let c = creer_cabinet(&env.admin, "Cabinet actif").await;
    let routeur = routeur_sans_limite(&env);
    let dossier = creer_dossier(&routeur, &c, c.administrateur, "Dossier actif").await;
    // Créations (portée cabinet) et exports (portée dossier) en continu, par 6 tâches…
    let ecritures: Vec<_> = (0..6)
        .map(|t| {
            let routeur = routeur.clone();
            let cabinet = c.cabinet;
            let admin = c.administrateur;
            tokio::spawn(async move {
                for i in 0..10 {
                    let creation = envoyer(
                        &routeur,
                        &format!("/api/cabinets/{cabinet}/dossiers"),
                        Some(admin),
                        &commun::referentiel::corps_creation(&format!("C{t}-{i}")),
                        None,
                    )
                    .await;
                    assert_eq!(creation.statut, StatusCode::CREATED);
                    let export = envoyer(
                        &routeur,
                        &format!("/api/cabinets/{cabinet}/dossiers/{dossier}/audit/export"),
                        Some(admin),
                        "",
                        None,
                    )
                    .await;
                    assert_eq!(export.statut, StatusCode::OK);
                }
            })
        })
        .collect();
    // … pendant que 3 tâches vérifient les deux chaînes sans relâche : aucune fausse alerte.
    let verifications: Vec<_> = (0..3)
        .map(|_| {
            let routeur = routeur.clone();
            tokio::spawn(async move {
                let mut n = 0;
                while n < 80 {
                    for portee in [None, Some(dossier)] {
                        let v = verification(&routeur, &c, portee).await;
                        assert_eq!(v["integre"], true, "fausse alerte : {v}");
                        n += 1;
                    }
                }
            })
        })
        .collect();
    for t in ecritures.into_iter().chain(verifications) {
        t.await.unwrap();
    }
    assert_eq!(verification(&routeur, &c, None).await["nombre"], 61);
    assert_eq!(
        verification(&routeur, &c, Some(dossier)).await["nombre"],
        61
    );
}

#[tokio::test]
async fn des_actions_concurrentes_dans_un_dossier_donnent_des_rangs_continus() {
    let env = env().await;
    let c = creer_cabinet(&env.admin, "Cabinet").await;
    let routeur = routeur_de(&env);
    let dossier = creer_dossier(&routeur, &c, c.administrateur, "Dossier partagé").await;
    let taches: Vec<_> = (0..8)
        .map(|_| {
            let routeur = routeur.clone();
            let chemin = format!(
                "/api/cabinets/{}/dossiers/{dossier}/audit/export",
                c.cabinet
            );
            let admin = c.administrateur;
            tokio::spawn(async move { envoyer(&routeur, &chemin, Some(admin), "", None).await })
        })
        .collect();
    let mut nombres = Vec::new();
    for t in taches {
        let reponse = t.await.unwrap();
        assert_eq!(reponse.statut, StatusCode::OK);
        let rapport =
            verifier_export_audit(reponse.corps.as_slice(), &AttentesAudit::default()).unwrap();
        assert!(rapport.est_integre(), "{rapport}");
        nombres.push(rapport.chaine.nombre);
    }
    // Chaque export a vu un état distinct de la chaîne : 1 à 8 entrées.
    nombres.sort_unstable();
    assert_eq!(nombres, (1..=8).collect::<Vec<_>>());
    let v = verification(&routeur, &c, Some(dossier)).await;
    assert_eq!(v["integre"], true, "{v}");
    assert_eq!(v["nombre"], 9);
    let rangs: Vec<u64> = page(&routeur, &c, Some(dossier))
        .await
        .iter()
        .map(|e| e["rang"].as_u64().unwrap())
        .collect();
    assert_eq!(rangs, (1..=9).collect::<Vec<_>>());
}

#[tokio::test]
async fn deux_entrees_reordonnees_par_le_proprietaire_sont_detectees() {
    let env = env().await;
    let c = creer_cabinet(&env.admin, "Cabinet réordonné").await;
    let routeur = routeur_de(&env);
    creer_dossier(&routeur, &c, c.administrateur, "Premier").await;
    creer_dossier(&routeur, &c, c.administrateur, "Second").await;
    assert_eq!(verification(&routeur, &c, None).await["integre"], true);
    // Échange des rangs 1 et 2 (en trois temps : la clé primaire est vérifiée ligne à ligne).
    for sql in [
        "UPDATE audit_cabinet SET rang = 100 WHERE rang = 1",
        "UPDATE audit_cabinet SET rang = 1 WHERE rang = 2",
        "UPDATE audit_cabinet SET rang = 2 WHERE rang = 100",
    ] {
        assert_eq!(sous_proprietaire(c.cabinet, None, sql).await, 1, "{sql}");
    }
    let v = verification(&routeur, &c, None).await;
    assert_eq!(v["integre"], false, "{v}");
    let codes: Vec<&str> = v["anomalies"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["code"].as_str().unwrap())
        .collect();
    assert!(codes.contains(&"rang_different"), "{v}");
    assert!(codes.contains(&"empreinte_alteree"), "{v}");
}

#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn chaque_mutation_produit_des_traces_sur_sa_portee() {
    let env = env().await;
    let c = creer_cabinet(&env.admin, "Cabinet").await;
    // Référentiel du dossier et ses sous-ressources (sous-compte, tiers) : PR 11.
    let fixtures =
        commun::referentiel::doter(&env.application, c.cabinet, c.dossier, c.administrateur).await;
    let routeur = routeur_de(&env);
    // Voie pièces (PR 12) : `{piece}` désigne la dernière pièce créée par une route précédente.
    let mut piece = uuid::Uuid::new_v4();
    // Voie brouillard (PR 13a) : `{brouillon}` est celui que crée la première route du module ;
    // la suppression porte sur un second brouillon (les routes suivantes gardent le premier) ;
    // le lien porte sur une pièce réelle.
    let mut brouillon = uuid::Uuid::nil();
    let mut version = 1;
    // Voie 13c : écriture validée par la route de validation, contre-passée plus loin.
    let mut validee = uuid::Uuid::nil();
    let piece_liee =
        commun::pieces::doter(&env.admin, c.cabinet, c.dossier, c.administrateur).await;
    // Voie administration (PR 9a) : `{membre}` est un membre jetable du cabinet.
    let membre = commun::ajouter_affecte(&env.admin, &c, "auditeur")
        .await
        .uuid();
    // `{invitation}` : celle que crée la route d'invitation, qui précède celle du renvoi.
    let mut invitation = uuid::Uuid::nil();
    // `{reinitialisation}` : celle que crée la route de demande, contestée ensuite par le membre.
    let mut reinitialisation = uuid::Uuid::nil();
    // Trésorerie (L3, PR 11) : `{compte_tresorerie}` est une caisse fixe.
    let caisse =
        commun::tresorerie::doter(&env.admin, c.cabinet, c.dossier, c.administrateur).await;
    // Relevés (L3, PR 12a) : `{profil}` est celui que crée la route de création, qui précède
    // celle de la nouvelle version.
    let mut profil = uuid::Uuid::nil();
    // Échéancier (L3, PR 8) : `{condition}` est une condition fixe.
    let condition = commun::echeancier::doter(&env.admin, c.cabinet, c.dossier).await;
    // Lettrage (L3, PR 9a) : `{lettrage}` est celui que crée la route de lettrage, rompu ensuite.
    let mut lettrage = uuid::Uuid::nil();
    for route in ROUTES
        .iter()
        .filter(|r| r.methode == Methode::Post && r.portee.du_cabinet())
    {
        let (cabinet_avant, dossier_avant) = (
            verification(&routeur, &c, None).await["nombre"]
                .as_u64()
                .unwrap(),
            verification(&routeur, &c, Some(c.dossier)).await["nombre"]
                .as_u64()
                .unwrap(),
        );
        let chemin = commun::chemin_concret(
            route.chemin,
            c.cabinet.uuid(),
            c.dossier.uuid(),
            |nom| match nom {
                "piece" if matches!(route.cas_usage, CasUsage::Brouillard(_)) => piece_liee,
                "piece" => piece,
                "brouillon" => brouillon,
                "membre" => membre,
                "invitation" => invitation,
                "reinitialisation" => reinitialisation,
                "ecriture" => validee,
                "compte_tresorerie" => caisse,
                "profil" => profil,
                "condition" => condition,
                "lettrage" => lettrage,
                autre => fixtures.sous_ressource(autre),
            },
        );
        let reponse = if let CasUsage::Brouillard(cas) = route.cas_usage {
            use api::serveur::routes::brouillard::Cas;
            let (chemin, corps) = match cas {
                Cas::Modifier => (
                    chemin,
                    commun::brouillard::saisie("Libellé modifié", Some(version)),
                ),
                Cas::Supprimer => {
                    let second = commun::brouillard::doter(
                        &env.admin,
                        c.cabinet,
                        c.dossier,
                        c.administrateur,
                    )
                    .await;
                    (
                        chemin.replace(&brouillon.to_string(), &second.to_string()),
                        r#"{"version":1}"#.to_owned(),
                    )
                }
                Cas::LierPiece => (
                    chemin,
                    format!(
                        r#"{{"piece":"{piece_liee}","role":"principale","version":{version}}}"#
                    ),
                ),
                Cas::RetirerPiece => (chemin, format!(r#"{{"version":{version}}}"#)),
                // Un brouillon à part, justifié par la pièce, validé par un valideur (le
                // créateur ne valide pas : séparation des tâches).
                Cas::Valider => {
                    let second = commun::brouillard::doter(
                        &env.admin,
                        c.cabinet,
                        c.dossier,
                        c.administrateur,
                    )
                    .await;
                    sqlx::query(
                        "INSERT INTO brouillon_piece (cabinet_id, dossier_id, brouillon_id, \
                         piece_id, role, lie_par) VALUES ($1, $2, $3, $4, 'principale', $5)",
                    )
                    .bind(c.cabinet.uuid())
                    .bind(c.dossier.uuid())
                    .bind(second)
                    .bind(piece_liee)
                    .bind(c.administrateur.uuid())
                    .execute(&env.admin)
                    .await
                    .unwrap();
                    (
                        chemin.replace(&brouillon.to_string(), &second.to_string()),
                        r#"{"version":1}"#.to_owned(),
                    )
                }
                // Lot : un brouillon justifié, validé par un valideur (session fraîche).
                Cas::ValiderParLot => {
                    let second = commun::brouillard::doter(
                        &env.admin,
                        c.cabinet,
                        c.dossier,
                        c.administrateur,
                    )
                    .await;
                    sqlx::query(
                        "INSERT INTO brouillon_piece (cabinet_id, dossier_id, brouillon_id, \
                         piece_id, role, lie_par) VALUES ($1, $2, $3, $4, 'principale', $5)",
                    )
                    .bind(c.cabinet.uuid())
                    .bind(c.dossier.uuid())
                    .bind(second)
                    .bind(piece_liee)
                    .bind(c.administrateur.uuid())
                    .execute(&env.admin)
                    .await
                    .unwrap();
                    (
                        chemin,
                        format!(r#"{{"ecritures":[{{"ecriture":"{second}","version":1}}]}}"#),
                    )
                }
                Cas::ContrePasser => (
                    chemin,
                    format!(
                        r#"{{"date":"{}","libelle":"Contre-passation","motif":"Erreur de saisie"}}"#,
                        commun::brouillard::texte_date(commun::brouillard::aujourd_hui())
                    ),
                ),
                _ => (chemin, commun::serveur::corps_de(route).to_owned()),
            };
            // Le drapeau se lève par un autre acteur que celui qui l'a posé.
            let acteur = if cas == Cas::LeverARevoir {
                c.collaborateur
            } else if cas == Cas::Valider || cas == Cas::ValiderParLot {
                commun::ajouter_affecte(&env.admin, &c, "valideur").await
            } else {
                c.administrateur
            };
            commun::serveur::envoyer(&routeur, &chemin, Some(acteur), &corps, None).await
        } else if route.cas_usage
            == CasUsage::Administration(
                api::serveur::routes::administration::Cas::ActiverSeparation,
            )
        {
            // Active par défaut : on la désactive d'abord pour que la route ait un effet.
            sqlx::query("UPDATE cabinet SET separation_taches = false WHERE id = $1")
                .bind(c.cabinet.uuid())
                .execute(&env.admin)
                .await
                .unwrap();
            appeler_route(&routeur, route, &chemin, Some(c.administrateur)).await
        } else if route.cas_usage
            == CasUsage::Administration(
                api::serveur::routes::administration::Cas::ContesterReinitialisation,
            )
        {
            // Contestation par le membre concerné, non par un administrateur ; la route de
            // suspension qui précède l'a suspendu : on le rétablit.
            sqlx::query("UPDATE appartenance SET actif = true WHERE utilisateur_id = $1")
                .bind(membre)
                .execute(&env.admin)
                .await
                .unwrap();
            commun::serveur::envoyer(
                &routeur,
                &chemin,
                Some(api::base::IdUtilisateur::depuis_uuid(membre)),
                "",
                None,
            )
            .await
        } else if route.cas_usage
            == CasUsage::Administration(api::serveur::routes::administration::Cas::Affecter)
        {
            // Affectation d'un dossier réel du cabinet.
            let corps = format!(r#"{{"dossier":"{}"}}"#, c.autre_dossier);
            commun::serveur::envoyer(&routeur, &chemin, Some(c.administrateur), &corps, None).await
        } else if route.cas_usage
            == CasUsage::ParametresFiscaux(api::serveur::routes::parametres_fiscaux::Cas::Creer)
        {
            // Une version propre qui change vraiment un taux (sinon : 422 « aucun changement »).
            commun::serveur::envoyer(
                &routeur,
                &chemin,
                Some(c.administrateur),
                &format!(
                    r#"{{"date_effet":"{}","motif":"Test d'audit","taux":{{"normal":"19"}}}}"#,
                    (time::OffsetDateTime::now_utc().date() + time::Duration::days(30))
                ),
                None,
            )
            .await
        } else if route.cas_usage
            == CasUsage::Echeancier(api::serveur::routes::echeancier::Cas::ReviserEcheance)
        {
            // Révision de la ligne 1 d'une écriture validée fixe à ligne de tiers, créée ici
            // (celle de la route de validation n'a pas de tiers), après les routes d'intégrité.
            let ecriture = commun::echeancier::doter_ecriture_tiers(
                &env.admin,
                c.cabinet,
                c.dossier,
                c.administrateur,
                fixtures.tiers,
                1000,
            )
            .await;
            commun::serveur::envoyer(
                &routeur,
                &chemin.replace(&validee.to_string(), &ecriture.to_string()),
                Some(c.administrateur),
                commun::serveur::corps_de(route),
                None,
            )
            .await
        } else if route.cas_usage
            == CasUsage::Lettrage(api::serveur::routes::lettrage::Cas::Lettrer)
        {
            // Deux lignes validées lettrables du tiers fixe (le corps d'exemple cite des lignes
            // inexistantes).
            let mut ecritures = Vec::new();
            for sens in ["debit", "credit"] {
                ecritures.push(
                    commun::lettrage::ecriture_lettrable(
                        &env.admin,
                        c.cabinet,
                        c.dossier,
                        c.administrateur,
                        fixtures.tiers,
                        sens,
                    )
                    .await,
                );
            }
            let corps = commun::lettrage::corps_lettrer(ecritures[0], ecritures[1]);
            commun::serveur::envoyer(&routeur, &chemin, Some(c.administrateur), &corps, None).await
        } else if route.cas_usage
            == CasUsage::Organisation(api::serveur::routes::organisation::Cas::ModifierContexte)
        {
            // Une modification qui change vraiment le contexte (sinon : 409 « aucun changement »).
            commun::serveur::envoyer(
                &routeur,
                &chemin,
                Some(c.administrateur),
                r#"{"fuseau":"Africa/Douala"}"#,
                None,
            )
            .await
        } else {
            appeler_route(&routeur, route, &chemin, Some(c.administrateur)).await
        };
        assert!(
            reponse.statut.is_success(),
            "{} : {}",
            route.chemin,
            reponse.texte()
        );
        if let CasUsage::Brouillard(cas) = route.cas_usage {
            use api::serveur::routes::brouillard::Cas;
            match cas {
                Cas::Creer => {
                    brouillon = reponse.json()["id"].as_str().unwrap().parse().unwrap();
                }
                Cas::Modifier | Cas::LierPiece | Cas::RetirerPiece => {
                    version = i32::try_from(reponse.json()["version"].as_i64().unwrap()).unwrap();
                }
                Cas::Valider => {
                    validee = reponse.json()["id"].as_str().unwrap().parse().unwrap();
                }
                _ => {}
            }
        } else if route.cas_usage
            == CasUsage::Administration(api::serveur::routes::administration::Cas::Inviter)
        {
            invitation = reponse.json()["id"].as_str().unwrap().parse().unwrap();
        } else if route.cas_usage
            == CasUsage::Administration(api::serveur::routes::administration::Cas::Reinitialiser)
        {
            reinitialisation = reponse.json()["id"].as_str().unwrap().parse().unwrap();
        } else if route.cas_usage
            == CasUsage::Releves(api::serveur::routes::releves::Cas::CreerProfil)
        {
            profil = reponse.json()["id"].as_str().unwrap().parse().unwrap();
        } else if route.cas_usage
            == CasUsage::Lettrage(api::serveur::routes::lettrage::Cas::Lettrer)
        {
            lettrage = reponse.json()["id"].as_str().unwrap().parse().unwrap();
        } else if route.chemin.contains("/pieces") && reponse.statut == StatusCode::CREATED {
            piece = reponse.json()["id"].as_str().unwrap().parse().unwrap();
        }
        let (cabinet_apres, dossier_apres) = (
            verification(&routeur, &c, None).await["nombre"]
                .as_u64()
                .unwrap(),
            verification(&routeur, &c, Some(c.dossier)).await["nombre"]
                .as_u64()
                .unwrap(),
        );
        // La portée de la route reçoit au moins une entrée.
        let trace = if route.chemin.contains("{dossier}") {
            dossier_apres > dossier_avant
        } else {
            cabinet_apres > cabinet_avant
        };
        assert!(
            trace,
            "{} {} : aucune trace",
            route.methode.nom(),
            route.chemin
        );
    }
}

#[tokio::test]
async fn un_rang_hors_limites_dans_la_pagination_donne_400() {
    let env = env().await;
    let c = creer_cabinet(&env.admin, "Cabinet").await;
    let routeur = routeur_de(&env);
    for portee in [None, Some(c.dossier)] {
        let base = chemin_audit(&c, portee);
        let r = lire(
            &routeur,
            &format!("{base}?apres=9223372036854775808"),
            Some(c.administrateur),
        )
        .await;
        assert_eq!(r.statut, StatusCode::BAD_REQUEST, "{}", r.texte());
        let r = lire(
            &routeur,
            &format!("{base}?apres=9223372036854775807"),
            Some(c.administrateur),
        )
        .await;
        assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
        assert_eq!(r.json()["entrees"], serde_json::json!([]));
    }
}
