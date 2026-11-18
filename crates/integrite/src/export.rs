//! Export d'intégrité, format **v1** : les écritures validées d'un dossier dans l'ordre de la
//! chaîne, chacune suivie de son empreinte, puis la tête et le nombre d'écritures.
//!
//! Format décrit dans `docs/integrite/format-v1.md`. La lecture traite le fichier comme
//! **hostile** : lecture ligne à ligne bornée, aucune allocation proportionnelle à une valeur
//! lue, erreurs typées avec numéro de ligne, aucune panique.

use std::io::{self, BufRead, Read as _, Write};

use compta_core::EcritureValidee;

use crate::canonique::{self, EcritureCanonique};
use crate::chaine::{Chaine, Empreinte, IdDossier};

/// Première ligne d'un export v1.
pub const ENTETE_EXPORT_V1: &str = "sigma-books export v1";
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
    /// Fin de l'export.
    Fin {
        /// Tête déclarée.
        tete: Empreinte,
        /// Nombre d'écritures déclaré.
        nombre: u64,
        /// Ligne de la tête déclarée.
        ligne: u64,
    },
}

/// Écrit un export v1, écriture par écriture.
pub struct EcrivainExport<W: Write> {
    sortie: W,
    chaine: Chaine,
}

impl<W: Write> EcrivainExport<W> {
    /// Écrit l'en-tête.
    ///
    /// # Errors
    /// Erreur d'écriture.
    pub fn nouveau(dossier: &IdDossier, mut sortie: W) -> io::Result<Self> {
        writeln!(sortie, "{ENTETE_EXPORT_V1}")?;
        writeln!(sortie, "dossier={dossier}")?;
        Ok(Self {
            sortie,
            chaine: Chaine::ouvrir(dossier),
        })
    }

    /// Ajoute l'écriture suivante de la chaîne et son empreinte.
    ///
    /// # Errors
    /// Erreur d'écriture.
    pub fn ecriture(&mut self, ecriture: &EcritureValidee) -> io::Result<Empreinte> {
        let contenu = EcritureCanonique::depuis(ecriture).serialiser();
        let empreinte = self.chaine.ajouter(ecriture);
        self.sortie.write_all(contenu.as_bytes())?;
        writeln!(self.sortie, "empreinte={empreinte}")?;
        Ok(empreinte)
    }

    /// Écrit la tête et le nombre d'écritures, puis rend la sortie et la tête.
    ///
    /// # Errors
    /// Erreur d'écriture.
    pub fn terminer(mut self) -> io::Result<(W, Empreinte)> {
        let tete = *self.chaine.tete();
        writeln!(self.sortie, "tete={tete}")?;
        writeln!(self.sortie, "nombre={}", self.chaine.longueur())?;
        writeln!(self.sortie, "{FIN_EXPORT}")?;
        self.sortie.flush()?;
        Ok((self.sortie, tete))
    }
}

/// Exporte des écritures validées, dans l'ordre de la chaîne. Renvoie la tête.
///
/// # Errors
/// Erreur d'écriture.
pub fn exporter<W: Write>(
    dossier: &IdDossier,
    ecritures: &[EcritureValidee],
    sortie: W,
) -> io::Result<Empreinte> {
    let mut ecrivain = EcrivainExport::nouveau(dossier, sortie)?;
    for e in ecritures {
        ecrivain.ecriture(e)?;
    }
    ecrivain.terminer().map(|(_, tete)| tete)
}

/// Lit un export v1 élément par élément, sans le charger en mémoire.
pub struct LecteurExport<R: BufRead> {
    entree: R,
    ligne: u64,
    tampon: Vec<u8>,
    termine: bool,
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
            termine: false,
        };
        let entete = lecteur.ligne_requise()?;
        if entete != ENTETE_EXPORT_V1 {
            return Err(lecteur.erreur(ErreurExport::Attendu(ENTETE_EXPORT_V1)));
        }
        let dossier = lecteur.valeur("dossier")?;
        let dossier = IdDossier::parse(&dossier)
            .map_err(|_| lecteur.erreur(ErreurExport::DossierInvalide))?;
        Ok((lecteur, dossier))
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
        if self.termine {
            return Ok(None);
        }
        let premiere = self.ligne_requise()?;
        let debut = self.ligne;
        if let Some(tete) = premiere.strip_prefix("tete=") {
            let tete =
                Empreinte::parse(tete).map_err(|_| self.erreur(ErreurExport::EmpreinteInvalide))?;
            let nombre = self.valeur("nombre")?;
            let nombre =
                lire_nombre(&nombre).ok_or_else(|| self.erreur(ErreurExport::NombreInvalide))?;
            if self.ligne_requise()? != FIN_EXPORT {
                return Err(self.erreur(ErreurExport::Attendu(FIN_EXPORT)));
            }
            let mut reste = [0u8; 1];
            if self.entree.read(&mut reste)? != 0 {
                self.ligne += 1;
                return Err(self.erreur(ErreurExport::DonneesApresFin));
            }
            self.termine = true;
            return Ok(Some(Element::Fin {
                tete,
                nombre,
                ligne: debut,
            }));
        }
        if premiere != canonique::ENTETE_V1 {
            return Err(self.erreur(ErreurExport::Attendu("ecriture v1 » ou « tete=")));
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
fn lire_nombre(texte: &str) -> Option<u64> {
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

    #[test]
    fn export_vide() {
        let dossier = IdDossier::parse("demo").unwrap();
        let mut sortie = Vec::new();
        let tete = exporter(&dossier, &[], &mut sortie).unwrap();
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
            erreur(b"sigma-books export v2\n").1,
            ErreurExport::Attendu(ENTETE_EXPORT_V1)
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
        let mut pas_utf8 = valide.clone().into_bytes();
        pas_utf8[25] = 0xff;
        assert_eq!(erreur(&pas_utf8), (2, ErreurExport::PasUtf8));
    }
}
