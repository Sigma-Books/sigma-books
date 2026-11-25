//! Administration du cabinet (L2-19, PR 9a ; ADR 0012, 0014, 0023 ; migration
//! `0102_administration.sql`) : membres, rôles, suspension, affectations aux dossiers, sessions
//! d'un membre, séparation des tâches, invitations, réinitialisations.
//!
//! Chaque fonction reçoit la [`TransactionTenant`] du cabinet et filtre par son contexte, en plus
//! de la RLS (isolation à trois niveaux). Un membre d'un autre cabinet est un membre absent.
//! Aucun secret ici : les liens sont dans [`super::liens`], hachés.

use time::OffsetDateTime;
use uuid::Uuid;

use super::session::MotifRevocation;
use super::{ErreurBase, IdCabinet, IdDossier, IdUtilisateur, TransactionTenant};

/// Rôles admis pour un membre (liste fermée de la table `appartenance`).
pub const ROLES: [&str; 4] = ["administrateur", "valideur", "collaborateur", "auditeur"];

/// Un membre du cabinet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Membre {
    /// Utilisateur.
    pub utilisateur: IdUtilisateur,
    /// Nom.
    pub nom: String,
    /// Courriel (visible des seuls administrateurs du cabinet).
    pub courriel: String,
    /// Rôle.
    pub role: String,
    /// Faux si l'appartenance est suspendue.
    pub actif: bool,
    /// Instant de l'appartenance.
    pub cree_le: OffsetDateTime,
}

/// Membres du cabinet du contexte, par nom.
///
/// # Errors
/// Erreur de PostgreSQL.
pub async fn lister_membres(tx: &mut TransactionTenant) -> Result<Vec<Membre>, ErreurBase> {
    let cabinet = tx.contexte().id_cabinet().uuid();
    let lignes = sqlx::query!(
        "SELECT u.id, u.nom, u.courriel, a.role, a.actif, a.cree_le \
         FROM appartenance a JOIN utilisateur u ON u.id = a.utilisateur_id \
         WHERE a.cabinet_id = $1 ORDER BY u.nom, u.id",
        cabinet,
    )
    .fetch_all(tx.connexion())
    .await?;
    Ok(lignes
        .into_iter()
        .map(|l| Membre {
            utilisateur: IdUtilisateur::depuis_uuid(l.id),
            nom: l.nom,
            courriel: l.courriel,
            role: l.role,
            actif: l.actif,
            cree_le: l.cree_le,
        })
        .collect())
}

/// Un membre du cabinet du contexte ; absent s'il est d'un autre cabinet.
///
/// # Errors
/// Erreur de PostgreSQL.
pub async fn lire_membre(
    tx: &mut TransactionTenant,
    utilisateur: IdUtilisateur,
) -> Result<Option<Membre>, ErreurBase> {
    let cabinet = tx.contexte().id_cabinet().uuid();
    let l = sqlx::query!(
        "SELECT u.id, u.nom, u.courriel, a.role, a.actif, a.cree_le \
         FROM appartenance a JOIN utilisateur u ON u.id = a.utilisateur_id \
         WHERE a.cabinet_id = $1 AND a.utilisateur_id = $2",
        cabinet,
        utilisateur.uuid(),
    )
    .fetch_optional(tx.connexion())
    .await?;
    Ok(l.map(|l| Membre {
        utilisateur: IdUtilisateur::depuis_uuid(l.id),
        nom: l.nom,
        courriel: l.courriel,
        role: l.role,
        actif: l.actif,
        cree_le: l.cree_le,
    }))
}

/// Verrouille les appartenances d'administrateur actif du cabinet et les compte : deux retraits
/// concurrents du dernier administrateur se sérialisent.
///
/// # Errors
/// Erreur de PostgreSQL.
pub async fn verrouiller_administrateurs_actifs(
    tx: &mut TransactionTenant,
) -> Result<Vec<IdUtilisateur>, ErreurBase> {
    let cabinet = tx.contexte().id_cabinet().uuid();
    let lignes = sqlx::query!(
        "SELECT utilisateur_id FROM appartenance \
         WHERE cabinet_id = $1 AND role = 'administrateur' AND actif \
         ORDER BY utilisateur_id FOR UPDATE",
        cabinet,
    )
    .fetch_all(tx.connexion())
    .await?;
    Ok(lignes
        .into_iter()
        .map(|l| IdUtilisateur::depuis_uuid(l.utilisateur_id))
        .collect())
}

/// Change le rôle d'un membre. Faux si le membre est absent.
///
/// # Errors
/// Erreur de PostgreSQL (rôle hors liste : contrainte).
pub async fn changer_role(
    tx: &mut TransactionTenant,
    utilisateur: IdUtilisateur,
    role: &str,
) -> Result<bool, ErreurBase> {
    let cabinet = tx.contexte().id_cabinet().uuid();
    let n = sqlx::query!(
        "UPDATE appartenance SET role = $3 WHERE cabinet_id = $1 AND utilisateur_id = $2",
        cabinet,
        utilisateur.uuid(),
        role,
    )
    .execute(tx.connexion())
    .await?
    .rows_affected();
    Ok(n == 1)
}

/// Suspend (`actif = false`) ou rétablit un membre. Faux si le membre est absent.
///
/// # Errors
/// Erreur de PostgreSQL.
pub async fn changer_activite(
    tx: &mut TransactionTenant,
    utilisateur: IdUtilisateur,
    actif: bool,
) -> Result<bool, ErreurBase> {
    let cabinet = tx.contexte().id_cabinet().uuid();
    let n = sqlx::query!(
        "UPDATE appartenance SET actif = $3 WHERE cabinet_id = $1 AND utilisateur_id = $2",
        cabinet,
        utilisateur.uuid(),
        actif,
    )
    .execute(tx.connexion())
    .await?
    .rows_affected();
    Ok(n == 1)
}

/// Affectation d'un membre à un dossier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Affectation {
    /// Dossier.
    pub dossier: IdDossier,
    /// Délégation du droit de valider (Q5.2).
    pub validation_deleguee: bool,
}

/// Affectations d'un membre du cabinet du contexte.
///
/// # Errors
/// Erreur de PostgreSQL.
pub async fn lister_affectations(
    tx: &mut TransactionTenant,
    utilisateur: IdUtilisateur,
) -> Result<Vec<Affectation>, ErreurBase> {
    let cabinet = tx.contexte().id_cabinet().uuid();
    let lignes = sqlx::query!(
        "SELECT dossier_id, validation_deleguee FROM acces_dossier \
         WHERE cabinet_id = $1 AND utilisateur_id = $2 ORDER BY dossier_id",
        cabinet,
        utilisateur.uuid(),
    )
    .fetch_all(tx.connexion())
    .await?;
    Ok(lignes
        .into_iter()
        .map(|l| Affectation {
            dossier: IdDossier::depuis_uuid(l.dossier_id),
            validation_deleguee: l.validation_deleguee,
        })
        .collect())
}

/// Affecte un membre à un dossier du cabinet (ou change sa délégation). Rend l'état précédent
/// (`None` : nouvelle affectation). `Err` si le dossier est absent du cabinet (clé composite).
///
/// # Errors
/// Erreur de PostgreSQL.
pub async fn affecter(
    tx: &mut TransactionTenant,
    utilisateur: IdUtilisateur,
    dossier: IdDossier,
    validation_deleguee: bool,
) -> Result<Option<bool>, ErreurBase> {
    let cabinet = tx.contexte().id_cabinet().uuid();
    let avant = sqlx::query!(
        "SELECT validation_deleguee FROM acces_dossier \
         WHERE cabinet_id = $1 AND dossier_id = $2 AND utilisateur_id = $3 FOR UPDATE",
        cabinet,
        dossier.uuid(),
        utilisateur.uuid(),
    )
    .fetch_optional(tx.connexion())
    .await?
    .map(|l| l.validation_deleguee);
    if avant.is_some() {
        sqlx::query!(
            "UPDATE acces_dossier SET validation_deleguee = $4 \
             WHERE cabinet_id = $1 AND dossier_id = $2 AND utilisateur_id = $3",
            cabinet,
            dossier.uuid(),
            utilisateur.uuid(),
            validation_deleguee,
        )
        .execute(tx.connexion())
        .await?;
    } else {
        sqlx::query!(
            "INSERT INTO acces_dossier (cabinet_id, dossier_id, utilisateur_id, id, \
                                        validation_deleguee) \
             VALUES ($1, $2, $3, $4, $5)",
            cabinet,
            dossier.uuid(),
            utilisateur.uuid(),
            Uuid::new_v4(),
            validation_deleguee,
        )
        .execute(tx.connexion())
        .await?;
    }
    Ok(avant)
}

/// Retire l'affectation d'un membre à un dossier. Rend l'état précédent de la délégation, ou
/// `None` si l'affectation n'existait pas.
///
/// # Errors
/// Erreur de PostgreSQL.
pub async fn retirer_affectation(
    tx: &mut TransactionTenant,
    utilisateur: IdUtilisateur,
    dossier: IdDossier,
) -> Result<Option<bool>, ErreurBase> {
    let cabinet = tx.contexte().id_cabinet().uuid();
    Ok(sqlx::query!(
        "DELETE FROM acces_dossier \
         WHERE cabinet_id = $1 AND dossier_id = $2 AND utilisateur_id = $3 \
         RETURNING validation_deleguee",
        cabinet,
        dossier.uuid(),
        utilisateur.uuid(),
    )
    .fetch_optional(tx.connexion())
    .await?
    .map(|l| l.validation_deleguee))
}

/// Une session d'un membre, telle que la voit l'administrateur : ni jeton, ni empreinte, ni
/// adresse (minimisation).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionMembre {
    /// Identifiant public.
    pub id: Uuid,
    /// Nature (`complete`, `restreinte`, `enrolement`).
    pub nature: String,
    /// Facteur vérifié à l'ouverture.
    pub methode: String,
    /// Ouverture.
    pub cree_le: OffsetDateTime,
    /// Dernière activité.
    pub derniere_activite: OffsetDateTime,
    /// Appareil déclaré.
    pub appareil: Option<String>,
}

/// Sessions non révoquées d'un membre du cabinet du contexte, lues sous la politique
/// `session_administration_lecture` (administrateur actif seulement).
///
/// # Errors
/// Erreur de PostgreSQL.
pub async fn lister_sessions_du_membre(
    tx: &mut TransactionTenant,
    utilisateur: IdUtilisateur,
) -> Result<Vec<SessionMembre>, ErreurBase> {
    let cabinet = tx.contexte().id_cabinet().uuid();
    let lignes = sqlx::query!(
        "SELECT s.id, s.nature, s.methode, s.cree_le, s.derniere_activite, s.appareil \
         FROM session s \
         WHERE s.utilisateur_id = $2 AND s.revoquee_le IS NULL \
           AND EXISTS (SELECT 1 FROM appartenance a \
                       WHERE a.cabinet_id = $1 AND a.utilisateur_id = s.utilisateur_id) \
         ORDER BY s.cree_le DESC, s.id",
        cabinet,
        utilisateur.uuid(),
    )
    .fetch_all(tx.connexion())
    .await?;
    Ok(lignes
        .into_iter()
        .map(|l| SessionMembre {
            id: l.id,
            nature: l.nature,
            methode: l.methode,
            cree_le: l.cree_le,
            derniere_activite: l.derniere_activite,
            appareil: l.appareil,
        })
        .collect())
}

/// Révoque une session d'un membre du cabinet. Faux si elle est absente, d'un autre utilisateur,
/// ou déjà révoquée.
///
/// # Errors
/// Erreur de PostgreSQL.
pub async fn revoquer_session_du_membre(
    tx: &mut TransactionTenant,
    utilisateur: IdUtilisateur,
    session: Uuid,
    instant: OffsetDateTime,
) -> Result<bool, ErreurBase> {
    let cabinet = tx.contexte().id_cabinet().uuid();
    let n = sqlx::query!(
        "UPDATE session SET revoquee_le = $4, motif_revocation = 'revocation_administrateur' \
         WHERE id = $3 AND utilisateur_id = $2 AND revoquee_le IS NULL \
           AND EXISTS (SELECT 1 FROM appartenance a \
                       WHERE a.cabinet_id = $1 AND a.utilisateur_id = session.utilisateur_id)",
        cabinet,
        utilisateur.uuid(),
        session,
        instant,
    )
    .execute(tx.connexion())
    .await?
    .rows_affected();
    Ok(n == 1)
}

/// Révoque toutes les sessions d'un membre du cabinet (changement de droits, suspension : ADR
/// 0012). Rend leur nombre.
///
/// # Errors
/// Erreur de PostgreSQL.
pub async fn revoquer_sessions_du_membre(
    tx: &mut TransactionTenant,
    utilisateur: IdUtilisateur,
    instant: OffsetDateTime,
    motif: MotifRevocation,
) -> Result<u64, ErreurBase> {
    let cabinet = tx.contexte().id_cabinet().uuid();
    Ok(sqlx::query!(
        "UPDATE session SET revoquee_le = $3, motif_revocation = $4 \
         WHERE utilisateur_id = $2 AND revoquee_le IS NULL \
           AND EXISTS (SELECT 1 FROM appartenance a \
                       WHERE a.cabinet_id = $1 AND a.utilisateur_id = session.utilisateur_id)",
        cabinet,
        utilisateur.uuid(),
        instant,
        motif.nom(),
    )
    .execute(tx.connexion())
    .await?
    .rows_affected())
}

/// Active ou désactive la séparation des tâches (Q5.3). Rend l'état précédent.
///
/// # Errors
/// Erreur de PostgreSQL.
pub async fn changer_separation_taches(
    tx: &mut TransactionTenant,
    active: bool,
) -> Result<bool, ErreurBase> {
    let cabinet = tx.contexte().id_cabinet().uuid();
    let avant = sqlx::query!(
        "SELECT separation_taches FROM cabinet WHERE id = $1 FOR UPDATE",
        cabinet
    )
    .fetch_one(tx.connexion())
    .await?
    .separation_taches;
    sqlx::query!(
        "UPDATE cabinet SET separation_taches = $2 WHERE id = $1",
        cabinet,
        active
    )
    .execute(tx.connexion())
    .await?;
    Ok(avant)
}

/// Une invitation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Invitation {
    /// Identifiant.
    pub id: Uuid,
    /// Courriel invité (normalisé en minuscules).
    pub courriel: String,
    /// Nom.
    pub nom: String,
    /// Rôle proposé.
    pub role: String,
    /// État (`en_attente`, `acceptee`, `revoquee`).
    pub etat: String,
    /// Création.
    pub cree_le: OffsetDateTime,
}

/// Données d'une nouvelle invitation.
#[derive(Debug, Clone, Copy)]
pub struct NouvelleInvitation<'a> {
    /// Identifiant tiré par l'appelant.
    pub id: Uuid,
    /// Courriel, déjà normalisé.
    pub courriel: &'a str,
    /// Nom.
    pub nom: &'a str,
    /// Rôle.
    pub role: &'a str,
    /// Administrateur qui invite (`None` : commande de l'instance).
    pub invite_par: Option<IdUtilisateur>,
    /// Instant.
    pub instant: OffsetDateTime,
}

/// Issue de la création d'une invitation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CreationInvitation {
    /// Créée.
    Creee,
    /// Le courriel est déjà celui d'un membre de ce cabinet.
    DejaMembre,
    /// Une invitation est déjà en attente pour ce courriel.
    EnCours,
}

/// Crée une invitation en attente dans le cabinet du contexte.
///
/// # Errors
/// Erreur de PostgreSQL.
pub async fn creer_invitation(
    tx: &mut TransactionTenant,
    nouvelle: &NouvelleInvitation<'_>,
) -> Result<CreationInvitation, ErreurBase> {
    let cabinet = tx.contexte().id_cabinet().uuid();
    let membre = sqlx::query_scalar!(
        "SELECT EXISTS (SELECT 1 FROM appartenance a JOIN utilisateur u ON u.id = a.utilisateur_id \
                        WHERE a.cabinet_id = $1 AND lower(u.courriel) = $2) AS \"existe!\"",
        cabinet,
        nouvelle.courriel,
    )
    .fetch_one(tx.connexion())
    .await?;
    if membre {
        return Ok(CreationInvitation::DejaMembre);
    }
    let n = sqlx::query!(
        "INSERT INTO invitation (cabinet_id, id, courriel, nom, role, invite_par, cree_le, etat) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, 'en_attente') ON CONFLICT DO NOTHING",
        cabinet,
        nouvelle.id,
        nouvelle.courriel,
        nouvelle.nom,
        nouvelle.role,
        nouvelle.invite_par.map(IdUtilisateur::uuid),
        nouvelle.instant,
    )
    .execute(tx.connexion())
    .await?
    .rows_affected();
    Ok(if n == 1 {
        CreationInvitation::Creee
    } else {
        CreationInvitation::EnCours
    })
}

fn invitation_lue(
    id: Uuid,
    courriel: String,
    nom: String,
    role: String,
    etat: String,
    cree_le: OffsetDateTime,
) -> Invitation {
    Invitation {
        id,
        courriel,
        nom,
        role,
        etat,
        cree_le,
    }
}

/// Invitations en attente du cabinet du contexte, de la plus récente.
///
/// # Errors
/// Erreur de PostgreSQL.
pub async fn lister_invitations(tx: &mut TransactionTenant) -> Result<Vec<Invitation>, ErreurBase> {
    let cabinet = tx.contexte().id_cabinet().uuid();
    Ok(sqlx::query!(
        "SELECT id, courriel, nom, role, etat, cree_le FROM invitation \
         WHERE cabinet_id = $1 AND etat = 'en_attente' ORDER BY cree_le DESC, id",
        cabinet,
    )
    .fetch_all(tx.connexion())
    .await?
    .into_iter()
    .map(|l| invitation_lue(l.id, l.courriel, l.nom, l.role, l.etat, l.cree_le))
    .collect())
}

/// Une invitation du cabinet du contexte (verrouillée), quel que soit son état.
///
/// # Errors
/// Erreur de PostgreSQL.
pub async fn lire_invitation(
    tx: &mut TransactionTenant,
    id: Uuid,
) -> Result<Option<Invitation>, ErreurBase> {
    let cabinet = tx.contexte().id_cabinet().uuid();
    Ok(sqlx::query!(
        "SELECT id, courriel, nom, role, etat, cree_le FROM invitation \
         WHERE cabinet_id = $1 AND id = $2 FOR UPDATE",
        cabinet,
        id,
    )
    .fetch_optional(tx.connexion())
    .await?
    .map(|l| invitation_lue(l.id, l.courriel, l.nom, l.role, l.etat, l.cree_le)))
}

/// Révoque une invitation en attente. Faux si elle ne l'était pas.
///
/// # Errors
/// Erreur de PostgreSQL.
pub async fn revoquer_invitation(
    tx: &mut TransactionTenant,
    id: Uuid,
    instant: OffsetDateTime,
) -> Result<bool, ErreurBase> {
    let cabinet = tx.contexte().id_cabinet().uuid();
    let n = sqlx::query!(
        "UPDATE invitation SET etat = 'revoquee', traitee_le = $3 \
         WHERE cabinet_id = $1 AND id = $2 AND etat = 'en_attente'",
        cabinet,
        id,
        instant,
    )
    .execute(tx.connexion())
    .await?
    .rows_affected();
    Ok(n == 1)
}

/// Une réinitialisation d'accès.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reinitialisation {
    /// Identifiant.
    pub id: Uuid,
    /// Membre concerné.
    pub utilisateur: IdUtilisateur,
    /// Demandeur (`None` : procédure de support de l'instance).
    pub demandee_par: Option<IdUtilisateur>,
    /// Motif.
    pub motif: String,
    /// Création.
    pub cree_le: OffsetDateTime,
    /// Fin du délai de 24 heures.
    pub echeance: OffsetDateTime,
    /// État (`en_attente`, `annulee`, `aboutie`, `expiree`).
    pub etat: String,
}

/// Délai de contestation d'une réinitialisation (ADR 0012).
pub const DELAI_REINITIALISATION: time::Duration = time::Duration::hours(24);

/// Issue de la création d'une réinitialisation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CreationReinitialisation {
    /// Créée.
    Creee,
    /// Une réinitialisation est déjà en attente pour ce membre.
    EnCours,
}

/// Crée une réinitialisation en attente pour un membre du cabinet du contexte. La vérification
/// hors bande est déclarée (contrainte de la table : toujours vraie).
///
/// # Errors
/// Erreur de PostgreSQL (membre d'un autre cabinet : clé étrangère).
pub async fn creer_reinitialisation(
    tx: &mut TransactionTenant,
    id: Uuid,
    utilisateur: IdUtilisateur,
    demandee_par: Option<IdUtilisateur>,
    motif: &str,
    instant: OffsetDateTime,
) -> Result<CreationReinitialisation, ErreurBase> {
    let cabinet = tx.contexte().id_cabinet().uuid();
    let n = sqlx::query!(
        "INSERT INTO reinitialisation (cabinet_id, id, utilisateur_id, demandee_par, motif, \
                                       verification_hors_bande, cree_le, echeance, etat) \
         VALUES ($1, $2, $3, $4, $5, true, $6, $7, 'en_attente') ON CONFLICT DO NOTHING",
        cabinet,
        id,
        utilisateur.uuid(),
        demandee_par.map(IdUtilisateur::uuid),
        motif,
        instant,
        instant + DELAI_REINITIALISATION,
    )
    .execute(tx.connexion())
    .await?
    .rows_affected();
    Ok(if n == 1 {
        CreationReinitialisation::Creee
    } else {
        CreationReinitialisation::EnCours
    })
}

fn reinitialisation_lue(
    id: Uuid,
    utilisateur_id: Uuid,
    demandee_par: Option<Uuid>,
    motif: String,
    cree_le: OffsetDateTime,
    echeance: OffsetDateTime,
    etat: String,
) -> Reinitialisation {
    Reinitialisation {
        id,
        utilisateur: IdUtilisateur::depuis_uuid(utilisateur_id),
        demandee_par: demandee_par.map(IdUtilisateur::depuis_uuid),
        motif,
        cree_le,
        echeance,
        etat,
    }
}

/// Réinitialisations en attente du cabinet du contexte.
///
/// # Errors
/// Erreur de PostgreSQL.
pub async fn lister_reinitialisations(
    tx: &mut TransactionTenant,
) -> Result<Vec<Reinitialisation>, ErreurBase> {
    let cabinet = tx.contexte().id_cabinet().uuid();
    Ok(sqlx::query!(
        "SELECT id, utilisateur_id, demandee_par, motif, cree_le, echeance, etat \
         FROM reinitialisation WHERE cabinet_id = $1 AND etat = 'en_attente' \
         ORDER BY cree_le DESC, id",
        cabinet,
    )
    .fetch_all(tx.connexion())
    .await?
    .into_iter()
    .map(|l| {
        reinitialisation_lue(
            l.id,
            l.utilisateur_id,
            l.demandee_par,
            l.motif,
            l.cree_le,
            l.echeance,
            l.etat,
        )
    })
    .collect())
}

/// Une réinitialisation du cabinet du contexte (verrouillée).
///
/// # Errors
/// Erreur de PostgreSQL.
pub async fn lire_reinitialisation(
    tx: &mut TransactionTenant,
    id: Uuid,
) -> Result<Option<Reinitialisation>, ErreurBase> {
    let cabinet = tx.contexte().id_cabinet().uuid();
    Ok(sqlx::query!(
        "SELECT id, utilisateur_id, demandee_par, motif, cree_le, echeance, etat \
         FROM reinitialisation WHERE cabinet_id = $1 AND id = $2 FOR UPDATE",
        cabinet,
        id,
    )
    .fetch_optional(tx.connexion())
    .await?
    .map(|l| {
        reinitialisation_lue(
            l.id,
            l.utilisateur_id,
            l.demandee_par,
            l.motif,
            l.cree_le,
            l.echeance,
            l.etat,
        )
    }))
}

/// Annule une réinitialisation en attente. Faux si elle ne l'était pas.
///
/// # Errors
/// Erreur de PostgreSQL.
pub async fn annuler_reinitialisation(
    tx: &mut TransactionTenant,
    id: Uuid,
    instant: OffsetDateTime,
) -> Result<bool, ErreurBase> {
    let cabinet = tx.contexte().id_cabinet().uuid();
    let n = sqlx::query!(
        "UPDATE reinitialisation SET etat = 'annulee', traitee_le = $3 \
         WHERE cabinet_id = $1 AND id = $2 AND etat = 'en_attente'",
        cabinet,
        id,
        instant,
    )
    .execute(tx.connexion())
    .await?
    .rows_affected();
    Ok(n == 1)
}

/// Cabinet du contexte (aide pour les traces).
#[must_use]
pub fn cabinet_du_contexte(tx: &TransactionTenant) -> IdCabinet {
    tx.contexte().id_cabinet()
}
