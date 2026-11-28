//! File de tâches de l'instance (L2-30, PR 15a ; migration `0400_horodatage.sql` ; ADR 0010).
//!
//! Deux accès, sous deux contextes de RLS distincts :
//! - **tenant** ([`planifier_horodatage`], [`notifier`]) : dans la transaction d'une action d'un
//!   cabinet ou d'un dossier. La ligne est créée avec elle : elle n'existe que si l'action est
//!   validée (une alerte d'export ne part jamais pour un export refusé) ;
//! - **travailleur** ([`TransactionTravailleur`]) : sans cabinet en contexte, avec le réglage
//!   `sigma.travail = 'file'`, que seule cette transaction pose. Il prend les tâches échues, les
//!   termine, signale les retards et purge. Le travail lui-même (lecture de la chaîne, appel à
//!   l'autorité) se fait dans une transaction de tenant ordinaire, jamais sous ce contexte.
//!
//! Les instants des tâches créées par un tenant sont ceux de l'horloge de la base (`now()`), ceux
//! du travailleur viennent de l'horloge de l'application (injectable en test).

use sqlx::postgres::PgConnection;
use sqlx::{Postgres, Transaction};
use time::OffsetDateTime;
use uuid::Uuid;

use super::{ErreurBase, TransactionTenant};

/// Déclencheur d'une tâche d'horodatage (sujet de la tâche).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SujetHorodatage {
    /// Passage quotidien : planifié par une validation, pour le début du jour suivant (UTC).
    Quotidien,
    /// Verrouillage d'une période : le jour même.
    Verrouillage,
    /// Clôture : le jour même.
    Cloture,
}

impl SujetHorodatage {
    /// Nom stocké.
    #[must_use]
    pub const fn nom(self) -> &'static str {
        match self {
            Self::Quotidien => "quotidien",
            Self::Verrouillage => "verrouillage",
            Self::Cloture => "cloture",
        }
    }

    /// Relit un nom stocké.
    #[must_use]
    pub fn depuis_nom(nom: &str) -> Option<Self> {
        [Self::Quotidien, Self::Verrouillage, Self::Cloture]
            .into_iter()
            .find(|s| s.nom() == nom)
    }
}

/// Sujet d'une notification à l'administrateur (envoyée par le courriel de la PR 9).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SujetNotification {
    /// Export de l'audit du cabinet (#157).
    ExportAuditCabinet,
    /// Export de l'audit d'un dossier (#157).
    ExportAuditDossier,
    /// Export d'intégrité d'un dossier.
    ExportIntegrite,
    /// Aucun jeton depuis plus de 24 h pour une chaîne qui a changé (F6.2).
    RetardHorodatage,
    /// Témoin à remettre au cabinet après un verrouillage ou une clôture (question 9b).
    Temoin,
    /// Réinitialisation de l'accès d'un membre demandée (ADR 0012) ; objet : le membre.
    ReinitialisationDemandee,
    /// Réinitialisation annulée par un administrateur ou par le membre ; objet : le membre.
    ReinitialisationAnnulee,
    /// Lien de réinitialisation utilisé : facteurs retirés ; objet : le membre.
    ReinitialisationAboutie,
    /// Droits modifiés ou membre arrivé dans le cabinet (question 10a) ; objet : le membre.
    DroitsModifies,
    /// Séparation des tâches désactivée (Q5.3).
    SeparationDesactivee,
    /// Connexion par la méthode de secours (ADR 0012) ; objet : l'utilisateur.
    ConnexionSecours,
    /// Code de récupération utilisé (ADR 0012, F1.12) ; objet : l'utilisateur.
    CodeRecuperationUtilise,
    /// Coordonnée bancaire d'un tiers ajoutée, modifiée ou retirée (ADR 0031 point 3, F1.26) ;
    /// dossier : celui du tiers. Aucune donnée du tiers.
    CoordonneesBancairesTiers,
}

impl SujetNotification {
    /// Nom stocké.
    #[must_use]
    pub const fn nom(self) -> &'static str {
        match self {
            Self::ExportAuditCabinet => "export_audit_cabinet",
            Self::ExportAuditDossier => "export_audit_dossier",
            Self::ExportIntegrite => "export_integrite",
            Self::RetardHorodatage => "retard_horodatage",
            Self::Temoin => "temoin",
            Self::ReinitialisationDemandee => "reinitialisation_demandee",
            Self::ReinitialisationAnnulee => "reinitialisation_annulee",
            Self::ReinitialisationAboutie => "reinitialisation_aboutie",
            Self::DroitsModifies => "droits_modifies",
            Self::SeparationDesactivee => "separation_desactivee",
            Self::ConnexionSecours => "connexion_secours",
            Self::CodeRecuperationUtilise => "code_recuperation_utilise",
            Self::CoordonneesBancairesTiers => "coordonnees_bancaires_tiers",
        }
    }

    /// Tous les sujets.
    pub const TOUS: [Self; 13] = [
        Self::ExportAuditCabinet,
        Self::ExportAuditDossier,
        Self::ExportIntegrite,
        Self::RetardHorodatage,
        Self::Temoin,
        Self::ReinitialisationDemandee,
        Self::ReinitialisationAnnulee,
        Self::ReinitialisationAboutie,
        Self::DroitsModifies,
        Self::SeparationDesactivee,
        Self::ConnexionSecours,
        Self::CodeRecuperationUtilise,
        Self::CoordonneesBancairesTiers,
    ];

    /// Relit un nom stocké.
    #[must_use]
    pub fn depuis_nom(nom: &str) -> Option<Self> {
        Self::TOUS.into_iter().find(|s| s.nom() == nom)
    }
}

/// Planifie une tâche d'horodatage du dossier du contexte, dans la transaction de l'appelant.
/// Une tâche non terminée du même sujet existe déjà : rien n'est ajouté (`false`).
///
/// Quotidien : échéance au début du jour suivant (UTC) ; verrouillage et clôture : tout de suite.
///
/// # Errors
/// Contexte sans dossier, erreur de PostgreSQL.
pub async fn planifier_horodatage(
    tx: &mut TransactionTenant,
    sujet: SujetHorodatage,
) -> Result<bool, ErreurBase> {
    let dossier = tx
        .contexte()
        .id_dossier()
        .ok_or(ErreurBase::Donnee("tâche d'horodatage hors d'un dossier"))?;
    let cabinet = tx.contexte().id_cabinet();
    let quotidien = sujet == SujetHorodatage::Quotidien;
    let n = sqlx::query!(
        "INSERT INTO tache (id, nature, sujet, cabinet_cible, dossier_cible, etat, cree_le, \
                            prochaine_tentative) \
         VALUES ($1, 'horodatage', $2, $3, $4, 'en_attente', now(), \
                 CASE WHEN $5 THEN ((now() AT TIME ZONE 'UTC')::date + 1)::timestamp \
                                   AT TIME ZONE 'UTC' ELSE now() END) \
         ON CONFLICT DO NOTHING",
        Uuid::new_v4(),
        sujet.nom(),
        cabinet.uuid(),
        dossier.uuid(),
        quotidien,
    )
    .execute(tx.connexion())
    .await?
    .rows_affected();
    Ok(n == 1)
}

/// Crée une notification pour l'administrateur du cabinet du contexte, dans la transaction de
/// l'action qui la motive. Le dossier est celui du contexte, s'il y en a un. Elle attend le
/// courriel de la PR 9 (`en_attente`).
///
/// # Errors
/// Erreur de PostgreSQL.
pub async fn notifier(
    tx: &mut TransactionTenant,
    sujet: SujetNotification,
    acteur: Option<Uuid>,
) -> Result<(), ErreurBase> {
    alerter(tx, sujet, acteur, None).await
}

/// Comme [`notifier`], avec l'utilisateur **concerné** par l'alerte (`objet`) : le membre dont
/// les droits changent, dont l'accès est réinitialisé, qui se connecte par le secours. Aucune
/// autre donnée : le courriel est composé à l'envoi, par le travailleur.
///
/// # Errors
/// Erreur de PostgreSQL.
pub async fn alerter(
    tx: &mut TransactionTenant,
    sujet: SujetNotification,
    acteur: Option<Uuid>,
    concerne: Option<Uuid>,
) -> Result<(), ErreurBase> {
    let cabinet = tx.contexte().id_cabinet().uuid();
    let dossier = tx.contexte().id_dossier().map(super::IdDossier::uuid);
    sqlx::query!(
        "INSERT INTO tache (id, nature, sujet, cabinet_cible, dossier_cible, acteur_cible, objet, \
                            etat, cree_le, prochaine_tentative) \
         VALUES ($1, 'notification', $2, $3, $4, $5, $6, 'en_attente', now(), now()) \
         ON CONFLICT DO NOTHING",
        Uuid::new_v4(),
        sujet.nom(),
        cabinet,
        dossier,
        acteur,
        concerne,
    )
    .execute(tx.connexion())
    .await?;
    Ok(())
}

/// Sujet d'un courriel de la file : le lien à remettre.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SujetCourriel {
    /// Lien d'invitation (24 h).
    Invitation,
    /// Lien de réinitialisation d'accès (1 h), émis au terme du délai de 24 h.
    Reinitialisation,
}

impl SujetCourriel {
    /// Nom stocké.
    #[must_use]
    pub const fn nom(self) -> &'static str {
        match self {
            Self::Invitation => "invitation",
            Self::Reinitialisation => "reinitialisation",
        }
    }

    /// Relit un nom stocké.
    #[must_use]
    pub fn depuis_nom(nom: &str) -> Option<Self> {
        [Self::Invitation, Self::Reinitialisation]
            .into_iter()
            .find(|s| s.nom() == nom)
    }
}

/// Planifie l'envoi du lien d'une invitation ou d'une réinitialisation du cabinet du contexte,
/// dans la transaction de l'action : le courriel n'existe que si l'action est validée. `echeance` :
/// instant de l'envoi (`None` : tout de suite). Un envoi non terminé du même objet existe déjà :
/// rien n'est ajouté (`false`). Aucun courriel, aucun nom, aucun lien n'est écrit ici.
///
/// # Errors
/// Erreur de PostgreSQL.
pub async fn planifier_courriel(
    tx: &mut TransactionTenant,
    sujet: SujetCourriel,
    objet: Uuid,
    echeance: Option<OffsetDateTime>,
) -> Result<bool, ErreurBase> {
    let cabinet = tx.contexte().id_cabinet().uuid();
    let n = sqlx::query!(
        "INSERT INTO tache (id, nature, sujet, cabinet_cible, objet, etat, cree_le, \
                            prochaine_tentative) \
         VALUES ($1, 'courriel', $2, $3, $4, 'en_attente', now(), COALESCE($5, now())) \
         ON CONFLICT DO NOTHING",
        Uuid::new_v4(),
        sujet.nom(),
        cabinet,
        objet,
        echeance,
    )
    .execute(tx.connexion())
    .await?
    .rows_affected();
    Ok(n == 1)
}

/// Plafond d'envois de lien d'invitation par adresse et par cabinet sur 24 heures (création et
/// renvois) : un administrateur, ou un compte détourné, ne fait pas de la messagerie de
/// l'instance un relais de harcèlement (ADR 0024, §7).
pub const INVITATIONS_MAX_PAR_ADRESSE_24H: i64 = 3;
/// Plafond d'envois de lien d'invitation par cabinet sur 24 heures.
pub const INVITATIONS_MAX_PAR_CABINET_24H: i64 = 100;

/// Plafonds atteints pour une invitation : par adresse, ou par cabinet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlafondInvitation {
    /// Trop d'envois vers cette adresse.
    Adresse,
    /// Trop d'envois depuis ce cabinet.
    Cabinet,
}

/// Vérifie les plafonds d'envoi de lien d'invitation du cabinet du contexte sur 24 heures, pour
/// l'adresse (déjà normalisée) donnée. Compte les tâches d'envoi, quel que soit leur état.
///
/// # Errors
/// Erreur de PostgreSQL.
pub async fn plafond_invitation(
    tx: &mut TransactionTenant,
    courriel: &str,
) -> Result<Option<PlafondInvitation>, ErreurBase> {
    let cabinet = tx.contexte().id_cabinet().uuid();
    let par_cabinet = sqlx::query_scalar!(
        "SELECT count(*) AS \"n!\" FROM tache \
         WHERE cabinet_cible = $1 AND nature = 'courriel' AND sujet = 'invitation' \
           AND cree_le > now() - interval '24 hours'",
        cabinet,
    )
    .fetch_one(tx.connexion())
    .await?;
    if par_cabinet >= INVITATIONS_MAX_PAR_CABINET_24H {
        return Ok(Some(PlafondInvitation::Cabinet));
    }
    let par_adresse = sqlx::query_scalar!(
        "SELECT count(*) AS \"n!\" FROM tache t \
         JOIN invitation i ON i.cabinet_id = t.cabinet_cible AND i.id = t.objet \
         WHERE t.cabinet_cible = $1 AND t.nature = 'courriel' AND t.sujet = 'invitation' \
           AND lower(i.courriel) = $2 AND t.cree_le > now() - interval '24 hours'",
        cabinet,
        courriel,
    )
    .fetch_one(tx.connexion())
    .await?;
    Ok((par_adresse >= INVITATIONS_MAX_PAR_ADRESSE_24H).then_some(PlafondInvitation::Adresse))
}

/// Envoi de courriel pris par le travailleur.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CourrielPris {
    /// Identifiant de la tâche.
    pub id: Uuid,
    /// Cabinet.
    pub cabinet: Uuid,
    /// Sujet.
    pub sujet: SujetCourriel,
    /// Invitation ou réinitialisation visée.
    pub objet: Uuid,
    /// Tentatives déjà faites (celle-ci non comprise).
    pub tentatives: i32,
    /// Jeton de cette prise : lui seul termine ou reporte la tâche.
    pub prise: Uuid,
}

/// Notification prise par le travailleur.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NotificationPrise {
    /// Identifiant de la tâche.
    pub id: Uuid,
    /// Cabinet.
    pub cabinet: Uuid,
    /// Dossier concerné, s'il y en a un.
    pub dossier: Option<Uuid>,
    /// Sujet.
    pub sujet: SujetNotification,
    /// Utilisateur concerné par l'alerte, s'il y en a un.
    pub concerne: Option<Uuid>,
    /// Tentatives déjà faites (celle-ci non comprise).
    pub tentatives: i32,
    /// Jeton de cette prise : lui seul termine ou reporte la tâche.
    pub prise: Uuid,
}

/// Tâche d'horodatage prise par le travailleur.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TachePrise {
    /// Identifiant de la tâche.
    pub id: Uuid,
    /// Cabinet visé.
    pub cabinet: Uuid,
    /// Dossier visé.
    pub dossier: Uuid,
    /// Déclencheur.
    pub sujet: SujetHorodatage,
    /// Tentatives déjà faites (celle-ci non comprise).
    pub tentatives: i32,
    /// Jeton de cette prise : lui seul termine ou reporte la tâche.
    pub prise: Uuid,
}

/// Transaction du travailleur de la file : contexte `sigma.travail = 'file'`, aucun cabinet.
/// Sous RLS, elle ne voit que la table `tache`.
#[derive(Debug)]
pub struct TransactionTravailleur {
    tx: Transaction<'static, Postgres>,
}

/// Ouvre la transaction du travailleur.
///
/// # Errors
/// Connexion indisponible ou erreur de PostgreSQL.
pub(crate) async fn ouvrir_transaction_travailleur(
    pool: &sqlx::PgPool,
) -> Result<TransactionTravailleur, ErreurBase> {
    let mut tx = pool.begin().await?;
    sqlx::query!(
        "SELECT set_config('sigma.cabinet', '', true) AS cabinet, \
                set_config('sigma.dossier', '', true) AS dossier, \
                set_config('sigma.utilisateur', '', true) AS utilisateur, \
                set_config('sigma.travail', 'file', true) AS travail",
    )
    .fetch_one(&mut *tx)
    .await?;
    Ok(TransactionTravailleur { tx })
}

impl TransactionTravailleur {
    fn connexion(&mut self) -> &mut PgConnection {
        &mut self.tx
    }

    /// Valide la transaction.
    ///
    /// # Errors
    /// Erreur de PostgreSQL à la validation.
    pub async fn confirmer(self) -> Result<(), ErreurBase> {
        self.tx.commit().await?;
        Ok(())
    }

    /// Prend au plus `limite` tâches d'horodatage échues (en attente à l'échéance, ou en cours
    /// dont le bail est passé), les marque en cours jusqu'à `maintenant + bail` avec un jeton de
    /// prise neuf. `SKIP LOCKED` : deux travailleurs ne prennent jamais la même tâche. Le
    /// travailleur en prend **une à la fois** : le bail ne couvre que la tâche en cours.
    ///
    /// # Errors
    /// Erreur de PostgreSQL ou ligne incohérente.
    pub async fn prendre_echues(
        &mut self,
        maintenant: OffsetDateTime,
        bail: time::Duration,
        limite: i64,
    ) -> Result<Vec<TachePrise>, ErreurBase> {
        let jusqua = maintenant + bail;
        let prise = Uuid::new_v4();
        let lignes = sqlx::query!(
            "WITH choisies AS MATERIALIZED ( \
                 SELECT id FROM tache \
                 WHERE nature = 'horodatage' \
                   AND ((etat = 'en_attente' AND prochaine_tentative <= $1) \
                        OR (etat = 'en_cours' AND reserve_jusqua <= $1)) \
                 ORDER BY prochaine_tentative, id LIMIT $3 FOR UPDATE SKIP LOCKED) \
             UPDATE tache SET etat = 'en_cours', reserve_jusqua = $2, prise = $4, \
                              tentatives = tache.tentatives + 1 \
             FROM choisies WHERE tache.id = choisies.id \
             RETURNING tache.id, tache.cabinet_cible, tache.dossier_cible AS \"dossier_cible!\", tache.sujet, \
                       tache.tentatives - 1 AS \"avant!\"",
            maintenant,
            jusqua,
            limite,
            prise,
        )
        .fetch_all(self.connexion())
        .await?;
        lignes
            .into_iter()
            .map(|l| {
                Ok(TachePrise {
                    id: l.id,
                    cabinet: l.cabinet_cible,
                    dossier: l.dossier_cible,
                    sujet: SujetHorodatage::depuis_nom(&l.sujet)
                        .ok_or(ErreurBase::Donnee("sujet de tâche inconnu"))?,
                    tentatives: l.avant,
                    prise,
                })
            })
            .collect()
    }

    /// Prend au plus `limite` envois de courriel échus, comme [`Self::prendre_echues`] : bail,
    /// jeton de prise, `SKIP LOCKED`. L'échéance (`prochaine_tentative`) est posée par l'horloge de
    /// **la base** (`now()`) ; elle est comparée à la plus avancée des deux horloges, celle de
    /// l'application (`maintenant`, injectable) et celle de la base : un écart entre elles ne
    /// retarde jamais une tâche déjà échue.
    ///
    /// # Errors
    /// Erreur de PostgreSQL ou ligne incohérente.
    pub async fn prendre_courriels(
        &mut self,
        maintenant: OffsetDateTime,
        bail: time::Duration,
        limite: i64,
    ) -> Result<Vec<CourrielPris>, ErreurBase> {
        let jusqua = maintenant + bail;
        let prise = Uuid::new_v4();
        let lignes = sqlx::query!(
            "WITH choisies AS MATERIALIZED ( \
                 SELECT id FROM tache \
                 WHERE nature = 'courriel' \
                   AND ((etat = 'en_attente' AND prochaine_tentative <= GREATEST($1::timestamptz, now())) \
                        OR (etat = 'en_cours' AND reserve_jusqua <= $1)) \
                 ORDER BY prochaine_tentative, id LIMIT $3 FOR UPDATE SKIP LOCKED) \
             UPDATE tache SET etat = 'en_cours', reserve_jusqua = $2, prise = $4, \
                              tentatives = tache.tentatives + 1 \
             FROM choisies WHERE tache.id = choisies.id \
             RETURNING tache.id, tache.cabinet_cible, tache.sujet, tache.objet AS \"objet!\", \
                       tache.tentatives - 1 AS \"avant!\"",
            maintenant,
            jusqua,
            limite,
            prise,
        )
        .fetch_all(self.connexion())
        .await?;
        lignes
            .into_iter()
            .map(|l| {
                Ok(CourrielPris {
                    id: l.id,
                    cabinet: l.cabinet_cible,
                    sujet: SujetCourriel::depuis_nom(&l.sujet)
                        .ok_or(ErreurBase::Donnee("sujet de courriel inconnu"))?,
                    objet: l.objet,
                    tentatives: l.avant,
                    prise,
                })
            })
            .collect()
    }

    /// Prend au plus `limite` notifications échues, comme [`Self::prendre_echues`].
    ///
    /// # Errors
    /// Erreur de PostgreSQL ou ligne incohérente.
    pub async fn prendre_notifications(
        &mut self,
        maintenant: OffsetDateTime,
        bail: time::Duration,
        limite: i64,
    ) -> Result<Vec<NotificationPrise>, ErreurBase> {
        let jusqua = maintenant + bail;
        let prise = Uuid::new_v4();
        let lignes = sqlx::query!(
            "WITH choisies AS MATERIALIZED ( \
                 SELECT id FROM tache \
                 WHERE nature = 'notification' \
                   AND ((etat = 'en_attente' AND prochaine_tentative <= GREATEST($1::timestamptz, now())) \
                        OR (etat = 'en_cours' AND reserve_jusqua <= $1)) \
                 ORDER BY prochaine_tentative, id LIMIT $3 FOR UPDATE SKIP LOCKED) \
             UPDATE tache SET etat = 'en_cours', reserve_jusqua = $2, prise = $4, \
                              tentatives = tache.tentatives + 1 \
             FROM choisies WHERE tache.id = choisies.id \
             RETURNING tache.id, tache.cabinet_cible, tache.dossier_cible, tache.sujet, tache.objet, \
                       tache.tentatives - 1 AS \"avant!\"",
            maintenant,
            jusqua,
            limite,
            prise,
        )
        .fetch_all(self.connexion())
        .await?;
        lignes
            .into_iter()
            .map(|l| {
                Ok(NotificationPrise {
                    id: l.id,
                    cabinet: l.cabinet_cible,
                    dossier: l.dossier_cible,
                    sujet: SujetNotification::depuis_nom(&l.sujet)
                        .ok_or(ErreurBase::Donnee("sujet de notification inconnu"))?,
                    concerne: l.objet,
                    tentatives: l.avant,
                    prise,
                })
            })
            .collect()
    }

    /// Termine une tâche prise, avec le code de son issue. Sans effet (`false`) si la prise n'est
    /// plus celle de l'appelant : son bail a expiré et un autre travailleur a repris la tâche.
    ///
    /// # Errors
    /// Erreur de PostgreSQL.
    pub async fn terminer(
        &mut self,
        id: Uuid,
        prise: Uuid,
        issue: &str,
        maintenant: OffsetDateTime,
    ) -> Result<bool, ErreurBase> {
        Ok(sqlx::query!(
            "UPDATE tache SET etat = 'terminee', termine_le = $4, reserve_jusqua = NULL, \
                              prise = NULL, derniere_issue = $3 \
             WHERE id = $1 AND etat = 'en_cours' AND prise = $2",
            id,
            prise,
            issue,
            maintenant,
        )
        .execute(self.connexion())
        .await?
        .rows_affected()
            == 1)
    }

    /// Remet une tâche prise en attente, à rejouer à `prochaine`, avec le code de l'issue
    /// (rejeu différé : échec de toutes les autorités, ou rappel du lendemain). Sans effet
    /// (`false`) si la prise n'est plus celle de l'appelant.
    ///
    /// Si une tâche du même sujet attend déjà pour ce dossier (planifiée pendant l'appel), les
    /// deux sont fusionnées : celle-ci se termine (`fusionnee`) et l'autre reprend la plus
    /// ancienne date de création, qui sert au retard.
    ///
    /// # Errors
    /// Erreur de PostgreSQL.
    pub async fn reporter(
        &mut self,
        id: Uuid,
        prise: Uuid,
        issue: &str,
        prochaine: OffsetDateTime,
        maintenant: OffsetDateTime,
    ) -> Result<bool, ErreurBase> {
        let fusion = sqlx::query!(
            "UPDATE tache a SET cree_le = LEAST(a.cree_le, t.cree_le) \
             FROM tache t \
             WHERE t.id = $1 AND t.etat = 'en_cours' AND t.prise = $2 \
               AND a.nature = 'horodatage' AND a.etat = 'en_attente' \
               AND a.cabinet_cible = t.cabinet_cible AND a.dossier_cible = t.dossier_cible \
               AND a.sujet = t.sujet",
            id,
            prise,
        )
        .execute(self.connexion())
        .await?
        .rows_affected();
        if fusion > 0 {
            return self.terminer(id, prise, "fusionnee", maintenant).await;
        }
        Ok(sqlx::query!(
            "UPDATE tache SET etat = 'en_attente', prochaine_tentative = $4, \
                              reserve_jusqua = NULL, prise = NULL, derniere_issue = $3 \
             WHERE id = $1 AND etat = 'en_cours' AND prise = $2",
            id,
            prise,
            issue,
            prochaine,
        )
        .execute(self.connexion())
        .await?
        .rows_affected()
            == 1)
    }

    /// **Une alerte par dossier et par période de `delai`**, quel que soit l'état de la précédente :
    /// une alerte envoyée puis terminée n'en appelle pas une nouvelle à chaque passage (un courriel
    /// par minute pendant une panne de l'autorité).
    ///
    /// Crée l'alerte de retard (F6.2) de chaque dossier dont une tâche d'horodatage non
    /// terminée date de plus de `delai` : toute écriture non couverte a une tâche, créée à la
    /// première validation qui a suivi le dernier jeton. Une seule alerte non traitée par dossier.
    /// Rend les dossiers nouvellement alertés.
    ///
    /// # Errors
    /// Erreur de PostgreSQL.
    pub async fn signaler_retards(
        &mut self,
        maintenant: OffsetDateTime,
        delai: time::Duration,
    ) -> Result<Vec<(Uuid, Uuid)>, ErreurBase> {
        let seuil = maintenant - delai;
        let lignes = sqlx::query!(
            "INSERT INTO tache (id, nature, sujet, cabinet_cible, dossier_cible, etat, cree_le, \
                                prochaine_tentative) \
             SELECT gen_random_uuid(), 'notification', 'retard_horodatage', t.cabinet_cible, \
                    t.dossier_cible, 'en_attente', $2, $2 \
             FROM tache t \
             WHERE t.nature = 'horodatage' AND t.etat <> 'terminee' AND t.cree_le <= $1 \
               AND NOT EXISTS ( \
                   SELECT 1 FROM tache n \
                   WHERE n.sujet = 'retard_horodatage' \
                     AND n.cabinet_cible = t.cabinet_cible AND n.dossier_cible = t.dossier_cible \
                     AND n.cree_le > $1) \
             GROUP BY t.cabinet_cible, t.dossier_cible \
             ON CONFLICT DO NOTHING \
             RETURNING cabinet_cible, dossier_cible AS \"dossier_cible!\"",
            seuil,
            maintenant,
        )
        .fetch_all(self.connexion())
        .await?;
        Ok(lignes
            .into_iter()
            .map(|l| (l.cabinet_cible, l.dossier_cible))
            .collect())
    }

    /// Nombre d'alertes du sujet déjà **envoyées** pour ce cabinet depuis `depuis` : base du
    /// plafond d'envoi par cabinet et par sujet (ADR 0025, §6).
    ///
    /// # Errors
    /// Erreur de PostgreSQL.
    pub async fn compter_alertes_envoyees(
        &mut self,
        cabinet: Uuid,
        sujet: SujetNotification,
        depuis: OffsetDateTime,
    ) -> Result<i64, ErreurBase> {
        Ok(sqlx::query_scalar!(
            "SELECT count(*) AS \"n!\" FROM tache \
             WHERE nature = 'notification' AND cabinet_cible = $1 AND sujet = $2 \
               AND etat = 'terminee' AND derniere_issue = 'envoye' AND termine_le > $3",
            cabinet,
            sujet.nom(),
            depuis,
        )
        .fetch_one(self.connexion())
        .await?)
    }

    /// Supprime les tâches d'horodatage, d'envoi de courriel et de notification terminées avant
    /// `avant`. Rend leur nombre.
    ///
    /// # Errors
    /// Erreur de PostgreSQL.
    pub async fn purger(&mut self, avant: OffsetDateTime) -> Result<u64, ErreurBase> {
        Ok(sqlx::query!(
            "DELETE FROM tache WHERE nature IN ('horodatage', 'courriel', 'notification') \
                                 AND etat = 'terminee' AND termine_le < $1",
            avant,
        )
        .execute(self.connexion())
        .await?
        .rows_affected())
    }
}
