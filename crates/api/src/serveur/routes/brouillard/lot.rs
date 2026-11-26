//! Validation par lot (PR 13c de `docs/lots/L2.md`, L2-27 ; ADR 0011, 0020, 0021).
//!
//! # Garanties
//! - **Tout ou rien** : si une seule écriture retenue est refusée, rien n'est écrit (409
//!   `lot_refuse` avec la liste `(ecriture, code)`), ni numéro consommé, ni chaîne avancée, ni
//!   trace. Le cas d'usage ne produit aucune écriture avant d'avoir validé **toutes** les
//!   autres en mémoire ; le pipeline annule de toute façon la transaction d'une réponse d'erreur.
//! - **Borne de 100** écritures (la tête de chaîne est tenue pendant tout le lot), sans doublon.
//! - **Ré-authentification** : Cedar l'exige (invariant `invariant-reauthentification`, sur
//!   `ValiderParLot`) ; le cas d'usage la recontrôle (`session_fraiche`) si Cedar se trompait.
//! - **Écritures « à revoir »** : exclues du lot et comptées dans la réponse
//!   (`exclues_a_revoir`) ; elles restent au brouillard. Un lot qui n'en retient aucune est refusé
//!   (409 `lot_sans_ecriture_a_valider`).
//! - **Autorisation** : [`Garde::decider_lot`] (`ValiderParLot` sur le dossier **et**
//!   `ValiderEcriture` sur chaque brouillon : séparation des tâches), puis rôle et séparation
//!   relus en base après les verrous, comme la validation unitaire. Un lot n'est jamais délégué
//!   (administrateur et valideur seulement, §20).
//!
//! # Ordre des verrous (le même que la validation unitaire, ADR 0020)
//! Calendrier, brouillons (`FOR UPDATE`, par identifiant), comptes et tiers (union du lot, par
//! ordre croissant), pièces (union du lot, par identifiant), tête de chaîne, compteurs (par
//! journal et exercice), tête d'audit. Trois temps : (A) verrous et faits, contrôles de forme de
//! chaque écriture ; (B) validation pure de toutes, dans l'ordre d'attribution, avec le compteur
//! et la tête qui avancent en mémoire ; (C) insertions.
//!
//! # Ordre d'attribution
//! Date comptable, instant de création, identifiant : déterministe et indépendant de l'ordre du
//! corps de la requête.

use std::collections::{BTreeMap, BTreeSet};

use axum::Json;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use compta_core::Contexte;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::validation::{
    Decision, Preparee, VueValidation, controler_pieces, decider, donnee,
    donnee_audit_avertissements, preparer, roles_des_pieces, vues_avertissements,
};
use super::{conflit, corps, referentiel_tva, sans_parametres, trace_de};
use crate::base::TransactionTenant;
use crate::base::brouillard::{self, BrouillonStocke};
use crate::base::organisation::{lire_appartenance, lire_cabinet, lire_contexte_du_dossier};
use crate::base::referentiel::charger;
use crate::base::validation::{self as base, Circonstances, Inscription, PieceVerrouillee, Tete};
use crate::pieces::interne;
use crate::serveur::erreur::{ErreurApi, MotifConflit, RefusLot, reponse_lot_refuse};
use crate::serveur::pipeline::{DossierAutorise, Garde};
use crate::serveur::routes::{Requete, Resultat};

/// Nombre maximal d'écritures d'un lot.
pub const LOT_MAX: usize = 100;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CorpsLot {
    ecritures: Vec<ElementLot>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ElementLot {
    ecriture: String,
    version: i32,
}

#[derive(Debug, Serialize)]
struct VueLot {
    ecritures: Vec<VueValidation>,
    exclues_a_revoir: usize,
}

/// Écriture retenue, prête à être validée en mémoire.
struct Retenue {
    brouillon: BrouillonStocke,
    preparee: Preparee,
    pieces: Vec<PieceVerrouillee>,
    journal_id: Uuid,
    exercice_id: Uuid,
}

/// Écriture validée en mémoire, prête à être inscrite.
struct Validee {
    brouillon: BrouillonStocke,
    pieces: Vec<PieceVerrouillee>,
    journal_id: Uuid,
    exercice_id: Uuid,
    decision: Decision,
}

/// Code stable d'un refus propre à une écriture (409 ou 422) ; une erreur interne ou un refus
/// global (403, 404) se propage telle quelle et annule le lot.
fn code_de(e: ErreurApi) -> Result<&'static str, ErreurApi> {
    match e {
        ErreurApi::Conflit { motif, .. } => Ok(motif.code()),
        ErreurApi::Validation { motif, .. } => Ok(motif.code()),
        autre => Err(autre),
    }
}

/// Liste fermée des identifiants du corps : canoniques, sans doublon, de 1 à [`LOT_MAX`].
fn elements(requete: &Requete<'_>) -> Result<Vec<(Uuid, i32)>, ErreurApi> {
    let CorpsLot { ecritures } = corps(requete.corps)?;
    if ecritures.is_empty() || ecritures.len() > LOT_MAX {
        return Err(ErreurApi::RequeteInvalide);
    }
    let mut vus = BTreeSet::new();
    let mut liste = Vec::with_capacity(ecritures.len());
    for e in ecritures {
        let id: Uuid = e.ecriture.parse().map_err(|_| ErreurApi::RequeteInvalide)?;
        if id.to_string() != e.ecriture || !vus.insert(id) {
            return Err(ErreurApi::RequeteInvalide);
        }
        liste.push((id, e.version));
    }
    Ok(liste)
}

/// Rôle relu en base : administrateur ou valideur (jamais de délégation pour un lot, §20).
async fn exiger_valideur_de_lot(tx: &mut TransactionTenant) -> Result<(), ErreurApi> {
    let a = lire_appartenance(tx)
        .await?
        .filter(|a| a.actif)
        .ok_or(ErreurApi::Introuvable)?;
    if a.role == authz::Role::Administrateur.nom() || a.role == authz::Role::Valideur.nom() {
        Ok(())
    } else {
        Err(ErreurApi::Interdit)
    }
}

#[allow(clippy::too_many_lines)]
pub(super) async fn valider_par_lot(
    tx: &mut TransactionTenant,
    garde: &Garde,
    dossier: &DossierAutorise,
    requete: &Requete<'_>,
) -> Result<Resultat, ErreurApi> {
    sans_parametres(requete)?;
    let demandees = elements(requete)?;
    let tva = referentiel_tva()?;
    let utilisateur = tx.contexte().id_utilisateur().uuid();

    // Ré-authentification : recontrôlée ici, même si Cedar se trompait.
    if !garde.session_fraiche() {
        return Err(ErreurApi::Interdit);
    }
    // Décision Cedar sur le lot **et** sur chaque brouillon (séparation des tâches).
    let ids: Vec<Uuid> = demandees.iter().map(|(id, _)| *id).collect();
    garde.decider_lot(tx, dossier.id(), &ids).await?;

    // (A) Verrous, dans l'ordre de l'ADR 0020 : calendrier, brouillons par identifiant, …
    base::verrouiller_calendrier(tx).await?;
    let mut tries = ids.clone();
    tries.sort_unstable();
    let mut versions = BTreeMap::new();
    for id in &tries {
        let courante = brouillard::verrouiller(tx, *id)
            .await?
            .ok_or(ErreurApi::Introuvable)?;
        versions.insert(*id, courante);
    }
    // Rôle relu **après** les verrous des brouillons (comme la validation unitaire).
    exiger_valideur_de_lot(tx).await?;
    let cabinet = lire_cabinet(tx).await?.ok_or(ErreurApi::Introuvable)?;

    // État relu après les verrous ; exclusions et refus de version, dans l'ordre de la requête.
    let mut refus: Vec<RefusLot> = Vec::new();
    let mut exclues = 0usize;
    let mut candidates: Vec<BrouillonStocke> = Vec::new();
    for (id, version) in &demandees {
        let b = brouillard::lire(tx, *id)
            .await?
            .ok_or(ErreurApi::Introuvable)?;
        // Séparation des tâches : même si Cedar se trompait (créateur et correcteurs).
        if cabinet.separation_taches && b.contributeurs.contains(&utilisateur) {
            return Err(ErreurApi::Interdit);
        }
        if b.a_revoir {
            exclues += 1;
        } else if versions.get(id) != Some(version) {
            refus.push(RefusLot {
                ecriture: *id,
                code: MotifConflit::BrouillonPerime.code(),
            });
        } else {
            candidates.push(b);
        }
    }
    if candidates.is_empty() && refus.is_empty() {
        return Err(conflit(MotifConflit::LotSansEcritureAValider));
    }

    // Comptes et tiers (union du lot, par ordre croissant), puis référentiel, puis pièces.
    let comptes: Vec<String> = candidates
        .iter()
        .flat_map(|b| b.contenu.lignes.iter().map(|l| l.compte.clone()))
        .collect();
    let tiers: Vec<Uuid> = candidates
        .iter()
        .flat_map(|b| b.contenu.lignes.iter().filter_map(|l| l.tiers))
        .collect();
    base::verrouiller_comptes_et_tiers(tx, &comptes, &tiers).await?;
    let referentiel = charger(tx).await?.ok_or(ErreurApi::Introuvable)?;
    let tenue = dossier.contexte().tenue()?;
    let toutes_pieces: Vec<Uuid> = candidates
        .iter()
        .flat_map(|b| b.pieces.iter().map(|l| l.piece))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let verrouillees = base::verrouiller_pieces(tx, &toutes_pieces).await?;

    // Contrôles de forme de chaque écriture retenue, sans rien consommer.
    let mut retenues: Vec<Retenue> = Vec::with_capacity(candidates.len());
    for b in candidates {
        let id = b.id;
        let pieces: Vec<PieceVerrouillee> = verrouillees
            .iter()
            .filter(|p| b.pieces.iter().any(|l| l.piece == p.id))
            .cloned()
            .collect();
        let controle = controler_pieces(&b, &pieces).and_then(|()| {
            let empreintes: Vec<String> = pieces.iter().map(|p| p.empreinte.clone()).collect();
            let preparee = preparer(&b, &empreintes, tva, &tenue, referentiel.exercices())?;
            let journal_id = referentiel
                .id_journal(preparee.journal())
                .ok_or_else(donnee)?;
            let exercice_id = preparee
                .exercice()
                .and_then(|e| referentiel.id_exercice(e))
                .ok_or(ErreurApi::Validation {
                    motif: crate::serveur::erreur::MotifValidation::DateHorsExercice,
                    ligne: None,
                })?;
            Ok((preparee, journal_id, exercice_id))
        });
        match controle {
            Ok((preparee, journal_id, exercice_id)) => retenues.push(Retenue {
                brouillon: b,
                preparee,
                pieces,
                journal_id,
                exercice_id,
            }),
            Err(e) => refus.push(RefusLot {
                ecriture: id,
                code: code_de(e)?,
            }),
        }
    }
    // Même si une écriture est déjà refusée, les autres sont validées en mémoire (B) pour rendre
    // **tous** les refus d'un coup ; rien n'est écrit dans tous les cas.

    // Ordre d'attribution déterministe : date comptable, création, identifiant.
    retenues.sort_by(|x, y| {
        let (a, b) = (&x.brouillon, &y.brouillon);
        (a.contenu.date, a.cree_le, a.id).cmp(&(b.contenu.date, b.cree_le, b.id))
    });

    // Tête de chaîne, instant, compteurs (par journal et exercice, dans l'ordre croissant).
    let genese = integrite::genese(
        &integrite::IdDossier::parse(&dossier.id().uuid().to_string()).map_err(|_| donnee())?,
    );
    let tete = base::verrouiller_tete(tx, &genese).await?;
    // Devise de tenue relue après la tête (ADR 0027) : voir la validation unitaire.
    if lire_contexte_du_dossier(tx).await?.tenue()? != tenue {
        return Err(conflit(MotifConflit::ContexteDossierModifie));
    }
    let instant = requete.horloge.maintenant();
    let mut compteurs: BTreeMap<(Uuid, Uuid), u32> = BTreeMap::new();
    for r in &retenues {
        compteurs.insert((r.journal_id, r.exercice_id), 0);
    }
    let cles: Vec<(Uuid, Uuid)> = compteurs.keys().copied().collect();
    for (journal, exercice) in cles {
        let dernier = base::verrouiller_compteur(tx, journal, exercice).await?;
        compteurs.insert((journal, exercice), dernier);
    }
    // Date du jour dans le fuseau du dossier (ADR 0027).
    let date_validation = dossier.contexte().date_du_jour(instant)?;
    let contexte = Contexte {
        plan: &referentiel.etat().plan,
        journaux: &referentiel.etat().journaux,
        exercices: referentiel.exercices(),
        delai_cloture: crate::base::referentiel::delai_cloture(),
        tenue,
    };

    // (B) Validation pure de toutes les écritures, dans l'ordre : compteurs et tête avancent en
    // mémoire ; un seul refus et rien n'est écrit.
    let mut courante = tete;
    let mut validees: Vec<Validee> = Vec::with_capacity(retenues.len());
    for r in retenues {
        let cle = (r.journal_id, r.exercice_id);
        let dernier = compteurs.get(&cle).copied().ok_or_else(donnee)?;
        let Retenue {
            brouillon,
            preparee,
            pieces,
            journal_id,
            exercice_id,
        } = r;
        let id = brouillon.id;
        match decider(preparee, &contexte, dernier, &courante, date_validation) {
            Ok(d) => {
                compteurs.insert(cle, d.validation.compteur().dernier());
                courante = Tete {
                    rang: d.rang,
                    empreinte: d.empreinte,
                };
                validees.push(Validee {
                    brouillon,
                    pieces,
                    journal_id,
                    exercice_id,
                    decision: d,
                });
            }
            Err(e) => refus.push(RefusLot {
                ecriture: id,
                code: code_de(e)?,
            }),
        }
    }
    if !refus.is_empty() {
        return Ok(Resultat::lecture(reponse_lot_refuse(&refus)));
    }

    // (C) Insertions, dans l'ordre d'attribution.
    let mut vues = Vec::with_capacity(validees.len());
    let mut traces = Vec::with_capacity(validees.len());
    let mut precedente = tete.empreinte;
    for v in &validees {
        let (b, d, r) = (&v.brouillon, &v.decision, v);
        let roles = roles_des_pieces(b, &v.pieces)?;
        let numero = d.validation.ecriture().numero().clone();
        let inscription = Inscription::depuis(
            &d.validation,
            Circonstances {
                id: b.id,
                journal_id: r.journal_id,
                exercice_id: r.exercice_id,
                canonique: d.canonique.clone(),
                format: d.format,
                empreinte: d.empreinte,
                precedente,
                rang: d.rang,
                valide_par: utilisateur,
                valide_le: instant,
                pieces: roles,
                origine: None,
            },
        )?;
        base::inserer(tx, &inscription).await?;
        precedente = d.empreinte;
        base::supprimer_brouillon(tx, b.id).await?;
        let mut trace = trace_de(dossier, b.id, "validation_par_lot");
        trace.donnees.push(("numero", numero.to_string()));
        trace.donnees.push(("rang", d.rang.to_string()));
        trace.donnees.push(("empreinte", d.empreinte.to_string()));
        trace.donnees.push(("format", d.format.to_string()));
        trace
            .donnees
            .extend(donnee_audit_avertissements(d.validation.avertissements()));
        traces.push(trace);
        vues.push(VueValidation {
            id: b.id.to_string(),
            numero: numero.to_string(),
            rang: d.rang,
            empreinte: d.empreinte.to_string(),
            format: d.format,
            date_validation: interne::date(date_validation),
            avertissements: vues_avertissements(d.validation.avertissements()),
        });
    }
    base::mettre_a_jour_tete(tx, courante.rang, &courante.empreinte).await?;
    for ((journal, exercice), dernier) in compteurs {
        base::mettre_a_jour_compteur(tx, journal, exercice, dernier).await?;
    }
    Ok(Resultat {
        reponse: (
            StatusCode::OK,
            Json(VueLot {
                ecritures: vues,
                exclues_a_revoir: exclues,
            }),
        )
            .into_response(),
        traces,
    })
}
