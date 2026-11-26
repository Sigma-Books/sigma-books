//! Validation unitaire d'un brouillon et lecture des écritures validées (PR 13b de
//! `docs/lots/L2.md`, L2-26).
//!
//! # Ce qui est dans `compta-core`, ce qui est ici
//! **Aucune règle de validation n'est écrite dans `api`** : équilibre et nombre de lignes
//! ([`EcritureBrouillon::valider_equilibre`]), période, exercice, échéance de clôture, opération
//! tardive, comptes, tiers, justification, journal de trésorerie, numéro suivant
//! ([`compta_core::valider`]). `api` rassemble les faits (relus après les verrous), traduit les
//! refus en codes stables, produit le contenu canonique et la chaîne avec `integrite`, et
//! persiste. Deux contrôles de forme lui reviennent : l'échelle des montants (voir
//! [`echelle_excessive`]) et l'unicité des empreintes de pièces.
//!
//! # Fonction d'application pure
//! [`preparer`] puis [`decider`] (réunies dans [`appliquer`]) ne touchent pas à la base : leur
//! résultat sur une suite d'opérations est celui de [`compta_core::Livre`] (propriété de
//! `tests` ci-dessous). Le cas d'usage les sépare pour prendre la tête de chaîne et le compteur
//! **entre les deux**, dans l'ordre des verrous de `crate::base::validation`.
//!
//! # Échelle des montants (XOF)
//! Le XOF n'a pas de subdivision, mais les décimales sont **acceptées** à la saisie et conservées
//! exactes (décision du mainteneur, issue #184 : ni l'AUDCIF, ni le Guide, ni le CGI ne
//! l'interdisent). `compta-core` n'impose aucune échelle à une ligne en devise de tenue (`Ligne::en_devise_tenue`) :
//! seule la contre-valeur d'une ligne en devise est arrondie au franc. Le format canonique
//! (`integrite`) relit un nombre de 28 chiffres au plus (partie entière et décimales,
//! zéros finaux retirés). La validation refuse donc (422 `montant_echelle_excessive`, avec la
//! ligne) tout montant XOF, tout montant en devise et tout cours qui dépasserait ce nombre de
//! chiffres, puis s'assure que le contenu canonique **se relit à l'identique**
//! (422 `ecriture_non_representable` sinon). Un écart d'arrondi ou de change est équilibré par
//! une ligne explicite saisie par l'utilisateur, jamais ajoutée par le serveur.
//!
//! # Défense en profondeur
//! Le rôle (administrateur, valideur ; collaborateur seulement s'il a la délégation du dossier) et
//! la séparation des tâches (le créateur et les correcteurs ne valident pas, administrateur
//! compris, si le cabinet l'a activée) sont **relus en base** après le verrou du brouillon,
//! même si Cedar a décidé.

use authz::Role;
use axum::Json;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use compta_core::{
    Avertissement, CodeExercice, CodeJournal, CodeMotif, CodeRegime, Compteur, Contexte, Cours,
    Devise, DeviseTenue, EcritureBrouillon, EcritureEquilibree, EcritureValidee, ErreurEcriture,
    ErreurPlan, EtiquetteFiscale, Exercices, Exigibilite, IdTiers, Justification, Ligne, Montant,
    NatureEtiquette, NumeroCompte, NumeroEcriture, ReferencePiece, Sens, SensTva, StatutPeriode,
    TauxTva, TexteCourt, Validation, VersionParametres,
};
use fiscal_sn::ReferentielTva;
use integrite::{EcritureCanonique, Empreinte, chainer};
use serde::Serialize;
use time::Date;
use uuid::Uuid;

use super::{
    CorpsVersion, VueLigne, conflit, corps, referentiel_tva, sans_parametres, trace_de, vue_ligne,
};
use crate::base::brouillard::{
    self, BrouillonStocke, ContenuStocke, LigneStockee, RolePiece, decimal_canonique,
};
use crate::base::organisation::{lire_acces_dossier, lire_appartenance, lire_cabinet};
use crate::base::referentiel::charger;
use crate::base::validation::{
    self as base, Circonstances, EcritureStockee, Inscription, PieceVerrouillee, Tete,
};
use crate::base::{ErreurBase, TransactionTenant, ouvrir_transaction_lecture};
use crate::pieces::interne;
use crate::serveur::erreur::{CategorieErreur, ErreurApi, MotifConflit, MotifValidation};
use crate::serveur::pipeline::{DossierAutorise, Garde, id_chemin};
use crate::serveur::routes::{Requete, Resultat};

/// Nombre maximal de chiffres d'un nombre du format canonique (celui de `Montant::parse`).
const CHIFFRES_MAX: usize = 28;

const PAGE_PAR_DEFAUT: u32 = 50;
const PAGE_MAX: u32 = 100;

pub(super) fn refus(motif: MotifValidation, ligne: Option<usize>) -> ErreurApi {
    ErreurApi::Validation {
        motif,
        ligne: ligne.and_then(|l| u32::try_from(l + 1).ok()),
    }
}

pub(super) fn donnee() -> ErreurApi {
    ErreurApi::Interne(CategorieErreur::Donnee)
}

// --- Reconstitution d'une ligne ou d'une écriture ----------------------------------------

/// Ligne de `compta-core` rebâtie d'une ligne stockée (brouillon ou écriture validée).
///
/// # Errors
/// Donnée stockée qui n'a pas la forme d'une ligne (jamais produite par l'API).
pub fn ligne_depuis_stockee(
    l: &LigneStockee,
    tva: &ReferentielTva,
    tenue: &DeviseTenue,
) -> Result<Ligne, ErreurBase> {
    let incoherent = || ErreurBase::Donnee("ligne stockée incohérente");
    let compte = NumeroCompte::parse(&l.compte).map_err(|_| incoherent())?;
    let sens = match l.sens {
        "debit" => Sens::Debit,
        "credit" => Sens::Credit,
        _ => return Err(incoherent()),
    };
    let mut ligne = match &l.devise {
        None => {
            // Relecture : aucune échelle imposée (ADR 0020 point 6 ; ADR 0027 point 3).
            let montant = Montant::parse(&l.montant, tenue.devise()).map_err(|_| incoherent())?;
            Ligne::en_devise_tenue(compte, sens, montant, tenue).map_err(|_| incoherent())?
        }
        Some(d) => {
            let devise = Devise::parse(&d.code).map_err(|_| incoherent())?;
            let montant = Montant::parse(&d.montant, devise).map_err(|_| incoherent())?;
            let cours = Cours::parse(&d.cours).map_err(|_| incoherent())?;
            let ligne =
                Ligne::en_devise(compte, sens, montant, cours, tenue).map_err(|_| incoherent())?;
            // La contre-valeur stockée est celle que le moteur recalcule.
            if ligne.montant().valeur().normalize().to_string() != l.montant {
                return Err(incoherent());
            }
            ligne
        }
    };
    if let Some(t) = l.tiers {
        ligne = ligne.avec_tiers(IdTiers::parse(&t.to_string()).map_err(|_| incoherent())?);
    }
    if let Some(e) = &l.etiquette {
        let nature = NatureEtiquette::depuis_code(&e.nature).ok_or_else(incoherent)?;
        let sens = SensTva::depuis_code(&e.sens).ok_or_else(incoherent)?;
        let regime = CodeRegime::parse(&e.regime).map_err(|_| incoherent())?;
        let taux = e
            .taux
            .as_deref()
            .map(TauxTva::parse)
            .transpose()
            .map_err(|_| incoherent())?;
        let exigibilite = e
            .exigibilite
            .as_deref()
            .map(|x| Exigibilite::depuis_code(x).ok_or_else(incoherent))
            .transpose()?;
        let motif = e
            .motif
            .as_deref()
            .map(CodeMotif::parse)
            .transpose()
            .map_err(|_| incoherent())?;
        ligne = ligne.avec_etiquette(
            EtiquetteFiscale::nouvelle(nature, sens, regime, taux, exigibilite, motif, tva)
                .map_err(|_| incoherent())?,
        );
    }
    Ok(ligne)
}

/// Brouillon de `compta-core` rebâti d'un contenu stocké et des empreintes de ses pièces.
#[allow(clippy::too_many_arguments)]
fn brouillon_de(
    code_journal: &str,
    date: Date,
    date_valeur: Option<Date>,
    libelle: &str,
    version_parametres: Option<&str>,
    lignes: &[LigneStockee],
    empreintes: &[String],
    tva: &ReferentielTva,
    tenue: &DeviseTenue,
) -> Result<EcritureBrouillon, ErreurBase> {
    let incoherent = || ErreurBase::Donnee("écriture stockée incohérente");
    let journal = CodeJournal::parse(code_journal).map_err(|_| incoherent())?;
    let libelle = TexteCourt::parse(libelle).map_err(|_| incoherent())?;
    let mut e = EcritureBrouillon::nouvelle(journal, date, libelle);
    e.mentionner_date_valeur_comptable(date_valeur);
    for l in lignes {
        e.ajouter_ligne(ligne_depuis_stockee(l, tva, tenue)?);
    }
    if !empreintes.is_empty() {
        let refs = empreintes
            .iter()
            .map(|h| ReferencePiece::parse(h))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| incoherent())?;
        e.justifier(Justification::pieces(refs).map_err(|_| incoherent())?);
    }
    if let Some(v) = version_parametres {
        e.utiliser_parametres(VersionParametres::parse(v).map_err(|_| incoherent())?);
    }
    Ok(e)
}

/// Relecture d'une écriture validée : l'[`EcritureValidee`] que `compta-core` redonne
/// ([`EcritureValidee::reconstituer`]) depuis les colonnes, de contenu canonique identique à
/// celui qui a été haché.
///
/// # Errors
/// Donnée relue incohérente.
pub fn reconstituer(
    e: &EcritureStockee,
    tva: &ReferentielTva,
    tenue: &DeviseTenue,
) -> Result<EcritureValidee, ErreurBase> {
    let empreintes: Vec<String> = e.pieces.iter().map(|p| p.empreinte.clone()).collect();
    reconstituer_colonnes(
        &ColonnesValidee {
            numero: &e.numero,
            journal: &e.journal,
            date: e.date,
            date_valeur_comptable: e.date_valeur_comptable,
            libelle: &e.libelle,
            version_parametres: e.version_parametres.as_deref(),
            date_validation: e.date_validation,
            origine_numero: e.origine_numero.as_deref(),
            lignes: &e.lignes,
        },
        &empreintes,
        tva,
        tenue,
    )
}

/// Colonnes d'une écriture validée nécessaires à sa reconstitution (sans chaîne ni pièces).
#[derive(Debug, Clone, Copy)]
pub struct ColonnesValidee<'a> {
    /// Numéro `AC-2026-000231`.
    pub numero: &'a str,
    /// Code du journal.
    pub journal: &'a str,
    /// Date comptable.
    pub date: Date,
    /// Date de valeur comptable.
    pub date_valeur_comptable: Option<Date>,
    /// Libellé.
    pub libelle: &'a str,
    /// Version des paramètres fiscaux.
    pub version_parametres: Option<&'a str>,
    /// Date de validation.
    pub date_validation: Date,
    /// Numéro de l'écriture contre-passée, pour une contre-passation.
    pub origine_numero: Option<&'a str>,
    /// Lignes.
    pub lignes: &'a [LigneStockee],
}

/// Reconstitution depuis les seules colonnes utiles à une édition (les pièces, si elles sont
/// fournies, rétablissent la justification).
///
/// # Errors
/// Donnée relue incohérente.
pub fn reconstituer_colonnes(
    c: &ColonnesValidee<'_>,
    empreintes: &[String],
    tva: &ReferentielTva,
    tenue: &DeviseTenue,
) -> Result<EcritureValidee, ErreurBase> {
    let incoherent = || ErreurBase::Donnee("écriture validée incohérente");
    let brouillon = brouillon_de(
        c.journal,
        c.date,
        c.date_valeur_comptable,
        c.libelle,
        c.version_parametres,
        c.lignes,
        empreintes,
        tva,
        tenue,
    )?;
    let equilibree = brouillon.valider_equilibre().map_err(|_| incoherent())?;
    let numero = NumeroEcriture::parse(c.numero).map_err(|_| incoherent())?;
    let origine = c
        .origine_numero
        .map(NumeroEcriture::parse)
        .transpose()
        .map_err(|_| incoherent())?;
    EcritureValidee::reconstituer(equilibree, numero, c.date_validation, origine)
        .map_err(|_| incoherent())
}

/// Écriture équilibrée d'un brouillon relu, ou `None` s'il n'est pas (encore) équilibré : les
/// éditions provisoires ne retiennent que le brouillard équilibré.
///
/// # Errors
/// Donnée relue incohérente (journal, texte, ligne).
pub fn equilibree_du_brouillon(
    c: &ContenuStocke,
    tva: &ReferentielTva,
    tenue: &DeviseTenue,
) -> Result<Option<EcritureEquilibree>, ErreurBase> {
    let brouillon = brouillon_de(
        &c.journal,
        c.date,
        c.date_valeur_comptable,
        &c.libelle,
        None,
        &c.lignes,
        &[],
        tva,
        tenue,
    )?;
    Ok(brouillon.valider_equilibre().ok())
}

// --- Fonction d'application pure ----------------------------------------------------------

/// Vrai si un nombre (texte décimal) dépasse les [`CHIFFRES_MAX`] chiffres du format canonique,
/// zéros finaux de la partie décimale retirés.
#[must_use]
pub fn echelle_excessive(texte: &str) -> bool {
    let canonique = decimal_canonique(texte);
    canonique.bytes().filter(u8::is_ascii_digit).count() > CHIFFRES_MAX
}

fn motif_ecriture(e: &ErreurEcriture) -> MotifValidation {
    match e {
        ErreurEcriture::Vide => MotifValidation::Vide,
        ErreurEcriture::UneSeuleLigne => MotifValidation::UneSeuleLigne,
        ErreurEcriture::DesequilibreTenue { .. } | ErreurEcriture::DesequilibreDevise { .. } => {
            MotifValidation::Desequilibree
        }
        _ => MotifValidation::EcritureInvalide,
    }
}

/// Écriture équilibrée, prête à valider, avec l'exercice de sa date s'il existe.
#[derive(Debug)]
pub struct Preparee {
    ecriture: EcritureEquilibree,
    exercice: Option<CodeExercice>,
}

impl Preparee {
    /// Journal de l'écriture.
    #[must_use]
    pub fn journal(&self) -> &CodeJournal {
        self.ecriture.journal()
    }

    /// Exercice qui contient la date de l'écriture, s'il y en a un.
    #[must_use]
    pub fn exercice(&self) -> Option<&CodeExercice> {
        self.exercice.as_ref()
    }
}

/// Première étape pure : rebâtit l'écriture, la rend équilibrée (`compta-core`), contrôle
/// l'échelle des montants et désigne l'exercice de sa date.
///
/// # Errors
/// 422 (déséquilibre, une seule ligne, échelle…) ; 500 si la donnée stockée est incohérente.
pub fn preparer(
    b: &BrouillonStocke,
    empreintes: &[String],
    tva: &ReferentielTva,
    tenue: &DeviseTenue,
    exercices: &Exercices,
) -> Result<Preparee, ErreurApi> {
    // Échelle des montants, avant de les relire en décimaux (le format canonique n'en relit
    // pas plus de 28 chiffres).
    for (i, l) in b.contenu.lignes.iter().enumerate() {
        let mut valeurs = vec![l.montant.as_str()];
        if let Some(d) = &l.devise {
            valeurs.push(d.montant.as_str());
            valeurs.push(d.cours.as_str());
        }
        if valeurs.into_iter().any(echelle_excessive) {
            return Err(refus(MotifValidation::MontantEchelleExcessive, Some(i)));
        }
        // Brouillon saisi avant le contexte du dossier, ou par un autre chemin : un montant de
        // ligne en devise de tenue ne dépasse pas l'échelle de cette devise (ADR 0027).
        if l.devise.is_none() && Montant::parse_tenue(&l.montant, tenue).is_err() {
            return Err(refus(MotifValidation::MontantEchelleTenue, Some(i)));
        }
    }
    let brouillon = brouillon_de(
        &b.contenu.journal,
        b.contenu.date,
        b.contenu.date_valeur_comptable,
        &b.contenu.libelle,
        None,
        &b.contenu.lignes,
        empreintes,
        tva,
        tenue,
    )
    .map_err(|_| donnee())?;
    let ecriture = brouillon
        .valider_equilibre()
        .map_err(|r| refus(motif_ecriture(r.erreur()), None))?;
    let exercice = exercices
        .exercice_contenant(ecriture.date())
        .map(|e| e.code().clone());
    Ok(Preparee { ecriture, exercice })
}

/// Résultat de la décision : la validation de `compta-core`, son contenu canonique et sa place
/// dans la chaîne.
#[derive(Debug)]
pub struct Decision {
    /// Preuve de validation (écriture numérotée et compteur suivant).
    pub validation: Validation,
    /// Contenu canonique haché.
    pub canonique: String,
    /// Version du format canonique (1 à 4).
    pub format: i16,
    /// Empreinte chaînée sur celle de la tête lue.
    pub empreinte: Empreinte,
    /// Rang attribué.
    pub rang: i64,
}

fn periode_cloturee(exercices: &Exercices, date: Date) -> bool {
    exercices
        .exercice_contenant(date)
        .and_then(|e| e.periode_contenant(date))
        .is_some_and(|p| p.statut() == StatutPeriode::Cloturee)
}

/// Traduit un refus de `compta-core` en motif stable.
pub(super) fn motif_validation(
    e: &compta_core::ErreurValidation,
    exercices: &Exercices,
) -> (MotifValidation, Option<usize>) {
    use compta_core::ErreurValidation as V;
    let motif = match e {
        V::JournalInconnu(_) => MotifValidation::JournalInconnu,
        V::JournalSansSaisie(_) => MotifValidation::JournalSansSaisie,
        V::DateHorsExercice(_) => MotifValidation::DateHorsExercice,
        V::ExerciceCloture(_) => MotifValidation::ExerciceCloture,
        V::ExerciceEnCloture(_) => MotifValidation::ExerciceEnCloture,
        V::PeriodeFermee(date) => {
            if periode_cloturee(exercices, *date) {
                MotifValidation::PeriodeCloturee
            } else {
                MotifValidation::PeriodeVerrouillee
            }
        }
        V::Compte { ligne, erreur } => {
            let motif = match erreur {
                ErreurPlan::Inactif(_) => MotifValidation::CompteInactif,
                ErreurPlan::Regroupement(_) => MotifValidation::CompteDeRegroupement,
                _ => MotifValidation::CompteInconnu,
            };
            return (motif, Some(*ligne));
        }
        V::TiersManquant { ligne, .. } => return (MotifValidation::TiersManquant, Some(*ligne)),
        V::JustificationManquante => MotifValidation::JustificationManquante,
        V::Equilibre(e) => motif_ecriture(e),
        V::Numerotation(_) => MotifValidation::SequenceEpuisee,
        V::DateValeurNonAnterieure { .. } => MotifValidation::DateValeurNonAnterieure,
        V::DateValeurDansPeriodeOuverte(_) => MotifValidation::DateValeurPeriodeOuverte,
        V::DateValeurDansPeriodeVerrouillee(_) => MotifValidation::DateValeurPeriodeVerrouillee,
        V::PeriodeAccueilVerrouillee(_) => MotifValidation::PeriodeAccueilVerrouillee,
        V::DateComptableTardive { .. } => MotifValidation::OperationTardiveMalDatee,
        // Compteur inadapté ou désynchronisé, contre-passation, calcul de date : impossibles ici.
        _ => MotifValidation::EcritureInvalide,
    };
    (motif, None)
}

/// Seconde étape pure : `compta_core::valider` avec le compteur lu sous verrou, contenu
/// canonique, empreinte chaînée sur la tête lue.
///
/// # Errors
/// 409 ou 422 selon le motif de `compta-core` ; 500 pour un cas impossible.
pub fn decider(
    p: Preparee,
    contexte: &Contexte<'_>,
    dernier: u32,
    tete: &Tete,
    date_validation: Date,
) -> Result<Decision, ErreurApi> {
    let Preparee { ecriture, exercice } = p;
    // Sans exercice, `valider` refuse la date avant d'examiner le compteur.
    let exercice_compteur = match exercice {
        Some(e) => e,
        None => CodeExercice::parse("0000").map_err(|_| donnee())?,
    };
    let compteur = Compteur::reprendre(ecriture.journal().clone(), exercice_compteur, dernier)
        .map_err(|_| refus(MotifValidation::SequenceEpuisee, None))?;
    let validation =
        compta_core::valider(ecriture, contexte, &compteur, date_validation).map_err(|r| {
            let (motif, ligne) = motif_validation(r.erreur(), contexte.exercices);
            refus(motif, ligne)
        })?;
    finaliser(validation, tete)
}

/// Contenu canonique d'une [`Validation`] et sa place dans la chaîne, après la tête lue :
/// commun à la validation, au lot et à la contre-passation.
///
/// # Errors
/// 422 si le contenu ne se relit pas à l'identique ; 500 pour un format inattendu.
pub(super) fn finaliser(validation: Validation, tete: &Tete) -> Result<Decision, ErreurApi> {
    let canonique = EcritureCanonique::depuis(validation.ecriture());
    let contenu_texte = canonique.serialiser();
    // Le contenu haché doit se relire à l'identique (format de `integrite`).
    if EcritureCanonique::lire(&contenu_texte).as_ref() != Ok(&canonique) {
        return Err(refus(MotifValidation::NonRepresentable, None));
    }
    // Version choisie par le contenu : v4 si la devise de tenue n'est pas le XOF sans décimale.
    let format = i16::from(canonique.version());
    let empreinte = chainer(&tete.empreinte, contenu_texte.as_bytes());
    Ok(Decision {
        validation,
        canonique: contenu_texte,
        format,
        empreinte,
        rang: tete.rang + 1,
    })
}

/// Fonction d'application complète sur des faits fournis (base simulée) : `dernier` donne la
/// dernière séquence d'un (journal, exercice). Même résultat que [`compta_core::Livre`].
///
/// # Errors
/// Voir [`preparer`] et [`decider`].
pub fn appliquer(
    b: &BrouillonStocke,
    empreintes: &[String],
    tva: &ReferentielTva,
    contexte: &Contexte<'_>,
    dernier: impl FnOnce(&CodeJournal, &CodeExercice) -> u32,
    tete: &Tete,
    date_validation: Date,
) -> Result<Decision, ErreurApi> {
    let p = preparer(b, empreintes, tva, &contexte.tenue, contexte.exercices)?;
    let d = match p.exercice() {
        Some(e) => dernier(p.journal(), e),
        None => 0,
    };
    decider(p, contexte, d, tete, date_validation)
}

// --- Cas d'usage : validation ------------------------------------------------------------

/// Rôle relu en base : administrateur et valideur valident ; un collaborateur seulement avec la
/// délégation du dossier ; l'auditeur jamais.
async fn exiger_valideur(
    tx: &mut TransactionTenant,
    dossier: &DossierAutorise,
) -> Result<(), ErreurApi> {
    let a = lire_appartenance(tx)
        .await?
        .filter(|a| a.actif)
        .ok_or(ErreurApi::Introuvable)?;
    if a.role == Role::Administrateur.nom() || a.role == Role::Valideur.nom() {
        return Ok(());
    }
    if a.role == Role::Collaborateur.nom() {
        let delegue = lire_acces_dossier(tx, dossier.id())
            .await?
            .is_some_and(|x| x.validation_deleguee);
        if delegue {
            return Ok(());
        }
    }
    Err(ErreurApi::Interdit)
}

/// Contrôle les pièces liées : toutes présentes et non écartées, empreintes distinctes.
pub(super) fn controler_pieces(
    b: &BrouillonStocke,
    p: &[PieceVerrouillee],
) -> Result<(), ErreurApi> {
    if p.len() != b.pieces.len() {
        return Err(donnee());
    }
    if p.iter().any(|x| x.ecartee) {
        return Err(conflit(MotifConflit::PieceEcartee));
    }
    let mut vues: Vec<&str> = p.iter().map(|x| x.empreinte.as_str()).collect();
    vues.sort_unstable();
    if vues.windows(2).any(|w| w[0] == w[1]) {
        return Err(refus(MotifValidation::EcritureInvalide, None));
    }
    Ok(())
}

/// Liens de pièces d'un brouillon avec leur rôle, dans l'ordre des pièces verrouillées.
pub(super) fn roles_des_pieces(
    b: &BrouillonStocke,
    pieces: &[PieceVerrouillee],
) -> Result<Vec<(Uuid, RolePiece)>, ErreurApi> {
    pieces
        .iter()
        .map(|p| {
            let role = b
                .pieces
                .iter()
                .find(|l| l.piece == p.id)
                .map(|l| l.role)
                .ok_or_else(donnee)?;
            Ok((p.id, role))
        })
        .collect()
}

#[derive(Debug, Serialize)]
pub(super) struct VueValidation {
    pub(super) id: String,
    pub(super) numero: String,
    pub(super) rang: i64,
    pub(super) empreinte: String,
    pub(super) format: i16,
    pub(super) date_validation: String,
    /// Avertissements de la validation (écriture validée malgré tout) : voir
    /// [`vues_avertissements`].
    pub(super) avertissements: Vec<VueAvertissement>,
}

/// Avertissement renvoyé avec une validation réussie (`compta-core`, [`Avertissement`]) : code
/// stable et détails utiles à l'utilisateur.
#[derive(Debug, Serialize)]
pub(super) struct VueAvertissement {
    code: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    periode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    echeance: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    journal: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    compte: Option<String>,
}

/// Code stable d'un avertissement (réponse et audit).
fn code_avertissement(a: &Avertissement) -> &'static str {
    match a {
        Avertissement::EcheanceDeClotureDepassee { .. } => "echeance_de_cloture_depassee",
        Avertissement::CompteTresorerieAbsent { .. } => "compte_tresorerie_absent",
    }
}

/// Donnée d'audit `avertissements` : les **codes** seulement (jamais de date, de compte ni de
/// montant), séparés par des virgules ; `None` sans avertissement. L'avertissement est dans la
/// trace d'audit, jamais dans l'écriture ni dans son contenu haché.
pub(super) fn donnee_audit_avertissements(
    avertissements: &[Avertissement],
) -> Option<(&'static str, String)> {
    (!avertissements.is_empty()).then(|| {
        let codes: Vec<&str> = avertissements.iter().map(code_avertissement).collect();
        ("avertissements", codes.join(","))
    })
}

/// Avertissements d'une validation, en vues stables.
pub(super) fn vues_avertissements(avertissements: &[Avertissement]) -> Vec<VueAvertissement> {
    avertissements
        .iter()
        .map(|a| match a {
            Avertissement::EcheanceDeClotureDepassee { periode, echeance } => VueAvertissement {
                code: code_avertissement(a),
                periode: Some(interne::date(*periode)),
                echeance: Some(interne::date(*echeance)),
                journal: None,
                compte: None,
            },
            Avertissement::CompteTresorerieAbsent { journal, compte } => VueAvertissement {
                code: code_avertissement(a),
                periode: None,
                echeance: None,
                journal: Some(journal.as_str().to_owned()),
                compte: Some(compte.as_str().to_owned()),
            },
        })
        .collect()
}

#[allow(clippy::too_many_lines)]
pub(super) async fn valider(
    tx: &mut TransactionTenant,
    garde: &Garde,
    dossier: &DossierAutorise,
    requete: &Requete<'_>,
) -> Result<Resultat, ErreurApi> {
    sans_parametres(requete)?;
    let id = id_chemin(requete.chemin, "brouillon")?;
    // La décision Cedar a porté sur ce brouillon, et sur lui seul.
    if garde.brouillon_du_chemin() != Some(id) {
        return Err(ErreurApi::Introuvable);
    }
    let CorpsVersion { version } = corps(requete.corps)?;
    let tva = referentiel_tva()?;
    let utilisateur = tx.contexte().id_utilisateur().uuid();

    // 1. Verrous, dans l'ordre de l'ADR 0011 et de l'ADR 0020 : calendrier, brouillon, comptes et
    //    tiers, pièces, tête, compteur ; puis la tête d'audit, prise par le pipeline à
    //    l'inscription des traces.
    base::verrouiller_calendrier(tx).await?;
    let courante = brouillard::verrouiller(tx, id)
        .await?
        .ok_or(ErreurApi::Introuvable)?;
    // Rôle et délégation relus **après** le verrou du brouillon : une délégation retirée pendant
    // l'attente du verrou est vue (ADR 0020, 5).
    exiger_valideur(tx, dossier).await?;
    if courante != version {
        return Err(conflit(MotifConflit::BrouillonPerime));
    }
    // 2. État relu après les verrous.
    let b = brouillard::lire(tx, id)
        .await?
        .ok_or(ErreurApi::Introuvable)?;
    // Comptes et tiers utilisés `FOR SHARE`, puis référentiel relu : un compte désactivé
    // concurremment attend la fin de la validation, ou est vu désactivé.
    let comptes: Vec<String> = b.contenu.lignes.iter().map(|l| l.compte.clone()).collect();
    let tiers: Vec<Uuid> = b.contenu.lignes.iter().filter_map(|l| l.tiers).collect();
    base::verrouiller_comptes_et_tiers(tx, &comptes, &tiers).await?;
    let referentiel = charger(tx).await?.ok_or(ErreurApi::Introuvable)?;
    let cabinet = lire_cabinet(tx).await?.ok_or(ErreurApi::Introuvable)?;
    // Séparation des tâches : même si Cedar se trompait (créateur et correcteurs).
    if cabinet.separation_taches && b.contributeurs.contains(&utilisateur) {
        return Err(ErreurApi::Interdit);
    }
    let ids: Vec<Uuid> = b.pieces.iter().map(|l| l.piece).collect();
    let pieces = base::verrouiller_pieces(tx, &ids).await?;
    controler_pieces(&b, &pieces)?;
    let empreintes: Vec<String> = pieces.iter().map(|p| p.empreinte.clone()).collect();

    let tenue = dossier.contexte().tenue()?;
    let preparee = preparer(&b, &empreintes, tva, &tenue, referentiel.exercices())?;
    let journal_id = referentiel
        .id_journal(preparee.journal())
        .ok_or_else(donnee)?;
    let exercice_id = preparee
        .exercice()
        .and_then(|e| referentiel.id_exercice(e))
        .ok_or_else(|| refus(MotifValidation::DateHorsExercice, None))?;

    // Contrôle d'abord, sans consommer de numéro : le refus d'un statut ne prend pas les
    // verrous de chaîne. Puis tête de chaîne, compteur : un seul ordre.
    let genese = integrite::genese(
        &integrite::IdDossier::parse(&dossier.id().uuid().to_string()).map_err(|_| donnee())?,
    );
    let tete = base::verrouiller_tete(tx, &genese).await?;
    // Instant relu **après** la tête : la validation a pu attendre, ou franchir minuit.
    let instant = requete.horloge.maintenant();
    let dernier = base::verrouiller_compteur(tx, journal_id, exercice_id).await?;
    let contexte = Contexte {
        plan: &referentiel.etat().plan,
        journaux: &referentiel.etat().journaux,
        exercices: referentiel.exercices(),
        delai_cloture: crate::base::referentiel::delai_cloture(),
        tenue,
    };
    // Date du jour dans le fuseau du dossier (jamais celle d'UTC, ADR 0027).
    let date_validation = dossier.contexte().date_du_jour(instant)?;
    let decision = decider(preparee, &contexte, dernier, &tete, date_validation)?;

    // 3. Persistance : écriture, lignes, liens ; le brouillon disparaît ; tête et compteur.
    let roles = roles_des_pieces(&b, &pieces)?;
    let numero = decision.validation.ecriture().numero().clone();
    let inscription = Inscription::depuis(
        &decision.validation,
        Circonstances {
            id,
            journal_id,
            exercice_id,
            canonique: decision.canonique.clone(),
            format: decision.format,
            empreinte: decision.empreinte,
            precedente: tete.empreinte,
            rang: decision.rang,
            valide_par: utilisateur,
            valide_le: instant,
            pieces: roles,
            origine: None,
        },
    )?;
    base::inserer(tx, &inscription).await?;
    base::supprimer_brouillon(tx, id).await?;
    base::mettre_a_jour_tete(tx, decision.rang, &decision.empreinte).await?;
    base::mettre_a_jour_compteur(
        tx,
        journal_id,
        exercice_id,
        decision.validation.compteur().dernier(),
    )
    .await?;

    // 4. Audit : qui, quoi, numéro, rang, empreinte ; jamais l'écriture (ADR 0014).
    let mut trace = trace_de(dossier, id, "validation");
    trace.donnees.push(("numero", numero.to_string()));
    trace.donnees.push(("rang", decision.rang.to_string()));
    trace
        .donnees
        .push(("empreinte", decision.empreinte.to_string()));
    trace.donnees.push(("format", decision.format.to_string()));
    trace.donnees.extend(donnee_audit_avertissements(
        decision.validation.avertissements(),
    ));
    let vue = VueValidation {
        id: id.to_string(),
        numero: numero.to_string(),
        rang: decision.rang,
        empreinte: decision.empreinte.to_string(),
        format: decision.format,
        date_validation: interne::date(date_validation),
        avertissements: vues_avertissements(decision.validation.avertissements()),
    };
    Ok(Resultat {
        reponse: (StatusCode::OK, Json(vue)).into_response(),
        traces: vec![trace],
    })
}

// --- Lecture -------------------------------------------------------------------------------

#[derive(Debug, Serialize)]
struct VueLien {
    piece: String,
    role: &'static str,
    empreinte: String,
}

#[derive(Debug, Serialize)]
struct VueEcriture {
    id: String,
    numero: String,
    journal: String,
    exercice: String,
    date: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    date_valeur_comptable: Option<String>,
    libelle: String,
    rang: i64,
    empreinte: String,
    empreinte_precedente: String,
    format: i16,
    valide_par: String,
    date_validation: String,
    valide_le: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    origine: Option<String>,
    /// Contre-passations de l'écriture, dans l'ordre de la chaîne (éventuellement plusieurs).
    contre_passations: Vec<String>,
    lignes: Vec<VueLigne>,
    pieces: Vec<VueLien>,
}

fn vue_ecriture(e: &EcritureStockee) -> VueEcriture {
    VueEcriture {
        id: e.id.to_string(),
        numero: e.numero.clone(),
        journal: e.journal.clone(),
        exercice: e.exercice.clone(),
        date: interne::date(e.date),
        date_valeur_comptable: e.date_valeur_comptable.map(interne::date),
        libelle: e.libelle.clone(),
        rang: e.rang,
        empreinte: e.empreinte.to_string(),
        empreinte_precedente: e.precedente.to_string(),
        format: e.format,
        valide_par: e.valide_par.to_string(),
        date_validation: interne::date(e.date_validation),
        valide_le: interne::instant(e.valide_le),
        origine: e.origine.map(|o| o.to_string()),
        contre_passations: e.contre_passations.clone(),
        lignes: e
            .lignes
            .iter()
            .enumerate()
            .map(|(i, l)| vue_ligne(i + 1, l))
            .collect(),
        pieces: e
            .pieces
            .iter()
            .map(|p| VueLien {
                piece: p.piece.to_string(),
                role: p.role.nom(),
                empreinte: p.empreinte.clone(),
            })
            .collect(),
    }
}

#[derive(Debug, Serialize)]
struct VueResumeEcriture {
    id: String,
    numero: String,
    date: String,
    libelle: String,
    rang: i64,
    empreinte: String,
    total: String,
}

#[derive(Debug, Serialize)]
struct VuePageEcritures {
    ecritures: Vec<VueResumeEcriture>,
    suivant: Option<String>,
}

/// Curseur `AAAA-MM-JJ_<numéro>` strict : date et numéro de la dernière écriture de la page
/// précédente.
fn curseur(texte: &str) -> Result<(Date, String), ErreurApi> {
    let (date, numero) = texte.split_once('_').ok_or(ErreurApi::RequeteInvalide)?;
    let date = super::saisie::date_lue(date).ok_or(ErreurApi::RequeteInvalide)?;
    NumeroEcriture::parse(numero).map_err(|_| ErreurApi::RequeteInvalide)?;
    Ok((date, numero.to_owned()))
}

fn filtre_liste(texte: Option<&str>) -> Result<(Option<(Date, String)>, u32), ErreurApi> {
    let mut apres = None;
    let mut limite = None;
    for paire in texte.unwrap_or("").split('&').filter(|p| !p.is_empty()) {
        let (cle, valeur) = paire.split_once('=').ok_or(ErreurApi::RequeteInvalide)?;
        let deja = match cle {
            "apres" => apres.replace(curseur(valeur)?).is_some(),
            "limite" => {
                let n = (!valeur.is_empty() && valeur.bytes().all(|b| b.is_ascii_digit()))
                    .then(|| valeur.parse::<u32>().ok())
                    .flatten()
                    .filter(|l| (1..=PAGE_MAX).contains(l))
                    .ok_or(ErreurApi::RequeteInvalide)?;
                limite.replace(n).is_some()
            }
            _ => return Err(ErreurApi::RequeteInvalide),
        };
        if deja {
            return Err(ErreurApi::RequeteInvalide);
        }
    }
    Ok((apres, limite.unwrap_or(PAGE_PAR_DEFAUT)))
}

pub(super) async fn lister(
    tx: &mut TransactionTenant,
    requete: &Requete<'_>,
) -> Result<Resultat, ErreurApi> {
    let (apres, limite) = filtre_liste(requete.parametres)?;
    let mut page = base::lister(
        tx,
        apres.as_ref().map(|(d, n)| (*d, n.as_str())),
        limite + 1,
    )
    .await?;
    let reste = page.len() > limite as usize;
    page.truncate(limite as usize);
    let suivant = page
        .last()
        .filter(|_| reste)
        .map(|p| format!("{}_{}", interne::date(p.date), p.numero));
    let vue = VuePageEcritures {
        ecritures: page
            .iter()
            .map(|p| VueResumeEcriture {
                id: p.id.to_string(),
                numero: p.numero.clone(),
                date: interne::date(p.date),
                libelle: p.libelle.clone(),
                rang: p.rang,
                empreinte: p.empreinte.to_string(),
                total: p.total.clone(),
            })
            .collect(),
        suivant,
    };
    Ok(Resultat::lecture(Json(vue).into_response()))
}

pub(super) async fn lire(
    tx: &mut TransactionTenant,
    requete: &Requete<'_>,
) -> Result<Resultat, ErreurApi> {
    sans_parametres(requete)?;
    let id = id_chemin(requete.chemin, "ecriture")?;
    // Instantané unique : en-tête, lignes et pièces d'un même état.
    let mut instantane = ouvrir_transaction_lecture(requete.pool, *tx.contexte()).await?;
    let e = base::lire(&mut instantane, id).await;
    instantane.annuler().await?;
    let e = e?.ok_or(ErreurApi::Introuvable)?;
    Ok(Resultat::lecture(Json(vue_ecriture(&e)).into_response()))
}

#[cfg(test)]
mod tests {
    fn tenue_xof() -> compta_core::DeviseTenue {
        compta_core::DeviseTenue::nouvelle(compta_core::Devise::parse("XOF").unwrap(), 0).unwrap()
    }

    use super::*;

    #[test]
    fn l_echelle_suit_le_format_canonique() {
        for ok in [
            "150000",
            "1000.505",
            "1000.50",
            "1234567890123456789.123456789",
            "0.000000000000000000000000001",
        ] {
            assert!(!echelle_excessive(ok), "{ok}");
        }
        // 29 chiffres.
        assert!(echelle_excessive("12345678901234567890.123456789"));
        assert!(echelle_excessive("0.0000000000000000000000000001"));
    }

    #[test]
    fn filtres_stricts() {
        let (a, l) = filtre_liste(Some("apres=2026-01-31_AC-2026-000003&limite=10")).unwrap();
        assert_eq!(a.unwrap().1, "AC-2026-000003");
        assert_eq!(l, 10);
        assert_eq!(filtre_liste(None).unwrap().1, PAGE_PAR_DEFAUT);
        for refuse in [
            "apres=2026-01-31",
            "apres=2026-01-31_x",
            "limite=0",
            "limite=101",
            "autre=1",
            "limite=1&limite=2",
        ] {
            assert!(filtre_liste(Some(refuse)).is_err(), "{refuse}");
        }
    }

    // --- Parité avec `Livre` (propriété) -----------------------------------------------------

    use std::collections::BTreeMap;

    use compta_core::{Livre, Refus};
    use proptest::prelude::*;
    use time::macros::date;

    use crate::base::referentiel::{Instanciation, Modele, Ouverture};

    struct Decor {
        inst: Instanciation,
        /// Calendrier de `inst`, janvier à avril clôturés (opérations tardives).
        clos: Exercices,
        journaux: Vec<String>,
        caisse: String,
        banque: String,
    }

    fn decor() -> Decor {
        let modele = Modele::officiel().unwrap();
        let inst = Instanciation::preparer(
            modele,
            Ouverture::Reprise { annee: 2026 },
            date!(2026 - 11 - 20),
        )
        .unwrap();
        let etat = inst.etat();
        let caisse = etat
            .journaux
            .journaux()
            .find_map(|j| j.compte_tresorerie().cloned())
            .unwrap();
        let banque = etat
            .plan
            .comptes()
            .filter(|c| etat.plan.compte_de_mouvement_actif(c.numero()).is_ok())
            .find(|c| c.est_admis_en_journal_de_tresorerie() && *c.numero() != caisse)
            .unwrap()
            .numero()
            .clone();
        let journaux = etat
            .journaux
            .journaux()
            .map(|j| j.code().as_str().to_owned())
            .collect();
        let mut clos = inst.exercices().clone();
        for mois in [
            date!(2026 - 01 - 15),
            date!(2026 - 02 - 15),
            date!(2026 - 03 - 15),
            date!(2026 - 04 - 15),
        ] {
            clos.cloturer_periode(mois, []).unwrap();
        }
        Decor {
            inst,
            clos,
            journaux,
            caisse: caisse.as_str().to_owned(),
            banque: banque.as_str().to_owned(),
        }
    }

    fn stockee(sens: &'static str, compte: &str, montant: &str) -> LigneStockee {
        LigneStockee {
            compte: compte.to_owned(),
            sens,
            montant: montant.to_owned(),
            devise: None,
            tiers: None,
            etiquette: None,
        }
    }

    #[derive(Debug, Clone)]
    struct Operation {
        journal: usize,
        date: usize,
        validation: usize,
        ecart: u32,
        lignes: usize,
        piece: bool,
        /// 0 : aucune date de valeur ; 1 : opération tardive mal datée (10 mai) ; 2 : bien datée
        /// (1er mai), les périodes de janvier à avril étant clôturées.
        tardive: usize,
    }

    const DATES: [Date; 4] = [
        date!(2025 - 12 - 31),
        date!(2026 - 01 - 10),
        date!(2026 - 03 - 15),
        date!(2026 - 11 - 20),
    ];
    const VALIDATIONS: [Date; 3] = [
        date!(2026 - 01 - 20),
        date!(2026 - 06 - 15),
        date!(2026 - 11 - 20),
    ];

    fn operations() -> impl Strategy<Value = Vec<Operation>> {
        proptest::collection::vec(
            (
                0usize..12,
                0usize..4,
                0usize..3,
                0u32..3,
                1usize..4,
                any::<bool>(),
                0usize..3,
            )
                .prop_map(
                    |(journal, date, validation, ecart, lignes, piece, tardive)| Operation {
                        journal,
                        date,
                        validation,
                        ecart,
                        lignes,
                        piece,
                        tardive,
                    },
                ),
            1..25,
        )
    }

    #[test]
    fn un_cas_nominal_se_valide_et_chaine() {
        let d = decor();
        let etat = d.inst.etat();
        let contexte = Contexte {
            plan: &etat.plan,
            journaux: &etat.journaux,
            exercices: d.inst.exercices(),
            delai_cloture: crate::base::referentiel::delai_cloture(),
            tenue: tenue_xof(),
        };
        let journal = etat
            .journaux
            .journaux()
            .find(|j| j.genre().accepte_la_saisie() && j.compte_tresorerie().is_none())
            .unwrap()
            .code()
            .as_str()
            .to_owned();
        let b = BrouillonStocke {
            id: Uuid::nil(),
            contenu: ContenuStocke {
                journal_id: Uuid::nil(),
                journal,
                date: date!(2026 - 03 - 15),
                date_valeur_comptable: None,
                libelle: "Nominal".to_owned(),
                lignes: vec![
                    stockee("debit", &d.caisse, "1000"),
                    stockee("credit", &d.banque, "1000.00"),
                ],
            },
            a_revoir: false,
            a_revoir_par: None,
            version: 1,
            cree_par: Uuid::nil(),
            cree_le: time::OffsetDateTime::UNIX_EPOCH,
            modifie_par: Uuid::nil(),
            modifie_le: time::OffsetDateTime::UNIX_EPOCH,
            pieces: vec![],
            contributeurs: vec![],
        };
        let tete = Tete {
            rang: 4,
            empreinte: Empreinte::calculer(b"tete"),
        };
        let r = appliquer(
            &b,
            &["b".repeat(64)],
            referentiel_tva().unwrap(),
            &contexte,
            |_, _| 230,
            &tete,
            date!(2026 - 03 - 20),
        )
        .unwrap();
        assert!(
            r.validation
                .ecriture()
                .numero()
                .to_string()
                .ends_with("-2026-000231")
        );
        assert_eq!(r.rang, 5);
        assert_eq!(r.format, 1);
        assert_eq!(
            r.empreinte,
            chainer(&tete.empreinte, r.canonique.as_bytes())
        );
        assert!(r.canonique.contains("montant=1000\n"));
    }

    /// Applique un brouillon à deux lignes (`debit`, `credit`) dans un dossier tenu en `tenue`.
    fn appliquer_deux_lignes(
        tenue: DeviseTenue,
        debit: &str,
        credit: &str,
    ) -> Result<Decision, ErreurApi> {
        let d = decor();
        let etat = d.inst.etat();
        let contexte = Contexte {
            plan: &etat.plan,
            journaux: &etat.journaux,
            exercices: d.inst.exercices(),
            tenue,
        };
        let journal = etat
            .journaux
            .journaux()
            .find(|j| j.genre().accepte_la_saisie() && j.compte_tresorerie().is_none())
            .unwrap()
            .code()
            .as_str()
            .to_owned();
        let b = BrouillonStocke {
            id: Uuid::nil(),
            contenu: ContenuStocke {
                journal_id: Uuid::nil(),
                journal,
                date: date!(2026 - 03 - 15),
                date_valeur_comptable: None,
                libelle: "Échelle".to_owned(),
                lignes: vec![
                    stockee("debit", &d.caisse, debit),
                    stockee("credit", &d.banque, credit),
                ],
            },
            a_revoir: false,
            a_revoir_par: None,
            version: 1,
            cree_par: Uuid::nil(),
            cree_le: time::OffsetDateTime::UNIX_EPOCH,
            modifie_par: Uuid::nil(),
            modifie_le: time::OffsetDateTime::UNIX_EPOCH,
            pieces: vec![],
            contributeurs: vec![],
        };
        let tete = Tete {
            rang: 0,
            empreinte: Empreinte::calculer(b"tete"),
        };
        appliquer(
            &b,
            &["b".repeat(64)],
            referentiel_tva().unwrap(),
            &contexte,
            |_, _| 0,
            &tete,
            date!(2026 - 03 - 20),
        )
    }

    #[test]
    fn echelle_de_la_devise_de_tenue_a_la_validation() {
        let cdf = DeviseTenue::nouvelle(Devise::parse("CDF").unwrap(), 2).unwrap();
        // XOF : aucune décimale ; un brouillon saisi avec des centimes est refusé.
        let ko = appliquer_deux_lignes(tenue_xof(), "1000.5", "1000.5").unwrap_err();
        assert_eq!(
            ko,
            ErreurApi::Validation {
                motif: MotifValidation::MontantEchelleTenue,
                ligne: Some(1)
            }
        );
        // CDF : deux décimales, en format v4 (la devise de tenue est dans le contenu haché).
        let r = appliquer_deux_lignes(cdf, "1000.55", "1000.55").unwrap();
        assert_eq!(r.format, 4);
        assert!(r.canonique.starts_with("ecriture v4\n"));
        assert!(
            r.canonique
                .contains("devise-tenue=CDF\ndecimales-tenue=2\n")
        );
        assert!(r.canonique.contains("montant=1000.55\n"));
        let ko = appliquer_deux_lignes(cdf, "1000.555", "1000.555").unwrap_err();
        assert_eq!(
            ko,
            ErreurApi::Validation {
                motif: MotifValidation::MontantEchelleTenue,
                ligne: Some(1)
            }
        );
        // XOF : v1, octet pour octet comme avant le contexte du dossier.
        let r = appliquer_deux_lignes(tenue_xof(), "1000", "1000").unwrap();
        assert_eq!(r.format, 1);
        assert!(r.canonique.starts_with("ecriture v1\n"));
        assert!(!r.canonique.contains("devise-tenue"));
    }

    #[test]
    fn operation_tardive_bien_ou_mal_datee() {
        let d = decor();
        let etat = d.inst.etat();
        let contexte = Contexte {
            plan: &etat.plan,
            journaux: &etat.journaux,
            exercices: &d.clos,
            delai_cloture: crate::base::referentiel::delai_cloture(),
            tenue: tenue_xof(),
        };
        let journal = etat
            .journaux
            .journaux()
            .find(|j| j.genre().accepte_la_saisie() && j.compte_tresorerie().is_none())
            .unwrap()
            .code()
            .as_str()
            .to_owned();
        let tete = Tete {
            rang: 0,
            empreinte: Empreinte::calculer(b"tete"),
        };
        let essai = |date: Date| {
            let b = BrouillonStocke {
                id: Uuid::nil(),
                contenu: ContenuStocke {
                    journal_id: Uuid::nil(),
                    journal: journal.clone(),
                    date,
                    date_valeur_comptable: Some(date!(2026 - 01 - 15)),
                    libelle: "Tardive".to_owned(),
                    lignes: vec![
                        stockee("debit", &d.caisse, "1000"),
                        stockee("credit", &d.banque, "1000"),
                    ],
                },
                a_revoir: false,
                a_revoir_par: None,
                version: 1,
                cree_par: Uuid::nil(),
                cree_le: time::OffsetDateTime::UNIX_EPOCH,
                modifie_par: Uuid::nil(),
                modifie_le: time::OffsetDateTime::UNIX_EPOCH,
                pieces: vec![],
                contributeurs: vec![],
            };
            appliquer(
                &b,
                &["c".repeat(64)],
                referentiel_tva().unwrap(),
                &contexte,
                |_, _| 0,
                &tete,
                date!(2026 - 06 - 15),
            )
        };
        assert_eq!(
            essai(date!(2026 - 05 - 10)).unwrap_err(),
            refus(MotifValidation::OperationTardiveMalDatee, None)
        );
        let ok = essai(date!(2026 - 05 - 01)).unwrap();
        assert_eq!(ok.format, 2);
    }

    proptest! {
        /// Pour toute suite de validations, la fonction d'application rend les mêmes numéros,
        /// contenus canoniques et refus que `Livre` de `compta-core`, et chaîne comme
        /// `integrite::Chaine`.
        #[test]
        fn l_application_est_celle_du_livre(ops in operations()) {
            let d = decor();
            let tva = referentiel_tva().unwrap();
            let etat = d.inst.etat();
            let contexte = Contexte {
                plan: &etat.plan,
                journaux: &etat.journaux,
                exercices: &d.clos,
                delai_cloture: crate::base::referentiel::delai_cloture(),
                tenue: tenue_xof(),
            };
            let mut livre = Livre::new();
            let mut chaine = integrite::Chaine::ouvrir(&integrite::IdDossier::parse("d").unwrap());
            let mut derniers: BTreeMap<(CodeJournal, CodeExercice), u32> = BTreeMap::new();
            let mut tete = Tete { rang: 0, empreinte: *chaine.tete() };
            let empreinte_piece = "a".repeat(64);
            for op in ops {
                let journal = d.journaux[op.journal % d.journaux.len()].clone();
                let (date, date_valeur) = match op.tardive {
                    1 => (date!(2026 - 05 - 10), Some(date!(2026 - 01 - 15))),
                    2 => (date!(2026 - 05 - 01), Some(date!(2026 - 01 - 15))),
                    _ => (DATES[op.date], None),
                };
                let mut lignes = vec![
                    stockee("debit", &d.caisse, "1000"),
                    stockee("credit", &d.banque, "1000"),
                ];
                match op.lignes {
                    1 => { lignes.truncate(1); }
                    3 => { lignes.push(stockee("credit", &d.banque, &(1000 + op.ecart).to_string())); }
                    _ => {
                        if op.ecart > 0 {
                            lignes[1].montant = format!("{}", 1000 + op.ecart);
                        }
                    }
                }
                let b = BrouillonStocke {
                    id: Uuid::nil(),
                    contenu: ContenuStocke {
                        journal_id: Uuid::nil(),
                        journal,
                        date,
                        date_valeur_comptable: date_valeur,
                        libelle: "Parité".to_owned(),
                        lignes,
                    },
                    a_revoir: false,
                    a_revoir_par: None,
                    version: 1,
                    cree_par: Uuid::nil(),
                    cree_le: time::OffsetDateTime::UNIX_EPOCH,
                    modifie_par: Uuid::nil(),
                    modifie_le: time::OffsetDateTime::UNIX_EPOCH,
                    pieces: vec![],
                    contributeurs: vec![],
                };
                let empreintes: Vec<String> =
                    if op.piece { vec![empreinte_piece.clone()] } else { vec![] };
                let date_validation = VALIDATIONS[op.validation];

                let resultat = appliquer(
                    &b,
                    &empreintes,
                    tva,
                    &contexte,
                    |j, e| derniers.get(&(j.clone(), e.clone())).copied().unwrap_or(0),
                    &tete,
                    date_validation,
                );

                // Côté `Livre`.
                let brouillon = brouillon_de(
                    &b.contenu.journal, date, date_valeur, &b.contenu.libelle, None,
                    &b.contenu.lignes, &empreintes, tva, &tenue_xof(),
                ).unwrap();
                let attendu: Result<(NumeroEcriture, String), ErreurApi> = match brouillon.valider_equilibre() {
                    Err(r) => Err(refus(motif_ecriture(r.erreur()), None)),
                    Ok(eq) => {
                        let exercice = contexte.exercices.exercice_contenant(date).map(|e| e.code().clone());
                        let (cj, ce) = (
                            eq.journal().clone(),
                            exercice.clone().unwrap_or_else(|| CodeExercice::parse("0000").unwrap()),
                        );
                        let dernier = exercice
                            .as_ref()
                            .and_then(|e| derniers.get(&(cj.clone(), e.clone())).copied())
                            .unwrap_or(0);
                        let compteur = Compteur::reprendre(cj, ce, dernier).unwrap();
                        match livre.valider(eq, &contexte, &compteur, date_validation) {
                            Ok((numero, _)) => {
                                let v = livre.ecriture(&numero).unwrap();
                                Ok((numero, EcritureCanonique::depuis(v).serialiser()))
                            }
                            Err(r) => {
                                let (_, e): (EcritureEquilibree, _) = Refus::decomposer(r);
                                let (motif, ligne) = motif_validation(&e, contexte.exercices);
                                Err(refus(motif, ligne))
                            }
                        }
                    }
                };
                match (resultat, attendu) {
                    (Ok(d), Ok((numero, canonique))) => {
                        prop_assert_eq!(d.validation.ecriture().numero(), &numero);
                        prop_assert_eq!(&d.canonique, &canonique);
                        let e = livre.ecriture(&numero).unwrap();
                        let suivante = chaine.ajouter(e);
                        prop_assert_eq!(d.empreinte, suivante);
                        prop_assert_eq!(d.rang, i64::try_from(chaine.longueur()).unwrap());
                        derniers.insert(
                            (numero.journal().clone(), numero.exercice().clone()),
                            numero.sequence(),
                        );
                        tete = Tete { rang: d.rang, empreinte: d.empreinte };
                    }
                    (Err(a), Err(b)) => prop_assert_eq!(a, b),
                    (a, b) => prop_assert!(false, "divergence : {:?} contre {:?}", a.map(|d| d.canonique), b),
                }
            }
        }
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    fn une_contre_passation_nominale_se_numerote_et_accepte_la_repetition_et_une_date_anterieure() {
        use super::super::contre_passation::decider_contre_passation;
        let d = decor();
        let etat = d.inst.etat();
        let contexte = Contexte {
            plan: &etat.plan,
            journaux: &etat.journaux,
            exercices: d.inst.exercices(),
            delai_cloture: crate::base::referentiel::delai_cloture(),
            tenue: tenue_xof(),
        };
        let journal = etat
            .journaux
            .journaux()
            .find(|j| j.genre().accepte_la_saisie() && j.compte_tresorerie().is_none())
            .unwrap()
            .code()
            .as_str()
            .to_owned();
        let empreintes = ["d".repeat(64)];
        let brouillon = brouillon_de(
            &journal,
            date!(2026 - 03 - 15),
            None,
            "Origine",
            None,
            &[
                stockee("debit", &d.caisse, "1000"),
                stockee("credit", &d.banque, "1000"),
            ],
            &empreintes,
            referentiel_tva().unwrap(),
            &tenue_xof(),
        )
        .unwrap();
        let mut livre = Livre::new();
        let exercice = d
            .inst
            .exercices()
            .exercice_contenant(date!(2026 - 03 - 15))
            .unwrap()
            .code()
            .clone();
        let eq = brouillon.valider_equilibre().unwrap();
        let compteur = Compteur::reprendre(eq.journal().clone(), exercice, 0).unwrap();
        let (numero, _) = livre
            .valider(eq, &contexte, &compteur, date!(2026 - 03 - 16))
            .unwrap();
        let origine = livre.ecriture(&numero).unwrap().clone();
        let tete = Tete {
            rang: 1,
            empreinte: Empreinte::calculer(b"tete"),
        };
        let libelle = TexteCourt::parse("Contre-passation").unwrap();
        let r = decider_contre_passation(
            &origine,
            date!(2026 - 03 - 20),
            libelle.clone(),
            &contexte,
            1,
            &tete,
            date!(2026 - 03 - 20),
        )
        .unwrap();
        assert!(
            r.validation
                .ecriture()
                .numero()
                .to_string()
                .ends_with("-000002")
        );
        assert_eq!(r.validation.ecriture().origine(), Some(&numero));
        assert!(r.canonique.contains(&format!("origine={numero}\n")));
        // Une seconde contre-passation de la même origine aboutit (ADR 0021 point 4, révisé) :
        // numéro suivant du compteur.
        let seconde = decider_contre_passation(
            &origine,
            date!(2026 - 03 - 21),
            libelle.clone(),
            &contexte,
            2,
            &tete,
            date!(2026 - 03 - 21),
        )
        .unwrap();
        assert!(
            seconde
                .validation
                .ecriture()
                .numero()
                .to_string()
                .ends_with("-000003")
        );
        assert_eq!(seconde.validation.ecriture().origine(), Some(&numero));
        // Une date antérieure à l'origine est acceptée (aucun moteur ne l'interdit) : même
        // numérotation, même origine.
        let avant = decider_contre_passation(
            &origine,
            date!(2026 - 03 - 01),
            libelle,
            &contexte,
            3,
            &tete,
            date!(2026 - 03 - 20),
        )
        .unwrap();
        assert!(
            avant
                .validation
                .ecriture()
                .numero()
                .to_string()
                .ends_with("-000004")
        );
        assert_eq!(avant.validation.ecriture().origine(), Some(&numero));
        assert_eq!(avant.validation.ecriture().date(), date!(2026 - 03 - 01));
    }

    proptest! {
        /// Pour toute suite de validations et de contre-passations (dont des répétées, des dates
        /// avant l'origine, hors exercice ou en période clôturée), la fonction d'application de
        /// la contre-passation rend les mêmes numéros, contenus canoniques et refus que `Livre`
        /// de `compta-core`, et chaîne comme `integrite::Chaine`.
        #[test]
        fn la_contre_passation_est_celle_du_livre(
            pas in proptest::collection::vec(
                (0usize..12, 0usize..4, 0usize..3, any::<bool>(), 0usize..64, 0usize..4),
                1..30,
            )
        ) {
            use super::super::contre_passation::decider_contre_passation;
            let d = decor();
            let tva = referentiel_tva().unwrap();
            let etat = d.inst.etat();
            let contexte = Contexte {
                plan: &etat.plan,
                journaux: &etat.journaux,
                exercices: &d.clos,
                delai_cloture: crate::base::referentiel::delai_cloture(),
                tenue: tenue_xof(),
            };
            let mut livre = Livre::new();
            let mut chaine = integrite::Chaine::ouvrir(&integrite::IdDossier::parse("d").unwrap());
            let mut derniers: BTreeMap<(CodeJournal, CodeExercice), u32> = BTreeMap::new();
            let mut tete = Tete { rang: 0, empreinte: *chaine.tete() };
            let mut numeros: Vec<NumeroEcriture> = Vec::new();
            // Journaux de saisie sans compte de trésorerie : la plupart des validations passent.
            let saisie_libre: Vec<String> = etat
                .journaux
                .journaux()
                .filter(|j| j.genre().accepte_la_saisie() && j.compte_tresorerie().is_none())
                .map(|j| j.code().as_str().to_owned())
                .collect();
            let empreinte_piece = "a".repeat(64);
            let dernier_de = |derniers: &BTreeMap<(CodeJournal, CodeExercice), u32>,
                              j: &CodeJournal,
                              date: Date| {
                contexte
                    .exercices
                    .exercice_contenant(date)
                    .and_then(|e| derniers.get(&(j.clone(), e.code().clone())).copied())
                    .unwrap_or(0)
            };
            for (journal, date, validation, contre, cible, date_cp) in pas {
                let date_validation = VALIDATIONS[validation];
                if contre && !numeros.is_empty() {
                    let numero = numeros[cible % numeros.len()].clone();
                    let origine = livre.ecriture(&numero).unwrap().clone();
                    let date_cp = DATES[date_cp];
                    let libelle = TexteCourt::parse("Contre-passation").unwrap();
                    let dernier = dernier_de(&derniers, origine.journal(), date_cp);
                    let resultat = decider_contre_passation(
                        &origine,
                        date_cp,
                        libelle.clone(),
                        &contexte,
                        dernier,
                        &tete,
                        date_validation,
                    );
                    let exercice = contexte
                        .exercices
                        .exercice_contenant(date_cp)
                        .map_or_else(|| CodeExercice::parse("0000").unwrap(), |e| e.code().clone());
                    let compteur =
                        Compteur::reprendre(origine.journal().clone(), exercice, dernier).unwrap();
                    let attendu = livre
                        .contre_passer(&numero, date_cp, libelle, &contexte, &compteur, date_validation)
                        .map_err(|e| super::super::contre_passation::erreur(&e, contexte.exercices));
                    match (resultat, attendu) {
                        (Ok(r), Ok((nouveau, _))) => {
                            let e = livre.ecriture(&nouveau).unwrap();
                            prop_assert_eq!(e.origine(), Some(&numero));
                            prop_assert_eq!(r.validation.ecriture().numero(), &nouveau);
                            prop_assert_eq!(&r.canonique, &EcritureCanonique::depuis(e).serialiser());
                            let suivante = chaine.ajouter(e);
                            prop_assert_eq!(r.empreinte, suivante);
                            derniers.insert(
                                (nouveau.journal().clone(), nouveau.exercice().clone()),
                                nouveau.sequence(),
                            );
                            tete = Tete { rang: r.rang, empreinte: r.empreinte };
                            numeros.push(nouveau);
                        }
                        (Err(a), Err(b)) => prop_assert_eq!(a, b),
                        (a, b) => prop_assert!(false, "divergence : {:?} contre {:?}", a.map(|r| r.canonique), b.map(|(n, _)| n)),
                    }
                    continue;
                }
                // Validation d'une écriture équilibrée (journal, date tirés au hasard).
                let b = BrouillonStocke {
                    id: Uuid::nil(),
                    contenu: ContenuStocke {
                        journal_id: Uuid::nil(),
                        journal: saisie_libre[journal % saisie_libre.len()].clone(),
                        date: DATES[date],
                        date_valeur_comptable: None,
                        libelle: "Parité".to_owned(),
                        lignes: vec![
                            stockee("debit", &d.caisse, "1000"),
                            stockee("credit", &d.banque, "1000"),
                        ],
                    },
                    a_revoir: false,
                    a_revoir_par: None,
                    version: 1,
                    cree_par: Uuid::nil(),
                    cree_le: time::OffsetDateTime::UNIX_EPOCH,
                    modifie_par: Uuid::nil(),
                    modifie_le: time::OffsetDateTime::UNIX_EPOCH,
                    pieces: vec![],
                    contributeurs: vec![],
                };
                let empreintes = vec![empreinte_piece.clone()];
                let resultat = appliquer(
                    &b,
                    &empreintes,
                    tva,
                    &contexte,
                    |j, e| derniers.get(&(j.clone(), e.clone())).copied().unwrap_or(0),
                    &tete,
                    date_validation,
                );
                if let Ok(r) = resultat {
                    // Même moteur que `Livre` : on rejoue l'écriture dans le livre.
                    let brouillon = brouillon_de(
                        &b.contenu.journal, DATES[date], None, &b.contenu.libelle, None,
                        &b.contenu.lignes, &empreintes, tva, &tenue_xof(),
                    ).unwrap();
                    let eq = brouillon.valider_equilibre().unwrap();
                    let exercice = contexte.exercices.exercice_contenant(DATES[date]).unwrap().code().clone();
                    let dernier = derniers
                        .get(&(eq.journal().clone(), exercice.clone()))
                        .copied()
                        .unwrap_or(0);
                    let compteur = Compteur::reprendre(eq.journal().clone(), exercice, dernier).unwrap();
                    let (numero, _) = livre.valider(eq, &contexte, &compteur, date_validation).unwrap();
                    prop_assert_eq!(r.validation.ecriture().numero(), &numero);
                    let e = livre.ecriture(&numero).unwrap();
                    chaine.ajouter(e);
                    derniers.insert((numero.journal().clone(), numero.exercice().clone()), numero.sequence());
                    tete = Tete { rang: r.rang, empreinte: r.empreinte };
                    numeros.push(numero);
                }
            }
        }
    }
}
