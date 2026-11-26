//! Une version de paramètres fiscaux d'un pays (ou d'un dossier), chargée et validée.

use std::collections::{BTreeMap, BTreeSet};

use compta_core::{CodeMotif, CodeRegime, Exigibilite, ProfilRegime, ReferentielFiscal, TauxTva};
use serde::Deserialize;
use time::Date;

use crate::date;
use crate::erreur::ErreurFiscale;
use crate::socle::Socle;
use crate::types::{Deduction, ExigibiliteLegale, Portee, Retention, RoleTaux, TauxNomme};

// --- Données brutes (une version = en-tête + régimes + motifs, concaténés) -----------------

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FichierVersion {
    pays: String,
    version: String,
    date_effet: String,
    date_fin: Option<String>,
    source: String,
    document: Vec<DocumentBrut>,
    taux: BTreeMap<String, TauxBrut>,
    retention: RetentionBrute,
    #[serde(default)]
    regime: Vec<RegimeBrut>,
    #[serde(default)]
    motif: Vec<MotifBrut>,
    #[serde(default)]
    chevauchement: Vec<ChevauchementBrut>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DocumentBrut {
    id: String,
    titre: String,
    pages_min: Option<u16>,
    pages_max: Option<u16>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TauxBrut {
    valeur: String,
    source: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RetentionBrute {
    annees_min: u16,
    source: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RegimeBrut {
    code: String,
    libelle: String,
    article: String,
    alinea: Option<String>,
    #[serde(default)]
    pages: Vec<u16>,
    document: Option<String>,
    deduction: Deduction,
    exigibilite: ExigibiliteLegale,
    source_deduction: String,
    source_exigibilite: String,
    source_ligne_taxe: String,
    a_valider: bool,
    note: Option<String>,
    debut: Option<String>,
    fin: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MotifBrut {
    code: String,
    regime: String,
    article: String,
    alinea: String,
    #[serde(default)]
    pages: Vec<u16>,
    document: Option<String>,
    libelle: String,
    deduction: Deduction,
    debut: Option<String>,
    fin: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ChevauchementBrut {
    article: String,
    alinea: String,
    #[serde(default)]
    pages: Vec<u16>,
    document: Option<String>,
    motif: String,
    note: String,
}

// --- Types validés ------------------------------------------------------------------------

/// Document source d'une version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Document {
    /// Identifiant dans la version.
    pub id: String,
    /// Titre et édition.
    pub titre: String,
    /// Plage de pages du chapitre cité (`None` : pas de contrôle de page).
    pub pages: Option<(u16, u16)>,
}

/// Régime de TVA offert par une version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegimeTva {
    code: CodeRegime,
    libelle: String,
    article: String,
    alinea: Option<String>,
    pages: Vec<u16>,
    exonere: bool,
    deduction: Deduction,
    exigibilite: ExigibiliteLegale,
    source_deduction: String,
    source_exigibilite: String,
    ligne_taxe: bool,
    source_ligne_taxe: String,
    roles_taux: Vec<RoleTaux>,
    taux: Vec<TauxTva>,
    exigibilites_etiquette: Vec<Exigibilite>,
    a_valider: bool,
    note: Option<String>,
    debut: Option<Date>,
    fin: Option<Date>,
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
    /// Article de la source.
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
    /// Exigibilité légale.
    #[must_use]
    pub fn exigibilite(&self) -> ExigibiliteLegale {
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
    /// Le régime comporte une ligne de taxe (sinon seule la base est étiquetée).
    #[must_use]
    pub fn porte_une_ligne_de_taxe(&self) -> bool {
        self.ligne_taxe
    }
    /// Source du champ précédent.
    #[must_use]
    pub fn source_ligne_taxe(&self) -> &str {
        &self.source_ligne_taxe
    }
    /// Rôles de taux admis.
    #[must_use]
    pub fn roles_taux(&self) -> &[RoleTaux] {
        &self.roles_taux
    }
    /// Taux admis (pourcent), valeurs de la version pour les rôles du régime.
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
    /// Début de validité (inclus).
    #[must_use]
    pub fn debut(&self) -> Option<Date> {
        self.debut
    }
    /// Fin de validité (incluse).
    #[must_use]
    pub fn fin(&self) -> Option<Date> {
        self.fin
    }
    /// Vrai si le régime est valide ce jour-là.
    #[must_use]
    pub fn valide_le(&self, jour: Date) -> bool {
        valide(self.debut, self.fin, jour)
    }
}

/// Motif d'exonération offert par une version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MotifExoneration {
    code: CodeMotif,
    regime: CodeRegime,
    article: String,
    alinea: String,
    pages: Vec<u16>,
    libelle: String,
    deduction: Deduction,
    debut: Option<Date>,
    fin: Option<Date>,
}

impl MotifExoneration {
    /// Code stable.
    #[must_use]
    pub fn code(&self) -> &CodeMotif {
        &self.code
    }
    /// Régime exonéré du motif.
    #[must_use]
    pub fn regime(&self) -> &CodeRegime {
        &self.regime
    }
    /// Article de la source.
    #[must_use]
    pub fn article(&self) -> &str {
        &self.article
    }
    /// Alinéa.
    #[must_use]
    pub fn alinea(&self) -> &str {
        &self.alinea
    }
    /// Pages de la source.
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
    /// Début de validité (inclus).
    #[must_use]
    pub fn debut(&self) -> Option<Date> {
        self.debut
    }
    /// Fin de validité (incluse).
    #[must_use]
    pub fn fin(&self) -> Option<Date> {
        self.fin
    }
    /// Vrai si le motif est valide ce jour-là.
    #[must_use]
    pub fn valide_le(&self, jour: Date) -> bool {
        valide(self.debut, self.fin, jour)
    }
}

/// Alinéa d'un régime sans droit à déduction qui n'a pas de code propre : un régime avec droit
/// le porte aussi.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chevauchement {
    /// Article.
    pub article: String,
    /// Alinéa.
    pub alinea: String,
    /// Pages de la source.
    pub pages: Vec<u16>,
    /// Code retenu.
    pub motif: CodeMotif,
    /// Précision.
    pub note: String,
}

fn valide(debut: Option<Date>, fin: Option<Date>, jour: Date) -> bool {
    debut.is_none_or(|d| d <= jour) && fin.is_none_or(|f| jour <= f)
}

/// Version des paramètres fiscaux d'un pays, ou d'un dossier : **immuable une fois publiée**.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParametresFiscaux {
    pays: String,
    version: String,
    identifiant: String,
    portee: Portee,
    date_effet: Date,
    date_fin: Option<Date>,
    source: String,
    documents: Vec<Document>,
    taux: Vec<TauxNomme>,
    regimes: Vec<RegimeTva>,
    motifs: Vec<MotifExoneration>,
    chevauchements: Vec<Chevauchement>,
    retention: Retention,
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

/// Résout le document d'une entrée et contrôle sa source : article, pages dans la plage.
fn controler_source(
    nom: &str,
    article: &str,
    pages: &[u16],
    document: Option<&str>,
    documents: &[Document],
) -> Result<(), ErreurFiscale> {
    if article.trim().is_empty() {
        return Err(ErreurFiscale::SansSource(nom.to_owned()));
    }
    let doc = match (document, documents) {
        (Some(id), _) => documents.iter().find(|d| d.id == id),
        (None, [seul]) => Some(seul),
        _ => None,
    }
    .ok_or_else(|| ErreurFiscale::Document(nom.to_owned()))?;
    if let Some((min, max)) = doc.pages {
        if pages.is_empty() {
            return Err(ErreurFiscale::SansSource(nom.to_owned()));
        }
        if let Some(page) = pages.iter().find(|p| !(min..=max).contains(p)) {
            return Err(ErreurFiscale::Page {
                code: nom.to_owned(),
                page: *page,
            });
        }
    }
    Ok(())
}

fn date_facultative(nom: &str, texte: Option<&str>) -> Result<Option<Date>, ErreurFiscale> {
    texte
        .map(|t| date::lire(t).ok_or_else(|| ErreurFiscale::Date(nom.to_owned())))
        .transpose()
}

fn periode(
    nom: &str,
    debut: Option<&str>,
    fin: Option<&str>,
) -> Result<(Option<Date>, Option<Date>), ErreurFiscale> {
    let debut = date_facultative(nom, debut)?;
    let fin = date_facultative(nom, fin)?;
    if let (Some(d), Some(f)) = (debut, fin)
        && f < d
    {
        return Err(ErreurFiscale::Periode(nom.to_owned()));
    }
    Ok((debut, fin))
}

fn pays_valide(texte: &str) -> bool {
    texte.len() == 2 && texte.bytes().all(|b| b.is_ascii_uppercase())
}

/// `AAAA.N` : quatre chiffres, un point, un entier sans zéro de tête superflu (au plus 4 chiffres).
fn version_valide(texte: &str) -> bool {
    let Some((annee, rang)) = texte.split_once('.') else {
        return false;
    };
    annee.len() == 4
        && annee.bytes().all(|b| b.is_ascii_digit())
        && (1..=4).contains(&rang.len())
        && rang.bytes().all(|b| b.is_ascii_digit())
        && (rang == "0" || !rang.starts_with('0'))
}

impl ParametresFiscaux {
    /// Charge et valide une version depuis son texte TOML complet (en-tête, régimes et motifs ;
    /// les fichiers d'un paquet se concatènent).
    ///
    /// # Errors
    /// [`ErreurFiscale`] : syntaxe, pays, version ou date invalides, code absent du socle,
    /// entrée sans source, taux non décimal ou sans valeur, doublon, motif d'un régime absent ou
    /// non exonéré, rétention sous le plancher…
    pub fn depuis_texte(socle: &Socle, portee: Portee, texte: &str) -> Result<Self, ErreurFiscale> {
        let f: FichierVersion =
            toml::from_str(texte).map_err(|e| ErreurFiscale::Syntaxe(e.to_string()))?;
        if !pays_valide(&f.pays) {
            return Err(ErreurFiscale::Pays(f.pays));
        }
        if !version_valide(&f.version) {
            return Err(ErreurFiscale::Version(f.version));
        }
        let date_effet = date::lire(&f.date_effet)
            .ok_or_else(|| ErreurFiscale::Date("date_effet".to_owned()))?;
        let date_fin = date_facultative("date_fin", f.date_fin.as_deref())?;
        if date_fin.is_some_and(|fin| fin < date_effet) {
            return Err(ErreurFiscale::Periode("version".to_owned()));
        }
        if f.source.trim().is_empty() {
            return Err(ErreurFiscale::SansSource("version".to_owned()));
        }
        let identifiant = match portee {
            Portee::Pack => format!("{}-{}", f.pays.to_ascii_lowercase(), f.version),
            Portee::Dossier(n) => format!("{}-{}+d{n}", f.pays.to_ascii_lowercase(), f.version),
        };
        let documents = documents_valides(f.document)?;
        let taux = taux_valides(socle, f.taux)?;
        if f.retention.source.trim().is_empty() {
            return Err(ErreurFiscale::SansSource("retention".to_owned()));
        }
        if f.retention.annees_min < socle.plancher_retention() {
            return Err(ErreurFiscale::RetentionSousPlancher {
                annees: f.retention.annees_min,
                plancher: socle.plancher_retention(),
            });
        }
        let mut unicite = Unicite::default();
        let regimes = f
            .regime
            .into_iter()
            .map(|r| regime_valide(r, socle, &taux, &documents, &mut unicite))
            .collect::<Result<Vec<_>, _>>()?;
        let motifs = f
            .motif
            .into_iter()
            .map(|m| motif_valide(m, socle, &regimes, &documents, &mut unicite))
            .collect::<Result<Vec<_>, _>>()?;
        let chevauchements = f
            .chevauchement
            .into_iter()
            .map(|c| chevauchement_valide(c, &motifs, &documents))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            pays: f.pays,
            version: f.version,
            identifiant,
            portee,
            date_effet,
            date_fin,
            source: f.source,
            documents,
            taux,
            regimes,
            motifs,
            chevauchements,
            retention: Retention {
                annees_min: f.retention.annees_min,
                source: f.retention.source,
            },
        })
    }

    /// Pays (ISO 3166 alpha-2, majuscules).
    #[must_use]
    pub fn pays(&self) -> &str {
        &self.pays
    }
    /// Version dans le pays (`AAAA.N`).
    #[must_use]
    pub fn version(&self) -> &str {
        &self.version
    }
    /// Identifiant enregistré dans l'écriture : `sn-2025.1`, ou `sn-2025.1+d2` pour un dossier.
    #[must_use]
    pub fn identifiant(&self) -> &str {
        &self.identifiant
    }
    /// Portée : paquet du pays ou dossier.
    #[must_use]
    pub fn portee(&self) -> Portee {
        self.portee
    }
    /// Premier jour d'application.
    #[must_use]
    pub fn date_effet(&self) -> Date {
        self.date_effet
    }
    /// Dernier jour d'application, s'il est fixé (abrogation sans successeur).
    #[must_use]
    pub fn date_fin(&self) -> Option<Date> {
        self.date_fin
    }
    /// Textes sources de la version.
    #[must_use]
    pub fn source(&self) -> &str {
        &self.source
    }
    /// Documents cités.
    #[must_use]
    pub fn documents(&self) -> &[Document] {
        &self.documents
    }
    /// Taux nommés.
    #[must_use]
    pub fn taux(&self) -> &[TauxNomme] {
        &self.taux
    }
    /// Valeur du taux d'un rôle, s'il est défini.
    #[must_use]
    pub fn taux_du_role(&self, role: &str) -> Option<&TauxTva> {
        self.taux
            .iter()
            .find(|t| t.role.as_str() == role)
            .map(|t| &t.valeur)
    }
    /// Régimes offerts, dans l'ordre du fichier.
    #[must_use]
    pub fn regimes(&self) -> &[RegimeTva] {
        &self.regimes
    }
    /// Motifs, dans l'ordre du fichier.
    #[must_use]
    pub fn motifs(&self) -> &[MotifExoneration] {
        &self.motifs
    }
    /// Chevauchements.
    #[must_use]
    pub fn chevauchements(&self) -> &[Chevauchement] {
        &self.chevauchements
    }
    /// Rétention des pièces.
    #[must_use]
    pub fn retention(&self) -> &Retention {
        &self.retention
    }
    /// Régime par code, sans égard aux dates.
    #[must_use]
    pub fn regime(&self, code: &str) -> Option<&RegimeTva> {
        self.regimes.iter().find(|r| r.code.as_str() == code)
    }
    /// Motif par code, sans égard aux dates.
    #[must_use]
    pub fn motif(&self, code: &str) -> Option<&MotifExoneration> {
        self.motifs.iter().find(|m| m.code.as_str() == code)
    }
    /// Motifs d'un régime exonéré, sans égard aux dates.
    pub fn motifs_du_regime<'a>(
        &'a self,
        regime: &'a CodeRegime,
    ) -> impl Iterator<Item = &'a MotifExoneration> {
        self.motifs.iter().filter(move |m| &m.regime == regime)
    }

    /// Vrai si la version couvre ce jour (de `date_effet` à `date_fin`), sans tenir compte des
    /// versions suivantes : voir [`crate::Registre`] pour la sélection.
    #[must_use]
    pub fn couvre(&self, jour: Date) -> bool {
        self.date_effet <= jour && self.date_fin.is_none_or(|f| jour <= f)
    }

    /// Vue de la version à une date : seules les entrées valides ce jour-là. Elle implémente
    /// [`ReferentielFiscal`].
    #[must_use]
    pub fn a_la_date(&self, jour: Date) -> VueAuJour<'_> {
        VueAuJour {
            parametres: self,
            jour,
        }
    }

    /// Vérifie qu'une version **suivante** du même pays garde tous les motifs de celle-ci, avec
    /// le même régime (un code de motif garde son sens, ADR 0030 point 4).
    ///
    /// # Errors
    /// [`ErreurFiscale::MotifRetire`] pour le premier motif perdu ou réaffecté.
    pub fn verifier_continuite_avec(&self, suivante: &Self) -> Result<(), ErreurFiscale> {
        for m in &self.motifs {
            match suivante.motif(m.code.as_str()) {
                Some(n) if n.regime == m.regime => {}
                _ => return Err(ErreurFiscale::MotifRetire(m.code.to_string())),
            }
        }
        Ok(())
    }
}

fn documents_valides(brut: Vec<DocumentBrut>) -> Result<Vec<Document>, ErreurFiscale> {
    if brut.is_empty() {
        return Err(ErreurFiscale::SansSource("document".to_owned()));
    }
    let mut vus = BTreeSet::new();
    brut.into_iter()
        .map(|d| {
            if d.id.trim().is_empty() || d.titre.trim().is_empty() || !vus.insert(d.id.clone()) {
                return Err(ErreurFiscale::Document(d.id));
            }
            let pages = match (d.pages_min, d.pages_max) {
                (Some(a), Some(b)) if a <= b => Some((a, b)),
                (None, None) => None,
                _ => return Err(ErreurFiscale::Document(d.id)),
            };
            Ok(Document {
                id: d.id,
                titre: d.titre,
                pages,
            })
        })
        .collect()
}

fn taux_valides(
    socle: &Socle,
    brut: BTreeMap<String, TauxBrut>,
) -> Result<Vec<TauxNomme>, ErreurFiscale> {
    // Dans l'ordre des rôles du socle.
    let mut brut = brut;
    let mut liste = Vec::new();
    for role in socle.roles() {
        if let Some(t) = brut.remove(role.as_str()) {
            if t.source.trim().is_empty() {
                return Err(ErreurFiscale::SansSource(format!("taux {}", role.as_str())));
            }
            let valeur = TauxTva::parse(&t.valeur).map_err(|erreur| ErreurFiscale::Taux {
                code: role.as_str().to_owned(),
                erreur,
            })?;
            liste.push(TauxNomme {
                role: role.clone(),
                valeur,
                source: t.source,
            });
        }
    }
    if let Some((inconnu, _)) = brut.into_iter().next() {
        return Err(ErreurFiscale::RoleInconnu(inconnu));
    }
    Ok(liste)
}

fn regime_valide(
    r: RegimeBrut,
    socle: &Socle,
    taux: &[TauxNomme],
    documents: &[Document],
    unicite: &mut Unicite,
) -> Result<RegimeTva, ErreurFiscale> {
    let code = CodeRegime::parse(&r.code).map_err(ErreurFiscale::Code)?;
    let base = socle
        .regime(&r.code)
        .ok_or_else(|| ErreurFiscale::HorsSocle(r.code.clone()))?;
    controler_source(
        &r.code,
        &r.article,
        &r.pages,
        r.document.as_deref(),
        documents,
    )?;
    if r.libelle.trim().is_empty() {
        return Err(ErreurFiscale::SansLibelle(r.code));
    }
    if r.source_deduction.trim().is_empty()
        || r.source_exigibilite.trim().is_empty()
        || r.source_ligne_taxe.trim().is_empty()
    {
        return Err(ErreurFiscale::SansSource(r.code));
    }
    unicite.verifier(&r.code, &r.article, r.alinea.as_deref())?;
    let (debut, fin) = periode(&r.code, r.debut.as_deref(), r.fin.as_deref())?;
    // Taux admis : les valeurs de la version pour les rôles que le socle prévoit.
    let admis: Vec<&TauxNomme> = base
        .roles_taux()
        .iter()
        .filter_map(|role| taux.iter().find(|t| &t.role == role))
        .collect();
    if !base.roles_taux().is_empty() && admis.is_empty() {
        return Err(ErreurFiscale::TauxAbsent(r.code));
    }
    Ok(RegimeTva {
        code,
        libelle: r.libelle,
        article: r.article,
        alinea: r.alinea,
        pages: r.pages,
        exonere: base.est_exonere(),
        deduction: r.deduction,
        exigibilite: r.exigibilite,
        source_deduction: r.source_deduction,
        source_exigibilite: r.source_exigibilite,
        ligne_taxe: base.porte_une_ligne_de_taxe(),
        source_ligne_taxe: r.source_ligne_taxe,
        roles_taux: admis.iter().map(|t| t.role.clone()).collect(),
        taux: admis.iter().map(|t| t.valeur).collect(),
        exigibilites_etiquette: base.exigibilites_etiquette().to_vec(),
        a_valider: r.a_valider,
        note: r.note,
        debut,
        fin,
    })
}

fn motif_valide(
    m: MotifBrut,
    socle: &Socle,
    regimes: &[RegimeTva],
    documents: &[Document],
    unicite: &mut Unicite,
) -> Result<MotifExoneration, ErreurFiscale> {
    let code = CodeMotif::parse(&m.code).map_err(ErreurFiscale::Code)?;
    let regime = CodeRegime::parse(&m.regime).map_err(ErreurFiscale::Code)?;
    controler_source(
        &m.code,
        &m.article,
        &m.pages,
        m.document.as_deref(),
        documents,
    )?;
    if m.libelle.trim().is_empty() || m.alinea.trim().is_empty() {
        return Err(ErreurFiscale::SansLibelle(m.code));
    }
    unicite.verifier(&m.code, &m.article, Some(&m.alinea))?;
    let (debut, fin) = periode(&m.code, m.debut.as_deref(), m.fin.as_deref())?;
    if regimes.iter().all(|r| r.code != regime) {
        return Err(ErreurFiscale::RegimeAbsent {
            motif: m.code,
            regime: m.regime,
        });
    }
    if !socle
        .regime(&m.regime)
        .is_some_and(crate::socle::RegimeSocle::est_exonere)
    {
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
        debut,
        fin,
    })
}

fn chevauchement_valide(
    c: ChevauchementBrut,
    motifs: &[MotifExoneration],
    documents: &[Document],
) -> Result<Chevauchement, ErreurFiscale> {
    let motif = CodeMotif::parse(&c.motif).map_err(ErreurFiscale::Code)?;
    let nom = format!("{}-{}", c.article, c.alinea);
    controler_source(&nom, &c.article, &c.pages, c.document.as_deref(), documents)?;
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

/// Vue d'une version à une date : les régimes et motifs valides ce jour-là.
#[derive(Debug, Clone, Copy)]
pub struct VueAuJour<'a> {
    parametres: &'a ParametresFiscaux,
    jour: Date,
}

impl<'a> VueAuJour<'a> {
    /// Version vue.
    #[must_use]
    pub fn parametres(&self) -> &'a ParametresFiscaux {
        self.parametres
    }
    /// Jour de la vue.
    #[must_use]
    pub fn jour(&self) -> Date {
        self.jour
    }
    /// Régimes valides ce jour-là.
    pub fn regimes(&self) -> impl Iterator<Item = &'a RegimeTva> + use<'a> {
        let jour = self.jour;
        self.parametres
            .regimes
            .iter()
            .filter(move |r| r.valide_le(jour))
    }
    /// Motifs valides ce jour-là.
    pub fn motifs(&self) -> impl Iterator<Item = &'a MotifExoneration> + use<'a> {
        let jour = self.jour;
        self.parametres
            .motifs
            .iter()
            .filter(move |m| m.valide_le(jour))
    }
    /// Régime valide ce jour-là.
    #[must_use]
    pub fn regime(&self, code: &str) -> Option<&'a RegimeTva> {
        self.regimes().find(|r| r.code.as_str() == code)
    }
}

impl ReferentielFiscal for VueAuJour<'_> {
    fn profil_regime(&self, code: &CodeRegime) -> Option<ProfilRegime> {
        self.regime(code.as_str()).map(|r| {
            ProfilRegime::nouveau(
                r.exonere,
                r.ligne_taxe,
                r.taux.clone(),
                r.exigibilites_etiquette.clone(),
            )
        })
    }

    fn motif_admis(&self, regime: &CodeRegime, motif: &CodeMotif) -> bool {
        self.motifs()
            .any(|m| &m.regime == regime && &m.code == motif)
    }
}
