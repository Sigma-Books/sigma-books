//! Validation unitaire d'une écriture (PR 13b, L2-26) : numérotation sans trou, chaîne de
//! hachage, audit, pièces figées, séparation des tâches, un refus par motif, concurrence,
//! atomicité, rapprochement avec l'export d'`integrite` et `sigma-verifier`, ajout seul.
//!
//! Isolation de chaque route et « Cedar se trompe » : `tests/isolation/brouillard.rs`. Propriété
//! contre `Livre` : tests de `api::serveur::routes::brouillard::validation`.
#![cfg(feature = "tests-postgres")]
// Fichier de test : les `expect`/`unwrap` y sont voulus.
#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::cast_possible_truncation,
    clippy::format_collect,
    clippy::many_single_char_names,
    clippy::too_many_lines
)]

mod commun;

use std::sync::Arc;

use api::base::referentiel::{EtatReferentiel, Modele, muter};
use api::base::validation as base;
use api::base::{ContexteTenant, ouvrir_transaction};
use api::serveur::routes::brouillard::referentiel_tva;
use api::serveur::routes::brouillard::validation::reconstituer;
use api::serveur::{Application, Reglages, routeur};
use authz::Autorisation;
use axum::Router;
use axum::http::StatusCode;
use commun::brouillard::{aujourd_hui, journal_de_saisie, ligne, saisie, texte_date};
use commun::referentiel::{Fixtures, comptes, doter};
use commun::serveur::{
    AuthentificationTest, HorlogeTest, ORIGINE, Reponse, application_base, envoyer, lire,
};
use commun::{CabinetTest, Env, ajouter_affecte, creer_cabinet, env};
use integrite::{Empreinte, chainer, genese};
use serde_json::Value;
use uuid::Uuid;

struct Banc {
    env: Env,
    c: CabinetTest,
    f: Fixtures,
    routeur: Router,
    valideur: api::base::IdUtilisateur,
    auditeur: api::base::IdUtilisateur,
}

async fn banc_avec(horloge: Option<&HorlogeTest>) -> Banc {
    let env = env().await;
    let c = creer_cabinet(&env.admin, "Cabinet").await;
    let f = doter(&env.application, c.cabinet, c.dossier, c.administrateur).await;
    let valideur = ajouter_affecte(&env.admin, &c, "valideur").await;
    let auditeur = ajouter_affecte(&env.admin, &c, "auditeur").await;
    let etat = match horloge {
        None => application_base(&env, Reglages::new(ORIGINE.to_owned())),
        Some(h) => {
            let mut reglages = commun::serveur::reglages_de_test();
            reglages.facteurs.cle_instance = Some(commun::serveur::cle_de_test());
            let etat = Application::avec_horloge(
                env.application.clone(),
                env.lecture_audit.clone(),
                Arc::new(Autorisation::charger().unwrap()),
                Arc::new(AuthentificationTest::default()),
                reglages,
                Arc::new(h.clone()),
            );
            commun::serveur::brancher_pieces_memoire(&etat);
            etat
        }
    };
    Banc {
        routeur: routeur(&etat),
        env,
        c,
        f,
        valideur,
        auditeur,
    }
}

async fn banc() -> Banc {
    banc_avec(None).await
}

fn code(r: &Reponse) -> String {
    r.json()["erreur"].as_str().unwrap_or_default().to_owned()
}

/// Corps d'une saisie à la date et au journal donnés.
fn contenu(journal: &str, date: &str, lignes: &[String], extra: &str) -> String {
    format!(
        r#"{{"journal":"{journal}","date":"{date}","libelle":"Écriture de test"{extra},"lignes":[{}]}}"#,
        lignes.join(",")
    )
}

impl Banc {
    fn base(&self) -> String {
        format!(
            "/api/cabinets/{}/dossiers/{}/brouillard",
            self.c.cabinet, self.c.dossier
        )
    }

    fn ecritures(&self) -> String {
        format!(
            "/api/cabinets/{}/dossiers/{}/ecritures",
            self.c.cabinet, self.c.dossier
        )
    }

    async fn creer(&self, corps: &str) -> Uuid {
        let r = envoyer(
            &self.routeur,
            &self.base(),
            Some(self.c.collaborateur),
            corps,
            None,
        )
        .await;
        assert_eq!(r.statut, StatusCode::CREATED, "{}", r.texte());
        r.json()["id"].as_str().unwrap().parse().unwrap()
    }

    /// Lie une pièce neuve au brouillon (version 1 attendue) ; rend la version suivante.
    async fn lier(&self, id: Uuid, version: i32) -> (Uuid, i32) {
        let piece = commun::pieces::doter(
            &self.env.admin,
            self.c.cabinet,
            self.c.dossier,
            self.c.administrateur,
        )
        .await;
        let r = envoyer(
            &self.routeur,
            &format!("{}/{id}/pieces", self.base()),
            Some(self.c.collaborateur),
            &format!(r#"{{"piece":"{piece}","role":"principale","version":{version}}}"#),
            None,
        )
        .await;
        assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
        (piece, r.json()["version"].as_i64().unwrap() as i32)
    }

    /// Brouillon équilibré du jour, avec une pièce liée, prêt à valider : (id, version, pièce).
    async fn pret(&self, libelle: &str) -> (Uuid, i32, Uuid) {
        let id = self.creer(&saisie(libelle, None)).await;
        let (piece, version) = self.lier(id, 1).await;
        (id, version, piece)
    }

    async fn pret_avec(&self, corps: &str) -> (Uuid, i32) {
        let id = self.creer(corps).await;
        let (_, version) = self.lier(id, 1).await;
        (id, version)
    }

    async fn valider_par(&self, u: api::base::IdUtilisateur, id: Uuid, version: i32) -> Reponse {
        envoyer(
            &self.routeur,
            &format!("{}/{id}/validation", self.base()),
            Some(u),
            &format!(r#"{{"version":{version}}}"#),
            None,
        )
        .await
    }

    async fn valider(&self, id: Uuid, version: i32) -> Reponse {
        self.valider_par(self.valideur, id, version).await
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

    async fn sql(&self, requete: &str) {
        sqlx::query(sqlx::AssertSqlSafe(requete.to_owned()))
            .bind(self.c.dossier.uuid())
            .execute(&self.env.admin)
            .await
            .unwrap();
    }

    /// Écritures de la chaîne, dans l'ordre : (rang, numéro, texte, empreinte, précédente).
    async fn chaine(&self) -> Vec<(i64, String, String, Vec<u8>, Vec<u8>)> {
        sqlx::query_as(
            "SELECT rang, numero, contenu_canonique, empreinte, empreinte_precedente \
             FROM ecriture_validee WHERE dossier_id = $1 ORDER BY rang",
        )
        .bind(self.c.dossier.uuid())
        .fetch_all(&self.env.admin)
        .await
        .unwrap()
    }

    fn genese(&self) -> Empreinte {
        genese(&integrite::IdDossier::parse(&self.c.dossier.uuid().to_string()).unwrap())
    }

    /// La chaîne stockée se recalcule : chaque empreinte suit la précédente et le texte stocké.
    async fn chaine_valide(&self) -> bool {
        let mut precedente = self.genese();
        for (i, (rang, _, texte, empreinte, prec)) in self.chaine().await.iter().enumerate() {
            let attendue = chainer(&precedente, texte.as_bytes());
            if *rang != i64::try_from(i + 1).unwrap()
                || prec.as_slice() != precedente.octets()
                || empreinte.as_slice() != attendue.octets()
            {
                return false;
            }
            precedente = attendue;
        }
        let (rang, tete): (i64, Vec<u8>) =
            sqlx::query_as("SELECT rang, empreinte FROM chaine_tete WHERE dossier_id = $1")
                .bind(self.c.dossier.uuid())
                .fetch_one(&self.env.admin)
                .await
                .unwrap_or((0, self.genese().octets().to_vec()));
        rang == i64::try_from(self.chaine().await.len()).unwrap()
            && tete.as_slice() == precedente.octets()
    }

    async fn compteur(&self) -> Vec<i32> {
        sqlx::query_scalar(
            "SELECT dernier FROM compteur_ecriture WHERE dossier_id = $1 ORDER BY dernier",
        )
        .bind(self.c.dossier.uuid())
        .fetch_all(&self.env.admin)
        .await
        .unwrap()
    }

    async fn annee(&self) -> String {
        sqlx::query_scalar("SELECT code FROM exercice WHERE dossier_id = $1 ORDER BY debut LIMIT 1")
            .bind(self.c.dossier.uuid())
            .fetch_one(&self.env.admin)
            .await
            .unwrap()
    }
}

fn caisse() -> &'static str {
    comptes().caisse.as_str()
}
fn banque() -> &'static str {
    comptes().banque.as_str()
}
fn aujourd() -> String {
    texte_date(aujourd_hui())
}
fn equilibre(montant: &str) -> Vec<String> {
    vec![
        ligne(caisse(), "debit", montant),
        ligne(banque(), "credit", montant),
    ]
}

// --- Nominal -------------------------------------------------------------------------------

#[tokio::test]
async fn validation_nominale() {
    let b = banc().await;
    let (id, version, piece) = b.pret("Première").await;
    let r = b.valider(id, version).await;
    assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
    let j = r.json();
    let numero = format!("{}-{}-000001", journal_de_saisie(), b.annee().await);
    assert_eq!(j["numero"], numero);
    assert_eq!(j["rang"], 1);
    assert_eq!(j["format"], 1);
    assert_eq!(j["id"], id.to_string());
    // Le brouillon et ses dépendances ont disparu ; l'écriture, ses lignes et ses liens existent.
    for t in [
        "brouillon",
        "brouillon_ligne",
        "brouillon_piece",
        "brouillon_contributeur",
    ] {
        assert_eq!(b.compter(t).await, 0, "{t}");
    }
    assert_eq!(b.compter("ecriture_validee").await, 1);
    assert_eq!(b.compter("ecriture_validee_ligne").await, 2);
    assert_eq!(b.compter("ecriture_validee_piece").await, 1);
    // Tête et compteur.
    let chaine = b.chaine().await;
    assert_eq!(chaine[0].1, numero);
    assert_eq!(
        j["empreinte"],
        Empreinte::parse(&hex(&chaine[0].3)).unwrap().to_string()
    );
    assert_eq!(b.compteur().await, vec![1]);
    assert!(b.chaine_valide().await);
    // Audit : qui, quoi, numéro, rang, empreinte ; ni libellé ni montant (ADR 0014).
    let entrees: Vec<String> =
        sqlx::query_scalar("SELECT contenu FROM audit_dossier WHERE dossier_id = $1 ORDER BY rang")
            .bind(b.c.dossier.uuid())
            .fetch_all(&b.env.admin)
            .await
            .unwrap();
    let derniere = entrees.last().unwrap();
    assert!(derniere.contains(&numero), "{derniere}");
    assert!(derniere.contains(j["empreinte"].as_str().unwrap()));
    assert!(derniere.contains("validation"));
    assert!(!derniere.contains("Première") && !derniere.contains("1000"));
    // Lecture.
    let r = lire(
        &b.routeur,
        &format!("{}/{id}", b.ecritures()),
        Some(b.auditeur),
    )
    .await;
    assert_eq!(r.statut, StatusCode::OK);
    assert_eq!(r.json()["numero"], numero);
    assert_eq!(r.json()["lignes"].as_array().unwrap().len(), 2);
    assert_eq!(r.json()["pieces"][0]["piece"], piece.to_string());
    let r = lire(&b.routeur, &b.ecritures(), Some(b.auditeur)).await;
    assert_eq!(r.json()["ecritures"][0]["numero"], numero);
    // Le brouillon n'existe plus : une seconde validation est une ressource absente.
    let r = b.valider(id, version).await;
    assert_eq!(r.statut, StatusCode::NOT_FOUND);
    // La pièce liée à une écriture validée ne s'écarte plus.
    let r = envoyer(
        &b.routeur,
        &format!(
            "/api/cabinets/{}/dossiers/{}/pieces/{piece}/ecart",
            b.c.cabinet, b.c.dossier
        ),
        Some(b.c.administrateur),
        r#"{"motif":"Doublon"}"#,
        None,
    )
    .await;
    assert_eq!(r.statut, StatusCode::CONFLICT, "{}", r.texte());
    assert_eq!(code(&r), "piece_liee");
}

fn hex(octets: &[u8]) -> String {
    octets.iter().map(|o| format!("{o:02x}")).collect()
}

#[tokio::test]
async fn numeros_continus_et_chaine() {
    let b = banc().await;
    let mut empreintes = Vec::new();
    for n in 1..=3 {
        let (id, v, _) = b.pret(&format!("Écriture {n}")).await;
        let r = b.valider(id, v).await;
        assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
        assert_eq!(r.json()["rang"], n);
        assert!(
            r.json()["numero"]
                .as_str()
                .unwrap()
                .ends_with(&format!("{n:06}"))
        );
        empreintes.push(r.json()["empreinte"].as_str().unwrap().to_owned());
    }
    let chaine = b.chaine().await;
    assert_eq!(
        chaine[1].4, chaine[0].3,
        "l'empreinte précédente est celle du rang 1"
    );
    assert!(b.chaine_valide().await);
    // Liste paginée, curseur strict (date, numéro).
    let r = lire(
        &b.routeur,
        &format!("{}?limite=2", b.ecritures()),
        Some(b.auditeur),
    )
    .await;
    assert_eq!(r.json()["ecritures"].as_array().unwrap().len(), 2);
    let suivant = r.json()["suivant"].as_str().unwrap().to_owned();
    let r = lire(
        &b.routeur,
        &format!("{}?apres={suivant}", b.ecritures()),
        Some(b.auditeur),
    )
    .await;
    assert_eq!(r.json()["ecritures"].as_array().unwrap().len(), 1);
    assert_eq!(r.json()["ecritures"][0]["rang"], 3);
    assert!(r.json()["suivant"].is_null());
}

#[tokio::test]
async fn decimales_acceptees_et_contre_valeur_arrondie() {
    let b = banc().await;
    // Décimales du XOF acceptées et conservées exactes.
    let (id, v) = b
        .pret_avec(&contenu(
            &journal_de_saisie(),
            &aujourd(),
            &equilibre("1000.5"),
            "",
        ))
        .await;
    let r = b.valider(id, v).await;
    assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
    let chaine = b.chaine().await;
    assert!(chaine[0].2.contains("montant=1000.5\n"), "{}", chaine[0].2);
    // Ligne en devise : contre-valeur arrondie au franc par le moteur (100 EUR à 655,957 :
    // 65 595,7 → 65 596) ; équilibrée par une ligne XOF explicite.
    let devise = format!(
        r#"{{"compte":"{}","sens":"debit","devise":{{"code":"EUR","montant":"100","cours":"655.957"}}}}"#,
        caisse()
    );
    let (id, v) = b
        .pret_avec(&contenu(
            &journal_de_saisie(),
            &aujourd(),
            &[devise.clone(), ligne(banque(), "credit", "65596")],
            "",
        ))
        .await;
    assert_eq!(b.valider(id, v).await.statut, StatusCode::OK);
    // Sans l'arrondi explicite, le serveur n'ajoute aucune ligne : déséquilibre.
    let (id, v) = b
        .pret_avec(&contenu(
            &journal_de_saisie(),
            &aujourd(),
            &[devise, ligne(banque(), "credit", "65595.7")],
            "",
        ))
        .await;
    let r = b.valider(id, v).await;
    assert_eq!(r.statut, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(code(&r), "ecriture_desequilibree");
    assert_eq!(b.compter("ecriture_validee_ligne").await, 4);
}

#[tokio::test]
async fn echelle_excessive_refusee() {
    let b = banc().await;
    let (id, v, _) = b.pret("Échelle").await;
    // Valeur hors de ce que la saisie admet (28 chiffres), posée par le propriétaire.
    b.sql(
        "UPDATE brouillon_ligne SET montant = 1000.1234567890123456789012345678901 \
         WHERE dossier_id = $1",
    )
    .await;
    let r = b.valider(id, v).await;
    assert_eq!(r.statut, StatusCode::UNPROCESSABLE_ENTITY, "{}", r.texte());
    assert_eq!(code(&r), "montant_echelle_excessive");
    assert_eq!(r.json()["ligne"], 1);
    assert_eq!(b.compter("ecriture_validee").await, 0);
    assert_eq!(b.compteur().await, Vec::<i32>::new());
}

#[tokio::test]
async fn etiquette_fiscale_en_format_v3() {
    let b = banc().await;
    let etiquetee = format!(
        r#"{{"compte":"{}","sens":"debit","montant":"1000","etiquette":{{"nature":"base","sens":"collectee","regime":"IMP18","taux":"18","exigibilite":"debit"}}}}"#,
        caisse()
    );
    let (id, v) = b
        .pret_avec(&contenu(
            &journal_de_saisie(),
            &aujourd(),
            &[etiquetee, ligne(banque(), "credit", "1000")],
            "",
        ))
        .await;
    let r = b.valider(id, v).await;
    assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
    assert_eq!(r.json()["format"], 3);
    let chaine = b.chaine().await;
    assert!(chaine[0].2.starts_with("ecriture v3\n"));
    assert!(chaine[0].2.contains("fiscal-regime=IMP18\n"));
    assert!(chaine[0].2.contains("fiscal-taux=18\n"));
    // Sans étiquette : format v1 (le v2 d'une écriture à date de valeur est dans la parité).
    let (id, v, _) = b.pret("Sans étiquette").await;
    let r = b.valider(id, v).await;
    assert_eq!(r.json()["format"], 1);
    assert!(b.chaine().await[1].2.starts_with("ecriture v1\n"));
    let r = lire(
        &b.routeur,
        &format!("{}/{id}", b.ecritures()),
        Some(b.auditeur),
    )
    .await;
    assert_eq!(r.json()["format"], 1);
}

// --- Refus -----------------------------------------------------------------------------------

async fn refuse(b: &Banc, id: Uuid, v: i32, statut: StatusCode, motif: &str) -> Reponse {
    let r = b.valider(id, v).await;
    assert_eq!(r.statut, statut, "{motif} : {}", r.texte());
    assert_eq!(code(&r), motif);
    // Atomicité : rien n'est consommé, rien n'est écrit, le brouillon reste.
    assert_eq!(b.compter("ecriture_validee").await, 0, "{motif}");
    assert_eq!(b.compteur().await.iter().sum::<i32>(), 0, "{motif}");
    assert_eq!(b.compter("brouillon").await, 1, "{motif}");
    r
}

/// Passe au statut donné la période du jour et, pour une clôture, celles qui la précèdent
/// (les périodes se clôturent dans l'ordre : le calendrier relu doit rester cohérent).
async fn periode_du_jour(b: &Banc, statut: &str) {
    let borne = if statut == "cloturee" {
        "debut <= $4"
    } else {
        "debut <= $4 AND fin >= $4"
    };
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "UPDATE periode SET statut = $2, statut_par = $3, statut_le = now() \
         WHERE dossier_id = $1 AND {borne}"
    )))
    .bind(b.c.dossier.uuid())
    .bind(statut)
    .bind(b.c.administrateur.uuid())
    .bind(aujourd_hui())
    .execute(&b.env.admin)
    .await
    .unwrap();
}

#[tokio::test]
async fn periode_verrouillee_refusee() {
    let b = banc().await;
    let (id, v, _) = b.pret("Verrouillée").await;
    periode_du_jour(&b, "verrouillee").await;
    refuse(&b, id, v, StatusCode::CONFLICT, "periode_verrouillee").await;
}

/// Test exigé par `docs/lots/L2.md` (ligne 13) : validation dans une période clôturée refusée.
/// La période clôturée reste **visible** sous `FOR SHARE` (les déclencheurs d'irréversibilité ne
/// jouent que sur `UPDATE`) : c'est `valider` qui la refuse, sans consommer de numéro.
#[tokio::test]
async fn validation_dans_une_periode_cloturee_refusee() {
    let b = banc().await;
    let (id, v, _) = b.pret("Clôturée").await;
    periode_du_jour(&b, "cloturee").await;
    refuse(&b, id, v, StatusCode::CONFLICT, "periode_cloturee").await;
    assert!(b.chaine_valide().await);
}

#[tokio::test]
async fn echeance_de_cloture_depassee() {
    let horloge = HorlogeTest::default();
    let b = banc_avec(Some(&horloge)).await;
    let (id, v, _) = b.pret("Tardive").await;
    // Deux ans plus tard : l'échéance de la clôture informatique de la période est dépassée.
    horloge.avancer(time::Duration::days(800));
    refuse(
        &b,
        id,
        v,
        StatusCode::CONFLICT,
        "echeance_de_cloture_depassee",
    )
    .await;
}

#[tokio::test]
async fn exercice_cloture_ou_en_cloture_refuse() {
    let b = banc().await;
    let (id, v, _) = b.pret("Exercice").await;
    b.sql("UPDATE exercice SET statut = 'en_cloture' WHERE dossier_id = $1")
        .await;
    refuse(&b, id, v, StatusCode::CONFLICT, "exercice_en_cloture").await;
    b.sql(&format!(
        "UPDATE periode SET statut = 'cloturee', statut_par = '{}', statut_le = now() \
         WHERE dossier_id = $1",
        b.c.administrateur.uuid()
    ))
    .await;
    b.sql("UPDATE exercice SET statut = 'cloture' WHERE dossier_id = $1")
        .await;
    refuse(&b, id, v, StatusCode::CONFLICT, "exercice_cloture").await;
}

/// Instant fixe : le 15 juin de l'exercice du dossier, à midi UTC.
fn midi(annee: &str, mois: time::Month, jour: u8) -> time::OffsetDateTime {
    time::Date::from_calendar_date(annee.parse().unwrap(), mois, jour)
        .unwrap()
        .with_hms(12, 0, 0)
        .unwrap()
        .assume_utc()
}

/// Opération tardive (AUDCIF art. 22 4°), **déterministe** : horloge fixée au 15 juin, janvier à
/// avril clôturés, date de valeur au 15 janvier. La période d'accueil est mai : l'écriture doit
/// être datée du 1er mai.
#[tokio::test]
async fn operation_tardive() {
    let horloge = HorlogeTest::default();
    let b = banc_avec(Some(&horloge)).await;
    let annee = b.annee().await;
    horloge.fixer(midi(&annee, time::Month::June, 15));
    let j = journal_de_saisie();
    let extra = format!(r#","date_valeur_comptable":"{annee}-01-15""#);
    let mal = b
        .pret_avec(&contenu(
            &j,
            &format!("{annee}-05-10"),
            &equilibre("1000"),
            &extra,
        ))
        .await;
    let bien = b
        .pret_avec(&contenu(
            &j,
            &format!("{annee}-05-01"),
            &equilibre("1000"),
            &extra,
        ))
        .await;
    sqlx::query(
        "UPDATE periode SET statut = 'cloturee', statut_par = $2, statut_le = now() \
         WHERE dossier_id = $1 AND debut < $3",
    )
    .bind(b.c.dossier.uuid())
    .bind(b.c.administrateur.uuid())
    .bind(time::Date::from_calendar_date(annee.parse().unwrap(), time::Month::May, 1).unwrap())
    .execute(&b.env.admin)
    .await
    .unwrap();
    let r = b.valider(mal.0, mal.1).await;
    assert_eq!(r.statut, StatusCode::CONFLICT, "{}", r.texte());
    assert_eq!(code(&r), "operation_tardive_mal_datee");
    assert_eq!(b.compter("ecriture_validee").await, 0);
    let r = b.valider(bien.0, bien.1).await;
    assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
    assert!(r.json()["numero"].as_str().unwrap().ends_with("-000001"));
    assert_eq!(
        r.json()["format"],
        2,
        "date de valeur sans étiquette : format v2"
    );
    assert_eq!(r.json()["date_validation"], format!("{annee}-06-15"));
}

/// Délégation retirée **pendant l'attente du verrou** du brouillon : le rôle est relu après le
/// verrou (ADR 0020, 5), la validation est refusée.
#[tokio::test]
async fn delegation_retiree_pendant_l_attente_du_verrou() {
    let b = banc().await;
    let (id, v, _) = b.pret("Délégation").await;
    let delegue = ajouter_affecte(&b.env.admin, &b.c, "collaborateur").await;
    sqlx::query("UPDATE acces_dossier SET validation_deleguee = true WHERE utilisateur_id = $1")
        .bind(delegue.uuid())
        .execute(&b.env.admin)
        .await
        .unwrap();
    // Le brouillon est tenu `FOR UPDATE` par une autre transaction.
    let mut verrou = b.env.admin.begin().await.unwrap();
    sqlx::query("SELECT id FROM brouillon WHERE id = $1 FOR UPDATE")
        .bind(id)
        .fetch_all(&mut *verrou)
        .await
        .unwrap();
    let routeur = b.routeur.clone();
    let chemin = format!("{}/{id}/validation", b.base());
    let tache = tokio::spawn(async move {
        envoyer(
            &routeur,
            &chemin,
            Some(delegue),
            &format!(r#"{{"version":{v}}}"#),
            None,
        )
        .await
    });
    tokio::time::sleep(std::time::Duration::from_millis(700)).await;
    assert!(!tache.is_finished(), "la validation attend le verrou");
    sqlx::query("UPDATE acces_dossier SET validation_deleguee = false WHERE utilisateur_id = $1")
        .bind(delegue.uuid())
        .execute(&b.env.admin)
        .await
        .unwrap();
    verrou.commit().await.unwrap();
    let r = tache.await.unwrap();
    assert_eq!(r.statut, StatusCode::FORBIDDEN, "{}", r.texte());
    assert_eq!(b.compter("ecriture_validee").await, 0);
    assert_eq!(b.compter("brouillon").await, 1);
}

/// L'instant de validation est relu après la tête de chaîne : il croît avec le rang.
#[tokio::test]
async fn les_instants_croissent_avec_le_rang() {
    let horloge = HorlogeTest::default();
    let b = banc_avec(Some(&horloge)).await;
    let annee = b.annee().await;
    horloge.fixer(midi(&annee, time::Month::June, 15));
    for n in 1..=3 {
        let (id, v, _) = b.pret(&format!("Instant {n}")).await;
        let r = b.valider(id, v).await;
        assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
        horloge.avancer(time::Duration::hours(13));
        assert_eq!(r.json()["rang"], n);
    }
    let lignes: Vec<(i64, time::OffsetDateTime, time::Date)> = sqlx::query_as(
        "SELECT rang, valide_le, date_validation FROM ecriture_validee \
         WHERE dossier_id = $1 ORDER BY rang",
    )
    .bind(b.c.dossier.uuid())
    .fetch_all(&b.env.admin)
    .await
    .unwrap();
    assert_eq!(lignes.len(), 3);
    for w in lignes.windows(2) {
        assert!(w[0].1 < w[1].1, "instants croissants avec le rang");
        assert!(w[0].2 <= w[1].2);
    }
    // Midi + 13 h : le deuxième instant franchit minuit, la date de validation le suit.
    assert_eq!(
        lignes[0].2,
        time::Date::from_calendar_date(annee.parse().unwrap(), time::Month::June, 15).unwrap()
    );
    assert_eq!(
        lignes[1].2,
        time::Date::from_calendar_date(annee.parse().unwrap(), time::Month::June, 16).unwrap()
    );
}

/// Un compte utilisé par la validation est verrouillé `FOR SHARE` : sa désactivation concurrente
/// attend la fin de la validation.
#[tokio::test]
async fn comptes_et_tiers_verrouilles_pendant_la_validation() {
    let b = banc().await;
    let (id, v, _) = b.pret("Verrou des comptes").await;
    // Une transaction crée la tête de chaîne sans la valider : la validation, après avoir
    // verrouillé ses comptes, attend sur `INSERT … ON CONFLICT`.
    let mut tenue = b.env.admin.begin().await.unwrap();
    sqlx::query(
        "INSERT INTO chaine_tete (cabinet_id, dossier_id, rang, empreinte) VALUES ($1, $2, 0, $3)",
    )
    .bind(b.c.cabinet.uuid())
    .bind(b.c.dossier.uuid())
    .bind(b.genese().octets().to_vec())
    .execute(&mut *tenue)
    .await
    .unwrap();
    let routeur = b.routeur.clone();
    let chemin = format!("{}/{id}/validation", b.base());
    let u = b.valideur;
    let validation = tokio::spawn(async move {
        envoyer(
            &routeur,
            &chemin,
            Some(u),
            &format!(r#"{{"version":{v}}}"#),
            None,
        )
        .await
    });
    tokio::time::sleep(std::time::Duration::from_millis(700)).await;
    assert!(!validation.is_finished());
    let admin = b.env.admin.clone();
    let dossier = b.c.dossier.uuid();
    let desactivation = tokio::spawn(async move {
        sqlx::query("UPDATE compte SET actif = false WHERE dossier_id = $1 AND numero = $2")
            .bind(dossier)
            .bind(caisse())
            .execute(&admin)
            .await
            .unwrap();
    });
    tokio::time::sleep(std::time::Duration::from_millis(700)).await;
    assert!(
        !desactivation.is_finished(),
        "la désactivation attend le verrou du compte"
    );
    tenue.commit().await.unwrap();
    let r = validation.await.unwrap();
    assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
    desactivation.await.unwrap();
    assert_eq!(b.compter("ecriture_validee").await, 1);
}

/// Seconde barrière : une pièce liée ne s'écarte pas, même par `UPDATE` direct du propriétaire.
#[tokio::test]
async fn piece_liee_non_ecartee_par_sql_direct() {
    let b = banc().await;
    let (id, v, piece) = b.pret("Pièce liée").await;
    let ecarter = |piece: Uuid| {
        let admin = b.env.admin.clone();
        let (cabinet, dossier, auteur) = (
            b.c.cabinet.uuid(),
            b.c.dossier.uuid(),
            b.c.administrateur.uuid(),
        );
        async move {
            sqlx::query(
                "UPDATE piece SET ecartee_le = now(), ecartee_par = $4, motif_ecart = 'Test' \
                 WHERE cabinet_id = $1 AND dossier_id = $2 AND id = $3",
            )
            .bind(cabinet)
            .bind(dossier)
            .bind(piece)
            .bind(auteur)
            .execute(&admin)
            .await
        }
    };
    // Liée à un brouillon.
    let e = ecarter(piece).await.unwrap_err();
    assert_eq!(commun::code_sql(&e).as_deref(), Some("23514"), "{e}");
    // Liée à une écriture validée.
    assert_eq!(b.valider(id, v).await.statut, StatusCode::OK);
    let e = ecarter(piece).await.unwrap_err();
    assert_eq!(commun::code_sql(&e).as_deref(), Some("23514"), "{e}");
    // Une pièce non liée s'écarte.
    let libre =
        commun::pieces::doter(&b.env.admin, b.c.cabinet, b.c.dossier, b.c.administrateur).await;
    ecarter(libre).await.unwrap();
}

#[tokio::test]
async fn ecritures_invalides_refusees() {
    let b = banc().await;
    let j = journal_de_saisie();
    let c = comptes();
    // Déséquilibre.
    let (id, v) = b
        .pret_avec(&contenu(
            &j,
            &aujourd(),
            &[
                ligne(caisse(), "debit", "1000"),
                ligne(banque(), "credit", "900"),
            ],
            "",
        ))
        .await;
    refuse(
        &b,
        id,
        v,
        StatusCode::UNPROCESSABLE_ENTITY,
        "ecriture_desequilibree",
    )
    .await;
    b.sql("DELETE FROM brouillon_ligne WHERE dossier_id = $1")
        .await;
    b.sql("DELETE FROM brouillon_piece WHERE dossier_id = $1")
        .await;
    b.sql("DELETE FROM brouillon_contributeur WHERE dossier_id = $1")
        .await;
    b.sql("DELETE FROM brouillon WHERE dossier_id = $1").await;
    // Une seule ligne.
    let (id, v) = b
        .pret_avec(&contenu(
            &j,
            &aujourd(),
            &[ligne(caisse(), "debit", "1000")],
            "",
        ))
        .await;
    refuse(
        &b,
        id,
        v,
        StatusCode::UNPROCESSABLE_ENTITY,
        "ecriture_une_seule_ligne",
    )
    .await;
    b.sql("DELETE FROM brouillon_ligne WHERE dossier_id = $1")
        .await;
    b.sql("DELETE FROM brouillon_piece WHERE dossier_id = $1")
        .await;
    b.sql("DELETE FROM brouillon_contributeur WHERE dossier_id = $1")
        .await;
    b.sql("DELETE FROM brouillon WHERE dossier_id = $1").await;
    // Tiers manquant sur un compte collectif (la ligne fautive est citée).
    let (id, v) = b
        .pret_avec(&contenu(
            &j,
            &aujourd(),
            &[
                ligne(banque(), "debit", "1000"),
                ligne(c.collectif.as_str(), "credit", "1000"),
            ],
            "",
        ))
        .await;
    let r = refuse(
        &b,
        id,
        v,
        StatusCode::UNPROCESSABLE_ENTITY,
        "tiers_manquant",
    )
    .await;
    assert_eq!(r.json()["ligne"], 2);
    // Compte désactivé après la saisie.
    b.sql(&format!(
        "UPDATE compte SET actif = false WHERE dossier_id = $1 AND numero = '{}'",
        c.collectif
    ))
    .await;
    let r = refuse(
        &b,
        id,
        v,
        StatusCode::UNPROCESSABLE_ENTITY,
        "validation_compte_inactif",
    )
    .await;
    assert_eq!(r.json()["ligne"], 2);
    b.sql(&format!(
        "UPDATE compte SET actif = true WHERE dossier_id = $1 AND numero = '{}'",
        c.collectif
    ))
    .await;
    // Compte devenu un compte de regroupement (il reçoit un sous-compte).
    let mut tx = ouvrir_transaction(
        &b.env.application,
        ContexteTenant::cabinet(b.c.cabinet, b.c.administrateur),
    )
    .await
    .unwrap();
    // Le contexte d'un dossier est requis par `muter`.
    drop(tx);
    tx = ouvrir_transaction(
        &b.env.application,
        ContexteTenant::dossier(b.c.cabinet, b.c.dossier, b.c.administrateur),
    )
    .await
    .unwrap();
    let numero = compta_core::NumeroCompte::parse(&format!("{}-SOUS", c.banque)).unwrap();
    muter(&mut tx, |_, etat: &mut EtatReferentiel| {
        etat.plan.ajouter_sous_compte(
            numero,
            compta_core::TexteCourt::parse("Sous-compte de test").unwrap(),
        )?;
        Ok(())
    })
    .await
    .unwrap();
    tx.confirmer().await.unwrap();
    let r = refuse(
        &b,
        id,
        v,
        StatusCode::UNPROCESSABLE_ENTITY,
        "validation_compte_de_regroupement",
    )
    .await;
    assert_eq!(r.json()["ligne"], 1);
}

#[tokio::test]
async fn justification_manquante() {
    let b = banc().await;
    let id = b.creer(&saisie("Sans pièce", None)).await;
    refuse(
        &b,
        id,
        1,
        StatusCode::UNPROCESSABLE_ENTITY,
        "justification_manquante",
    )
    .await;
}

#[tokio::test]
async fn journal_de_tresorerie_sans_son_compte() {
    let b = banc().await;
    let (journal, treso) = Modele::officiel()
        .unwrap()
        .etat()
        .journaux
        .journaux()
        .find_map(|j| {
            j.compte_tresorerie()
                .map(|c| (j.code().as_str().to_owned(), c.as_str().to_owned()))
        })
        .unwrap();
    let autre = if treso == caisse() {
        banque()
    } else {
        caisse()
    };
    let collectif = comptes().collectif.as_str();
    let lignes = vec![
        ligne(autre, "debit", "1000"),
        format!(
            r#"{{"compte":"{collectif}","sens":"credit","montant":"1000","tiers":"{}"}}"#,
            b.f.tiers
        ),
    ];
    let (id, v) = b
        .pret_avec(&contenu(&journal, &aujourd(), &lignes, ""))
        .await;
    refuse(
        &b,
        id,
        v,
        StatusCode::UNPROCESSABLE_ENTITY,
        "compte_tresorerie_absent",
    )
    .await;
}

#[tokio::test]
async fn brouillon_perime_et_session() {
    let b = banc().await;
    let (id, v, _) = b.pret("Périmé").await;
    refuse(&b, id, v - 1, StatusCode::CONFLICT, "brouillon_perime").await;
    let r = envoyer(
        &b.routeur,
        &format!("{}/{id}/validation", b.base()),
        None,
        &format!(r#"{{"version":{v}}}"#),
        None,
    )
    .await;
    assert_eq!(r.statut, StatusCode::UNAUTHORIZED);
    // Corps invalide.
    let r = envoyer(
        &b.routeur,
        &format!("{}/{id}/validation", b.base()),
        Some(b.valideur),
        r#"{"version":1,"autre":2}"#,
        None,
    )
    .await;
    assert_eq!(r.statut, StatusCode::BAD_REQUEST);
}

// --- Rôles, séparation des tâches, délégation ---------------------------------------------------

#[tokio::test]
async fn roles_separation_des_taches_et_delegation() {
    let b = banc().await;
    let (id, v, _) = b.pret("Séparation").await;
    // Auditeur : rôle non autorisé.
    assert_eq!(
        b.valider_par(b.auditeur, id, v).await.statut,
        StatusCode::FORBIDDEN
    );
    // Créateur (collaborateur), même délégué : séparation des tâches.
    b.sql("UPDATE acces_dossier SET validation_deleguee = true WHERE dossier_id = $1")
        .await;
    assert_eq!(
        b.valider_par(b.c.collaborateur, id, v).await.statut,
        StatusCode::FORBIDDEN
    );
    // L'administrateur qui a corrigé le brouillon en devient contributeur : refusé aussi.
    let r = envoyer(
        &b.routeur,
        &format!("{}/{id}/modification", b.base()),
        Some(b.c.administrateur),
        &format!(
            r#"{{"version":{v},"journal":"{}","date":"{}","libelle":"Corrigé","lignes":[{}]}}"#,
            journal_de_saisie(),
            aujourd(),
            equilibre("1000").join(",")
        ),
        None,
    )
    .await;
    assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
    let v = r.json()["version"].as_i64().unwrap() as i32;
    assert_eq!(
        b.valider_par(b.c.administrateur, id, v).await.statut,
        StatusCode::FORBIDDEN
    );
    assert_eq!(b.compter("ecriture_validee").await, 0);
    // Collaborateur sans délégation (autre que le créateur) : refusé ; avec : accepté.
    let autre = ajouter_affecte(&b.env.admin, &b.c, "collaborateur").await;
    b.sql("UPDATE acces_dossier SET validation_deleguee = false WHERE dossier_id = $1")
        .await;
    assert_eq!(
        b.valider_par(autre, id, v).await.statut,
        StatusCode::FORBIDDEN
    );
    sqlx::query("UPDATE acces_dossier SET validation_deleguee = true WHERE utilisateur_id = $1")
        .bind(autre.uuid())
        .execute(&b.env.admin)
        .await
        .unwrap();
    let r = b.valider_par(autre, id, v).await;
    assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
}

#[tokio::test]
async fn sans_separation_le_createur_valide() {
    let b = banc().await;
    sqlx::query("UPDATE cabinet SET separation_taches = false WHERE id = $1")
        .bind(b.c.cabinet.uuid())
        .execute(&b.env.admin)
        .await
        .unwrap();
    let (id, v, _) = b.pret("Sans séparation").await;
    let r = b.valider_par(b.c.administrateur, id, v).await;
    assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
    sqlx::query("UPDATE cabinet SET separation_taches = true WHERE id = $1")
        .bind(b.c.cabinet.uuid())
        .execute(&b.env.admin)
        .await
        .unwrap();
}

// --- Concurrence et atomicité --------------------------------------------------------------------

#[tokio::test]
async fn validations_simultanees_sans_trou() {
    let b = banc().await;
    let mut prets = Vec::new();
    for n in 0..8 {
        prets.push(b.pret(&format!("Concurrente {n}")).await);
    }
    let mut taches = Vec::new();
    for (id, v, _) in prets {
        let routeur = b.routeur.clone();
        let chemin = format!("{}/{id}/validation", b.base());
        let u = b.valideur;
        taches.push(tokio::spawn(async move {
            envoyer(
                &routeur,
                &chemin,
                Some(u),
                &format!(r#"{{"version":{v}}}"#),
                None,
            )
            .await
        }));
    }
    let mut sequences = Vec::new();
    for t in taches {
        let r = t.await.unwrap();
        assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
        sequences.push(r.json()["numero"].as_str().unwrap().to_owned());
    }
    sequences.sort();
    let annee = b.annee().await;
    let attendus: Vec<String> = (1..=8)
        .map(|n| format!("{}-{annee}-{n:06}", journal_de_saisie()))
        .collect();
    assert_eq!(
        sequences, attendus,
        "numéros continus, sans trou ni doublon"
    );
    assert_eq!(b.compteur().await, vec![8]);
    assert!(b.chaine_valide().await, "la chaîne recalculée est valide");
}

#[tokio::test]
async fn deux_validations_du_meme_brouillon() {
    let b = banc().await;
    let (id, v, _) = b.pret("Double").await;
    let (r1, r2) = tokio::join!(b.valider(id, v), b.valider(id, v));
    let statuts = [r1.statut, r2.statut];
    assert_eq!(
        statuts.iter().filter(|s| **s == StatusCode::OK).count(),
        1,
        "{statuts:?}"
    );
    assert!(statuts.contains(&StatusCode::NOT_FOUND) || statuts.contains(&StatusCode::CONFLICT));
    assert_eq!(b.compter("ecriture_validee").await, 1);
    assert_eq!(b.compteur().await, vec![1]);
    assert!(b.chaine_valide().await);
}

/// Déclencheur d'échec, limité au dossier du banc, sur une table : simule la défaillance de
/// l'audit ou d'une insertion en cours de validation.
async fn echec_sur(b: &Banc, table: &str) -> String {
    let nom = format!("echec_{}", Uuid::new_v4().simple());
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "CREATE FUNCTION {nom}() RETURNS trigger LANGUAGE plpgsql AS \
         $$ BEGIN RAISE EXCEPTION 'échec voulu'; END $$"
    )))
    .execute(&b.env.admin)
    .await
    .unwrap();
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "CREATE TRIGGER {nom} BEFORE INSERT ON {table} FOR EACH ROW \
         WHEN (NEW.dossier_id = '{}') EXECUTE FUNCTION {nom}()",
        b.c.dossier.uuid()
    )))
    .execute(&b.env.admin)
    .await
    .unwrap();
    nom
}

async fn lever_echec(b: &Banc, table: &str, nom: &str) {
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "DROP TRIGGER {nom} ON {table}"
    )))
    .execute(&b.env.admin)
    .await
    .unwrap();
    sqlx::query(sqlx::AssertSqlSafe(format!("DROP FUNCTION {nom}()")))
        .execute(&b.env.admin)
        .await
        .unwrap();
}

#[tokio::test]
async fn un_echec_d_audit_ou_d_insertion_annule_la_numerotation() {
    let b = banc().await;
    let (id, v, _) = b.pret("Atomicité").await;
    for table in [
        "audit_dossier",
        "ecriture_validee_ligne",
        "ecriture_validee_piece",
    ] {
        let nom = echec_sur(&b, table).await;
        let r = b.valider(id, v).await;
        assert_eq!(r.statut, StatusCode::INTERNAL_SERVER_ERROR, "{table}");
        lever_echec(&b, table, &nom).await;
        assert_eq!(b.compter("ecriture_validee").await, 0, "{table}");
        assert_eq!(b.compter("ecriture_validee_ligne").await, 0, "{table}");
        assert_eq!(b.compteur().await.iter().sum::<i32>(), 0, "{table}");
        assert_eq!(b.compter("brouillon").await, 1, "{table}");
        let tete: Option<i64> =
            sqlx::query_scalar("SELECT rang FROM chaine_tete WHERE dossier_id = $1")
                .bind(b.c.dossier.uuid())
                .fetch_optional(&b.env.admin)
                .await
                .unwrap();
        assert_eq!(tete.unwrap_or(0), 0, "{table}");
    }
    // Aucun numéro n'a été perdu : la première validation réussie porte la séquence 1.
    let r = b.valider(id, v).await;
    assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
    assert!(r.json()["numero"].as_str().unwrap().ends_with("-000001"));
    assert_eq!(r.json()["rang"], 1);
}

// --- Chaîne, export, vérificateur ------------------------------------------------------------------

/// Écritures validées relues de la base, dans l'ordre de la chaîne, et reconstituées par
/// `compta-core` ([`EcritureValidee::reconstituer`](compta_core::EcritureValidee)).
async fn relues(b: &Banc) -> Vec<(base::EcritureStockee, compta_core::EcritureValidee)> {
    let ids: Vec<Uuid> =
        sqlx::query_scalar("SELECT id FROM ecriture_validee WHERE dossier_id = $1 ORDER BY rang")
            .bind(b.c.dossier.uuid())
            .fetch_all(&b.env.admin)
            .await
            .unwrap();
    let mut tx = ouvrir_transaction(
        &b.env.application,
        ContexteTenant::dossier(b.c.cabinet, b.c.dossier, b.c.administrateur),
    )
    .await
    .unwrap();
    let mut rendues = Vec::new();
    for id in ids {
        let e = base::lire(&mut tx, id).await.unwrap().unwrap();
        let v = reconstituer(&e, referentiel_tva().unwrap()).unwrap();
        rendues.push((e, v));
    }
    tx.annuler().await.unwrap();
    rendues
}

#[tokio::test]
async fn la_chaine_de_la_base_est_celle_de_l_export() {
    let b = banc().await;
    for n in 1..=3 {
        let (id, v, _) = b.pret(&format!("Export {n}")).await;
        assert_eq!(b.valider(id, v).await.statut, StatusCode::OK);
    }
    // Une écriture à étiquette fiscale (export v4).
    let etiquetee = format!(
        r#"{{"compte":"{}","sens":"debit","montant":"500","etiquette":{{"nature":"base","sens":"collectee","regime":"IMP18","taux":"18","exigibilite":"debit"}}}}"#,
        caisse()
    );
    let (id, v) = b
        .pret_avec(&contenu(
            &journal_de_saisie(),
            &aujourd(),
            &[etiquetee, ligne(banque(), "credit", "500")],
            "",
        ))
        .await;
    assert_eq!(b.valider(id, v).await.statut, StatusCode::OK);
    let relues = relues(&b).await;
    assert_eq!(relues.len(), 4);
    // La relecture redonne une écriture de même contenu canonique que celui qui a été haché.
    for (stockee, validee) in &relues {
        assert_eq!(
            integrite::EcritureCanonique::depuis(validee).serialiser(),
            stockee.canonique,
            "{}",
            stockee.numero
        );
    }
    let validees: Vec<_> = relues.iter().map(|(_, v)| v.clone()).collect();
    let dossier = integrite::IdDossier::parse(&b.c.dossier.uuid().to_string()).unwrap();
    let mut export = Vec::new();
    let tete = integrite::exporter(&dossier, &validees, &[], &mut export).unwrap();
    // La tête de l'export est celle de la base ; chaque empreinte aussi.
    let chaine = b.chaine().await;
    assert_eq!(
        tete.octets().as_slice(),
        chaine.last().unwrap().3.as_slice()
    );
    assert!(b.chaine_valide().await);
    // Vérification par la bibliothèque.
    let rapport = integrite::verifier(export.as_slice(), &integrite::Options::default()).unwrap();
    assert!(rapport.anomalies.is_empty(), "{:?}", rapport.anomalies);
    assert_eq!(rapport.nombre, 4);
    // Vérification par le binaire `sigma-verifier` : « intègre », code 0.
    let chemin = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("export-{}.txt", Uuid::new_v4()));
    std::fs::write(&chemin, &export).unwrap();
    let sortie = std::process::Command::new(env!("CARGO"))
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
        .arg(&chemin)
        .output()
        .unwrap();
    let _ = std::fs::remove_file(&chemin);
    assert_eq!(
        sortie.status.code(),
        Some(0),
        "{}{}",
        String::from_utf8_lossy(&sortie.stdout),
        String::from_utf8_lossy(&sortie.stderr)
    );
}

#[tokio::test]
async fn une_ecriture_alteree_en_base_est_detectee() {
    let b = banc().await;
    for n in 1..=3 {
        let (id, v, _) = b.pret(&format!("Altérée {n}")).await;
        assert_eq!(b.valider(id, v).await.statut, StatusCode::OK);
    }
    assert!(b.chaine_valide().await);
    // Le propriétaire (le rôle applicatif ne le peut pas) altère les montants de l'écriture 2 :
    // la relecture ne redonne plus le contenu haché.
    b.sql(
        "UPDATE ecriture_validee_ligne SET montant = 999 WHERE dossier_id = $1 AND ecriture_id = \
         (SELECT id FROM ecriture_validee WHERE dossier_id = $1 AND rang = 2)",
    )
    .await;
    let relues = relues(&b).await;
    let divergentes: Vec<_> = relues
        .iter()
        .filter(|(s, v)| integrite::EcritureCanonique::depuis(v).serialiser() != s.canonique)
        .map(|(s, _)| s.rang)
        .collect();
    assert_eq!(divergentes, vec![2]);
    // Altération du texte stocké : la chaîne recalculée est rompue.
    b.sql(
        "UPDATE ecriture_validee SET contenu_canonique = replace(contenu_canonique, 'Altérée', \
         'Falsifiée') WHERE dossier_id = $1 AND rang = 3",
    )
    .await;
    assert!(!b.chaine_valide().await);
}

// --- Ajout seul et routes -----------------------------------------------------------------------

#[tokio::test]
async fn aucune_modification_ni_suppression_d_une_ecriture_validee() {
    let b = banc().await;
    let (id, v, _) = b.pret("Ajout seul").await;
    assert_eq!(b.valider(id, v).await.statut, StatusCode::OK);
    // Le rôle applicatif, dans le contexte du dossier, ne modifie ni ne supprime rien.
    let mut tx = ouvrir_transaction(
        &b.env.application,
        ContexteTenant::dossier(b.c.cabinet, b.c.dossier, b.c.administrateur),
    )
    .await
    .unwrap();
    for requete in [
        "UPDATE ecriture_validee SET libelle = 'x'",
        "UPDATE ecriture_validee SET rang = 9",
        "UPDATE ecriture_validee_ligne SET montant = 1",
        "UPDATE ecriture_validee_piece SET role = 'complementaire'",
        "DELETE FROM ecriture_validee",
        "DELETE FROM ecriture_validee_ligne",
        "DELETE FROM ecriture_validee_piece",
        "UPDATE chaine_tete SET dossier_id = dossier_id",
        "DELETE FROM chaine_tete",
        "DELETE FROM compteur_ecriture",
    ] {
        let erreur = sqlx::query(sqlx::AssertSqlSafe(requete.to_owned()))
            .execute(tx.connexion())
            .await
            .unwrap_err();
        assert_eq!(
            commun::code_sql(&erreur).as_deref(),
            Some("42501"),
            "{requete} : {erreur}"
        );
        // Une erreur de PostgreSQL abandonne la transaction : on en rouvre une.
        tx = ouvrir_transaction(
            &b.env.application,
            ContexteTenant::dossier(b.c.cabinet, b.c.dossier, b.c.administrateur),
        )
        .await
        .unwrap();
    }
    assert_eq!(b.compter("ecriture_validee").await, 1);
}

#[test]
fn aucune_route_de_modification_ni_de_suppression_des_ecritures() {
    use api::serveur::routes::{Methode, ROUTES};
    let routes: Vec<_> = ROUTES
        .iter()
        .filter(|r| r.chemin.contains("/ecritures"))
        .collect();
    // Deux lectures, et la seule mutation : la contre-passation, qui ajoute une écriture miroir
    // sans toucher à l'origine (ajout seul). Ni modification ni suppression.
    assert_eq!(routes.len(), 3);
    for r in routes {
        if r.methode == Methode::Post {
            assert!(
                r.chemin.ends_with("/{ecriture}/contre-passation"),
                "{}",
                r.chemin
            );
        } else {
            assert_eq!(r.methode, Methode::Get, "{}", r.chemin);
        }
    }
}

#[allow(dead_code)]
fn _types(_: Value) {}
