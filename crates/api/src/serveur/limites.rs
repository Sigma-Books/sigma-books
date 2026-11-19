//! Limitation de débit en mémoire (F1.9 ; ASVS 4.0.3 V11.1.4).
//!
//! Fenêtre fixe par clé : au plus `capacite` requêtes par `periode`. Deux limiteurs : par adresse
//! du client (avant toute autre étape) et par utilisateur authentifié (juste après
//! l'authentification). Simple et local à une instance : la limitation globale, le nombre de
//! connexions et la protection contre une inondation distribuée relèvent du mandataire inverse
//! (Fondations §3.3, L9).
//!
//! **Mémoire et coût bornés** : au plus `cles_max` clés. Table pleine, une nouvelle clé déclenche
//! d'abord la purge des fenêtres échues, **au plus une fois par période** (instant de la dernière
//! purge conservé) ; si la table reste pleine, les fenêtres **les plus anciennes** sont évincées
//! dans l'ordre de leur ouverture (file, coût amorti constant). Un nouveau client n'est donc
//! jamais refusé parce que la table est pleine ; en contrepartie, sous une inondation de clés
//! distinctes, une clé évincée retrouve un quota neuf : ce cas relève du mandataire (L9).
//!
//! Adresse du client : celle de la connexion TCP. Derrière un mandataire inverse (L9), elle
//! viendra d'un en-tête de confiance configurable (`SIGMA_EN_TETE_ADRESSE_CLIENT`), **désactivé
//! par défaut** : sans mandataire, un client forgerait cet en-tête pour changer de quota.
//! Une adresse IPv6 compte pour son préfixe /64, attribué d'ordinaire à un seul abonné.

use std::collections::{HashMap, VecDeque};
use std::hash::Hash;
use std::net::{IpAddr, Ipv6Addr, SocketAddr};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use axum::extract::ConnectInfo;
use axum::http::{HeaderName, Request};

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

#[derive(Debug)]
struct Table<K> {
    fenetres: HashMap<K, Fenetre>,
    /// Ouvertures de fenêtres, de la plus ancienne à la plus récente. Une entrée dont la
    /// fenêtre a depuis été renouvelée (début différent) est périmée et ignorée.
    ordre: VecDeque<(K, Instant)>,
    derniere_purge: Option<Instant>,
    /// Entrées examinées par la purge et l'éviction (mesure du coût, pour les tests).
    examinees: u64,
}

/// Limiteur de débit à fenêtre fixe, par clé.
#[derive(Debug)]
pub struct LimiteurDebit<K> {
    reglage: ReglageDebit,
    table: Mutex<Table<K>>,
}

impl<K: Eq + Hash + Copy> LimiteurDebit<K> {
    /// Nouveau limiteur, vide.
    #[must_use]
    pub fn new(reglage: ReglageDebit) -> Self {
        Self {
            reglage,
            table: Mutex::new(Table {
                fenetres: HashMap::new(),
                ordre: VecDeque::new(),
                derniere_purge: None,
                examinees: 0,
            }),
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
        let mut table = self.table.lock().unwrap_or_else(PoisonError::into_inner);
        let table = &mut *table;
        if !table.fenetres.contains_key(&cle) && table.fenetres.len() >= self.reglage.cles_max {
            self.faire_place(table, maintenant);
        }
        let fenetre = table.fenetres.entry(cle).or_insert_with(|| {
            table.ordre.push_back((cle, maintenant));
            Fenetre {
                debut: maintenant,
                compte: 0,
            }
        });
        if maintenant.saturating_duration_since(fenetre.debut) >= periode {
            *fenetre = Fenetre {
                debut: maintenant,
                compte: 0,
            };
            table.ordre.push_back((cle, maintenant));
        }
        if fenetre.compte >= self.reglage.capacite {
            return Err(secondes_restantes(periode.saturating_sub(
                maintenant.saturating_duration_since(fenetre.debut),
            )));
        }
        fenetre.compte += 1;
        Ok(())
    }

    /// Libère au moins une place : purge des fenêtres échues (au plus une fois par période),
    /// puis éviction des plus anciennes.
    fn faire_place(&self, table: &mut Table<K>, maintenant: Instant) {
        let periode = self.reglage.periode;
        let purge_due = table
            .derniere_purge
            .is_none_or(|purge| maintenant.saturating_duration_since(purge) >= periode);
        if purge_due {
            table.derniere_purge = Some(maintenant);
            table.examinees += table.fenetres.len() as u64;
            table
                .fenetres
                .retain(|_, f| maintenant.saturating_duration_since(f.debut) < periode);
            let fenetres = &table.fenetres;
            table
                .ordre
                .retain(|(k, debut)| fenetres.get(k).is_some_and(|f| f.debut == *debut));
        }
        while table.fenetres.len() >= self.reglage.cles_max {
            let Some((k, debut)) = table.ordre.pop_front() else {
                break;
            };
            table.examinees += 1;
            if table.fenetres.get(&k).is_some_and(|f| f.debut == debut) {
                table.fenetres.remove(&k);
            }
        }
    }

    #[cfg(test)]
    fn examinees(&self) -> u64 {
        self.table
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .examinees
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

/// Adresse du client d'une requête : en-tête de confiance s'il est configuré, sinon adresse de
/// la connexion. De l'en-tête, seul compte **strictement le dernier élément** de sa dernière
/// occurrence, celui qu'ajoute le mandataire ; s'il n'est pas une adresse valide, l'adresse de
/// la connexion fait foi (jamais un élément antérieur, que le client peut forger).
pub fn adresse_client<B>(
    requete: &Request<B>,
    en_tete_confiance: Option<&HeaderName>,
) -> CleAdresse {
    if let Some(nom) = en_tete_confiance
        && let Some(ip) = derniere_adresse(requete, nom)
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

fn derniere_adresse(requete: &Request<impl Sized>, nom: &HeaderName) -> Option<IpAddr> {
    let valeur = requete.headers().get_all(nom).iter().next_back()?;
    let dernier = valeur.to_str().ok()?.rsplit(',').next()?;
    dernier.trim().parse().ok()
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
    fn table_pleine_un_nouveau_client_passe_et_le_cout_reste_borne() {
        const CLES: usize = 10_000;
        let limiteur = LimiteurDebit::new(reglage(5, CLES));
        let t0 = Instant::now();
        for cle in 0..CLES {
            assert!(limiteur.verifier(cle, t0).is_ok());
        }
        // Table pleine de fenêtres vivantes : chaque nouveau client passe (éviction des plus
        // anciennes), sans reparcourir la table à chaque fois.
        let avant = limiteur.examinees();
        for cle in CLES..2 * CLES {
            assert!(
                limiteur.verifier(cle, t0 + Duration::from_secs(1)).is_ok(),
                "{cle}"
            );
        }
        let cout = limiteur.examinees() - avant;
        // Une seule purge (CLES entrées) plus une éviction par nouvelle clé.
        assert!(cout <= 3 * CLES as u64, "coût {cout}");
        // Les plus récentes gardent leur fenêtre (et leur compte) ; les plus anciennes sont parties.
        let table = limiteur.table.lock().unwrap();
        assert_eq!(table.fenetres.len(), CLES);
        assert!(table.fenetres.contains_key(&(2 * CLES - 1)));
        assert!(!table.fenetres.contains_key(&0));
    }

    #[test]
    fn la_purge_n_a_lieu_qu_une_fois_par_periode() {
        let limiteur = LimiteurDebit::new(reglage(5, 100));
        let t0 = Instant::now();
        for cle in 0..100 {
            limiteur.verifier(cle, t0).unwrap();
        }
        limiteur.verifier(1000, t0).unwrap();
        let apres_premiere = limiteur.examinees();
        assert_eq!(apres_premiere, 100 + 1, "une purge, une éviction");
        for cle in 1001..1050 {
            limiteur.verifier(cle, t0 + Duration::from_secs(1)).unwrap();
        }
        assert_eq!(
            limiteur.examinees() - apres_premiere,
            49,
            "évictions seules"
        );
        // Période écoulée : nouvelle purge, qui libère les fenêtres échues.
        limiteur
            .verifier(2000, t0 + Duration::from_secs(61))
            .unwrap();
        let table = limiteur.table.lock().unwrap();
        assert!(table.fenetres.len() <= 51, "{}", table.fenetres.len());
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
    fn seul_le_dernier_element_de_l_en_tete_de_confiance_compte() {
        let nom = HeaderName::from_static("x-forwarded-for");
        let connexion = CleAdresse::Ip("192.0.2.7".parse().unwrap());
        let cle = |valeurs: &[&str]| {
            let mut requete = Request::builder();
            for v in valeurs {
                requete = requete.header("x-forwarded-for", *v);
            }
            let mut requete = requete.body(()).unwrap();
            requete
                .extensions_mut()
                .insert(ConnectInfo::<SocketAddr>("192.0.2.7:4000".parse().unwrap()));
            adresse_client(&requete, Some(&nom))
        };
        // Dernier élément invalide ou vide : adresse de la connexion, jamais un élément antérieur
        // (forgeable par le client).
        assert_eq!(cle(&["198.51.100.1, pas-une-adresse"]), connexion);
        assert_eq!(cle(&["198.51.100.1,"]), connexion);
        assert_eq!(cle(&["198.51.100.1", "inconnue"]), connexion);
        assert_eq!(cle(&[]), connexion);
        // Dernière occurrence, dernier élément.
        assert_eq!(
            cle(&["198.51.100.1", "203.0.113.5, 203.0.113.9"]),
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
