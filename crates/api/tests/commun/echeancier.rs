//! Aides de test du module de routes `echeancier` (lot L3, PR 8 ; ADR 0033) : corps d'exemple des
//! mutations, condition de règlement fixe et écriture validée à ligne de tiers d'un dossier doté.

#![allow(clippy::expect_used, clippy::unwrap_used, dead_code)]

use api::base::{IdCabinet, IdDossier, IdUtilisateur};
use sqlx::PgPool;
use uuid::Uuid;

use super::brouillard::{aujourd_hui, journal_de_saisie};
use super::referentiel::comptes;

/// Corps d'exemple des mutations du module `echeancier` (`commun::serveur::corps_de`). Joués une
/// fois par dossier (isolation, audit), ils réussissent : création d'une condition, changement de
/// libellé de la condition fixe, révision de la ligne de tiers (position 1) de l'écriture fixe à
/// ligne de tiers ([`doter_ecriture_tiers`]).
pub fn corps(cas: api::serveur::routes::echeancier::Cas) -> &'static str {
    use api::serveur::routes::echeancier::Cas;
    match cas {
        Cas::CreerCondition => {
            r#"{"code":"TEST-30J","libelle":"30 jours du test","termes":[{"valeur":"solde","delai":"jours","jours":30}]}"#
        }
        Cas::ModifierCondition => r#"{"libelle":"Libellé modifié"}"#,
        Cas::ReviserEcheance => {
            r#"{"position":1,"nouvelle_echeance":"2099-12-31","motif":"Report accordé"}"#
        }
        Cas::ListerConditions
        | Cas::LireCondition
        | Cas::CalculerEcheances
        | Cas::LireEcheances => "",
    }
}

/// Insère, par le super-utilisateur, la condition de règlement fixe d'un dossier (« 30 jours »,
/// code `FIXE-30J`) ; rend son identifiant, sous-ressource `{condition}` des routes.
pub async fn doter(admin: &PgPool, cabinet: IdCabinet, dossier: IdDossier) -> Uuid {
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO condition_reglement (cabinet_id, dossier_id, id, code, libelle, cree_le) \
         VALUES ($1, $2, $3, 'FIXE-30J', '30 jours fixe', now())",
    )
    .bind(cabinet.uuid())
    .bind(dossier.uuid())
    .bind(id)
    .execute(admin)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO condition_reglement_terme (cabinet_id, dossier_id, condition_id, rang, \
         valeur, delai, jours) VALUES ($1, $2, $3, 1, 'solde', 'jours', 30)",
    )
    .bind(cabinet.uuid())
    .bind(dossier.uuid())
    .bind(id)
    .execute(admin)
    .await
    .unwrap();
    id
}

/// Insère, par le super-utilisateur, une écriture validée minimale (lecture seule : contenu
/// canonique et empreintes fictifs, comme `brouillard::doter_validee`) dont la **ligne 1** porte
/// le tiers `tiers` sur le compte collectif de test, avec une échéance initiale au dernier jour de
/// l'année courante ; la ligne 2 est sur la caisse. Rang de chaîne `rang` (unique par dossier).
pub async fn doter_ecriture_tiers(
    admin: &PgPool,
    cabinet: IdCabinet,
    dossier: IdDossier,
    utilisateur: IdUtilisateur,
    tiers: Uuid,
    rang: i64,
) -> Uuid {
    let id = Uuid::new_v4();
    let c = comptes();
    let code = journal_de_saisie();
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
    let sequence = 900 + rang;
    let numero = format!("{code}-{annee}-{sequence:06}");
    sqlx::query(
        "INSERT INTO ecriture_validee (cabinet_id, dossier_id, id, journal_id, exercice_id, \
         sequence, numero, date_comptable, libelle, format, contenu_canonique, empreinte, \
         empreinte_precedente, rang, valide_par, date_validation, valide_le) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 'Facture à échéance', 1, 'fictif', $9, $10, \
                 $11, $12, $8, now())",
    )
    .bind(cabinet.uuid())
    .bind(dossier.uuid())
    .bind(id)
    .bind(journal)
    .bind(exercice)
    .bind(i32::try_from(sequence).unwrap())
    .bind(numero)
    .bind(aujourd_hui())
    .bind(vec![2u8; 32])
    .bind(vec![0u8; 32])
    .bind(rang)
    .bind(utilisateur.uuid())
    .execute(admin)
    .await
    .unwrap();
    for (position, compte, sens, tiers) in [
        (1i16, c.collectif.as_str(), "debit", Some(tiers)),
        (2, c.caisse.as_str(), "credit", None),
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
    let fin_d_annee =
        time::Date::from_calendar_date(aujourd_hui().year(), time::Month::December, 31).unwrap();
    sqlx::query(
        "INSERT INTO echeance_ligne (cabinet_id, dossier_id, ecriture_id, position, \
         date_echeance) VALUES ($1, $2, $3, 1, $4)",
    )
    .bind(cabinet.uuid())
    .bind(dossier.uuid())
    .bind(id)
    .bind(fin_d_annee)
    .execute(admin)
    .await
    .unwrap();
    id
}
