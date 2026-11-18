//! Quand horodater (spec §6.4, ADR 0010) : chaque jour si la chaîne a changé, et à chaque
//! verrouillage de période et à chaque clôture.
//!
//! La règle ne dépend que du dernier jeton **obtenu** : une demande qui échoue (TSA
//! injoignable) laisse la règle vraie, et la demande est rejouée au déclenchement suivant
//! (rejeu différé). Le témoin est reconstruit à ce moment-là, sur la tête du jour : la chaîne
//! étant cumulative, il couvre aussi les écritures restées sans jeton.

use time::Date;

/// Dernier jeton obtenu pour le dossier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DernierJeton {
    /// Jour d'obtention (UTC).
    pub jour: Date,
    /// Nombre d'écritures couvertes.
    pub nombre: u64,
}

/// Événement qui peut déclencher un horodatage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Declencheur {
    /// Passage quotidien du worker, ce jour-là (UTC).
    Quotidien(Date),
    /// Verrouillage d'une période.
    VerrouillagePeriode,
    /// Clôture d'un exercice.
    Cloture,
}

/// Vrai s'il faut demander un jeton pour une chaîne de `longueur` écritures.
///
/// - Rien à horodater tant que la chaîne est vide, ou si le dernier jeton la couvre déjà.
/// - Passage quotidien : au plus un jeton par jour.
/// - Verrouillage et clôture : toujours, pour dater l'état arrêté.
#[must_use]
pub fn horodatage_requis(
    dernier: Option<DernierJeton>,
    longueur: u64,
    declencheur: Declencheur,
) -> bool {
    let couverte = dernier.is_some_and(|d| d.nombre >= longueur);
    if longueur == 0 || couverte {
        return false;
    }
    match declencheur {
        Declencheur::Quotidien(jour) => dernier.is_none_or(|d| d.jour < jour),
        Declencheur::VerrouillagePeriode | Declencheur::Cloture => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::date;

    const LUNDI: Date = date!(2026 - 11 - 16);
    const MARDI: Date = date!(2026 - 11 - 17);

    #[test]
    fn quotidien_si_la_chaine_a_change() {
        let lundi = Some(DernierJeton {
            jour: LUNDI,
            nombre: 10,
        });
        assert!(!horodatage_requis(None, 0, Declencheur::Quotidien(MARDI)));
        assert!(horodatage_requis(None, 1, Declencheur::Quotidien(MARDI)));
        assert!(!horodatage_requis(lundi, 10, Declencheur::Quotidien(MARDI)));
        assert!(horodatage_requis(lundi, 11, Declencheur::Quotidien(MARDI)));
        // Au plus un jeton par jour.
        assert!(!horodatage_requis(lundi, 11, Declencheur::Quotidien(LUNDI)));
    }

    #[test]
    fn verrouillage_et_cloture_le_jour_meme() {
        let lundi = Some(DernierJeton {
            jour: LUNDI,
            nombre: 10,
        });
        for declencheur in [Declencheur::VerrouillagePeriode, Declencheur::Cloture] {
            assert!(horodatage_requis(lundi, 12, declencheur));
            assert!(!horodatage_requis(lundi, 10, declencheur));
        }
    }

    #[test]
    fn rejeu_apres_echec() {
        // La demande de mardi a échoué : le dernier jeton reste celui de lundi, mercredi
        // redemande.
        let lundi = Some(DernierJeton {
            jour: LUNDI,
            nombre: 10,
        });
        assert!(horodatage_requis(
            lundi,
            15,
            Declencheur::Quotidien(date!(2026 - 11 - 18))
        ));
    }
}
