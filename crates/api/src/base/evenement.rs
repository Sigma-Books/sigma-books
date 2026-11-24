//! Journal d'instance des événements d'authentification (ADR 0017) : table
//! `evenement_authentification`, en ajout seul, non chaînée, sous RLS par utilisateur
//! (migration `0100_secours.sql`).
//!
//! Inscription : par l'utilisateur authentifié ([`inscrire`], [`TransactionUtilisateur`]), ou,
//! à la connexion, pour le compte du courriel saisi ([`super::facteurs::TransactionConnexion`]).
//! Rien n'est inscrit pour un courriel inconnu. Aucun champ ne peut porter un secret : la
//! nature, la méthode et l'issue viennent de listes fermées, l'appareil est la description
//! déclarée par le navigateur (200 caractères au plus) et l'adresse celle du client.

use std::net::IpAddr;

use sqlx::PgConnection;
use time::OffsetDateTime;
use uuid::Uuid;

use super::session::{APPAREIL_MAX, MethodeSession};
use super::{ErreurBase, IdSession, IdUtilisateur, TransactionUtilisateur};

/// Nature d'un événement (liste fermée, contrainte de la table).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NatureEvenement {
    /// Connexion (méthode de secours ou code de récupération).
    Connexion,
    /// Ré-authentification.
    Reauthentification,
    /// Enrôlement (ou remplacement) de la méthode de secours.
    EnrolementSecours,
    /// Nouvelle série de codes de récupération.
    GenerationCodes,
    /// Déconnexion.
    Deconnexion,
    /// Révocation d'une de ses sessions.
    RevocationSession,
    /// Enrôlement d'une passkey (PR 8c).
    EnrolementPasskey,
    /// Suppression d'une passkey (PR 8c).
    SuppressionPasskey,
    /// Compteur de signatures régressif : clonage possible d'une passkey (PR 8c).
    ClonagePasskey,
}

impl NatureEvenement {
    /// Nom enregistré.
    #[must_use]
    pub const fn nom(self) -> &'static str {
        match self {
            Self::Connexion => "connexion",
            Self::Reauthentification => "reauthentification",
            Self::EnrolementSecours => "enrolement_secours",
            Self::GenerationCodes => "generation_codes",
            Self::Deconnexion => "deconnexion",
            Self::RevocationSession => "revocation_session",
            Self::EnrolementPasskey => "enrolement_passkey",
            Self::SuppressionPasskey => "suppression_passkey",
            Self::ClonagePasskey => "clonage_passkey",
        }
    }
}

/// Issue d'un événement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Issue {
    /// Réussite.
    Succes,
    /// Échec (facteur refusé, code déjà utilisé…).
    Echec,
}

impl Issue {
    /// Nom enregistré.
    #[must_use]
    pub const fn nom(self) -> &'static str {
        match self {
            Self::Succes => "succes",
            Self::Echec => "echec",
        }
    }
}

/// Un événement à inscrire.
#[derive(Debug, Clone)]
pub struct NouvelEvenement {
    /// Utilisateur concerné (jamais absent : rien pour un courriel inconnu).
    pub utilisateur: IdUtilisateur,
    /// Session, s'il y en a une.
    pub session: Option<IdSession>,
    /// Nature.
    pub nature: NatureEvenement,
    /// Facteur en jeu.
    pub methode: Option<MethodeSession>,
    /// Issue.
    pub issue: Issue,
    /// Instant.
    pub instant: OffsetDateTime,
    /// Appareil déclaré (tronqué à 200 caractères).
    pub appareil: Option<String>,
    /// Adresse du client.
    pub adresse: Option<IpAddr>,
}

/// Un événement lu par son titulaire.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvenementLu {
    /// Identifiant.
    pub id: Uuid,
    /// Session, s'il y en avait une.
    pub session: Option<IdSession>,
    /// Nature enregistrée.
    pub nature: String,
    /// Méthode enregistrée.
    pub methode: Option<String>,
    /// Issue enregistrée.
    pub issue: String,
    /// Instant.
    pub instant: OffsetDateTime,
    /// Appareil.
    pub appareil: Option<String>,
    /// Adresse.
    pub adresse: Option<String>,
}

/// Insère l'événement, dans la connexion d'une transaction dont le contexte l'admet (RLS).
pub(super) async fn inserer(
    connexion: &mut PgConnection,
    e: &NouvelEvenement,
) -> Result<(), ErreurBase> {
    let appareil = e
        .appareil
        .as_ref()
        .map(|a| a.chars().take(APPAREIL_MAX).collect::<String>());
    let adresse = e.adresse.map(|a| a.to_string());
    sqlx::query!(
        "INSERT INTO evenement_authentification \
             (id, utilisateur_id, session_id, nature, methode, issue, instant, appareil, adresse) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, ($9::text)::inet)",
        Uuid::new_v4(),
        e.utilisateur.uuid(),
        e.session.map(IdSession::uuid),
        e.nature.nom(),
        e.methode.map(MethodeSession::nom),
        e.issue.nom(),
        e.instant,
        appareil,
        adresse,
    )
    .execute(connexion)
    .await?;
    Ok(())
}

/// Inscrit un événement de l'utilisateur du contexte.
///
/// # Errors
/// [`ErreurBase::Donnee`] si l'événement est d'un autre utilisateur, ou erreur de PostgreSQL.
pub async fn inscrire(
    tx: &mut TransactionUtilisateur,
    e: &NouvelEvenement,
) -> Result<(), ErreurBase> {
    if e.utilisateur != tx.utilisateur() {
        return Err(ErreurBase::Donnee("titulaire de l'événement"));
    }
    inserer(tx.connexion(), e).await
}

/// Les `limite` derniers événements de l'utilisateur du contexte, du plus récent au plus
/// ancien.
///
/// # Errors
/// Erreur de PostgreSQL.
pub async fn lister(
    tx: &mut TransactionUtilisateur,
    limite: i64,
) -> Result<Vec<EvenementLu>, ErreurBase> {
    let utilisateur = tx.utilisateur().uuid();
    let lignes = sqlx::query!(
        r#"
        SELECT id, session_id, nature, methode, issue, instant, appareil,
               host(adresse) AS adresse
        FROM evenement_authentification
        WHERE utilisateur_id = $1
        ORDER BY instant DESC, id
        LIMIT $2
        "#,
        utilisateur,
        limite,
    )
    .fetch_all(tx.connexion())
    .await?;
    Ok(lignes
        .into_iter()
        .map(|l| EvenementLu {
            id: l.id,
            session: l.session_id.map(IdSession::depuis_uuid),
            nature: l.nature,
            methode: l.methode,
            issue: l.issue,
            instant: l.instant,
            appareil: l.appareil,
            adresse: l.adresse,
        })
        .collect())
}
