//! Relevés bancaires et mobile money au format CSV (ADR 0034 §4 à §7 ; spec §9).
//!
//! [`analyser`] lit un fichier reçu (`&[u8]`) avec un [`ProfilReleve`] et des [`Limites`]
//! fournies par l'appelant ; c'est l'**aperçu sans écriture** de l'import : la fonction est
//! pure (aucune I/O, aucune horloge) et ne fait que lire. Étapes :
//! 1. taille du fichier contrôlée avant toute lecture ; empreinte SHA-256 du fichier ;
//! 2. reconnaissance du type « texte » et décodage ligne par ligne ([`decoder`]) ;
//! 3. découpage CSV, limites de lignes, de colonnes et de cellule appliquées pendant la lecture ;
//! 4. lignes vides finales retirées, puis `lignes_entete` lignes en tête et `lignes_pied` en
//!    fin ignorées ; lignes entièrement vides ignorées (OCA `skip_empty_lines`) ;
//! 5. chaque ligne restante devient un [`Mouvement`] lu par le profil ; la ligne brute est
//!    conservée telle quelle ; un libellé à formule est importé tel quel avec une alerte.
//!
//! La première erreur arrête l'analyse et porte le rang de la ligne fautive (comme Tryton et
//! OCA, qui lèvent une exception sur la ligne).

mod date;
mod decodage;
mod ecriture;
mod empreinte;
mod erreur;
mod lecteur;
mod nombre;
mod profil;
#[cfg(test)]
mod tests;

use std::fmt;

use rust_decimal::Decimal;
use time::Date;

pub use date::{ErreurFormatDate, FormatDate};
pub use decodage::{Encodage, decoder};
pub use ecriture::{ErreurEcriture, serialiser};
pub use empreinte::{
    Empreinte, EmpreinteRangee, empreinte_fichier, empreinte_ligne, empreintes_rangees,
    normaliser_libelle,
};
pub use erreur::ErreurReleve;
pub use profil::{Colonne, ColonnesBrutes, ErreurProfil, Montants, ProfilBrut, ProfilReleve};

use lecteur::Enregistrement;

/// Limites de l'analyse, fournies par l'appelant (paramètres de l'instance lus par `api`,
/// jamais codés ici, ADR 0034 §6). Valeurs initiales de l'ADR : 5 Mio, 20 000 lignes,
/// 50 colonnes, 64 Kio par ligne, 1 000 caractères par cellule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limites {
    /// Taille maximale du fichier, en octets.
    pub taille_fichier: usize,
    /// Nombre maximal d'enregistrements CSV (en-tête, pied et lignes vides compris).
    pub lignes: usize,
    /// Nombre maximal de colonnes d'un enregistrement.
    pub colonnes: usize,
    /// Longueur maximale d'une ligne physique, en octets du fichier.
    pub longueur_ligne: usize,
    /// Longueur maximale d'une cellule, en caractères.
    pub longueur_cellule: usize,
}

/// Champ d'un profil.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Champ {
    /// Date d'opération.
    Date,
    /// Date de valeur.
    DateValeur,
    /// Libellé.
    Libelle,
    /// Référence.
    Reference,
    /// Montant signé.
    Montant,
    /// Débit.
    Debit,
    /// Crédit.
    Credit,
    /// Solde.
    Solde,
    /// Devise.
    Devise,
    /// Identifiant de transaction.
    IdentifiantTransaction,
}

impl Champ {
    /// Nom stable du champ (celui du profil).
    #[must_use]
    pub fn nom(self) -> &'static str {
        match self {
            Self::Date => "date",
            Self::DateValeur => "date_valeur",
            Self::Libelle => "libelle",
            Self::Reference => "reference",
            Self::Montant => "montant",
            Self::Debit => "debit",
            Self::Credit => "credit",
            Self::Solde => "solde",
            Self::Devise => "devise",
            Self::IdentifiantTransaction => "identifiant_transaction",
        }
    }
}

impl fmt::Display for Champ {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.nom())
    }
}

/// Mouvement lu sur une ligne de relevé. Montant positif = entrée d'argent (ADR 0034 §5).
/// Les textes sont tels que dans le fichier (aucune normalisation) ; une cellule vide d'un
/// champ facultatif donne `None`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mouvement {
    /// Date d'opération.
    pub date: Date,
    /// Date de valeur.
    pub date_valeur: Option<Date>,
    /// Libellé (vide si le profil n'a pas de colonne de libellé).
    pub libelle: String,
    /// Référence.
    pub reference: Option<String>,
    /// Montant, en décimal exact, à l'échelle lue.
    pub montant: Decimal,
    /// Solde après l'opération.
    pub solde: Option<Decimal>,
    /// Devise, telle que lue.
    pub devise: Option<String>,
    /// Identifiant de transaction.
    pub identifiant_transaction: Option<String>,
}

/// Ligne de relevé : ligne physique, texte brut conservé et mouvement lu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LigneReleve {
    ligne: usize,
    brut: String,
    mouvement: Mouvement,
}

impl LigneReleve {
    /// Ligne physique du fichier où commence l'enregistrement (à partir de 1).
    #[must_use]
    pub fn ligne(&self) -> usize {
        self.ligne
    }

    /// Texte brut de l'enregistrement, décodé, sans la fin de ligne.
    #[must_use]
    pub fn brut(&self) -> &str {
        &self.brut
    }

    /// Mouvement lu.
    #[must_use]
    pub fn mouvement(&self) -> &Mouvement {
        &self.mouvement
    }
}

/// Nature d'une alerte d'analyse.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NatureAlerte {
    /// Libellé commençant par `=`, `+`, `-`, `@`, une tabulation ou un retour chariot :
    /// importé tel quel, jamais exécuté (ADR 0034 §6, F7.6, F11.8).
    LibelleFormule,
}

impl NatureAlerte {
    /// Code stable de l'alerte.
    #[must_use]
    pub fn code(self) -> &'static str {
        match self {
            Self::LibelleFormule => "libelle_formule",
        }
    }
}

/// Alerte sur une ligne, sans blocage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Alerte {
    /// Ligne physique.
    pub ligne: usize,
    /// Nature.
    pub nature: NatureAlerte,
}

/// Relevé analysé.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Releve {
    empreinte: Empreinte,
    lignes: Vec<LigneReleve>,
    alertes: Vec<Alerte>,
}

impl Releve {
    /// SHA-256 du fichier tel que reçu.
    #[must_use]
    pub fn empreinte(&self) -> Empreinte {
        self.empreinte
    }

    /// Lignes, dans l'ordre du fichier.
    #[must_use]
    pub fn lignes(&self) -> &[LigneReleve] {
        &self.lignes
    }

    /// Alertes, dans l'ordre du fichier.
    #[must_use]
    pub fn alertes(&self) -> &[Alerte] {
        &self.alertes
    }

    /// Mouvements, dans l'ordre du fichier.
    pub fn mouvements(&self) -> impl Iterator<Item = &Mouvement> {
        self.lignes.iter().map(|l| &l.mouvement)
    }

    /// Empreintes de ligne et rangs d'occurrence pour le compte de trésorerie `compte`.
    #[must_use]
    pub fn empreintes_rangees(&self, compte: &str) -> Vec<EmpreinteRangee> {
        empreintes_rangees(compte, self.mouvements())
    }
}

/// Rangs des colonnes, résolus pour un fichier.
struct Rangs {
    champs: Vec<(Champ, usize)>,
}

impl Rangs {
    fn rang(&self, champ: Champ) -> Option<usize> {
        self.champs
            .iter()
            .find_map(|(c, r)| (*c == champ).then_some(*r))
    }
}

fn resoudre(profil: &ProfilReleve, titres: Option<&Enregistrement>) -> Result<Rangs, ErreurReleve> {
    let mut champs = Vec::new();
    for (champ, col) in profil.toutes_colonnes() {
        let rang = match col {
            Colonne::Rang(r) => *r,
            Colonne::Titre(t) => {
                let titres = titres.ok_or(ErreurReleve::TitresAbsents)?;
                titres
                    .cellules
                    .iter()
                    .position(|c| c.trim() == t.trim())
                    .ok_or(ErreurReleve::TitreIntrouvable {
                        ligne: titres.ligne,
                        champ,
                    })?
            }
        };
        champs.push((champ, rang));
    }
    Ok(Rangs { champs })
}

/// Lecture d'une cellule numérique vide.
#[derive(Clone, Copy)]
enum Vide {
    /// Vaut zéro (colonnes de débit et de crédit).
    Zero,
    /// Valeur absente (solde).
    Absent,
    /// Refusée (montant signé).
    Refuse,
}

struct Ligne<'a> {
    enregistrement: &'a Enregistrement,
    rangs: &'a Rangs,
    profil: &'a ProfilReleve,
}

impl Ligne<'_> {
    /// Cellule du champ ; `None` si le profil n'a pas ce champ. Une colonne désignée mais
    /// absente de la ligne est une erreur (Tryton : `row[colonne]`).
    fn cellule(&self, champ: Champ) -> Result<Option<&str>, ErreurReleve> {
        let Some(rang) = self.rangs.rang(champ) else {
            return Ok(None);
        };
        self.enregistrement
            .cellules
            .get(rang)
            .map(|c| Some(c.as_str()))
            .ok_or(ErreurReleve::ColonneAbsente {
                ligne: self.enregistrement.ligne,
                champ,
            })
    }

    fn texte(&self, champ: Champ) -> Result<Option<String>, ErreurReleve> {
        Ok(self
            .cellule(champ)?
            .filter(|c| !c.is_empty())
            .map(str::to_owned))
    }

    fn date(&self, champ: Champ) -> Result<Option<Date>, ErreurReleve> {
        let Some(c) = self.cellule(champ)?.map(str::trim) else {
            return Ok(None);
        };
        if c.is_empty() && champ != Champ::Date {
            return Ok(None);
        }
        self.profil
            .format_date
            .lire(c)
            .map(Some)
            .ok_or(ErreurReleve::DateInvalide {
                ligne: self.enregistrement.ligne,
                champ,
            })
    }

    /// Nombre du champ ; `None` si le profil n'a pas ce champ ; cellule vide selon `vide`.
    fn nombre(&self, champ: Champ, vide: Vide) -> Result<Option<Decimal>, ErreurReleve> {
        let Some(c) = self.cellule(champ)? else {
            return Ok(None);
        };
        if c.trim().is_empty() {
            match vide {
                Vide::Zero => return Ok(Some(Decimal::ZERO)),
                Vide::Absent => return Ok(None),
                Vide::Refuse => {}
            }
        }
        nombre::lire(
            c,
            self.profil.separateur_decimal,
            self.profil.separateur_milliers,
        )
        .map(Some)
        .ok_or(ErreurReleve::NombreInvalide {
            ligne: self.enregistrement.ligne,
            champ,
        })
    }

    fn mouvement(&self) -> Result<Mouvement, ErreurReleve> {
        let ligne = self.enregistrement.ligne;
        let invalide = |champ| ErreurReleve::NombreInvalide { ligne, champ };
        let montant = match self.profil.montants {
            // Montant signé vide : refusé, comme Tryton (`Decimal('')`) ; OCA le lit comme 0.
            Montants::Signe(_) => self.nombre(Champ::Montant, Vide::Refuse)?,
            Montants::DebitCredit { .. } => {
                let debit = self.nombre(Champ::Debit, Vide::Zero)?;
                let credit = self.nombre(Champ::Credit, Vide::Zero)?;
                let (debit, credit) = (debit.unwrap_or_default(), credit.unwrap_or_default());
                Some(
                    credit
                        .abs()
                        .checked_sub(debit.abs())
                        .ok_or(invalide(Champ::Montant))?,
                )
            }
        }
        .ok_or(invalide(Champ::Montant))?;
        let solde = self.nombre(Champ::Solde, Vide::Absent)?;
        let date = self.date(Champ::Date)?.ok_or(ErreurReleve::DateInvalide {
            ligne,
            champ: Champ::Date,
        })?;
        let (montant, solde) = if self.profil.signe_inverse {
            (-montant, solde.map(|s| -s))
        } else {
            (montant, solde)
        };
        Ok(Mouvement {
            date,
            date_valeur: self.date(Champ::DateValeur)?,
            libelle: self.cellule(Champ::Libelle)?.unwrap_or_default().to_owned(),
            reference: self.texte(Champ::Reference)?,
            montant,
            solde,
            devise: self.texte(Champ::Devise)?,
            identifiant_transaction: self.texte(Champ::IdentifiantTransaction)?,
        })
    }
}

fn libelle_formule(libelle: &str) -> bool {
    libelle.starts_with(['=', '+', '-', '@', '\t', '\r'])
}

/// Analyse un relevé (aperçu sans écriture). Voir le module.
///
/// # Errors
/// Première erreur rencontrée, avec le rang de la ligne fautive : voir [`ErreurReleve`].
pub fn analyser(
    octets: &[u8],
    profil: &ProfilReleve,
    limites: &Limites,
) -> Result<Releve, ErreurReleve> {
    let texte = decoder(octets, profil.encodage, limites)?;
    let mut enregistrements = lecteur::lire(&texte, profil.delimiteur, profil.guillemet, limites)?;
    drop(texte);
    while enregistrements.last().is_some_and(Enregistrement::est_vide) {
        enregistrements.pop();
    }
    let entete = profil.lignes_entete.min(enregistrements.len());
    let titres = profil
        .lignes_entete
        .checked_sub(1)
        .and_then(|i| enregistrements.get(i));
    let rangs = resoudre(profil, titres)?;
    let fin = enregistrements
        .len()
        .saturating_sub(profil.lignes_pied)
        .max(entete);
    let mut lignes = Vec::new();
    let mut alertes = Vec::new();
    for e in enregistrements.into_iter().take(fin).skip(entete) {
        if e.est_vide() {
            continue;
        }
        let mouvement = Ligne {
            enregistrement: &e,
            rangs: &rangs,
            profil,
        }
        .mouvement()?;
        if libelle_formule(&mouvement.libelle) {
            alertes.push(Alerte {
                ligne: e.ligne,
                nature: NatureAlerte::LibelleFormule,
            });
        }
        lignes.push(LigneReleve {
            ligne: e.ligne,
            brut: e.brut,
            mouvement,
        });
    }
    Ok(Releve {
        empreinte: empreinte_fichier(octets),
        lignes,
        alertes,
    })
}
