//! Pièces justificatives (ADR 0015 ; L2 PR 12, issue L2-24).
//!
//! - **Types admis** par liste blanche, reconnus par leur **signature** (premiers octets), jamais
//!   par l'extension ni par le type annoncé : PDF, JPEG, PNG ([`TypePiece::reconnaitre`]). Aucun
//!   autre octet n'est lu : ni analyse, ni rendu, ni extraction de métadonnées (F1.5).
//! - **Empreinte** SHA-256 calculée par le serveur sur les octets reçus, puis **recalculée à
//!   chaque lecture** de l'objet (F3.1).
//! - **Stockage** objet derrière le trait [`Stockage`] ; implémentation compatible S3,
//!   [`s3::StockageS3`] : versionnement et verrouillage d'objet vérifiés au démarrage, rétention
//!   posée à l'écriture en mode gouvernance, clé `<cabinet>/<dossier>/<UUID v4>` ([`CleObjet`]).
//!   L'objet est écrit **avant** la ligne de la table `piece`, dans la transaction de la requête.
//! - **Pièce interne** : note d'opérations diverses sous forme canonique ([`interne`],
//!   `docs/integrite/piece-interne-v1.md`), hachée et stockée comme une pièce externe.
//!
//! Routes et cas d'usage : [`crate::serveur::routes::pieces`]. Accès à la table `piece` :
//! [`base`]. Configuration : [`config`].

pub mod base;
pub mod config;
pub mod interne;
pub mod s3;
pub mod stockage;

use std::sync::Arc;
use std::time::Duration;

use time::OffsetDateTime;

pub use stockage::{CleObjet, Depot, ErreurStockage, ObjetADeposer, Stockage};

/// Taille maximale d'une pièce déposée, par défaut : 20 Mio (ADR 0015).
pub const TAILLE_MAX_PAR_DEFAUT: usize = 20 * 1024 * 1024;

/// Délai de traitement d'un dépôt, par défaut : la réception de 20 Mio sur un lien lent.
pub const DELAI_DEPOT_PAR_DEFAUT: Duration = Duration::from_secs(120);

/// Réglages du service des pièces.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReglagesPieces {
    /// Taille maximale d'une pièce déposée, en octets : appliquée pendant la réception du flux.
    pub taille_max: usize,
    /// Réglage d'instance : rétention minimale **voulue** par l'exploitant, en années. Elle ne
    /// fait que **relever** celle du paquet du pays du dossier (paramètre fiscal, ADR 0030, dont
    /// le plancher OHADA est de dix ans, AUDCIF art. 24) ; elle ne la baisse jamais.
    pub retention_ans: Option<u16>,
    /// Délai de traitement d'une requête de dépôt (réception du corps comprise).
    pub delai_depot: Duration,
}

impl Default for ReglagesPieces {
    fn default() -> Self {
        Self {
            taille_max: TAILLE_MAX_PAR_DEFAUT,
            retention_ans: None,
            delai_depot: DELAI_DEPOT_PAR_DEFAUT,
        }
    }
}

/// Service des pièces branché sur l'application : le stockage objet et ses réglages.
pub struct Pieces {
    stockage: Arc<dyn Stockage>,
    reglages: ReglagesPieces,
}

impl std::fmt::Debug for Pieces {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Pieces")
            .field("reglages", &self.reglages)
            .finish_non_exhaustive()
    }
}

impl Pieces {
    /// Service sur ce stockage, avec ces réglages.
    #[must_use]
    pub fn new(stockage: Arc<dyn Stockage>, reglages: ReglagesPieces) -> Self {
        Self { stockage, reglages }
    }

    /// Le stockage objet.
    #[must_use]
    pub fn stockage(&self) -> &dyn Stockage {
        self.stockage.as_ref()
    }

    /// Les réglages.
    #[must_use]
    pub fn reglages(&self) -> &ReglagesPieces {
        &self.reglages
    }

    /// Années de rétention d'un objet : celles du paquet du pays du dossier (`annees_du_pays`),
    /// relevées par le réglage d'instance s'il est plus long.
    #[must_use]
    pub fn annees_de_retention(&self, annees_du_pays: u16) -> u16 {
        annees_du_pays.max(self.reglages.retention_ans.unwrap_or(0))
    }

    /// Fin de la rétention d'un objet écrit à `instant` : `instant` plus
    /// [`Pieces::annees_de_retention`] années civiles (un 29 février devient le 1er mars),
    /// arrondi à la seconde **supérieure** (précision des dates de rétention S3).
    #[must_use]
    pub fn fin_de_retention(&self, instant: OffsetDateTime, annees_du_pays: u16) -> OffsetDateTime {
        let fin = ajouter_annees(instant, self.annees_de_retention(annees_du_pays));
        let entiere = fin.replace_nanosecond(0).unwrap_or(fin);
        if entiere == fin {
            fin
        } else {
            entiere + time::Duration::SECOND
        }
    }
}

/// `instant` plus `annees` années civiles ; un 29 février sans équivalent devient le 1er mars.
/// Au-delà de l'an 9999 (inatteignable avec la configuration), la date maximale.
#[must_use]
pub fn ajouter_annees(instant: OffsetDateTime, annees: u16) -> OffsetDateTime {
    let annee = instant.year().saturating_add(i32::from(annees));
    let date = instant.date();
    let cible = date
        .replace_year(annee)
        .or_else(|_| {
            date.replace_day(1)
                .and_then(|d| d.replace_month(time::Month::March))
                .and_then(|d| d.replace_year(annee))
        })
        .unwrap_or(time::Date::MAX);
    instant.replace_date(cible)
}

/// Vrai pour un caractère refusé dans un texte d'une pièce (nom d'origine, motif, note
/// interne) : contrôle (`Cc`), format (`Cf`, dont les contrôles bidirectionnels comme U+202E
/// qui inversent l'affichage d'un nom de fichier), séparateurs de ligne et de paragraphe
/// (`Zl`, `Zp`). Liste `Cf` d'Unicode 16.0.
#[must_use]
pub fn caractere_interdit(c: char) -> bool {
    c.is_control()
        || matches!(
            c,
            '\u{00AD}'
                | '\u{0600}'..='\u{0605}'
                | '\u{061C}'
                | '\u{06DD}'
                | '\u{070F}'
                | '\u{0890}'..='\u{0891}'
                | '\u{08E2}'
                | '\u{180E}'
                | '\u{200B}'..='\u{200F}'
                | '\u{2028}'..='\u{202E}'
                | '\u{2060}'..='\u{2064}'
                | '\u{2066}'..='\u{206F}'
                | '\u{FEFF}'
                | '\u{FFF9}'..='\u{FFFB}'
                | '\u{110BD}'
                | '\u{110CD}'
                | '\u{13430}'..='\u{1343F}'
                | '\u{1BCA0}'..='\u{1BCA3}'
                | '\u{1D173}'..='\u{1D17A}'
                | '\u{E0001}'
                | '\u{E0020}'..='\u{E007F}'
        )
}

/// Texte d'une pièce admis : sans [`caractere_interdit`].
#[must_use]
pub fn texte_sur(texte: &str) -> bool {
    !texte.chars().any(caractere_interdit)
}

/// Nature d'une pièce (ADR 0015).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Nature {
    /// Fichier déposé, stocké tel quel.
    Externe,
    /// Pièce saisie dans Sigma Books (note d'opérations diverses).
    Interne,
}

impl Nature {
    /// Valeur de la colonne `nature`.
    #[must_use]
    pub const fn nom(self) -> &'static str {
        match self {
            Self::Externe => "externe",
            Self::Interne => "interne",
        }
    }

    /// Nature d'après la colonne `nature`.
    #[must_use]
    pub fn depuis_nom(nom: &str) -> Option<Self> {
        [Self::Externe, Self::Interne]
            .into_iter()
            .find(|n| n.nom() == nom)
    }
}

/// Type d'une pièce. Les types externes sont reconnus par leur signature seulement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TypePiece {
    /// PDF (`%PDF-`).
    Pdf,
    /// JPEG (`FF D8 FF`).
    Jpeg,
    /// PNG (`89 50 4E 47 0D 0A 1A 0A`).
    Png,
    /// Note d'opérations diverses, forme canonique de [`interne`] (pièce interne seulement).
    NoteOd,
    /// Relevé CSV (type « texte » de l'ADR 0034 point 4, amendement de l'ADR 0015) : sans
    /// signature, reconnu par son **contenu** et seulement par la route de dépôt de relevé
    /// (`routes::releves`), jamais par [`TypePiece::reconnaitre`]. Jamais rendu : servi en
    /// `application/octet-stream` ([`TypePiece::mime_servi`]).
    Texte,
}

/// Signatures des types externes admis, dans l'ordre d'essai.
const SIGNATURES: [(&[u8], TypePiece); 3] = [
    (b"%PDF-", TypePiece::Pdf),
    (&[0xFF, 0xD8, 0xFF], TypePiece::Jpeg),
    (
        &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A],
        TypePiece::Png,
    ),
];

impl TypePiece {
    /// Tous les types, dans l'ordre de la contrainte de la table `piece`.
    pub const TOUS: [Self; 5] = [Self::Pdf, Self::Jpeg, Self::Png, Self::NoteOd, Self::Texte];

    /// Type d'un fichier déposé, d'après ses **seuls premiers octets** (au plus 8), à
    /// l'offset 0 ; `None` pour tout autre contenu. Ni l'extension, ni le nom, ni le type annoncé
    /// par le client ne comptent.
    #[must_use]
    pub fn reconnaitre(octets: &[u8]) -> Option<Self> {
        SIGNATURES
            .iter()
            .find(|(signature, _)| octets.starts_with(signature))
            .map(|(_, type_piece)| *type_piece)
    }

    /// Type MIME : valeur de la colonne `type_contenu`.
    #[must_use]
    pub const fn mime(self) -> &'static str {
        match self {
            Self::Pdf => "application/pdf",
            Self::Jpeg => "image/jpeg",
            Self::Png => "image/png",
            Self::NoteOd => "text/plain; charset=utf-8",
            Self::Texte => "text/csv",
        }
    }

    /// Type annoncé au téléchargement (`Content-Type`) et au stockage objet : celui de la
    /// colonne, sauf pour le type texte, **jamais rendu** (ADR 0034 point 4) :
    /// `application/octet-stream`.
    #[must_use]
    pub const fn mime_servi(self) -> &'static str {
        match self {
            Self::Texte => "application/octet-stream",
            autre => autre.mime(),
        }
    }

    /// Type d'après la colonne `type_contenu`.
    #[must_use]
    pub fn depuis_mime(mime: &str) -> Option<Self> {
        Self::TOUS.into_iter().find(|t| t.mime() == mime)
    }

    /// Extension du nom proposé au téléchargement quand la pièce n'a pas de nom d'origine.
    #[must_use]
    pub const fn extension(self) -> &'static str {
        match self {
            Self::Pdf => "pdf",
            Self::Jpeg => "jpg",
            Self::Png => "png",
            Self::NoteOd => "txt",
            Self::Texte => "csv",
        }
    }
}

#[cfg(test)]
mod tests {
    use time::macros::datetime;

    use super::*;

    #[test]
    fn signatures_reconnues_et_refusees() {
        assert_eq!(
            TypePiece::reconnaitre(b"%PDF-1.7\n..."),
            Some(TypePiece::Pdf)
        );
        assert_eq!(
            TypePiece::reconnaitre(&[0xFF, 0xD8, 0xFF, 0xE0, 0, 0x10]),
            Some(TypePiece::Jpeg)
        );
        assert_eq!(
            TypePiece::reconnaitre(b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR"),
            Some(TypePiece::Png)
        );
        for refuse in [
            &b""[..],
            b"%PDF",
            b" %PDF-1.7",
            b"MZ\x90\0\x03\0\0\0",
            b"\x7fELF\x02\x01\x01",
            b"<html><body>%PDF-1.7",
            b"<?xml version=\"1.0\"?><svg/>",
            b"GIF89a",
            b"PK\x03\x04",
            b"\xFF\xD8",
            b"\x89PNG\r\n\x1a",
        ] {
            assert_eq!(TypePiece::reconnaitre(refuse), None, "{refuse:?}");
        }
    }

    #[test]
    fn caracteres_interdits() {
        for c in [
            '\u{7}',
            '\n',
            '\u{202E}',
            '\u{200B}',
            '\u{FEFF}',
            '\u{2028}',
            '\u{2029}',
            '\u{E0041}',
        ] {
            assert!(caractere_interdit(c), "{:X}", u32::from(c));
        }
        for c in ['a', 'é', ' ', '°', '€', '\u{00A0}', 'ض'] {
            assert!(!caractere_interdit(c), "{:X}", u32::from(c));
        }
        assert!(texte_sur("Facture n°1.pdf"));
        assert!(!texte_sur("facture\u{202E}fdp.cmd"));
    }

    #[test]
    fn types_et_colonne() {
        for t in TypePiece::TOUS {
            assert_eq!(TypePiece::depuis_mime(t.mime()), Some(t));
        }
        assert_eq!(TypePiece::depuis_mime("text/html"), None);
        assert_eq!(TypePiece::Texte.mime_servi(), "application/octet-stream");
        assert_eq!(TypePiece::Pdf.mime_servi(), "application/pdf");
        // Le type texte n'a pas de signature : le dépôt général ne le reconnaît jamais.
        assert_eq!(TypePiece::reconnaitre(b"Date;Montant\n"), None);
        assert_eq!(Nature::depuis_nom("externe"), Some(Nature::Externe));
        assert_eq!(Nature::depuis_nom("interne"), Some(Nature::Interne));
        assert_eq!(Nature::depuis_nom("autre"), None);
    }

    #[test]
    fn retention_en_annees_civiles() {
        assert_eq!(
            ajouter_annees(datetime!(2026-11-19 10:00:00.5 UTC), 10),
            datetime!(2036-11-19 10:00:00.5 UTC)
        );
        assert_eq!(
            ajouter_annees(datetime!(2028-02-29 23:59:59 UTC), 10),
            datetime!(2038-03-01 23:59:59 UTC)
        );
        assert_eq!(
            ajouter_annees(datetime!(2028-02-29 00:00 UTC), 12),
            datetime!(2040-02-29 00:00 UTC)
        );
    }
}
