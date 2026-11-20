//! Régimes de TVA et motifs d'exonération (CGI du Sénégal consolidé, janvier 2025).

use std::collections::BTreeSet;

use compta_core::{
    CodeMotif, CodeRegime, ErreurEtiquette, Exigibilite, ProfilRegime, ReferentielFiscal, TauxTva,
};
use serde::Deserialize;

const REGIMES_TOML: &str = include_str!("../data/regimes-tva.toml");
const MOTIFS_TOML: &str = include_str!("../data/motifs-exoneration.toml");

/// Pages PDF du chapitre TVA dans le CGI consolidé (art. 351 à 390 et suivants).
const PAGE_MIN: u16 = 157;
const PAGE_MAX: u16 = 210;

/// Erreurs de chargement ou de validation des données.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ErreurFiscale {
    /// Fichier TOML illisible ou non conforme.
    #[error("données illisibles : {0}")]
    Syntaxe(String),
    /// Code mal formé.
    #[error("code invalide : {0}")]
    Code(ErreurEtiquette),
    /// Taux qui n'est pas un décimal exact canonique.
    #[error("taux invalide pour {code} : {erreur}")]
    Taux {
        /// Entrée concernée.
        code: String,
        /// Cause.
        erreur: ErreurEtiquette,
    },
    /// Code en double.
    #[error("code en double : {0}")]
    CodeEnDouble(String),
    /// Même article et même alinéa pour deux entrées.
    #[error("article et alinéa en double : {0}")]
    ArticleEnDouble(String),
    /// Entrée sans source : article vide ou aucune page.
    #[error("entrée {0} sans source (article et page)")]
    SansSource(String),
    /// Page hors du chapitre TVA du CGI.
    #[error("entrée {code} : page {page} hors de {PAGE_MIN} à {PAGE_MAX}")]
    Page {
        /// Entrée concernée.
        code: String,
        /// Page fautive.
        page: u16,
    },
    /// Libellé vide.
    #[error("entrée {0} sans libellé")]
    SansLibelle(String),
    /// Incohérence entre champs (exonéré avec taux, motif d'un régime non exonéré…).
    #[error("entrée {code} incohérente : {motif}")]
    Incoherente {
        /// Entrée concernée.
        code: String,
        /// Description.
        motif: &'static str,
    },
    /// Motif rattaché à un régime absent de la liste.
    #[error("motif {motif} : régime {regime} absent de la liste")]
    RegimeAbsent {
        /// Motif.
        motif: String,
        /// Régime cité.
        regime: String,
    },
    /// Chevauchement vers un motif absent.
    #[error("chevauchement {0} : motif cible absent")]
    CibleAbsente(String),
}

/// Droit à déduction de la taxe d'amont.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Deduction {
    /// Oui.
    Oui,
    /// Non.
    Non,
    /// Non, sauf pour les alinéas qui chevauchent l'art. 380.
    NonSaufChevauchement,
    /// Oui si la taxe a été déclarée (précompte).
    OuiSiDeclaree,
    /// Sans objet.
    SansObjet,
    /// Le CGI consolidé ne le dit pas.
    NonTrouve,
}

/// Règle d'exigibilité du CGI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExigibiliteCgi {
    /// Fait générateur : livraison, service, facture (art. 362-1 à 4).
    FaitGenerateur,
    /// Encaissement.
    Encaissement,
    /// Règle générale (art. 362).
    General,
    /// Déclaration trimestrielle (art. 363-4).
    Trimestrielle,
    /// Mois suivant l'événement (art. 387-1).
    MoisSuivant,
    /// Sans objet.
    SansObjet,
    /// Non trouvée dans le CGI consolidé.
    NonTrouve,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FichierRegimes {
    regime: Vec<RegimeBrut>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(clippy::struct_excessive_bools)]
struct RegimeBrut {
    code: String,
    libelle: String,
    article: String,
    alinea: Option<String>,
    pages: Vec<u16>,
    exonere: bool,
    deduction: Deduction,
    exigibilite: ExigibiliteCgi,
    source_deduction: String,
    source_exigibilite: String,
    taux: Vec<String>,
    exigibilites_etiquette: Vec<String>,
    a_valider: bool,
    note: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FichierMotifs {
    motif: Vec<MotifBrut>,
    chevauchement: Vec<ChevauchementBrut>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MotifBrut {
    code: String,
    regime: String,
    article: String,
    alinea: String,
    pages: Vec<u16>,
    libelle: String,
    deduction: Deduction,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ChevauchementBrut {
    article: String,
    alinea: String,
    pages: Vec<u16>,
    motif: String,
    note: String,
}

/// Régime de TVA de la liste fermée.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegimeTva {
    code: CodeRegime,
    libelle: String,
    article: String,
    alinea: Option<String>,
    pages: Vec<u16>,
    exonere: bool,
    deduction: Deduction,
    exigibilite: ExigibiliteCgi,
    source_deduction: String,
    source_exigibilite: String,
    taux: Vec<TauxTva>,
    exigibilites_etiquette: Vec<Exigibilite>,
    a_valider: bool,
    note: Option<String>,
}

impl RegimeTva {
    /// Code stable.
    #[must_use]
    pub fn code(&self) -> &CodeRegime {
        &self.code
    }
    /// Intitulé court.
    #[must_use]
    pub fn libelle(&self) -> &str {
        &self.libelle
    }
    /// Article du CGI.
    #[must_use]
    pub fn article(&self) -> &str {
        &self.article
    }
    /// Alinéa, s'il y a lieu.
    #[must_use]
    pub fn alinea(&self) -> Option<&str> {
        self.alinea.as_deref()
    }
    /// Pages PDF de la source.
    #[must_use]
    pub fn pages(&self) -> &[u16] {
        &self.pages
    }
    /// Régime d'exonération.
    #[must_use]
    pub fn est_exonere(&self) -> bool {
        self.exonere
    }
    /// Droit à déduction.
    #[must_use]
    pub fn deduction(&self) -> Deduction {
        self.deduction
    }
    /// Exigibilité selon le CGI.
    #[must_use]
    pub fn exigibilite(&self) -> ExigibiliteCgi {
        self.exigibilite
    }
    /// Source du droit à déduction.
    #[must_use]
    pub fn source_deduction(&self) -> &str {
        &self.source_deduction
    }
    /// Source de l'exigibilité.
    #[must_use]
    pub fn source_exigibilite(&self) -> &str {
        &self.source_exigibilite
    }
    /// Taux admis (pourcent).
    #[must_use]
    pub fn taux(&self) -> &[TauxTva] {
        &self.taux
    }
    /// Exigibilités que porte l'étiquette.
    #[must_use]
    pub fn exigibilites_etiquette(&self) -> &[Exigibilite] {
        &self.exigibilites_etiquette
    }
    /// À faire valider par l'expert-comptable et le fiscaliste.
    #[must_use]
    pub fn a_valider(&self) -> bool {
        self.a_valider
    }
    /// Précision.
    #[must_use]
    pub fn note(&self) -> Option<&str> {
        self.note.as_deref()
    }
}

/// Motif d'exonération de la liste fermée.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MotifExoneration {
    code: CodeMotif,
    regime: CodeRegime,
    article: String,
    alinea: String,
    pages: Vec<u16>,
    libelle: String,
    deduction: Deduction,
}

impl MotifExoneration {
    /// Code stable.
    #[must_use]
    pub fn code(&self) -> &CodeMotif {
        &self.code
    }
    /// Régime (EXO361 ou EXO380).
    #[must_use]
    pub fn regime(&self) -> &CodeRegime {
        &self.regime
    }
    /// Article du CGI.
    #[must_use]
    pub fn article(&self) -> &str {
        &self.article
    }
    /// Alinéa.
    #[must_use]
    pub fn alinea(&self) -> &str {
        &self.alinea
    }
    /// Pages PDF de la source.
    #[must_use]
    pub fn pages(&self) -> &[u16] {
        &self.pages
    }
    /// Intitulé court.
    #[must_use]
    pub fn libelle(&self) -> &str {
        &self.libelle
    }
    /// Droit à déduction.
    #[must_use]
    pub fn deduction(&self) -> Deduction {
        self.deduction
    }
}

/// Alinéa de l'art. 361 qui n'a pas de code propre : il est aussi à l'art. 380.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chevauchement {
    /// Article (361).
    pub article: String,
    /// Alinéa (14, 15 ou 26).
    pub alinea: String,
    /// Pages PDF.
    pub pages: Vec<u16>,
    /// Code retenu (art. 380).
    pub motif: CodeMotif,
    /// Précision.
    pub note: String,
}

/// Régimes, motifs et chevauchements, validés.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReferentielTva {
    regimes: Vec<RegimeTva>,
    motifs: Vec<MotifExoneration>,
    chevauchements: Vec<Chevauchement>,
}

fn controler_source(code: &str, article: &str, pages: &[u16]) -> Result<(), ErreurFiscale> {
    if article.trim().is_empty() || pages.is_empty() {
        return Err(ErreurFiscale::SansSource(code.to_owned()));
    }
    match pages.iter().find(|p| !(PAGE_MIN..=PAGE_MAX).contains(p)) {
        Some(page) => Err(ErreurFiscale::Page {
            code: code.to_owned(),
            page: *page,
        }),
        None => Ok(()),
    }
}

/// Codes et couples (article, alinéa) déjà vus.
#[derive(Default)]
struct Unicite {
    codes: BTreeSet<String>,
    articles: BTreeSet<(String, Option<String>)>,
}

impl Unicite {
    fn verifier(
        &mut self,
        code: &str,
        article: &str,
        alinea: Option<&str>,
    ) -> Result<(), ErreurFiscale> {
        if !self.codes.insert(code.to_owned()) {
            return Err(ErreurFiscale::CodeEnDouble(code.to_owned()));
        }
        if !self
            .articles
            .insert((article.to_owned(), alinea.map(str::to_owned)))
        {
            return Err(ErreurFiscale::ArticleEnDouble(code.to_owned()));
        }
        Ok(())
    }
}

fn regime_valide(r: RegimeBrut, unicite: &mut Unicite) -> Result<RegimeTva, ErreurFiscale> {
    let code = CodeRegime::parse(&r.code).map_err(ErreurFiscale::Code)?;
    controler_source(&r.code, &r.article, &r.pages)?;
    if r.libelle.trim().is_empty() {
        return Err(ErreurFiscale::SansLibelle(r.code));
    }
    if r.source_deduction.trim().is_empty() || r.source_exigibilite.trim().is_empty() {
        return Err(ErreurFiscale::SansSource(r.code));
    }
    unicite.verifier(&r.code, &r.article, r.alinea.as_deref())?;
    let taux = r
        .taux
        .iter()
        .map(|t| {
            TauxTva::parse(t).map_err(|erreur| ErreurFiscale::Taux {
                code: r.code.clone(),
                erreur,
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let exigibilites_etiquette = r
        .exigibilites_etiquette
        .iter()
        .map(|e| {
            Exigibilite::depuis_code(e).ok_or_else(|| ErreurFiscale::Incoherente {
                code: r.code.clone(),
                motif: "exigibilité d'étiquette inconnue",
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    if r.exonere && (!taux.is_empty() || !exigibilites_etiquette.is_empty()) {
        return Err(ErreurFiscale::Incoherente {
            code: r.code,
            motif: "un régime exonéré n'a ni taux ni exigibilité d'étiquette",
        });
    }
    Ok(RegimeTva {
        code,
        libelle: r.libelle,
        article: r.article,
        alinea: r.alinea,
        pages: r.pages,
        exonere: r.exonere,
        deduction: r.deduction,
        exigibilite: r.exigibilite,
        source_deduction: r.source_deduction,
        source_exigibilite: r.source_exigibilite,
        taux,
        exigibilites_etiquette,
        a_valider: r.a_valider,
        note: r.note,
    })
}

fn motif_valide(
    m: MotifBrut,
    regimes: &[RegimeTva],
    unicite: &mut Unicite,
) -> Result<MotifExoneration, ErreurFiscale> {
    let code = CodeMotif::parse(&m.code).map_err(ErreurFiscale::Code)?;
    let regime = CodeRegime::parse(&m.regime).map_err(ErreurFiscale::Code)?;
    controler_source(&m.code, &m.article, &m.pages)?;
    if m.libelle.trim().is_empty() || m.alinea.trim().is_empty() {
        return Err(ErreurFiscale::SansLibelle(m.code));
    }
    unicite.verifier(&m.code, &m.article, Some(&m.alinea))?;
    let Some(parent) = regimes.iter().find(|r| r.code == regime) else {
        return Err(ErreurFiscale::RegimeAbsent {
            motif: m.code,
            regime: m.regime,
        });
    };
    if !parent.exonere {
        return Err(ErreurFiscale::Incoherente {
            code: m.code,
            motif: "motif d'un régime non exonéré",
        });
    }
    let suffixe = m
        .code
        .strip_prefix(regime.as_str())
        .and_then(|r| r.strip_prefix('-'));
    if suffixe.is_none_or(str::is_empty) {
        return Err(ErreurFiscale::Incoherente {
            code: m.code,
            motif: "le code d'un motif est préfixé du code de son régime et de « - »",
        });
    }
    Ok(MotifExoneration {
        code,
        regime,
        article: m.article,
        alinea: m.alinea,
        pages: m.pages,
        libelle: m.libelle,
        deduction: m.deduction,
    })
}

fn chevauchement_valide(
    c: ChevauchementBrut,
    motifs: &[MotifExoneration],
) -> Result<Chevauchement, ErreurFiscale> {
    let motif = CodeMotif::parse(&c.motif).map_err(ErreurFiscale::Code)?;
    let nom = format!("{}-{}", c.article, c.alinea);
    controler_source(&nom, &c.article, &c.pages)?;
    if !motifs.iter().any(|m| m.code == motif) {
        return Err(ErreurFiscale::CibleAbsente(nom));
    }
    // Un alinéa qui chevauche n'est pas un code distinct.
    if motifs
        .iter()
        .any(|m| m.article == c.article && m.alinea == c.alinea)
    {
        return Err(ErreurFiscale::ArticleEnDouble(nom));
    }
    Ok(Chevauchement {
        article: c.article,
        alinea: c.alinea,
        pages: c.pages,
        motif,
        note: c.note,
    })
}

impl ReferentielTva {
    /// Charge et valide les données embarquées.
    ///
    /// # Errors
    /// [`ErreurFiscale`] : code absent de la forme attendue, entrée sans source, taux non
    /// décimal, doublon d'article et d'alinéa, motif d'un régime absent ou non exonéré…
    pub fn charger() -> Result<Self, ErreurFiscale> {
        Self::depuis_textes(REGIMES_TOML, MOTIFS_TOML)
    }

    /// Charge depuis des textes TOML (tests de rejet).
    ///
    /// # Errors
    /// Comme [`ReferentielTva::charger`].
    pub fn depuis_textes(regimes: &str, motifs: &str) -> Result<Self, ErreurFiscale> {
        let regimes: FichierRegimes =
            toml::from_str(regimes).map_err(|e| ErreurFiscale::Syntaxe(e.to_string()))?;
        let motifs: FichierMotifs =
            toml::from_str(motifs).map_err(|e| ErreurFiscale::Syntaxe(e.to_string()))?;
        let mut unicite = Unicite::default();
        let liste = regimes
            .regime
            .into_iter()
            .map(|r| regime_valide(r, &mut unicite))
            .collect::<Result<Vec<_>, _>>()?;
        let liste_motifs = motifs
            .motif
            .into_iter()
            .map(|m| motif_valide(m, &liste, &mut unicite))
            .collect::<Result<Vec<_>, _>>()?;
        let chevauchements = motifs
            .chevauchement
            .into_iter()
            .map(|c| chevauchement_valide(c, &liste_motifs))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            regimes: liste,
            motifs: liste_motifs,
            chevauchements,
        })
    }

    /// Régimes, dans l'ordre du fichier.
    #[must_use]
    pub fn regimes(&self) -> &[RegimeTva] {
        &self.regimes
    }

    /// Motifs d'exonération, dans l'ordre du fichier : la liste fermée.
    #[must_use]
    pub fn motifs(&self) -> &[MotifExoneration] {
        &self.motifs
    }

    /// Alinéas de l'art. 361 portés par un code de l'art. 380.
    #[must_use]
    pub fn chevauchements(&self) -> &[Chevauchement] {
        &self.chevauchements
    }

    /// Recherche d'un régime par code ; `None` si le code est absent de la liste.
    #[must_use]
    pub fn regime(&self, code: &str) -> Option<&RegimeTva> {
        self.regimes.iter().find(|r| r.code.as_str() == code)
    }

    /// Recherche d'un motif par code.
    #[must_use]
    pub fn motif(&self, code: &str) -> Option<&MotifExoneration> {
        self.motifs.iter().find(|m| m.code.as_str() == code)
    }

    /// Motifs d'un régime exonéré.
    pub fn motifs_du_regime<'a>(
        &'a self,
        regime: &'a CodeRegime,
    ) -> impl Iterator<Item = &'a MotifExoneration> {
        self.motifs.iter().filter(move |m| &m.regime == regime)
    }
}

impl ReferentielFiscal for ReferentielTva {
    fn profil_regime(&self, code: &CodeRegime) -> Option<ProfilRegime> {
        self.regime(code.as_str()).map(|r| {
            ProfilRegime::nouveau(r.exonere, r.taux.clone(), r.exigibilites_etiquette.clone())
        })
    }

    fn motif_admis(&self, regime: &CodeRegime, motif: &CodeMotif) -> bool {
        self.motifs_du_regime(regime).any(|m| &m.code == motif)
    }
}
