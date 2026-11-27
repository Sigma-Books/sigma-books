//! Vérification structurelle des entrées d'audit : ni libellé ni montant (ADR 0014).
//!
//! Une recherche de sous-chaîne (`contains("1000")`) est fragile : les UUID, les empreintes et
//! les identifiants hexadécimaux d'une entrée contiennent « 1000 » par hasard. Ici, l'entrée
//! (format canonique `audit v1`, une ligne `clé=valeur` par champ) est lue ligne à ligne et
//! chaque VALEUR est comparée entière : un libellé interdit ne doit figurer dans aucune ligne,
//! un montant interdit ne doit être la valeur d'aucun champ.

/// Écriture décimale normalisée : sans zéros inutiles (`1000.00` devient `1000`).
fn normalise(texte: &str) -> String {
    let t = texte.trim();
    if t.contains('.') {
        t.trim_end_matches('0').trim_end_matches('.').to_owned()
    } else {
        t.to_owned()
    }
}

/// Échoue si une ligne de l'entrée d'audit contient un `libelle` interdit, ou si la valeur d'un
/// champ (partie après le premier `=`) vaut un `montant` interdit (valeur entière, jamais une
/// sous-chaîne : indépendant des UUID et des empreintes).
pub fn verifier_sans_valeur_metier(entree: &str, libelles: &[&str], montants: &[&str]) {
    for (numero, ligne) in entree.lines().enumerate() {
        if let Some(l) = libelles.iter().find(|l| ligne.contains(**l)) {
            panic!(
                "l'entrée d'audit contient le libellé « {l} » (ligne {}) : {entree}",
                numero + 1
            );
        }
        let valeur = ligne.split_once('=').map_or(ligne, |(_, v)| v);
        let n = normalise(valeur);
        if let Some(m) = montants.iter().find(|m| normalise(m) == n) {
            panic!(
                "l'entrée d'audit contient le montant {m} (ligne {}) : {entree}",
                numero + 1
            );
        }
    }
}
