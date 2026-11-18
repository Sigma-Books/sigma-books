//! Écritures du brouillard et équilibre (spec §6.1, §6.2 invariants 1 et 2, §6.6 ; Fondations §2.4).
//!
//! Le cycle est porté par les types :
//! `EcritureBrouillon` (modifiable) → [`EcritureBrouillon::valider_equilibre`] →
//! [`EcritureEquilibree`] (lecture seule) → validation par le [`crate::Livre`].
//!
//! - Une [`Ligne`] porte un débit **ou** un crédit strictement positif (invariant 2) : c'est
//!   garanti à sa construction, une ligne nulle ou négative est irreprésentable.
//! - Une ligne en devise conserve la devise, le montant en devise, le cours et la contre-valeur
//!   XOF, **calculée par le moteur** et arrondie au franc (spec §6.6, principe 8).
//! - L'équilibre ∑D = ∑C est vérifié en XOF. Si **toutes** les lignes sont dans une même devise
//!   étrangère, il est aussi vérifié dans cette devise (invariant 1, décision du mainteneur du
//!   2026-11-18). Une écriture qui mêle XOF et devise, ou plusieurs devises (encaissement en
//!   EUR sur une banque en XOF), n'est équilibrée qu'en XOF : l'écart de change se constate au
//!   lettrage (L3) et à la clôture (L6), comme dans Tryton et Odoo.
//!
//! Comme dans Tryton (`account.move`, état `draft` puis `posted`), une écriture vide ou
//! déséquilibrée reste au brouillard. Contrairement à Tryton, une ligne à zéro est refusée
//! (spec §6.2, invariant 2).

use std::fmt;

use rust_decimal::Decimal;
use time::Date;

use crate::compte::NumeroCompte;
use crate::journal::CodeJournal;
use crate::montant::{Cours, Devise, ErreurMontant, Montant};
use crate::texte::{ErreurTexte, TexteCourt};

/// Longueur d'une empreinte SHA-256 écrite en hexadécimal.
const EMPREINTE_HEX: usize = 64;

/// Erreurs de construction et d'équilibre d'une écriture.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ErreurEcriture {
    /// Montant d'une ligne nul ou négatif (invariant 2).
    #[error("le montant d'une ligne doit être strictement positif : {0}")]
    MontantNonPositif(Montant),
    /// Ligne « en XOF » construite avec un montant dans une autre devise.
    #[error("ligne en XOF attendue, montant en {0}")]
    PasEnXof(Devise),
    /// Ligne « en devise » construite avec un montant en XOF.
    #[error("une ligne en devise exige une devise autre que XOF")]
    DeviseXof,
    /// Contre-valeur XOF nulle après arrondi au franc.
    #[error("contre-valeur XOF nulle après arrondi : {0} au cours {1}")]
    ContreValeurNulle(Montant, Decimal),
    /// Erreur de calcul sur un montant.
    #[error("montant : {0}")]
    Montant(#[from] ErreurMontant),
    /// Texte invalide (libellé, identifiant de tiers, motif, version de paramètres).
    #[error("texte : {0}")]
    Texte(#[from] ErreurTexte),
    /// Référence de pièce qui n'est pas une empreinte SHA-256 hexadécimale minuscule.
    #[error("référence de pièce invalide : « {0} » (empreinte SHA-256, 64 caractères 0-9 a-f)")]
    ReferencePieceInvalide(String),
    /// Justification « pièces » sans aucune pièce.
    #[error("au moins une pièce est attendue")]
    AucunePiece,
    /// Écriture sans ligne.
    #[error("écriture vide")]
    Vide,
    /// Une seule ligne : une écriture en partie double en a au moins deux.
    #[error("une écriture comporte au moins deux lignes")]
    UneSeuleLigne,
    /// Indice de ligne hors de l'écriture.
    #[error("ligne {0} inexistante")]
    LigneInexistante(usize),
    /// ∑D ≠ ∑C en XOF.
    #[error("écriture déséquilibrée en XOF : débit {debit}, crédit {credit}")]
    DesequilibreXof {
        /// Total des débits.
        debit: Decimal,
        /// Total des crédits.
        credit: Decimal,
    },
    /// ∑D ≠ ∑C dans la devise étrangère commune à toutes les lignes.
    #[error("écriture déséquilibrée en {devise} : débit {debit}, crédit {credit}")]
    DesequilibreDevise {
        /// Devise concernée.
        devise: Devise,
        /// Total des débits dans cette devise.
        debit: Decimal,
        /// Total des crédits dans cette devise.
        credit: Decimal,
    },
}

/// Refus d'une étape du cycle de l'écriture : l'écriture est rendue intacte avec le motif.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refus<T, E>(Box<(T, E)>);

impl<T, E> Refus<T, E> {
    pub(crate) fn nouveau(ecriture: T, erreur: E) -> Self {
        Self(Box::new((ecriture, erreur)))
    }

    /// Motif du refus.
    #[must_use]
    pub fn erreur(&self) -> &E {
        &self.0.1
    }

    /// Écriture refusée, intacte.
    #[must_use]
    pub fn ecriture(&self) -> &T {
        &self.0.0
    }

    /// Récupère l'écriture et le motif.
    #[must_use]
    pub fn decomposer(self) -> (T, E) {
        *self.0
    }
}

/// Sens d'une ligne.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Sens {
    /// Débit.
    Debit,
    /// Crédit.
    Credit,
}

impl Sens {
    /// Sens opposé (contre-passation).
    #[must_use]
    pub fn inverse(self) -> Self {
        match self {
            Self::Debit => Self::Credit,
            Self::Credit => Self::Debit,
        }
    }
}

/// Identifiant opaque d'un tiers porté par une ligne (spec §6.2, invariant 4). Le module Tiers
/// relève du lot L3 ; en L1, seul l'identifiant compte.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct IdTiers(TexteCourt);

impl IdTiers {
    /// Valide un identifiant de tiers (texte court).
    ///
    /// # Errors
    /// [`ErreurEcriture::Texte`] si le texte est invalide.
    pub fn parse(texte: &str) -> Result<Self, ErreurEcriture> {
        Ok(Self(TexteCourt::parse(texte)?))
    }

    /// L'identifiant.
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl fmt::Display for IdTiers {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Référence d'une pièce justificative : son empreinte SHA-256 en hexadécimal minuscule
/// (spec §6.5). Le stockage des pièces relève des lots L2 et L4.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ReferencePiece(String);

impl ReferencePiece {
    /// Valide une empreinte : 64 caractères parmi `0-9a-f`.
    ///
    /// # Errors
    /// [`ErreurEcriture::ReferencePieceInvalide`] sinon.
    pub fn parse(texte: &str) -> Result<Self, ErreurEcriture> {
        let valide = texte.len() == EMPREINTE_HEX
            && texte
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
        if valide {
            Ok(Self(texte.to_owned()))
        } else {
            Err(ErreurEcriture::ReferencePieceInvalide(texte.to_owned()))
        }
    }

    /// L'empreinte.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Justification d'une écriture (spec §2 principe 2 ; §6.2, invariant 6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Justification {
    /// Une ou plusieurs pièces, sans doublon, dans l'ordre de rattachement.
    Pieces(Vec<ReferencePiece>),
    /// Motif explicite « sans pièce » ; accepté seulement pour les journaux qui l'autorisent.
    SansPiece(TexteCourt),
}

impl Justification {
    /// Justification par pièces : au moins une, doublons retirés (ordre conservé).
    ///
    /// # Errors
    /// [`ErreurEcriture::AucunePiece`] si la liste est vide.
    pub fn pieces(
        pieces: impl IntoIterator<Item = ReferencePiece>,
    ) -> Result<Self, ErreurEcriture> {
        let mut liste: Vec<ReferencePiece> = Vec::new();
        for piece in pieces {
            if !liste.contains(&piece) {
                liste.push(piece);
            }
        }
        if liste.is_empty() {
            return Err(ErreurEcriture::AucunePiece);
        }
        Ok(Self::Pieces(liste))
    }

    /// Justification « sans pièce », avec son motif.
    #[must_use]
    pub fn sans_piece(motif: TexteCourt) -> Self {
        Self::SansPiece(motif)
    }
}

/// Version des paramètres fiscaux utilisée par l'écriture (spec §5.4) : identifiant opaque.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct VersionParametres(TexteCourt);

impl VersionParametres {
    /// Valide un identifiant de version (texte court).
    ///
    /// # Errors
    /// [`ErreurEcriture::Texte`] si le texte est invalide.
    pub fn parse(texte: &str) -> Result<Self, ErreurEcriture> {
        Ok(Self(TexteCourt::parse(texte)?))
    }

    /// L'identifiant.
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

/// Montant d'origine d'une ligne en devise étrangère (spec §5.5, §6.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MontantDevise {
    montant: Montant,
    cours: Cours,
}

impl MontantDevise {
    /// Montant en devise (strictement positif, devise autre que XOF).
    #[must_use]
    pub fn montant(&self) -> Montant {
        self.montant
    }

    /// Cours appliqué : francs CFA pour une unité de devise.
    #[must_use]
    pub fn cours(&self) -> Cours {
        self.cours
    }
}

/// Ligne d'écriture : un compte, un sens, un montant XOF strictement positif.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ligne {
    compte: NumeroCompte,
    sens: Sens,
    montant: Montant,
    devise: Option<MontantDevise>,
    tiers: Option<IdTiers>,
}

impl Ligne {
    /// Ligne en XOF.
    ///
    /// # Errors
    /// Montant qui n'est pas en XOF ou qui n'est pas strictement positif.
    pub fn en_xof(
        compte: NumeroCompte,
        sens: Sens,
        montant: Montant,
    ) -> Result<Self, ErreurEcriture> {
        if montant.devise() != Devise::XOF {
            return Err(ErreurEcriture::PasEnXof(montant.devise()));
        }
        if !montant.est_strictement_positif() {
            return Err(ErreurEcriture::MontantNonPositif(montant));
        }
        Ok(Self {
            compte,
            sens,
            montant,
            devise: None,
            tiers: None,
        })
    }

    /// Ligne en devise étrangère : la contre-valeur XOF est calculée par le moteur
    /// (`montant × cours`, arrondie au franc, spec §6.6).
    ///
    /// # Errors
    /// Devise XOF, montant non strictement positif, contre-valeur nulle ou dépassement.
    pub fn en_devise(
        compte: NumeroCompte,
        sens: Sens,
        montant: Montant,
        cours: Cours,
    ) -> Result<Self, ErreurEcriture> {
        if montant.devise() == Devise::XOF {
            return Err(ErreurEcriture::DeviseXof);
        }
        if !montant.est_strictement_positif() {
            return Err(ErreurEcriture::MontantNonPositif(montant));
        }
        let contre_valeur = montant.contre_valeur_xof(cours)?;
        if !contre_valeur.est_strictement_positif() {
            return Err(ErreurEcriture::ContreValeurNulle(montant, cours.valeur()));
        }
        Ok(Self {
            compte,
            sens,
            montant: contre_valeur,
            devise: Some(MontantDevise { montant, cours }),
            tiers: None,
        })
    }

    /// Rattache un tiers à la ligne.
    #[must_use]
    pub fn avec_tiers(mut self, tiers: IdTiers) -> Self {
        self.tiers = Some(tiers);
        self
    }

    /// Compte mouvementé.
    #[must_use]
    pub fn compte(&self) -> &NumeroCompte {
        &self.compte
    }

    /// Sens.
    #[must_use]
    pub fn sens(&self) -> Sens {
        self.sens
    }

    /// Montant en XOF (contre-valeur pour une ligne en devise), strictement positif.
    #[must_use]
    pub fn montant(&self) -> Montant {
        self.montant
    }

    /// Débit en XOF (zéro pour une ligne au crédit).
    #[must_use]
    pub fn debit(&self) -> Decimal {
        match self.sens {
            Sens::Debit => self.montant.valeur(),
            Sens::Credit => Decimal::ZERO,
        }
    }

    /// Crédit en XOF (zéro pour une ligne au débit).
    #[must_use]
    pub fn credit(&self) -> Decimal {
        match self.sens {
            Sens::Credit => self.montant.valeur(),
            Sens::Debit => Decimal::ZERO,
        }
    }

    /// Montant d'origine si la ligne est en devise étrangère.
    #[must_use]
    pub fn devise(&self) -> Option<&MontantDevise> {
        self.devise.as_ref()
    }

    /// Tiers rattaché.
    #[must_use]
    pub fn tiers(&self) -> Option<&IdTiers> {
        self.tiers.as_ref()
    }

    /// Ligne miroir : même compte, même montant, même devise et même tiers, sens inversé.
    #[must_use]
    pub(crate) fn miroir(&self) -> Self {
        Self {
            sens: self.sens.inverse(),
            ..self.clone()
        }
    }
}

/// Écriture au brouillard : modifiable et supprimable, sans numéro (spec §6.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EcritureBrouillon {
    journal: CodeJournal,
    date: Date,
    libelle: TexteCourt,
    lignes: Vec<Ligne>,
    justification: Option<Justification>,
    version_parametres: Option<VersionParametres>,
}

impl EcritureBrouillon {
    /// Nouvelle écriture vide.
    #[must_use]
    pub fn nouvelle(journal: CodeJournal, date: Date, libelle: TexteCourt) -> Self {
        Self {
            journal,
            date,
            libelle,
            lignes: Vec::new(),
            justification: None,
            version_parametres: None,
        }
    }

    /// Journal.
    #[must_use]
    pub fn journal(&self) -> &CodeJournal {
        &self.journal
    }

    /// Date comptable.
    #[must_use]
    pub fn date(&self) -> Date {
        self.date
    }

    /// Libellé, tel que saisi.
    #[must_use]
    pub fn libelle(&self) -> &TexteCourt {
        &self.libelle
    }

    /// Lignes, dans l'ordre de saisie.
    #[must_use]
    pub fn lignes(&self) -> &[Ligne] {
        &self.lignes
    }

    /// Justification, si elle est déjà renseignée.
    #[must_use]
    pub fn justification(&self) -> Option<&Justification> {
        self.justification.as_ref()
    }

    /// Change de journal.
    pub fn changer_journal(&mut self, journal: CodeJournal) {
        self.journal = journal;
    }

    /// Change de date.
    pub fn changer_date(&mut self, date: Date) {
        self.date = date;
    }

    /// Change de libellé.
    pub fn changer_libelle(&mut self, libelle: TexteCourt) {
        self.libelle = libelle;
    }

    /// Ajoute une ligne en fin d'écriture.
    pub fn ajouter_ligne(&mut self, ligne: Ligne) {
        self.lignes.push(ligne);
    }

    /// Retire la ligne d'indice `indice` et la renvoie.
    ///
    /// # Errors
    /// [`ErreurEcriture::LigneInexistante`] si l'indice est hors de l'écriture.
    pub fn retirer_ligne(&mut self, indice: usize) -> Result<Ligne, ErreurEcriture> {
        if indice < self.lignes.len() {
            Ok(self.lignes.remove(indice))
        } else {
            Err(ErreurEcriture::LigneInexistante(indice))
        }
    }

    /// Renseigne la justification.
    pub fn justifier(&mut self, justification: Justification) {
        self.justification = Some(justification);
    }

    /// Renseigne la version des paramètres fiscaux utilisée.
    pub fn utiliser_parametres(&mut self, version: VersionParametres) {
        self.version_parametres = Some(version);
    }

    /// Contrôle l'équilibre et fige l'écriture. Seule voie vers une [`EcritureEquilibree`].
    ///
    /// # Errors
    /// Écriture vide ou d'une seule ligne ; ∑D ≠ ∑C en XOF, ou dans la devise étrangère commune
    /// à toutes les lignes ; dépassement de capacité. En cas d'erreur, le brouillon est rendu intact dans `Err`.
    pub fn valider_equilibre(self) -> Result<EcritureEquilibree, Refus<Self, ErreurEcriture>> {
        match controler_equilibre(&self.lignes) {
            Ok(()) => Ok(EcritureEquilibree { brouillon: self }),
            Err(erreur) => Err(Refus::nouveau(self, erreur)),
        }
    }
}

/// Contrôle de l'invariant 1 : au moins deux lignes, ∑D = ∑C en XOF et, si toutes les lignes
/// sont dans une même devise étrangère, dans cette devise.
fn controler_equilibre(lignes: &[Ligne]) -> Result<(), ErreurEcriture> {
    match lignes.len() {
        0 => return Err(ErreurEcriture::Vide),
        1 => return Err(ErreurEcriture::UneSeuleLigne),
        _ => {}
    }
    let (mut debit, mut credit) = (Decimal::ZERO, Decimal::ZERO);
    for ligne in lignes {
        debit = ajouter(debit, ligne.debit())?;
        credit = ajouter(credit, ligne.credit())?;
    }
    if debit != credit {
        return Err(ErreurEcriture::DesequilibreXof { debit, credit });
    }
    let devise = lignes[0].devise().map(|d| d.montant().devise());
    let Some(devise) = devise else {
        return Ok(());
    };
    let (mut debit, mut credit) = (Decimal::ZERO, Decimal::ZERO);
    for ligne in lignes {
        match ligne.devise() {
            Some(origine) if origine.montant().devise() == devise => {
                let valeur = origine.montant().valeur();
                match ligne.sens() {
                    Sens::Debit => debit = ajouter(debit, valeur)?,
                    Sens::Credit => credit = ajouter(credit, valeur)?,
                }
            }
            // Écriture multidevise : équilibre en XOF seulement.
            _ => return Ok(()),
        }
    }
    if debit != credit {
        return Err(ErreurEcriture::DesequilibreDevise {
            devise,
            debit,
            credit,
        });
    }
    Ok(())
}

fn ajouter(a: Decimal, b: Decimal) -> Result<Decimal, ErreurEcriture> {
    a.checked_add(b)
        .ok_or(ErreurEcriture::Montant(ErreurMontant::Depassement))
}

/// Écriture équilibrée, en lecture seule : la seule qui puisse être présentée à la validation
/// (Fondations §2.4). Pour la modifier, il faut la remettre au brouillard.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EcritureEquilibree {
    brouillon: EcritureBrouillon,
}

impl EcritureEquilibree {
    /// Remet l'écriture au brouillard pour la modifier.
    #[must_use]
    pub fn en_brouillon(self) -> EcritureBrouillon {
        self.brouillon
    }

    /// Journal.
    #[must_use]
    pub fn journal(&self) -> &CodeJournal {
        &self.brouillon.journal
    }

    /// Date comptable.
    #[must_use]
    pub fn date(&self) -> Date {
        self.brouillon.date
    }

    /// Libellé, tel que saisi.
    #[must_use]
    pub fn libelle(&self) -> &TexteCourt {
        &self.brouillon.libelle
    }

    /// Lignes, dans l'ordre de saisie.
    #[must_use]
    pub fn lignes(&self) -> &[Ligne] {
        &self.brouillon.lignes
    }

    /// Justification.
    #[must_use]
    pub fn justification(&self) -> Option<&Justification> {
        self.brouillon.justification.as_ref()
    }

    /// Version des paramètres fiscaux.
    #[must_use]
    pub fn version_parametres(&self) -> Option<&VersionParametres> {
        self.brouillon.version_parametres.as_ref()
    }

    /// Total des débits (égal au total des crédits) en XOF.
    #[must_use]
    pub fn total(&self) -> Decimal {
        self.lignes().iter().map(Ligne::debit).sum()
    }

    pub(crate) fn decomposer(self) -> EcritureBrouillon {
        self.brouillon
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use proptest::prelude::*;
    use time::macros::date;

    fn dec(texte: &str) -> Decimal {
        texte.parse().unwrap()
    }

    pub(crate) fn n(texte: &str) -> NumeroCompte {
        NumeroCompte::parse(texte).unwrap()
    }

    pub(crate) fn xof(texte: &str) -> Montant {
        Montant::parse(texte, Devise::XOF).unwrap()
    }

    pub(crate) fn ligne(compte: &str, sens: Sens, montant: &str) -> Ligne {
        Ligne::en_xof(n(compte), sens, xof(montant)).unwrap()
    }

    pub(crate) fn brouillon(lignes: Vec<Ligne>) -> EcritureBrouillon {
        let mut e = EcritureBrouillon::nouvelle(
            CodeJournal::parse("OD").unwrap(),
            date!(2026 - 03 - 10),
            TexteCourt::parse("Écriture de test").unwrap(),
        );
        for l in lignes {
            e.ajouter_ligne(l);
        }
        e
    }

    fn eur(texte: &str) -> Montant {
        Montant::parse(texte, Devise::parse("EUR").unwrap()).unwrap()
    }

    fn cours_eur() -> Cours {
        Cours::parse("655.957").unwrap()
    }

    #[test]
    fn ligne_nulle_ou_negative_irreprésentable() {
        assert!(matches!(
            Ligne::en_xof(n("521"), Sens::Debit, xof("0")),
            Err(ErreurEcriture::MontantNonPositif(_))
        ));
        assert!(matches!(
            Ligne::en_xof(n("521"), Sens::Debit, xof("-5")),
            Err(ErreurEcriture::MontantNonPositif(_))
        ));
        assert!(matches!(
            Ligne::en_xof(n("521"), Sens::Debit, eur("5")),
            Err(ErreurEcriture::PasEnXof(_))
        ));
        assert!(matches!(
            Ligne::en_devise(n("521"), Sens::Debit, xof("5"), cours_eur()),
            Err(ErreurEcriture::DeviseXof)
        ));
        assert!(matches!(
            Ligne::en_devise(n("521"), Sens::Debit, eur("0.0001"), cours_eur()),
            Err(ErreurEcriture::ContreValeurNulle(..))
        ));
    }

    #[test]
    fn ligne_en_devise_contre_valeur_arrondie_au_franc() {
        let l = Ligne::en_devise(n("4011"), Sens::Credit, eur("100.50"), cours_eur()).unwrap();
        // 100,50 × 655,957 = 65 923,6785 → 65 924 F.
        assert_eq!(l.montant(), xof("65924"));
        assert_eq!(l.credit(), dec("65924"));
        assert_eq!(l.debit(), Decimal::ZERO);
        assert_eq!(l.devise().unwrap().montant(), eur("100.50"));
    }

    #[test]
    fn equilibre_en_xof() {
        let e = brouillon(vec![
            ligne("6052", Sens::Debit, "1000"),
            ligne("4011", Sens::Credit, "1000"),
        ])
        .valider_equilibre()
        .unwrap();
        assert_eq!(e.total(), dec("1000"));
    }

    #[test]
    fn refus_desequilibre_vide_et_ligne_unique() {
        let (b, erreur) = brouillon(vec![
            ligne("6052", Sens::Debit, "1000"),
            ligne("4011", Sens::Credit, "999"),
        ])
        .valider_equilibre()
        .unwrap_err()
        .decomposer();
        assert!(matches!(erreur, ErreurEcriture::DesequilibreXof { .. }));
        assert_eq!(b.lignes().len(), 2, "le brouillon est rendu intact");
        assert_eq!(
            brouillon(vec![])
                .valider_equilibre()
                .unwrap_err()
                .decomposer()
                .1,
            ErreurEcriture::Vide
        );
        assert_eq!(
            brouillon(vec![ligne("6052", Sens::Debit, "1")])
                .valider_equilibre()
                .unwrap_err()
                .decomposer()
                .1,
            ErreurEcriture::UneSeuleLigne
        );
    }

    #[test]
    fn equilibre_dans_la_devise_commune() {
        // 100 EUR au débit et 100 EUR au crédit : équilibré en EUR et en XOF.
        let ok = brouillon(vec![
            Ligne::en_devise(n("521"), Sens::Debit, eur("100"), cours_eur()).unwrap(),
            Ligne::en_devise(n("4011"), Sens::Credit, eur("100"), cours_eur()).unwrap(),
        ]);
        assert!(ok.valider_equilibre().is_ok());
        // Toutes les lignes en EUR, équilibrées en XOF (65 596 F de chaque côté après arrondi)
        // mais pas en EUR : refusé (§6.2, inv. 1).
        let ko = brouillon(vec![
            Ligne::en_devise(n("521"), Sens::Debit, eur("100"), cours_eur()).unwrap(),
            Ligne::en_devise(n("4011"), Sens::Credit, eur("100.0001"), cours_eur()).unwrap(),
        ]);
        assert!(matches!(
            ko.valider_equilibre().unwrap_err().decomposer().1,
            ErreurEcriture::DesequilibreDevise { .. }
        ));
    }

    #[test]
    fn ecriture_multidevise_equilibree_en_xof_seulement() {
        // Encaissement de 100 EUR sur une banque en XOF : équilibré en XOF, accepté.
        let encaissement = brouillon(vec![
            Ligne::en_devise(n("4111"), Sens::Credit, eur("100"), cours_eur()).unwrap(),
            ligne("521", Sens::Debit, "65596"),
        ]);
        assert!(encaissement.valider_equilibre().is_ok());
        // Deux devises, équilibrées en XOF : acceptée.
        let usd = Montant::parse("100", Devise::parse("USD").unwrap()).unwrap();
        let deux_devises = brouillon(vec![
            Ligne::en_devise(n("521"), Sens::Debit, eur("100"), cours_eur()).unwrap(),
            Ligne::en_devise(
                n("4011"),
                Sens::Credit,
                usd,
                Cours::parse("655.96").unwrap(),
            )
            .unwrap(),
        ]);
        assert!(deux_devises.valider_equilibre().is_ok());
        // Déséquilibre en XOF : toujours refusé.
        let ko = brouillon(vec![
            Ligne::en_devise(n("4111"), Sens::Credit, eur("100"), cours_eur()).unwrap(),
            ligne("521", Sens::Debit, "65595"),
        ]);
        assert!(matches!(
            ko.valider_equilibre().unwrap_err().decomposer().1,
            ErreurEcriture::DesequilibreXof { .. }
        ));
    }

    #[test]
    fn brouillon_modifiable_puis_remis_au_brouillard() {
        let mut b = brouillon(vec![
            ligne("6052", Sens::Debit, "1000"),
            ligne("4011", Sens::Credit, "999"),
        ]);
        b.retirer_ligne(1).unwrap();
        assert!(matches!(
            b.retirer_ligne(5),
            Err(ErreurEcriture::LigneInexistante(5))
        ));
        b.ajouter_ligne(ligne("4011", Sens::Credit, "1000"));
        b.changer_libelle(TexteCourt::parse("Corrigée").unwrap());
        let e = b.valider_equilibre().unwrap();
        let b = e.en_brouillon();
        assert_eq!(b.libelle().as_str(), "Corrigée");
    }

    #[test]
    fn libelle_conserve_exactement_meme_avec_formule() {
        // Q11 : libellés commençant par = + - @ acceptés et conservés à l'identique.
        for texte in ["=SOMME(A1:A2)", "+33 frais", "-50 % remise", "@cmd"] {
            let mut b = brouillon(vec![
                ligne("6052", Sens::Debit, "10"),
                ligne("4011", Sens::Credit, "10"),
            ]);
            b.changer_libelle(TexteCourt::parse(texte).unwrap());
            assert_eq!(b.valider_equilibre().unwrap().libelle().as_str(), texte);
        }
    }

    #[test]
    fn references_de_pieces() {
        let bonne = "a".repeat(64);
        assert!(ReferencePiece::parse(&bonne).is_ok());
        assert!(ReferencePiece::parse(&"A".repeat(64)).is_err());
        assert!(ReferencePiece::parse(&"a".repeat(63)).is_err());
        assert!(ReferencePiece::parse(&"g".repeat(64)).is_err());
        let p = ReferencePiece::parse(&bonne).unwrap();
        let j = Justification::pieces([p.clone(), p.clone()]).unwrap();
        assert_eq!(j, Justification::Pieces(vec![p]), "doublons retirés");
        assert_eq!(
            Justification::pieces(Vec::new()).unwrap_err(),
            ErreurEcriture::AucunePiece
        );
    }

    proptest! {
        /// Toute écriture construite par paires débit/crédit de même montant est équilibrée.
        #[test]
        fn paires_equilibrees(montants in proptest::collection::vec(1u64..10_000_000_000, 1..20)) {
            let mut lignes = Vec::new();
            for m in &montants {
                let montant = Montant::nouveau(Decimal::from(*m), Devise::XOF);
                lignes.push(Ligne::en_xof(n("6052"), Sens::Debit, montant).unwrap());
                lignes.push(Ligne::en_xof(n("4011"), Sens::Credit, montant).unwrap());
            }
            let e = brouillon(lignes).valider_equilibre().unwrap();
            let attendu: u64 = montants.iter().sum();
            prop_assert_eq!(e.total(), Decimal::from(attendu));
        }

        /// Un écart d'un centime suffit à refuser l'écriture.
        #[test]
        fn le_moindre_ecart_est_refuse(m in 1u64..10_000_000_000) {
            let montant = Montant::nouveau(Decimal::from(m), Devise::XOF);
            let ecart = Montant::nouveau(Decimal::from(m) + Decimal::new(1, 2), Devise::XOF);
            let b = brouillon(vec![
                Ligne::en_xof(n("6052"), Sens::Debit, montant).unwrap(),
                Ligne::en_xof(n("4011"), Sens::Credit, ecart).unwrap(),
            ]);
            prop_assert!(b.valider_equilibre().is_err());
        }
    }
}
