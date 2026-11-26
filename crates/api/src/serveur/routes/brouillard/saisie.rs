//! Lecture et contrôle d'une saisie du brouillard (corps de la création et de la modification).
//!
//! # Ce qui est refusé dès la saisie
//! Tout ce qui est **invalide en soi** : forme (400, sans écho), montant négatif ou nul, compte
//! inexistant, inactif ou de regroupement, journal inconnu ou sans saisie, date de valeur non
//! antérieure, tiers inconnu, inactif ou sur un compte hors de sa fiche, étiquette fiscale
//! incohérente (422, un code par motif et la position de la ligne). Un brouillon peut en
//! revanche être **déséquilibré** et sans justification : ces contrôles dépendent de
//! l'ensemble de l'écriture et se font à la validation (`EcritureEquilibree`, PR 13b).
//!
//! # Règle de la fiche du tiers
//! Un tiers peut porter **n'importe quel compte de sa fiche** (marque `tiers` du référentiel
//! copié, héritée par les descendants : [`types_de_tiers_admis`]), jamais un compte d'une autre
//! fiche. Un fournisseur passe sur 4011, 4091 ou 4081 (fiche 40), pas sur 411 (fiche 41) ni
//! sur un compte sans fiche.

use std::sync::OnceLock;

use compta_core::{
    CodeJournal, CodeMotif, CodeRegime, Cours, Devise, DeviseTenue, EcritureBrouillon,
    ErreurEtiquette, ErreurMontant, ErreurPlan, EtiquetteFiscale, Exigibilite, IdTiers, Ligne,
    Montant, NatureEtiquette, NumeroCompte, Sens, SensTva, TauxTva, TexteCourt,
};
use fiscal_sn::ReferentielTva;
use serde::Deserialize;
use time::{Date, Month};
use uuid::Uuid;

use crate::base::ErreurBase;
use crate::base::brouillard::{ContenuStocke, LIGNES_MAX, LigneStockee, RefusSaisie};
use crate::base::referentiel::{ReferentielDossier, types_de_tiers_admis};
use crate::serveur::erreur::ErreurApi;

/// Référentiel de TVA de `fiscal-sn`, chargé et validé une fois (le serveur le charge au
/// démarrage et refuse de démarrer s'il est invalide).
///
/// # Errors
/// [`ErreurBase::Donnee`] si les données embarquées sont invalides.
pub fn referentiel_tva() -> Result<&'static ReferentielTva, ErreurBase> {
    static TVA: OnceLock<Option<ReferentielTva>> = OnceLock::new();
    TVA.get_or_init(|| ReferentielTva::charger().ok())
        .as_ref()
        .ok_or(ErreurBase::Donnee("référentiel de TVA invalide"))
}

/// Corps de la création et de la modification (`version` : modification seulement).
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CorpsContenu {
    pub version: Option<i32>,
    journal: String,
    date: String,
    libelle: String,
    date_valeur_comptable: Option<String>,
    lignes: Vec<CorpsLigne>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CorpsLigne {
    compte: String,
    sens: String,
    montant: Option<String>,
    devise: Option<CorpsDevise>,
    tiers: Option<String>,
    etiquette: Option<CorpsEtiquette>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CorpsDevise {
    code: String,
    montant: String,
    cours: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CorpsEtiquette {
    nature: String,
    sens: String,
    regime: String,
    taux: Option<String>,
    exigibilite: Option<String>,
    motif: Option<String>,
}

/// Date `AAAA-MM-JJ` stricte.
pub(crate) fn date_lue(texte: &str) -> Option<Date> {
    let o = texte.as_bytes();
    let forme = o.len() == 10
        && o[4] == b'-'
        && o[7] == b'-'
        && o.iter()
            .enumerate()
            .all(|(i, b)| i == 4 || i == 7 || b.is_ascii_digit());
    if !forme {
        return None;
    }
    let mois = Month::try_from(texte.get(5..7)?.parse::<u8>().ok()?).ok()?;
    Date::from_calendar_date(
        texte.get(0..4)?.parse().ok()?,
        mois,
        texte.get(8..10)?.parse().ok()?,
    )
    .ok()
}

/// Contenu contrôlé : l'écriture de `compta-core` et sa forme stockée.
pub(super) struct Controle {
    pub contenu: ContenuStocke,
}

fn refus(refus: RefusSaisie, ligne: Option<usize>) -> ErreurApi {
    ErreurApi::Saisie {
        refus,
        ligne: ligne.and_then(|l| u32::try_from(l).ok()),
    }
}

fn motif_etiquette(e: &ErreurEtiquette) -> RefusSaisie {
    match e {
        ErreurEtiquette::CodeInvalide(_) => RefusSaisie::EtiquetteCodeInvalide,
        ErreurEtiquette::TauxInvalide(_) => RefusSaisie::EtiquetteTauxInvalide,
        ErreurEtiquette::RegimeInconnu(_) => RefusSaisie::EtiquetteRegimeInconnu,
        ErreurEtiquette::MotifManquant(_) => RefusSaisie::EtiquetteMotifManquant,
        ErreurEtiquette::MotifInterdit(_) => RefusSaisie::EtiquetteMotifInterdit,
        ErreurEtiquette::MotifHorsRegime { .. } => RefusSaisie::EtiquetteMotifHorsRegime,
        ErreurEtiquette::TaxeAvecMotif => RefusSaisie::EtiquetteTaxeAvecMotif,
        ErreurEtiquette::LigneTaxeInterdite(_) => RefusSaisie::EtiquetteLigneTaxeInterdite,
        ErreurEtiquette::TauxManquant(_) => RefusSaisie::EtiquetteTauxManquant,
        ErreurEtiquette::TauxInterdit(_) => RefusSaisie::EtiquetteTauxInterdit,
        ErreurEtiquette::TauxNonAdmis { .. } => RefusSaisie::EtiquetteTauxNonAdmis,
        ErreurEtiquette::ExigibiliteManquante(_) => RefusSaisie::EtiquetteExigibiliteManquante,
        ErreurEtiquette::ExigibiliteNonAdmise(_) => RefusSaisie::EtiquetteExigibiliteNonAdmise,
    }
}

fn etiquette(
    c: &CorpsEtiquette,
    tva: &ReferentielTva,
    ligne: usize,
) -> Result<EtiquetteFiscale, ErreurApi> {
    let invalide = ErreurApi::RequeteInvalide;
    let sur = |e: &ErreurEtiquette| refus(motif_etiquette(e), Some(ligne));
    let nature = NatureEtiquette::depuis_code(&c.nature).ok_or(invalide)?;
    let sens = SensTva::depuis_code(&c.sens).ok_or(invalide)?;
    let regime = CodeRegime::parse(&c.regime).map_err(|e| sur(&e))?;
    let taux = c
        .taux
        .as_deref()
        .map(TauxTva::parse)
        .transpose()
        .map_err(|e| sur(&e))?;
    let exigibilite = c
        .exigibilite
        .as_deref()
        .map(|x| Exigibilite::depuis_code(x).ok_or(invalide))
        .transpose()?;
    let motif = c
        .motif
        .as_deref()
        .map(CodeMotif::parse)
        .transpose()
        .map_err(|e| sur(&e))?;
    EtiquetteFiscale::nouvelle(nature, sens, regime, taux, exigibilite, motif, tva)
        .map_err(|e| sur(&e))
}

/// Borne de la base (`CHECK` de la migration `0202` : strictement moins de 1e20) : une valeur de
/// plus de 20 chiffres entiers est refusée en saisie (422), non par une erreur de la base (500).
fn hors_borne(valeur: &str) -> bool {
    valeur
        .trim_start_matches('-')
        .split('.')
        .next()
        .is_some_and(|entier| entier.trim_start_matches('0').len() > 20)
}

fn ligne(
    c: &CorpsLigne,
    position: usize,
    referentiel: &ReferentielDossier,
    tva: &ReferentielTva,
    tenue: &DeviseTenue,
) -> Result<Ligne, ErreurApi> {
    let numero = NumeroCompte::parse(&c.compte)
        .map_err(|_| refus(RefusSaisie::CompteInconnu, Some(position)))?;
    let etat = referentiel.etat();
    etat.plan.compte_de_mouvement_actif(&numero).map_err(|e| {
        refus(
            match e {
                ErreurPlan::Inactif(_) => RefusSaisie::CompteInactif,
                ErreurPlan::Regroupement(_) => RefusSaisie::CompteDeRegroupement,
                _ => RefusSaisie::CompteInconnu,
            },
            Some(position),
        )
    })?;
    let sens = match c.sens.as_str() {
        "debit" => Sens::Debit,
        "credit" => Sens::Credit,
        _ => return Err(ErreurApi::RequeteInvalide),
    };
    let montant_invalide = || refus(RefusSaisie::MontantInvalide, Some(position));
    let mut ligne = match (&c.montant, &c.devise) {
        (Some(m), None) => {
            // Échelle de la devise de tenue : plus de décimales que l'échelle est refusé
            // (ADR 0027), un format invalide reste un montant invalide.
            let montant = Montant::parse_tenue(m, tenue).map_err(|e| match e {
                ErreurMontant::EchelleExcessive { .. } => {
                    refus(RefusSaisie::MontantEchelleTenue, Some(position))
                }
                _ => montant_invalide(),
            })?;
            if hors_borne(&montant.valeur().to_string()) {
                return Err(montant_invalide());
            }
            Ligne::en_devise_tenue(numero.clone(), sens, montant, tenue)
                .map_err(|_| montant_invalide())?
        }
        (None, Some(d)) => {
            let devise = Devise::parse(&d.code).map_err(|_| montant_invalide())?;
            let montant = Montant::parse(&d.montant, devise).map_err(|_| montant_invalide())?;
            let cours = Cours::parse(&d.cours).map_err(|_| montant_invalide())?;
            if hors_borne(&montant.valeur().to_string()) || hors_borne(&cours.valeur().to_string())
            {
                return Err(montant_invalide());
            }
            let ligne = Ligne::en_devise(numero.clone(), sens, montant, cours, tenue)
                .map_err(|_| montant_invalide())?;
            // Contre-valeur en devise de tenue calculée par le moteur.
            if hors_borne(&ligne.montant().valeur().to_string()) {
                return Err(montant_invalide());
            }
            ligne
        }
        _ => return Err(ErreurApi::RequeteInvalide),
    };
    if let Some(texte) = &c.tiers {
        let id = Uuid::try_parse(texte).map_err(|_| ErreurApi::RequeteInvalide)?;
        let tiers = etat
            .tiers
            .get(&id)
            .ok_or_else(|| refus(RefusSaisie::TiersInconnu, Some(position)))?;
        if !tiers.est_actif() {
            return Err(refus(RefusSaisie::TiersInactif, Some(position)));
        }
        let admis = types_de_tiers_admis(etat, &numero).is_some_and(|t| t.contains(&tiers.genre()));
        if !admis {
            return Err(refus(RefusSaisie::CompteHorsFicheDuTiers, Some(position)));
        }
        let id_tiers = IdTiers::parse(&id.to_string()).map_err(|_| ErreurApi::RequeteInvalide)?;
        ligne = ligne.avec_tiers(id_tiers);
    }
    if let Some(e) = &c.etiquette {
        ligne = ligne.avec_etiquette(etiquette(e, tva, position)?);
    }
    Ok(ligne)
}

/// Contrôle une saisie contre le référentiel du dossier relu et celui de la TVA.
///
/// # Errors
/// 400 pour une forme invalide, 422 pour une saisie invalide en soi (un code par motif).
pub(super) fn controler(
    corps: &CorpsContenu,
    referentiel: &ReferentielDossier,
    tva: &ReferentielTva,
    tenue: &DeviseTenue,
) -> Result<Controle, ErreurApi> {
    let libelle = TexteCourt::parse(&corps.libelle).map_err(|_| ErreurApi::RequeteInvalide)?;
    let date = date_lue(&corps.date).ok_or(ErreurApi::RequeteInvalide)?;
    let date_valeur = corps
        .date_valeur_comptable
        .as_deref()
        .map(|d| date_lue(d).ok_or(ErreurApi::RequeteInvalide))
        .transpose()?;
    let code = CodeJournal::parse(&corps.journal).map_err(|_| ErreurApi::RequeteInvalide)?;
    let journal = referentiel
        .etat()
        .journaux
        .journal(&code)
        .ok_or_else(|| refus(RefusSaisie::JournalInconnu, None))?;
    if !journal.genre().accepte_la_saisie() {
        return Err(refus(RefusSaisie::JournalSansSaisie, None));
    }
    let journal_id = referentiel
        .id_journal(&code)
        .ok_or_else(|| refus(RefusSaisie::JournalInconnu, None))?;
    if date_valeur.is_some_and(|v| v >= date) {
        return Err(refus(RefusSaisie::DateValeurNonAnterieure, None));
    }
    if corps.lignes.len() > LIGNES_MAX {
        return Err(refus(RefusSaisie::TropDeLignes, None));
    }
    let mut ecriture = EcritureBrouillon::nouvelle(code, date, libelle.clone());
    ecriture.mentionner_date_valeur_comptable(date_valeur);
    let mut lignes = Vec::with_capacity(corps.lignes.len());
    for (i, c) in corps.lignes.iter().enumerate() {
        let l = ligne(c, i + 1, referentiel, tva, tenue)?;
        lignes.push(LigneStockee::depuis(&l)?);
        ecriture.ajouter_ligne(l);
    }
    Ok(Controle {
        contenu: ContenuStocke {
            journal_id,
            journal: corps.journal.clone(),
            date: ecriture.date(),
            date_valeur_comptable: ecriture.date_valeur_comptable(),
            libelle: libelle.as_str().to_owned(),
            lignes,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_strictes() {
        assert!(date_lue("2026-11-19").is_some());
        for refuse in [
            "2026-1-19",
            "2026-13-01",
            "2026-02-30",
            "26-11-19",
            "2026/11/19",
            "",
        ] {
            assert!(date_lue(refuse).is_none(), "{refuse}");
        }
    }

    #[test]
    fn la_borne_de_la_base_est_connue() {
        assert!(!hors_borne("99999999999999999999"));
        assert!(!hors_borne("99999999999999999999.99"));
        assert!(hors_borne("100000000000000000000"));
        assert!(hors_borne("100000000000000000000.5"));
        assert!(hors_borne("-100000000000000000000"));
        assert!(!hors_borne("0.5"));
    }

    #[test]
    fn le_referentiel_de_tva_se_charge() {
        assert!(referentiel_tva().is_ok());
    }
}
