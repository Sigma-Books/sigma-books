//! Export d'intégrité : les écritures validées d'un dossier dans l'ordre de la chaîne, chacune
//! suivie de son empreinte, puis la tête, le nombre d'écritures et, depuis la **v2**, les
//! jetons d'horodatage RFC 3161 de la chaîne. La **v3** admet en plus les écritures canoniques
//! v2 (date de valeur comptable, AUDCIF art. 22 4°) et la **v4** les écritures canoniques v3
//! (étiquette fiscale des lignes, ADR 0019).
//!
//! Formats décrits dans `docs/integrite/format-v1.md` à `format-v4.md`. L'écriture produit
//! la version la plus basse qui admet toutes les écritures (v3, ou v4 si l'une porte une
//! étiquette fiscale : les empreintes et les exports sans étiquette ne changent pas) ; la
//! lecture accepte toutes les versions. Elle traite le fichier comme **hostile** :
//! lecture ligne à ligne bornée, aucune allocation proportionnelle à une valeur lue, erreurs
//! typées avec numéro de ligne, aucune panique.

use std::io::{self, BufRead, Read as _, Write};

use base64ct::{Base64, Encoding};
use compta_core::EcritureValidee;

use crate::canonique::{self, EcritureCanonique};
use crate::chaine::{Chaine, Empreinte, IdDossier};
use crate::horodatage::JETON_MAX;

/// Première ligne d'un export v1 (lecture seule).
pub const ENTETE_EXPORT_V1: &str = "sigma-books export v1";
/// Première ligne d'un export v2 (lecture seule).
pub const ENTETE_EXPORT_V2: &str = "sigma-books export v2";
/// Première ligne d'un export v3.
pub const ENTETE_EXPORT_V3: &str = "sigma-books export v3";
/// Première ligne d'un export v4 (écritures avec étiquette fiscale).
pub const ENTETE_EXPORT_V4: &str = "sigma-books export v4";
/// Première ligne d'un export v5 (écritures avec devise de tenue, ADR 0027).
pub const ENTETE_EXPORT_V5: &str = "sigma-books export v5";
/// Longueur d'une ligne `der=` complète (base64), la dernière pouvant être plus courte.
pub const BASE64_LIGNE: usize = 64;
/// Nombre maximal de jetons dans un export (un par jour pendant plus de 25 ans).
pub const JETONS_MAX: usize = 10_000;
/// Dernière ligne d'un export.
pub const FIN_EXPORT: &str = "fin-export";
/// Longueur maximale d'une ligne, fin de ligne comprise (un texte court fait au plus
/// 200 caractères, soit 800 octets UTF-8).
pub const LIGNE_MAX: usize = 1024;
/// Taille maximale d'une écriture canonique.
pub const ECRITURE_MAX: usize = 4 * 1024 * 1024;

/// Défaut de forme d'un export.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ErreurExport {
    /// Ligne de plus de [`LIGNE_MAX`] octets.
    #[error("ligne trop longue (plus de {LIGNE_MAX} octets)")]
    LigneTropLongue,
    /// Octets qui ne sont pas de l'UTF-8.
    #[error("texte qui n'est pas en UTF-8")]
    PasUtf8,
    /// Dernière ligne sans `\n`.
    #[error("fin de ligne manquante")]
    FinDeLigneManquante,
    /// Fin du fichier avant `fin-export` : export tronqué.
    #[error("fin du fichier avant « {FIN_EXPORT} » : export tronqué")]
    Tronque,
    /// Ligne inattendue.
    #[error("« {0} » attendu")]
    Attendu(&'static str),
    /// Identifiant de dossier invalide.
    #[error("identifiant de dossier invalide")]
    DossierInvalide,
    /// Empreinte invalide.
    #[error("empreinte invalide")]
    EmpreinteInvalide,
    /// Nombre d'écritures invalide.
    #[error("nombre d'écritures invalide")]
    NombreInvalide,
    /// Écriture de plus de [`ECRITURE_MAX`] octets.
    #[error("écriture trop volumineuse (plus de {ECRITURE_MAX} octets)")]
    EcritureTropGrande,
    /// Écriture qui n'est pas au format canonique v1.
    #[error("écriture illisible : {0}")]
    Ecriture(#[from] canonique::ErreurCanonique),
    /// Données après `fin-export`.
    #[error("données après « {FIN_EXPORT} »")]
    DonneesApresFin,
    /// Jeton mal encodé (base64 non canonique, découpage ou taille).
    #[error("jeton mal encodé : {0}")]
    JetonMalEncode(&'static str),
    /// Plus de [`JETONS_MAX`] jetons.
    #[error("plus de {JETONS_MAX} jetons")]
    TropDeJetons,
    /// Export d'audit : UUID invalide (cabinet, dossier, acteur de l'annexe).
    #[error("UUID invalide")]
    UuidInvalide,
    /// Export d'audit : entrée de plus de 4 Mio.
    #[error("entrée d'audit trop volumineuse (plus de 4 Mio)")]
    EntreeAuditTropGrande,
    /// Export d'audit : annexe mal formée (acteurs non triés ou en double, nom invalide).
    #[error("annexe des acteurs mal formée")]
    AnnexeInvalide,
}

/// Version du format d'export.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Version {
    /// v1 : écritures, tête, nombre.
    V1,
    /// v2 : v1 et jetons d'horodatage.
    V2,
    /// v3 : v2 et écritures canoniques v2 (date de valeur comptable).
    V3,
    /// v4 : v3 et écritures canoniques v3 (étiquette fiscale des lignes).
    V4,
    /// v5 : v4 et écritures canoniques v4 (devise de tenue et échelle du dossier).
    V5,
}

impl Version {
    /// Plus haute version d'écriture canonique admise par cette version d'export.
    #[must_use]
    pub fn ecriture_max(self) -> u8 {
        match self {
            Self::V1 | Self::V2 => 1,
            Self::V3 => 2,
            Self::V4 => 3,
            Self::V5 => 4,
        }
    }

    /// Version d'export la plus basse qui admet une écriture de version `ecriture`.
    #[must_use]
    pub fn pour_ecriture(ecriture: u8) -> Self {
        match ecriture {
            0..=2 => Self::V3,
            3 => Self::V4,
            _ => Self::V5,
        }
    }

    /// En-tête de l'export, que Sigma Books écrit pour les versions 3 à 5.
    fn entete(self) -> &'static str {
        match self {
            Self::V1 => ENTETE_EXPORT_V1,
            Self::V2 => ENTETE_EXPORT_V2,
            Self::V3 => ENTETE_EXPORT_V3,
            Self::V4 => ENTETE_EXPORT_V4,
            Self::V5 => ENTETE_EXPORT_V5,
        }
    }
}

/// Jeton d'horodatage de la chaîne, tel qu'exporté : le témoin (nombre d'écritures et tête à
/// cet instant) et le jeton RFC 3161 (DER).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JetonExporte {
    /// Nombre d'écritures chaînées au moment du témoin.
    pub nombre: u64,
    /// Empreinte de tête à ce moment.
    pub tete: Empreinte,
    /// Jeton RFC 3161 (`TimeStampToken`, DER).
    pub der: Vec<u8>,
}

/// Erreur de lecture : défaut de forme (avec la ligne du fichier, à partir de 1) ou erreur
/// d'entrée-sortie.
#[derive(Debug, thiserror::Error)]
pub enum ErreurLecture {
    /// Défaut de forme.
    #[error("ligne {ligne} : {erreur}")]
    Forme {
        /// Ligne du fichier.
        ligne: u64,
        /// Défaut constaté.
        erreur: ErreurExport,
    },
    /// Erreur d'entrée-sortie.
    #[error("lecture impossible : {0}")]
    Io(#[from] io::Error),
}

/// Élément lu après l'en-tête.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Element {
    /// Une écriture et l'empreinte déclarée pour elle.
    Ecriture {
        /// Ligne du fichier où commence l'écriture.
        ligne: u64,
        /// Écriture lue (sa forme canonique est exactement le texte du fichier).
        ecriture: Box<EcritureCanonique>,
        /// Octets hachés : la forme canonique.
        contenu: String,
        /// Empreinte déclarée par l'export.
        empreinte: Empreinte,
        /// Ligne de l'empreinte déclarée.
        ligne_empreinte: u64,
    },
    /// Jeton d'horodatage (v2), après la fin de la chaîne.
    Jeton {
        /// Ligne du marqueur `jeton`.
        ligne: u64,
        /// Jeton lu.
        jeton: Box<JetonExporte>,
    },
    /// Fin de la chaîne (tête et nombre déclarés) ; en v2, les jetons suivent.
    Fin {
        /// Tête déclarée.
        tete: Empreinte,
        /// Nombre d'écritures déclaré.
        nombre: u64,
        /// Ligne de la tête déclarée.
        ligne: u64,
    },
}

/// Écrit un export v3, v4 ou v5, écriture par écriture.
pub struct EcrivainExport<W: Write> {
    sortie: W,
    chaine: Chaine,
    version: Version,
}

impl<W: Write> EcrivainExport<W> {
    /// Écrit l'en-tête d'un export v3 (écritures sans étiquette fiscale).
    ///
    /// # Errors
    /// Erreur d'écriture.
    pub fn nouveau(dossier: &IdDossier, sortie: W) -> io::Result<Self> {
        Self::avec_version(dossier, Version::V3, sortie)
    }

    /// Écrit l'en-tête d'un export v4 (admet les écritures avec étiquette fiscale).
    ///
    /// # Errors
    /// Erreur d'écriture.
    pub fn nouveau_v4(dossier: &IdDossier, sortie: W) -> io::Result<Self> {
        Self::avec_version(dossier, Version::V4, sortie)
    }

    /// Écrit l'en-tête d'un export v5 (admet les écritures avec devise de tenue, ADR 0027).
    ///
    /// # Errors
    /// Erreur d'écriture.
    pub fn nouveau_v5(dossier: &IdDossier, sortie: W) -> io::Result<Self> {
        Self::avec_version(dossier, Version::V5, sortie)
    }

    fn avec_version(dossier: &IdDossier, version: Version, mut sortie: W) -> io::Result<Self> {
        writeln!(sortie, "{}", version.entete())?;
        writeln!(sortie, "dossier={dossier}")?;
        Ok(Self {
            sortie,
            chaine: Chaine::ouvrir(dossier),
            version,
        })
    }

    /// Ajoute l'écriture suivante de la chaîne et son empreinte.
    ///
    /// # Errors
    /// Erreur d'écriture, ou [`io::ErrorKind::InvalidInput`] si le format de l'écriture (v3 avec
    /// étiquette fiscale, v4 avec devise de tenue) n'est pas admis par la version de l'export
    /// (rien n'est écrit, la chaîne n'avance pas).
    pub fn ecriture(&mut self, ecriture: &EcritureValidee) -> io::Result<Empreinte> {
        let canonique = EcritureCanonique::depuis(ecriture);
        if canonique.version() > self.version.ecriture_max() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "format d'écriture trop récent pour cette version d'export",
            ));
        }
        let contenu = canonique.serialiser();
        let empreinte = self.chaine.ajouter(ecriture);
        self.sortie.write_all(contenu.as_bytes())?;
        writeln!(self.sortie, "empreinte={empreinte}")?;
        Ok(empreinte)
    }

    /// Écrit la tête et le nombre d'écritures, puis rend la sortie et la tête.
    ///
    /// # Errors
    /// Erreur d'écriture.
    pub fn terminer(self) -> io::Result<(W, Empreinte)> {
        self.terminer_avec(&[])
    }

    /// Écrit la tête, le nombre d'écritures et les jetons d'horodatage (dans l'ordre donné),
    /// puis rend la sortie et la tête.
    ///
    /// # Errors
    /// Erreur d'écriture.
    pub fn terminer_avec(mut self, jetons: &[JetonExporte]) -> io::Result<(W, Empreinte)> {
        let tete = *self.chaine.tete();
        writeln!(self.sortie, "tete={tete}")?;
        writeln!(self.sortie, "nombre={}", self.chaine.longueur())?;
        for jeton in jetons {
            writeln!(self.sortie, "jeton")?;
            writeln!(self.sortie, "nombre={}", jeton.nombre)?;
            writeln!(self.sortie, "tete={}", jeton.tete)?;
            let base64 = Base64::encode_string(&jeton.der);
            for morceau in base64.as_bytes().chunks(BASE64_LIGNE) {
                self.sortie.write_all(b"der=")?;
                self.sortie.write_all(morceau)?;
                self.sortie.write_all(b"\n")?;
            }
            writeln!(self.sortie, "fin-jeton")?;
        }
        writeln!(self.sortie, "{FIN_EXPORT}")?;
        self.sortie.flush()?;
        Ok((self.sortie, tete))
    }
}

/// Exporte des écritures validées, dans l'ordre de la chaîne, et leurs jetons. Renvoie la
/// tête. Version la plus basse qui admet toutes les écritures : v5 si l'une est tenue dans une
/// autre devise que le XOF sans décimale (ADR 0027), sinon v4 si l'une porte une étiquette
/// fiscale, v3 sinon.
///
/// # Errors
/// Erreur d'écriture.
pub fn exporter<W: Write>(
    dossier: &IdDossier,
    ecritures: &[EcritureValidee],
    jetons: &[JetonExporte],
    sortie: W,
) -> io::Result<Empreinte> {
    let plus_haute = ecritures
        .iter()
        .map(|e| EcritureCanonique::depuis(e).version())
        .max()
        .unwrap_or(1);
    let mut ecrivain =
        EcrivainExport::avec_version(dossier, Version::pour_ecriture(plus_haute), sortie)?;
    for e in ecritures {
        ecrivain.ecriture(e)?;
    }
    ecrivain.terminer_avec(jetons).map(|(_, tete)| tete)
}

/// Étape de la lecture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Etape {
    /// Écritures, puis tête et nombre.
    Chaine,
    /// Jetons (v2), puis `fin-export`.
    Jetons,
    /// `fin-export` lu.
    Termine,
}

/// Lit un export v1 à v5 élément par élément, sans le charger en mémoire.
pub struct LecteurExport<R: BufRead> {
    entree: R,
    ligne: u64,
    tampon: Vec<u8>,
    version: Version,
    etape: Etape,
    jetons: usize,
}

impl<R: BufRead> LecteurExport<R> {
    /// Lit l'en-tête et renvoie le lecteur et l'identifiant du dossier.
    ///
    /// # Errors
    /// En-tête absent ou invalide, erreur d'entrée-sortie.
    pub fn ouvrir(entree: R) -> Result<(Self, IdDossier), ErreurLecture> {
        let mut lecteur = Self {
            entree,
            ligne: 0,
            tampon: Vec::with_capacity(LIGNE_MAX + 1),
            version: Version::V5,
            etape: Etape::Chaine,
            jetons: 0,
        };
        lecteur.version = match lecteur.ligne_requise()?.as_str() {
            ENTETE_EXPORT_V5 => Version::V5,
            ENTETE_EXPORT_V4 => Version::V4,
            ENTETE_EXPORT_V3 => Version::V3,
            ENTETE_EXPORT_V2 => Version::V2,
            ENTETE_EXPORT_V1 => Version::V1,
            _ => return Err(lecteur.erreur(ErreurExport::Attendu(ENTETE_EXPORT_V5))),
        };
        let dossier = lecteur.valeur("dossier")?;
        let dossier = IdDossier::parse(&dossier)
            .map_err(|_| lecteur.erreur(ErreurExport::DossierInvalide))?;
        Ok((lecteur, dossier))
    }

    /// Version du format lu.
    #[must_use]
    pub fn version(&self) -> Version {
        self.version
    }

    /// Numéro de la dernière ligne lue.
    #[must_use]
    pub fn ligne(&self) -> u64 {
        self.ligne
    }

    /// Élément suivant ; `None` une fois la fin lue.
    ///
    /// # Errors
    /// Défaut de forme ou erreur d'entrée-sortie.
    pub fn suivant(&mut self) -> Result<Option<Element>, ErreurLecture> {
        match self.etape {
            Etape::Termine => return Ok(None),
            Etape::Jetons => return self.jeton_ou_fin(),
            Etape::Chaine => {}
        }
        let premiere = self.ligne_requise()?;
        let debut = self.ligne;
        if let Some(tete) = premiere.strip_prefix("tete=") {
            let tete =
                Empreinte::parse(tete).map_err(|_| self.erreur(ErreurExport::EmpreinteInvalide))?;
            let nombre = self.nombre()?;
            if self.version == Version::V1 {
                self.fin()?;
            } else {
                self.etape = Etape::Jetons;
            }
            return Ok(Some(Element::Fin {
                tete,
                nombre,
                ligne: debut,
            }));
        }
        let maximum = self.version.ecriture_max();
        let canonique_admise = (premiere == canonique::ENTETE_V1)
            || (premiere == canonique::ENTETE_V2 && maximum >= 2)
            || (premiere == canonique::ENTETE_V3 && maximum >= 3)
            || (premiere == canonique::ENTETE_V4 && maximum >= 4);
        if !canonique_admise {
            return Err(self.erreur(ErreurExport::Attendu(match self.version {
                Version::V5 => {
                    "ecriture v1 », « ecriture v2 », « ecriture v3 », « ecriture v4 » ou « tete="
                }
                Version::V4 => "ecriture v1 », « ecriture v2 », « ecriture v3 » ou « tete=",
                Version::V3 => "ecriture v1 », « ecriture v2 » ou « tete=",
                Version::V1 | Version::V2 => "ecriture v1 » ou « tete=",
            })));
        }
        let mut contenu = premiere;
        contenu.push('\n');
        loop {
            let l = self.ligne_requise()?;
            if contenu.len() + l.len() + 1 > ECRITURE_MAX {
                return Err(self.erreur(ErreurExport::EcritureTropGrande));
            }
            contenu.push_str(&l);
            contenu.push('\n');
            if l == canonique::FIN {
                break;
            }
        }
        let ecriture = EcritureCanonique::lire(&contenu).map_err(|e| {
            let decalage = match &e {
                canonique::ErreurCanonique::Attendu { ligne, .. }
                | canonique::ErreurCanonique::ValeurInvalide { ligne, .. }
                | canonique::ErreurCanonique::EtiquetteIncoherente { ligne }
                | canonique::ErreurCanonique::ContreValeurIncoherente { ligne }
                | canonique::ErreurCanonique::TexteEnTrop { ligne } => *ligne,
                canonique::ErreurCanonique::NonCanonique => 1,
            };
            ErreurLecture::Forme {
                ligne: debut + (decalage.max(1) as u64) - 1,
                erreur: e.into(),
            }
        })?;
        let empreinte = self.valeur("empreinte")?;
        let empreinte = Empreinte::parse(&empreinte)
            .map_err(|_| self.erreur(ErreurExport::EmpreinteInvalide))?;
        Ok(Some(Element::Ecriture {
            ligne: debut,
            ecriture: Box::new(ecriture),
            contenu,
            empreinte,
            ligne_empreinte: self.ligne,
        }))
    }

    /// v2 : jeton suivant, ou `fin-export`.
    fn jeton_ou_fin(&mut self) -> Result<Option<Element>, ErreurLecture> {
        let premiere = self.ligne_requise()?;
        if premiere == FIN_EXPORT {
            self.verifier_fin_de_fichier()?;
            return Ok(None);
        }
        if premiere != "jeton" {
            return Err(self.erreur(ErreurExport::Attendu("jeton » ou « fin-export")));
        }
        let debut = self.ligne;
        self.jetons += 1;
        if self.jetons > JETONS_MAX {
            return Err(self.erreur(ErreurExport::TropDeJetons));
        }
        let nombre = self.nombre()?;
        let tete = self.valeur("tete")?;
        let tete =
            Empreinte::parse(&tete).map_err(|_| self.erreur(ErreurExport::EmpreinteInvalide))?;
        let mut base64 = String::new();
        loop {
            let l = self.ligne_requise()?;
            if l == "fin-jeton" {
                break;
            }
            let Some(morceau) = l.strip_prefix("der=") else {
                return Err(self.erreur(ErreurExport::Attendu("der » ou « fin-jeton")));
            };
            // Toutes les lignes ont 64 caractères, sauf la dernière (1 à 64).
            if morceau.is_empty()
                || morceau.len() > BASE64_LIGNE
                || !base64.len().is_multiple_of(BASE64_LIGNE)
            {
                return Err(self.erreur(ErreurExport::JetonMalEncode("découpage des lignes")));
            }
            if base64.len() + morceau.len() > JETON_MAX.div_ceil(3) * 4 {
                return Err(self.erreur(ErreurExport::JetonMalEncode("jeton trop volumineux")));
            }
            base64.push_str(morceau);
        }
        let der = Base64::decode_vec(&base64)
            .map_err(|_| self.erreur(ErreurExport::JetonMalEncode("base64 invalide")))?;
        if der.is_empty() || Base64::encode_string(&der) != base64 {
            return Err(self.erreur(ErreurExport::JetonMalEncode("base64 non canonique")));
        }
        Ok(Some(Element::Jeton {
            ligne: debut,
            jeton: Box::new(JetonExporte { nombre, tete, der }),
        }))
    }

    fn nombre(&mut self) -> Result<u64, ErreurLecture> {
        let nombre = self.valeur("nombre")?;
        lire_nombre(&nombre).ok_or_else(|| self.erreur(ErreurExport::NombreInvalide))
    }

    /// `fin-export`, puis plus rien.
    fn fin(&mut self) -> Result<(), ErreurLecture> {
        if self.ligne_requise()? != FIN_EXPORT {
            return Err(self.erreur(ErreurExport::Attendu(FIN_EXPORT)));
        }
        self.verifier_fin_de_fichier()
    }

    fn verifier_fin_de_fichier(&mut self) -> Result<(), ErreurLecture> {
        let mut reste = [0u8; 1];
        if self.entree.read(&mut reste)? != 0 {
            self.ligne += 1;
            return Err(self.erreur(ErreurExport::DonneesApresFin));
        }
        self.etape = Etape::Termine;
        Ok(())
    }

    fn erreur(&self, erreur: ErreurExport) -> ErreurLecture {
        ErreurLecture::Forme {
            ligne: self.ligne,
            erreur,
        }
    }

    /// Ligne suivante, sans sa fin de ligne ; erreur en fin de fichier.
    fn ligne_requise(&mut self) -> Result<String, ErreurLecture> {
        self.tampon.clear();
        let lus = (&mut self.entree)
            .take(LIGNE_MAX as u64)
            .read_until(b'\n', &mut self.tampon)?;
        self.ligne += 1;
        if lus == 0 {
            return Err(self.erreur(ErreurExport::Tronque));
        }
        if self.tampon.last() != Some(&b'\n') {
            return Err(self.erreur(if lus == LIGNE_MAX {
                ErreurExport::LigneTropLongue
            } else {
                ErreurExport::FinDeLigneManquante
            }));
        }
        self.tampon.pop();
        String::from_utf8(std::mem::take(&mut self.tampon))
            .map_err(|_| self.erreur(ErreurExport::PasUtf8))
    }

    /// Valeur de la ligne `cle=valeur` suivante.
    fn valeur(&mut self, cle: &'static str) -> Result<String, ErreurLecture> {
        let l = self.ligne_requise()?;
        match l.split_once('=') {
            Some((k, v)) if k == cle => Ok(v.to_owned()),
            _ => Err(self.erreur(ErreurExport::Attendu(cle))),
        }
    }
}

/// Entier décimal sans zéro de tête (`0`, `12`).
pub(crate) fn lire_nombre(texte: &str) -> Option<u64> {
    let canonique = !texte.is_empty()
        && texte.bytes().all(|b| b.is_ascii_digit())
        && (texte == "0" || !texte.starts_with('0'));
    if canonique { texte.parse().ok() } else { None }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nombres() {
        assert_eq!(lire_nombre("0"), Some(0));
        assert_eq!(lire_nombre("12"), Some(12));
        for invalide in ["", "01", "+1", "-1", "1 ", "99999999999999999999999"] {
            assert_eq!(lire_nombre(invalide), None, "{invalide}");
        }
    }

    fn avec_jeton(lignes_der: &[&str]) -> String {
        let tete = crate::chaine::genese(&IdDossier::parse("demo").unwrap());
        let mut s = format!(
            "{ENTETE_EXPORT_V2}\ndossier=demo\ntete={tete}\nnombre=0\njeton\nnombre=0\ntete={tete}\n"
        );
        for l in lignes_der {
            s.push_str("der=");
            s.push_str(l);
            s.push('\n');
        }
        s.push_str("fin-jeton\nfin-export\n");
        s
    }

    #[test]
    fn jetons_lus_et_defauts_d_encodage() {
        let ligne = "A".repeat(BASE64_LIGNE);
        // Lecture : 64 + 4 caractères → 51 octets.
        let texte = avec_jeton(&[&ligne, "AAAA"]);
        let (mut lecteur, _) = LecteurExport::ouvrir(texte.as_bytes()).unwrap();
        assert!(matches!(
            lecteur.suivant().unwrap(),
            Some(Element::Fin { .. })
        ));
        match lecteur.suivant().unwrap() {
            Some(Element::Jeton { jeton, ligne: 5 }) => assert_eq!(jeton.der.len(), 51),
            autre => panic!("{autre:?}"),
        }
        assert_eq!(lecteur.suivant().unwrap(), None);
        // Découpage non canonique, base64 invalide ou non canonique, jeton vide.
        for (lignes, ligne_attendue) in [
            (vec!["AAAA", "AAAA"], 9),
            (vec![ligne.as_str(), ""], 9),
            (vec!["A*AA"], 9),
            (vec!["AB=="], 9),
        ] {
            let (l, e) = erreur(avec_jeton(&lignes).as_bytes());
            assert_eq!(l, ligne_attendue, "{lignes:?}");
            assert!(
                matches!(e, ErreurExport::JetonMalEncode(_)),
                "{lignes:?} {e:?}"
            );
        }
        assert!(matches!(
            erreur(avec_jeton(&[]).as_bytes()).1,
            ErreurExport::JetonMalEncode(_)
        ));
    }

    #[test]
    fn export_vide() {
        let dossier = IdDossier::parse("demo").unwrap();
        let mut sortie = Vec::new();
        let tete = exporter(&dossier, &[], &[], &mut sortie).unwrap();
        assert_eq!(tete, crate::chaine::genese(&dossier));
        let (mut lecteur, lu) = LecteurExport::ouvrir(sortie.as_slice()).unwrap();
        assert_eq!(lu, dossier);
        assert_eq!(
            lecteur.suivant().unwrap(),
            Some(Element::Fin {
                tete,
                nombre: 0,
                ligne: 3
            })
        );
        assert_eq!(lecteur.suivant().unwrap(), None);
        assert_eq!(lecteur.version(), Version::V3);
    }

    #[test]
    fn version_de_l_ecrivain() {
        let dossier = IdDossier::parse("demo").unwrap();
        let mut sortie = Vec::new();
        EcrivainExport::nouveau_v4(&dossier, &mut sortie)
            .unwrap()
            .terminer()
            .unwrap();
        let (lecteur, _) = LecteurExport::ouvrir(sortie.as_slice()).unwrap();
        assert_eq!(lecteur.version(), Version::V4);
        let mut sortie = Vec::new();
        EcrivainExport::nouveau_v5(&dossier, &mut sortie)
            .unwrap()
            .terminer()
            .unwrap();
        let (lecteur, _) = LecteurExport::ouvrir(sortie.as_slice()).unwrap();
        assert_eq!(lecteur.version(), Version::V5);
    }

    #[test]
    fn version_d_export_la_plus_basse() {
        assert_eq!(Version::pour_ecriture(1), Version::V3);
        assert_eq!(Version::pour_ecriture(2), Version::V3);
        assert_eq!(Version::pour_ecriture(3), Version::V4);
        assert_eq!(Version::pour_ecriture(4), Version::V5);
        for v in [Version::V3, Version::V4, Version::V5] {
            assert!(v.ecriture_max() >= 2);
        }
    }

    fn erreur(texte: &[u8]) -> (u64, ErreurExport) {
        let resultat = LecteurExport::ouvrir(texte).and_then(|(mut l, _)| {
            while l.suivant()?.is_some() {}
            Ok(())
        });
        match resultat {
            Err(ErreurLecture::Forme { ligne, erreur }) => (ligne, erreur),
            autre => panic!("défaut attendu : {autre:?}"),
        }
    }

    #[test]
    fn defauts_de_forme() {
        let tete = crate::chaine::genese(&IdDossier::parse("demo").unwrap());
        let valide =
            format!("{ENTETE_EXPORT_V1}\ndossier=demo\ntete={tete}\nnombre=0\nfin-export\n");
        assert_eq!(erreur(b""), (1, ErreurExport::Tronque));
        assert_eq!(
            erreur(b"sigma-books export v6\n").1,
            ErreurExport::Attendu(ENTETE_EXPORT_V5)
        );
        assert_eq!(
            erreur(valide.trim_end().as_bytes()),
            (5, ErreurExport::FinDeLigneManquante)
        );
        assert_eq!(
            erreur(format!("{valide}x").as_bytes()),
            (6, ErreurExport::DonneesApresFin)
        );
        assert_eq!(
            erreur(&valide.as_bytes()[..valide.len() - 11]),
            (5, ErreurExport::Tronque)
        );
        assert_eq!(
            erreur(valide.replace("demo", "dé mo").as_bytes()),
            (2, ErreurExport::DossierInvalide)
        );
        assert_eq!(
            erreur(valide.replace("nombre=0", "nombre=00").as_bytes()),
            (4, ErreurExport::NombreInvalide)
        );
        let longue = format!("{ENTETE_EXPORT_V1}\ndossier={}\n", "a".repeat(2000));
        assert_eq!(
            erreur(longue.as_bytes()),
            (2, ErreurExport::LigneTropLongue)
        );
        // Écriture canonique v2 dans un export v2 : refusée (admise à partir de la v3).
        let v2 = format!("{ENTETE_EXPORT_V2}\ndossier=demo\necriture v2\nnumero=OD-2026-000001\n");
        assert_eq!(
            erreur(v2.as_bytes()),
            (3, ErreurExport::Attendu("ecriture v1 » ou « tete="))
        );
        // Écriture canonique v3 dans un export v3 : refusée (admise à partir de la v4).
        let v3 = format!("{ENTETE_EXPORT_V3}\ndossier=demo\necriture v3\nnumero=OD-2026-000001\n");
        assert_eq!(
            erreur(v3.as_bytes()),
            (
                3,
                ErreurExport::Attendu("ecriture v1 », « ecriture v2 » ou « tete=")
            )
        );
        // v1 : pas de jeton.
        let v1 = format!("{ENTETE_EXPORT_V1}\ndossier=demo\ntete={tete}\nnombre=0\njeton\n");
        assert_eq!(
            erreur(v1.as_bytes()),
            (5, ErreurExport::Attendu(FIN_EXPORT))
        );
        let mut pas_utf8 = valide.clone().into_bytes();
        pas_utf8[25] = 0xff;
        assert_eq!(erreur(&pas_utf8), (2, ErreurExport::PasUtf8));
    }
}
