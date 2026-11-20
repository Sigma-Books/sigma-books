//! Périodes et exercice suivant (PR 11b de `docs/lots/L2.md`, L2-23) : verrouiller,
//! déverrouiller (motif), clôturer (AUDCIF art. 22 3°, irréversible) une période ; ouvrir
//! l'exercice suivant.
//!
//! Persistance, verrous et brouillard : [`crate::base::referentiel::changer_statut`] et
//! [`crate::base::referentiel::ouvrir_exercice_suivant`].
//!
//! Défense en profondeur (ADR 0011) : même si Cedar se trompait, chaque cas d'usage relit le
//! dossier du chemin **visible** (fait par l'appelant), exige une appartenance active du rôle
//! autorisé, **lue en base** (déverrouiller : administrateur seul ; sinon administrateur ou
//! valideur), et, pour les actions ré-authentifiées (déverrouiller, clôturer, ouvrir l'exercice
//! suivant), une session fraîche. La période `{periode}` est cherchée dans le seul dossier du
//! contexte : celle d'un autre dossier est absente (404).
//!
//! Une transition refusée par les règles donne 409, avec un code stable par motif
//! ([`MotifConflit`] : période déjà clôturée, antérieure non clôturée, dernière période,
//! brouillard daté dans la période avec le nombre d'écritures, deux exercices déjà ouverts…) ;
//! rien n'est écrit. Un corps invalide donne 400.

use super::{Cas, Requete, Resultat, corps, vue_exercice, vue_periode};
use crate::base::audit::PorteeAudit;
use crate::base::organisation::lire_appartenance;
use crate::base::referentiel::{
    ErreurPeriode, RefusPeriode, Transition, changer_statut, nom_statut_exercice,
    nom_statut_periode, ouvrir_exercice_suivant,
};
use crate::base::{IdDossier, TransactionTenant};
use crate::serveur::audit::Trace;
use crate::serveur::erreur::{CategorieErreur, ErreurApi, MotifConflit};
use crate::serveur::pipeline::{Garde, id_chemin};
use authz::Role;
use axum::Json;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use compta_core::TexteCourt;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CorpsVide {}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CorpsDeverrouillage {
    motif: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CorpsCloture {
    confirmation: String,
}

/// Confirmation exigée dans le corps d'une clôture : l'action est irréversible.
const CONFIRMATION_CLOTURE: &str = "irreversible";

/// Rôles admis pour un cas, et exigence d'une session fraîche (comme `roles/dossier.cedar` et
/// `invariants.cedar`).
fn exigences(cas: Cas) -> (&'static [Role], bool) {
    const PARAMETREURS: &[Role] = &[Role::Administrateur, Role::Valideur];
    match cas {
        Cas::DeverrouillerPeriode => (&[Role::Administrateur], true),
        Cas::CloturerPeriode | Cas::OuvrirExerciceSuivant => (PARAMETREURS, true),
        _ => (PARAMETREURS, false),
    }
}

/// Défense en profondeur : appartenance **active** lue en base, du rôle admis (absente ou
/// suspendue : 404 ; autre rôle : 403), et session fraîche si l'action l'exige (403).
async fn exiger(tx: &mut TransactionTenant, garde: &Garde, cas: Cas) -> Result<(), ErreurApi> {
    let (roles, fraiche) = exigences(cas);
    let appartenance = lire_appartenance(tx)
        .await?
        .filter(|a| a.actif)
        .ok_or(ErreurApi::Introuvable)?;
    if !roles.iter().any(|r| r.nom() == appartenance.role) {
        return Err(ErreurApi::Interdit);
    }
    if fraiche && !garde.session_fraiche() {
        return Err(ErreurApi::Interdit);
    }
    Ok(())
}

fn erreur(e: ErreurPeriode) -> ErreurApi {
    let conflit = |motif| ErreurApi::Conflit {
        motif,
        nombre: None,
    };
    match e {
        ErreurPeriode::Absente => ErreurApi::Introuvable,
        ErreurPeriode::Refus(refus) => match refus {
            RefusPeriode::PeriodeDejaCloturee => conflit(MotifConflit::PeriodeDejaCloturee),
            RefusPeriode::PeriodeNonOuverte => conflit(MotifConflit::PeriodeNonOuverte),
            RefusPeriode::PeriodeNonVerrouillee => conflit(MotifConflit::PeriodeNonVerrouillee),
            RefusPeriode::DernierePeriode => conflit(MotifConflit::DernierePeriode),
            RefusPeriode::PeriodeAnterieureNonCloturee => {
                conflit(MotifConflit::PeriodeAnterieureNonCloturee)
            }
            RefusPeriode::BrouillardNonValide { nombre } => ErreurApi::Conflit {
                motif: MotifConflit::BrouillardNonValide,
                nombre: Some(u32::try_from(nombre).unwrap_or(u32::MAX)),
            },
            RefusPeriode::ExercicesNonCloturesMax => conflit(MotifConflit::ExercicesNonCloturesMax),
            RefusPeriode::Autre(_) => ErreurApi::RequeteInvalide,
        },
        ErreurPeriode::Base(e) => ErreurApi::from(e),
    }
}

/// Exécute un cas d'usage des périodes ou de l'exercice suivant.
pub(super) async fn executer(
    cas: Cas,
    tx: &mut TransactionTenant,
    garde: &Garde,
    dossier: IdDossier,
    requete: &Requete<'_>,
) -> Result<Resultat, ErreurApi> {
    exiger(tx, garde, cas).await?;
    if cas == Cas::OuvrirExerciceSuivant {
        corps::<CorpsVide>(requete.corps)?;
        return ouvrir(tx, dossier).await;
    }
    let periode = id_chemin(requete.chemin, "periode")?;
    let transition = match cas {
        Cas::VerrouillerPeriode => {
            corps::<CorpsVide>(requete.corps)?;
            Transition::Verrouiller
        }
        Cas::DeverrouillerPeriode => {
            let c: CorpsDeverrouillage = corps(requete.corps)?;
            let motif = TexteCourt::parse(&c.motif).map_err(|_| ErreurApi::RequeteInvalide)?;
            Transition::Deverrouiller(motif)
        }
        Cas::CloturerPeriode => {
            let c: CorpsCloture = corps(requete.corps)?;
            if c.confirmation != CONFIRMATION_CLOTURE {
                return Err(ErreurApi::RequeteInvalide);
            }
            Transition::Cloturer
        }
        _ => return Err(ErreurApi::Interne(CategorieErreur::Donnee)),
    };
    let changement = changer_statut(
        tx,
        periode,
        transition,
        garde.utilisateur().uuid(),
        requete.maintenant,
    )
    .await
    .map_err(erreur)?;

    let interne = ErreurApi::Interne(CategorieErreur::Donnee);
    let exercice = changement
        .exercices
        .exercice_contenant(changement.debut)
        .ok_or(interne)?;
    let periode = exercice
        .periode_contenant(changement.debut)
        .ok_or(interne)?;
    let vue = vue_periode(changement.id, exercice, periode)?;
    let mut trace = Trace::new(PorteeAudit::Dossier(dossier), "periode", changement.id);
    trace.motif = changement.motif.as_ref().map(ToString::to_string);
    trace.donnees = vec![
        ("exercice", changement.exercice.to_string()),
        ("debut", changement.debut.to_string()),
    ];
    trace.champs = vec![(
        "statut",
        Some(nom_statut_periode(changement.avant).to_owned()),
        Some(nom_statut_periode(changement.apres).to_owned()),
    )];
    Ok(Resultat {
        reponse: (StatusCode::OK, Json(vue)).into_response(),
        traces: vec![trace],
    })
}

/// Ouvre l'exercice suivant : 201 et l'exercice, avec ses périodes.
async fn ouvrir(tx: &mut TransactionTenant, dossier: IdDossier) -> Result<Resultat, ErreurApi> {
    let ouvert = ouvrir_exercice_suivant(tx).await.map_err(erreur)?;
    let e = &ouvert.exercice;
    let vue = vue_exercice(ouvert.id, e, |d| ouvert.ids_periodes.get(&d).copied())?;
    let mut trace = Trace::new(PorteeAudit::Dossier(dossier), "exercice", ouvert.id);
    trace.donnees = vec![("periodes", e.periodes().len().to_string())];
    trace.champs = vec![
        ("code", None, Some(e.code().to_string())),
        ("debut", None, Some(e.debut().to_string())),
        ("fin", None, Some(e.fin().to_string())),
        (
            "statut",
            None,
            Some(nom_statut_exercice(e.statut()).to_owned()),
        ),
    ];
    Ok(Resultat {
        reponse: (StatusCode::CREATED, Json(vue)).into_response(),
        traces: vec![trace],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chaque_refus_donne_son_conflit() {
        let cas = [
            (
                RefusPeriode::PeriodeDejaCloturee,
                "periode_deja_cloturee",
                None,
            ),
            (RefusPeriode::PeriodeNonOuverte, "periode_non_ouverte", None),
            (
                RefusPeriode::PeriodeNonVerrouillee,
                "periode_non_verrouillee",
                None,
            ),
            (RefusPeriode::DernierePeriode, "derniere_periode", None),
            (
                RefusPeriode::PeriodeAnterieureNonCloturee,
                "periode_anterieure_non_cloturee",
                None,
            ),
            (
                RefusPeriode::BrouillardNonValide { nombre: 4 },
                "brouillard_non_valide",
                Some(4),
            ),
            (
                RefusPeriode::ExercicesNonCloturesMax,
                "exercices_non_clotures_max",
                None,
            ),
        ];
        for (refus, code, nombre_attendu) in cas {
            let e = erreur(ErreurPeriode::Refus(refus));
            assert_eq!(e.statut(), StatusCode::CONFLICT);
            assert_eq!(e.code(), code);
            let ErreurApi::Conflit { nombre, .. } = e else {
                panic!("{e:?}");
            };
            assert_eq!(nombre, nombre_attendu);
        }
    }
}
