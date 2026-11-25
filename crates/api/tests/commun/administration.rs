//! Aides de test de la voie administration (PR 9a).

/// Corps d'exemple des mutations du module `administration` (`commun::serveur::corps_de`).
pub fn corps(cas: api::serveur::routes::administration::Cas) -> &'static str {
    use api::serveur::routes::administration::Cas;
    match cas {
        Cas::ChangerRole => r#"{"role":"valideur"}"#,
        // Dossier inexistant : l'affectation d'un dossier réel est testée dans `administration.rs`.
        Cas::Affecter => r#"{"dossier":"6f1d3c2e-5a47-4b8e-9c10-2d7e8a9b0c11"}"#,
        Cas::Inviter => {
            r#"{"courriel":"invite.isolation@cabinet.example","nom":"Invité","role":"auditeur"}"#
        }
        Cas::Reinitialiser => {
            r#"{"motif_code":"autre","motif":"Vérification par un appel au titulaire","verification_hors_bande":true}"#
        }
        // Usage d'un lien : corps vide (refus avant toute lecture de la base) ; les scénarios
        // d'isolation de ces deux routes fabriquent leurs propres liens.
        _ => "",
    }
}
