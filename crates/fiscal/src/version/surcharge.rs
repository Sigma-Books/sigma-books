//! Surcharge d'une version par un dossier (ADR 0030 point 3) : une NOUVELLE version, au contenu
//! complet, dérivée de la version applicable ce jour-là. Le contenu se sérialise dans le même
//! format que celui d'un paquet de pays et se recharge par le même validateur.

use std::fmt::Write as _;

use time::Date;

use super::{ParametresFiscaux, RegimeTva};
use crate::erreur::ErreurFiscale;
use crate::socle::Socle;
use crate::types::Portee;
use compta_core::TauxTva;

/// Longueur maximale du motif d'une surcharge, en octets UTF-8.
pub const MOTIF_MAX_OCTETS: usize = 400;

/// Vrai si le motif est un texte sûr : non blanc, au plus [`MOTIF_MAX_OCTETS`] octets, sans
/// caractère de contrôle (C0 et C1, au sens de `char::is_control`, indépendant de la locale) ni
/// caractère de contrôle bidirectionnel (U+200E, U+200F, U+202A à U+202E, U+2066 à U+2069), qui
/// permettraient de falsifier l'affichage du motif.
#[must_use]
pub fn motif_sur(motif: &str) -> bool {
    !motif.trim().is_empty()
        && motif.len() <= MOTIF_MAX_OCTETS
        && !motif.chars().any(|c| {
            c.is_control()
                || matches!(c, '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')
        })
}

/// Ce qu'un dossier change dans la version de base.
#[derive(Debug, Clone, Copy)]
pub struct Surcharge<'a> {
    /// Numéro de la version dans le dossier (`+d<numero>`).
    pub numero: u32,
    /// Premier jour d'application.
    pub date_effet: Date,
    /// Dernier jour d'application, s'il est fixé.
    pub date_fin: Option<Date>,
    /// Motif (texte libre : 1 à [`MOTIF_MAX_OCTETS`] octets, sans caractère de contrôle).
    pub motif: &'a str,
    /// Taux remplacés : (rôle, valeur décimale). Le rôle doit exister dans la version de base.
    pub taux: &'a [(String, String)],
    /// Régimes retirés (codes du socle offerts par la version de base, sans motifs).
    pub regimes_retires: &'a [String],
}

/// Chaîne TOML « basique » : guillemets, barre oblique inverse et contrôles échappés.
fn chaine(texte: &str) -> String {
    let mut s = String::with_capacity(texte.len() + 2);
    s.push('"');
    for c in texte.chars() {
        match c {
            '"' => s.push_str("\\\""),
            '\\' => s.push_str("\\\\"),
            c if c.is_control() => {
                let _ = write!(s, "\\u{:04X}", u32::from(c));
            }
            c => s.push(c),
        }
    }
    s.push('"');
    s
}

fn pages(p: &[u16]) -> String {
    let liste: Vec<String> = p.iter().map(ToString::to_string).collect();
    format!("[{}]", liste.join(", "))
}

fn ligne_optionnelle(texte: &mut String, nom: &str, valeur: Option<&str>) {
    if let Some(v) = valeur {
        let _ = writeln!(texte, "{nom} = {}", chaine(v));
    }
}

impl ParametresFiscaux {
    /// Contenu complet de la version, dans le format des fichiers de données : [`Self::depuis_texte`]
    /// le relit à l'identique (même version, mêmes régimes, mêmes motifs).
    #[must_use]
    pub fn vers_texte(&self) -> String {
        let mut t = String::new();
        let _ = writeln!(t, "pays = {}", chaine(&self.pays));
        let _ = writeln!(t, "version = {}", chaine(&self.version));
        let _ = writeln!(t, "date_effet = {}", chaine(&self.date_effet.to_string()));
        ligne_optionnelle(
            &mut t,
            "date_fin",
            self.date_fin.map(|d| d.to_string()).as_deref(),
        );
        let _ = writeln!(t, "source = {}", chaine(&self.source));
        for d in &self.documents {
            let _ = writeln!(
                t,
                "\n[[document]]\nid = {}\ntitre = {}",
                chaine(&d.id),
                chaine(&d.titre)
            );
            if let Some((min, max)) = d.pages {
                let _ = writeln!(t, "pages_min = {min}\npages_max = {max}");
            }
        }
        for x in &self.taux {
            let _ = writeln!(
                t,
                "\n[taux.{}]\nvaleur = {}\nsource = {}",
                x.role.as_str(),
                chaine(&x.valeur.to_string()),
                chaine(&x.source)
            );
        }
        let _ = writeln!(
            t,
            "\n[retention]\nannees_min = {}\nsource = {}",
            self.retention.annees_min,
            chaine(&self.retention.source)
        );
        for r in &self.regimes {
            ecrire_regime(&mut t, r);
        }
        for m in &self.motifs {
            let _ = writeln!(
                t,
                "\n[[motif]]\ncode = {}\nregime = {}\narticle = {}\nalinea = {}\npages = {}",
                chaine(m.code.as_str()),
                chaine(m.regime.as_str()),
                chaine(&m.article),
                chaine(&m.alinea),
                pages(&m.pages)
            );
            ligne_optionnelle(&mut t, "document", m.document.as_deref());
            let _ = writeln!(
                t,
                "libelle = {}\ndeduction = {}",
                chaine(&m.libelle),
                chaine(m.deduction.nom())
            );
            ligne_optionnelle(&mut t, "debut", m.debut.map(|d| d.to_string()).as_deref());
            ligne_optionnelle(&mut t, "fin", m.fin.map(|d| d.to_string()).as_deref());
        }
        for c in &self.chevauchements {
            let _ = writeln!(
                t,
                "\n[[chevauchement]]\narticle = {}\nalinea = {}\npages = {}",
                chaine(&c.article),
                chaine(&c.alinea),
                pages(&c.pages)
            );
            ligne_optionnelle(&mut t, "document", c.document.as_deref());
            let _ = writeln!(
                t,
                "motif = {}\nnote = {}",
                chaine(c.motif.as_str()),
                chaine(&c.note)
            );
        }
        t
    }

    /// Version de dossier dérivée de celle-ci (la version applicable à `date_effet`) : les taux
    /// remplacés et les régimes retirés, tout le reste identique. La version dérivée est **relue
    /// par le validateur commun** ([`Self::depuis_texte`]) : mêmes contrôles qu'un paquet.
    ///
    /// # Errors
    /// [`ErreurFiscale::MotifInvalide`] (motif vide, trop long, avec contrôle),
    /// [`ErreurFiscale::SurchargeVide`], [`ErreurFiscale::RoleInconnu`] (rôle absent de la base),
    /// [`ErreurFiscale::Taux`] (taux non décimal, nul, supérieur à 100),
    /// [`ErreurFiscale::RegimeNonOffert`], [`ErreurFiscale::RegimeNonRetirable`], [`ErreurFiscale::Periode`]
    /// (fin avant le début) et toute erreur de validation du contenu dérivé.
    pub fn deriver(&self, socle: &Socle, s: &Surcharge<'_>) -> Result<Self, ErreurFiscale> {
        if !motif_sur(s.motif) {
            return Err(ErreurFiscale::MotifInvalide);
        }
        if s.taux.is_empty() && s.regimes_retires.is_empty() {
            return Err(ErreurFiscale::SurchargeVide);
        }
        if s.date_fin.is_some_and(|f| f < s.date_effet) {
            return Err(ErreurFiscale::Periode("surcharge".to_owned()));
        }
        let mut d = self.clone();
        d.date_effet = s.date_effet;
        d.date_fin = s.date_fin;
        d.source = format!(
            "Version propre au dossier, dérivée de {} (motif au registre du dossier)",
            self.identifiant
        );
        for (role, valeur) in s.taux {
            let nouveau = TauxTva::parse(valeur).map_err(|erreur| ErreurFiscale::Taux {
                code: role.clone(),
                erreur,
            })?;
            let cible = d
                .taux
                .iter_mut()
                .find(|t| t.role.as_str() == role)
                .ok_or_else(|| ErreurFiscale::RoleInconnu(role.clone()))?;
            cible.valeur = nouveau;
            "Surcharge du dossier (motif au registre du dossier)".clone_into(&mut cible.source);
        }
        for code in s.regimes_retires {
            let Some(regime) = self.regime(code) else {
                return Err(ErreurFiscale::RegimeNonOffert(code.clone()));
            };
            if regime.est_exonere() || self.motifs_du_regime(regime.code()).next().is_some() {
                return Err(ErreurFiscale::RegimeNonRetirable(code.clone()));
            }
            d.regimes.retain(|r| r.code.as_str() != code);
        }
        // Relecture par le validateur commun : les taux admis de chaque régime sont recalculés.
        Self::depuis_texte(socle, Portee::Dossier(s.numero), &d.vers_texte())
    }
}

fn ecrire_regime(t: &mut String, r: &RegimeTva) {
    let _ = writeln!(
        t,
        "\n[[regime]]\ncode = {}\nlibelle = {}\narticle = {}",
        chaine(r.code.as_str()),
        chaine(&r.libelle),
        chaine(&r.article)
    );
    ligne_optionnelle(t, "alinea", r.alinea.as_deref());
    let _ = writeln!(t, "pages = {}", pages(&r.pages));
    ligne_optionnelle(t, "document", r.document.as_deref());
    let _ = writeln!(
        t,
        "deduction = {}\nexigibilite = {}\nsource_deduction = {}\nsource_exigibilite = {}\nsource_ligne_taxe = {}\na_valider = {}",
        chaine(r.deduction.nom()),
        chaine(r.exigibilite.nom()),
        chaine(&r.source_deduction),
        chaine(&r.source_exigibilite),
        chaine(&r.source_ligne_taxe),
        r.a_valider
    );
    ligne_optionnelle(t, "note", r.note.as_deref());
    ligne_optionnelle(t, "debut", r.debut.map(|d| d.to_string()).as_deref());
    ligne_optionnelle(t, "fin", r.fin.map(|d| d.to_string()).as_deref());
}
