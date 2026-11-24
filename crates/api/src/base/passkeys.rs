//! Passkeys, WebAuthn (ADR 0012 ; L2 PR 8c) : tables d'instance `facteur_passkey` et
//! `defi_webauthn`, sous RLS (migration `0101_passkeys.sql`).
//!
//! # Rien de secret
//! `facteur_passkey.cle` est la `Passkey` de `webauthn-rs` sérialisée : clé **publique**,
//! identifiant du credential, compteur de signatures, indicateurs de sauvegarde. La clé privée
//! reste dans l'authentificateur. `defi_webauthn.etat` est l'état d'une cérémonie (défi),
//! retrouvé par l'empreinte SHA-256 du jeton du cookie, jamais par le jeton.
//!
//! # Connexion
//! Comme la méthode de secours (module parent [`super::facteurs`]), la connexion par passkey
//! s'exécute dans une [`TransactionConnexion`] : `sigma.courriel_connexion` posé, jamais
//! d'utilisateur. Sous RLS, elle ne voit que les passkeys et les défis du compte du courriel
//! saisi (ses défis de connexion seulement), et ne peut modifier d'une passkey que son état et sa
//! dernière utilisation
//! (déclencheur de la migration). Elle ne devient une transaction d'utilisateur que sur
//! présentation de la preuve rendue par [`TransactionConnexion::consommer_passkey`], qui
//! fabrique le facteur vérifié **après** la mise à jour conditionnelle de l'état.
//!
//! # Anti-rejeu
//! - Le défi est **à usage unique** : `DELETE … WHERE jeton_hache = … RETURNING` (atomique). Deux
//!   requêtes qui présentent la même réponse : la seconde ne trouve plus rien.
//! - L'état de la passkey (compteur) est remplacé seulement si la ligne est encore celle qu'on a
//!   lue (`WHERE cle = <ancien état>`) : deux connexions concurrentes ne se superposent pas.

use time::OffsetDateTime;
use uuid::Uuid;
use webauthn_rs::prelude::Passkey;

use super::facteurs::{FacteurVerifie, TransactionConnexion};
use super::{ErreurBase, IdSession, IdUtilisateur, TransactionUtilisateur};

/// Une passkey lue en base.
#[derive(Debug, Clone)]
pub struct PasskeyLue {
    /// Identifiant de la ligne (celui que cite l'interface).
    pub id: Uuid,
    /// Utilisateur titulaire.
    pub utilisateur: IdUtilisateur,
    /// La passkey (clé publique, compteur).
    pub passkey: Passkey,
    /// L'état tel que lu (JSON), pour la mise à jour conditionnelle.
    pub etat_lu: String,
    /// Libellé choisi par l'utilisateur.
    pub nom: String,
    /// Création.
    pub cree_le: OffsetDateTime,
    /// Dernière connexion par cette passkey.
    pub derniere_utilisation: Option<OffsetDateTime>,
}

/// Nature d'une cérémonie (contrainte de la table).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NatureDefi {
    /// Connexion.
    Connexion,
    /// Ré-authentification d'une session.
    Reauthentification,
    /// Enrôlement d'une passkey.
    Enrolement,
}

impl NatureDefi {
    /// Nom enregistré.
    #[must_use]
    pub const fn nom(self) -> &'static str {
        match self {
            Self::Connexion => "connexion",
            Self::Reauthentification => "reauthentification",
            Self::Enrolement => "enrolement",
        }
    }
}

/// Une cérémonie à enregistrer.
#[derive(Debug)]
pub struct NouveauDefi {
    /// Utilisateur qui l'accomplira.
    pub utilisateur: IdUtilisateur,
    /// Nature.
    pub nature: NatureDefi,
    /// Session de la requête (enrôlement, ré-authentification) ; `None` pour une connexion.
    pub session: Option<IdSession>,
    /// Empreinte SHA-256 du jeton du cookie.
    pub jeton_hache: [u8; 32],
    /// État de la cérémonie, sérialisé en JSON.
    pub etat: String,
    /// Création.
    pub instant: OffsetDateTime,
    /// Échéance.
    pub expire_le: OffsetDateTime,
}

/// Une cérémonie retrouvée, et consommée.
#[derive(Debug)]
pub struct DefiLu {
    /// Utilisateur de la cérémonie.
    pub utilisateur: IdUtilisateur,
    /// État sérialisé.
    pub etat: String,
}

fn lue(
    id: Uuid,
    utilisateur: Uuid,
    cle: String,
    nom: String,
    cree_le: OffsetDateTime,
    derniere_utilisation: Option<OffsetDateTime>,
) -> Result<PasskeyLue, ErreurBase> {
    let passkey = serde_json::from_str::<Passkey>(&cle)
        .map_err(|_| ErreurBase::Donnee("passkey illisible"))?;
    Ok(PasskeyLue {
        id,
        utilisateur: IdUtilisateur::depuis_uuid(utilisateur),
        passkey,
        etat_lu: cle,
        nom,
        cree_le,
        derniere_utilisation,
    })
}

/// Sérialise une passkey pour la base.
///
/// # Errors
/// [`ErreurBase::Donnee`] si la sérialisation échoue (jamais en pratique).
pub fn serialiser(passkey: &Passkey) -> Result<String, ErreurBase> {
    serde_json::to_string(passkey).map_err(|_| ErreurBase::Donnee("passkey non sérialisable"))
}

// --- Connexion -------------------------------------------------------------------------------

impl TransactionConnexion {
    /// Les passkeys du compte du courriel saisi (la RLS n'en laisse voir aucune autre).
    ///
    /// # Errors
    /// Erreur de PostgreSQL, ou donnée incohérente.
    pub async fn lire_passkeys(&mut self) -> Result<Vec<PasskeyLue>, ErreurBase> {
        let (courriel, connexion) = self.contexte();
        let lignes = sqlx::query!(
            "SELECT p.id, p.utilisateur_id, p.cle::text AS \"cle!\", p.nom, p.cree_le, \
                    p.derniere_utilisation \
             FROM facteur_passkey p JOIN utilisateur u ON u.id = p.utilisateur_id \
             WHERE lower(u.courriel) = lower($1) ORDER BY p.cree_le, p.id",
            courriel,
        )
        .fetch_all(connexion)
        .await?;
        lignes
            .into_iter()
            .map(|l| {
                lue(
                    l.id,
                    l.utilisateur_id,
                    l.cle,
                    l.nom,
                    l.cree_le,
                    l.derniere_utilisation,
                )
            })
            .collect()
    }

    /// Enregistre une cérémonie de connexion pour le compte du courriel saisi, après avoir
    /// élagué les défis échus de ce compte et ne gardé que les `conserves - 1` plus récents :
    /// le stockage reste borné.
    ///
    /// # Errors
    /// Erreur de PostgreSQL (dont le refus de la RLS pour une autre nature ou un autre compte).
    pub async fn creer_defi(
        &mut self,
        defi: &NouveauDefi,
        conserves: i64,
    ) -> Result<(), ErreurBase> {
        let (_, connexion) = self.contexte();
        elaguer_et_inserer(connexion, defi, conserves).await
    }

    /// Retrouve **et supprime** la cérémonie de connexion de jeton d'empreinte `jeton_hache`,
    /// si elle n'est pas échue : à usage unique.
    ///
    /// # Errors
    /// Erreur de PostgreSQL.
    pub async fn consommer_defi(
        &mut self,
        jeton_hache: &[u8; 32],
        instant: OffsetDateTime,
    ) -> Result<Option<DefiLu>, ErreurBase> {
        let (_, connexion) = self.contexte();
        let ligne = sqlx::query!(
            "DELETE FROM defi_webauthn \
             WHERE jeton_hache = $1 AND nature = 'connexion' AND expire_le > $2 \
             RETURNING utilisateur_id, etat::text AS \"etat!\"",
            jeton_hache.as_slice(),
            instant,
        )
        .fetch_optional(&mut *connexion)
        .await?;
        Ok(ligne.map(|l| DefiLu {
            utilisateur: IdUtilisateur::depuis_uuid(l.utilisateur_id),
            etat: l.etat,
        }))
    }

    /// Remplace l'état de la passkey `id` de `utilisateur` par `nouvel_etat` (compteur) et date
    /// son utilisation, **si** elle est encore dans l'état `etat_lu`. Rend la preuve du facteur
    /// vérifié, ou `None` (état changé entre-temps : réponse rejouée ou connexions
    /// concurrentes).
    ///
    /// # Errors
    /// Erreur de PostgreSQL.
    pub async fn consommer_passkey(
        &mut self,
        utilisateur: IdUtilisateur,
        id: Uuid,
        etat_lu: &str,
        nouvel_etat: &str,
        instant: OffsetDateTime,
    ) -> Result<Option<FacteurVerifie>, ErreurBase> {
        let (_, connexion) = self.contexte();
        let touchee =
            remplacer_etat(connexion, utilisateur, id, etat_lu, nouvel_etat, instant).await?;
        Ok(touchee.then(|| FacteurVerifie::passkey(utilisateur)))
    }
}

async fn elaguer_et_inserer(
    connexion: &mut sqlx::PgConnection,
    defi: &NouveauDefi,
    conserves: i64,
) -> Result<(), ErreurBase> {
    // Échus, puis tous les anciens au-delà des `conserves - 1` plus récents de la même
    // cérémonie (même utilisateur, même nature, même session) : le nouveau fait le compte.
    sqlx::query!(
        "DELETE FROM defi_webauthn WHERE utilisateur_id = $1 AND expire_le <= $2",
        defi.utilisateur.uuid(),
        defi.instant,
    )
    .execute(&mut *connexion)
    .await?;
    sqlx::query!(
        "DELETE FROM defi_webauthn WHERE id IN ( \
             SELECT id FROM defi_webauthn \
             WHERE utilisateur_id = $1 AND nature = $2 \
               AND session_id IS NOT DISTINCT FROM $3 \
             ORDER BY cree_le DESC, id OFFSET $4)",
        defi.utilisateur.uuid(),
        defi.nature.nom(),
        defi.session.map(|s| s.uuid()),
        (conserves - 1).max(0),
    )
    .execute(&mut *connexion)
    .await?;
    sqlx::query!(
        "INSERT INTO defi_webauthn \
             (id, jeton_hache, utilisateur_id, nature, session_id, etat, cree_le, expire_le) \
         VALUES ($1, $2, $3, $4, $5, $6::text::jsonb, $7, $8)",
        Uuid::new_v4(),
        defi.jeton_hache.as_slice(),
        defi.utilisateur.uuid(),
        defi.nature.nom(),
        defi.session.map(|s| s.uuid()),
        defi.etat,
        defi.instant,
        defi.expire_le,
    )
    .execute(&mut *connexion)
    .await?;
    Ok(())
}

async fn remplacer_etat(
    connexion: &mut sqlx::PgConnection,
    utilisateur: IdUtilisateur,
    id: Uuid,
    etat_lu: &str,
    nouvel_etat: &str,
    instant: OffsetDateTime,
) -> Result<bool, ErreurBase> {
    let touchee = sqlx::query_scalar!(
        "UPDATE facteur_passkey SET cle = $4::text::jsonb, derniere_utilisation = $5 \
         WHERE id = $1 AND utilisateur_id = $2 AND cle = $3::text::jsonb \
         RETURNING id",
        id,
        utilisateur.uuid(),
        etat_lu,
        nouvel_etat,
        instant,
    )
    .fetch_optional(connexion)
    .await?;
    Ok(touchee.is_some())
}

// --- Utilisateur authentifié -----------------------------------------------------------------

/// Les passkeys de l'utilisateur du contexte, de la plus ancienne à la plus récente.
///
/// # Errors
/// Erreur de PostgreSQL, ou donnée incohérente.
pub async fn lister(tx: &mut TransactionUtilisateur) -> Result<Vec<PasskeyLue>, ErreurBase> {
    let utilisateur = tx.utilisateur().uuid();
    let lignes = sqlx::query!(
        "SELECT id, utilisateur_id, cle::text AS \"cle!\", nom, cree_le, derniere_utilisation \
         FROM facteur_passkey WHERE utilisateur_id = $1 ORDER BY cree_le, id",
        utilisateur,
    )
    .fetch_all(tx.connexion())
    .await?;
    lignes
        .into_iter()
        .map(|l| {
            lue(
                l.id,
                l.utilisateur_id,
                l.cle,
                l.nom,
                l.cree_le,
                l.derniere_utilisation,
            )
        })
        .collect()
}

/// Enregistre une passkey de l'utilisateur du contexte. Faux si son identifiant de credential
/// est déjà enregistré (pour ce compte ou un autre : WebAuthn l'interdit).
///
/// # Errors
/// Erreur de PostgreSQL, ou donnée incohérente.
pub async fn inserer(
    tx: &mut TransactionUtilisateur,
    passkey: &Passkey,
    nom: &str,
    instant: OffsetDateTime,
) -> Result<Option<Uuid>, ErreurBase> {
    let utilisateur = tx.utilisateur().uuid();
    let etat = serialiser(passkey)?;
    let identifiant: Vec<u8> = passkey.cred_id().to_vec();
    let id = Uuid::new_v4();
    let cree = sqlx::query_scalar!(
        "INSERT INTO facteur_passkey \
             (id, utilisateur_id, identifiant_credential, cle, nom, cree_le) \
         VALUES ($1, $2, $3, $4::text::jsonb, $5, $6) \
         ON CONFLICT (identifiant_credential) DO NOTHING RETURNING id",
        id,
        utilisateur,
        identifiant.as_slice(),
        etat,
        nom,
        instant,
    )
    .fetch_optional(tx.connexion())
    .await?;
    Ok(cree)
}

/// Supprime la passkey `id` de l'utilisateur du contexte. Faux si elle n'existe pas (ou est à un
/// autre : la RLS ne la montre pas).
///
/// # Errors
/// Erreur de PostgreSQL.
pub async fn supprimer(tx: &mut TransactionUtilisateur, id: Uuid) -> Result<bool, ErreurBase> {
    let utilisateur = tx.utilisateur().uuid();
    let touchees = sqlx::query!(
        "DELETE FROM facteur_passkey WHERE id = $1 AND utilisateur_id = $2",
        id,
        utilisateur,
    )
    .execute(tx.connexion())
    .await?;
    Ok(touchees.rows_affected() == 1)
}

/// Enregistre une cérémonie de l'utilisateur du contexte (enrôlement, ré-authentification),
/// après avoir élagué ses défis échus et remplacé son défi précédent de même nature et même
/// session.
///
/// # Errors
/// Erreur de PostgreSQL.
pub async fn creer_defi(
    tx: &mut TransactionUtilisateur,
    defi: &NouveauDefi,
) -> Result<(), ErreurBase> {
    if defi.utilisateur != tx.utilisateur() {
        return Err(ErreurBase::Donnee("titulaire du défi"));
    }
    elaguer_et_inserer(tx.connexion(), defi, 1).await
}

/// Retrouve **et supprime** la cérémonie de l'utilisateur du contexte, de la nature et de la
/// session données, d'empreinte `jeton_hache`, si elle n'est pas échue.
///
/// # Errors
/// Erreur de PostgreSQL.
pub async fn consommer_defi(
    tx: &mut TransactionUtilisateur,
    nature: NatureDefi,
    session: IdSession,
    jeton_hache: &[u8; 32],
    instant: OffsetDateTime,
) -> Result<Option<String>, ErreurBase> {
    let utilisateur = tx.utilisateur().uuid();
    let etat = sqlx::query_scalar!(
        "DELETE FROM defi_webauthn \
         WHERE jeton_hache = $1 AND utilisateur_id = $2 AND nature = $3 \
           AND session_id = $4 AND expire_le > $5 \
         RETURNING etat::text AS \"etat!\"",
        jeton_hache.as_slice(),
        utilisateur,
        nature.nom(),
        session.uuid(),
        instant,
    )
    .fetch_optional(tx.connexion())
    .await?;
    Ok(etat)
}

/// Ré-authentification : remplace l'état de la passkey `id` de l'utilisateur du contexte (même
/// condition que [`TransactionConnexion::consommer_passkey`]). Faux si l'état a changé.
///
/// # Errors
/// Erreur de PostgreSQL.
pub async fn consommer_passkey(
    tx: &mut TransactionUtilisateur,
    id: Uuid,
    etat_lu: &str,
    nouvel_etat: &str,
    instant: OffsetDateTime,
) -> Result<bool, ErreurBase> {
    let utilisateur = tx.utilisateur();
    remplacer_etat(
        tx.connexion(),
        utilisateur,
        id,
        etat_lu,
        nouvel_etat,
        instant,
    )
    .await
}
