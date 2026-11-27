//! Profil d'import d'un relevé : une **donnée** (ADR 0034 §6), validée à la création.
//!
//! Champs repris de Tryton (`StatementImportCSV`, `statement.py:1449-1576` : encodage en liste
//! fermée, délimiteur, guillemet, lignes à sauter, format de date contrôlé, séparateurs
//! décimal et de milliers, colonnes par rang) et d'OCA
//! (`account_statement_import_sheet_mapping.py` : colonnes par titre, montant signé ou colonnes
//! de débit et de crédit distinctes, signe inversé, solde, devise, identifiant de transaction,
//! lignes d'en-tête et de pied). Forme TOML (ou toute forme `serde`) :
//!
//! ```toml
//! encodage = "windows-1252"
//! delimiteur = ";"
//! guillemet = '"'            # "" : aucun ; absent : '"' (défaut de Tryton et d'OCA)
//! lignes_entete = 1          # lignes ignorées en tête ; la dernière porte les titres
//! lignes_pied = 0
//! format_date = "%d/%m/%Y"
//! separateur_decimal = ","
//! separateur_milliers = " "  # facultatif
//! signe_inverse = false
//!
//! [colonnes]                 # rang (0 = première colonne) ou titre
//! date = "Date opération"
//! date_valeur = 1
//! libelle = "Libellé"
//! debit = "Débit"            # ou `montant = …` (montant signé)
//! credit = "Crédit"
//! ```

use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::Champ;
use super::date::{ErreurFormatDate, FormatDate};
use super::decodage::Encodage;

/// Désignation d'une colonne : par rang (0 = première) ou par titre (ligne de titres = dernière
/// ligne d'en-tête ; espaces de bord ignorées, première colonne de même titre).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Colonne {
    /// Rang de la colonne, à partir de 0 (Tryton, OCA sans en-tête).
    Rang(usize),
    /// Titre de la colonne (OCA).
    Titre(String),
}

/// Colonnes du profil, forme brute.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ColonnesBrutes {
    /// Date d'opération (obligatoire).
    pub date: Colonne,
    /// Date de valeur.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub date_valeur: Option<Colonne>,
    /// Libellé.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub libelle: Option<Colonne>,
    /// Référence.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reference: Option<Colonne>,
    /// Montant signé (exclusif de `debit` et `credit`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub montant: Option<Colonne>,
    /// Débit : sortie d'argent du compte, au sens du relevé.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub debit: Option<Colonne>,
    /// Crédit : entrée d'argent sur le compte, au sens du relevé.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credit: Option<Colonne>,
    /// Solde après l'opération.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub solde: Option<Colonne>,
    /// Devise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub devise: Option<Colonne>,
    /// Identifiant de transaction fourni par la banque ou l'opérateur.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identifiant_transaction: Option<Colonne>,
}

fn guillemet_par_defaut() -> String {
    "\"".to_owned()
}

/// Profil, forme brute (telle que stockée ou saisie), avant validation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfilBrut {
    /// Libellé d'encodage (liste fermée : [`Encodage::libelles_admis`]).
    pub encodage: String,
    /// Délimiteur (un caractère).
    pub delimiteur: String,
    /// Guillemet (un caractère), ou chaîne vide pour aucun.
    #[serde(default = "guillemet_par_defaut")]
    pub guillemet: String,
    /// Lignes ignorées en tête ; la dernière porte les titres.
    #[serde(default)]
    pub lignes_entete: usize,
    /// Lignes ignorées en fin de fichier (après les lignes vides finales).
    #[serde(default)]
    pub lignes_pied: usize,
    /// Format de date (voir [`FormatDate`]).
    pub format_date: String,
    /// Séparateur décimal (un caractère).
    pub separateur_decimal: String,
    /// Séparateur de milliers (un caractère), facultatif.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub separateur_milliers: Option<String>,
    /// Inverse le signe des montants et du solde (OCA `amount_inverse_sign`).
    #[serde(default)]
    pub signe_inverse: bool,
    /// Colonnes.
    pub colonnes: ColonnesBrutes,
}

/// Refus d'un profil.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ErreurProfil {
    /// Texte TOML ou structure illisible.
    #[error("profil illisible : {0}")]
    Syntaxe(String),
    /// Encodage hors de la liste fermée.
    #[error("encodage non admis")]
    EncodageInconnu,
    /// Délimiteur : un caractère, ni fin de ligne ni guillemet.
    #[error("délimiteur invalide")]
    Delimiteur,
    /// Guillemet : au plus un caractère, ni fin de ligne.
    #[error("guillemet invalide")]
    Guillemet,
    /// Séparateur décimal : un caractère.
    #[error("séparateur décimal invalide")]
    SeparateurDecimal,
    /// Séparateur de milliers : un caractère, distinct du séparateur décimal (OCA).
    #[error("séparateur de milliers invalide")]
    SeparateurMilliers,
    /// Format de date refusé.
    #[error(transparent)]
    FormatDate(#[from] ErreurFormatDate),
    /// Ni montant signé ni débit et crédit, ou les deux, ou débit sans crédit (OCA
    /// `_check_amount_type`).
    #[error("colonnes de montant incohérentes : montant signé, ou débit et crédit")]
    ColonnesMontant,
    /// Colonne désignée par un titre sans ligne d'en-tête.
    #[error("colonne « {0} » désignée par un titre sans ligne d'en-tête")]
    TitreSansEntete(Champ),
    /// Titre vide.
    #[error("colonne « {0} » : titre vide")]
    TitreVide(Champ),
}

/// Colonnes de montant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Montants {
    /// Montant signé : positif = entrée d'argent (ADR 0034 §5).
    Signe(Colonne),
    /// Débit (sortie) et crédit (entrée) : montant = |crédit| − |débit|, cellule vide = 0.
    DebitCredit {
        /// Colonne de débit.
        debit: Colonne,
        /// Colonne de crédit.
        credit: Colonne,
    },
}

/// Profil validé. Se construit par [`ProfilReleve::depuis_toml`] ou par `serde` (forme
/// [`ProfilBrut`], validée à la désérialisation).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "ProfilBrut", into = "ProfilBrut")]
pub struct ProfilReleve {
    pub(crate) encodage: Encodage,
    pub(crate) delimiteur: char,
    pub(crate) guillemet: Option<char>,
    pub(crate) lignes_entete: usize,
    pub(crate) lignes_pied: usize,
    pub(crate) format_date: FormatDate,
    pub(crate) separateur_decimal: char,
    pub(crate) separateur_milliers: Option<char>,
    pub(crate) signe_inverse: bool,
    pub(crate) colonnes: Vec<(Champ, Colonne)>,
    pub(crate) montants: Montants,
}

fn un_caractere(s: &str) -> Option<char> {
    let mut c = s.chars();
    let premier = c.next()?;
    c.next().is_none().then_some(premier)
}

fn fin_de_ligne(c: char) -> bool {
    c == '\r' || c == '\n'
}

impl ProfilReleve {
    /// Lit et valide un profil TOML.
    ///
    /// # Errors
    /// [`ErreurProfil::Syntaxe`] si le texte n'est pas un profil TOML ; sinon les refus de
    /// [`ProfilReleve::try_from`].
    pub fn depuis_toml(texte: &str) -> Result<Self, ErreurProfil> {
        let brut: ProfilBrut =
            toml::from_str(texte).map_err(|e| ErreurProfil::Syntaxe(e.message().to_owned()))?;
        Self::try_from(brut)
    }

    /// Encodage.
    #[must_use]
    pub fn encodage(&self) -> Encodage {
        self.encodage
    }

    /// Colonnes de montant.
    #[must_use]
    pub fn montants(&self) -> &Montants {
        &self.montants
    }

    /// Colonnes, montant exclu, dans l'ordre des champs.
    #[must_use]
    pub fn colonnes(&self) -> &[(Champ, Colonne)] {
        &self.colonnes
    }

    pub(crate) fn colonne(&self, champ: Champ) -> Option<&Colonne> {
        self.colonnes
            .iter()
            .find_map(|(c, col)| (*c == champ).then_some(col))
    }

    /// Toutes les colonnes désignées, montant compris.
    pub(crate) fn toutes_colonnes(&self) -> Vec<(Champ, &Colonne)> {
        let mut v: Vec<(Champ, &Colonne)> =
            self.colonnes.iter().map(|(c, col)| (*c, col)).collect();
        match &self.montants {
            Montants::Signe(m) => v.push((Champ::Montant, m)),
            Montants::DebitCredit { debit, credit } => {
                v.push((Champ::Debit, debit));
                v.push((Champ::Credit, credit));
            }
        }
        v
    }
}

impl TryFrom<ProfilBrut> for ProfilReleve {
    type Error = ErreurProfil;

    fn try_from(b: ProfilBrut) -> Result<Self, ErreurProfil> {
        let encodage =
            Encodage::depuis_libelle(&b.encodage).ok_or(ErreurProfil::EncodageInconnu)?;
        let guillemet = if b.guillemet.is_empty() {
            None
        } else {
            Some(
                un_caractere(&b.guillemet)
                    .filter(|&c| !fin_de_ligne(c))
                    .ok_or(ErreurProfil::Guillemet)?,
            )
        };
        let delimiteur = un_caractere(&b.delimiteur)
            .filter(|&c| !fin_de_ligne(c) && Some(c) != guillemet)
            .ok_or(ErreurProfil::Delimiteur)?;
        let separateur_decimal =
            un_caractere(&b.separateur_decimal).ok_or(ErreurProfil::SeparateurDecimal)?;
        let separateur_milliers = match b.separateur_milliers.as_deref() {
            None => None,
            Some(s) => Some(
                un_caractere(s)
                    .filter(|&c| c != separateur_decimal)
                    .ok_or(ErreurProfil::SeparateurMilliers)?,
            ),
        };
        let format_date = FormatDate::analyser(&b.format_date)?;
        let c = b.colonnes;
        let montants = match (c.montant, c.debit, c.credit) {
            (Some(m), None, None) => Montants::Signe(m),
            (None, Some(debit), Some(credit)) => Montants::DebitCredit { debit, credit },
            _ => return Err(ErreurProfil::ColonnesMontant),
        };
        let mut colonnes = vec![(Champ::Date, c.date)];
        for (champ, col) in [
            (Champ::DateValeur, c.date_valeur),
            (Champ::Libelle, c.libelle),
            (Champ::Reference, c.reference),
            (Champ::Solde, c.solde),
            (Champ::Devise, c.devise),
            (Champ::IdentifiantTransaction, c.identifiant_transaction),
        ] {
            if let Some(col) = col {
                colonnes.push((champ, col));
            }
        }
        let profil = Self {
            encodage,
            delimiteur,
            guillemet,
            lignes_entete: b.lignes_entete,
            lignes_pied: b.lignes_pied,
            format_date,
            separateur_decimal,
            separateur_milliers,
            signe_inverse: b.signe_inverse,
            colonnes,
            montants,
        };
        for (champ, col) in profil.toutes_colonnes() {
            if let Colonne::Titre(t) = col {
                if t.trim().is_empty() {
                    return Err(ErreurProfil::TitreVide(champ));
                }
                if profil.lignes_entete == 0 {
                    return Err(ErreurProfil::TitreSansEntete(champ));
                }
            }
        }
        Ok(profil)
    }
}

impl From<ProfilReleve> for ProfilBrut {
    fn from(p: ProfilReleve) -> Self {
        let (montant, debit, credit) = match p.montants.clone() {
            Montants::Signe(m) => (Some(m), None, None),
            Montants::DebitCredit { debit, credit } => (None, Some(debit), Some(credit)),
        };
        Self {
            encodage: p.encodage.libelle().to_owned(),
            delimiteur: p.delimiteur.to_string(),
            guillemet: p.guillemet.map_or_default(String::from),
            lignes_entete: p.lignes_entete,
            lignes_pied: p.lignes_pied,
            format_date: p.format_date.source().to_owned(),
            separateur_decimal: p.separateur_decimal.to_string(),
            separateur_milliers: p.separateur_milliers.map(String::from),
            signe_inverse: p.signe_inverse,
            colonnes: ColonnesBrutes {
                date: p.colonne(Champ::Date).cloned().unwrap_or(Colonne::Rang(0)),
                date_valeur: p.colonne(Champ::DateValeur).cloned(),
                libelle: p.colonne(Champ::Libelle).cloned(),
                reference: p.colonne(Champ::Reference).cloned(),
                montant,
                debit,
                credit,
                solde: p.colonne(Champ::Solde).cloned(),
                devise: p.colonne(Champ::Devise).cloned(),
                identifiant_transaction: p.colonne(Champ::IdentifiantTransaction).cloned(),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: &str = r#"
encodage = "UTF-8"
delimiteur = ";"
lignes_entete = 1
format_date = "%d/%m/%Y"
separateur_decimal = ","
separateur_milliers = " "
[colonnes]
date = "Date"
libelle = 2
montant = "Montant"
"#;

    #[test]
    fn profil_valide_et_aller_retour_toml() {
        let p = ProfilReleve::depuis_toml(BASE).unwrap();
        assert_eq!(p.encodage().libelle(), "utf-8");
        assert_eq!(p.guillemet, Some('"'));
        assert_eq!(
            p.montants(),
            &Montants::Signe(Colonne::Titre("Montant".into()))
        );
        assert_eq!(p.colonne(Champ::Libelle), Some(&Colonne::Rang(2)));
        let texte = toml::to_string(&p).unwrap();
        assert_eq!(ProfilReleve::depuis_toml(&texte).unwrap(), p);
    }

    fn refus(remplacer: &str, par: &str) -> ErreurProfil {
        ProfilReleve::depuis_toml(&BASE.replace(remplacer, par)).unwrap_err()
    }

    #[test]
    fn profils_refuses() {
        assert_eq!(refus("UTF-8", "utf-16"), ErreurProfil::EncodageInconnu);
        assert_eq!(
            refus("delimiteur = \";\"", "delimiteur = \";;\""),
            ErreurProfil::Delimiteur
        );
        assert_eq!(
            refus("delimiteur = \";\"", "delimiteur = '\"'"),
            ErreurProfil::Delimiteur
        );
        assert_eq!(
            refus("delimiteur = \";\"", "delimiteur = \"\\n\""),
            ErreurProfil::Delimiteur
        );
        assert_eq!(
            refus(
                "delimiteur = \";\"",
                "delimiteur = \";\"\nguillemet = \"''\""
            ),
            ErreurProfil::Guillemet
        );
        assert_eq!(
            refus("milliers = \" \"", "milliers = \",\""),
            ErreurProfil::SeparateurMilliers
        );
        assert_eq!(
            refus("%d/%m/%Y", "%d/%m"),
            ErreurProfil::FormatDate(ErreurFormatDate::SansAnnee)
        );
        assert_eq!(
            refus("montant = \"Montant\"", "debit = 3"),
            ErreurProfil::ColonnesMontant
        );
        assert_eq!(
            refus("montant = \"Montant\"", "montant = 3\ncredit = 4"),
            ErreurProfil::ColonnesMontant
        );
        assert_eq!(
            refus("lignes_entete = 1", ""),
            ErreurProfil::TitreSansEntete(Champ::Date)
        );
        assert_eq!(
            refus("\"Date\"", "\" \""),
            ErreurProfil::TitreVide(Champ::Date)
        );
        assert!(matches!(
            refus("libelle = 2", "libelle = -2"),
            ErreurProfil::Syntaxe(_)
        ));
        assert!(matches!(
            refus("[colonnes]", "inconnu = 1\n[colonnes]"),
            ErreurProfil::Syntaxe(_)
        ));
    }

    #[test]
    fn sans_guillemet_et_debit_credit() {
        let p = ProfilReleve::depuis_toml(
            &BASE
                .replace("lignes_entete = 1", "lignes_entete = 1\nguillemet = \"\"")
                .replace("montant = \"Montant\"", "debit = 3\ncredit = \"Crédit\""),
        )
        .unwrap();
        assert_eq!(p.guillemet, None);
        assert_eq!(
            p.montants(),
            &Montants::DebitCredit {
                debit: Colonne::Rang(3),
                credit: Colonne::Titre("Crédit".into())
            }
        );
    }
}
