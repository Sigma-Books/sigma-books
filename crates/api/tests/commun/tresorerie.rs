//! Aides de test du module de routes `tresorerie` (lot L3, PR 11) : corps d'exemple des mutations
//! et compte de trésorerie fixe d'un dossier de test (sous-ressource `{compte_tresorerie}`).
//!
//! Aucun numéro de compte écrit en dur : la caisse fixe reprend le journal de caisse par défaut et
//! son compte (données de `syscohada`, [`super::referentiel::comptes`]).

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::sync::OnceLock;

use api::base::{IdCabinet, IdDossier, IdUtilisateur};
use sqlx::PgPool;
use uuid::Uuid;

/// Corps d'exemple des mutations du module `tresorerie` (`commun::serveur::corps_de`). La
/// création est assistée (numérotation libre : chaque appel crée un compte) ; l'arrêté compte
/// 1 000, le solde de l'écriture validée fixe ([`super::brouillard::doter_validee`]).
pub fn corps(cas: api::serveur::routes::tresorerie::Cas) -> &'static str {
    use api::serveur::routes::tresorerie::Cas;
    static ARRETE: OnceLock<String> = OnceLock::new();
    match cas {
        Cas::Creer => r#"{"type":"banque","nom":"Banque du test"}"#,
        Cas::Modifier => r#"{"nom":"Caisse renommée"}"#,
        Cas::ArreterCaisse => ARRETE.get_or_init(|| {
            format!(
                r#"{{"date":"{}","montant_compte":"1000","detail":["10 billets de 100"]}}"#,
                super::brouillard::texte_date(super::brouillard::aujourd_hui())
            )
        }),
        Cas::Lister | Cas::Lire | Cas::VirementsInternes | Cas::ListerArretes => "",
    }
}

/// Dote un dossier de test (référentiel déjà instancié) d'une caisse : le journal de caisse par
/// défaut et son compte, repris tels quels (super-utilisateur, sans l'API). Rend son identifiant.
pub async fn doter(
    admin: &PgPool,
    cabinet: IdCabinet,
    dossier: IdDossier,
    utilisateur: IdUtilisateur,
) -> Uuid {
    let caisse = super::referentiel::comptes().caisse.to_string();
    let (journal, compte, devise): (Uuid, Uuid, String) = sqlx::query_as(
        "SELECT j.id, c.id, d.devise_tenue FROM journal j \
         JOIN compte c ON c.cabinet_id = j.cabinet_id AND c.dossier_id = j.dossier_id \
                      AND c.numero = j.compte_tresorerie \
         JOIN dossier d ON d.cabinet_id = j.cabinet_id AND d.id = j.dossier_id \
         WHERE j.cabinet_id = $1 AND j.dossier_id = $2 AND j.compte_tresorerie = $3",
    )
    .bind(cabinet.uuid())
    .bind(dossier.uuid())
    .bind(&caisse)
    .fetch_one(admin)
    .await
    .unwrap();
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO compte_tresorerie (cabinet_id, dossier_id, id, type, nom, compte_id, \
         journal_id, devise, cree_par, cree_le) \
         VALUES ($1, $2, $3, 'caisse', 'Caisse fixe', $4, $5, $6, $7, now())",
    )
    .bind(cabinet.uuid())
    .bind(dossier.uuid())
    .bind(id)
    .bind(compte)
    .bind(journal)
    .bind(devise)
    .bind(utilisateur.uuid())
    .execute(admin)
    .await
    .unwrap();
    id
}
