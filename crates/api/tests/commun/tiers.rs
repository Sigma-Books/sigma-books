//! Aides de test du module de routes `tiers` (lot L3, PR 7a ; ADR 0031).

use std::sync::OnceLock;

/// Corps d'exemple des mutations du module `tiers` (`commun::serveur::corps_de`). Joués dans
/// l'ordre de la table sur le tiers fixe d'un dossier (isolation, audit), ils réussissent tous :
/// création d'un client, renommage, archivage, désarchivage, une coordonnée bancaire.
pub fn corps(cas: api::serveur::routes::tiers::Cas) -> &'static str {
    use api::serveur::routes::tiers::Cas;
    static CREATION: OnceLock<String> = OnceLock::new();
    match cas {
        Cas::Creer => CREATION.get_or_init(|| {
            format!(
                r#"{{"nom":"Client du test","type":"client","compte_collectif":"{}"}}"#,
                super::referentiel::comptes().collectif
            )
        }),
        Cas::Modifier => r#"{"nom":"Tiers renommé"}"#,
        Cas::Archiver => r#"{"motif":"Plus de relation commerciale"}"#,
        Cas::Desarchiver => "{}",
        Cas::ModifierComptesBancaires => {
            r#"{"comptes":[{"type":"RIB","numero":"SN000 00000 000000000000 00"}]}"#
        }
        Cas::Lister | Cas::Lire | Cas::LireComptesBancaires => "",
    }
}
