//! Sérialisation canonique d'une écriture validée, format **v1** (spec §6.4).
//!
//! Le format est décrit dans `docs/integrite/format-v1.md`. Il est **figé** : toute
//! modification produit une version 2, jamais un changement silencieux (vecteurs de test dans
//! `tests/vecteurs/`).
//!
//! Une écriture s'écrit en texte UTF-8, une donnée par ligne (`cle=valeur`, fin de ligne `\n`),
//! dans un ordre fixe. Les valeurs ne contiennent jamais de fin de ligne : les textes saisis
//! (`TexteCourt`) excluent les caractères de contrôle, et les autres champs ont un alphabet
//! restreint. La lecture est **stricte** : un texte n'est accepté que s'il est exactement la
//! sérialisation de ce qu'il décrit (garde contre les ambiguïtés, menace F7.3).

use std::fmt::Write as _;

use compta_core::{
    CodeMotif, CodeRegime, Cours, Devise, EcritureValidee, EtiquetteRelue, Exigibilite, IdTiers,
    Ligne, Montant, NatureEtiquette, NumeroCompte, NumeroEcriture, ReferencePiece, Sens, SensTva,
    TauxTva, TexteCourt, VersionParametres,
};
use rust_decimal::Decimal;
use time::{Date, Month};

/// Première ligne d'une écriture au format v1. La version entre dans le contenu haché.
pub const ENTETE_V1: &str = "ecriture v1";
/// Première ligne d'une écriture au format v2 : v1 et une date de valeur comptable (AUDCIF
/// art. 22 4°). Une écriture sans date de valeur comptable reste au format v1, à l'identique.
pub const ENTETE_V2: &str = "ecriture v2";
/// Première ligne d'une écriture au format v3 : v1 et, sur les lignes, une étiquette fiscale
/// (ADR 0019) ; la date de valeur comptable y est facultative. Une écriture sans étiquette
/// garde son format v1 ou v2, à l'identique.
pub const ENTETE_V3: &str = "ecriture v3";
/// Dernière ligne d'une écriture.
pub const FIN: &str = "fin";
/// Marqueur d'une ligne d'écriture.
const LIGNE: &str = "ligne";

/// Erreur de lecture d'une écriture canonique. `ligne` compte à partir de 1 dans l'écriture.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ErreurCanonique {
    /// Une ligne attendue manque ou ne porte pas la bonne clé.
    #[error("ligne {ligne} : « {attendu} » attendu")]
    Attendu {
        /// Ligne fautive.
        ligne: usize,
        /// Clé ou marqueur attendu.
        attendu: &'static str,
    },
    /// La valeur d'une clé est invalide.
    #[error("ligne {ligne} : valeur de « {cle} » invalide")]
    ValeurInvalide {
        /// Ligne fautive.
        ligne: usize,
        /// Clé concernée.
        cle: &'static str,
    },
    /// La contre-valeur XOF d'une ligne en devise ne correspond pas au montant et au cours.
    #[error("ligne {ligne} : contre-valeur XOF incohérente avec le montant en devise et le cours")]
    ContreValeurIncoherente {
        /// Ligne du montant XOF.
        ligne: usize,
    },
    /// Étiquette fiscale d'une ligne incohérente (taxe avec motif, motif hors régime).
    #[error("ligne {ligne} : étiquette fiscale incohérente")]
    EtiquetteIncoherente {
        /// Ligne de `fiscal-nature=`.
        ligne: usize,
    },
    /// Texte après la ligne `fin`.
    #[error("ligne {ligne} : texte après « fin »")]
    TexteEnTrop {
        /// Première ligne en trop.
        ligne: usize,
    },
    /// Le texte décrit une écriture valide mais n'est pas sa forme canonique
    /// (zéros superflus, doublon de pièce…).
    #[error("forme non canonique")]
    NonCanonique,
}

/// Justification telle qu'elle figure au format canonique.
///
/// `sans-piece=` n'existe que dans des exports v1 et v2 anciens : le moteur ne produit plus
/// d'écriture sans pièce (AUDCIF art. 17 et 22), mais ces exports figés restent vérifiables.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JustificationCanonique {
    /// Une ou plusieurs pièces.
    Pieces(Vec<ReferencePiece>),
    /// Motif « sans pièce » (lecture seule).
    SansPiece(TexteCourt),
}

/// Contenu d'une écriture validée tel qu'il entre dans la chaîne de hachage.
///
/// Se construit depuis une [`EcritureValidee`] ([`EcritureCanonique::depuis`]) ou par lecture
/// d'un export ([`EcritureCanonique::lire`]). N'autorise aucune modification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EcritureCanonique {
    numero: NumeroEcriture,
    date: Date,
    date_validation: Date,
    date_valeur_comptable: Option<Date>,
    libelle: TexteCourt,
    origine: Option<NumeroEcriture>,
    parametres: Option<VersionParametres>,
    justification: JustificationCanonique,
    /// Lignes **sans** étiquette : l'étiquette lue est une [`EtiquetteRelue`], qui ne se pose pas
    /// sur une `Ligne` (règle absolue 4).
    lignes: Vec<Ligne>,
    /// Étiquette de chaque ligne, dans le même ordre que `lignes`.
    etiquettes: Vec<Option<EtiquetteRelue>>,
}

impl EcritureCanonique {
    /// Contenu canonique d'une écriture validée.
    #[must_use]
    pub fn depuis(ecriture: &EcritureValidee) -> Self {
        Self {
            numero: ecriture.numero().clone(),
            date: ecriture.date(),
            date_validation: ecriture.date_validation(),
            date_valeur_comptable: ecriture.date_valeur_comptable(),
            libelle: ecriture.libelle().clone(),
            origine: ecriture.origine().cloned(),
            parametres: ecriture.version_parametres().cloned(),
            justification: JustificationCanonique::Pieces(
                ecriture.justification().references().to_vec(),
            ),
            lignes: ecriture
                .lignes()
                .iter()
                .map(|l| l.clone().sans_etiquette())
                .collect(),
            etiquettes: ecriture
                .lignes()
                .iter()
                .map(|l| l.etiquette().map(compta_core::EtiquetteFiscale::en_relue))
                .collect(),
        }
    }

    /// Numéro définitif.
    #[must_use]
    pub fn numero(&self) -> &NumeroEcriture {
        &self.numero
    }

    /// Pour une contre-passation : numéro de l'écriture d'origine.
    #[must_use]
    pub fn origine(&self) -> Option<&NumeroEcriture> {
        self.origine.as_ref()
    }

    /// Lignes, dans l'ordre de saisie (sans leur étiquette : voir [`Self::etiquettes`]).
    #[must_use]
    pub fn lignes(&self) -> &[Ligne] {
        &self.lignes
    }

    /// Étiquette fiscale de chaque ligne, dans l'ordre des lignes.
    #[must_use]
    pub fn etiquettes(&self) -> &[Option<EtiquetteRelue>] {
        &self.etiquettes
    }

    /// Vrai si le total des débits égale le total des crédits (en XOF) et s'il y a des lignes.
    #[must_use]
    pub fn est_equilibree(&self) -> bool {
        let debit: Decimal = self.lignes.iter().map(Ligne::debit).sum();
        let credit: Decimal = self.lignes.iter().map(Ligne::credit).sum();
        !self.lignes.is_empty() && debit == credit
    }

    /// Vrai si une ligne porte une étiquette fiscale (format v3).
    #[must_use]
    pub fn a_une_etiquette(&self) -> bool {
        self.etiquettes.iter().any(Option::is_some)
    }

    /// Date de valeur comptable (formats v2 et v3).
    #[must_use]
    pub fn date_valeur_comptable(&self) -> Option<Date> {
        self.date_valeur_comptable
    }

    /// Sérialisation canonique : les octets hachés. Format v3 si une ligne porte une étiquette
    /// fiscale, sinon v2 si l'écriture porte une date de valeur comptable, sinon v1.
    #[must_use]
    pub fn serialiser(&self) -> String {
        let mut s = String::new();
        let mut cle = |cle: &str, valeur: &dyn std::fmt::Display| {
            // L'écriture dans une `String` ne peut pas échouer.
            let _ = writeln!(s, "{cle}={valeur}");
        };
        cle("numero", &self.numero);
        cle("date", &DateIso(self.date));
        cle("date-validation", &DateIso(self.date_validation));
        if let Some(date_valeur) = self.date_valeur_comptable {
            cle("date-valeur-comptable", &DateIso(date_valeur));
        }
        cle("libelle", &self.libelle);
        if let Some(origine) = &self.origine {
            cle("origine", origine);
        }
        if let Some(parametres) = &self.parametres {
            cle("parametres", &parametres.as_str());
        }
        match &self.justification {
            JustificationCanonique::Pieces(pieces) => {
                for piece in pieces {
                    cle("piece", &piece.as_str());
                }
            }
            JustificationCanonique::SansPiece(motif) => cle("sans-piece", motif),
        }
        let mut corps = String::new();
        for (ligne, etiquette) in self.lignes.iter().zip(&self.etiquettes) {
            corps.push_str(LIGNE);
            corps.push('\n');
            let mut cle = |cle: &str, valeur: &dyn std::fmt::Display| {
                let _ = writeln!(corps, "{cle}={valeur}");
            };
            cle("compte", &ligne.compte().as_str());
            cle(
                "sens",
                &match ligne.sens() {
                    Sens::Debit => "D",
                    Sens::Credit => "C",
                },
            );
            cle("montant", &Nombre(ligne.montant().valeur()));
            if let Some(devise) = ligne.devise() {
                cle("devise", &devise.montant().devise());
                cle("montant-devise", &Nombre(devise.montant().valeur()));
                cle("cours", &Nombre(devise.cours().valeur()));
            }
            if let Some(tiers) = ligne.tiers() {
                cle("tiers", tiers);
            }
            if let Some(e) = etiquette {
                cle("fiscal-nature", &e.nature().code());
                cle("fiscal-sens", &e.sens().code());
                cle("fiscal-regime", e.regime());
                if let Some(taux) = e.taux() {
                    cle("fiscal-taux", &taux);
                }
                if let Some(exigibilite) = e.exigibilite() {
                    cle("fiscal-exigibilite", &exigibilite.code());
                }
                if let Some(motif) = e.motif() {
                    cle("fiscal-motif", motif);
                }
            }
        }
        let entete = if self.a_une_etiquette() {
            ENTETE_V3
        } else if self.date_valeur_comptable.is_some() {
            ENTETE_V2
        } else {
            ENTETE_V1
        };
        format!("{entete}\n{s}{corps}{FIN}\n")
    }

    /// Lit une écriture au format v1, v2 ou v3 et vérifie que le texte est exactement sa forme
    /// canonique.
    ///
    /// # Errors
    /// [`ErreurCanonique`] : structure, valeur, contre-valeur ou forme non canonique.
    pub fn lire(texte: &str) -> Result<Self, ErreurCanonique> {
        let ecriture = Lecteur::nouveau(texte).ecriture()?;
        if ecriture.serialiser() == texte {
            Ok(ecriture)
        } else {
            Err(ErreurCanonique::NonCanonique)
        }
    }
}

/// Nombre décimal sous forme canonique : sans zéro superflu, sans exposant (`228.67`, `150000`).
struct Nombre(Decimal);

impl std::fmt::Display for Nombre {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0.normalize())
    }
}

/// Date `AAAA-MM-JJ`.
struct DateIso(Date);

impl std::fmt::Display for DateIso {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{:04}-{:02}-{:02}",
            self.0.year(),
            u8::from(self.0.month()),
            self.0.day()
        )
    }
}

fn lire_date(texte: &str) -> Option<Date> {
    let octets = texte.as_bytes();
    let forme = octets.len() == 10
        && octets[4] == b'-'
        && octets[7] == b'-'
        && octets
            .iter()
            .enumerate()
            .all(|(i, b)| i == 4 || i == 7 || b.is_ascii_digit());
    if !forme {
        return None;
    }
    let annee: i32 = texte.get(0..4)?.parse().ok()?;
    let mois: u8 = texte.get(5..7)?.parse().ok()?;
    let jour: u8 = texte.get(8..10)?.parse().ok()?;
    Date::from_calendar_date(annee, Month::try_from(mois).ok()?, jour).ok()
}

/// Lecture séquentielle des lignes d'une écriture.
struct Lecteur<'a> {
    lignes: std::iter::Peekable<std::iter::Enumerate<std::str::Split<'a, char>>>,
}

impl<'a> Lecteur<'a> {
    fn nouveau(texte: &'a str) -> Self {
        // `split('\n')` sur un texte terminé par `\n` donne une dernière pièce vide, contrôlée
        // en fin de lecture.
        Self {
            lignes: texte.split('\n').enumerate().peekable(),
        }
    }

    /// Prochaine ligne (numéro à partir de 1, contenu) ; `None` en fin de texte.
    fn suivante(&mut self) -> Option<(usize, &'a str)> {
        self.lignes.next().map(|(i, l)| (i + 1, l))
    }

    fn rang_courant(&mut self) -> usize {
        self.lignes.peek().map_or(0, |(i, _)| i + 1)
    }

    /// Valeur de la clé attendue sur la ligne suivante.
    fn cle(&mut self, cle: &'static str) -> Result<(usize, &'a str), ErreurCanonique> {
        let rang = self.rang_courant();
        match self.suivante() {
            Some((ligne, l)) => match l.split_once('=') {
                Some((k, v)) if k == cle => Ok((ligne, v)),
                _ => Err(ErreurCanonique::Attendu {
                    ligne,
                    attendu: cle,
                }),
            },
            None => Err(ErreurCanonique::Attendu {
                ligne: rang,
                attendu: cle,
            }),
        }
    }

    /// Valeur de la clé si la ligne suivante la porte.
    fn cle_optionnelle(
        &mut self,
        cle: &'static str,
    ) -> Result<Option<(usize, &'a str)>, ErreurCanonique> {
        let presente = self
            .lignes
            .peek()
            .and_then(|(_, l)| l.split_once('='))
            .is_some_and(|(k, _)| k == cle);
        if presente {
            self.cle(cle).map(Some)
        } else {
            Ok(None)
        }
    }

    fn valeur<T, E>(
        &mut self,
        cle: &'static str,
        analyse: impl FnOnce(&str) -> Result<T, E>,
    ) -> Result<T, ErreurCanonique> {
        let (ligne, v) = self.cle(cle)?;
        analyse(v).map_err(|_| ErreurCanonique::ValeurInvalide { ligne, cle })
    }

    fn valeur_optionnelle<T, E>(
        &mut self,
        cle: &'static str,
        analyse: impl FnOnce(&str) -> Result<T, E>,
    ) -> Result<Option<T>, ErreurCanonique> {
        match self.cle_optionnelle(cle)? {
            Some((ligne, v)) => analyse(v)
                .map(Some)
                .map_err(|_| ErreurCanonique::ValeurInvalide { ligne, cle }),
            None => Ok(None),
        }
    }

    fn ecriture(mut self) -> Result<EcritureCanonique, ErreurCanonique> {
        let version = match self.suivante() {
            Some((_, ENTETE_V1)) => 1,
            Some((_, ENTETE_V2)) => 2,
            Some((_, ENTETE_V3)) => 3,
            Some((ligne, _)) => {
                return Err(ErreurCanonique::Attendu {
                    ligne,
                    attendu: ENTETE_V1,
                });
            }
            None => {
                return Err(ErreurCanonique::Attendu {
                    ligne: 0,
                    attendu: ENTETE_V1,
                });
            }
        };
        let numero = self.valeur("numero", NumeroEcriture::parse)?;
        let date = self.valeur("date", |v| lire_date(v).ok_or(()))?;
        let date_validation = self.valeur("date-validation", |v| lire_date(v).ok_or(()))?;
        let date_valeur_comptable = match version {
            1 => None,
            2 => Some(self.valeur("date-valeur-comptable", |v| lire_date(v).ok_or(()))?),
            _ => self.valeur_optionnelle("date-valeur-comptable", |v| lire_date(v).ok_or(()))?,
        };
        let libelle = self.valeur("libelle", TexteCourt::parse)?;
        let origine = self.valeur_optionnelle("origine", NumeroEcriture::parse)?;
        let parametres = self.valeur_optionnelle("parametres", VersionParametres::parse)?;
        let rang_justification = self.rang_courant();
        let justification = self.justification()?;
        // `sans-piece=` n'existe qu'en v1 (exports anciens) : les formats v2 et v3 exigent des
        // pièces.
        if version >= 2 && matches!(justification, JustificationCanonique::SansPiece(_)) {
            return Err(ErreurCanonique::Attendu {
                ligne: rang_justification,
                attendu: "piece",
            });
        }
        let mut lignes = Vec::new();
        let mut etiquettes = Vec::new();
        loop {
            match self.suivante() {
                Some((_, l)) if l == LIGNE => {
                    let (ligne, etiquette) = self.ligne(version >= 3)?;
                    lignes.push(ligne);
                    etiquettes.push(etiquette);
                }
                Some((_, l)) if l == FIN => break,
                Some((ligne, _)) => {
                    return Err(ErreurCanonique::Attendu {
                        ligne,
                        attendu: "ligne",
                    });
                }
                None => {
                    return Err(ErreurCanonique::Attendu {
                        ligne: 0,
                        attendu: FIN,
                    });
                }
            }
        }
        // Après `fin\n`, il ne reste que la pièce vide finale.
        match (self.suivante(), self.suivante()) {
            (Some((_, "")), None) => {}
            (Some((ligne, _)), _) => return Err(ErreurCanonique::TexteEnTrop { ligne }),
            (None, _) => {
                return Err(ErreurCanonique::Attendu {
                    ligne: 0,
                    attendu: FIN,
                });
            }
        }
        Ok(EcritureCanonique {
            numero,
            date,
            date_validation,
            date_valeur_comptable,
            libelle,
            origine,
            parametres,
            justification,
            lignes,
            etiquettes,
        })
    }

    fn justification(&mut self) -> Result<JustificationCanonique, ErreurCanonique> {
        if let Some(motif) = self.valeur_optionnelle("sans-piece", TexteCourt::parse)? {
            return Ok(JustificationCanonique::SansPiece(motif));
        }
        let rang = self.rang_courant();
        let mut pieces = Vec::new();
        while let Some(piece) = self.valeur_optionnelle("piece", ReferencePiece::parse)? {
            pieces.push(piece);
        }
        if pieces.is_empty() {
            return Err(ErreurCanonique::Attendu {
                ligne: rang,
                attendu: "piece",
            });
        }
        // Une pièce en double n'a pas de forme canonique (le moteur retire les doublons).
        if pieces
            .iter()
            .enumerate()
            .any(|(i, p)| pieces[..i].contains(p))
        {
            return Err(ErreurCanonique::NonCanonique);
        }
        Ok(JustificationCanonique::Pieces(pieces))
    }

    fn ligne(
        &mut self,
        avec_etiquette: bool,
    ) -> Result<(Ligne, Option<EtiquetteRelue>), ErreurCanonique> {
        let compte = self.valeur("compte", NumeroCompte::parse)?;
        let sens = self.valeur("sens", |v| match v {
            "D" => Ok(Sens::Debit),
            "C" => Ok(Sens::Credit),
            _ => Err(()),
        })?;
        let (ligne_montant, montant) = self.cle("montant")?;
        let montant =
            Montant::parse(montant, Devise::XOF).map_err(|_| ErreurCanonique::ValeurInvalide {
                ligne: ligne_montant,
                cle: "montant",
            })?;
        let ligne = match self.valeur_optionnelle("devise", |v| {
            Devise::parse(v)
                .ok()
                .filter(|d| *d != Devise::XOF)
                .ok_or(())
        })? {
            Some(devise) => {
                let montant_devise =
                    self.valeur("montant-devise", |v| Montant::parse(v, devise))?;
                let cours = self.valeur("cours", Cours::parse)?;
                let l = Ligne::en_devise(compte, sens, montant_devise, cours).map_err(|_| {
                    ErreurCanonique::ValeurInvalide {
                        ligne: ligne_montant,
                        cle: "montant-devise",
                    }
                })?;
                if l.montant() != montant {
                    return Err(ErreurCanonique::ContreValeurIncoherente {
                        ligne: ligne_montant,
                    });
                }
                l
            }
            None => Ligne::en_xof(compte, sens, montant).map_err(|_| {
                ErreurCanonique::ValeurInvalide {
                    ligne: ligne_montant,
                    cle: "montant",
                }
            })?,
        };
        let ligne = match self.valeur_optionnelle("tiers", IdTiers::parse)? {
            Some(tiers) => ligne.avec_tiers(tiers),
            None => ligne,
        };
        let etiquette = if avec_etiquette {
            self.etiquette()?
        } else {
            None
        };
        Ok((ligne, etiquette))
    }

    /// Étiquette fiscale d'une ligne (format v3) : tout ou rien. `fiscal-nature`, `fiscal-sens`
    /// et `fiscal-regime` vont ensemble ; `fiscal-taux`, `fiscal-exigibilite` et `fiscal-motif`
    /// sont facultatifs, dans cet ordre.
    fn etiquette(&mut self) -> Result<Option<EtiquetteRelue>, ErreurCanonique> {
        let Some(nature) = self.valeur_optionnelle("fiscal-nature", |v| {
            NatureEtiquette::depuis_code(v).ok_or(())
        })?
        else {
            return Ok(None);
        };
        // `rang_courant` pointait la ligne de `fiscal-sens` ; la nature est la précédente.
        let rang = self.rang_courant().saturating_sub(1);
        let sens = self.valeur("fiscal-sens", |v| SensTva::depuis_code(v).ok_or(()))?;
        let regime = self.valeur("fiscal-regime", CodeRegime::parse)?;
        let taux = self.valeur_optionnelle("fiscal-taux", TauxTva::parse)?;
        let exigibilite = self.valeur_optionnelle("fiscal-exigibilite", |v| {
            Exigibilite::depuis_code(v).ok_or(())
        })?;
        let motif = self.valeur_optionnelle("fiscal-motif", CodeMotif::parse)?;
        EtiquetteRelue::nouvelle(nature, sens, regime, taux, exigibilite, motif)
            .map(Some)
            .map_err(|_| ErreurCanonique::EtiquetteIncoherente { ligne: rang })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exemple() -> String {
        format!(
            "ecriture v1\nnumero=BQ1-2026-000002\ndate=2026-03-15\ndate-validation=2026-03-16\n\
             libelle=Règlement client = facture 12\norigine=BQ1-2026-000001\n\
             parametres=sn-2026.1\npiece={}\n\
             ligne\ncompte=521\nsens=D\nmontant=149998\n\
             ligne\ncompte=4111\nsens=C\nmontant=149998\ndevise=EUR\n\
             montant-devise=228.67\ncours=655.957\ntiers=C001\nfin\n",
            "ab".repeat(32)
        )
    }

    #[test]
    fn aller_retour() {
        let texte = exemple();
        let e = EcritureCanonique::lire(&texte).unwrap();
        assert_eq!(e.serialiser(), texte);
        assert_eq!(e.numero().to_string(), "BQ1-2026-000002");
        assert!(e.est_equilibree());
        assert_eq!(
            e.lignes()[1].devise().unwrap().montant().devise().code(),
            "EUR"
        );
    }

    #[test]
    fn refuse_les_formes_non_canoniques() {
        for (avant, apres) in [
            ("montant=149998\nligne", "montant=149998.00\nligne"),
            ("cours=655.957", "cours=655.9570"),
            ("date=2026-03-15", "date=2026-3-15"),
        ] {
            let texte = exemple().replacen(avant, apres, 1);
            assert!(EcritureCanonique::lire(&texte).is_err(), "{apres}");
        }
        let doublon =
            exemple().replacen("\nligne", &format!("\npiece={}\nligne", "ab".repeat(32)), 1);
        assert_eq!(
            EcritureCanonique::lire(&doublon),
            Err(ErreurCanonique::NonCanonique)
        );
    }

    #[test]
    fn version_v2_et_date_de_valeur_comptable() {
        let v1 = exemple();
        let v2 = v1.replacen("ecriture v1\n", "ecriture v2\n", 1).replacen(
            "date-validation=2026-03-16\n",
            "date-validation=2026-03-16\ndate-valeur-comptable=2026-02-27\n",
            1,
        );
        let e = EcritureCanonique::lire(&v2).unwrap();
        assert_eq!(
            e.date_valeur_comptable(),
            Some(time::macros::date!(2026 - 02 - 27))
        );
        assert_eq!(e.serialiser(), v2);
        // v1 avec date de valeur comptable, v2 sans : refusés.
        let v1_avec = v2.replacen("ecriture v2\n", "ecriture v1\n", 1);
        assert!(EcritureCanonique::lire(&v1_avec).is_err());
        let v2_sans = v1.replacen("ecriture v1\n", "ecriture v2\n", 1);
        assert!(EcritureCanonique::lire(&v2_sans).is_err());
        // « sans-piece= » : admis en v1 (exports anciens), refusé en v2.
        let piece = format!("piece={}\n", "ab".repeat(32));
        let v1_sans_piece = v1.replacen(&piece, "sans-piece=Motif\n", 1);
        assert!(EcritureCanonique::lire(&v1_sans_piece).is_ok());
        let v2_sans_piece = v2.replacen(&piece, "sans-piece=Motif\n", 1);
        assert_eq!(
            EcritureCanonique::lire(&v2_sans_piece),
            Err(ErreurCanonique::Attendu {
                ligne: 9,
                attendu: "piece"
            })
        );
        assert!(
            EcritureCanonique::lire(&v1.replacen("ecriture v1\n", "ecriture v4\n", 1)).is_err()
        );
    }

    /// Exemple v3 : deux lignes étiquetées (une taxe, une base exonérée) et une sans étiquette.
    fn exemple_v3() -> String {
        format!(
            "ecriture v3\nnumero=VT-2026-000007\ndate=2026-03-15\ndate-validation=2026-03-16\n\
             libelle=Vente\nparametres=sn-2026.1\npiece={}\n\
             ligne\ncompte=4111\nsens=D\nmontant=118000\ntiers=C001\n\
             ligne\ncompte=7011\nsens=C\nmontant=100000\n\
             fiscal-nature=base\nfiscal-sens=collectee\nfiscal-regime=IMP18\n\
             fiscal-taux=18\nfiscal-exigibilite=debit\n\
             ligne\ncompte=4431\nsens=C\nmontant=18000\n\
             fiscal-nature=taxe\nfiscal-sens=collectee\nfiscal-regime=IMP18\n\
             fiscal-taux=18\nfiscal-exigibilite=debit\nfin\n",
            "ab".repeat(32)
        )
    }

    #[test]
    fn version_v3_et_etiquette_fiscale() {
        let v3 = exemple_v3();
        let e = EcritureCanonique::lire(&v3).unwrap();
        assert_eq!(e.serialiser(), v3);
        assert!(e.a_une_etiquette());
        assert_eq!(e.date_valeur_comptable(), None);
        let taxe = e.etiquettes()[2].as_ref().unwrap();
        assert_eq!(taxe.nature(), NatureEtiquette::Taxe);
        assert_eq!(taxe.regime().as_str(), "IMP18");
        assert_eq!(e.etiquettes()[0], None);
        // Date de valeur comptable facultative en v3.
        let avec = v3.replacen(
            "date-validation=2026-03-16\n",
            "date-validation=2026-03-16\ndate-valeur-comptable=2026-02-27\n",
            1,
        );
        assert!(EcritureCanonique::lire(&avec).is_ok());
        // Base exonérée avec motif, sans taux ni exigibilité.
        let exo = v3.replace(
            "fiscal-regime=IMP18\nfiscal-taux=18\nfiscal-exigibilite=debit\nligne",
            "fiscal-regime=EXO361\nfiscal-motif=EXO361-7\nligne",
        );
        assert!(EcritureCanonique::lire(&exo).is_ok());
    }

    #[test]
    fn refuse_les_etiquettes_invalides() {
        let v3 = exemple_v3();
        let cas = [
            // Clés en désordre, manquantes, inconnues, en double.
            (
                "fiscal-sens=collectee\nfiscal-regime=IMP18\nfiscal-taux=18\nfiscal-exigibilite=debit\nligne",
                "fiscal-regime=IMP18\nfiscal-sens=collectee\nfiscal-taux=18\nfiscal-exigibilite=debit\nligne",
            ),
            (
                "fiscal-sens=collectee\nfiscal-regime=IMP18\nfiscal-taux=18\nfiscal-exigibilite=debit\nligne",
                "fiscal-regime=IMP18\nfiscal-taux=18\nfiscal-exigibilite=debit\nligne",
            ),
            (
                "fiscal-nature=base\n",
                "fiscal-nature=base\nfiscal-nature=base\n",
            ),
            ("fiscal-nature=base", "fiscal-nature=Base"),
            (
                "fiscal-sens=collectee\nfiscal-regime=IMP18\nfiscal-taux=18\nfiscal-exigibilite=debit\nligne",
                "fiscal-sens=percue\nfiscal-regime=IMP18\nfiscal-taux=18\nfiscal-exigibilite=debit\nligne",
            ),
            (
                "fiscal-exigibilite=debit\nligne",
                "fiscal-exigibilite=credit\nligne",
            ),
            (
                "fiscal-regime=IMP18\nfiscal-taux=18\nfiscal-exigibilite=debit\nligne",
                "fiscal-regime=imp 18\nfiscal-taux=18\nfiscal-exigibilite=debit\nligne",
            ),
            // Taux non canoniques.
            (
                "fiscal-taux=18\nfiscal-exigibilite=debit\nligne",
                "fiscal-taux=18.0\nfiscal-exigibilite=debit\nligne",
            ),
            (
                "fiscal-taux=18\nfiscal-exigibilite=debit\nligne",
                "fiscal-taux=018\nfiscal-exigibilite=debit\nligne",
            ),
            (
                "fiscal-taux=18\nfiscal-exigibilite=debit\nligne",
                "fiscal-taux=0\nfiscal-exigibilite=debit\nligne",
            ),
            (
                "fiscal-taux=18\nfiscal-exigibilite=debit\nligne",
                "fiscal-taux=\nfiscal-exigibilite=debit\nligne",
            ),
            // Motif sur une ligne de taxe ; motif d'un autre régime.
            (
                "fiscal-exigibilite=debit\nfin",
                "fiscal-exigibilite=debit\nfiscal-motif=IMP18-1\nfin",
            ),
            (
                "fiscal-exigibilite=debit\nligne",
                "fiscal-motif=EXO361-7\nligne",
            ),
        ];
        for (avant, apres) in cas {
            let texte = v3.replacen(avant, apres, 1);
            assert_ne!(texte, v3, "{avant}");
            assert!(
                EcritureCanonique::lire(&texte).is_err(),
                "{avant} → {apres}"
            );
        }
        // v3 sans aucune étiquette : ce serait une v1, donc forme non canonique.
        let mut sans = String::new();
        for l in v3.lines().filter(|l| !l.starts_with("fiscal-")) {
            sans.push_str(l);
            sans.push('\n');
        }
        assert_eq!(
            EcritureCanonique::lire(&sans),
            Err(ErreurCanonique::NonCanonique)
        );
        assert!(EcritureCanonique::lire(&sans.replacen("ecriture v3", "ecriture v1", 1)).is_ok());
        // Motif sur une taxe : étiquette incohérente, pas seulement non canonique.
        let taxe_motif = v3.replacen(
            "fiscal-exigibilite=debit\nfin",
            "fiscal-motif=IMP18-1\nfin",
            1,
        );
        assert!(matches!(
            EcritureCanonique::lire(&taxe_motif),
            Err(ErreurCanonique::EtiquetteIncoherente { .. })
        ));
        // Clés fiscales interdites en v1 et en v2.
        let v1 = exemple().replacen(
            "tiers=C001\n",
            "tiers=C001\nfiscal-nature=base\nfiscal-sens=collectee\nfiscal-regime=HORS_CHAMP\n",
            1,
        );
        assert!(EcritureCanonique::lire(&v1).is_err());
        assert!(EcritureCanonique::lire(&v1.replacen("ecriture v1", "ecriture v2", 1)).is_err());
        // v3 sans pièce.
        let sans_piece = v3.replacen(&format!("piece={}\n", "ab".repeat(32)), "sans-piece=M\n", 1);
        assert!(EcritureCanonique::lire(&sans_piece).is_err());
    }

    #[test]
    fn une_ecriture_sans_etiquette_garde_son_format() {
        let v1 = exemple();
        assert_eq!(EcritureCanonique::lire(&v1).unwrap().serialiser(), v1);
        assert!(!EcritureCanonique::lire(&v1).unwrap().a_une_etiquette());
    }

    #[test]
    fn refuse_les_structures_invalides() {
        let cas = [
            ("ecriture v1\n", "ecriture v4\n"),
            ("ecriture v1\n", "ecriture v3\n"),
            ("sens=D", "sens=X"),
            ("montant-devise=228.67", "montant-devise=228.68"),
            ("fin\n", "fin\nfin\n"),
            ("fin\n", ""),
            ("fin\n", "fin"),
            ("tiers=C001\n", "tiers=C001\r\n"),
            ("date-validation=2026-03-16\n", ""),
            ("\nligne\ncompte=521", "\nlignes\ncompte=521"),
        ];
        for (avant, apres) in cas {
            let texte = exemple().replacen(avant, apres, 1);
            assert!(
                EcritureCanonique::lire(&texte).is_err(),
                "{avant} → {apres}"
            );
        }
    }

    #[test]
    fn contre_valeur_incoherente() {
        let texte = exemple().replacen("sens=C\nmontant=149998", "sens=C\nmontant=149999", 1);
        assert_eq!(
            EcritureCanonique::lire(&texte),
            Err(ErreurCanonique::ContreValeurIncoherente { ligne: 16 })
        );
    }

    #[test]
    fn dates() {
        assert!(lire_date("2026-02-29").is_none());
        assert!(lire_date("2024-02-29").is_some());
        assert!(lire_date("+026-01-01").is_none());
        assert!(lire_date("2026-01-01 ").is_none());
    }

    mod proprietes {
        use super::*;
        use proptest::prelude::*;

        fn texte() -> impl Strategy<Value = TexteCourt> {
            // Unicode quelconque, `=` et espaces compris ; jamais de caractère de contrôle.
            "[^\\p{Cc}]{1,60}".prop_filter_map("texte court", |t| TexteCourt::parse(&t).ok())
        }

        fn date() -> impl Strategy<Value = Date> {
            (0i32..3000).prop_map(|j| {
                time::macros::date!(2025 - 01 - 01) + time::Duration::days(i64::from(j))
            })
        }

        /// Étiquette syntaxiquement valide (la cohérence avec un référentiel n'importe pas ici).
        fn etiquette() -> impl Strategy<Value = EtiquetteRelue> {
            (
                prop_oneof![Just(NatureEtiquette::Base), Just(NatureEtiquette::Taxe)],
                prop_oneof![Just(SensTva::Collectee), Just(SensTva::Deductible)],
                prop_oneof![Just("IMP18"), Just("EXO361"), Just("EXO380"), Just("X_1")],
                proptest::option::of(prop_oneof![
                    Just("18"),
                    Just("10"),
                    Just("5.5"),
                    Just("0.000001")
                ]),
                proptest::option::of(prop_oneof![
                    Just(Exigibilite::Debit),
                    Just(Exigibilite::Encaissement)
                ]),
                proptest::option::of("[0-9a-f]{1,2}"),
            )
                .prop_filter_map(
                    "étiquette valide",
                    |(nature, sens, regime, taux, exigibilite, suffixe)| {
                        let regime = CodeRegime::parse(regime).ok()?;
                        let motif = suffixe
                            .map(|s| CodeMotif::parse(&format!("{regime}-{s}")))
                            .transpose()
                            .ok()?;
                        EtiquetteRelue::nouvelle(
                            nature,
                            sens,
                            regime,
                            taux.map(|t| TauxTva::parse(t).unwrap()),
                            exigibilite,
                            motif,
                        )
                        .ok()
                    },
                )
        }

        fn ligne() -> impl Strategy<Value = (Ligne, Option<EtiquetteRelue>)> {
            let compte = prop_oneof![Just("4111"), Just("521"), Just("605"), Just("4011-SENELEC")]
                .prop_map(|c| NumeroCompte::parse(c).unwrap());
            let sens = prop_oneof![Just(Sens::Debit), Just(Sens::Credit)];
            let tiers =
                proptest::option::of("[A-Z0-9]{1,8}".prop_map(|t| IdTiers::parse(&t).unwrap()));
            let devise = proptest::option::of((1i64..10_000_000, 1i64..10_000_000));
            let etiquette = proptest::option::weighted(0.3, etiquette());
            (
                compte,
                sens,
                1i64..1_000_000_000_000,
                devise,
                tiers,
                etiquette,
            )
                .prop_filter_map(
                    "ligne valide",
                    |(compte, sens, xof, devise, tiers, etiquette)| {
                        let ligne = match devise {
                            None => Ligne::en_xof(
                                compte,
                                sens,
                                Montant::nouveau(Decimal::new(xof, 0), Devise::XOF),
                            ),
                            Some((montant, cours)) => Ligne::en_devise(
                                compte,
                                sens,
                                Montant::nouveau(
                                    Decimal::new(montant, 2),
                                    Devise::parse("EUR").ok()?,
                                ),
                                Cours::nouveau(Decimal::new(cours, 3)).ok()?,
                            ),
                        }
                        .ok()?;
                        let ligne = match tiers {
                            Some(t) => ligne.avec_tiers(t),
                            None => ligne,
                        };
                        Some((ligne, etiquette))
                    },
                )
        }

        prop_compose! {
            fn ecriture()(
                rang in 1u32..999_999,
                date in date(),
                date_validation in date(),
                date_valeur_comptable in proptest::option::of(date()),
                libelle in texte(),
                origine in proptest::option::of(1u32..999_999),
                parametres in proptest::option::of("[a-z0-9.-]{1,12}"),
                pieces in proptest::collection::btree_set("[0-9a-f]{64}", 1..4),
                sans_piece in proptest::option::weighted(0.2, texte()),
                lignes_etiquetees in proptest::collection::vec(ligne(), 1..6),
            ) -> EcritureCanonique {
                let (lignes, etiquettes): (Vec<Ligne>, Vec<Option<EtiquetteRelue>>) =
                    lignes_etiquetees.into_iter().unzip();
                let numero = |r: u32| NumeroEcriture::parse(&format!("OD-2026-{r:06}")).unwrap();
                EcritureCanonique {
                    numero: numero(rang),
                    date,
                    date_validation,
                    date_valeur_comptable,
                    libelle,
                    origine: origine.map(numero),
                    parametres: parametres.map(|p| VersionParametres::parse(&p).unwrap()),
                    // « Sans pièce » n'existe qu'en v1 (exports anciens, lecture seule).
                    justification: match sans_piece {
                        Some(motif)
                            if date_valeur_comptable.is_none()
                                && etiquettes.iter().all(Option::is_none) =>
                        {
                            JustificationCanonique::SansPiece(motif)
                        }
                        _ => JustificationCanonique::Pieces(
                            pieces.iter().map(|p| ReferencePiece::parse(p).unwrap()).collect(),
                        ),
                    },
                    lignes,
                    etiquettes,
                }
            }
        }

        proptest! {
            #![proptest_config(ProptestConfig::with_cases(512))]

            /// Menace F7.3 : la sérialisation est injective. Toute écriture se relit exactement
            /// depuis sa forme canonique (v1, v2 avec date de valeur comptable, v3 avec étiquette) ; deux
            /// écritures distinctes ont donc des formes canoniques distinctes.
            #[test]
            fn serialisation_injective(a in ecriture(), b in ecriture()) {
                let (ta, tb) = (a.serialiser(), b.serialiser());
                prop_assert_eq!(EcritureCanonique::lire(&ta), Ok(a.clone()));
                // Version choisie par le contenu : v3 si une étiquette, sinon v2 si une date de
                // valeur comptable, sinon v1.
                let attendu = if a.a_une_etiquette() {
                    ENTETE_V3
                } else if a.date_valeur_comptable.is_some() {
                    ENTETE_V2
                } else {
                    ENTETE_V1
                };
                let entete = format!("{attendu}\n");
                prop_assert!(ta.starts_with(&entete));
                prop_assert_eq!(a == b, ta == tb);
            }
        }
    }
}
