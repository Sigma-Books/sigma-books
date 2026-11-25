//! Adresses de courriel et noms d'affichage saisis (L2-19, PR 9 ; ADR 0012 et 0023).
//!
//! Entrées **hostiles** : une adresse ou un nom saisi par un administrateur finit un jour dans un
//! en-tête de courriel (PR 9b) ; un retour à la ligne y permettrait d'injecter des en-têtes
//! (`Bcc:`), des destinataires ou un corps. La validation est donc **stricte et ASCII** : une
//! liste blanche de caractères, aucune citation, aucun commentaire, aucune adresse
//! internationalisée (limite assumée : un domaine non ASCII se saisit en `xn--`). Pur : aucun
//! réseau, aucune horloge.

/// Longueur maximale d'une adresse (RFC 5321).
pub const ADRESSE_MAX: usize = 254;
const LOCAL_MAX: usize = 64;
/// Longueur maximale d'un nom d'affichage.
pub const NOM_MAX: usize = 200;

fn caractere_local(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '+' | '-')
}

fn etiquette_valide(e: &str) -> bool {
    !e.is_empty()
        && e.len() <= 63
        && !e.starts_with('-')
        && !e.ends_with('-')
        && e.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

/// Normalise (minuscules, sans espaces autour) et valide une adresse. `None` si elle sort de la
/// liste blanche : partie locale de `[a-z0-9._+-]` (ni `%`, ni `!`, ni `/` : routes de source et jokers) sans point en tête, en queue ni doublé ;
/// domaine d'au moins deux étiquettes `[a-z0-9-]`, dont la dernière n'est pas numérique.
#[must_use]
pub fn adresse(saisie: &str) -> Option<String> {
    let a = saisie.trim().to_ascii_lowercase();
    if a.len() > ADRESSE_MAX || !a.is_ascii() {
        return None;
    }
    let (local, domaine) = a.split_once('@')?;
    let local_ok = !local.is_empty()
        && local.len() <= LOCAL_MAX
        && local.chars().all(caractere_local)
        && !local.starts_with('.')
        && !local.ends_with('.')
        && !local.contains("..");
    let etiquettes: Vec<&str> = domaine.split('.').collect();
    let domaine_ok = etiquettes.len() >= 2
        && etiquettes.iter().all(|e| etiquette_valide(e))
        && !etiquettes
            .last()
            .is_some_and(|t| t.chars().all(|c| c.is_ascii_digit()));
    (local_ok && domaine_ok).then_some(a)
}

/// Nom d'affichage : 1 à [`NOM_MAX`] caractères après rognage, sans caractère de contrôle (ni
/// retour à la ligne), sans caractère de formatage bidirectionnel ou invisible.
#[must_use]
pub fn nom(saisie: &str) -> Option<String> {
    let n = saisie.trim();
    let invisible = |c: char| {
        matches!(c, '\u{200b}'..='\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}'
            | '\u{feff}' | '\u{2028}' | '\u{2029}')
    };
    (!n.is_empty()
        && n.chars().count() <= NOM_MAX
        && !n.chars().any(|c| c.is_control() || invisible(c)))
    .then(|| n.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn adresses_admises_et_normalisees() {
        assert_eq!(
            adresse("  Prenom.Nom+tag@Cabinet.Example.SN ").as_deref(),
            Some("prenom.nom+tag@cabinet.example.sn")
        );
    }

    #[test]
    fn adresses_refusees() {
        for mauvaise in [
            "",
            "sans-arobase",
            "a@b",
            "@cabinet.sn",
            "a@@cabinet.sn",
            "a@cabinet.sn\r\nBcc: autre@x.sn",
            "a@cabinet.sn\nBcc: x@y.sn",
            "a b@cabinet.sn",
            "\"a b\"@cabinet.sn",
            "a,b@cabinet.sn",
            "a;b@cabinet.sn",
            "a<b>@cabinet.sn",
            "a%b@cabinet.sn",
            "a!b@cabinet.sn",
            "a@cabinet.sn>",
            "a@-cabinet.sn",
            "a@cabinet-.sn",
            "a@cabinet..sn",
            "a@cabinet.123",
            ".a@cabinet.sn",
            "a.@cabinet.sn",
            "a..b@cabinet.sn",
            "é@cabinet.sn",
            "a@cabïnet.sn",
            "a@[127.0.0.1]",
            "a@cabinet.sn\0",
        ] {
            assert_eq!(adresse(mauvaise), None, "{mauvaise:?}");
        }
        assert_eq!(adresse(&format!("{}@cabinet.sn", "a".repeat(65))), None);
        assert_eq!(adresse(&format!("a@{}.sn", "b".repeat(250))), None);
    }

    #[test]
    fn noms_admis_et_refuses() {
        assert_eq!(nom("  Awa Diop ").as_deref(), Some("Awa Diop"));
        assert_eq!(nom(&"é".repeat(200)).map(|n| n.chars().count()), Some(200));
        for mauvais in [
            "",
            "   ",
            "A\nBcc: x",
            "A\rB",
            "A\tB",
            "A\u{202e}B",
            "A\u{200b}B",
            "A\u{0}B",
            &"a".repeat(201),
        ] {
            assert_eq!(nom(mauvais), None, "{mauvais:?}");
        }
    }

    proptest! {
        /// Une adresse acceptée ne contient jamais de quoi casser un en-tête.
        #[test]
        fn une_adresse_acceptee_est_inoffensive(saisie in "\\PC{0,80}") {
            if let Some(a) = adresse(&saisie) {
                prop_assert!(a.is_ascii());
                prop_assert!(!a.chars().any(|c| c.is_control() || c.is_whitespace()
                    || matches!(c, '<' | '>' | ',' | ';' | '"' | '(' | ')' | '\\' | ':')));
                prop_assert_eq!(a.matches('@').count(), 1);
                // Idempotence : une adresse normalisée se normalise en elle-même.
                prop_assert_eq!(adresse(&a), Some(a));
            }
        }

        /// Un nom accepté tient sur une ligne, sans contrôle.
        #[test]
        fn un_nom_accepte_tient_sur_une_ligne(saisie in "\\PC{0,80}[\\n\\r]?\\PC{0,5}") {
            if let Some(n) = nom(&saisie) {
                prop_assert!(!n.chars().any(char::is_control));
                prop_assert!(!n.contains('\n') && !n.contains('\r'));
            }
        }
    }
}
