//! Empreintes de relevé (ADR 0034 §7) : deux couches contre le rejeu et les doublons (F11.5).
//!
//! 1. **Fichier** : SHA-256 des octets tels que reçus, sans séparation de domaine, pour être
//!    égale à l'empreinte de la pièce (ADR 0015).
//! 2. **Ligne** : SHA-256 d'une forme canonique, **format `releve-ligne-v1`** (toute
//!    modification = nouvelle version, jamais un changement silencieux : les empreintes sont
//!    stockées sous la clé `UNIQUE (compte de trésorerie, empreinte, rang)`). Octets hachés, en
//!    UTF-8 :
//!
//! ```text
//! sigma-books/releve-ligne/v1\n
//! <mode>\n                         « identifiant » si la ligne a un identifiant de transaction,
//!                                  « champs » sinon
//! <nom>=<longueur>:<valeur>\n      pour chaque champ, dans l'ordre ci-dessous ; longueur en
//!                                  octets UTF-8, en décimal ; un champ absent s'écrit <nom>=-\n
//! ```
//!
//! Mode `identifiant` (l'identifiant prime) : `compte`, `identifiant`. Mode `champs` :
//! `compte`, `date` (`AAAA-MM-JJ`), `montant` (décimal normalisé : point, sans zéro final, `-`
//! en tête si négatif, zéro sans signe), `libelle` (normalisé : espaces de bord retirées,
//! suites d'espaces réduites à une espace U+0020, **casse conservée**), `reference` (telle
//! quelle). `compte` est l'identifiant du compte de trésorerie fourni par l'appelant : la
//! portée n'est jamais plus large que ce compte (ADR 0034 §7, F3.5).
//!
//! Le **rang d'occurrence** d'une ligne est son numéro parmi les lignes de même empreinte du
//! fichier, à partir de 1 : la k-ième ligne identique est un doublon si le rang k existe déjà.

use std::collections::HashMap;
use std::fmt;

use sha2::{Digest, Sha256};

use super::Mouvement;

const DOMAINE: &[u8] = b"sigma-books/releve-ligne/v1\n";

/// Empreinte SHA-256.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Empreinte([u8; 32]);

impl Empreinte {
    /// Octets de l'empreinte.
    #[must_use]
    pub fn octets(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Display for Empreinte {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.iter().try_for_each(|b| write!(f, "{b:02x}"))
    }
}

impl fmt::Debug for Empreinte {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Empreinte({self})")
    }
}

/// Empreinte d'une ligne et son rang d'occurrence dans le fichier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct EmpreinteRangee {
    /// Empreinte de la ligne.
    pub empreinte: Empreinte,
    /// Rang d'occurrence, à partir de 1.
    pub rang: usize,
}

/// SHA-256 du fichier tel que reçu.
#[must_use]
pub fn empreinte_fichier(octets: &[u8]) -> Empreinte {
    Empreinte(Sha256::digest(octets).into())
}

/// Libellé normalisé de l'empreinte de ligne.
#[must_use]
pub fn normaliser_libelle(libelle: &str) -> String {
    libelle.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn champ(h: &mut Sha256, nom: &str, valeur: Option<&str>) {
    h.update(nom.as_bytes());
    match valeur {
        None => h.update(b"=-\n"),
        Some(v) => {
            h.update(format!("={}:", v.len()).as_bytes());
            h.update(v.as_bytes());
            h.update(b"\n");
        }
    }
}

/// Empreinte de ligne, format `releve-ligne-v1` (voir le module).
#[must_use]
pub fn empreinte_ligne(compte: &str, m: &Mouvement) -> Empreinte {
    let mut h = Sha256::new();
    h.update(DOMAINE);
    if let Some(id) = &m.identifiant_transaction {
        h.update(b"identifiant\n");
        champ(&mut h, "compte", Some(compte));
        champ(&mut h, "identifiant", Some(id));
    } else {
        h.update(b"champs\n");
        champ(&mut h, "compte", Some(compte));
        let date = format!(
            "{:04}-{:02}-{:02}",
            m.date.year(),
            u8::from(m.date.month()),
            m.date.day()
        );
        champ(&mut h, "date", Some(&date));
        champ(&mut h, "montant", Some(&m.montant.normalize().to_string()));
        champ(&mut h, "libelle", Some(&normaliser_libelle(&m.libelle)));
        champ(&mut h, "reference", m.reference.as_deref());
    }
    Empreinte(h.finalize().into())
}

/// Empreintes et rangs d'occurrence des mouvements d'un fichier, dans leur ordre.
pub fn empreintes_rangees<'a>(
    compte: &str,
    mouvements: impl IntoIterator<Item = &'a Mouvement>,
) -> Vec<EmpreinteRangee> {
    let mut vues: HashMap<Empreinte, usize> = HashMap::new();
    mouvements
        .into_iter()
        .map(|m| {
            let empreinte = empreinte_ligne(compte, m);
            let rang = vues.entry(empreinte).or_insert(0);
            *rang += 1;
            EmpreinteRangee {
                empreinte,
                rang: *rang,
            }
        })
        .collect()
}
