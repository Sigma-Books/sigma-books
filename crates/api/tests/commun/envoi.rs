//! Transport de test des courriels (PR 9b) : mémoire, avec échecs programmables. Il n'existe pas
//! dans les sources du crate : le binaire ne peut pas le sélectionner (`tests/structure.rs`).

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use api::envoi::modele::Message;
use api::envoi::transport::{ErreurEnvoi, FuturEnvoi, Transport};

#[derive(Debug, Default)]
struct Etat {
    lenteur: Option<std::time::Duration>,
    remis: Vec<Message>,
    /// Résultats programmés, consommés dans l'ordre ; vide : succès.
    resultats: VecDeque<Result<(), ErreurEnvoi>>,
    tentatives: u32,
}

/// Transport en mémoire, partageable.
#[derive(Debug, Clone, Default)]
pub struct TransportMemoire(Arc<Mutex<Etat>>);

impl TransportMemoire {
    /// Programme les résultats des prochains envois.
    pub fn programmer(&self, resultats: impl IntoIterator<Item = Result<(), ErreurEnvoi>>) {
        self.0.lock().unwrap().resultats.extend(resultats);
    }

    /// Messages effectivement remis.
    pub fn remis(&self) -> Vec<Message> {
        self.0.lock().unwrap().remis.clone()
    }

    /// Même mémoire, avec une lenteur simulée à chaque envoi (budget de temps).
    pub fn avec_lenteur(&self, duree: std::time::Duration) -> Self {
        self.0.lock().unwrap().lenteur = Some(duree);
        self.clone()
    }

    /// Vrai si aucun message n'a été remis.
    pub fn vide(&self) -> bool {
        self.0.lock().unwrap().remis.is_empty()
    }

    /// Nombre d'appels au transport (réussis ou non).
    pub fn tentatives(&self) -> u32 {
        self.0.lock().unwrap().tentatives
    }
}

impl Transport for TransportMemoire {
    fn envoyer<'a>(&'a self, message: &'a Message) -> FuturEnvoi<'a> {
        Box::pin(async move {
            let lenteur = self.0.lock().unwrap().lenteur;
            if let Some(d) = lenteur {
                tokio::time::sleep(d).await;
            }
            let mut etat = self.0.lock().unwrap();
            etat.tentatives += 1;
            match etat.resultats.pop_front().unwrap_or(Ok(())) {
                Ok(()) => {
                    etat.remis.push(message.clone());
                    Ok(())
                }
                Err(e) => Err(e),
            }
        })
    }
}

/// Jeton d'un lien contenu dans le corps d'un message (`…/lien#<jeton>`).
pub fn jeton_du_message(message: &Message) -> String {
    let apres = message
        .texte
        .split_once("/lien#")
        .expect("lien dans le message")
        .1;
    apres
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .collect()
}
