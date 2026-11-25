//! Envoi des liens par la file de tâches (L2-19, PR 9b ; ADR 0012, 0022, 0024).
//!
//! Un [`Expediteur`] envoie aussi les **alertes** (nature `notification`, PR 9c, ADR 0025) aux
//! administrateurs actifs du cabinet (et au membre concerné pour la réinitialisation de son
//! accès) : exports d'audit et d'intégrité (#157), retard d'horodatage (F6.2), témoin remis au
//! cabinet (question 9b), étapes d'une réinitialisation, changements de droits, séparation des
//! tâches désactivée, connexion par le secours ou par un code de récupération. Les destinataires
//! et le texte sont établis **à l'envoi**, en base, sous le contexte du cabinet : la tâche ne
//! porte que des identifiants. Un échec provisoire rejoue l'envoi à tous les destinataires (un
//! destinataire déjà servi peut recevoir un doublon : au moins une fois).
//!
//! Un [`Expediteur`] repasse périodiquement : il prend les envois de courriel échus
//! ([`crate::base::taches`], nature `courriel`), **une tâche à la fois**, et pour chacune
//! 1. dans une transaction **de tenant ordinaire** (cabinet de la tâche) : relit l'invitation ou
//!    la réinitialisation (toujours en attente ? délai de 24 heures écoulé ?), **émet le lien**
//!    ([`crate::base::liens::emettre`] : seule son empreinte est stockée) et compose le message ;
//! 2. fait remettre le message **hors de toute transaction** (l'échange SMTP peut durer) ;
//! 3. termine la tâche (`envoye`), la remet à plus tard (échec provisoire : délai doublé, 1 min à
//!    1 h, huit tentatives au plus) ou l'abandonne (refus définitif : sans rejeu).
//!
//! Le lien n'existe donc en clair que dans la mémoire de ce travailleur, le temps de l'envoi, et
//! dans le courriel. Un envoi rejoué émet un lien **neuf**, qui ferme le précédent. Un arrêt
//! entre l'émission et la remise laisse la tâche prise : reprise au bail (5 minutes), nouveau lien.
//! Aucun courriel, aucun nom, aucun lien dans les journaux techniques ni dans les issues
//! (codes stables seulement).

use std::sync::Arc;
use std::time::Duration;

use sqlx::PgPool;
use time::OffsetDateTime;

use super::modele::{Destinataire, TemoinAlerte, alerte};
use super::modele::{Message, adresse_du_lien, invitation, reinitialisation};
use super::transport::Transport;
use crate::base::administration::{lire_invitation, lire_membre, lire_reinitialisation};
use crate::base::jetons::{dernier_jeton, lire_tete};
use crate::base::liens::{NatureLien, emettre};
use crate::base::taches::{
    CourrielPris, NotificationPrise, SujetCourriel, SujetNotification,
    ouvrir_transaction_travailleur,
};
use crate::base::{ContexteTenant, ErreurBase, IdCabinet, IdUtilisateur, ouvrir_transaction};
use crate::serveur::journal::CIBLE;
use crate::serveur::session::Horloge;

/// Réglages de l'expéditeur.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Reglages {
    /// Délai entre deux passages de la boucle.
    pub periode: Duration,
    /// Bail d'une tâche prise (plus long que l'échange SMTP le plus long).
    pub bail: time::Duration,
    /// Tâches prises par passage.
    pub lot: i64,
    /// Tentatives au plus avant abandon.
    pub tentatives_max: i32,
    /// Budget de temps d'une tâche d'envoi (tous destinataires), **inférieur au bail** : la
    /// tâche ne peut pas être reprise par un autre passage pendant qu'elle s'exécute.
    pub budget: Duration,
    /// Plafond d'alertes envoyées par cabinet et par sujet sur 24 heures : au-delà, l'alerte est
    /// ignorée (issue `plafond`) plutôt que d'inonder les administrateurs.
    pub alertes_max_par_sujet_24h: i64,
    /// Attente avant de relancer la boucle après son arrêt inattendu (panique).
    pub relance: Duration,
}

impl Default for Reglages {
    fn default() -> Self {
        Self {
            periode: Duration::from_secs(30),
            bail: time::Duration::minutes(5),
            lot: 20,
            tentatives_max: 8,
            budget: Duration::from_secs(240),
            alertes_max_par_sujet_24h: 10,
            relance: Duration::from_secs(5),
        }
    }
}

/// Bilan d'un passage.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Bilan {
    /// Tâches prises.
    pub prises: u32,
    /// Courriels remis au relais.
    pub envoyes: u32,
    /// Envois remis à plus tard.
    pub reportes: u32,
    /// Envois sans objet (invitation révoquée, réinitialisation annulée) ou abandonnés.
    pub clos: u32,
    /// Tâches en erreur interne (base) : reprises au bail.
    pub erreurs: u32,
}

/// Expéditeur de la file de courriels.
#[derive(Debug, Clone)]
pub struct Expediteur {
    pool: PgPool,
    transport: Arc<dyn Transport>,
    horloge: Arc<dyn Horloge>,
    origine: String,
    reglages: Reglages,
}

enum Preparation {
    Message(Message),
    SansObjet,
    Invalide,
    /// Pas avant cet instant (réinitialisation dont le délai de contestation court encore).
    Pas(OffsetDateTime),
}

enum Sortie {
    Envoye,
    SansObjet,
    Invalide,
    Pas(OffsetDateTime),
    Echec(&'static str),
    Refus(&'static str),
}

/// Délai avant le rejeu après `tentatives` échecs : une minute, doublée à chaque échec, une
/// heure au plus.
fn delai_de_rejeu(tentatives: i32) -> time::Duration {
    let doublements = u32::try_from(tentatives).unwrap_or(0).min(6);
    time::Duration::minutes(1 << doublements).min(time::Duration::hours(1))
}

impl Expediteur {
    /// Expéditeur sur ce pool (rôle applicatif), ce transport, cette horloge et l'origine publique
    /// de l'instance (domaine des liens).
    #[must_use]
    pub fn new(
        pool: PgPool,
        transport: Arc<dyn Transport>,
        horloge: Arc<dyn Horloge>,
        origine: String,
        reglages: Reglages,
    ) -> Self {
        Self {
            pool,
            transport,
            horloge,
            origine,
            reglages,
        }
    }

    /// Un passage : les envois échus, un par un.
    ///
    /// # Errors
    /// Erreur de base lors de la prise des tâches. L'erreur d'une tâche n'interrompt pas les
    /// autres : elle est comptée dans [`Bilan::erreurs`].
    pub async fn passer(&self) -> Result<Bilan, ErreurBase> {
        let mut bilan = Bilan::default();
        for _ in 0..self.reglages.lot {
            let mut tx = ouvrir_transaction_travailleur(&self.pool).await?;
            let mut prises = tx
                .prendre_courriels(self.horloge.maintenant(), self.reglages.bail, 1)
                .await?;
            tx.confirmer().await?;
            let Some(tache) = prises.pop() else { break };
            bilan.prises += 1;
            // Plafond de tentatives **avant** le traitement : une tâche en erreur interne (base)
            // est reprise au bail, elle ne l'est pas indéfiniment.
            if tache.tentatives >= self.reglages.tentatives_max {
                self.abandonner(tache.id, tache.prise).await?;
                bilan.clos += 1;
                continue;
            }
            match self.traiter(tache).await {
                Ok(Sortie::Envoye) => bilan.envoyes += 1,
                Ok(Sortie::Echec(_) | Sortie::Pas(_)) => bilan.reportes += 1,
                Ok(Sortie::SansObjet | Sortie::Invalide | Sortie::Refus(_)) => bilan.clos += 1,
                Err(_) => bilan.erreurs += 1,
            }
        }
        for _ in 0..self.reglages.lot {
            let mut tx = ouvrir_transaction_travailleur(&self.pool).await?;
            let mut prises = tx
                .prendre_notifications(self.horloge.maintenant(), self.reglages.bail, 1)
                .await?;
            tx.confirmer().await?;
            let Some(tache) = prises.pop() else { break };
            bilan.prises += 1;
            if tache.tentatives >= self.reglages.tentatives_max {
                self.abandonner(tache.id, tache.prise).await?;
                bilan.clos += 1;
                continue;
            }
            match self.traiter_notification(tache).await {
                Ok(Sortie::Envoye) => bilan.envoyes += 1,
                Ok(Sortie::Echec(_) | Sortie::Pas(_)) => bilan.reportes += 1,
                Ok(Sortie::SansObjet | Sortie::Invalide | Sortie::Refus(_)) => bilan.clos += 1,
                Err(_) => bilan.erreurs += 1,
            }
        }
        Ok(bilan)
    }

    /// Termine une tâche qui a épuisé ses tentatives.
    async fn abandonner(&self, id: uuid::Uuid, prise: uuid::Uuid) -> Result<(), ErreurBase> {
        let maintenant = self.horloge.maintenant();
        let mut tx = ouvrir_transaction_travailleur(&self.pool).await?;
        tx.terminer(id, prise, "abandon", maintenant).await?;
        tx.confirmer().await?;
        tracing::warn!(target: CIBLE, issue = "abandon", "envoi de courriel abandonné");
        Ok(())
    }

    /// Messages d'une alerte : un par administrateur actif, plus le membre concerné pour les
    /// étapes d'une réinitialisation. Lus et composés sous le contexte du cabinet (et du dossier).
    async fn preparer_alerte(&self, tache: &NotificationPrise) -> Result<Vec<Message>, ErreurBase> {
        let cabinet = IdCabinet::depuis_uuid(tache.cabinet);
        let acteur = IdUtilisateur::nouveau();
        let contexte = match tache.dossier {
            Some(d) => {
                ContexteTenant::dossier(cabinet, crate::base::IdDossier::depuis_uuid(d), acteur)
            }
            None => ContexteTenant::cabinet(cabinet, acteur),
        };
        let mut tx = ouvrir_transaction(&self.pool, contexte).await?;
        let membres = crate::base::administration::lister_membres(&mut tx).await?;
        let temoin = if tache.sujet == SujetNotification::Temoin && tache.dossier.is_some() {
            match lire_tete(&mut tx).await? {
                Some(t) => {
                    let jeton = dernier_jeton(&mut tx).await?.and_then(|j| {
                        let quand = j
                            .obtenu_le
                            .to_offset(time::UtcOffset::UTC)
                            .format(&time::format_description::well_known::Rfc3339)
                            .ok()?;
                        Some((j.nombre, quand))
                    });
                    Some(TemoinAlerte {
                        nombre: t.rang,
                        tete: t.empreinte.to_string(),
                        jeton,
                    })
                }
                None => None,
            }
        } else {
            None
        };
        tx.confirmer().await?;
        let reinit = matches!(
            tache.sujet,
            SujetNotification::ReinitialisationDemandee
                | SujetNotification::ReinitialisationAnnulee
                | SujetNotification::ReinitialisationAboutie
        );
        let mut messages = Vec::new();
        for m in membres.iter().filter(|m| m.actif) {
            let admin = m.role == authz::Role::Administrateur.nom();
            let concerne = reinit && Some(m.utilisateur.uuid()) == tache.concerne;
            if !admin && !concerne {
                continue;
            }
            // Le membre concerné est alerté en tant que tel (une seule fois), même s'il administre
            // le cabinet.
            let qui = if concerne {
                Destinataire::Concerne
            } else {
                Destinataire::Administrateur
            };
            if let Some(message) = alerte(
                tache.sujet,
                &m.courriel,
                &m.nom,
                qui,
                tache.dossier,
                temoin.as_ref(),
            ) {
                messages.push(message);
            }
        }
        Ok(messages)
    }

    async fn traiter_notification(&self, tache: NotificationPrise) -> Result<Sortie, ErreurBase> {
        let maintenant = self.horloge.maintenant();
        // Plafond par cabinet et par sujet sur 24 heures : au-delà, rien n'est envoyé.
        let mut tx = ouvrir_transaction_travailleur(&self.pool).await?;
        let deja = tx
            .compter_alertes_envoyees(
                tache.cabinet,
                tache.sujet,
                maintenant - time::Duration::hours(24),
            )
            .await?;
        tx.confirmer().await?;
        let messages = if deja >= self.reglages.alertes_max_par_sujet_24h {
            Vec::new()
        } else {
            self.preparer_alerte(&tache).await?
        };
        let sortie = if deja >= self.reglages.alertes_max_par_sujet_24h {
            Sortie::Refus("plafond")
        } else if messages.is_empty() {
            Sortie::SansObjet
        } else {
            let (mut envoyes, mut rejouable, mut refus) = (0, None, false);
            // Budget de temps global, inférieur au bail : tous les destinataires, ou rien de plus.
            let remise = tokio::time::timeout(self.reglages.budget, async {
                for message in &messages {
                    match self.transport.envoyer(message).await {
                        Ok(()) => envoyes += 1,
                        Err(e) if e.a_rejouer() => rejouable = Some(e),
                        Err(_) => refus = true,
                    }
                }
            })
            .await;
            let delai_depasse = remise.is_err();
            match rejouable {
                Some(e) if tache.tentatives + 1 < self.reglages.tentatives_max => {
                    Sortie::Echec(e.code())
                }
                None if delai_depasse && tache.tentatives + 1 < self.reglages.tentatives_max => {
                    Sortie::Echec("delai_depasse")
                }
                Some(_) => Sortie::Refus("abandon"),
                None if delai_depasse => Sortie::Refus("abandon"),
                None if envoyes > 0 => Sortie::Envoye,
                None if refus => Sortie::Refus("smtp_refus"),
                None => Sortie::SansObjet,
            }
        };
        let maintenant = self.horloge.maintenant();
        let mut tx = ouvrir_transaction_travailleur(&self.pool).await?;
        match &sortie {
            Sortie::Envoye => {
                tx.terminer(tache.id, tache.prise, "envoye", maintenant)
                    .await?;
            }
            Sortie::SansObjet | Sortie::Invalide | Sortie::Pas(_) => {
                tx.terminer(tache.id, tache.prise, "sans_objet", maintenant)
                    .await?;
            }
            Sortie::Refus(code) => {
                tx.terminer(tache.id, tache.prise, code, maintenant).await?;
            }
            Sortie::Echec(code) => {
                tx.reporter(
                    tache.id,
                    tache.prise,
                    code,
                    maintenant + delai_de_rejeu(tache.tentatives),
                    maintenant,
                )
                .await?;
            }
        }
        tx.confirmer().await?;
        if let Sortie::Echec(code) | Sortie::Refus(code) = &sortie {
            tracing::warn!(target: CIBLE, issue = *code, "envoi d'alerte non abouti");
        }
        Ok(sortie)
    }

    /// Boucle **supervisée** : une panique est journalisée (sans détail) et la boucle relancée.
    /// Ne rend jamais la main : à lancer dans une tâche abandonnée à l'arrêt du serveur.
    pub async fn superviser(&self) {
        loop {
            let boucle = self.clone();
            let mut issue = AbandonAuDrop(tokio::spawn(async move {
                boucle.boucler(std::future::pending()).await;
            }));
            // `boucler` ne se termine pas : toute fin est une panique ou une annulation.
            let _ = (&mut issue.0).await;
            tracing::error!(
                target: CIBLE,
                erreur = "envoi_courriels",
                "boucle d'envoi des courriels arrêtée, relance"
            );
            tokio::time::sleep(self.reglages.relance).await;
        }
    }

    /// Boucle jusqu'à ce que `arret` se termine.
    pub async fn boucler(&self, arret: impl std::future::Future<Output = ()>) {
        tokio::pin!(arret);
        loop {
            match self.passer().await {
                Ok(bilan) if bilan.prises > 0 => tracing::info!(
                    target: CIBLE,
                    prises = bilan.prises,
                    envoyes = bilan.envoyes,
                    reportes = bilan.reportes,
                    erreurs = bilan.erreurs,
                    "passage de la file des courriels"
                ),
                Ok(_) => {}
                Err(_) => tracing::error!(
                    target: CIBLE,
                    erreur = "base",
                    "passage de la file des courriels interrompu"
                ),
            }
            tokio::select! {
                () = &mut arret => return,
                () = tokio::time::sleep(self.reglages.periode) => {}
            }
        }
    }

    /// Émet le lien et compose le message, dans la transaction du cabinet de la tâche.
    async fn preparer(
        &self,
        tache: &CourrielPris,
        maintenant: OffsetDateTime,
    ) -> Result<Preparation, ErreurBase> {
        let contexte = ContexteTenant::cabinet(
            IdCabinet::depuis_uuid(tache.cabinet),
            IdUtilisateur::nouveau(),
        );
        let mut tx = ouvrir_transaction(&self.pool, contexte).await?;
        let preparation = match tache.sujet {
            SujetCourriel::Invitation => match lire_invitation(&mut tx, tache.objet).await? {
                Some(i) if i.etat == "en_attente" => {
                    let jeton =
                        emettre(&mut tx, NatureLien::Invitation, tache.objet, maintenant).await?;
                    invitation(
                        &i.courriel,
                        &i.nom,
                        &adresse_du_lien(&self.origine, &jeton.texte()),
                    )
                    .map_or(Preparation::Invalide, Preparation::Message)
                }
                _ => Preparation::SansObjet,
            },
            SujetCourriel::Reinitialisation => {
                match lire_reinitialisation(&mut tx, tache.objet).await? {
                    Some(r) if r.etat == "en_attente" && r.echeance > maintenant => {
                        Preparation::Pas(r.echeance)
                    }
                    Some(r) if r.etat == "en_attente" => {
                        match lire_membre(&mut tx, r.utilisateur).await? {
                            Some(m) if m.actif => {
                                let jeton = emettre(
                                    &mut tx,
                                    NatureLien::Reinitialisation,
                                    tache.objet,
                                    maintenant,
                                )
                                .await?;
                                reinitialisation(
                                    &m.courriel,
                                    &m.nom,
                                    &adresse_du_lien(&self.origine, &jeton.texte()),
                                )
                                .map_or(Preparation::Invalide, Preparation::Message)
                            }
                            _ => Preparation::SansObjet,
                        }
                    }
                    _ => Preparation::SansObjet,
                }
            }
        };
        tx.confirmer().await?;
        Ok(preparation)
    }

    async fn traiter(&self, tache: CourrielPris) -> Result<Sortie, ErreurBase> {
        // Erreur de base : la tâche reste prise, reprise au bail ; rien à écrire.
        let maintenant = self.horloge.maintenant();
        let sortie = match self.preparer(&tache, maintenant).await? {
            Preparation::SansObjet => Sortie::SansObjet,
            Preparation::Invalide => Sortie::Invalide,
            Preparation::Pas(instant) => Sortie::Pas(instant),
            Preparation::Message(message) => {
                // Hors de toute transaction : l'échange SMTP peut durer, mais pas plus que le
                // budget de la tâche (inférieur au bail).
                let envoi =
                    tokio::time::timeout(self.reglages.budget, self.transport.envoyer(&message))
                        .await
                        .unwrap_or(Err(super::transport::ErreurEnvoi::Temporaire));
                match envoi {
                    Ok(()) => Sortie::Envoye,
                    Err(e)
                        if e.a_rejouer() && tache.tentatives + 1 < self.reglages.tentatives_max =>
                    {
                        Sortie::Echec(e.code())
                    }
                    Err(e) if e.a_rejouer() => Sortie::Refus("abandon"),
                    Err(e) => Sortie::Refus(e.code()),
                }
            }
        };
        let maintenant = self.horloge.maintenant();
        let mut tx = ouvrir_transaction_travailleur(&self.pool).await?;
        match &sortie {
            Sortie::Envoye => {
                tx.terminer(tache.id, tache.prise, "envoye", maintenant)
                    .await?;
            }
            Sortie::SansObjet => {
                tx.terminer(tache.id, tache.prise, "sans_objet", maintenant)
                    .await?;
            }
            Sortie::Invalide => {
                tx.terminer(tache.id, tache.prise, "message_invalide", maintenant)
                    .await?;
            }
            Sortie::Refus(code) => {
                tx.terminer(tache.id, tache.prise, code, maintenant).await?;
            }
            Sortie::Pas(instant) => {
                tx.reporter(
                    tache.id,
                    tache.prise,
                    "delai_en_cours",
                    *instant,
                    maintenant,
                )
                .await?;
            }
            Sortie::Echec(code) => {
                tx.reporter(
                    tache.id,
                    tache.prise,
                    code,
                    maintenant + delai_de_rejeu(tache.tentatives),
                    maintenant,
                )
                .await?;
            }
        }
        tx.confirmer().await?;
        if let Sortie::Echec(code) | Sortie::Refus(code) = &sortie {
            tracing::warn!(target: CIBLE, issue = *code, "envoi de courriel non abouti");
        }
        Ok(sortie)
    }
}

/// Abandonne la tâche si la supervision est abandonnée : la boucle ne lui survit pas.
struct AbandonAuDrop(tokio::task::JoinHandle<()>);

impl Drop for AbandonAuDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn le_rejeu_double_puis_plafonne() {
        let delais: Vec<i64> = (0..9).map(|n| delai_de_rejeu(n).whole_minutes()).collect();
        assert_eq!(delais, [1, 2, 4, 8, 16, 32, 60, 60, 60]);
        assert_eq!(delai_de_rejeu(-3), time::Duration::minutes(1));
    }
}
