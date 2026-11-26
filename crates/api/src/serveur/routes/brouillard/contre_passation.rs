//! Contre-passation d'une écriture validée (PR 13c de `docs/lots/L2.md`, L2-27 ; spec §6.1 ;
//! AUDCIF art. 20 ; ADR 0011, 0020, 0021).
//!
//! # Ce qui est dans `compta-core`, ce qui est ici
//! L'écriture miroir (sens inversés ; mêmes comptes, montants, devises, tiers, **justification**,
//! **étiquette fiscale** copiée telle quelle par `Ligne::miroir`, version de paramètres) et les
//! contrôles de la validation (la date n'est pas bornée par celle de l'origine)
//! sont ceux de [`compta_core::contre_passer`]. `api` fournit les **faits** : l'origine relue
//! (`reconstituer`), le compteur du journal pour l'exercice de la
//! **nouvelle date**, la tête de chaîne.
//!
//! # Sérialisation
//! Aucun verrou de ligne n'est possible sur une écriture validée (ajout seul). La tête de chaîne
//! du dossier, prise **avant** de relire l'origine, sérialise les contre-passations concurrentes,
//! y compris de la même origine : une origine peut être contre-passée plusieurs fois (ADR 0021
//! point 4, révisé le 2026-11-25), chacune avec son numéro et son rang. Aucun filet d'unicité.
//!
//! # Intégrité de l'origine
//! L'origine relue est **vérifiée** : son contenu canonique recalculé depuis les colonnes doit
//! être celui qui est stocké, et son empreinte celle de la chaîne. Sinon l'origine a été altérée
//! en base : erreur interne (500, catégorie « donnée »), rien n'est écrit.
//!
//! # Droits
//! Administrateur et valideur (§20), rôle relu en base. Pas de ré-authentification (liste de
//! l'invariant Cedar) ni de séparation des tâches : la contre-passation est validée aussitôt
//! (spec §6.1, risque F1.16 accepté, question 10 a). Le motif est **facultatif** (Odoo
//! `wizard/account_move_reversal.py:18`, Tryton `CancelMoves` : aucun motif exigé) ; absent ou blanc,
//! le serveur inscrit « Contre-passation de <numéro de l'origine> ». Il va au journal d'audit.
//! Une origine, contre-passation comprise, peut être contre-passée plusieurs fois.

use authz::Role;
use axum::Json;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use compta_core::{
    CodeExercice, Compteur, Contexte, EcritureValidee, ErreurValidation, TexteCourt,
};
use integrite::{EcritureCanonique, chainer};
use serde::{Deserialize, Serialize};
use time::Date;
use uuid::Uuid;

use super::saisie::date_lue;
use super::validation::{
    Decision, VueAvertissement, donnee, donnee_audit_avertissements, finaliser, motif_validation,
    reconstituer, refus, vues_avertissements,
};
use super::{corps, referentiel_tva, sans_parametres};
use crate::base::TransactionTenant;
use crate::base::audit::PorteeAudit;
use crate::base::brouillard::RolePiece;
use crate::base::organisation::lire_appartenance;
use crate::base::referentiel::charger;
use crate::base::validation::{self as base, Circonstances, Inscription, Tete};
use crate::pieces::interne;
use crate::serveur::audit::Trace;
use crate::serveur::erreur::{ErreurApi, MotifValidation};
use crate::serveur::pipeline::{DossierAutorise, Garde, id_chemin};
use crate::serveur::routes::{Requete, Resultat};

/// Longueur maximale du motif, en octets (une ligne d'audit canonique tient en 1 024 octets).
const MOTIF_MAX: usize = 400;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CorpsContrePassation {
    date: String,
    libelle: String,
    /// Facultatif : absent, `null` ou blanc, le serveur le remplace par un motif par défaut.
    #[serde(default)]
    motif: Option<String>,
}

#[derive(Debug, Serialize)]
struct VueContrePassation {
    id: String,
    numero: String,
    origine: String,
    origine_numero: String,
    rang: i64,
    empreinte: String,
    format: i16,
    date_validation: String,
    avertissements: Vec<VueAvertissement>,
}

/// Motif saisi : `None` s'il est absent ou blanc (le défaut est posé après la lecture de
/// l'origine) ; sinon au plus [`MOTIF_MAX`] octets UTF-8 et sans caractère de contrôle.
fn motif_saisi(texte: Option<&str>) -> Result<Option<String>, ErreurApi> {
    let Some(texte) = texte.filter(|t| !t.trim().is_empty()) else {
        return Ok(None);
    };
    if texte.len() <= MOTIF_MAX && !texte.chars().any(char::is_control) {
        Ok(Some(texte.to_owned()))
    } else {
        Err(ErreurApi::RequeteInvalide)
    }
}

/// Motif par défaut, sans donnée personnelle : le numéro de l'origine seul.
fn motif_par_defaut(numero_origine: &str) -> String {
    format!("Contre-passation de {numero_origine}")
}

/// Rôle relu en base : administrateur ou valideur (§20).
async fn exiger_contre_passeur(tx: &mut TransactionTenant) -> Result<(), ErreurApi> {
    let a = lire_appartenance(tx)
        .await?
        .filter(|a| a.actif)
        .ok_or(ErreurApi::Introuvable)?;
    if a.role == Role::Administrateur.nom() || a.role == Role::Valideur.nom() {
        Ok(())
    } else {
        Err(ErreurApi::Interdit)
    }
}

/// Traduit un refus de `compta-core` propre à la contre-passation, ou le motif de validation.
pub(super) fn erreur(e: &ErreurValidation, exercices: &compta_core::Exercices) -> ErreurApi {
    match e {
        ErreurValidation::EcritureInconnue(_) => ErreurApi::Introuvable,
        autre => {
            let (motif, ligne) = motif_validation(autre, exercices);
            refus(motif, ligne)
        }
    }
}

/// Fonction d'application pure de la contre-passation, sur des faits fournis : l'origine relue,
/// la dernière séquence du journal pour l'exercice de la
/// **nouvelle** date (`dernier`), la tête de chaîne. Même résultat que
/// [`compta_core::Livre::contre_passer`] (propriété des tests).
///
/// # Errors
/// 409 (période, exercice…), 422 (écriture invalide).
#[allow(clippy::too_many_arguments)]
pub(super) fn decider_contre_passation(
    origine: &EcritureValidee,
    date: Date,
    libelle: TexteCourt,
    contexte: &Contexte<'_>,
    dernier: u32,
    tete: &Tete,
    date_validation: Date,
) -> Result<Decision, ErreurApi> {
    // Sans exercice, `compta-core` refuse la date avant d'examiner le compteur.
    let exercice = match contexte.exercices.exercice_contenant(date) {
        Some(e) => e.code().clone(),
        None => CodeExercice::parse("0000").map_err(|_| donnee())?,
    };
    let compteur = Compteur::reprendre(origine.journal().clone(), exercice, dernier)
        .map_err(|_| refus(MotifValidation::SequenceEpuisee, None))?;
    let validation =
        compta_core::contre_passer(origine, date, libelle, contexte, &compteur, date_validation)
            .map_err(|e| erreur(&e, contexte.exercices))?;
    finaliser(validation, tete)
}

#[allow(clippy::too_many_lines)]
pub(super) async fn contre_passer(
    tx: &mut TransactionTenant,
    garde: &Garde,
    dossier: &DossierAutorise,
    requete: &Requete<'_>,
) -> Result<Resultat, ErreurApi> {
    sans_parametres(requete)?;
    let id = id_chemin(requete.chemin, "ecriture")?;
    // La décision Cedar a porté sur cette écriture validée, et sur elle seule.
    if garde.ecriture_validee_du_chemin() != Some(id) {
        return Err(ErreurApi::Introuvable);
    }
    let c: CorpsContrePassation = corps(requete.corps)?;
    let date = date_lue(&c.date).ok_or(ErreurApi::RequeteInvalide)?;
    let libelle = TexteCourt::parse(&c.libelle).map_err(|_| ErreurApi::RequeteInvalide)?;
    let motif_saisi = motif_saisi(c.motif.as_deref())?;
    let tva = referentiel_tva()?;
    let utilisateur = tx.contexte().id_utilisateur().uuid();

    // 1. Verrous, dans l'ordre de l'ADR 0020 : calendrier, comptes et tiers, pièces, tête,
    //    compteur ; puis la tête d'audit, prise par le pipeline. Rôle relu en base.
    base::verrouiller_calendrier(tx).await?;
    exiger_contre_passeur(tx).await?;
    // Les lignes et les liens d'une écriture validée sont définitifs : leur lecture sans verrou
    // désigne ce qu'il faut verrouiller ; l'origine est relue après la tête de chaîne.
    let provisoire = base::lire(tx, id).await?.ok_or(ErreurApi::Introuvable)?;
    let motif = motif_saisi.unwrap_or_else(|| motif_par_defaut(&provisoire.numero));
    let comptes: Vec<String> = provisoire.lignes.iter().map(|l| l.compte.clone()).collect();
    let tiers: Vec<Uuid> = provisoire.lignes.iter().filter_map(|l| l.tiers).collect();
    base::verrouiller_comptes_et_tiers(tx, &comptes, &tiers).await?;
    let referentiel = charger(tx).await?.ok_or(ErreurApi::Introuvable)?;
    let ids_pieces = base::pieces_de(tx, id).await?;
    let pieces = base::verrouiller_pieces(tx, &ids_pieces).await?;
    if pieces.len() != ids_pieces.len() {
        return Err(donnee());
    }

    // Journal de l'origine, exercice de la **nouvelle** date.
    let code_journal =
        compta_core::CodeJournal::parse(&provisoire.journal).map_err(|_| donnee())?;
    let journal_id = referentiel.id_journal(&code_journal).ok_or_else(donnee)?;
    let exercice: CodeExercice = referentiel
        .exercices()
        .exercice_contenant(date)
        .map(|e| e.code().clone())
        .ok_or_else(|| refus(MotifValidation::DateHorsExercice, None))?;
    let exercice_id = referentiel.id_exercice(&exercice).ok_or_else(donnee)?;

    let genese = integrite::genese(
        &integrite::IdDossier::parse(&dossier.id().uuid().to_string()).map_err(|_| donnee())?,
    );
    let tete = base::verrouiller_tete(tx, &genese).await?;
    // Instant relu **après** la tête : la contre-passation a pu attendre, ou franchir minuit.
    let instant = requete.horloge.maintenant();
    let dernier = base::verrouiller_compteur(tx, journal_id, exercice_id).await?;

    // 2. Faits relus **après** la tête : l'origine et son intégrité.
    let stockee = base::lire(tx, id).await?.ok_or(ErreurApi::Introuvable)?;
    let origine = reconstituer(&stockee, tva)?;
    let recalcule = EcritureCanonique::depuis(&origine).serialiser();
    if recalcule != stockee.canonique
        || chainer(&stockee.precedente, recalcule.as_bytes()) != stockee.empreinte
    {
        // Origine altérée en base : jamais contre-passée (erreur interne, journal technique).
        return Err(donnee());
    }
    let contexte = Contexte {
        plan: &referentiel.etat().plan,
        journaux: &referentiel.etat().journaux,
        exercices: referentiel.exercices(),
        delai_cloture: crate::base::referentiel::delai_cloture(),
    };
    let date_validation = instant.date();
    let decision = decider_contre_passation(
        &origine,
        date,
        libelle,
        &contexte,
        dernier,
        &tete,
        date_validation,
    )?;

    // 3. Persistance : écriture miroir, lignes, liens de pièces de l'origine, tête, compteur.
    let nouveau = Uuid::new_v4();
    let roles: Vec<(Uuid, RolePiece)> = stockee.pieces.iter().map(|l| (l.piece, l.role)).collect();
    let numero = decision.validation.ecriture().numero().clone();
    let inscription = Inscription::depuis(
        &decision.validation,
        Circonstances {
            id: nouveau,
            journal_id,
            exercice_id,
            canonique: decision.canonique.clone(),
            format: decision.format,
            empreinte: decision.empreinte,
            precedente: tete.empreinte,
            rang: decision.rang,
            valide_par: utilisateur,
            valide_le: instant,
            pieces: roles,
            origine: Some(id),
        },
    )?;
    base::inserer(tx, &inscription).await?;
    base::mettre_a_jour_tete(tx, decision.rang, &decision.empreinte).await?;
    base::mettre_a_jour_compteur(
        tx,
        journal_id,
        exercice_id,
        decision.validation.compteur().dernier(),
    )
    .await?;

    // 4. Audit : qui, quoi, numéros, rang, empreinte, motif ; jamais l'écriture (ADR 0014).
    let mut trace = Trace::new(PorteeAudit::Dossier(dossier.id()), "ecriture", nouveau);
    trace.motif = Some(motif);
    trace
        .donnees
        .push(("operation", "contre_passation".to_owned()));
    trace.donnees.push(("numero", numero.to_string()));
    trace.donnees.push(("origine", stockee.numero.clone()));
    trace.donnees.push(("rang", decision.rang.to_string()));
    trace
        .donnees
        .push(("empreinte", decision.empreinte.to_string()));
    trace.donnees.push(("format", decision.format.to_string()));
    trace.donnees.extend(donnee_audit_avertissements(
        decision.validation.avertissements(),
    ));
    let vue = VueContrePassation {
        id: nouveau.to_string(),
        numero: numero.to_string(),
        origine: id.to_string(),
        origine_numero: stockee.numero,
        rang: decision.rang,
        empreinte: decision.empreinte.to_string(),
        format: decision.format,
        date_validation: interne::date(date_validation),
        avertissements: vues_avertissements(decision.validation.avertissements()),
    };
    Ok(Resultat {
        reponse: (StatusCode::CREATED, Json(vue)).into_response(),
        traces: vec![trace],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn le_motif_est_facultatif_et_borne() {
        assert_eq!(motif_saisi(None).unwrap(), None);
        assert_eq!(motif_saisi(Some("")).unwrap(), None);
        assert_eq!(motif_saisi(Some("  \t ")).unwrap(), None);
        assert_eq!(
            motif_saisi(Some("Erreur de saisie")).unwrap().as_deref(),
            Some("Erreur de saisie")
        );
        // 400 octets UTF-8 : accepté ; 401 : refusé ; « é » compte pour deux octets.
        assert!(motif_saisi(Some(&"a".repeat(400))).unwrap().is_some());
        assert!(motif_saisi(Some(&"a".repeat(401))).is_err());
        assert!(motif_saisi(Some(&"é".repeat(200))).unwrap().is_some());
        assert!(motif_saisi(Some(&"é".repeat(201))).is_err());
        // Caractère de contrôle : refusé.
        assert!(motif_saisi(Some("a\nb")).is_err());
        assert_eq!(
            motif_par_defaut("AC-2026-000001"),
            "Contre-passation de AC-2026-000001"
        );
    }
}
