//! Méthode de secours et codes de récupération (ADR 0012 ; L2 PR 8b) : tables d'instance
//! `facteur_secours` et `code_recuperation`, sous RLS (migration `0100_secours.sql`).
//!
//! # Connexion : [`TransactionConnexion`]
//! Une connexion n'a pas encore d'utilisateur authentifié. Sa transaction ne pose **jamais**
//! `sigma.utilisateur` (doc du module parent, « `sigma.utilisateur` ») mais
//! `sigma.courriel_connexion`, le courriel saisi : sous RLS, elle ne voit que la ligne
//! `utilisateur` de ce courriel, son secours et ses codes, et ne peut qu'y consommer un pas TOTP
//! ou un code, et inscrire un événement. Type distinct de [`TransactionUtilisateur`] : il
//! n'expose que ces requêtes.
//!
//! Elle ne devient une [`TransactionUtilisateur`] ([`TransactionConnexion::authentifier`]) que
//! sur présentation d'un [`FacteurVerifie`], que seules les consommations atomiques d'un pas
//! TOTP ou d'un code fabriquent (champs privés). L'ouverture de la session, l'événement et
//! l'audit des cabinets suivent dans la **même transaction** : un code consommé sans session
//! ouverte est impossible, et inversement.
//!
//! # Anti-rejeu
//! - TOTP : `UPDATE … SET totp_dernier_pas = $pas WHERE … AND totp_dernier_pas < $pas`, et
//!   `mot_de_passe` inchangé depuis sa lecture. Deux transactions concurrentes sur le même pas :
//!   la seconde attend la première, relit la ligne (`READ COMMITTED`), ne la touche plus, et
//!   échoue.
//! - Code de récupération : `UPDATE … SET utilise_le = $instant WHERE … AND utilise_le IS
//!   NULL RETURNING id` : usage unique, atomique.

use integrite::audit::Evenement;
use sqlx::{PgConnection, Postgres, Transaction};
use time::OffsetDateTime;
use uuid::Uuid;

use super::audit::{PorteeAudit, inscrire as inscrire_audit};
use super::evenement::{NouvelEvenement, inserer};
use super::session::MethodeSession;
use super::{
    ContexteTenant, ErreurBase, IdAppartenance, IdCabinet, IdUtilisateur, TransactionTenant,
    TransactionUtilisateur,
};

/// Preuve qu'un facteur vient d'être vérifié et consommé (pas TOTP, code de récupération) pour
/// un utilisateur. Champs privés : seules les fonctions de ce module la fabriquent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FacteurVerifie {
    utilisateur: IdUtilisateur,
    methode: MethodeSession,
}

impl FacteurVerifie {
    /// L'utilisateur dont le facteur est vérifié.
    #[must_use]
    pub const fn utilisateur(&self) -> IdUtilisateur {
        self.utilisateur
    }

    /// Le facteur vérifié.
    #[must_use]
    pub const fn methode(&self) -> MethodeSession {
        self.methode
    }
}

/// Méthode de secours lue en base.
#[derive(Clone, PartialEq, Eq)]
pub struct SecoursLu {
    /// Titulaire.
    pub utilisateur: IdUtilisateur,
    /// Chaîne PHC du mot de passe.
    pub mot_de_passe: String,
    /// Secret TOTP chiffré (nonce, chiffré, étiquette).
    pub totp_chiffre: Vec<u8>,
    /// Identifiant de la clé qui l'a chiffré.
    pub totp_cle_id: Vec<u8>,
    /// Dernier pas TOTP accepté.
    pub totp_dernier_pas: u64,
    /// Dernier enrôlement.
    pub modifie_le: OffsetDateTime,
}

impl std::fmt::Debug for SecoursLu {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SecoursLu")
            .field("utilisateur", &self.utilisateur)
            .field("totp_dernier_pas", &self.totp_dernier_pas)
            .finish_non_exhaustive()
    }
}

fn pas_base(pas: u64) -> Result<i64, ErreurBase> {
    i64::try_from(pas).map_err(|_| ErreurBase::Donnee("pas TOTP"))
}

fn pas_lu(pas: i64) -> Result<u64, ErreurBase> {
    u64::try_from(pas).map_err(|_| ErreurBase::Donnee("pas TOTP"))
}

/// Transaction d'une connexion : contexte du seul courriel saisi (`sigma.courriel_connexion`),
/// jamais d'utilisateur. Abandonnée sans [`TransactionConnexion::confirmer`] ni
/// [`TransactionConnexion::authentifier`], elle est annulée.
#[derive(Debug)]
pub struct TransactionConnexion {
    tx: Transaction<'static, Postgres>,
    courriel: String,
}

/// Ouvre une transaction de connexion pour le courriel saisi : cabinet, dossier, jeton de
/// session et utilisateur vides ; `sigma.courriel_connexion` posé.
///
/// # Errors
/// Connexion indisponible ou erreur de PostgreSQL.
pub async fn ouvrir_transaction_connexion(
    pool: &sqlx::PgPool,
    courriel: &str,
) -> Result<TransactionConnexion, ErreurBase> {
    let mut tx = pool.begin().await?;
    sqlx::query!(
        "SELECT set_config('sigma.cabinet', '', true) AS cabinet, \
                set_config('sigma.dossier', '', true) AS dossier, \
                set_config('sigma.jeton_session', '', true) AS jeton, \
                set_config('sigma.utilisateur', '', true) AS utilisateur, \
                set_config('sigma.courriel_connexion', $1, true) AS courriel",
        courriel,
    )
    .fetch_one(&mut *tx)
    .await?;
    Ok(TransactionConnexion {
        tx,
        courriel: courriel.to_owned(),
    })
}

impl TransactionConnexion {
    /// L'utilisateur du courriel saisi, s'il existe.
    ///
    /// # Errors
    /// Erreur de PostgreSQL.
    pub async fn lire_utilisateur(&mut self) -> Result<Option<IdUtilisateur>, ErreurBase> {
        let id = sqlx::query_scalar!(
            "SELECT id FROM utilisateur WHERE lower(courriel) = lower($1)",
            self.courriel,
        )
        .fetch_optional(&mut *self.tx)
        .await?;
        Ok(id.map(IdUtilisateur::depuis_uuid))
    }

    /// La méthode de secours du compte du courriel saisi, s'il existe et en a une.
    ///
    /// # Errors
    /// Erreur de PostgreSQL, ou donnée incohérente.
    pub async fn lire_secours(&mut self) -> Result<Option<SecoursLu>, ErreurBase> {
        let ligne = sqlx::query!(
            "SELECT s.utilisateur_id, s.mot_de_passe, s.totp_chiffre, s.totp_cle_id, \
                    s.totp_dernier_pas, s.modifie_le \
             FROM facteur_secours s JOIN utilisateur u ON u.id = s.utilisateur_id \
             WHERE lower(u.courriel) = lower($1)",
            self.courriel,
        )
        .fetch_optional(&mut *self.tx)
        .await?;
        ligne
            .map(|l| {
                Ok(SecoursLu {
                    utilisateur: IdUtilisateur::depuis_uuid(l.utilisateur_id),
                    mot_de_passe: l.mot_de_passe,
                    totp_chiffre: l.totp_chiffre,
                    totp_cle_id: l.totp_cle_id,
                    totp_dernier_pas: pas_lu(l.totp_dernier_pas)?,
                    modifie_le: l.modifie_le,
                })
            })
            .transpose()
    }

    /// Consomme le pas TOTP `pas` du secours de `utilisateur`, si le pas dépasse le dernier
    /// accepté et que le mot de passe vérifié (`mot_de_passe_lu`) est toujours celui en base.
    /// Rend la preuve du facteur vérifié, ou `None` (code rejoué, secours changé entre-temps).
    ///
    /// # Errors
    /// Erreur de PostgreSQL.
    pub async fn consommer_pas_totp(
        &mut self,
        utilisateur: IdUtilisateur,
        mot_de_passe_lu: &str,
        pas: u64,
    ) -> Result<Option<FacteurVerifie>, ErreurBase> {
        let touchees =
            consommer_pas(&mut self.tx, utilisateur, mot_de_passe_lu, pas_base(pas)?).await?;
        Ok((touchees == 1).then_some(FacteurVerifie {
            utilisateur,
            methode: MethodeSession::Secours,
        }))
    }

    /// Consomme le code de récupération d'empreinte `empreinte` de `utilisateur`, s'il n'a
    /// pas encore servi. Rend la preuve du facteur vérifié, ou `None`.
    ///
    /// # Errors
    /// Erreur de PostgreSQL.
    pub async fn consommer_code(
        &mut self,
        utilisateur: IdUtilisateur,
        empreinte: &[u8; 32],
        instant: OffsetDateTime,
    ) -> Result<Option<FacteurVerifie>, ErreurBase> {
        let id = sqlx::query_scalar!(
            "UPDATE code_recuperation SET utilise_le = $3 \
             WHERE utilisateur_id = $1 AND empreinte = $2 AND utilise_le IS NULL \
             RETURNING id",
            utilisateur.uuid(),
            empreinte.as_slice(),
            instant,
        )
        .fetch_optional(&mut *self.tx)
        .await?;
        Ok(id.map(|_| FacteurVerifie {
            utilisateur,
            methode: MethodeSession::CodeRecuperation,
        }))
    }

    /// Nombre d'échecs de connexion inscrits pour `utilisateur` depuis `depuis` (plafond
    /// persistant par compte ; la RLS ne laisse lire à la connexion que ces échecs).
    ///
    /// # Errors
    /// Erreur de PostgreSQL.
    pub async fn compter_echecs(
        &mut self,
        utilisateur: IdUtilisateur,
        depuis: OffsetDateTime,
    ) -> Result<i64, ErreurBase> {
        let n = sqlx::query_scalar!(
            r#"SELECT count(*) AS "n!" FROM evenement_authentification
               WHERE utilisateur_id = $1 AND nature = 'connexion' AND issue = 'echec'
                 AND instant >= $2"#,
            utilisateur.uuid(),
            depuis,
        )
        .fetch_one(&mut *self.tx)
        .await?;
        Ok(n)
    }

    /// Inscrit l'échec d'une connexion pour le compte du courriel saisi (la RLS refuse tout
    /// autre compte, toute autre nature, une réussite et une session).
    ///
    /// # Errors
    /// Erreur de PostgreSQL (dont le refus de la RLS).
    pub async fn inscrire_evenement(&mut self, e: &NouvelEvenement) -> Result<(), ErreurBase> {
        inserer(&mut self.tx, e).await
    }

    /// Valide la transaction (`COMMIT`).
    ///
    /// # Errors
    /// Erreur de PostgreSQL à la validation.
    pub async fn confirmer(self) -> Result<(), ErreurBase> {
        self.tx.commit().await?;
        Ok(())
    }

    /// Fait de cette transaction celle de l'utilisateur dont le facteur vient d'être vérifié :
    /// `sigma.utilisateur` posé, `sigma.courriel_connexion` effacé. La suite (session,
    /// événement, audit) s'exécute dans la même transaction.
    ///
    /// # Errors
    /// Erreur de PostgreSQL.
    pub async fn authentifier(
        mut self,
        preuve: FacteurVerifie,
    ) -> Result<TransactionUtilisateur, ErreurBase> {
        let texte = preuve.utilisateur.to_string();
        sqlx::query!(
            "SELECT set_config('sigma.courriel_connexion', '', true) AS courriel, \
                    set_config('sigma.utilisateur', $1, true) AS utilisateur",
            texte,
        )
        .fetch_one(&mut *self.tx)
        .await?;
        Ok(TransactionUtilisateur {
            tx: self.tx,
            utilisateur: preuve.utilisateur,
        })
    }
}

async fn consommer_pas(
    connexion: &mut PgConnection,
    utilisateur: IdUtilisateur,
    mot_de_passe_lu: &str,
    pas: i64,
) -> Result<u64, ErreurBase> {
    let resultat = sqlx::query!(
        "UPDATE facteur_secours SET totp_dernier_pas = $3 \
         WHERE utilisateur_id = $1 AND mot_de_passe = $2 AND totp_dernier_pas < $3",
        utilisateur.uuid(),
        mot_de_passe_lu,
        pas,
    )
    .execute(connexion)
    .await?;
    Ok(resultat.rows_affected())
}

/// La méthode de secours de l'utilisateur du contexte.
///
/// # Errors
/// Erreur de PostgreSQL, ou donnée incohérente.
pub async fn lire_secours(
    tx: &mut TransactionUtilisateur,
) -> Result<Option<SecoursLu>, ErreurBase> {
    let utilisateur = tx.utilisateur().uuid();
    let ligne = sqlx::query!(
        "SELECT utilisateur_id, mot_de_passe, totp_chiffre, totp_cle_id, totp_dernier_pas, \
                modifie_le \
         FROM facteur_secours WHERE utilisateur_id = $1",
        utilisateur,
    )
    .fetch_optional(tx.connexion())
    .await?;
    ligne
        .map(|l| {
            Ok(SecoursLu {
                utilisateur: IdUtilisateur::depuis_uuid(l.utilisateur_id),
                mot_de_passe: l.mot_de_passe,
                totp_chiffre: l.totp_chiffre,
                totp_cle_id: l.totp_cle_id,
                totp_dernier_pas: pas_lu(l.totp_dernier_pas)?,
                modifie_le: l.modifie_le,
            })
        })
        .transpose()
}

/// Consomme le pas TOTP `pas` du secours de l'utilisateur du contexte (ré-authentification),
/// aux mêmes conditions que [`TransactionConnexion::consommer_pas_totp`]. Faux si le code est
/// rejoué ou le secours changé entre-temps.
///
/// # Errors
/// Erreur de PostgreSQL.
pub async fn consommer_pas_totp(
    tx: &mut TransactionUtilisateur,
    mot_de_passe_lu: &str,
    pas: u64,
) -> Result<bool, ErreurBase> {
    let utilisateur = tx.utilisateur();
    let touchees =
        consommer_pas(tx.connexion(), utilisateur, mot_de_passe_lu, pas_base(pas)?).await?;
    Ok(touchees == 1)
}

/// Enregistre (ou remplace) la méthode de secours de l'utilisateur du contexte : mot de passe
/// haché, secret TOTP chiffré, identifiant de clé, et pas du code de confirmation (déjà
/// consommé : il ne servira pas à une connexion). Le dernier pas accepté ne recule jamais : un
/// réenrôlement avec un code d'un pas antérieur ne rouvre pas un code déjà consommé.
///
/// # Errors
/// Erreur de PostgreSQL.
pub async fn enregistrer_secours(
    tx: &mut TransactionUtilisateur,
    mot_de_passe: &str,
    totp_chiffre: &[u8],
    totp_cle_id: &[u8],
    pas_confirme: u64,
    instant: OffsetDateTime,
) -> Result<(), ErreurBase> {
    let utilisateur = tx.utilisateur().uuid();
    sqlx::query!(
        "INSERT INTO facteur_secours (utilisateur_id, mot_de_passe, totp_chiffre, totp_cle_id, \
                                      totp_dernier_pas, cree_le, modifie_le) \
         VALUES ($1, $2, $3, $4, $5, $6, $6) \
         ON CONFLICT (utilisateur_id) DO UPDATE SET mot_de_passe = EXCLUDED.mot_de_passe, \
             totp_chiffre = EXCLUDED.totp_chiffre, totp_cle_id = EXCLUDED.totp_cle_id, \
             totp_dernier_pas = GREATEST(facteur_secours.totp_dernier_pas, \
                                         EXCLUDED.totp_dernier_pas), \
             modifie_le = EXCLUDED.modifie_le",
        utilisateur,
        mot_de_passe,
        totp_chiffre,
        totp_cle_id,
        pas_base(pas_confirme)?,
        instant,
    )
    .execute(tx.connexion())
    .await?;
    Ok(())
}

/// Remplace la chaîne du mot de passe vérifié (`ancienne`) par `nouvelle`, aux paramètres
/// courants d'Argon2id, si elle n'a pas changé entre-temps.
///
/// # Errors
/// Erreur de PostgreSQL.
pub async fn rehacher(
    tx: &mut TransactionUtilisateur,
    ancienne: &str,
    nouvelle: &str,
) -> Result<(), ErreurBase> {
    let utilisateur = tx.utilisateur().uuid();
    sqlx::query!(
        "UPDATE facteur_secours SET mot_de_passe = $3 \
         WHERE utilisateur_id = $1 AND mot_de_passe = $2",
        utilisateur,
        ancienne,
        nouvelle,
    )
    .execute(tx.connexion())
    .await?;
    Ok(())
}

/// Remplace les codes de récupération de l'utilisateur du contexte par une nouvelle série
/// (empreintes) : l'ancienne est annulée (ADR 0012).
///
/// # Errors
/// Erreur de PostgreSQL.
pub async fn remplacer_codes(
    tx: &mut TransactionUtilisateur,
    empreintes: &[[u8; 32]],
    instant: OffsetDateTime,
) -> Result<(), ErreurBase> {
    let utilisateur = tx.utilisateur().uuid();
    sqlx::query!(
        "DELETE FROM code_recuperation WHERE utilisateur_id = $1",
        utilisateur
    )
    .execute(tx.connexion())
    .await?;
    for empreinte in empreintes {
        sqlx::query!(
            "INSERT INTO code_recuperation (id, utilisateur_id, empreinte, cree_le) \
             VALUES ($1, $2, $3, $4)",
            Uuid::new_v4(),
            utilisateur,
            empreinte.as_slice(),
            instant,
        )
        .execute(tx.connexion())
        .await?;
    }
    Ok(())
}

/// Nombre de codes de récupération encore utilisables de l'utilisateur du contexte.
///
/// # Errors
/// Erreur de PostgreSQL.
pub async fn compter_codes_restants(tx: &mut TransactionUtilisateur) -> Result<i64, ErreurBase> {
    let utilisateur = tx.utilisateur().uuid();
    let n = sqlx::query_scalar!(
        r#"SELECT count(*) AS "n!" FROM code_recuperation
           WHERE utilisateur_id = $1 AND utilise_le IS NULL"#,
        utilisateur,
    )
    .fetch_one(tx.connexion())
    .await?;
    Ok(n)
}

/// Courriel de l'utilisateur du contexte (libellé du compte dans l'URI `otpauth`).
///
/// # Errors
/// Erreur de PostgreSQL.
pub async fn lire_courriel(tx: &mut TransactionUtilisateur) -> Result<Option<String>, ErreurBase> {
    let utilisateur = tx.utilisateur().uuid();
    let courriel = sqlx::query_scalar!(
        "SELECT courriel FROM utilisateur WHERE id = $1",
        utilisateur
    )
    .fetch_optional(tx.connexion())
    .await?;
    Ok(courriel)
}

/// Appartenance **active** de l'utilisateur du contexte à un cabinet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MembreActif {
    /// Appartenance.
    pub appartenance: IdAppartenance,
    /// Cabinet.
    pub cabinet: IdCabinet,
    /// Rôle enregistré.
    pub role: String,
}

/// Inscrit un événement dans la chaîne d'audit de **chaque cabinet** où l'utilisateur du
/// contexte a une appartenance active (ADR 0017 §2), dans la transaction de l'action : un
/// échec l'annule. `evenement` construit l'entrée de chaque cabinet (acteur avec son rôle).
/// Les cabinets sont pris dans l'ordre de leurs identifiants : deux inscriptions concurrentes
/// verrouillent les têtes dans le même ordre. Rend la transaction, au contexte du seul
/// utilisateur, et le nombre de cabinets.
///
/// # Errors
/// Erreur de PostgreSQL, ou inscription au journal d'audit refusée.
pub async fn inscrire_dans_les_cabinets(
    tx: TransactionUtilisateur,
    instant: OffsetDateTime,
    evenement: impl Fn(&MembreActif) -> Result<Evenement, ErreurBase>,
) -> Result<(TransactionUtilisateur, usize), ErreurBase> {
    let TransactionUtilisateur {
        mut tx,
        utilisateur,
    } = tx;
    // Sans cabinet en contexte, la politique `appartenance_membre` ne laisse voir que les
    // appartenances de l'utilisateur ; la requête filtre aussi.
    let membres: Vec<MembreActif> = sqlx::query!(
        "SELECT id, cabinet_id, role FROM appartenance \
         WHERE utilisateur_id = $1 AND actif ORDER BY cabinet_id",
        utilisateur.uuid(),
    )
    .fetch_all(&mut *tx)
    .await?
    .into_iter()
    .map(|l| MembreActif {
        appartenance: IdAppartenance::depuis_uuid(l.id),
        cabinet: IdCabinet::depuis_uuid(l.cabinet_id),
        role: l.role,
    })
    .collect();
    for membre in &membres {
        let texte = membre.cabinet.to_string();
        sqlx::query!(
            "SELECT set_config('sigma.cabinet', $1, true) AS cabinet",
            texte
        )
        .fetch_one(&mut *tx)
        .await?;
        let mut tenant = TransactionTenant {
            tx,
            contexte: ContexteTenant::cabinet(membre.cabinet, utilisateur),
        };
        inscrire_audit(
            &mut tenant,
            PorteeAudit::Cabinet,
            instant,
            evenement(membre)?,
        )
        .await?;
        tx = tenant.tx;
    }
    sqlx::query!("SELECT set_config('sigma.cabinet', '', true) AS cabinet")
        .fetch_one(&mut *tx)
        .await?;
    Ok((TransactionUtilisateur { tx, utilisateur }, membres.len()))
}
