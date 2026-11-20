//! Brouillons des dossiers de test (PR 13a) : un brouillon fixe par dossier doté, créé par le
//! code de production ([`api::base::brouillard::creer`]), et les corps d'exemple des routes.
//!
//! Les comptes de mouvement sont la caisse et la banque du référentiel (le compte de base de `doter` porte un sous-compte fixe, donc un regroupement). Aucun numéro de compte écrit en dur : les comptes viennent de [`super::referentiel::comptes`],
//! tirés du référentiel officiel selon leurs caractères.

#![allow(clippy::expect_used, clippy::unwrap_used, dead_code)]

use std::sync::OnceLock;

use api::base::brouillard::{ContenuStocke, LigneStockee, creer};
use api::base::referentiel::{Modele, charger};
use api::base::{ContexteTenant, IdCabinet, IdDossier, IdUtilisateur, ouvrir_transaction};
use api::serveur::routes::brouillard::Cas;
use sqlx::PgPool;
use uuid::Uuid;

use super::referentiel::comptes;

/// Date du jour (période ouverte du dossier doté), `AAAA-MM-JJ`.
pub fn aujourd_hui() -> time::Date {
    time::OffsetDateTime::now_utc().date()
}

/// Texte `AAAA-MM-JJ` d'une date.
pub fn texte_date(d: time::Date) -> String {
    format!("{:04}-{:02}-{:02}", d.year(), u8::from(d.month()), d.day())
}

/// Code du premier journal du référentiel officiel qui accepte la saisie.
pub fn journal_de_saisie() -> String {
    Modele::officiel()
        .unwrap()
        .etat()
        .journaux
        .journaux()
        .find(|j| j.genre().accepte_la_saisie())
        .unwrap()
        .code()
        .as_str()
        .to_owned()
}

/// Ligne JSON d'un montant XOF sur un compte.
pub fn ligne(compte: &str, sens: &str, montant: &str) -> String {
    format!(r#"{{"compte":"{compte}","sens":"{sens}","montant":"{montant}"}}"#)
}

/// Corps d'une saisie équilibrée de 1 000 XOF, libellé donné, à la date du jour.
pub fn saisie(libelle: &str, version: Option<i32>) -> String {
    let c = comptes();
    let v = version.map_or(String::new(), |v| format!(r#""version":{v},"#));
    format!(
        r#"{{{v}"journal":"{}","date":"{}","libelle":"{libelle}","lignes":[{},{}]}}"#,
        journal_de_saisie(),
        texte_date(aujourd_hui()),
        ligne(c.caisse.as_str(), "debit", "1000"),
        ligne(c.banque.as_str(), "credit", "1000"),
    )
}

/// Crée le brouillon fixe d'un dossier (déjà doté du référentiel) ; rend son identifiant.
pub async fn doter(
    pool: &PgPool,
    cabinet: IdCabinet,
    dossier: IdDossier,
    utilisateur: IdUtilisateur,
) -> Uuid {
    let mut tx = ouvrir_transaction(pool, ContexteTenant::dossier(cabinet, dossier, utilisateur))
        .await
        .unwrap();
    let relu = charger(&mut tx).await.unwrap().unwrap();
    let code = journal_de_saisie();
    let journal_id = relu
        .id_journal(&compta_core::CodeJournal::parse(&code).unwrap())
        .unwrap();
    let c = comptes();
    let l = |compte: &str, sens: &'static str| LigneStockee {
        compte: compte.to_owned(),
        sens,
        montant: "1000".to_owned(),
        devise: None,
        tiers: None,
        etiquette: None,
    };
    let contenu = ContenuStocke {
        journal_id,
        journal: code,
        date: aujourd_hui(),
        date_valeur_comptable: None,
        libelle: "Brouillon fixe".to_owned(),
        lignes: vec![
            l(c.caisse.as_str(), "debit"),
            l(c.banque.as_str(), "credit"),
        ],
    };
    let id = Uuid::new_v4();
    creer(&mut tx, id, &contenu, time::OffsetDateTime::now_utc())
        .await
        .unwrap();
    tx.confirmer().await.unwrap();
    id
}

/// Corps d'exemple des mutations du module `brouillard` (`commun::serveur::corps_de`).
pub fn corps(cas: Cas) -> &'static str {
    static CREATION: OnceLock<String> = OnceLock::new();
    static MODIFICATION: OnceLock<String> = OnceLock::new();
    match cas {
        Cas::Creer => CREATION.get_or_init(|| saisie("Saisie du test", None)),
        Cas::Modifier => MODIFICATION.get_or_init(|| saisie("Libellé modifié", Some(1))),
        Cas::PoserARevoir | Cas::LeverARevoir => "{}",
        Cas::LierPiece => {
            r#"{"piece":"00000000-0000-4000-8000-0000000000aa","role":"principale","version":0}"#
        }
        Cas::Supprimer | Cas::RetirerPiece => r#"{"version":0}"#,
        Cas::Lister | Cas::Lire => "",
    }
}
