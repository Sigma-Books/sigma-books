//! Aides de test du module de routes `tresorerie` (lot L3, PR 11) : squelette posé par la PR 6
//! (L3-14), à remplir par la PR 11 sans toucher aux autres fichiers de `commun`.

/// Corps d'exemple des mutations du module `tresorerie` (`commun::serveur::corps_de`) : aucune route
/// encore, donc aucun cas.
pub fn corps(cas: api::serveur::routes::tresorerie::Cas) -> &'static str {
    match cas {}
}
