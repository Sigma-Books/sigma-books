//! Décodage et reconnaissance du type « texte » (ADR 0034 §4, amendement de l'ADR 0015).
//!
//! Un relevé CSV n'a pas de signature : il est reconnu par son **contenu**. Le fichier est
//! refusé s'il dépasse la taille admise (avant toute lecture), s'il contient un octet nul ou un
//! caractère de contrôle autre que tabulation, fin de ligne et retour chariot, s'il n'est pas
//! décodable dans l'encodage du profil (liste fermée) ou si une ligne dépasse la longueur
//! admise. Le décodage se fait **ligne par ligne** : chaque erreur porte le rang de sa ligne.
//!
//! Les encodages admis sont tous compatibles avec l'ASCII : les octets `\r` et `\n` y sont
//! toujours des fins de ligne, ce qui permet de découper le fichier avant de décoder.

use encoding_rs::Encoding;

use super::{ErreurReleve, Limites};

/// Libellés admis (liste fermée, ADR 0034 §4). Hors UTF-8, le décodage suit la norme
/// *Encoding* du WHATWG (`encoding_rs`) : `iso-8859-1` y est lu comme `windows-1252` (même
/// table, sauf les codes 0x80 à 0x9F, caractères de contrôle en ISO-8859-1, imprimables en
/// windows-1252) et `iso-8859-9` comme `windows-1254`.
const LIBELLES: [&str; 16] = [
    "utf-8",
    "windows-1252",
    "iso-8859-1",
    "iso-8859-2",
    "iso-8859-3",
    "iso-8859-4",
    "iso-8859-5",
    "iso-8859-6",
    "iso-8859-7",
    "iso-8859-8",
    "iso-8859-9",
    "iso-8859-10",
    "iso-8859-13",
    "iso-8859-14",
    "iso-8859-15",
    "iso-8859-16",
];

const BOM_UTF8: &[u8] = b"\xEF\xBB\xBF";

/// Encodage d'un relevé, choisi dans une liste fermée.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Encodage {
    libelle: &'static str,
    table: &'static Encoding,
}

impl Encodage {
    /// UTF-8, avec ou sans BOM.
    pub const UTF8: Self = Self {
        libelle: "utf-8",
        table: encoding_rs::UTF_8,
    };

    /// Encodage désigné par son libellé (casse indifférente), s'il est dans la liste fermée.
    #[must_use]
    pub fn depuis_libelle(libelle: &str) -> Option<Self> {
        let libelle = LIBELLES.iter().find(|l| l.eq_ignore_ascii_case(libelle))?;
        let table = Encoding::for_label_no_replacement(libelle.as_bytes())?;
        Some(Self { libelle, table })
    }

    /// Libellé canonique (minuscules).
    #[must_use]
    pub fn libelle(&self) -> &'static str {
        self.libelle
    }

    /// Libellés admis.
    #[must_use]
    pub fn libelles_admis() -> &'static [&'static str] {
        &LIBELLES
    }

    pub(crate) fn est_utf8(self) -> bool {
        self.table == encoding_rs::UTF_8
    }

    pub(crate) fn table(self) -> &'static Encoding {
        self.table
    }

    fn decoder_segment(self, segment: &[u8]) -> Option<std::borrow::Cow<'_, str>> {
        if self.est_utf8() {
            std::str::from_utf8(segment)
                .ok()
                .map(std::borrow::Cow::Borrowed)
        } else {
            self.table
                .decode_without_bom_handling_and_without_replacement(segment)
        }
    }
}

/// Reconnaît un fichier texte et le décode (ADR 0034 §4). Le BOM UTF-8 éventuel est retiré
/// quand l'encodage est UTF-8 ; les fins de ligne sont conservées telles quelles.
///
/// # Errors
/// [`ErreurReleve::FichierTropGrand`] avant toute lecture ; puis, avec le rang de la première
/// ligne fautive : [`ErreurReleve::LigneTropLongue`], [`ErreurReleve::OctetNul`],
/// [`ErreurReleve::EncodageInvalide`], [`ErreurReleve::CaractereDeControle`].
pub fn decoder(
    octets: &[u8],
    encodage: Encodage,
    limites: &Limites,
) -> Result<String, ErreurReleve> {
    if octets.len() > limites.taille_fichier {
        return Err(ErreurReleve::FichierTropGrand {
            taille: octets.len(),
            limite: limites.taille_fichier,
        });
    }
    let corps = if encodage.est_utf8() {
        octets.strip_prefix(BOM_UTF8).unwrap_or(octets)
    } else {
        octets
    };
    // Borné par la taille admise : au plus trois octets UTF-8 par octet d'un encodage à un octet.
    let mut sortie = String::with_capacity(corps.len());
    let mut ligne = 1;
    let mut debut = 0;
    loop {
        let reste = corps.get(debut..).unwrap_or_default();
        let fin = reste.iter().position(|&b| b == b'\r' || b == b'\n');
        let segment = reste.get(..fin.unwrap_or(reste.len())).unwrap_or_default();
        if segment.len() > limites.longueur_ligne {
            return Err(ErreurReleve::LigneTropLongue {
                ligne,
                limite: limites.longueur_ligne,
            });
        }
        if segment.contains(&0) {
            return Err(ErreurReleve::OctetNul { ligne });
        }
        let texte = encodage
            .decoder_segment(segment)
            .ok_or(ErreurReleve::EncodageInvalide { ligne })?;
        if texte.chars().any(|c| c.is_control() && c != '\t') {
            return Err(ErreurReleve::CaractereDeControle { ligne });
        }
        sortie.push_str(&texte);
        let Some(fin) = fin else {
            return Ok(sortie);
        };
        if reste.get(fin..fin + 2) == Some(b"\r\n") {
            sortie.push_str("\r\n");
            debut += fin + 2;
        } else {
            sortie.push(if reste.get(fin) == Some(&b'\r') {
                '\r'
            } else {
                '\n'
            });
            debut += fin + 1;
        }
        ligne += 1;
    }
}
