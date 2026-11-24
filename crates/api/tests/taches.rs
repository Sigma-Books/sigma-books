//! File de tâches et horodatage quotidien (PR 15a, L2-30 ; ADR 0010 ; F2.5, F6.2, F6.3) :
//! planification par la validation et par le verrouillage, passage du travailleur, rejeu après
//! échec, un jeton par jour, alerte au-delà de 24 h, reprise après arrêt, isolation de la file,
//! alerte d'export d'audit (#157). L'autorité est simulée (aucun accès réseau) : elle rend le
//! jeton figé de la PR 5 de L1 ; le client réel est testé dans `src/horodatage` et
//! `tests/horodatage.rs`.
#![cfg(feature = "tests-postgres")]
// Fichier de test : les `expect`/`unwrap` y sont voulus.
#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::too_many_lines,
    clippy::assert_is_empty,
    clippy::format_collect,
    clippy::manual_map
)]

mod commun;

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex as MutexStd};

use api::horodatage::tache::{Bilan, Reglages, Travailleur};
use api::horodatage::{Autorite, EchecAutorite, ErreurHorodatage, Horodateur, JetonObtenu, Nonce};
use axum::http::StatusCode;
use commun::banc::{Banc, banc};
use commun::serveur::{HorlogeTest, envoyer};
use integrite::horodatage::{JetonVerifie, Temoin};
use sqlx::PgPool;
use time::Duration;
use tokio::sync::{Mutex, Notify};
use uuid::Uuid;

const JETON_FIGE: &[u8] = include_bytes!("../../integrite/tests/vecteurs/horodatage/digicert.tsr");

/// Les tests partagent la base et la file : un seul passage de travailleur à la fois.
static SERIE: Mutex<()> = Mutex::const_new(());

/// Autorité simulée : rend le jeton figé, ou échoue.
struct AbandonAuDrop(tokio::task::JoinHandle<()>);

impl Drop for AbandonAuDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

type Pause = (String, Arc<Notify>, Arc<Notify>);

#[derive(Debug, Default)]
struct Simulee {
    panne: MutexStd<bool>,
    temoins: MutexStd<Vec<Temoin>>,
    /// Dossier pour lequel l'appel s'arrête (signale `arrive`, attend `libere`).
    pause: MutexStd<Option<Pause>>,
    /// Dossier pour lequel l'appel panique, une seule fois.
    panique: MutexStd<Option<String>>,
}

impl Simulee {
    fn en_panne(&self, panne: bool) {
        *self.panne.lock().unwrap() = panne;
    }

    fn faire_pause(&self, dossier: Uuid) -> (Arc<Notify>, Arc<Notify>) {
        let (arrive, libere) = (Arc::new(Notify::new()), Arc::new(Notify::new()));
        *self.pause.lock().unwrap() = Some((dossier.to_string(), arrive.clone(), libere.clone()));
        (arrive, libere)
    }

    fn contenus(&self) -> Vec<String> {
        self.temoins
            .lock()
            .unwrap()
            .iter()
            .map(Temoin::contenu)
            .collect()
    }
}

impl Horodateur for Simulee {
    fn horodater<'a>(
        &'a self,
        temoin: &'a Temoin,
        _nonce: Nonce,
    ) -> Pin<Box<dyn Future<Output = Result<JetonObtenu, ErreurHorodatage>> + Send + 'a>> {
        Box::pin(async move {
            self.temoins.lock().unwrap().push(temoin.clone());
            let dossier = temoin.dossier().as_str().to_owned();
            let panique = {
                let mut p = self.panique.lock().unwrap();
                if p.as_deref() == Some(dossier.as_str()) {
                    p.take()
                } else {
                    None
                }
            };
            assert!(panique.is_none(), "panique voulue de l'autorité simulée");
            let pause = self.pause.lock().unwrap().clone();
            if let Some((d, arrive, libere)) = pause
                && d == dossier
            {
                arrive.notify_one();
                libere.notified().await;
            }
            let autorite = Autorite::parse("http://tsa.exemple.invalid").unwrap();
            if *self.panne.lock().unwrap() {
                return Err(ErreurHorodatage::ToutesEnEchec(vec![(
                    autorite,
                    EchecAutorite::DelaiTotal,
                )]));
            }
            Ok(JetonObtenu {
                autorite,
                jeton: JETON_FIGE.to_vec(),
                verifie: JetonVerifie {
                    date: time::OffsetDateTime::now_utc(),
                    autorite: "Autorité simulée".to_owned(),
                    politique: "1.2.3".to_owned(),
                    ancre: "Ancre simulée".to_owned(),
                },
            })
        })
    }
}

struct Atelier {
    b: Banc,
    autorite: Arc<Simulee>,
    horloge: HorlogeTest,
    travailleur: Travailleur,
}

async fn atelier() -> Atelier {
    atelier_avec(Reglages::default()).await
}

async fn atelier_avec(reglages: Reglages) -> Atelier {
    let b = banc().await;
    let autorite = Arc::new(Simulee::default());
    let horloge = HorlogeTest::default();
    let reglages = Reglages {
        lot: 10_000,
        ..reglages
    };
    let travailleur = Travailleur::new(
        b.env.application.clone(),
        autorite.clone(),
        Arc::new(horloge.clone()),
        reglages,
    );
    Atelier {
        b,
        autorite,
        horloge,
        travailleur,
    }
}

impl Atelier {
    async fn valider_une(&self, libelle: &str) {
        let (id, version, _) = self.b.pret(libelle).await;
        let r = self.b.valider(id, version).await;
        assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
    }

    /// Appels à l'autorité pour le dossier de l'atelier (d'autres processus de test créent des
    /// tâches échues, que le travailleur prend aussi).
    fn appels(&self) -> usize {
        let dossier = self.b.c.dossier.uuid().to_string();
        self.autorite
            .contenus()
            .iter()
            .filter(|c| c.contains(&dossier))
            .count()
    }

    async fn passer(&self) -> Bilan {
        self.travailleur.passer().await.unwrap()
    }

    /// Tâches du dossier : (nature, sujet, état).
    async fn taches(&self) -> Vec<(String, String, String)> {
        sqlx::query_as(
            "SELECT nature, sujet, etat FROM tache WHERE dossier_cible = $1 \
             ORDER BY nature, sujet, cree_le",
        )
        .bind(self.b.c.dossier.uuid())
        .fetch_all(&self.b.env.admin)
        .await
        .unwrap()
    }

    /// Jetons du dossier : (nombre, tête).
    async fn jetons(&self) -> Vec<(i64, Vec<u8>)> {
        sqlx::query_as(
            "SELECT nombre, tete FROM jeton_horodatage WHERE dossier_id = $1 ORDER BY nombre",
        )
        .bind(self.b.c.dossier.uuid())
        .fetch_all(&self.b.env.admin)
        .await
        .unwrap()
    }

    /// Tâches d'horodatage seules (l'horloge avancée de plusieurs jours crée aussi une alerte).
    async fn horodatages(&self) -> Vec<(String, String, String)> {
        self.taches()
            .await
            .into_iter()
            .filter(|t| t.0 == "horodatage")
            .collect()
    }

    async fn notifications(&self, sujet: &str) -> i64 {
        sqlx::query_scalar(
            "SELECT count(*) FROM tache WHERE nature = 'notification' AND sujet = $2 \
             AND cabinet_cible = $1",
        )
        .bind(self.b.c.cabinet.uuid())
        .bind(sujet)
        .fetch_one(&self.b.env.admin)
        .await
        .unwrap()
    }
}

#[tokio::test]
async fn la_validation_planifie_une_seule_tache_quotidienne_pour_le_jour_suivant() {
    let _s = SERIE.lock().await;
    let a = atelier().await;
    assert!(a.taches().await.is_empty());
    a.valider_une("Première").await;
    a.valider_une("Seconde").await;
    assert_eq!(
        a.taches().await,
        vec![("horodatage".into(), "quotidien".into(), "en_attente".into())]
    );
    // Échéance : le début du jour suivant (UTC), jamais le jour même.
    let (echue, avant_minuit): (bool, bool) = sqlx::query_as(
        "SELECT prochaine_tentative <= now(), \
                prochaine_tentative = ((now() AT TIME ZONE 'UTC')::date + 1)::timestamp \
                                      AT TIME ZONE 'UTC' \
         FROM tache WHERE dossier_cible = $1",
    )
    .bind(a.b.c.dossier.uuid())
    .fetch_one(&a.b.env.admin)
    .await
    .unwrap();
    assert!(!echue);
    assert!(avant_minuit);
    // Aujourd'hui : rien n'est dû.
    a.passer().await;
    assert!(a.jetons().await.is_empty());
}

#[tokio::test]
async fn le_passage_quotidien_obtient_un_jeton_pour_la_tete_et_l_inscrit_a_l_audit() {
    let _s = SERIE.lock().await;
    let a = atelier().await;
    a.valider_une("Première").await;
    a.horloge.avancer(Duration::days(2));
    let bilan = a.passer().await;
    assert_eq!((bilan.prises >= 1, bilan.erreurs), (true, 0));
    let chaine = a.b.chaine().await;
    let jetons = a.jetons().await;
    assert_eq!(jetons.len(), 1);
    assert_eq!(jetons[0].0, 1);
    assert_eq!(jetons[0].1, chaine[0].3, "jeton de la tête de la chaîne");
    assert_eq!(
        a.taches().await,
        vec![("horodatage".into(), "quotidien".into(), "terminee".into())]
    );
    // F6.3 : l'autorité ne reçoit que le témoin (dossier, tête, nombre), sans donnée comptable.
    let envoyes: Vec<String> = a
        .autorite
        .contenus()
        .into_iter()
        .filter(|c| c.contains(&a.b.c.dossier.uuid().to_string()))
        .collect();
    assert_eq!(envoyes.len(), 1);
    let tete: String = chaine[0].3.iter().map(|o| format!("{o:02x}")).collect();
    assert_eq!(
        envoyes[0],
        format!(
            "sigma-books temoin v1\ndossier={}\ntete={tete}\nnombre=1\n",
            a.b.c.dossier.uuid()
        )
    );
    // Audit du dossier : acteur système, sans donnée comptable.
    let entree: String = sqlx::query_scalar(
        "SELECT contenu FROM audit_dossier WHERE dossier_id = $1 \
         AND contenu LIKE '%ObtenirJetonHorodatage%'",
    )
    .bind(a.b.c.dossier.uuid())
    .fetch_one(&a.b.env.admin)
    .await
    .unwrap();
    assert!(entree.contains("systeme"), "{entree}");
    assert!(!entree.contains("Première"), "{entree}");
    // Rien de nouveau : un second passage ne redemande rien.
    let appels = a.appels();
    a.passer().await;
    assert_eq!(a.appels(), appels);
    assert_eq!(a.jetons().await.len(), 1);
}

#[tokio::test]
async fn un_echec_est_rejoue_avec_un_delai_croissant_sur_la_tete_du_moment() {
    let _s = SERIE.lock().await;
    let a = atelier().await;
    a.valider_une("Première").await;
    a.horloge.avancer(Duration::days(2));
    a.autorite.en_panne(true);
    let bilan = a.passer().await;
    assert!(bilan.reportees >= 1);
    assert!(a.jetons().await.is_empty());
    let (etat, tentatives, issue): (String, i32, Option<String>) = sqlx::query_as(
        "SELECT etat, tentatives, derniere_issue FROM tache WHERE dossier_cible = $1",
    )
    .bind(a.b.c.dossier.uuid())
    .fetch_one(&a.b.env.admin)
    .await
    .unwrap();
    assert_eq!((etat.as_str(), tentatives), ("en_attente", 1));
    assert_eq!(issue.as_deref(), Some("autorites"));
    // Avant le délai de rejeu (une minute) : rien.
    let appels = a.appels();
    a.passer().await;
    assert_eq!(a.appels(), appels);
    // Une écriture de plus est validée pendant la panne ; l'autorité revient : le jeton couvre
    // les deux écritures (rejeu différé, ADR 0010).
    a.valider_une("Seconde").await;
    a.autorite.en_panne(false);
    a.horloge.avancer(Duration::minutes(2));
    a.passer().await;
    let jetons = a.jetons().await;
    assert_eq!(jetons.len(), 1);
    assert_eq!(jetons[0].0, 2);
    let chaine = a.b.chaine().await;
    assert_eq!(jetons[0].1, chaine[1].3);
}

#[tokio::test]
async fn au_plus_un_jeton_par_jour_puis_le_lendemain() {
    let _s = SERIE.lock().await;
    let a = atelier().await;
    a.valider_une("Première").await;
    a.horloge.avancer(Duration::days(2));
    a.passer().await;
    assert_eq!(a.jetons().await.len(), 1);
    // Nouvelle écriture le même jour : une tâche attend, mais la règle diffère au lendemain.
    a.valider_une("Seconde").await;
    a.passer().await;
    // La tâche est échue pour l'horloge de la base, pas pour celle des tests : on la rend
    // échue.
    sqlx::query(
        "UPDATE tache SET prochaine_tentative = now() - interval '1 minute' \
         WHERE dossier_cible = $1 AND etat = 'en_attente'",
    )
    .bind(a.b.c.dossier.uuid())
    .execute(&a.b.env.admin)
    .await
    .unwrap();
    let bilan = a.passer().await;
    assert!(bilan.reportees >= 1);
    assert_eq!(a.jetons().await.len(), 1, "un seul jeton ce jour-là");
    a.horloge.avancer(Duration::days(1));
    a.passer().await;
    let jetons = a.jetons().await;
    assert_eq!(jetons.iter().map(|j| j.0).collect::<Vec<_>>(), [1, 2]);
}

#[tokio::test]
async fn le_verrouillage_horodate_le_jour_meme_et_prepare_le_temoin_du_cabinet() {
    let _s = SERIE.lock().await;
    let a = atelier().await;
    a.valider_une("Première").await;
    let periode: Uuid =
        sqlx::query_scalar("SELECT id FROM periode WHERE dossier_id = $1 ORDER BY debut LIMIT 1")
            .bind(a.b.c.dossier.uuid())
            .fetch_one(&a.b.env.admin)
            .await
            .unwrap();
    let chemin = format!(
        "/api/cabinets/{}/dossiers/{}/periodes/{periode}/verrouillage",
        a.b.c.cabinet, a.b.c.dossier
    );
    let r = envoyer(&a.b.routeur, &chemin, Some(a.b.valideur), "{}", None).await;
    assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
    let taches = a.taches().await;
    assert!(
        taches.contains(&(
            "horodatage".into(),
            "verrouillage".into(),
            "en_attente".into()
        )),
        "{taches:?}"
    );
    // Échue tout de suite : un passage le jour même suffit (horloge non avancée).
    a.passer().await;
    assert_eq!(a.jetons().await.len(), 1);
    assert_eq!(a.notifications("temoin").await, 1);
    // Un verrouillage refusé (période déjà verrouillée) ne laisse aucune tâche de plus.
    let avant = a.taches().await.len();
    let r = envoyer(&a.b.routeur, &chemin, Some(a.b.valideur), "{}", None).await;
    assert_eq!(r.statut, StatusCode::CONFLICT, "{}", r.texte());
    assert_eq!(a.taches().await.len(), avant);
}

#[tokio::test]
async fn une_alerte_de_retard_apres_24_heures_une_seule_fois() {
    let _s = SERIE.lock().await;
    let a = atelier().await;
    a.valider_une("Première").await;
    a.autorite.en_panne(true);
    a.horloge.avancer(Duration::hours(23));
    a.passer().await;
    assert_eq!(a.notifications("retard_horodatage").await, 0, "avant 24 h");
    a.horloge.avancer(Duration::hours(2));
    let bilan = a.passer().await;
    assert!(bilan.alertes >= 1);
    assert_eq!(a.notifications("retard_horodatage").await, 1);
    // Pas de doublon tant que l'alerte n'est pas traitée.
    a.horloge.avancer(Duration::hours(1));
    a.passer().await;
    assert_eq!(a.notifications("retard_horodatage").await, 1);
    // L'autorité revient : le jeton est obtenu, la chaîne est couverte.
    a.autorite.en_panne(false);
    a.horloge.avancer(Duration::hours(2));
    a.passer().await;
    assert_eq!(a.jetons().await.len(), 1);
}

#[tokio::test]
async fn une_tache_prise_puis_abandonnee_est_reprise_apres_son_bail() {
    let _s = SERIE.lock().await;
    let a = atelier().await;
    a.valider_une("Première").await;
    // Arrêt du processus pendant l'appel : la tâche reste « en cours », bail passé.
    sqlx::query(
        "UPDATE tache SET etat = 'en_cours', reserve_jusqua = now() - interval '1 minute', \
                          prise = gen_random_uuid(), tentatives = 1, prochaine_tentative = now() - interval '2 minutes' \
         WHERE dossier_cible = $1",
    )
    .bind(a.b.c.dossier.uuid())
    .execute(&a.b.env.admin)
    .await
    .unwrap();
    a.passer().await;
    assert_eq!(a.jetons().await.len(), 1);
    let tentatives: i32 =
        sqlx::query_scalar("SELECT tentatives FROM tache WHERE dossier_cible = $1")
            .bind(a.b.c.dossier.uuid())
            .fetch_one(&a.b.env.admin)
            .await
            .unwrap();
    assert_eq!(tentatives, 2);
    // Une tâche dont le bail court encore n'est pas reprise.
    a.valider_une("Seconde").await;
    sqlx::query(
        "UPDATE tache SET etat = 'en_cours', reserve_jusqua = now() + interval '1 hour', \
                          prise = gen_random_uuid(), prochaine_tentative = now() - interval '2 minutes' \
         WHERE dossier_cible = $1 AND etat = 'en_attente'",
    )
    .bind(a.b.c.dossier.uuid())
    .execute(&a.b.env.admin)
    .await
    .unwrap();
    let appels = a.appels();
    a.passer().await;
    assert_eq!(a.appels(), appels);
}

#[tokio::test]
async fn un_jeton_ne_se_modifie_ni_ne_se_supprime() {
    let _s = SERIE.lock().await;
    let a = atelier().await;
    a.valider_une("Première").await;
    a.horloge.avancer(Duration::days(2));
    a.passer().await;
    assert_eq!(a.jetons().await.len(), 1);
    let ctx = |pool: &PgPool| {
        let pool = pool.clone();
        let (c, d) = (a.b.c.cabinet.to_string(), a.b.c.dossier.to_string());
        async move {
            let mut tx = pool.begin().await.unwrap();
            sqlx::query("SELECT set_config('sigma.cabinet', $1, true), set_config('sigma.dossier', $2, true)")
                .bind(c)
                .bind(d)
                .execute(&mut *tx)
                .await
                .unwrap();
            tx
        }
    };
    let mut tx = ctx(&a.b.env.application).await;
    assert!(
        sqlx::query("UPDATE jeton_horodatage SET nombre = 9")
            .execute(&mut *tx)
            .await
            .is_err()
    );
    let mut tx = ctx(&a.b.env.application).await;
    assert!(
        sqlx::query("DELETE FROM jeton_horodatage")
            .execute(&mut *tx)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn la_file_est_cloisonnee_par_cabinet_et_le_tenant_ne_la_modifie_pas() {
    let _s = SERIE.lock().await;
    let a = atelier().await;
    let autre = atelier().await;
    a.valider_une("Première").await;
    autre.valider_une("Autre").await;
    let pool = &a.b.env.application;
    let lire = |cabinet: Option<Uuid>, dossier: Option<Uuid>| async move {
        let mut tx = pool.begin().await.unwrap();
        sqlx::query(
            "SELECT set_config('sigma.cabinet', $1, true), set_config('sigma.dossier', $2, true)",
        )
        .bind(cabinet.map_or_else(String::new, |c| c.to_string()))
        .bind(dossier.map_or_else(String::new, |c| c.to_string()))
        .execute(&mut *tx)
        .await
        .unwrap();
        let n: i64 = sqlx::query_scalar("SELECT count(*) FROM tache")
            .fetch_one(&mut *tx)
            .await
            .unwrap();
        let (cible,): (i64,) = sqlx::query_as(
            "SELECT count(*) FROM tache WHERE cabinet_cible <> COALESCE(NULLIF(current_setting('sigma.cabinet', true), '')::uuid, gen_random_uuid())",
        )
        .fetch_one(&mut *tx)
        .await
        .unwrap();
        let modifiees = sqlx::query("UPDATE tache SET etat = 'terminee', termine_le = now()")
            .execute(&mut *tx)
            .await
            .unwrap()
            .rows_affected();
        let supprimees = sqlx::query("DELETE FROM tache")
            .execute(&mut *tx)
            .await
            .unwrap()
            .rows_affected();
        (n, cible, modifiees, supprimees)
    };
    // Sans contexte : aucune ligne.
    assert_eq!(lire(None, None).await, (0, 0, 0, 0));
    // Au contexte du dossier A : sa tâche seule, jamais celle de l'autre cabinet ; ni mise à
    // jour ni suppression.
    let (n, hors_cible, modifiees, supprimees) =
        lire(Some(a.b.c.cabinet.uuid()), Some(a.b.c.dossier.uuid())).await;
    assert_eq!(n, 1);
    assert_eq!((hors_cible, modifiees, supprimees), (0, 0, 0));
    // Au contexte du cabinet seul : les tâches de dossier ne sont pas visibles.
    assert_eq!(lire(Some(a.b.c.cabinet.uuid()), None).await.0, 0);
    // Création d'une tâche pour un autre cabinet : refusée par la RLS.
    let mut tx = pool.begin().await.unwrap();
    sqlx::query(
        "SELECT set_config('sigma.cabinet', $1, true), set_config('sigma.dossier', $2, true)",
    )
    .bind(a.b.c.cabinet.uuid().to_string())
    .bind(a.b.c.dossier.uuid().to_string())
    .execute(&mut *tx)
    .await
    .unwrap();
    let refus = sqlx::query(
        "INSERT INTO tache (id, nature, sujet, cabinet_cible, dossier_cible, etat, cree_le, prochaine_tentative) \
         VALUES (gen_random_uuid(), 'horodatage', 'cloture', $1, $2, 'en_attente', now(), now())",
    )
    .bind(autre.b.c.cabinet.uuid())
    .bind(autre.b.c.dossier.uuid())
    .execute(&mut *tx)
    .await;
    assert!(refus.is_err());
}

#[tokio::test]
async fn un_export_d_audit_cree_une_alerte_et_un_export_refuse_aucune() {
    let _s = SERIE.lock().await;
    let a = atelier().await;
    let dossier = a.b.c.dossier;
    let chemin_dossier = format!(
        "/api/cabinets/{}/dossiers/{dossier}/audit/export",
        a.b.c.cabinet
    );
    let chemin_cabinet = format!("/api/cabinets/{}/audit/export", a.b.c.cabinet);
    // Refusés : session non fraîche (ré-authentification), rôle sans droit.
    for (chemin, par, anciennete) in [
        (&chemin_dossier, a.b.c.administrateur, Some(600)),
        (&chemin_cabinet, a.b.c.administrateur, Some(600)),
        (&chemin_cabinet, a.b.valideur, None),
    ] {
        let r = envoyer(&a.b.routeur, chemin, Some(par), "", anciennete).await;
        assert_eq!(r.statut, StatusCode::FORBIDDEN, "{}", r.texte());
    }
    assert_eq!(a.notifications("export_audit_dossier").await, 0);
    assert_eq!(a.notifications("export_audit_cabinet").await, 0);
    // Acceptés : une alerte chacun, sans donnée du dossier.
    for chemin in [&chemin_dossier, &chemin_cabinet] {
        let r = envoyer(&a.b.routeur, chemin, Some(a.b.c.administrateur), "", None).await;
        assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
    }
    assert_eq!(a.notifications("export_audit_dossier").await, 1);
    assert_eq!(a.notifications("export_audit_cabinet").await, 1);
    let (acteur, dossier_cible): (Option<Uuid>, Option<Uuid>) = sqlx::query_as(
        "SELECT acteur_cible, dossier_cible FROM tache WHERE sujet = 'export_audit_cabinet' \
         AND cabinet_cible = $1",
    )
    .bind(a.b.c.cabinet.uuid())
    .fetch_one(&a.b.env.admin)
    .await
    .unwrap();
    assert_eq!(acteur, Some(a.b.c.administrateur.uuid()));
    assert_eq!(dossier_cible, None);
    // Un second export : une seconde alerte (une par export).
    envoyer(
        &a.b.routeur,
        &chemin_dossier,
        Some(a.b.c.administrateur),
        "",
        None,
    )
    .await;
    assert_eq!(a.notifications("export_audit_dossier").await, 2);
}

#[tokio::test]
async fn une_validation_pendant_l_appel_a_l_autorite_garde_sa_tache() {
    let _s = SERIE.lock().await;
    let a = atelier().await;
    a.valider_une("Première").await;
    a.horloge.avancer(Duration::days(2));
    let (arrive, libere) = a.autorite.faire_pause(a.b.c.dossier.uuid());
    let travailleur = a.travailleur.clone();
    let passage = tokio::spawn(async move { travailleur.passer().await.unwrap() });
    arrive.notified().await;
    // La tâche est en cours ; une seconde écriture est validée pendant l'appel.
    a.valider_une("Seconde").await;
    libere.notify_one();
    passage.await.unwrap();
    assert_eq!(
        a.horodatages().await,
        vec![
            ("horodatage".into(), "quotidien".into(), "terminee".into()),
            ("horodatage".into(), "quotidien".into(), "en_attente".into()),
        ]
    );
    assert_eq!(
        a.jetons().await.iter().map(|j| j.0).collect::<Vec<_>>(),
        [1]
    );
    // La tâche planifiée pendant l'appel obtient le jeton qui couvre la seconde écriture.
    *a.autorite.pause.lock().unwrap() = None;
    a.horloge.avancer(Duration::days(2));
    a.passer().await;
    assert_eq!(
        a.jetons().await.iter().map(|j| j.0).collect::<Vec<_>>(),
        [1, 2]
    );
}

#[tokio::test]
async fn un_travailleur_dont_le_bail_a_expire_ne_clot_pas_la_prise_du_suivant() {
    let _s = SERIE.lock().await;
    let a = atelier().await;
    a.valider_une("Première").await;
    a.horloge.avancer(Duration::days(2));
    let (arrive, libere) = a.autorite.faire_pause(a.b.c.dossier.uuid());
    let travailleur = a.travailleur.clone();
    let passage = tokio::spawn(async move { travailleur.passer().await.unwrap() });
    arrive.notified().await;
    // Un autre travailleur a repris la tâche : nouvelle prise.
    let n = sqlx::query("UPDATE tache SET prise = gen_random_uuid() WHERE dossier_cible = $1")
        .bind(a.b.c.dossier.uuid())
        .execute(&a.b.env.admin)
        .await
        .unwrap()
        .rows_affected();
    assert_eq!(n, 1);
    libere.notify_one();
    passage.await.unwrap();
    // Le premier n'a pas clos la prise du second.
    assert_eq!(
        a.horodatages().await,
        vec![("horodatage".into(), "quotidien".into(), "en_cours".into())]
    );
}

#[tokio::test]
async fn deux_passages_simultanes_ne_prennent_jamais_la_meme_tache() {
    let _s = SERIE.lock().await;
    let env = commun::env().await;
    let cabinet = Uuid::new_v4();
    for _ in 0..12 {
        sqlx::query(
            "INSERT INTO tache (id, nature, sujet, cabinet_cible, dossier_cible, etat, cree_le, \
                                prochaine_tentative) \
             VALUES (gen_random_uuid(), 'horodatage', 'cloture', $1, gen_random_uuid(), \
                     'en_attente', now(), now() - interval '1 minute')",
        )
        .bind(cabinet)
        .execute(&env.admin)
        .await
        .unwrap();
    }
    let travailleur = || {
        Travailleur::new(
            env.application.clone(),
            Arc::new(Simulee::default()),
            Arc::new(HorlogeTest::default()),
            Reglages {
                lot: 10_000,
                ..Reglages::default()
            },
        )
    };
    let (t1, t2) = (travailleur(), travailleur());
    // Deux passages simultanés se disputent les tâches : un passage s'arrête quand l'autre tient
    // les lignes restantes (`SKIP LOCKED`), le nombre de tâches prises par chacun n'est donc pas
    // garanti. Ce qui l'est : aucune tâche n'est prise deux fois (chaque tâche a exactement une
    // tentative, plus bas) ; un second tour séquentiel termine ce qui reste.
    let (r1, r2) = tokio::join!(t1.passer(), t2.passer());
    let (r1, r2) = (r1.unwrap(), r2.unwrap());
    let r3 = t1.passer().await.unwrap();
    assert!(r1.prises + r2.prises + r3.prises >= 12);
    let lignes: Vec<(String, i32)> =
        sqlx::query_as("SELECT etat, tentatives FROM tache WHERE cabinet_cible = $1")
            .bind(cabinet)
            .fetch_all(&env.admin)
            .await
            .unwrap();
    assert_eq!(lignes.len(), 12);
    // Chaque tâche a été prise une seule fois, et terminée.
    assert!(
        lignes.iter().all(|(e, n)| e == "terminee" && *n == 1),
        "{lignes:?}"
    );
}

#[tokio::test]
async fn la_boucle_supervisee_survit_a_une_panique() {
    let _s = SERIE.lock().await;
    let a = atelier_avec(Reglages {
        periode: std::time::Duration::from_millis(50),
        relance: std::time::Duration::from_millis(50),
        ..Reglages::default()
    })
    .await;
    a.valider_une("Première").await;
    a.horloge.avancer(Duration::days(2));
    *a.autorite.panique.lock().unwrap() = Some(a.b.c.dossier.uuid().to_string());
    let travailleur = a.travailleur.clone();
    // Abandonnée à la fin du test, même en cas d'échec : elle ne doit pas prendre les tâches
    // des autres tests.
    let boucle = AbandonAuDrop(tokio::spawn(async move { travailleur.superviser().await }));
    let attendre = async {
        while a.autorite.panique.lock().unwrap().is_some() {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    };
    tokio::time::timeout(std::time::Duration::from_secs(20), attendre)
        .await
        .expect("la panique devait avoir lieu");
    // La tâche est restée en cours (bail) : passé le bail, la boucle relancée la reprend.
    a.horloge.avancer(Duration::minutes(10));
    let obtenu = async {
        while a.jetons().await.is_empty() {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    };
    tokio::time::timeout(std::time::Duration::from_secs(20), obtenu)
        .await
        .expect("la boucle relancée devait horodater");
    // Abandon, puis attente de l'arrêt effectif de la boucle : une transaction de prise
    // interrompue garde ses verrous jusqu'à son annulation, et ferait sauter la tâche d'un test
    // suivant (`SKIP LOCKED`).
    boucle.0.abort();
    let _ = (&mut { boucle }.0).await;
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
}

#[tokio::test]
async fn un_echec_pendant_qu_une_tache_est_planifiee_fusionne_les_deux() {
    let _s = SERIE.lock().await;
    let a = atelier().await;
    a.valider_une("Première").await;
    a.horloge.avancer(Duration::days(2));
    let (arrive, libere) = a.autorite.faire_pause(a.b.c.dossier.uuid());
    let travailleur = a.travailleur.clone();
    let passage = tokio::spawn(async move { travailleur.passer().await.unwrap() });
    arrive.notified().await;
    a.valider_une("Seconde").await;
    // L'autorité échoue : la tâche en cours doit être reportée, alors qu'une autre attend déjà.
    a.autorite.en_panne(true);
    // La tâche planifiée pendant l'appel est reprise dans le même passage : plus de pause.
    *a.autorite.pause.lock().unwrap() = None;
    libere.notify_one();
    passage.await.unwrap();
    // Après la fusion, les deux tâches ont la même date de création : l'ordre n'est pas celui
    // de l'état, on compare donc trié.
    let mut t = a.horodatages().await;
    t.sort();
    assert_eq!(
        t,
        vec![
            ("horodatage".into(), "quotidien".into(), "en_attente".into()),
            ("horodatage".into(), "quotidien".into(), "terminee".into()),
        ]
    );
    // La tâche restante garde la date de création la plus ancienne (calcul du retard).
    let (plus_ancienne,): (bool,) = sqlx::query_as(
        "SELECT cree_le <= (SELECT min(cree_le) FROM tache WHERE dossier_cible = $1 \
                            AND etat = 'terminee' AND nature = 'horodatage') \
         FROM tache WHERE dossier_cible = $1 AND etat = 'en_attente' \
                      AND nature = 'horodatage'",
    )
    .bind(a.b.c.dossier.uuid())
    .fetch_one(&a.b.env.admin)
    .await
    .unwrap();
    assert!(plus_ancienne);
}
