//! Aides de test du module de routes `echeancier` (lot L3, PR 8) : squelette posé par la PR 6
//! (L3-14), à remplir par la PR 8 sans toucher aux autres fichiers de `commun`.

/// Corps d'exemple des mutations du module `echeancier` (`commun::serveur::corps_de`) : aucune route
/// encore, donc aucun cas.
pub fn corps(cas: api::serveur::routes::echeancier::Cas) -> &'static str {
    match cas {}
}
