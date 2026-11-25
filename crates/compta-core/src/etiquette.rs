//! Étiquette fiscale d'une ligne d'écriture (L2, question 7.2 ; ADR 0019).
//!
//! L'étiquette dit **ce que la ligne représente pour la TVA** : une *base* (ligne de produit ou
//! de charge) ou une *taxe* (ligne de TVA), collectée ou déductible, sous quel régime, à quel
//! taux, avec quelle exigibilité et, pour une opération exonérée, pour quel motif. Elle entre
//! dans le contenu haché de l'écriture : elle ne peut plus changer une fois l'écriture validée.
//!
//! **Pureté.** `compta-core` ne connaît aucun régime ni aucun motif : ce sont des paramètres
//! fiscaux, qui vivent dans `fiscal-sn` (CLAUDE.md, règle 9). Le constructeur
//! [`EtiquetteFiscale::nouvelle`] reçoit donc un [`ReferentielFiscal`] (un trait) qui décrit le
//! régime demandé ([`ProfilRegime`]) et confirme l'appartenance du motif. Les valeurs stockées
//! sont des codes opaques, **stables** : ni lien vers une configuration modifiable, ni taux
//! « courant » ; le taux est recopié sous forme décimale exacte.
//!
//! Le relecteur d'un export (`integrite`, `sigma-verifier`) n'a pas de référentiel. Il lit un
//! [`EtiquetteRelue`] : un type **distinct**, qui ne contrôle que la cohérence interne (règles
//! ci-dessous) et qui **ne peut pas être posé sur une [`Ligne`]** (`Ligne::avec_etiquette` n'accepte
//! qu'une [`EtiquetteFiscale`]). Une étiquette incohérente n'est donc construisible par aucune
//! voie qui mène à une écriture validable (règle absolue 4). Toute étiquette produite par
//! `nouvelle` se convertit en [`EtiquetteRelue`] ([`EtiquetteFiscale::en_relue`]).
//!
//! Règles de cohérence interne : une opération exonérée n'a pas de ligne de taxe, donc un motif
//! n'accompagne qu'une ligne de **base** ; le code d'un motif est préfixé par le code de son
//! régime suivi de `-` (`EXO361-7` pour le régime `EXO361`) ; un taux est un décimal exact,
//! strictement positif et au plus 100, sans zéro superflu.

use std::fmt;

use rust_decimal::Decimal;

use crate::compte::NumeroCompte;
use crate::ecriture::Ligne;

/// Longueur maximale d'un code de régime ou de motif.
pub const CODE_LONGUEUR_MAX: usize = 32;

/// Erreurs de construction d'une étiquette fiscale.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ErreurEtiquette {
    /// Code de régime ou de motif mal formé (`A-Z a-z 0-9 _ -`, 1 à 32 caractères, commençant
    /// par un caractère alphanumérique).
    #[error("code fiscal invalide : « {0} »")]
    CodeInvalide(String),
    /// Taux qui n'est pas un décimal exact, strictement positif, au plus 100, sans zéro superflu.
    #[error("taux invalide : « {0} »")]
    TauxInvalide(String),
    /// Régime absent du référentiel.
    #[error("régime inconnu : « {0} »")]
    RegimeInconnu(String),
    /// Un régime exonéré exige un motif d'exonération.
    #[error("le régime exonéré « {0} » exige un motif d'exonération")]
    MotifManquant(String),
    /// Un motif n'accompagne qu'un régime exonéré.
    #[error("le régime « {0} » n'est pas exonéré : aucun motif d'exonération")]
    MotifInterdit(String),
    /// Le motif n'est pas rattaché au régime (préfixe du code ou référentiel).
    #[error("le motif « {motif} » n'appartient pas au régime « {regime} »")]
    MotifHorsRegime {
        /// Régime de l'étiquette.
        regime: String,
        /// Motif refusé.
        motif: String,
    },
    /// Une opération exonérée n'a pas de ligne de taxe : seule la base est étiquetée.
    #[error("une ligne de taxe ne porte pas de motif d'exonération (seule la base l'est)")]
    TaxeAvecMotif,
    /// Le régime n'a pas de ligne de taxe : seule la base peut être étiquetée.
    #[error("le régime « {0} » ne porte pas de ligne de taxe")]
    LigneTaxeInterdite(String),
    /// Taux absent alors que le régime en exige un.
    #[error("le régime « {0} » exige un taux")]
    TauxManquant(String),
    /// Taux présent alors que le régime n'en comporte pas.
    #[error("le régime « {0} » ne comporte pas de taux")]
    TauxInterdit(String),
    /// Taux que le régime n'admet pas.
    #[error("taux {taux} non admis pour le régime « {regime} »")]
    TauxNonAdmis {
        /// Régime de l'étiquette.
        regime: String,
        /// Taux refusé.
        taux: TauxTva,
    },
    /// Exigibilité absente alors que le régime en exige une.
    #[error("le régime « {0} » exige une exigibilité")]
    ExigibiliteManquante(String),
    /// Exigibilité que le régime n'admet pas (ou aucune n'est admise).
    #[error("exigibilité non admise pour le régime « {0} »")]
    ExigibiliteNonAdmise(String),
}

/// Ligne de base (produit, charge) ou ligne de taxe (TVA).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum NatureEtiquette {
    /// Ligne de produit ou de charge, assiette de la taxe.
    Base,
    /// Ligne de TVA.
    Taxe,
}

impl NatureEtiquette {
    /// Code stable (format canonique).
    #[must_use]
    pub fn code(self) -> &'static str {
        match self {
            Self::Base => "base",
            Self::Taxe => "taxe",
        }
    }

    /// Relit un code stable.
    #[must_use]
    pub fn depuis_code(code: &str) -> Option<Self> {
        match code {
            "base" => Some(Self::Base),
            "taxe" => Some(Self::Taxe),
            _ => None,
        }
    }
}

/// TVA collectée (ventes) ou déductible (achats).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum SensTva {
    /// TVA due sur les ventes.
    Collectee,
    /// TVA récupérable sur les achats.
    Deductible,
}

impl SensTva {
    /// Code stable (format canonique).
    #[must_use]
    pub fn code(self) -> &'static str {
        match self {
            Self::Collectee => "collectee",
            Self::Deductible => "deductible",
        }
    }

    /// Relit un code stable.
    #[must_use]
    pub fn depuis_code(code: &str) -> Option<Self> {
        match code {
            "collectee" => Some(Self::Collectee),
            "deductible" => Some(Self::Deductible),
            _ => None,
        }
    }
}

/// Exigibilité de la taxe : au débit (fait générateur : livraison, service, facture) ou à
/// l'encaissement (CGI art. 362-5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Exigibilite {
    /// Au débit.
    Debit,
    /// À l'encaissement.
    Encaissement,
}

impl Exigibilite {
    /// Code stable (format canonique).
    #[must_use]
    pub fn code(self) -> &'static str {
        match self {
            Self::Debit => "debit",
            Self::Encaissement => "encaissement",
        }
    }

    /// Relit un code stable.
    #[must_use]
    pub fn depuis_code(code: &str) -> Option<Self> {
        match code {
            "debit" => Some(Self::Debit),
            "encaissement" => Some(Self::Encaissement),
            _ => None,
        }
    }
}

fn code_bien_forme(texte: &str) -> bool {
    let octets = texte.as_bytes();
    !octets.is_empty()
        && octets.len() <= CODE_LONGUEUR_MAX
        && octets[0].is_ascii_alphanumeric()
        && octets
            .iter()
            .all(|b| b.is_ascii_alphanumeric() || *b == b'_' || *b == b'-')
}

/// Code d'un régime de TVA (liste fermée de `fiscal-sn`), par exemple `IMP18`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CodeRegime(String);

impl CodeRegime {
    /// Valide la forme d'un code (l'appartenance à la liste est contrôlée par le référentiel).
    ///
    /// # Errors
    /// [`ErreurEtiquette::CodeInvalide`].
    pub fn parse(texte: &str) -> Result<Self, ErreurEtiquette> {
        if code_bien_forme(texte) {
            Ok(Self(texte.to_owned()))
        } else {
            Err(ErreurEtiquette::CodeInvalide(texte.to_owned()))
        }
    }

    /// Le code.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for CodeRegime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Code d'un motif d'exonération (liste fermée de `fiscal-sn`), par exemple `EXO361-7`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CodeMotif(String);

impl CodeMotif {
    /// Valide la forme d'un code (l'appartenance à la liste est contrôlée par le référentiel).
    ///
    /// # Errors
    /// [`ErreurEtiquette::CodeInvalide`].
    pub fn parse(texte: &str) -> Result<Self, ErreurEtiquette> {
        if code_bien_forme(texte) {
            Ok(Self(texte.to_owned()))
        } else {
            Err(ErreurEtiquette::CodeInvalide(texte.to_owned()))
        }
    }

    /// Le code.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Vrai si le code est de la forme `<régime>-<suffixe>` (suffixe non vide).
    fn rattache_a(&self, regime: &CodeRegime) -> bool {
        self.0
            .strip_prefix(regime.as_str())
            .and_then(|reste| reste.strip_prefix('-'))
            .is_some_and(|suffixe| !suffixe.is_empty())
    }
}

impl fmt::Display for CodeMotif {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Taux de TVA en pourcent, décimal exact (`18`, `10`, `5.5`) : strictement positif, au plus 100,
/// au plus 6 décimales, sans zéro de tête ni zéro final superflu, sans signe ni exposant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TauxTva(Decimal);

impl TauxTva {
    /// Lit un taux sous sa forme canonique.
    ///
    /// # Errors
    /// [`ErreurEtiquette::TauxInvalide`].
    pub fn parse(texte: &str) -> Result<Self, ErreurEtiquette> {
        let invalide = || ErreurEtiquette::TauxInvalide(texte.to_owned());
        let (entier, fraction) = match texte.split_once('.') {
            Some((e, f)) => (e, Some(f)),
            None => (texte, None),
        };
        let chiffres = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
        let forme = chiffres(entier)
            && entier.len() <= 3
            && (entier == "0" || !entier.starts_with('0'))
            && fraction.is_none_or(|f| chiffres(f) && f.len() <= 6 && !f.ends_with('0'));
        if !forme {
            return Err(invalide());
        }
        let valeur: Decimal = texte.parse().map_err(|_| invalide())?;
        if valeur <= Decimal::ZERO || valeur > Decimal::ONE_HUNDRED {
            return Err(invalide());
        }
        Ok(Self(valeur))
    }

    /// Valeur en pourcent.
    #[must_use]
    pub fn valeur(self) -> Decimal {
        self.0
    }
}

impl fmt::Display for TauxTva {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0.normalize())
    }
}

/// Ce que le référentiel fiscal dit d'un régime : de quoi contrôler une étiquette.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfilRegime {
    exonere: bool,
    ligne_taxe: bool,
    taux_admis: Vec<TauxTva>,
    exigibilites_admises: Vec<Exigibilite>,
}

impl ProfilRegime {
    /// Profil d'un régime. `ligne_taxe` : le régime comporte une ligne de TVA (sinon seule la
    /// base est étiquetée). `taux_admis` vide : le régime ne comporte pas de taux.
    /// `exigibilites_admises` vide : le régime n'a pas d'exigibilité propre.
    #[must_use]
    pub fn nouveau(
        exonere: bool,
        ligne_taxe: bool,
        taux_admis: Vec<TauxTva>,
        exigibilites_admises: Vec<Exigibilite>,
    ) -> Self {
        Self {
            exonere,
            ligne_taxe,
            taux_admis,
            exigibilites_admises,
        }
    }

    /// Régime d'exonération (motif obligatoire).
    #[must_use]
    pub fn est_exonere(&self) -> bool {
        self.exonere
    }

    /// Le régime comporte une ligne de taxe.
    #[must_use]
    pub fn porte_une_ligne_de_taxe(&self) -> bool {
        self.ligne_taxe
    }
}

/// Paramètres fiscaux dont `compta-core` a besoin pour contrôler une étiquette. Implémenté par
/// `fiscal-sn` ; `compta-core` n'en connaît aucune valeur.
pub trait ReferentielFiscal {
    /// Profil du régime, `None` s'il est absent de la liste fermée.
    fn profil_regime(&self, code: &CodeRegime) -> Option<ProfilRegime>;

    /// Vrai si le motif appartient à la liste fermée des motifs de ce régime.
    fn motif_admis(&self, regime: &CodeRegime, motif: &CodeMotif) -> bool;
}

/// Étiquette fiscale d'une ligne. Champs privés : seule une combinaison cohérente existe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EtiquetteFiscale {
    nature: NatureEtiquette,
    sens: SensTva,
    regime: CodeRegime,
    taux: Option<TauxTva>,
    exigibilite: Option<Exigibilite>,
    motif: Option<CodeMotif>,
}

impl EtiquetteFiscale {
    /// Construit une étiquette et la contrôle contre le référentiel : régime connu, motif
    /// obligatoire et admis pour un régime exonéré (interdit sinon), taux et exigibilité requis
    /// ou interdits selon le profil du régime.
    ///
    /// # Errors
    /// [`ErreurEtiquette`] pour la première incohérence trouvée.
    pub fn nouvelle(
        nature: NatureEtiquette,
        sens: SensTva,
        regime: CodeRegime,
        taux: Option<TauxTva>,
        exigibilite: Option<Exigibilite>,
        motif: Option<CodeMotif>,
        referentiel: &dyn ReferentielFiscal,
    ) -> Result<Self, ErreurEtiquette> {
        let profil = referentiel
            .profil_regime(&regime)
            .ok_or_else(|| ErreurEtiquette::RegimeInconnu(regime.to_string()))?;
        controle_interne(nature, &regime, motif.as_ref())?;
        if nature == NatureEtiquette::Taxe && !profil.ligne_taxe {
            return Err(ErreurEtiquette::LigneTaxeInterdite(regime.to_string()));
        }
        let etiquette = Self {
            nature,
            sens,
            regime,
            taux,
            exigibilite,
            motif,
        };
        let regime = &etiquette.regime;
        match (&etiquette.motif, profil.exonere) {
            (None, true) => return Err(ErreurEtiquette::MotifManquant(regime.to_string())),
            (Some(_), false) => return Err(ErreurEtiquette::MotifInterdit(regime.to_string())),
            (Some(motif), true) if !referentiel.motif_admis(regime, motif) => {
                return Err(ErreurEtiquette::MotifHorsRegime {
                    regime: regime.to_string(),
                    motif: motif.to_string(),
                });
            }
            _ => {}
        }
        match (etiquette.taux, profil.taux_admis.is_empty()) {
            (None, false) => return Err(ErreurEtiquette::TauxManquant(regime.to_string())),
            (Some(_), true) => return Err(ErreurEtiquette::TauxInterdit(regime.to_string())),
            (Some(taux), false) if !profil.taux_admis.contains(&taux) => {
                return Err(ErreurEtiquette::TauxNonAdmis {
                    regime: regime.to_string(),
                    taux,
                });
            }
            _ => {}
        }
        match (
            etiquette.exigibilite,
            profil.exigibilites_admises.is_empty(),
        ) {
            (None, false) => {
                return Err(ErreurEtiquette::ExigibiliteManquante(regime.to_string()));
            }
            (Some(_), true) => {
                return Err(ErreurEtiquette::ExigibiliteNonAdmise(regime.to_string()));
            }
            (Some(e), false) if !profil.exigibilites_admises.contains(&e) => {
                return Err(ErreurEtiquette::ExigibiliteNonAdmise(regime.to_string()));
            }
            _ => {}
        }
        Ok(etiquette)
    }

    /// Vue « relue » de l'étiquette, pour la sérialisation canonique.
    #[must_use]
    pub fn en_relue(&self) -> EtiquetteRelue {
        EtiquetteRelue {
            nature: self.nature,
            sens: self.sens,
            regime: self.regime.clone(),
            taux: self.taux,
            exigibilite: self.exigibilite,
            motif: self.motif.clone(),
        }
    }

    /// Base ou taxe.
    #[must_use]
    pub fn nature(&self) -> NatureEtiquette {
        self.nature
    }

    /// Collectée ou déductible.
    #[must_use]
    pub fn sens(&self) -> SensTva {
        self.sens
    }

    /// Régime.
    #[must_use]
    pub fn regime(&self) -> &CodeRegime {
        &self.regime
    }

    /// Taux en pourcent, recopié (jamais une référence à une configuration).
    #[must_use]
    pub fn taux(&self) -> Option<TauxTva> {
        self.taux
    }

    /// Exigibilité.
    #[must_use]
    pub fn exigibilite(&self) -> Option<Exigibilite> {
        self.exigibilite
    }

    /// Motif d'exonération (régime exonéré, ligne de base).
    #[must_use]
    pub fn motif(&self) -> Option<&CodeMotif> {
        self.motif.as_ref()
    }
}

/// Cohérence **interne** d'une étiquette (sans référentiel) : un motif n'accompagne qu'une ligne
/// de base, et son code est préfixé de celui du régime.
fn controle_interne(
    nature: NatureEtiquette,
    regime: &CodeRegime,
    motif: Option<&CodeMotif>,
) -> Result<(), ErreurEtiquette> {
    let Some(motif) = motif else {
        return Ok(());
    };
    if nature == NatureEtiquette::Taxe {
        return Err(ErreurEtiquette::TaxeAvecMotif);
    }
    if !motif.rattache_a(regime) {
        return Err(ErreurEtiquette::MotifHorsRegime {
            regime: regime.to_string(),
            motif: motif.to_string(),
        });
    }
    Ok(())
}

/// Étiquette **relue** d'un contenu déjà validé et haché (export, vérificateur autonome) : seule
/// la cohérence interne est contrôlée, pas l'appartenance aux listes de `fiscal-sn`.
///
/// Ce type ne peut pas être posé sur une [`Ligne`] : il ne sert qu'à lire et à écrire le format
/// canonique. Impossible de s'en servir pour fabriquer une écriture validable :
///
/// ```compile_fail
/// use compta_core::{EtiquetteRelue, Ligne};
/// fn pose(ligne: Ligne, relue: EtiquetteRelue) -> Ligne {
///     ligne.avec_etiquette(relue)
/// }
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EtiquetteRelue {
    nature: NatureEtiquette,
    sens: SensTva,
    regime: CodeRegime,
    taux: Option<TauxTva>,
    exigibilite: Option<Exigibilite>,
    motif: Option<CodeMotif>,
}

impl EtiquetteRelue {
    /// Relit une étiquette : cohérence interne seulement.
    ///
    /// # Errors
    /// [`ErreurEtiquette::TaxeAvecMotif`] ou [`ErreurEtiquette::MotifHorsRegime`] (préfixe).
    pub fn nouvelle(
        nature: NatureEtiquette,
        sens: SensTva,
        regime: CodeRegime,
        taux: Option<TauxTva>,
        exigibilite: Option<Exigibilite>,
        motif: Option<CodeMotif>,
    ) -> Result<Self, ErreurEtiquette> {
        controle_interne(nature, &regime, motif.as_ref())?;
        Ok(Self {
            nature,
            sens,
            regime,
            taux,
            exigibilite,
            motif,
        })
    }

    /// Base ou taxe.
    #[must_use]
    pub fn nature(&self) -> NatureEtiquette {
        self.nature
    }

    /// Collectée ou déductible.
    #[must_use]
    pub fn sens(&self) -> SensTva {
        self.sens
    }

    /// Régime.
    #[must_use]
    pub fn regime(&self) -> &CodeRegime {
        &self.regime
    }

    /// Taux en pourcent.
    #[must_use]
    pub fn taux(&self) -> Option<TauxTva> {
        self.taux
    }

    /// Exigibilité.
    #[must_use]
    pub fn exigibilite(&self) -> Option<Exigibilite> {
        self.exigibilite
    }

    /// Motif d'exonération.
    #[must_use]
    pub fn motif(&self) -> Option<&CodeMotif> {
        self.motif.as_ref()
    }
}

/// Totaux nets des lignes de taxe d'un ensemble de lignes : collectée = crédits − débits,
/// déductible = débits − crédits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TotauxTaxe {
    /// TVA collectée nette.
    pub collectee: Decimal,
    /// TVA déductible nette.
    pub deductible: Decimal,
}

/// Total des lignes étiquetées « taxe ».
#[must_use]
pub fn totaux_taxe(lignes: &[Ligne]) -> TotauxTaxe {
    let mut totaux = TotauxTaxe::default();
    for ligne in lignes {
        let Some(etiquette) = ligne.etiquette() else {
            continue;
        };
        if etiquette.nature() != NatureEtiquette::Taxe {
            continue;
        }
        match etiquette.sens() {
            SensTva::Collectee => totaux.collectee += ligne.credit() - ligne.debit(),
            SensTva::Deductible => totaux.deductible += ligne.debit() - ligne.credit(),
        }
    }
    totaux
}

/// Écart entre les étiquettes « taxe » et les comptes de TVA.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("écart de TVA : étiquettes {etiquettes}, comptes {comptes} ({sens:?})")]
pub struct EcartTaxe {
    /// Sens concerné.
    pub sens: SensTva,
    /// Total des lignes étiquetées « taxe ».
    pub etiquettes: Decimal,
    /// Solde des lignes sur les comptes de TVA de ce sens.
    pub comptes: Decimal,
}

/// Contrôle de cohérence étiquettes ↔ comptes : pour chaque sens, le total des lignes étiquetées
/// « taxe » égale le solde des lignes portées sur les comptes de TVA de ce sens. Les comptes de
/// TVA (443x collectée, 445x déductible) ne sont pas codés ici : `sens_du_compte` les désigne
/// (données de `syscohada`), `None` pour un compte étranger à la TVA. Les contrôles d'un
/// dossier entier (soldes cumulés) relèvent de L5.
///
/// # Errors
/// [`EcartTaxe`] pour le premier sens en écart.
pub fn controler_taxes_et_comptes(
    lignes: &[Ligne],
    sens_du_compte: impl Fn(&NumeroCompte) -> Option<SensTva>,
) -> Result<(), EcartTaxe> {
    let etiquettes = totaux_taxe(lignes);
    let mut comptes = TotauxTaxe::default();
    for ligne in lignes {
        match sens_du_compte(ligne.compte()) {
            Some(SensTva::Collectee) => comptes.collectee += ligne.credit() - ligne.debit(),
            Some(SensTva::Deductible) => comptes.deductible += ligne.debit() - ligne.credit(),
            None => {}
        }
    }
    for (sens, e, c) in [
        (SensTva::Collectee, etiquettes.collectee, comptes.collectee),
        (
            SensTva::Deductible,
            etiquettes.deductible,
            comptes.deductible,
        ),
    ] {
        if e != c {
            return Err(EcartTaxe {
                sens,
                etiquettes: e,
                comptes: c,
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Référentiel de test (valeurs arbitraires, sans valeur normative).
    struct Fictif;

    fn t(texte: &str) -> TauxTva {
        TauxTva::parse(texte).unwrap()
    }

    impl ReferentielFiscal for Fictif {
        fn profil_regime(&self, code: &CodeRegime) -> Option<ProfilRegime> {
            match code.as_str() {
                "IMP" => Some(ProfilRegime::nouveau(
                    false,
                    true,
                    vec![t("18"), t("10")],
                    vec![Exigibilite::Debit, Exigibilite::Encaissement],
                )),
                "EXO" => Some(ProfilRegime::nouveau(true, false, vec![], vec![])),
                "HC" => Some(ProfilRegime::nouveau(false, false, vec![], vec![])),
                _ => None,
            }
        }

        fn motif_admis(&self, regime: &CodeRegime, motif: &CodeMotif) -> bool {
            regime.as_str() == "EXO" && matches!(motif.as_str(), "EXO-1" | "EXO-a")
        }
    }

    fn regime(c: &str) -> CodeRegime {
        CodeRegime::parse(c).unwrap()
    }

    fn motif(c: &str) -> CodeMotif {
        CodeMotif::parse(c).unwrap()
    }

    #[allow(clippy::too_many_arguments)]
    fn faire(
        nature: NatureEtiquette,
        r: &str,
        taux: Option<&str>,
        exigibilite: Option<Exigibilite>,
        m: Option<&str>,
    ) -> Result<EtiquetteFiscale, ErreurEtiquette> {
        EtiquetteFiscale::nouvelle(
            nature,
            SensTva::Collectee,
            regime(r),
            taux.map(t),
            exigibilite,
            m.map(motif),
            &Fictif,
        )
    }

    #[test]
    fn combinaisons_coherentes() {
        let e = faire(
            NatureEtiquette::Taxe,
            "IMP",
            Some("18"),
            Some(Exigibilite::Debit),
            None,
        )
        .unwrap();
        assert_eq!(e.taux().map(TauxTva::valeur), Some(Decimal::from(18)));
        assert!(faire(NatureEtiquette::Base, "EXO", None, None, Some("EXO-1")).is_ok());
        assert!(faire(NatureEtiquette::Base, "HC", None, None, None).is_ok());
    }

    #[test]
    fn combinaisons_incoherentes() {
        use ErreurEtiquette as E;
        let debit = Some(Exigibilite::Debit);
        assert!(matches!(
            faire(NatureEtiquette::Base, "XX", None, None, None),
            Err(E::RegimeInconnu(_))
        ));
        // Régime exonéré sans motif ; motif hors liste ; motif d'un autre régime ; taux.
        assert!(matches!(
            faire(NatureEtiquette::Base, "EXO", None, None, None),
            Err(E::MotifManquant(_))
        ));
        assert!(matches!(
            faire(NatureEtiquette::Base, "EXO", None, None, Some("EXO-9")),
            Err(E::MotifHorsRegime { .. })
        ));
        assert!(matches!(
            faire(NatureEtiquette::Base, "EXO", None, None, Some("IMP-1")),
            Err(E::MotifHorsRegime { .. })
        ));
        assert!(matches!(
            faire(
                NatureEtiquette::Base,
                "EXO",
                Some("18"),
                None,
                Some("EXO-1")
            ),
            Err(E::TauxInterdit(_))
        ));
        // Une ligne de taxe d'une opération exonérée n'existe pas.
        assert_eq!(
            faire(NatureEtiquette::Taxe, "EXO", None, None, Some("EXO-1")),
            Err(E::TaxeAvecMotif)
        );
        // Motif sur un régime imposable ; taux manquant ou non admis ; exigibilité.
        assert!(matches!(
            faire(
                NatureEtiquette::Base,
                "IMP",
                Some("18"),
                debit,
                Some("IMP-1")
            ),
            Err(E::MotifInterdit(_))
        ));
        assert!(matches!(
            faire(NatureEtiquette::Base, "IMP", None, debit, None),
            Err(E::TauxManquant(_))
        ));
        assert!(matches!(
            faire(NatureEtiquette::Base, "IMP", Some("20"), debit, None),
            Err(E::TauxNonAdmis { .. })
        ));
        assert!(matches!(
            faire(NatureEtiquette::Base, "IMP", Some("18"), None, None),
            Err(E::ExigibiliteManquante(_))
        ));
        assert!(matches!(
            faire(NatureEtiquette::Base, "HC", None, debit, None),
            Err(E::ExigibiliteNonAdmise(_))
        ));
        // Un régime sans ligne de taxe n'admet que la base.
        assert!(matches!(
            faire(NatureEtiquette::Taxe, "HC", None, None, None),
            Err(E::LigneTaxeInterdite(_))
        ));
    }

    #[test]
    fn taux_canoniques() {
        for ok in ["18", "10", "5.5", "0.5", "100", "0.000001"] {
            assert_eq!(TauxTva::parse(ok).unwrap().to_string(), ok);
        }
        for ko in [
            "",
            "0",
            "0.0",
            "018",
            "18.0",
            "18.",
            ".5",
            "-1",
            "+18",
            "1e1",
            "101",
            "100.5",
            "1.1234567",
            "18 ",
            "１８",
            "1_8",
            "NaN",
        ] {
            assert!(TauxTva::parse(ko).is_err(), "{ko}");
        }
    }

    #[test]
    fn codes() {
        for ok in ["IMP18", "EXO361-7", "EXO380-a", "A"] {
            assert!(CodeRegime::parse(ok).is_ok(), "{ok}");
        }
        for ko in ["", "-A", "_A", "A B", "é", "A=B", &"A".repeat(33), "A\n"] {
            assert!(CodeRegime::parse(ko).is_err(), "{ko:?}");
        }
    }

    #[test]
    fn relire_controle_la_forme_seulement() {
        let r = || regime("EXO361");
        assert!(
            EtiquetteRelue::nouvelle(
                NatureEtiquette::Base,
                SensTva::Collectee,
                r(),
                None,
                None,
                Some(motif("EXO361-7"))
            )
            .is_ok()
        );
        for m in ["EXO361", "EXO361-", "EXO380-a", "EXO3617"] {
            assert!(
                EtiquetteRelue::nouvelle(
                    NatureEtiquette::Base,
                    SensTva::Collectee,
                    r(),
                    None,
                    None,
                    Some(motif(m))
                )
                .is_err(),
                "{m}"
            );
        }
    }
}
