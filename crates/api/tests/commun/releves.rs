//! Aides de test du module de routes `releves` (lot L3, PR 12a et 12b) : squelette posé par la PR 6
//! (L3-14), à remplir par la PR 12a et 12b sans toucher aux autres fichiers de `commun`.

/// Corps d'exemple des mutations du module `releves` (`commun::serveur::corps_de`) : aucune route
/// encore, donc aucun cas.
pub fn corps(cas: api::serveur::routes::releves::Cas) -> &'static str {
    match cas {}
}
