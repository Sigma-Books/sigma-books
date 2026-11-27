//! Prélèvements et taxes perçus à la source par un intermédiaire (ADR 0034 point 10) : fichier
//! `prelevements.toml` d'une version. Le module donne le **mécanisme** (valeurs sourcées, calcul
//! de contrôle avant arrondi) ; il ne dit rien de la comptabilisation (B5).

use std::collections::BTreeSet;

use compta_core::{Devise, Montant, TauxTva};
use rust_decimal::Decimal;
use serde::Deserialize;
use time::Date;

use super::{Document, controler_source, periode, valide};
use crate::erreur::ErreurFiscale;
use crate::socle::Socle;
use crate::types::{Assiette, CodePrelevement, ExigibiliteLegale};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PrelevementBrut {
    code: String,
    libelle: String,
    article: String,
    alinea: Option<String>,
    #[serde(default)]
    pages: Vec<u16>,
    document: Option<String>,
    taux: String,
    source_taux: String,
    plafond: Option<String>,
    source_plafond: Option<String>,
    devise: String,
    assiette: Assiette,
    source_assiette: String,
    redevable: String,
    source_redevable: String,
    collecte_par: String,
    source_collecte: String,
    fait_generateur: String,
    source_fait_generateur: String,
    exigibilite: ExigibiliteLegale,
    source_exigibilite: String,
    a_valider: bool,
    note: Option<String>,
    debut: Option<String>,
    fin: Option<String>,
    #[serde(default)]
    exoneration: Vec<ExonerationBrute>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ExonerationBrute {
    code: String,
    article: String,
    alinea: String,
    #[serde(default)]
    pages: Vec<u16>,
    document: Option<String>,
    libelle: String,
    seuil: Option<String>,
    fenetre_heures: Option<u16>,
}

/// Exonération d'un prélèvement (cas exonéré cité par le texte).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExonerationPrelevement {
    code: String,
    article: String,
    alinea: String,
    pages: Vec<u16>,
    document: Option<String>,
    libelle: String,
    seuil: Option<Montant>,
    fenetre_heures: Option<u16>,
}

impl ExonerationPrelevement {
    /// Code `<code du prélèvement>-<suffixe>` (suffixe : clé opaque du paquet).
    #[must_use]
    pub fn code(&self) -> &str {
        &self.code
    }
    /// Article de la source.
    #[must_use]
    pub fn article(&self) -> &str {
        &self.article
    }
    /// Alinéa (ou point) de l'article.
    #[must_use]
    pub fn alinea(&self) -> &str {
        &self.alinea
    }
    /// Pages de la source.
    #[must_use]
    pub fn pages(&self) -> &[u16] {
        &self.pages
    }
    /// Document source, s'il est nommé.
    #[must_use]
    pub fn document(&self) -> Option<&str> {
        self.document.as_deref()
    }
    /// Intitulé court.
    #[must_use]
    pub fn libelle(&self) -> &str {
        &self.libelle
    }
    /// Montant au plus égal auquel l'exonération s'applique (inclus), s'il y en a un.
    #[must_use]
    pub fn seuil(&self) -> Option<&Montant> {
        self.seuil.as_ref()
    }
    /// Durée, en heures, sur laquelle le seuil s'apprécie.
    #[must_use]
    pub fn fenetre_heures(&self) -> Option<u16> {
        self.fenetre_heures
    }
}

/// Prélèvement ou taxe perçu à la source, avec ses valeurs sourcées.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prelevement {
    code: CodePrelevement,
    libelle: String,
    article: String,
    alinea: Option<String>,
    pages: Vec<u16>,
    document: Option<String>,
    taux: TauxTva,
    source_taux: String,
    plafond: Option<(Montant, String)>,
    devise: Devise,
    assiette: Assiette,
    source_assiette: String,
    redevable: String,
    source_redevable: String,
    collecte_par: String,
    source_collecte: String,
    fait_generateur: String,
    source_fait_generateur: String,
    exigibilite: ExigibiliteLegale,
    source_exigibilite: String,
    a_valider: bool,
    note: Option<String>,
    debut: Option<Date>,
    fin: Option<Date>,
    exonerations: Vec<ExonerationPrelevement>,
}

impl Prelevement {
    /// Code générique (socle).
    #[must_use]
    pub fn code(&self) -> &CodePrelevement {
        &self.code
    }
    /// Intitulé court.
    #[must_use]
    pub fn libelle(&self) -> &str {
        &self.libelle
    }
    /// Article qui l'institue.
    #[must_use]
    pub fn article(&self) -> &str {
        &self.article
    }
    /// Alinéa, s'il y a lieu.
    #[must_use]
    pub fn alinea(&self) -> Option<&str> {
        self.alinea.as_deref()
    }
    /// Pages de la source.
    #[must_use]
    pub fn pages(&self) -> &[u16] {
        &self.pages
    }
    /// Document source, s'il est nommé.
    #[must_use]
    pub fn document(&self) -> Option<&str> {
        self.document.as_deref()
    }
    /// Taux, en pourcent.
    #[must_use]
    pub fn taux(&self) -> TauxTva {
        self.taux
    }
    /// Source du taux.
    #[must_use]
    pub fn source_taux(&self) -> &str {
        &self.source_taux
    }
    /// Montant maximal par transaction, s'il est fixé.
    #[must_use]
    pub fn plafond(&self) -> Option<&Montant> {
        self.plafond.as_ref().map(|(m, _)| m)
    }
    /// Source du plafond.
    #[must_use]
    pub fn source_plafond(&self) -> Option<&str> {
        self.plafond.as_ref().map(|(_, s)| s.as_str())
    }
    /// Monnaie des montants (plafond, seuils).
    #[must_use]
    pub fn devise(&self) -> Devise {
        self.devise
    }
    /// Assiette légale.
    #[must_use]
    pub fn assiette(&self) -> Assiette {
        self.assiette
    }
    /// Source de l'assiette.
    #[must_use]
    pub fn source_assiette(&self) -> &str {
        &self.source_assiette
    }
    /// Personne visée par le texte.
    #[must_use]
    pub fn redevable(&self) -> &str {
        &self.redevable
    }
    /// Source du redevable.
    #[must_use]
    pub fn source_redevable(&self) -> &str {
        &self.source_redevable
    }
    /// Personne qui prélève ou collecte.
    #[must_use]
    pub fn collecte_par(&self) -> &str {
        &self.collecte_par
    }
    /// Source de la collecte.
    #[must_use]
    pub fn source_collecte(&self) -> &str {
        &self.source_collecte
    }
    /// Fait générateur.
    #[must_use]
    pub fn fait_generateur(&self) -> &str {
        &self.fait_generateur
    }
    /// Source du fait générateur.
    #[must_use]
    pub fn source_fait_generateur(&self) -> &str {
        &self.source_fait_generateur
    }
    /// Exigibilité légale (le délai est dans la source).
    #[must_use]
    pub fn exigibilite(&self) -> ExigibiliteLegale {
        self.exigibilite
    }
    /// Source de l'exigibilité.
    #[must_use]
    pub fn source_exigibilite(&self) -> &str {
        &self.source_exigibilite
    }
    /// À faire valider par le fiscaliste.
    #[must_use]
    pub fn a_valider(&self) -> bool {
        self.a_valider
    }
    /// Précision.
    #[must_use]
    pub fn note(&self) -> Option<&str> {
        self.note.as_deref()
    }
    /// Début de validité propre (inclus).
    #[must_use]
    pub fn debut(&self) -> Option<Date> {
        self.debut
    }
    /// Fin de validité propre (incluse).
    #[must_use]
    pub fn fin(&self) -> Option<Date> {
        self.fin
    }
    /// Vrai si le prélèvement est valide ce jour-là.
    #[must_use]
    pub fn valide_le(&self, jour: Date) -> bool {
        valide(self.debut, self.fin, jour)
    }
    /// Exonérations, dans l'ordre du fichier.
    #[must_use]
    pub fn exonerations(&self) -> &[ExonerationPrelevement] {
        &self.exonerations
    }

    /// Montant théorique sur une assiette, **avant tout arrondi** : `assiette × taux / 100`,
    /// borné par le plafond par transaction. Sert de **contrôle** du montant lu sur un relevé,
    /// jamais à le recalculer (ADR 0034 point 10) ; la règle d'arrondi n'est pas dans le texte
    /// (question au fiscaliste) : elle reste à l'appelant.
    ///
    /// # Errors
    /// [`ErreurFiscale::Montant`] si l'assiette est négative ou n'est pas dans la devise du
    /// prélèvement.
    pub fn montant_avant_arrondi(&self, assiette: &Montant) -> Result<Montant, ErreurFiscale> {
        if assiette.devise() != self.devise || assiette.valeur() < Decimal::ZERO {
            return Err(ErreurFiscale::Montant(self.code.as_str().to_owned()));
        }
        let brut = assiette
            .valeur()
            .checked_mul(self.taux.valeur())
            .and_then(|v| v.checked_div(Decimal::ONE_HUNDRED))
            .ok_or_else(|| ErreurFiscale::Montant(self.code.as_str().to_owned()))?;
        let borne = match self.plafond() {
            Some(p) if p.valeur() < brut => p.valeur(),
            _ => brut,
        };
        Ok(Montant::nouveau(borne, self.devise))
    }
}

fn texte_source(nom: &str, valeurs: &[&str]) -> Result<(), ErreurFiscale> {
    if valeurs.iter().any(|v| v.trim().is_empty()) {
        return Err(ErreurFiscale::SansSource(nom.to_owned()));
    }
    Ok(())
}

/// Montant strictement positif dans la devise donnée.
fn montant_positif(nom: &str, texte: &str, devise: Devise) -> Result<Montant, ErreurFiscale> {
    let m = Montant::parse(texte, devise).map_err(|_| ErreurFiscale::Montant(nom.to_owned()))?;
    if !m.est_strictement_positif() {
        return Err(ErreurFiscale::Montant(nom.to_owned()));
    }
    Ok(m)
}

/// Valide les prélèvements d'une version : codes du socle uniques, source de chaque champ,
/// taux décimal exact dans ]0 ; 100], plafond et seuils strictement positifs, exonérations
/// préfixées du code de leur prélèvement, une valeur `non_trouve` signalée `a_valider`.
pub(super) fn prelevements_valides(
    brut: Vec<PrelevementBrut>,
    socle: &Socle,
    documents: &[Document],
) -> Result<Vec<Prelevement>, ErreurFiscale> {
    let mut codes = BTreeSet::new();
    let mut exos = BTreeSet::new();
    brut.into_iter()
        .map(|p| {
            let code = CodePrelevement::parse(&p.code)?;
            if socle.prelevement(&p.code).is_none() {
                return Err(ErreurFiscale::HorsSocle(p.code));
            }
            if !codes.insert(p.code.clone()) {
                return Err(ErreurFiscale::CodeEnDouble(p.code));
            }
            controler_source(
                &p.code,
                &p.article,
                &p.pages,
                p.document.as_deref(),
                documents,
            )?;
            if p.libelle.trim().is_empty()
                || p.redevable.trim().is_empty()
                || p.collecte_par.trim().is_empty()
                || p.fait_generateur.trim().is_empty()
            {
                return Err(ErreurFiscale::SansLibelle(p.code));
            }
            texte_source(
                &p.code,
                &[
                    &p.source_taux,
                    &p.source_assiette,
                    &p.source_redevable,
                    &p.source_collecte,
                    &p.source_fait_generateur,
                    &p.source_exigibilite,
                ],
            )?;
            let taux = TauxTva::parse(&p.taux).map_err(|erreur| ErreurFiscale::Taux {
                code: p.code.clone(),
                erreur,
            })?;
            let devise =
                Devise::parse(&p.devise).map_err(|_| ErreurFiscale::Montant(p.code.clone()))?;
            let plafond = match (p.plafond, p.source_plafond) {
                (Some(m), Some(s)) if !s.trim().is_empty() => {
                    Some((montant_positif(&p.code, &m, devise)?, s))
                }
                (None, None) => None,
                _ => return Err(ErreurFiscale::SansSource(format!("{} : plafond", p.code))),
            };
            if (p.assiette == Assiette::NonTrouve || p.exigibilite == ExigibiliteLegale::NonTrouve)
                && !p.a_valider
            {
                return Err(ErreurFiscale::Incoherente {
                    code: p.code,
                    motif: "une valeur non trouvée dans le texte est à faire valider",
                });
            }
            let (debut, fin) = periode(&p.code, p.debut.as_deref(), p.fin.as_deref())?;
            let mut articles = BTreeSet::new();
            let exonerations = p
                .exoneration
                .into_iter()
                .map(|e| exoneration_valide(e, &code, devise, documents, &mut exos, &mut articles))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(Prelevement {
                code,
                libelle: p.libelle,
                article: p.article,
                alinea: p.alinea,
                pages: p.pages,
                document: p.document,
                taux,
                source_taux: p.source_taux,
                plafond,
                devise,
                assiette: p.assiette,
                source_assiette: p.source_assiette,
                redevable: p.redevable,
                source_redevable: p.source_redevable,
                collecte_par: p.collecte_par,
                source_collecte: p.source_collecte,
                fait_generateur: p.fait_generateur,
                source_fait_generateur: p.source_fait_generateur,
                exigibilite: p.exigibilite,
                source_exigibilite: p.source_exigibilite,
                a_valider: p.a_valider,
                note: p.note,
                debut,
                fin,
                exonerations,
            })
        })
        .collect()
}

fn exoneration_valide(
    e: ExonerationBrute,
    prelevement: &CodePrelevement,
    devise: Devise,
    documents: &[Document],
    codes: &mut BTreeSet<String>,
    articles: &mut BTreeSet<(String, String)>,
) -> Result<ExonerationPrelevement, ErreurFiscale> {
    let suffixe = e
        .code
        .strip_prefix(prelevement.as_str())
        .and_then(|r| r.strip_prefix('-'));
    let suffixe_valide = suffixe.is_some_and(|s| {
        !s.is_empty() && s.len() <= 8 && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
    });
    if !suffixe_valide {
        return Err(ErreurFiscale::Incoherente {
            code: e.code,
            motif: "le code d'une exonération est préfixé du code de son prélèvement et de « - »",
        });
    }
    controler_source(
        &e.code,
        &e.article,
        &e.pages,
        e.document.as_deref(),
        documents,
    )?;
    if e.libelle.trim().is_empty() || e.alinea.trim().is_empty() {
        return Err(ErreurFiscale::SansLibelle(e.code));
    }
    if !codes.insert(e.code.clone()) {
        return Err(ErreurFiscale::CodeEnDouble(e.code));
    }
    if !articles.insert((e.article.clone(), e.alinea.clone())) {
        return Err(ErreurFiscale::ArticleEnDouble(e.code));
    }
    let seuil = e
        .seuil
        .as_deref()
        .map(|s| montant_positif(&e.code, s, devise))
        .transpose()?;
    if e.fenetre_heures.is_some_and(|h| h == 0) || (e.fenetre_heures.is_some() && seuil.is_none()) {
        return Err(ErreurFiscale::Incoherente {
            code: e.code,
            motif: "une fenêtre de temps exige un seuil et dure au moins une heure",
        });
    }
    Ok(ExonerationPrelevement {
        code: e.code,
        article: e.article,
        alinea: e.alinea,
        pages: e.pages,
        document: e.document,
        libelle: e.libelle,
        seuil,
        fenetre_heures: e.fenetre_heures,
    })
}
