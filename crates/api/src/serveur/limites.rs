//! Limitation de débit en mémoire (F1.9 ; ASVS 4.0.3 V11.1.4).
//!
//! Fenêtre fixe par clé : au plus `capacite` requêtes par `periode`. Deux limiteurs : par adresse
//! du client (avant toute autre étape) et par utilisateur authentifié (juste après
//! l'authentification). Simple et local à une instance : la limitation globale et la protection
//! contre une inondation distribuée relèvent du mandataire inverse (Fondations §3.3, L9).
//!
//! Mémoire bornée : au-delà de `cles_max` clés, les fenêtres échues sont purgées ; si la table
//! reste pleine, toute **nouvelle** clé est refusée (refus en cas de doute) jusqu'à la purge
//! suivante, tandis que les clés déjà connues gardent leur quota.
//!
//! Adresse du client : celle de la connexion TCP. Derrière un mandataire inverse (L9), elle
//! viendra d'un en-tête de confiance configurable (`SIGMA_EN_TETE_ADRESSE_CLIENT`), **désactivé
//! par défaut** : sans mandataire, un client forgerait cet en-tête pour changer de quota.
//! Une adresse IPv6 compte pour son préfixe /64, attribué d'ordinaire à un seul abonné.

use std::collections::HashMap;
use std::hash::Hash;
use std::net::{IpAddr, Ipv6Addr, SocketAddr};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use axum::extract::ConnectInfo;
use axum::http::{HeaderMap, HeaderName, Request};

/// Réglage d'un limiteur.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReglageDebit {
    /// Requêtes admises par période.
    pub capacite: u32,
    /// Durée de la fenêtre.
    pub periode: Duration,
    /// Nombre maximal de clés suivies.
    pub cles_max: usize,
}

#[derive(Debug, Clone, Copy)]
struct Fenetre {
    debut: Instant,
    compte: u32,
}

/// Limiteur de débit à fenêtre fixe, par clé.
#[derive(Debug)]
pub struct LimiteurDebit<K> {
    reglage: ReglageDebit,
    fenetres: Mutex<HashMap<K, Fenetre>>,
}

impl<K: Eq + Hash + Copy> LimiteurDebit<K> {
    /// Nouveau limiteur, vide.
    #[must_use]
    pub fn new(reglage: ReglageDebit) -> Self {
        Self {
            reglage,
            fenetres: Mutex::new(HashMap::new()),
        }
    }

    /// Compte une requête de `cle` à l'instant `maintenant`. `Err(secondes)` si la limite est
    /// atteinte : délai avant la fin de la fenêtre, arrondi au-dessus (au moins 1).
    ///
    /// # Errors
    /// Le délai avant de réessayer, en secondes.
    pub fn verifier(&self, cle: K, maintenant: Instant) -> Result<(), u64> {
        let periode = self.reglage.periode;
        // Un verrou empoisonné ne protège qu'un compteur : on reprend son contenu.
        let mut fenetres = self.fenetres.lock().unwrap_or_else(PoisonError::into_inner);
        if !fenetres.contains_key(&cle) && fenetres.len() >= self.reglage.cles_max {
            fenetres.retain(|_, f| maintenant.saturating_duration_since(f.debut) < periode);
            if fenetres.len() >= self.reglage.cles_max {
                return Err(secondes_restantes(periode));
            }
        }
        let fenetre = fenetres.entry(cle).or_insert(Fenetre {
            debut: maintenant,
            compte: 0,
        });
        let ecoule = maintenant.saturating_duration_since(fenetre.debut);
        if ecoule >= periode {
            *fenetre = Fenetre {
                debut: maintenant,
                compte: 0,
            };
        }
        if fenetre.compte >= self.reglage.capacite {
            return Err(secondes_restantes(periode.saturating_sub(
                maintenant.saturating_duration_since(fenetre.debut),
            )));
        }
        fenetre.compte += 1;
        Ok(())
    }
}

fn secondes_restantes(duree: Duration) -> u64 {
    let secondes = duree.as_secs() + u64::from(duree.subsec_nanos() > 0);
    secondes.max(1)
}

/// Clé du limiteur par adresse : l'adresse IPv4, ou le préfixe /64 d'une adresse IPv6.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CleAdresse {
    /// Adresse inconnue (aucune information de connexion) : un seul quota partagé.
    Inconnue,
    /// Adresse IP, IPv6 réduite à son /64.
    Ip(IpAddr),
}

impl CleAdresse {
    fn depuis(ip: IpAddr) -> Self {
        let ip = match ip {
            IpAddr::V6(v6) => {
                if let Some(v4) = v6.to_ipv4_mapped() {
                    IpAddr::V4(v4)
                } else {
                    let s = v6.segments();
                    IpAddr::V6(Ipv6Addr::new(s[0], s[1], s[2], s[3], 0, 0, 0, 0))
                }
            }
            v4 @ IpAddr::V4(_) => v4,
        };
        Self::Ip(ip)
    }
}

/// Adresse du client d'une requête : en-tête de confiance s'il est configuré (dernière adresse
/// valide de la liste, celle qu'ajoute le mandataire), sinon adresse de la connexion.
pub fn adresse_client<B>(
    requete: &Request<B>,
    en_tete_confiance: Option<&HeaderName>,
) -> CleAdresse {
    if let Some(nom) = en_tete_confiance
        && let Some(ip) = derniere_adresse(requete.headers(), nom)
    {
        return CleAdresse::depuis(ip);
    }
    requete
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map_or(CleAdresse::Inconnue, |ConnectInfo(pair)| {
            CleAdresse::depuis(pair.ip())
        })
}

fn derniere_adresse(entetes: &HeaderMap, nom: &HeaderName) -> Option<IpAddr> {
    entetes
        .get_all(nom)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(','))
        .filter_map(|v| v.trim().parse::<IpAddr>().ok())
        .next_back()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reglage(capacite: u32, cles_max: usize) -> ReglageDebit {
        ReglageDebit {
            capacite,
            periode: Duration::from_secs(60),
            cles_max,
        }
    }

    #[test]
    fn la_fenetre_borne_puis_se_renouvelle() {
        let limiteur = LimiteurDebit::new(reglage(2, 10));
        let t0 = Instant::now();
        assert_eq!(limiteur.verifier(1, t0), Ok(()));
        assert_eq!(limiteur.verifier(1, t0), Ok(()));
        assert_eq!(limiteur.verifier(1, t0 + Duration::from_secs(10)), Err(50));
        // Une autre clé a son propre quota.
        assert_eq!(limiteur.verifier(2, t0), Ok(()));
        assert_eq!(limiteur.verifier(1, t0 + Duration::from_secs(60)), Ok(()));
    }

    #[test]
    fn la_table_pleine_refuse_les_nouvelles_cles_jusqu_a_la_purge() {
        let limiteur = LimiteurDebit::new(reglage(5, 2));
        let t0 = Instant::now();
        assert!(limiteur.verifier(1, t0).is_ok());
        assert!(limiteur.verifier(2, t0).is_ok());
        assert!(limiteur.verifier(3, t0).is_err());
        assert!(limiteur.verifier(1, t0).is_ok(), "clé connue");
        assert!(limiteur.verifier(3, t0 + Duration::from_secs(61)).is_ok());
    }

    #[test]
    fn l_adresse_vient_de_la_connexion_sauf_en_tete_de_confiance_configure() {
        let pair: SocketAddr = "192.0.2.7:4000".parse().unwrap();
        let mut requete = Request::builder()
            .header("x-forwarded-for", "198.51.100.1, 203.0.113.9")
            .body(())
            .unwrap();
        requete.extensions_mut().insert(ConnectInfo(pair));
        // Par défaut, l'en-tête est ignoré : il serait forgé par le client.
        assert_eq!(
            adresse_client(&requete, None),
            CleAdresse::Ip("192.0.2.7".parse().unwrap())
        );
        let nom = HeaderName::from_static("x-forwarded-for");
        assert_eq!(
            adresse_client(&requete, Some(&nom)),
            CleAdresse::Ip("203.0.113.9".parse().unwrap())
        );
    }

    #[test]
    fn une_adresse_ipv6_compte_pour_son_prefixe() {
        let a: IpAddr = "2001:db8:1:2:aaaa::1".parse().unwrap();
        let b: IpAddr = "2001:db8:1:2:bbbb::2".parse().unwrap();
        assert_eq!(CleAdresse::depuis(a), CleAdresse::depuis(b));
        let mappee: IpAddr = "::ffff:192.0.2.1".parse().unwrap();
        assert_eq!(
            CleAdresse::depuis(mappee),
            CleAdresse::Ip("192.0.2.1".parse().unwrap())
        );
    }
}
