//! Aides de test du module de routes `lettrage` (lot L3, PR 9a) : corps d'exemple des mutations,
//! écriture validée lettrable et lettrage fixe d'un dossier de test (sous-ressource
//! `{lettrage}`).
//!
//! Aucun numéro de compte écrit en dur : le compte lettrable est le compte collectif des tiers de
//! test ([`super::referentiel::comptes`], tiré des données de `syscohada`).

#![allow(clippy::expect_used, clippy::unwrap_used, dead_code)]

use api::base::{IdCabinet, IdDossier, IdUtilisateur};
use sqlx::PgPool;
use uuid::Uuid;

/// Corps d'exemple des mutations du module `lettrage` (`commun::serveur::corps_de`). Le lettrage
/// cite deux lignes **inexistantes** : refusé (422 `lettrage_ligne_inconnue`) pour qui a le droit,
/// sans rien écrire ; les tests qui lettrent vraiment passent leurs lignes. La rupture donne un
/// motif.
pub fn corps(cas: api::serveur::routes::lettrage::Cas) -> &'static str {
    use api::serveur::routes::lettrage::Cas;
    match cas {
        Cas::Lettrer => {
            r#"{"lignes":[{"ecriture":"7a1d0000-0000-4000-8000-000000000001","position":1},{"ecriture":"7a1d0000-0000-4000-8000-000000000001","position":2}]}"#
        }
        Cas::Rompre => r#"{"motif":"Règlement imputé sur la mauvaise facture"}"#,
        Cas::LireTiers | Cas::LireCompte => "",
    }
}

/// Corps d'un lettrage des lignes 1 des écritures `debit` et `credit`.
pub fn corps_lettrer(debit: Uuid, credit: Uuid) -> String {
    format!(
        r#"{{"lignes":[{{"ecriture":"{debit}","position":1}},{{"ecriture":"{credit}","position":1}}]}}"#
    )
}

/// Insère, par le super-utilisateur, une écriture validée du jour (contenu canonique et empreintes
/// fictifs, rang et séquence suivants) de 1 000 : ligne 1 au compte collectif des tiers de test,
/// pour le tiers `tiers`, au sens `sens` ; ligne 2 à la caisse, au sens opposé. Rend son
/// identifiant.
pub async fn ecriture_lettrable(
    admin: &PgPool,
    cabinet: IdCabinet,
    dossier: IdDossier,
    utilisateur: IdUtilisateur,
    tiers: Uuid,
    sens: &str,
) -> Uuid {
    let id = Uuid::new_v4();
    let code = super::brouillard::journal_de_saisie();
    let journal: Uuid =
        sqlx::query_scalar("SELECT id FROM journal WHERE dossier_id = $1 AND code = $2")
            .bind(dossier.uuid())
            .bind(&code)
            .fetch_one(admin)
            .await
            .unwrap();
    let (exercice, annee): (Uuid, String) = sqlx::query_as(
        "SELECT id, code FROM exercice WHERE dossier_id = $1 ORDER BY debut LIMIT 1",
    )
    .bind(dossier.uuid())
    .fetch_one(admin)
    .await
    .unwrap();
    let (rang, sequence): (i64, i32) = sqlx::query_as(
        "SELECT COALESCE(max(rang), 0) + 1, COALESCE(max(sequence), 0) + 1 \
         FROM ecriture_validee WHERE dossier_id = $1",
    )
    .bind(dossier.uuid())
    .fetch_one(admin)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO ecriture_validee (cabinet_id, dossier_id, id, journal_id, exercice_id, \
         sequence, numero, date_comptable, libelle, format, contenu_canonique, empreinte, \
         empreinte_precedente, rang, valide_par, date_validation, valide_le) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 'Écriture lettrable fixe', 1, 'fictif', $9, \
                 $10, $11, $12, $8, now())",
    )
    .bind(cabinet.uuid())
    .bind(dossier.uuid())
    .bind(id)
    .bind(journal)
    .bind(exercice)
    .bind(sequence)
    .bind(format!("{code}-{annee}-{sequence:06}"))
    .bind(super::brouillard::aujourd_hui())
    .bind(vec![2u8; 32])
    .bind(vec![0u8; 32])
    .bind(rang)
    .bind(utilisateur.uuid())
    .execute(admin)
    .await
    .unwrap();
    let c = super::referentiel::comptes();
    let oppose = if sens == "debit" { "credit" } else { "debit" };
    for (position, compte, sens, tiers) in [
        (1i16, c.collectif.to_string(), sens, Some(tiers)),
        (2, c.caisse.to_string(), oppose, None),
    ] {
        sqlx::query(
            "INSERT INTO ecriture_validee_ligne (cabinet_id, dossier_id, ecriture_id, position, \
             compte_numero, sens, montant, tiers_id) VALUES ($1, $2, $3, $4, $5, $6, 1000, $7)",
        )
        .bind(cabinet.uuid())
        .bind(dossier.uuid())
        .bind(id)
        .bind(position)
        .bind(compte)
        .bind(sens)
        .bind(tiers)
        .execute(admin)
        .await
        .unwrap();
    }
    // Une écriture validée porte toujours une justification (AUDCIF art. 17 et 22), comme
    // `super::brouillard::doter_validee`.
    let piece = super::pieces::doter(admin, cabinet, dossier, utilisateur).await;
    sqlx::query(
        "INSERT INTO ecriture_validee_piece (cabinet_id, dossier_id, ecriture_id, piece_id, \
         position, role) VALUES ($1, $2, $3, $4, 1, 'principale')",
    )
    .bind(cabinet.uuid())
    .bind(dossier.uuid())
    .bind(id)
    .bind(piece)
    .execute(admin)
    .await
    .unwrap();
    id
}

/// Dote un dossier de test (référentiel et tiers fixe déjà créés) d'un lettrage fixe, lettre `A`,
/// des lignes collectives de deux [`ecriture_lettrable`] (super-utilisateur, sans l'API). Rend son
/// identifiant, sous-ressource `{lettrage}` des routes du dossier.
pub async fn doter(
    admin: &PgPool,
    cabinet: IdCabinet,
    dossier: IdDossier,
    utilisateur: IdUtilisateur,
    tiers: Uuid,
) -> Uuid {
    let debit = ecriture_lettrable(admin, cabinet, dossier, utilisateur, tiers, "debit").await;
    let credit = ecriture_lettrable(admin, cabinet, dossier, utilisateur, tiers, "credit").await;
    let compte: Uuid =
        sqlx::query_scalar("SELECT id FROM compte WHERE dossier_id = $1 AND numero = $2")
            .bind(dossier.uuid())
            .bind(super::referentiel::comptes().collectif.to_string())
            .fetch_one(admin)
            .await
            .unwrap();
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO lettrage (cabinet_id, dossier_id, id, compte_id, lettre, rang, \
         date_operation, origine, auteur, cree_le) \
         VALUES ($1, $2, $3, $4, 'A', 1, $5, 'manuelle', $6, now())",
    )
    .bind(cabinet.uuid())
    .bind(dossier.uuid())
    .bind(id)
    .bind(compte)
    .bind(super::brouillard::aujourd_hui())
    .bind(utilisateur.uuid())
    .execute(admin)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO lettrage_lien (cabinet_id, dossier_id, lettrage_id, position, \
         debit_ecriture_id, debit_position, credit_ecriture_id, credit_position, montant) \
         VALUES ($1, $2, $3, 1, $4, 1, $5, 1, 1000)",
    )
    .bind(cabinet.uuid())
    .bind(dossier.uuid())
    .bind(id)
    .bind(debit)
    .bind(credit)
    .execute(admin)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO compteur_lettrage (cabinet_id, dossier_id, compte_id, derniere_lettre, \
         dernier_rang) VALUES ($1, $2, $3, 'A', 1)",
    )
    .bind(cabinet.uuid())
    .bind(dossier.uuid())
    .bind(compte)
    .execute(admin)
    .await
    .unwrap();
    id
}
