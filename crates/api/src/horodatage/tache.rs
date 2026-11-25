//! Tâche d'horodatage de la file (L2-30, PR 15a ; ADR 0010 ; menaces F2.5, F6.2).
//!
//! Un [`Travailleur`] repasse périodiquement : il prend les tâches d'horodatage échues
//! ([`crate::base::taches`]), et pour chacune
//! 1. lit la tête de chaîne et le dernier jeton du dossier (transaction de lecture) et applique
//!    la règle d'`integrite` ([`integrite::horodatage::horodatage_requis`]) : rien à faire si le
//!    dernier jeton couvre déjà la chaîne ;
//! 2. fait horodater le témoin **hors de toute transaction** (l'appel à l'autorité peut durer
//!    des dizaines de secondes) ;
//! 3. enregistre le jeton (ajout seul) et son inscription à l'audit du dossier, acteur
//!    « système », puis termine la tâche.
//!
//! **Rejeu différé** : si toutes les autorités échouent, la tâche est remise en attente avec un
//! délai croissant ; la tentative suivante porte sur la tête du moment et couvre aussi les
//! écritures validées entre-temps. **Reprise après arrêt** : une tâche prise a un bail ; passé ce
//! délai, un autre passage la reprend. L'enregistrement du jeton et la fin de la tâche sont deux
//! transactions : un arrêt entre les deux laisse un jeton valide et une tâche rejouée, que la
//! règle écarte (« sans objet »).
//!
//! **Retard** (F6.2) : toute écriture non couverte a une tâche non terminée, créée à la première
//! validation qui a suivi le dernier jeton ; une tâche non terminée depuis plus de 24 h crée une
//! notification `retard_horodatage` pour le cabinet, et un avertissement technique.
//!
//! Ne contient aucune donnée comptable : le témoin est celui d'`integrite` (identifiant du
//! dossier, tête, nombre) ; seule son empreinte part vers l'autorité.

use std::sync::Arc;
use std::time::Duration;

use integrite::audit::{Acteur, Evenement, Ressource, Symbole, Texte, Uuid as UuidAudit};
use integrite::horodatage::{Declencheur, DernierJeton, Temoin, horodatage_requis};
use sqlx::PgPool;
use time::{Date, OffsetDateTime, UtcOffset};

use super::{ErreurHorodatage, Horodateur, tirer_nonce};
use crate::base::audit::{PorteeAudit, inscrire};
use crate::base::jetons::{NouveauJeton, dernier_jeton, inserer_jeton, lire_tete};
use crate::base::taches::{
    SujetHorodatage, SujetNotification, TachePrise, notifier, ouvrir_transaction_travailleur,
};
use crate::base::{
    ContexteTenant, ErreurBase, IdCabinet, IdDossier, ouvrir_transaction,
    ouvrir_transaction_lecture,
};
use crate::serveur::journal::CIBLE;
use crate::serveur::session::Horloge;

/// Réglages du travailleur.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Reglages {
    /// Délai entre deux passages de la boucle.
    pub periode: Duration,
    /// Bail d'une tâche prise : plus long que le pire échange (autorités × délai total).
    pub bail: time::Duration,
    /// Tâches prises par passage.
    pub lot: i64,
    /// Retard au-delà duquel une alerte est créée (F6.2).
    pub delai_alerte: time::Duration,
    /// Conservation des tâches d'horodatage terminées.
    pub conservation: time::Duration,
    /// Attente avant de relancer la boucle après son arrêt inattendu (panique).
    pub relance: Duration,
}

impl Default for Reglages {
    fn default() -> Self {
        Self {
            periode: Duration::from_secs(60),
            bail: time::Duration::minutes(5),
            lot: 20,
            delai_alerte: time::Duration::hours(24),
            conservation: time::Duration::days(30),
            relance: Duration::from_secs(5),
        }
    }
}

/// Bilan d'un passage.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Bilan {
    /// Tâches prises.
    pub prises: u32,
    /// Jetons obtenus et enregistrés.
    pub jetons: u32,
    /// Tâches remises à plus tard (échec des autorités, ou un jeton déjà obtenu ce jour).
    pub reportees: u32,
    /// Tâches sans objet (chaîne déjà couverte).
    pub sans_objet: u32,
    /// Tâches en erreur interne (base) : reprises au bail.
    pub erreurs: u32,
    /// Dossiers nouvellement alertés d'un retard.
    pub alertes: u32,
}

/// Travailleur de la file d'horodatage.
#[derive(Debug, Clone)]
pub struct Travailleur {
    pool: PgPool,
    horodateur: Arc<dyn Horodateur>,
    horloge: Arc<dyn Horloge>,
    reglages: Reglages,
}

fn jour_utc(instant: OffsetDateTime) -> Date {
    instant.to_offset(UtcOffset::UTC).date()
}

/// Début du jour suivant (UTC).
fn lendemain(instant: OffsetDateTime) -> OffsetDateTime {
    jour_utc(instant)
        .next_day()
        .unwrap_or(Date::MAX)
        .midnight()
        .assume_utc()
}

/// Délai avant le rejeu après `tentatives` échecs : une minute, doublée à chaque échec, une
/// heure au plus.
fn delai_de_rejeu(tentatives: i32) -> time::Duration {
    let doublements = u32::try_from(tentatives).unwrap_or(0).min(6);
    time::Duration::minutes(1 << doublements).min(time::Duration::hours(1))
}

impl Travailleur {
    /// Travailleur sur ce pool (rôle applicatif), cet horodateur et cette horloge.
    #[must_use]
    pub fn new(
        pool: PgPool,
        horodateur: Arc<dyn Horodateur>,
        horloge: Arc<dyn Horloge>,
        reglages: Reglages,
    ) -> Self {
        Self {
            pool,
            horodateur,
            horloge,
            reglages,
        }
    }

    /// Un passage : tâches échues, alertes de retard, purge.
    ///
    /// # Errors
    /// Erreur de base lors de la prise des tâches, des alertes ou de la purge. L'erreur d'une
    /// tâche n'interrompt pas les autres : elle est comptée dans [`Bilan::erreurs`].
    pub async fn passer(&self) -> Result<Bilan, ErreurBase> {
        let mut bilan = Bilan::default();
        // Une tâche à la fois : le bail ne couvre que celle qui est en cours (un lot de tâches
        // lentes ne laisse pas expirer le bail des dernières), et deux instances se partagent
        // le travail tâche par tâche.
        for _ in 0..self.reglages.lot {
            let mut tx = ouvrir_transaction_travailleur(&self.pool).await?;
            let mut prises = tx
                .prendre_echues(self.horloge.maintenant(), self.reglages.bail, 1)
                .await?;
            tx.confirmer().await?;
            let Some(tache) = prises.pop() else { break };
            bilan.prises += 1;
            match self.traiter(tache).await {
                Ok(Issue::Jeton) => bilan.jetons += 1,
                Ok(Issue::Reportee) => bilan.reportees += 1,
                Ok(Issue::SansObjet) => bilan.sans_objet += 1,
                Err(_) => bilan.erreurs += 1,
            }
        }
        let maintenant = self.horloge.maintenant();
        let mut tx = ouvrir_transaction_travailleur(&self.pool).await?;
        let alertes = tx
            .signaler_retards(maintenant, self.reglages.delai_alerte)
            .await?;
        tx.purger(maintenant - self.reglages.conservation).await?;
        tx.confirmer().await?;
        bilan.alertes = u32::try_from(alertes.len()).unwrap_or(u32::MAX);
        if bilan.alertes > 0 {
            tracing::warn!(
                target: CIBLE,
                alertes = bilan.alertes,
                "horodatage en retard de plus de 24 h"
            );
        }
        Ok(bilan)
    }

    /// Boucle **supervisée** : si la boucle s'arrête de façon inattendue (panique), l'arrêt est
    /// journalisé (erreur, sans détail) et la boucle est relancée après [`Reglages::relance`].
    /// Ne rend jamais la main : à lancer dans une tâche que l'on abandonne à l'arrêt du serveur.
    pub async fn superviser(&self) {
        loop {
            let boucle = self.clone();
            let mut issue = AbandonAuDrop(tokio::spawn(async move {
                boucle.boucler(std::future::pending()).await;
            }));
            // `boucler` ne se termine pas : toute fin est une panique ou une annulation.
            // Si la supervision est abandonnée, la boucle l'est avec elle (garde).
            let _ = (&mut issue.0).await;
            tracing::error!(
                target: CIBLE,
                erreur = "horodatage",
                "boucle d'horodatage arrêtée, relance"
            );
            tokio::time::sleep(self.reglages.relance).await;
        }
    }

    /// Boucle jusqu'à ce que `arret` se termine. Une erreur de passage est journalisée (code
    /// seul) et la boucle continue.
    pub async fn boucler(&self, arret: impl std::future::Future<Output = ()>) {
        tokio::pin!(arret);
        loop {
            if let Ok(bilan) = self.passer().await {
                tracing::info!(
                    target: CIBLE,
                    prises = bilan.prises,
                    jetons = bilan.jetons,
                    reportees = bilan.reportees,
                    erreurs = bilan.erreurs,
                    "passage de la file d'horodatage"
                );
            } else {
                tracing::error!(
                    target: CIBLE,
                    erreur = "base",
                    "passage de la file d'horodatage interrompu"
                );
            }
            tokio::select! {
                () = &mut arret => return,
                () = tokio::time::sleep(self.reglages.periode) => {}
            }
        }
    }

    async fn traiter(&self, tache: TachePrise) -> Result<Issue, ErreurBase> {
        // Erreur de base : la tâche reste prise, reprise au bail ; rien à écrire.
        let resultat = self.horodater(&tache).await?;
        let maintenant = self.horloge.maintenant();
        let mut tx = ouvrir_transaction_travailleur(&self.pool).await?;
        let issue = match resultat {
            Sortie::Jeton => {
                tx.terminer(tache.id, tache.prise, "jeton_obtenu", maintenant)
                    .await?;
                Issue::Jeton
            }
            Sortie::SansObjet => {
                tx.terminer(tache.id, tache.prise, "sans_objet", maintenant)
                    .await?;
                Issue::SansObjet
            }
            Sortie::Lendemain => {
                tx.reporter(
                    tache.id,
                    tache.prise,
                    "jeton_du_jour",
                    lendemain(maintenant),
                    maintenant,
                )
                .await?;
                Issue::Reportee
            }
            Sortie::Echec(issue) => {
                tx.reporter(
                    tache.id,
                    tache.prise,
                    issue,
                    maintenant + delai_de_rejeu(tache.tentatives),
                    maintenant,
                )
                .await?;
                Issue::Reportee
            }
        };
        tx.confirmer().await?;
        Ok(issue)
    }

    async fn horodater(&self, tache: &TachePrise) -> Result<Sortie, ErreurBase> {
        let contexte = ContexteTenant::systeme(
            IdCabinet::depuis_uuid(tache.cabinet),
            IdDossier::depuis_uuid(tache.dossier),
        );
        // 1. État du dossier, sans verrou.
        let mut lecture = ouvrir_transaction_lecture(&self.pool, contexte).await?;
        let tete = lire_tete(&mut lecture).await?;
        let dernier = dernier_jeton(&mut lecture).await?;
        lecture.annuler().await?;
        let Some(tete) = tete.filter(|t| t.rang > 0) else {
            return Ok(Sortie::SansObjet);
        };
        let maintenant = self.horloge.maintenant();
        let longueur = u64::try_from(tete.rang).map_err(|_| ErreurBase::Donnee("rang"))?;
        let dernier_regle = dernier
            .map(|d| {
                Ok::<_, ErreurBase>(DernierJeton {
                    jour: jour_utc(d.obtenu_le),
                    nombre: u64::try_from(d.nombre).map_err(|_| ErreurBase::Donnee("nombre"))?,
                })
            })
            .transpose()?;
        let declencheur = match tache.sujet {
            SujetHorodatage::Quotidien => Declencheur::Quotidien(jour_utc(maintenant)),
            SujetHorodatage::Verrouillage => Declencheur::VerrouillagePeriode,
            SujetHorodatage::Cloture => Declencheur::Cloture,
        };
        if !horodatage_requis(dernier_regle, longueur, declencheur) {
            // Couverte : rien. Non couverte mais un jeton déjà obtenu aujourd'hui : demain.
            let couverte = dernier_regle.is_some_and(|d| d.nombre >= longueur);
            return Ok(if couverte {
                Sortie::SansObjet
            } else {
                Sortie::Lendemain
            });
        }
        // 2. Appel à l'autorité, hors de toute transaction.
        let dossier = integrite::IdDossier::parse(&tache.dossier.to_string())
            .map_err(|_| ErreurBase::Donnee("identifiant de dossier"))?;
        let temoin = Temoin::nouveau(dossier, tete.empreinte, longueur);
        let Ok(nonce) = tirer_nonce() else {
            return Ok(Sortie::Echec("alea"));
        };
        let obtenu = match self.horodateur.horodater(&temoin, nonce).await {
            Ok(obtenu) => obtenu,
            Err(ErreurHorodatage::ToutesEnEchec(_)) => return Ok(Sortie::Echec("autorites")),
            Err(ErreurHorodatage::Requete(_)) => return Ok(Sortie::Echec("requete")),
        };
        // 3. Jeton et audit, dans une même transaction.
        let instant = self.horloge.maintenant();
        let mut tx = ouvrir_transaction(&self.pool, contexte).await?;
        let nom = obtenu.autorite.nom();
        let id = inserer_jeton(
            &mut tx,
            &NouveauJeton {
                nombre: tete.rang,
                tete: tete.empreinte,
                autorite: &nom,
                date_jeton: obtenu.verifie.date,
                obtenu_le: instant,
                declencheur: tache.sujet.nom(),
                der: &obtenu.jeton,
            },
        )
        .await?;
        let invalide = || ErreurBase::Audit("événement de jeton invalide");
        let evenement = Evenement::new(
            Acteur::Systeme,
            Symbole::parse("ObtenirJetonHorodatage").map_err(|_| invalide())?,
            Ressource {
                genre: Symbole::parse("jeton").map_err(|_| invalide())?,
                id: UuidAudit::depuis_octets(*id.as_bytes()),
            },
        );
        let donnee = |e: Evenement, nom: &str, valeur: &str| {
            let (nom, valeur) = (
                Symbole::parse(nom).map_err(|_| invalide())?,
                Texte::parse(valeur).map_err(|_| invalide())?,
            );
            e.avec_donnee(nom, valeur).map_err(|_| invalide())
        };
        let evenement = donnee(evenement, "nombre", &tete.rang.to_string())?;
        let evenement = donnee(evenement, "declencheur", tache.sujet.nom())?;
        inscrire(
            &mut tx,
            PorteeAudit::Dossier(IdDossier::depuis_uuid(tache.dossier)),
            instant,
            evenement,
        )
        .await?;
        if matches!(
            tache.sujet,
            SujetHorodatage::Verrouillage | SujetHorodatage::Cloture
        ) {
            // Le témoin est remis au cabinet à chaque verrouillage et clôture (question 9b).
            notifier(&mut tx, SujetNotification::Temoin, None).await?;
        }
        tx.confirmer().await?;
        Ok(Sortie::Jeton)
    }
}

/// Abandonne la tâche si la supervision est abandonnée : la boucle ne lui survit pas.
struct AbandonAuDrop(tokio::task::JoinHandle<()>);

impl Drop for AbandonAuDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

enum Sortie {
    Jeton,
    SansObjet,
    Lendemain,
    Echec(&'static str),
}

enum Issue {
    Jeton,
    Reportee,
    SansObjet,
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::datetime;

    #[test]
    fn le_lendemain_est_le_debut_du_jour_suivant_en_utc() {
        assert_eq!(
            lendemain(datetime!(2026-11-24 23:59:59 UTC)),
            datetime!(2026-11-25 00:00 UTC)
        );
        // Un instant à +01:00 est lu en UTC.
        assert_eq!(
            lendemain(datetime!(2026-11-25 00:30 +1)),
            datetime!(2026-11-25 00:00 UTC)
        );
    }

    #[test]
    fn le_rejeu_double_puis_plafonne() {
        let delais: Vec<i64> = (0..9).map(|n| delai_de_rejeu(n).whole_minutes()).collect();
        assert_eq!(delais, [1, 2, 4, 8, 16, 32, 60, 60, 60]);
        assert_eq!(delai_de_rejeu(-3), time::Duration::minutes(1));
    }
}
