//! Aides de test du module de routes `prelevements` (lot L3, PR 14) : squelette posé par la PR 6
//! (L3-14), à remplir par la PR 14 sans toucher aux autres fichiers de `commun`.

/// Corps d'exemple des mutations du module `prelevements` (`commun::serveur::corps_de`) : aucune route
/// encore, donc aucun cas.
pub fn corps(cas: api::serveur::routes::prelevements::Cas) -> &'static str {
    match cas {}
}
