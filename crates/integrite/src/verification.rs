//! Vérification hors ligne d'un export d'intégrité (spec §6.4) : recalcul de la chaîne,
//! contrôle de la numérotation, de l'équilibre et des contre-passations, vérification des
//! jetons d'horodatage RFC 3161 (export v2), rapport pour l'auditeur.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::io::{self, BufRead};

use compta_core::{CodeExercice, CodeJournal, NumeroEcriture};

use crate::chaine::{Empreinte, IdDossier, chainer, genese};
use crate::export::{Element, ErreurExport, ErreurLecture, JetonExporte, LecteurExport, Version};
use crate::horodatage::{
    Ancre, ErreurJeton, JetonVerifie, Temoin, ancres_par_defaut, verifier_jeton,
};

/// Nombre maximal d'anomalies détaillées dans le rapport (les suivantes sont comptées).
pub const ANOMALIES_DETAILLEES: usize = 100;

/// Contrôles supplémentaires demandés par l'auditeur.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Options {
    /// Dossier attendu.
    pub dossier: Option<IdDossier>,
    /// Empreinte de tête attendue, issue d'une source externe (témoin).
    pub tete: Option<Empreinte>,
    /// Ancres de confiance ajoutées aux racines intégrées (`ancres_par_defaut`).
    pub ancres: Vec<Ancre>,
}

/// Anomalie constatée.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Anomalie {
    /// L'export est illisible à partir de cette ligne ; la vérification s'arrête.
    Illisible {
        /// Ligne du fichier.
        ligne: u64,
        /// Défaut de forme.
        erreur: ErreurExport,
    },
    /// L'export ne concerne pas le dossier attendu.
    DossierInattendu {
        /// Dossier attendu.
        attendu: IdDossier,
        /// Dossier de l'export.
        trouve: IdDossier,
    },
    /// L'empreinte déclarée ne correspond pas au contenu de l'écriture et à l'empreinte
    /// précédente : écriture ou empreinte altérée.
    EmpreinteDifferente {
        /// Écriture concernée.
        numero: NumeroEcriture,
        /// Ligne de l'empreinte déclarée.
        ligne: u64,
    },
    /// Numéro hors séquence dans son journal et son exercice : trou, doublon ou inversion.
    NumerotationDiscontinue {
        /// Numéro trouvé.
        numero: NumeroEcriture,
        /// Rang attendu.
        attendu: u32,
        /// Ligne de l'écriture.
        ligne: u64,
    },
    /// Débits et crédits différents.
    Desequilibre {
        /// Écriture concernée.
        numero: NumeroEcriture,
        /// Ligne de l'écriture.
        ligne: u64,
    },
    /// Contre-passation d'une écriture absente de l'export ou postérieure.
    OrigineInconnue {
        /// Contre-passation.
        numero: NumeroEcriture,
        /// Origine déclarée.
        origine: NumeroEcriture,
        /// Ligne de l'écriture.
        ligne: u64,
    },
    /// La tête déclarée par l'export diffère de la tête recalculée.
    TeteDifferente {
        /// Tête déclarée.
        declaree: Empreinte,
    },
    /// Le nombre d'écritures déclaré diffère du nombre lu.
    NombreDifferent {
        /// Nombre déclaré.
        declare: u64,
    },
    /// La tête recalculée diffère de la tête attendue (source externe).
    AncrageDifferent {
        /// Tête attendue.
        attendue: Empreinte,
    },
    /// Le témoin d'un jeton ne correspond pas à la chaîne recalculée : l'export n'est pas
    /// celui qui a été horodaté.
    TemoinDifferent {
        /// Ligne du jeton.
        ligne: u64,
        /// Nombre d'écritures du témoin.
        nombre: u64,
    },
    /// Jeton d'horodatage invalide (signature, empreinte, chaîne de certificats…).
    JetonInvalide {
        /// Ligne du jeton.
        ligne: u64,
        /// Motif.
        erreur: ErreurJeton,
    },
}

impl fmt::Display for Anomalie {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Illisible { ligne, erreur } => {
                write!(f, "export illisible, ligne {ligne} : {erreur}")
            }
            Self::DossierInattendu { attendu, trouve } => {
                write!(f, "export du dossier « {trouve} », « {attendu} » attendu")
            }
            Self::EmpreinteDifferente { numero, ligne } => write!(
                f,
                "{numero} (ligne {ligne}) : empreinte différente du contenu, écriture ou \
                 empreinte altérée"
            ),
            Self::NumerotationDiscontinue {
                numero,
                attendu,
                ligne,
            } => write!(
                f,
                "{numero} (ligne {ligne}) : numérotation discontinue, rang {attendu:06} attendu"
            ),
            Self::Desequilibre { numero, ligne } => {
                write!(f, "{numero} (ligne {ligne}) : débits et crédits différents")
            }
            Self::OrigineInconnue {
                numero,
                origine,
                ligne,
            } => write!(
                f,
                "{numero} (ligne {ligne}) : contre-passe {origine}, absente de la chaîne avant elle"
            ),
            Self::TeteDifferente { declaree } => {
                write!(
                    f,
                    "tête déclarée {declaree} différente de la tête recalculée"
                )
            }
            Self::NombreDifferent { declare } => {
                write!(
                    f,
                    "{declare} écriture(s) déclarée(s), nombre différent du nombre lu"
                )
            }
            Self::AncrageDifferent { attendue } => write!(
                f,
                "tête recalculée différente de la tête attendue {attendue} : l'export ne \
                 correspond pas à la référence externe"
            ),
            Self::TemoinDifferent { ligne, nombre } => write!(
                f,
                "jeton (ligne {ligne}) : la chaîne recalculée après {nombre} écriture(s) \
                 n'est pas celle qui a été horodatée"
            ),
            Self::JetonInvalide { ligne, erreur } => {
                write!(f, "jeton (ligne {ligne}) invalide : {erreur}")
            }
        }
    }
}

/// Plage de numéros vus pour un journal et un exercice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Plage {
    /// Premier rang.
    pub premier: u32,
    /// Dernier rang.
    pub dernier: u32,
}

/// Horodatage vérifié : les `nombre` premières écritures existaient à la date du jeton.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Horodatage {
    /// Écritures couvertes (les `nombre` premières de la chaîne).
    pub nombre: u64,
    /// Ce qu'atteste le jeton.
    pub jeton: JetonVerifie,
}

/// Résultat de la vérification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rapport {
    /// Version du format d'export (absente si l'en-tête est illisible).
    pub version: Option<Version>,
    /// Dossier déclaré par l'export (absent si l'en-tête est illisible).
    pub dossier: Option<IdDossier>,
    /// Écritures lues.
    pub nombre: u64,
    /// Première et dernière écritures de la chaîne.
    pub bornes: Option<(NumeroEcriture, NumeroEcriture)>,
    /// Numéros par journal et exercice.
    pub sequences: BTreeMap<(CodeJournal, CodeExercice), Plage>,
    /// Tête recalculée à partir des écritures lues.
    pub tete: Option<Empreinte>,
    /// Vrai si la tête a été comparée à une référence externe (et concorde, sauf anomalie).
    pub ancre: bool,
    /// Jetons d'horodatage vérifiés, dans l'ordre de l'export.
    pub horodatages: Vec<Horodatage>,
    /// Anomalies détaillées (au plus [`ANOMALIES_DETAILLEES`]).
    pub anomalies: Vec<Anomalie>,
    /// Anomalies non détaillées.
    pub anomalies_omises: u64,
}

impl Rapport {
    /// Vrai si aucune anomalie n'a été constatée.
    #[must_use]
    pub fn est_integre(&self) -> bool {
        self.anomalies.is_empty()
    }

    fn vide() -> Self {
        Self {
            version: None,
            dossier: None,
            nombre: 0,
            bornes: None,
            sequences: BTreeMap::new(),
            tete: None,
            ancre: false,
            horodatages: Vec::new(),
            anomalies: Vec::new(),
            anomalies_omises: 0,
        }
    }

    fn signaler(&mut self, anomalie: Anomalie) {
        if self.anomalies.len() < ANOMALIES_DETAILLEES {
            self.anomalies.push(anomalie);
        } else {
            self.anomalies_omises += 1;
        }
    }
}

/// Vérifie un export. Les défauts de forme et les altérations sont des anomalies du rapport ;
/// seule une erreur d'entrée-sortie interrompt la vérification sans rapport.
///
/// # Errors
/// Erreur d'entrée-sortie.
pub fn verifier<R: BufRead>(entree: R, options: &Options) -> io::Result<Rapport> {
    let mut rapport = Rapport::vide();
    let (mut lecteur, dossier) = match LecteurExport::ouvrir(entree) {
        Ok(ouvert) => ouvert,
        Err(e) => return illisible(rapport, e),
    };
    rapport.dossier = Some(dossier.clone());
    rapport.version = Some(lecteur.version());
    let mut ancres = match ancres_par_defaut() {
        Ok(ancres) => ancres,
        Err(e) => return Err(io::Error::other(e)),
    };
    ancres.extend(options.ancres.iter().cloned());
    // Têtes recalculées après chaque écriture (32 octets par écriture) : un témoin peut porter
    // sur n'importe quel état passé de la chaîne.
    let mut tetes: Vec<Empreinte> = vec![genese(&dossier)];
    if let Some(attendu) = &options.dossier
        && *attendu != dossier
    {
        rapport.signaler(Anomalie::DossierInattendu {
            attendu: attendu.clone(),
            trouve: dossier.clone(),
        });
    }
    let mut recalculee = genese(&dossier);
    let mut precedente_declaree = recalculee;
    let mut vus: BTreeSet<NumeroEcriture> = BTreeSet::new();
    loop {
        let element = match lecteur.suivant() {
            Ok(Some(element)) => element,
            Ok(None) => break,
            Err(e) => return illisible(rapport, e),
        };
        match element {
            Element::Ecriture {
                ligne,
                ecriture,
                contenu,
                empreinte,
                ligne_empreinte,
            } => {
                let numero = ecriture.numero().clone();
                rapport.nombre += 1;
                recalculee = chainer(&recalculee, contenu.as_bytes());
                tetes.push(recalculee);
                if chainer(&precedente_declaree, contenu.as_bytes()) != empreinte {
                    rapport.signaler(Anomalie::EmpreinteDifferente {
                        numero: numero.clone(),
                        ligne: ligne_empreinte,
                    });
                }
                precedente_declaree = empreinte;
                controler_numero(&mut rapport, &numero, ligne);
                if !ecriture.est_equilibree() {
                    rapport.signaler(Anomalie::Desequilibre {
                        numero: numero.clone(),
                        ligne,
                    });
                }
                if let Some(origine) = ecriture.origine()
                    && !vus.contains(origine)
                {
                    rapport.signaler(Anomalie::OrigineInconnue {
                        numero: numero.clone(),
                        origine: origine.clone(),
                        ligne,
                    });
                }
                rapport.bornes = Some(match rapport.bornes.take() {
                    None => (numero.clone(), numero.clone()),
                    Some((premiere, _)) => (premiere, numero.clone()),
                });
                vus.insert(numero);
            }
            Element::Fin { tete, nombre, .. } => {
                if tete != recalculee {
                    rapport.signaler(Anomalie::TeteDifferente { declaree: tete });
                }
                if nombre != rapport.nombre {
                    rapport.signaler(Anomalie::NombreDifferent { declare: nombre });
                }
            }
            Element::Jeton { ligne, jeton } => {
                controler_jeton(&mut rapport, &tetes, &dossier, &ancres, ligne, &jeton);
            }
        }
    }
    rapport.tete = Some(recalculee);
    if let Some(attendue) = options.tete {
        rapport.ancre = true;
        if attendue != recalculee {
            rapport.signaler(Anomalie::AncrageDifferent { attendue });
        }
    }
    Ok(rapport)
}

/// Rattache un jeton à l'état de la chaîne recalculée qu'il décrit, puis le vérifie.
fn controler_jeton(
    rapport: &mut Rapport,
    tetes: &[Empreinte],
    dossier: &IdDossier,
    ancres: &[Ancre],
    ligne: u64,
    jeton: &JetonExporte,
) {
    let recalculee_alors = usize::try_from(jeton.nombre)
        .ok()
        .and_then(|n| tetes.get(n));
    if recalculee_alors != Some(&jeton.tete) {
        rapport.signaler(Anomalie::TemoinDifferent {
            ligne,
            nombre: jeton.nombre,
        });
        return;
    }
    let temoin = Temoin::nouveau(dossier.clone(), jeton.tete, jeton.nombre);
    match verifier_jeton(&jeton.der, &temoin, ancres) {
        Ok(verifie) => rapport.horodatages.push(Horodatage {
            nombre: jeton.nombre,
            jeton: verifie,
        }),
        Err(erreur) => rapport.signaler(Anomalie::JetonInvalide { ligne, erreur }),
    }
}

fn date(jeton: &JetonVerifie) -> String {
    let d = jeton.date;
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        d.year(),
        u8::from(d.month()),
        d.day(),
        d.hour(),
        d.minute(),
        d.second()
    )
}

fn controler_numero(rapport: &mut Rapport, numero: &NumeroEcriture, ligne: u64) {
    let cle = (numero.journal().clone(), numero.exercice().clone());
    let attendu = rapport
        .sequences
        .get(&cle)
        .map_or(1, |p| p.dernier.saturating_add(1));
    if numero.sequence() != attendu {
        rapport.signaler(Anomalie::NumerotationDiscontinue {
            numero: numero.clone(),
            attendu,
            ligne,
        });
    }
    rapport
        .sequences
        .entry(cle)
        .and_modify(|p| p.dernier = numero.sequence())
        .or_insert(Plage {
            premier: numero.sequence(),
            dernier: numero.sequence(),
        });
}

fn illisible(mut rapport: Rapport, erreur: ErreurLecture) -> io::Result<Rapport> {
    match erreur {
        ErreurLecture::Io(e) => Err(e),
        ErreurLecture::Forme { ligne, erreur } => {
            rapport.signaler(Anomalie::Illisible { ligne, erreur });
            Ok(rapport)
        }
    }
}

impl fmt::Display for Rapport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let version = match self.version {
            Some(Version::V1) => "export v1",
            Some(Version::V2) => "export v2",
            None => "export illisible",
        };
        writeln!(f, "Sigma Books — vérification d'intégrité ({version})")?;
        match &self.dossier {
            Some(d) => writeln!(f, "Dossier            : {d}")?,
            None => writeln!(f, "Dossier            : illisible")?,
        }
        write!(f, "Écritures          : {}", self.nombre)?;
        if let Some((premiere, derniere)) = &self.bornes {
            write!(f, " (de {premiere} à {derniere})")?;
        }
        writeln!(f)?;
        if !self.sequences.is_empty() {
            writeln!(f, "Numérotation       :")?;
            for ((journal, exercice), plage) in &self.sequences {
                writeln!(
                    f,
                    "  {journal} {exercice} : {:06} à {:06}",
                    plage.premier, plage.dernier
                )?;
            }
        }
        if let Some(tete) = &self.tete {
            writeln!(f, "Empreinte de tête  : {tete}")?;
        }
        match self.horodatages.iter().max_by_key(|h| h.nombre) {
            None => writeln!(f, "Horodatage         : aucun jeton vérifié")?,
            Some(dernier) => {
                writeln!(
                    f,
                    "Horodatage         : {} jeton(s) vérifié(s)",
                    self.horodatages.len()
                )?;
                for h in &self.horodatages {
                    writeln!(
                        f,
                        "  {} premières écritures au plus tard le {} UTC — {} (racine : {})",
                        h.nombre,
                        date(&h.jeton),
                        h.jeton.autorite,
                        h.jeton.ancre
                    )?;
                }
                if dernier.nombre < self.nombre {
                    writeln!(
                        f,
                        "  {} écriture(s) postérieure(s) au dernier jeton, non couvertes",
                        self.nombre - dernier.nombre
                    )?;
                }
            }
        }
        writeln!(f)?;
        if self.est_integre() {
            writeln!(
                f,
                "RÉSULTAT : INTÈGRE — chaîne recalculée, numérotation continue, écritures \
                 équilibrées."
            )?;
            if !self.ancre && self.horodatages.is_empty() {
                writeln!(
                    f,
                    "Attention : sans référence externe, une réécriture complète de l'export ne \
                     serait pas détectée. Il faut un jeton d'horodatage dans l'export ou une \
                     empreinte de tête conservée par un témoin (option --tete)."
                )?;
            }
        } else {
            let total = self.anomalies.len() as u64 + self.anomalies_omises;
            writeln!(f, "RÉSULTAT : ANOMALIES ({total})")?;
            for a in &self.anomalies {
                writeln!(f, "  - {a}")?;
            }
            if self.anomalies_omises > 0 {
                writeln!(f, "  … et {} autre(s)", self.anomalies_omises)?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chaque_anomalie_a_un_message() {
        let numero = NumeroEcriture::parse("VT-2026-000002").unwrap();
        let empreinte = genese(&IdDossier::parse("a").unwrap());
        let anomalies = [
            Anomalie::Illisible {
                ligne: 3,
                erreur: ErreurExport::Tronque,
            },
            Anomalie::DossierInattendu {
                attendu: IdDossier::parse("a").unwrap(),
                trouve: IdDossier::parse("b").unwrap(),
            },
            Anomalie::EmpreinteDifferente {
                numero: numero.clone(),
                ligne: 4,
            },
            Anomalie::NumerotationDiscontinue {
                numero: numero.clone(),
                attendu: 1,
                ligne: 5,
            },
            Anomalie::Desequilibre {
                numero: numero.clone(),
                ligne: 6,
            },
            Anomalie::OrigineInconnue {
                numero: numero.clone(),
                origine: numero,
                ligne: 7,
            },
            Anomalie::TeteDifferente {
                declaree: empreinte,
            },
            Anomalie::NombreDifferent { declare: 9 },
            Anomalie::AncrageDifferent {
                attendue: empreinte,
            },
            Anomalie::TemoinDifferent {
                ligne: 8,
                nombre: 2,
            },
            Anomalie::JetonInvalide {
                ligne: 9,
                erreur: ErreurJeton::ChaineNonFiable,
            },
        ];
        let mut rapport = Rapport {
            version: None,
            horodatages: Vec::new(),
            dossier: None,
            nombre: 0,
            bornes: None,
            sequences: BTreeMap::new(),
            tete: None,
            ancre: false,
            anomalies: Vec::new(),
            anomalies_omises: 0,
        };
        for a in anomalies {
            assert!(a.to_string().len() > 10);
            rapport.signaler(a);
        }
        let texte = rapport.to_string();
        assert!(texte.contains("Dossier            : illisible"));
        assert!(texte.contains("RÉSULTAT : ANOMALIES (11)"));
        assert!(texte.contains("ligne 3 : fin du fichier"));
        assert!(texte.contains("rang 000001 attendu"));
    }

    #[test]
    fn erreur_d_entree_sortie_remontee() {
        struct EnPanne;
        impl io::Read for EnPanne {
            fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
                Err(io::Error::other("disque"))
            }
        }
        let resultat = verifier(io::BufReader::new(EnPanne), &Options::default());
        assert!(resultat.is_err());
    }
}
