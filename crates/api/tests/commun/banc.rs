//! Banc de test de la voie comptabilité (PR 13b et 13c) : cabinet doté, routeur, utilisateurs, saisie
//! et validation par les routes, relecture de la chaîne.

#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    dead_code,
    clippy::cast_possible_truncation,
    clippy::too_many_lines
)]

use std::sync::Arc;

use api::serveur::{Application, Reglages, routeur};
use authz::Autorisation;
use axum::Router;
use axum::http::StatusCode;
use integrite::{Empreinte, chainer, genese};
use uuid::Uuid;

use super::brouillard::{aujourd_hui, ligne, saisie, texte_date};
use super::referentiel::{Fixtures, comptes, doter};
use super::serveur::{
    AuthentificationTest, HorlogeTest, ORIGINE, Reponse, application_base, envoyer,
};
use super::{CabinetTest, Env, ajouter_affecte, creer_cabinet, env};

pub struct Banc {
    pub env: Env,
    pub c: CabinetTest,
    pub f: Fixtures,
    pub routeur: Router,
    pub valideur: api::base::IdUtilisateur,
    pub auditeur: api::base::IdUtilisateur,
}

pub async fn banc_avec(horloge: Option<&HorlogeTest>) -> Banc {
    let env = env().await;
    let c = creer_cabinet(&env.admin, "Cabinet").await;
    let f = doter(&env.application, c.cabinet, c.dossier, c.administrateur).await;
    let valideur = ajouter_affecte(&env.admin, &c, "valideur").await;
    let auditeur = ajouter_affecte(&env.admin, &c, "auditeur").await;
    let etat = match horloge {
        None => application_base(&env, Reglages::new(ORIGINE.to_owned())),
        Some(h) => {
            let mut reglages = super::serveur::reglages_de_test();
            reglages.facteurs.cle_instance = Some(super::serveur::cle_de_test());
            let etat = Application::avec_horloge(
                env.application.clone(),
                env.lecture_audit.clone(),
                Arc::new(Autorisation::charger().unwrap()),
                Arc::new(AuthentificationTest::default()),
                reglages,
                Arc::new(h.clone()),
            );
            super::serveur::brancher_pieces_memoire(&etat);
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

pub async fn banc() -> Banc {
    banc_avec(None).await
}

pub fn code(r: &Reponse) -> String {
    r.json()["erreur"].as_str().unwrap_or_default().to_owned()
}

/// Corps d'une saisie à la date et au journal donnés.
pub fn contenu(journal: &str, date: &str, lignes: &[String], extra: &str) -> String {
    format!(
        r#"{{"journal":"{journal}","date":"{date}","libelle":"Écriture de test"{extra},"lignes":[{}]}}"#,
        lignes.join(",")
    )
}

impl Banc {
    pub fn base(&self) -> String {
        format!(
            "/api/cabinets/{}/dossiers/{}/brouillard",
            self.c.cabinet, self.c.dossier
        )
    }

    pub fn ecritures(&self) -> String {
        format!(
            "/api/cabinets/{}/dossiers/{}/ecritures",
            self.c.cabinet, self.c.dossier
        )
    }

    pub async fn creer(&self, corps: &str) -> Uuid {
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
    pub async fn lier(&self, id: Uuid, version: i32) -> (Uuid, i32) {
        let piece = super::pieces::doter(
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
    pub async fn pret(&self, libelle: &str) -> (Uuid, i32, Uuid) {
        let id = self.creer(&saisie(libelle, None)).await;
        let (piece, version) = self.lier(id, 1).await;
        (id, version, piece)
    }

    pub async fn pret_avec(&self, corps: &str) -> (Uuid, i32) {
        let id = self.creer(corps).await;
        let (_, version) = self.lier(id, 1).await;
        (id, version)
    }

    pub async fn valider_par(
        &self,
        u: api::base::IdUtilisateur,
        id: Uuid,
        version: i32,
    ) -> Reponse {
        envoyer(
            &self.routeur,
            &format!("{}/{id}/validation", self.base()),
            Some(u),
            &format!(r#"{{"version":{version}}}"#),
            None,
        )
        .await
    }

    pub async fn valider(&self, id: Uuid, version: i32) -> Reponse {
        self.valider_par(self.valideur, id, version).await
    }

    pub async fn compter(&self, table: &str) -> i64 {
        sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
            "SELECT count(*) FROM {table} WHERE dossier_id = $1"
        )))
        .bind(self.c.dossier.uuid())
        .fetch_one(&self.env.admin)
        .await
        .unwrap()
    }

    pub async fn sql(&self, requete: &str) {
        sqlx::query(sqlx::AssertSqlSafe(requete.to_owned()))
            .bind(self.c.dossier.uuid())
            .execute(&self.env.admin)
            .await
            .unwrap();
    }

    /// Écritures de la chaîne, dans l'ordre : (rang, numéro, texte, empreinte, précédente).
    pub async fn chaine(&self) -> Vec<(i64, String, String, Vec<u8>, Vec<u8>)> {
        sqlx::query_as(
            "SELECT rang, numero, contenu_canonique, empreinte, empreinte_precedente \
             FROM ecriture_validee WHERE dossier_id = $1 ORDER BY rang",
        )
        .bind(self.c.dossier.uuid())
        .fetch_all(&self.env.admin)
        .await
        .unwrap()
    }

    pub fn genese(&self) -> Empreinte {
        genese(&integrite::IdDossier::parse(&self.c.dossier.uuid().to_string()).unwrap())
    }

    /// La chaîne stockée se recalcule : chaque empreinte suit la précédente et le texte stocké.
    pub async fn chaine_valide(&self) -> bool {
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

    pub async fn compteur(&self) -> Vec<i32> {
        sqlx::query_scalar(
            "SELECT dernier FROM compteur_ecriture WHERE dossier_id = $1 ORDER BY dernier",
        )
        .bind(self.c.dossier.uuid())
        .fetch_all(&self.env.admin)
        .await
        .unwrap()
    }

    pub async fn annee(&self) -> String {
        sqlx::query_scalar("SELECT code FROM exercice WHERE dossier_id = $1 ORDER BY debut LIMIT 1")
            .bind(self.c.dossier.uuid())
            .fetch_one(&self.env.admin)
            .await
            .unwrap()
    }
}

pub fn caisse() -> &'static str {
    comptes().caisse.as_str()
}
pub fn banque() -> &'static str {
    comptes().banque.as_str()
}
pub fn aujourd() -> String {
    texte_date(aujourd_hui())
}
pub fn equilibre(montant: &str) -> Vec<String> {
    vec![
        ligne(caisse(), "debit", montant),
        ligne(banque(), "credit", montant),
    ]
}
