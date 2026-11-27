//! Aides de test du module de routes `rapprochement` (lot L3, PR 13a et 13b) : squelette posé par la PR 6
//! (L3-14), à remplir par la PR 13a et 13b sans toucher aux autres fichiers de `commun`.

/// Corps d'exemple des mutations du module `rapprochement` (`commun::serveur::corps_de`) : aucune route
/// encore, donc aucun cas.
pub fn corps(cas: api::serveur::routes::rapprochement::Cas) -> &'static str {
    match cas {}
}
