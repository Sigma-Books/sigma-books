//! Brouillard persistant et journalisé (PR 13a, L2-25) : création, modification (version),
//! suppression, drapeau « à revoir », pièces, contributeurs, journal d'audit chaîné, clôture des
//! périodes, règle de la fiche du tiers, refus dès la saisie, concurrence.
//!
//! Isolation de chaque route et « Cedar se trompe » : `tests/isolation/brouillard.rs`. Fil-piège
//! du catalogue : `tests/catalogue.rs`. La part « validation dans une période clôturée refusée »
//! du test exigé par la ligne 13 de `docs/lots/L2.md` relève de la **validation** (PR 13b) : la
//! création et le changement de date d'un brouillon dans une période clôturée sont refusés ici.
#![cfg(feature = "tests-postgres")]
// Fichier de test : les `expect`/`unwrap` y sont voulus.
#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::assert_is_empty,
    clippy::many_single_char_names,
    clippy::too_many_lines
)]

mod commun;

use std::collections::BTreeSet;

use api::base::referentiel::{Modele, TypeTiers, types_de_tiers_admis};
use api::serveur::{Reglages, routeur};
use axum::Router;
use axum::http::StatusCode;
use commun::brouillard::{journal_de_saisie, ligne, saisie, texte_date};
use commun::referentiel::{comptes, doter};
use commun::serveur::{ORIGINE, Reponse, application_base, envoyer, lire};
use commun::{CabinetTest, Env, ajouter_affecte, creer_cabinet, env};
use serde_json::Value;
use uuid::Uuid;

struct Banc {
    env: Env,
    c: CabinetTest,
    routeur: Router,
    valideur: api::base::IdUtilisateur,
    auditeur: api::base::IdUtilisateur,
    periodes: Vec<Uuid>,
}

async fn banc() -> Banc {
    let env = env().await;
    let c = creer_cabinet(&env.admin, "Cabinet").await;
    doter(&env.application, c.cabinet, c.dossier, c.administrateur).await;
    let valideur = ajouter_affecte(&env.admin, &c, "valideur").await;
    let auditeur = ajouter_affecte(&env.admin, &c, "auditeur").await;
    let periodes =
        sqlx::query_scalar("SELECT id FROM periode WHERE dossier_id = $1 ORDER BY debut")
            .bind(c.dossier.uuid())
            .fetch_all(&env.admin)
            .await
            .unwrap();
    let routeur = routeur(&application_base(&env, Reglages::new(ORIGINE.to_owned())));
    Banc {
        env,
        c,
        routeur,
        valideur,
        auditeur,
        periodes,
    }
}

impl Banc {
    fn base(&self) -> String {
        format!(
            "/api/cabinets/{}/dossiers/{}/brouillard",
            self.c.cabinet, self.c.dossier
        )
    }

    async fn poster(&self, chemin: &str, u: api::base::IdUtilisateur, corps: &str) -> Reponse {
        envoyer(&self.routeur, chemin, Some(u), corps, None).await
    }

    async fn creer(&self, u: api::base::IdUtilisateur, corps: &str) -> Reponse {
        self.poster(&self.base(), u, corps).await
    }

    /// Crée un brouillon valide au nom du collaborateur ; rend son identifiant.
    async fn nouveau(&self, libelle: &str) -> Uuid {
        let r = self
            .creer(self.c.collaborateur, &saisie(libelle, None))
            .await;
        assert_eq!(r.statut, StatusCode::CREATED, "{}", r.texte());
        r.json()["id"].as_str().unwrap().parse().unwrap()
    }

    fn chemin(&self, id: Uuid, suite: &str) -> String {
        format!("{}/{id}{suite}", self.base())
    }

    async fn lire(&self, id: Uuid) -> Reponse {
        lire(
            &self.routeur,
            &self.chemin(id, ""),
            Some(self.c.collaborateur),
        )
        .await
    }

    /// Liste filtrée, lue par l'auditeur.
    async fn liste(&self, q: String) -> Reponse {
        lire(
            &self.routeur,
            &format!("{}?{q}", self.base()),
            Some(self.auditeur),
        )
        .await
    }

    async fn compter(&self, table: &str) -> i64 {
        sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
            "SELECT count(*) FROM {table} WHERE dossier_id = $1"
        )))
        .bind(self.c.dossier.uuid())
        .fetch_one(&self.env.admin)
        .await
        .unwrap()
    }

    async fn entrees_audit(&self) -> Vec<String> {
        sqlx::query_scalar("SELECT contenu FROM audit_dossier WHERE dossier_id = $1 ORDER BY rang")
            .bind(self.c.dossier.uuid())
            .fetch_all(&self.env.admin)
            .await
            .unwrap()
    }

    async fn chaine_integre(&self) -> bool {
        let r = lire(
            &self.routeur,
            &format!(
                "/api/cabinets/{}/dossiers/{}/audit/verification",
                self.c.cabinet, self.c.dossier
            ),
            Some(self.c.administrateur),
        )
        .await;
        assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
        r.json()["integre"] == true
    }
}

fn code(r: &Reponse) -> String {
    r.json()["erreur"].as_str().unwrap_or_default().to_owned()
}

fn attendre(r: &Reponse, statut: StatusCode, erreur: &str) {
    assert_eq!(r.statut, statut, "{}", r.texte());
    assert_eq!(code(r), erreur, "{}", r.texte());
}

/// Corps de saisie sur mesure : journal, date, libellé et lignes déjà écrites en JSON.
fn corps(journal: &str, date: &str, lignes: &[String], reste: &str) -> String {
    format!(
        r#"{{"journal":"{journal}","date":"{date}","libelle":"Test"{reste},"lignes":[{}]}}"#,
        lignes.join(",")
    )
}

fn aujourd_hui() -> String {
    texte_date(commun::brouillard::aujourd_hui())
}

// --- Création, lecture, liste ----------------------------------------------------------------

#[tokio::test]
async fn creation_lecture_et_liste() {
    let b = banc().await;
    let r = b
        .creer(b.c.collaborateur, &saisie("Achat de fournitures", None))
        .await;
    assert_eq!(r.statut, StatusCode::CREATED, "{}", r.texte());
    let v = r.json();
    assert_eq!(v["version"], 1);
    assert_eq!(v["a_revoir"], false);
    assert_eq!(v["libelle"], "Achat de fournitures");
    assert_eq!(v["lignes"].as_array().unwrap().len(), 2);
    assert_eq!(v["lignes"][0]["montant"], "1000");
    assert_eq!(
        v["contributeurs"],
        serde_json::json!([b.c.collaborateur.to_string()])
    );
    let id: Uuid = v["id"].as_str().unwrap().parse().unwrap();
    assert_eq!(b.lire(id).await.json()["id"], id.to_string());

    // Liste : filtres sur le journal et la période ; l'auditeur lit.
    let autre = b.nouveau("Second").await;
    let l = lire(&b.routeur, &b.base(), Some(b.auditeur)).await;
    assert_eq!(l.statut, StatusCode::OK, "{}", l.texte());
    let ids: BTreeSet<String> = l.json()["brouillons"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["id"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(ids, [id, autre].map(|i| i.to_string()).into());
    assert_eq!(l.json()["brouillons"][0]["debit"], "1000");

    let par_journal = b.liste(format!("journal={}", journal_de_saisie())).await;
    assert_eq!(
        par_journal.json()["brouillons"].as_array().unwrap().len(),
        2
    );
    let autre_journal = b.liste("journal=ZZ9".to_owned()).await;
    assert!(
        autre_journal.json()["brouillons"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let periode_du_jour: Uuid = sqlx::query_scalar(
        "SELECT id FROM periode WHERE dossier_id = $1 AND debut <= current_date AND fin >= current_date",
    )
    .bind(b.c.dossier.uuid())
    .fetch_one(&b.env.admin)
    .await
    .unwrap();
    let par_periode = b.liste(format!("periode={periode_du_jour}")).await;
    assert_eq!(
        par_periode.json()["brouillons"].as_array().unwrap().len(),
        2
    );
    let autre_periode = b.liste(format!("periode={}", Uuid::new_v4())).await;
    assert!(
        autre_periode.json()["brouillons"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    // Pagination par curseur.
    let p1 = b.liste("limite=1".to_owned()).await.json();
    assert_eq!(p1["brouillons"].as_array().unwrap().len(), 1);
    let suivant = p1["suivant"].as_str().unwrap().to_owned();
    let p2 = b.liste(format!("limite=1&apres={suivant}")).await.json();
    assert_eq!(p2["brouillons"].as_array().unwrap().len(), 1);
    assert_ne!(p2["brouillons"][0]["id"], p1["brouillons"][0]["id"]);
    // Rôle qui ne saisit pas.
    let r = b.creer(b.auditeur, &saisie("Interdit", None)).await;
    assert_eq!(r.statut, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn un_brouillon_peut_etre_desequilibre_ou_vide() {
    let b = banc().await;
    let c = comptes();
    let desequilibre = corps(
        &journal_de_saisie(),
        &aujourd_hui(),
        &[ligne(c.caisse.as_str(), "debit", "1000")],
        "",
    );
    assert_eq!(
        b.creer(b.c.collaborateur, &desequilibre).await.statut,
        StatusCode::CREATED
    );
    let vide = corps(&journal_de_saisie(), &aujourd_hui(), &[], "");
    assert_eq!(
        b.creer(b.c.collaborateur, &vide).await.statut,
        StatusCode::CREATED
    );
}

// --- Refus dès la saisie -------------------------------------------------------------------------

#[tokio::test]
async fn chaque_refus_de_saisie() {
    let b = banc().await;
    let c = comptes();
    let j = journal_de_saisie();
    let d = aujourd_hui();
    let (caisse, banque) = (c.caisse.as_str(), c.banque.as_str());
    let a_nouveaux = Modele::officiel()
        .unwrap()
        .etat()
        .journaux
        .journaux()
        .find(|x| !x.genre().accepte_la_saisie())
        .unwrap()
        .code()
        .as_str()
        .to_owned();
    // Un compte désactivé et un tiers désactivé.
    let desactive = banque;
    sqlx::query("UPDATE compte SET actif = false WHERE dossier_id = $1 AND numero = $2")
        .bind(b.c.dossier.uuid())
        .bind(desactive)
        .execute(&b.env.admin)
        .await
        .unwrap();
    let tiers_inactif = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO tiers (cabinet_id, dossier_id, id, nom, type, compte_collectif, actif) \
         VALUES ($1, $2, $3, 'Inactif', 'client', $4, false)",
    )
    .bind(b.c.cabinet.uuid())
    .bind(b.c.dossier.uuid())
    .bind(tiers_inactif)
    .bind(c.collectif.as_str())
    .execute(&b.env.admin)
    .await
    .unwrap();
    let etiquette = |extra: &str| {
        format!(
            r#"{{"compte":"{caisse}","sens":"debit","montant":"1000","etiquette":{{"nature":"base","sens":"collectee","regime":"IMP18"{extra}}}}}"#
        )
    };
    let regroupement = c.base.as_str();
    let cas: Vec<(&str, String, StatusCode, &str)> = vec![
        (
            "montant négatif",
            corps(&j, &d, &[ligne(caisse, "debit", "-5")], ""),
            StatusCode::UNPROCESSABLE_ENTITY,
            "montant_invalide",
        ),
        (
            "montant nul",
            corps(&j, &d, &[ligne(caisse, "debit", "0")], ""),
            StatusCode::UNPROCESSABLE_ENTITY,
            "montant_invalide",
        ),
        (
            "compte inexistant",
            corps(&j, &d, &[ligne("9999-ZZ", "debit", "5")], ""),
            StatusCode::UNPROCESSABLE_ENTITY,
            "compte_inconnu",
        ),
        (
            "compte inactif",
            corps(&j, &d, &[ligne(desactive, "debit", "5")], ""),
            StatusCode::UNPROCESSABLE_ENTITY,
            "compte_inactif",
        ),
        (
            "compte de regroupement",
            corps(&j, &d, &[ligne(regroupement, "debit", "5")], ""),
            StatusCode::UNPROCESSABLE_ENTITY,
            "compte_de_regroupement",
        ),
        (
            "journal inconnu",
            corps("ZZ9", &d, &[ligne(caisse, "debit", "5")], ""),
            StatusCode::UNPROCESSABLE_ENTITY,
            "journal_inconnu",
        ),
        (
            "journal sans saisie",
            corps(&a_nouveaux, &d, &[ligne(caisse, "debit", "5")], ""),
            StatusCode::UNPROCESSABLE_ENTITY,
            "journal_sans_saisie",
        ),
        (
            "date hors exercice",
            corps(&j, "1999-01-01", &[ligne(caisse, "debit", "5")], ""),
            StatusCode::UNPROCESSABLE_ENTITY,
            "date_hors_exercice",
        ),
        (
            "date de valeur non antérieure",
            corps(
                &j,
                &d,
                &[ligne(caisse, "debit", "5")],
                &format!(r#","date_valeur_comptable":"{d}""#),
            ),
            StatusCode::UNPROCESSABLE_ENTITY,
            "date_valeur_non_anterieure",
        ),
        (
            "tiers inconnu",
            corps(
                &j,
                &d,
                &[format!(
                    r#"{{"compte":"{}","sens":"credit","montant":"5","tiers":"{}"}}"#,
                    c.collectif,
                    Uuid::new_v4()
                )],
                "",
            ),
            StatusCode::UNPROCESSABLE_ENTITY,
            "tiers_inconnu",
        ),
        (
            "tiers inactif",
            corps(
                &j,
                &d,
                &[format!(
                    r#"{{"compte":"{}","sens":"credit","montant":"5","tiers":"{tiers_inactif}"}}"#,
                    c.collectif
                )],
                "",
            ),
            StatusCode::UNPROCESSABLE_ENTITY,
            "tiers_inactif",
        ),
        (
            "étiquette : régime inconnu",
            corps(&j, &d, &[etiquette("").replace("IMP18", "XXX999")], ""),
            StatusCode::UNPROCESSABLE_ENTITY,
            "etiquette_regime_inconnu",
        ),
        (
            "étiquette : taux manquant",
            corps(&j, &d, &[etiquette(r#","exigibilite":"debit""#)], ""),
            StatusCode::UNPROCESSABLE_ENTITY,
            "etiquette_taux_manquant",
        ),
        (
            "étiquette : taux non admis",
            corps(
                &j,
                &d,
                &[etiquette(r#","taux":"7","exigibilite":"debit""#)],
                "",
            ),
            StatusCode::UNPROCESSABLE_ENTITY,
            "etiquette_taux_non_admis",
        ),
        (
            "étiquette : motif interdit",
            corps(
                &j,
                &d,
                &[etiquette(
                    r#","taux":"18","exigibilite":"debit","motif":"IMP18-1""#,
                )],
                "",
            ),
            StatusCode::UNPROCESSABLE_ENTITY,
            "etiquette_motif_interdit",
        ),
        (
            "étiquette : exigibilité manquante",
            corps(&j, &d, &[etiquette(r#","taux":"18""#)], ""),
            StatusCode::UNPROCESSABLE_ENTITY,
            "etiquette_exigibilite_manquante",
        ),
        (
            "sens inconnu",
            corps(&j, &d, &[ligne(caisse, "autre", "5")], ""),
            StatusCode::BAD_REQUEST,
            "requete_invalide",
        ),
        (
            "champ inconnu",
            corps(&j, &d, &[ligne(caisse, "debit", "5")], r#","surprise":1"#),
            StatusCode::BAD_REQUEST,
            "requete_invalide",
        ),
        (
            "date mal formée",
            corps(&j, "19/11/2026", &[ligne(caisse, "debit", "5")], ""),
            StatusCode::BAD_REQUEST,
            "requete_invalide",
        ),
        (
            "version à la création",
            corps(&j, &d, &[ligne(caisse, "debit", "5")], r#","version":1"#),
            StatusCode::BAD_REQUEST,
            "requete_invalide",
        ),
        (
            "trop de lignes",
            corps(&j, &d, &vec![ligne(caisse, "debit", "1"); 201], ""),
            StatusCode::UNPROCESSABLE_ENTITY,
            "trop_de_lignes",
        ),
    ];
    for (nom, corps, statut, erreur) in cas {
        let r = b.creer(b.c.collaborateur, &corps).await;
        assert_eq!(r.statut, statut, "{nom} : {}", r.texte());
        assert_eq!(code(&r), erreur, "{nom}");
    }
    // Le motif de l'étiquette reste valide avec un régime et un taux admis ; la position de la
    // ligne fautive est rendue.
    let r = b
        .creer(
            b.c.collaborateur,
            &corps(
                &j,
                &d,
                &[ligne(caisse, "debit", "5"), ligne(banque, "credit", "-1")],
                "",
            ),
        )
        .await;
    assert_eq!(r.json()["ligne"], 2);
    let bonne = corps(
        &j,
        &d,
        &[etiquette(r#","taux":"18","exigibilite":"debit""#)],
        "",
    );
    let r = b.creer(b.c.collaborateur, &bonne).await;
    assert_eq!(r.statut, StatusCode::CREATED, "{}", r.texte());
    assert_eq!(r.json()["lignes"][0]["etiquette"]["regime"], "IMP18");
    assert_eq!(r.json()["lignes"][0]["etiquette"]["taux"], "18");
    // Seul le brouillon valide existe : rien n'a été écrit par les refus.
    assert_eq!(b.compter("brouillon").await, 1);
    assert_eq!(b.compter("brouillon_ligne").await, 1);
}

#[tokio::test]
async fn regle_de_la_fiche_du_tiers() {
    let b = banc().await;
    let etat = Modele::officiel().unwrap().etat();
    let plan = &etat.plan;
    // Comptes de mouvement du référentiel par fiche : fournisseur (fiche 40), client (fiche 41).
    let de = |genre: TypeTiers| -> Vec<String> {
        plan.comptes()
            .filter(|c| plan.compte_de_mouvement_actif(c.numero()).is_ok())
            .filter(|c| {
                types_de_tiers_admis(etat, c.numero())
                    .is_some_and(|t| t.len() == 1 && t.contains(&genre))
            })
            .map(|c| c.numero().as_str().to_owned())
            .collect()
    };
    // Trois comptes de la fiche fournisseur, de trois sous-groupes distincts (4011, 4091, 4081 du
    // plan officiel, trouvés par leurs caractères).
    let fournisseurs = de(TypeTiers::Fournisseur);
    let mut vus = BTreeSet::new();
    let trois: Vec<&String> = fournisseurs
        .iter()
        .filter(|n| vus.insert(n[..3].to_owned()))
        .take(3)
        .collect();
    assert_eq!(trois.len(), 3, "fiche 40 : {fournisseurs:?}");
    let clients = de(TypeTiers::Client);
    assert!(!clients.is_empty());
    let fournisseur = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO tiers (cabinet_id, dossier_id, id, nom, type, compte_collectif, actif) \
         VALUES ($1, $2, $3, 'Fournisseur', 'fournisseur', $4, true)",
    )
    .bind(b.c.cabinet.uuid())
    .bind(b.c.dossier.uuid())
    .bind(fournisseur)
    .bind(trois[0])
    .execute(&b.env.admin)
    .await
    .unwrap();
    let client: Uuid =
        sqlx::query_scalar("SELECT id FROM tiers WHERE dossier_id = $1 AND type = 'client'")
            .bind(b.c.dossier.uuid())
            .fetch_one(&b.env.admin)
            .await
            .unwrap();
    let (j, d) = (journal_de_saisie(), aujourd_hui());
    let avec = |compte: &str, tiers: Uuid| {
        corps(
            &j,
            &d,
            &[format!(
                r#"{{"compte":"{compte}","sens":"credit","montant":"100","tiers":"{tiers}"}}"#
            )],
            "",
        )
    };
    for compte in &trois {
        let r = b.creer(b.c.collaborateur, &avec(compte, fournisseur)).await;
        assert_eq!(
            r.statut,
            StatusCode::CREATED,
            "fournisseur sur {compte} : {}",
            r.texte()
        );
    }
    // Client sur un compte de sa fiche.
    let r = b.creer(b.c.collaborateur, &avec(&clients[0], client)).await;
    assert_eq!(r.statut, StatusCode::CREATED, "{}", r.texte());
    // Fournisseur sur un compte client, sur un compte sans fiche ; client sur un compte fournisseur.
    let sans_fiche = comptes().caisse.as_str().to_owned();
    for (compte, tiers) in [
        (clients[0].clone(), fournisseur),
        (sans_fiche.clone(), fournisseur),
        (sans_fiche, client),
        (trois[0].clone(), client),
    ] {
        let r = b.creer(b.c.collaborateur, &avec(&compte, tiers)).await;
        attendre(
            &r,
            StatusCode::UNPROCESSABLE_ENTITY,
            "compte_hors_fiche_du_tiers",
        );
        assert_eq!(r.json()["ligne"], 1);
    }
}

// --- Modification, contributeurs, audit ----------------------------------------------------------------

#[tokio::test]
async fn modification_version_contributeurs_et_audit() {
    let b = banc().await;
    let id = b.nouveau("Premier libellé").await;
    let avant = b.entrees_audit().await;
    assert_eq!(avant.len(), 1 + 2, "en-tête + deux lignes : {avant:?}");
    assert!(avant[0].contains("creation") && avant[0].contains("Premier libellé"));

    // Un second utilisateur corrige : le libellé et une ligne changent.
    let corrige = saisie("Libellé corrigé", Some(1)).replace("1000", "1500");
    let r = b
        .poster(&b.chemin(id, "/modification"), b.valideur, &corrige)
        .await;
    assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
    assert_eq!(r.json()["version"], 2);
    assert_eq!(r.json()["lignes"][0]["montant"], "1500");
    let contributeurs: BTreeSet<String> = r.json()["contributeurs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|u| u.as_str().unwrap().to_owned())
        .collect();
    assert_eq!(
        contributeurs,
        [b.c.collaborateur, b.valideur]
            .map(|u| u.to_string())
            .into(),
        "le créateur et le correcteur sont enregistrés"
    );
    let table: i64 =
        sqlx::query_scalar("SELECT count(*) FROM brouillon_contributeur WHERE brouillon_id = $1")
            .bind(id)
            .fetch_one(&b.env.admin)
            .await
            .unwrap();
    assert_eq!(table, 2);

    // Journal : en-tête (libellé et version avant/après) et une entrée par ligne changée.
    let apres = b.entrees_audit().await;
    let nouvelles = &apres[avant.len()..];
    assert_eq!(nouvelles.len(), 1 + 2, "{nouvelles:?}");
    let en_tete = &nouvelles[0];
    assert!(en_tete.contains("modification"));
    assert!(en_tete.contains("Premier libellé") && en_tete.contains("Libellé corrigé"));
    assert!(
        nouvelles[1..]
            .iter()
            .any(|e| e.contains("1000") && e.contains("1500"))
    );
    assert!(b.chaine_integre().await);

    // Version périmée, puis modification sans changement.
    let perime = saisie("Autre", Some(1));
    attendre(
        &b.poster(&b.chemin(id, "/modification"), b.c.collaborateur, &perime)
            .await,
        StatusCode::CONFLICT,
        "brouillon_perime",
    );
    let identique = saisie("Libellé corrigé", Some(2)).replace("1000", "1500");
    attendre(
        &b.poster(
            &b.chemin(id, "/modification"),
            b.c.collaborateur,
            &identique,
        )
        .await,
        StatusCode::CONFLICT,
        "aucun_changement",
    );
    assert_eq!(
        b.entrees_audit().await.len(),
        apres.len(),
        "un refus n'écrit rien"
    );
    assert_eq!(b.lire(id).await.json()["version"], 2);
}

#[tokio::test]
async fn deux_modifications_concurrentes_de_la_meme_version() {
    let b = banc().await;
    let id = b.nouveau("Origine").await;
    let chemin = b.chemin(id, "/modification");
    let (corps_a, corps_b) = (saisie("Version A", Some(1)), saisie("Version B", Some(1)));
    let (a, c) = tokio::join!(
        b.poster(&chemin, b.c.collaborateur, &corps_a),
        b.poster(&chemin, b.valideur, &corps_b),
    );
    let mut statuts = [a.statut, c.statut];
    statuts.sort();
    assert_eq!(
        statuts,
        [StatusCode::OK, StatusCode::CONFLICT],
        "{} / {}",
        a.texte(),
        c.texte()
    );
    let perdante = if a.statut == StatusCode::CONFLICT {
        &a
    } else {
        &c
    };
    assert_eq!(code(perdante), "brouillon_perime");
    assert_eq!(b.lire(id).await.json()["version"], 2);
}

// --- Suppression ------------------------------------------------------------------------------

#[tokio::test]
async fn suppression_et_son_audit() {
    let b = banc().await;
    let id = b.nouveau("À supprimer").await;
    attendre(
        &b.poster(
            &b.chemin(id, "/suppression"),
            b.c.collaborateur,
            r#"{"version":9}"#,
        )
        .await,
        StatusCode::CONFLICT,
        "brouillon_perime",
    );
    let avant = b.entrees_audit().await.len();
    let r = b
        .poster(
            &b.chemin(id, "/suppression"),
            b.c.collaborateur,
            r#"{"version":1}"#,
        )
        .await;
    assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
    assert_eq!(b.lire(id).await.statut, StatusCode::NOT_FOUND);
    for table in [
        "brouillon",
        "brouillon_ligne",
        "brouillon_contributeur",
        "brouillon_piece",
    ] {
        assert_eq!(b.compter(table).await, 0, "{table}");
    }
    let nouvelles = &b.entrees_audit().await[avant..];
    assert_eq!(nouvelles.len(), 1 + 2);
    assert!(nouvelles[0].contains("suppression") && nouvelles[0].contains("À supprimer"));
    assert!(nouvelles[1..].iter().all(|e| e.contains("avant=")));
    assert!(b.chaine_integre().await);
    // Deuxième suppression : le brouillon n'existe plus.
    let r = b
        .poster(
            &b.chemin(id, "/suppression"),
            b.c.collaborateur,
            r#"{"version":1}"#,
        )
        .await;
    assert_eq!(r.statut, StatusCode::NOT_FOUND);
}

// --- Drapeau --------------------------------------------------------------------------------

#[tokio::test]
async fn drapeau_a_revoir() {
    let b = banc().await;
    let id = b.nouveau("À relire").await;
    let r = b.poster(&b.chemin(id, "/a-revoir"), b.valideur, "{}").await;
    assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
    assert_eq!(r.json()["a_revoir"], true);
    assert_eq!(r.json()["version"], 1, "le drapeau n'est pas un contenu");
    // Le relecteur n'est pas contributeur : il pourra valider.
    assert_eq!(
        r.json()["contributeurs"],
        serde_json::json!([b.c.collaborateur.to_string()])
    );
    attendre(
        &b.poster(&b.chemin(id, "/a-revoir"), b.valideur, "{}").await,
        StatusCode::CONFLICT,
        "drapeau_inchange",
    );
    // Le drapeau ne bloque pas la modification, et celle-ci le laisse en place.
    let m = b
        .poster(
            &b.chemin(id, "/modification"),
            b.c.collaborateur,
            &saisie("Relu", Some(1)),
        )
        .await;
    assert_eq!(m.json()["a_revoir"], true);
    let r = b
        .poster(&b.chemin(id, "/revue-faite"), b.valideur, "")
        .await;
    assert_eq!(r.json()["a_revoir"], false);
    attendre(
        &b.poster(&b.chemin(id, "/revue-faite"), b.valideur, "{}")
            .await,
        StatusCode::CONFLICT,
        "drapeau_inchange",
    );
    let audit = b.entrees_audit().await;
    assert!(
        audit
            .iter()
            .any(|e| e.contains("a_revoir") && e.contains("apres=oui"))
    );
    assert!(
        audit
            .iter()
            .any(|e| e.contains("revue_faite") && e.contains("apres=non"))
    );
    assert!(b.chaine_integre().await);
}

// --- Pièces ---------------------------------------------------------------------------------------

#[tokio::test]
async fn pieces_liens_et_ecart() {
    let b = banc().await;
    let id = b.nouveau("Justifiée").await;
    let nouvelle_piece =
        || commun::pieces::doter(&b.env.admin, b.c.cabinet, b.c.dossier, b.c.administrateur);
    let (p1, p2, p3) = (
        nouvelle_piece().await,
        nouvelle_piece().await,
        nouvelle_piece().await,
    );
    let lien = |piece: Uuid, role: &str, version: i32| {
        format!(r#"{{"piece":"{piece}","role":"{role}","version":{version}}}"#)
    };
    let chemin = b.chemin(id, "/pieces");
    let r = b
        .poster(&chemin, b.c.collaborateur, &lien(p1, "principale", 1))
        .await;
    assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
    assert_eq!(r.json()["version"], 2);
    assert_eq!(r.json()["pieces"][0]["role"], "principale");
    // Même pièce, deuxième principale, version périmée.
    attendre(
        &b.poster(&chemin, b.c.collaborateur, &lien(p1, "complementaire", 2))
            .await,
        StatusCode::CONFLICT,
        "piece_deja_liee",
    );
    attendre(
        &b.poster(&chemin, b.c.collaborateur, &lien(p2, "principale", 2))
            .await,
        StatusCode::CONFLICT,
        "piece_principale_existante",
    );
    attendre(
        &b.poster(&chemin, b.c.collaborateur, &lien(p2, "complementaire", 1))
            .await,
        StatusCode::CONFLICT,
        "brouillon_perime",
    );
    let r = b
        .poster(&chemin, b.valideur, &lien(p2, "complementaire", 2))
        .await;
    assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
    assert_eq!(r.json()["pieces"].as_array().unwrap().len(), 2);
    // Un lien est un contenu : le valideur est contributeur.
    assert_eq!(r.json()["contributeurs"].as_array().unwrap().len(), 2);
    // Pièce inconnue, ou d'un autre dossier.
    let ailleurs = commun::pieces::doter(
        &b.env.admin,
        b.c.cabinet,
        b.c.autre_dossier,
        b.c.administrateur,
    )
    .await;
    for piece in [Uuid::new_v4(), ailleurs] {
        let r = b
            .poster(
                &chemin,
                b.c.collaborateur,
                &lien(piece, "complementaire", 3),
            )
            .await;
        assert_eq!(r.statut, StatusCode::NOT_FOUND, "{}", r.texte());
    }
    // Pièce écartée : elle ne se lie pas.
    let ecart = |piece: Uuid| {
        format!(
            "/api/cabinets/{}/dossiers/{}/pieces/{piece}/ecart",
            b.c.cabinet, b.c.dossier
        )
    };
    let r = b
        .poster(
            &ecart(p3),
            b.c.administrateur,
            r#"{"motif":"Dépôt erroné"}"#,
        )
        .await;
    assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
    attendre(
        &b.poster(&chemin, b.c.collaborateur, &lien(p3, "complementaire", 3))
            .await,
        StatusCode::CONFLICT,
        "piece_ecartee",
    );
    // Pièce liée : elle ne s'écarte pas ; liée à un brouillon, elle l'est tant que le lien dure.
    attendre(
        &b.poster(&ecart(p1), b.c.administrateur, r#"{"motif":"Erreur"}"#)
            .await,
        StatusCode::CONFLICT,
        "piece_liee",
    );
    // Retrait (version périmée, puis courante, puis déjà retirée).
    let retrait = b.chemin(id, &format!("/pieces/{p1}/retrait"));
    attendre(
        &b.poster(&retrait, b.c.collaborateur, r#"{"version":1}"#)
            .await,
        StatusCode::CONFLICT,
        "brouillon_perime",
    );
    let r = b
        .poster(&retrait, b.c.collaborateur, r#"{"version":3}"#)
        .await;
    assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
    assert_eq!(r.json()["pieces"].as_array().unwrap().len(), 1);
    let r = b
        .poster(&retrait, b.c.collaborateur, r#"{"version":4}"#)
        .await;
    assert_eq!(r.statut, StatusCode::NOT_FOUND);
    // Libérée, la pièce peut s'écarter.
    let r = b
        .poster(&ecart(p1), b.c.administrateur, r#"{"motif":"Erreur"}"#)
        .await;
    assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
    // La suppression du brouillon libère les liens restants (p2 s'écarte ensuite).
    let r = b
        .poster(
            &b.chemin(id, "/suppression"),
            b.c.collaborateur,
            r#"{"version":4}"#,
        )
        .await;
    assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
    let r = b
        .poster(&ecart(p2), b.c.administrateur, r#"{"motif":"Erreur"}"#)
        .await;
    assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
    let audit = b.entrees_audit().await;
    assert!(
        audit
            .iter()
            .any(|e| e.contains("lien_piece") && e.contains(&p1.to_string()))
    );
    assert!(audit.iter().any(|e| e.contains("retrait_piece")));
    assert!(b.chaine_integre().await);
}

#[tokio::test]
async fn la_piece_interne_se_depose_puis_se_lie() {
    let b = banc().await;
    let id = b.nouveau("Opération diverse").await;
    let r = b
        .poster(
            &format!(
                "/api/cabinets/{}/dossiers/{}/pieces/internes",
                b.c.cabinet, b.c.dossier
            ),
            b.c.collaborateur,
            r#"{"date":"2026-11-19","motif":"Régularisation"}"#,
        )
        .await;
    assert_eq!(r.statut, StatusCode::CREATED, "{}", r.texte());
    let piece = r.json()["id"].as_str().unwrap().to_owned();
    let r = b
        .poster(
            &b.chemin(id, "/pieces"),
            b.c.collaborateur,
            &format!(r#"{{"piece":"{piece}","role":"principale","version":1}}"#),
        )
        .await;
    assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
}

// --- Clôture des périodes --------------------------------------------------------------------------

#[tokio::test]
async fn cloture_refusee_avec_un_brouillon_dans_la_periode_et_creation_refusee_apres() {
    let b = banc().await;
    let (c, j) = (comptes(), journal_de_saisie());
    let p0 = b.periodes[0];
    let debut: time::Date = sqlx::query_scalar("SELECT debut FROM periode WHERE id = $1")
        .bind(p0)
        .fetch_one(&b.env.admin)
        .await
        .unwrap();
    let date0 = texte_date(debut);
    let lignes = [
        ligne(c.caisse.as_str(), "debit", "10"),
        ligne(c.banque.as_str(), "credit", "10"),
    ];
    let r = b
        .creer(b.c.collaborateur, &corps(&j, &date0, &lignes, ""))
        .await;
    assert_eq!(r.statut, StatusCode::CREATED, "{}", r.texte());
    let id: Uuid = r.json()["id"].as_str().unwrap().parse().unwrap();
    let cloture = format!(
        "/api/cabinets/{}/dossiers/{}/periodes/{p0}/cloture",
        b.c.cabinet, b.c.dossier
    );
    let r = b
        .poster(
            &cloture,
            b.c.administrateur,
            r#"{"confirmation":"irreversible"}"#,
        )
        .await;
    assert_eq!(r.statut, StatusCode::CONFLICT, "{}", r.texte());
    assert_eq!(code(&r), "brouillard_non_valide");
    assert_eq!(r.json()["nombre"], 1);
    // Verrouillée, la période admet encore une saisie (le déverrouillage n'est pas exigé).
    let verrou = format!(
        "/api/cabinets/{}/dossiers/{}/periodes/{p0}/verrouillage",
        b.c.cabinet, b.c.dossier
    );
    assert_eq!(
        b.poster(&verrou, b.c.administrateur, "{}").await.statut,
        StatusCode::OK
    );
    assert_eq!(
        b.creer(b.c.collaborateur, &corps(&j, &date0, &lignes, ""))
            .await
            .statut,
        StatusCode::CREATED
    );
    // Sans brouillon dans la période, la clôture passe (supprimer les deux).
    let r = b
        .poster(
            &b.chemin(id, "/suppression"),
            b.c.collaborateur,
            r#"{"version":1}"#,
        )
        .await;
    assert_eq!(r.statut, StatusCode::OK);
    let restant: Uuid = sqlx::query_scalar("SELECT id FROM brouillon WHERE dossier_id = $1")
        .bind(b.c.dossier.uuid())
        .fetch_one(&b.env.admin)
        .await
        .unwrap();
    b.poster(
        &b.chemin(restant, "/suppression"),
        b.c.collaborateur,
        r#"{"version":1}"#,
    )
    .await;
    let r = b
        .poster(
            &cloture,
            b.c.administrateur,
            r#"{"confirmation":"irreversible"}"#,
        )
        .await;
    assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
    // Création et changement de date dans la période clôturée : refusés.
    attendre(
        &b.creer(b.c.collaborateur, &corps(&j, &date0, &lignes, ""))
            .await,
        StatusCode::CONFLICT,
        "date_dans_periode_cloturee",
    );
    let ouvert = b.nouveau("Dans la période du jour").await;
    let vers_cloture = corps(&j, &date0, &lignes, r#","version":1"#);
    attendre(
        &b.poster(
            &b.chemin(ouvert, "/modification"),
            b.c.collaborateur,
            &vers_cloture,
        )
        .await,
        StatusCode::CONFLICT,
        "date_dans_periode_cloturee",
    );
    assert_eq!(b.lire(ouvert).await.json()["version"], 1);
}

// --- Séparation des tâches : ce que Cedar reçoit --------------------------------------------------------

#[tokio::test]
async fn les_contributeurs_lus_en_base_suffisent_a_la_separation_des_taches() {
    use api::base::{ContexteTenant, ouvrir_transaction};
    let b = banc().await;
    let id = b.nouveau("Séparée").await;
    b.poster(
        &b.chemin(id, "/modification"),
        b.valideur,
        &saisie("Corrigée", Some(1)),
    )
    .await;
    let mut tx = ouvrir_transaction(
        &b.env.application,
        ContexteTenant::dossier(b.c.cabinet, b.c.dossier, b.c.administrateur),
    )
    .await
    .unwrap();
    let (createur, autres) = api::base::brouillard::contributeurs(&mut tx, id)
        .await
        .unwrap()
        .unwrap();
    tx.annuler().await.unwrap();
    assert_eq!(createur, b.c.collaborateur.uuid());
    assert_eq!(autres, vec![b.valideur.uuid()]);
}

// --- Rôles --------------------------------------------------------------------------------------------

#[tokio::test]
async fn l_auditeur_lit_sans_rien_modifier() {
    let b = banc().await;
    let id = b.nouveau("Lisible").await;
    assert_eq!(
        lire(&b.routeur, &b.chemin(id, ""), Some(b.auditeur))
            .await
            .statut,
        StatusCode::OK
    );
    for (suite, corps) in [
        ("/modification", saisie("Non", Some(1))),
        ("/suppression", r#"{"version":1}"#.to_owned()),
        ("/a-revoir", "{}".to_owned()),
        ("/revue-faite", "{}".to_owned()),
    ] {
        let r = b.poster(&b.chemin(id, suite), b.auditeur, &corps).await;
        assert_eq!(r.statut, StatusCode::FORBIDDEN, "{suite} : {}", r.texte());
    }
    assert_eq!(b.lire(id).await.json()["version"], 1);
}

#[allow(dead_code)]
fn _valeur(_: Value) {}
