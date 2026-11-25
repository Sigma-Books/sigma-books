//! Envoi des liens par la file de tâches (L2-19, PR 9b ; ADR 0012, 0022, 0024).
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

use super::modele::{Message, adresse_du_lien, invitation, reinitialisation};
use super::transport::Transport;
use crate::base::administration::{lire_invitation, lire_membre, lire_reinitialisation};
use crate::base::liens::{NatureLien, emettre};
use crate::base::taches::{CourrielPris, SujetCourriel, ouvrir_transaction_travailleur};
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
            match self.traiter(tache).await {
                Ok(Sortie::Envoye) => bilan.envoyes += 1,
                Ok(Sortie::Echec(_) | Sortie::Pas(_)) => bilan.reportes += 1,
                Ok(Sortie::SansObjet | Sortie::Invalide | Sortie::Refus(_)) => bilan.clos += 1,
                Err(_) => bilan.erreurs += 1,
            }
        }
        Ok(bilan)
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
                // Hors de toute transaction : l'échange SMTP peut durer.
                match self.transport.envoyer(&message).await {
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
