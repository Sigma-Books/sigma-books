//! Référentiel d'un dossier en base : plan de comptes, journaux, exercices et périodes, tiers
//! minimal (L2-22, PR 11a ; migration `0200_referentiel.sql` ; note d'étude de la PR 11).
//!
//! # Stockage à plat
//! Une ligne par compte, par journal, par exercice, par période et par tiers. Les types de
//! `compta-core` se rechargent par des fonctions **pures** : [`aplatir_plan`] et
//! [`reconstituer_plan`], [`aplatir_journaux`] et [`reconstituer_journaux`] (propriété
//! `reconstituer(aplatir(x)) == x`, tests du module), `Exercices::aplatir` et
//! `Exercices::reconstituer` (`compta-core`). La reconstitution du plan rejoue les ajouts par
//! les fonctions publiques de `compta-core` et compare le sens et les caractères collectif et
//! de trésorerie recalculés aux colonnes relues : tout écart est une erreur de relecture.
//!
//! # Copie du référentiel officiel
//! À sa création, chaque dossier reçoit une **copie** du plan officiel de `syscohada` et des
//! journaux par défaut ([`Modele`]), avec la version du référentiel et une empreinte de ce qui
//! a été copié ([`Modele::empreinte`]) : une correction ultérieure des données de `syscohada` ne
//! change pas silencieusement les dossiers existants. Tout est calculé en pur
//! ([`Instanciation::preparer`]) avant la première écriture ([`instancier`]).
//!
//! # Une seule voie de mutation
//! Toute modification du plan, des journaux ou des tiers d'un dossier passe par [`muter`] :
//! verrou du référentiel, relecture, mutation pure d'une **copie**, contrôle croisé
//! ([`verifier`] : `Journaux::verifier_plan`, comptes de trésorerie distincts, comptes
//! collectifs des tiers), puis enregistrement des **seules différences** entre l'état relu et
//! la copie. Rien d'autre n'écrit dans ces tables après la création.
//!
//! # Verrou du référentiel
//! `SELECT … FROM dossier … FOR NO KEY UPDATE` ([`verrouiller`]) : deux mutations du référentiel
//! d'un même dossier se suivent. `NO KEY` n'entre pas en conflit avec le `FOR KEY SHARE` que
//! prend l'insertion d'une ligne enfant du dossier (audit, écritures) : elles ne l'attendent pas.
//!
//! Niveau « requête » de l'isolation (ADR 0011) : chaque requête filtre par le cabinet et le
//! dossier du contexte, en plus de la RLS.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use compta_core::{
    CodeExercice, CodeJournal, Compte, CompteReference, ErreurExercice, ErreurJournal, ErreurPlan,
    ErreurReconstitutionExercices, Exercices, FinPremierExercice, GenreCompte, Journal, Journaux,
    LigneExercice, LignePeriode, NumeroCompte, PlanComptes, SensNormal, StatutExercice,
    StatutPeriode, TexteCourt, TypeJournal,
};
use integrite::Empreinte;
use time::Date;
use uuid::Uuid;

use super::{ErreurBase, IdDossier, TransactionTenant};

// --- Tiers minimal (pur) -------------------------------------------------------------------

/// Type d'un tiers (spec §5.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum TypeTiers {
    /// Client.
    Client,
    /// Fournisseur.
    Fournisseur,
    /// Salarié.
    Salarie,
    /// Associé.
    Associe,
    /// Autre.
    Autre,
}

impl TypeTiers {
    /// Tous les types.
    pub const TOUS: [Self; 5] = [
        Self::Client,
        Self::Fournisseur,
        Self::Salarie,
        Self::Associe,
        Self::Autre,
    ];

    /// Nom stocké et échangé.
    #[must_use]
    pub const fn nom(self) -> &'static str {
        match self {
            Self::Client => "client",
            Self::Fournisseur => "fournisseur",
            Self::Salarie => "salarie",
            Self::Associe => "associe",
            Self::Autre => "autre",
        }
    }

    /// Type d'après son nom.
    #[must_use]
    pub fn parse(texte: &str) -> Option<Self> {
        Self::TOUS.into_iter().find(|t| t.nom() == texte)
    }
}

/// Tiers minimal (L2, question 6 a) : un nom, un rôle et un compte collectif. Un client et un
/// fournisseur sont deux tiers distincts, chacun sur son compte collectif (fiches 40 et 41,
/// AUDCIF art. 34 : aucune compensation). Le type et le compte collectif ne changent pas.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tiers {
    nom: TexteCourt,
    genre: TypeTiers,
    compte_collectif: NumeroCompte,
    actif: bool,
}

impl Tiers {
    /// Nouveau tiers, actif.
    #[must_use]
    pub fn nouveau(nom: TexteCourt, genre: TypeTiers, compte_collectif: NumeroCompte) -> Self {
        Self {
            nom,
            genre,
            compte_collectif,
            actif: true,
        }
    }

    /// Nom.
    #[must_use]
    pub fn nom(&self) -> &TexteCourt {
        &self.nom
    }

    /// Type.
    #[must_use]
    pub fn genre(&self) -> TypeTiers {
        self.genre
    }

    /// Compte collectif.
    #[must_use]
    pub fn compte_collectif(&self) -> &NumeroCompte {
        &self.compte_collectif
    }

    /// Vrai si le tiers est actif.
    #[must_use]
    pub fn est_actif(&self) -> bool {
        self.actif
    }

    /// Renomme le tiers.
    pub fn renommer(&mut self, nom: TexteCourt) {
        self.nom = nom;
    }

    /// Active ou désactive le tiers.
    pub fn definir_actif(&mut self, actif: bool) {
        self.actif = actif;
    }
}

// --- État pur et contrôle croisé -----------------------------------------------------------

/// Motif de refus d'une mutation du référentiel : la demande est contraire aux règles. Rendu
/// au client comme une requête invalide, sans détail.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RefusReferentiel {
    /// Règle du plan de comptes.
    #[error("plan de comptes : {0}")]
    Plan(#[from] ErreurPlan),
    /// Règle des journaux, dont le contrôle croisé du plan et des journaux.
    #[error("journaux : {0}")]
    Journal(#[from] ErreurJournal),
    /// Règle des exercices.
    #[error("exercices : {0}")]
    Exercice(#[from] ErreurExercice),
    /// Deux journaux de trésorerie sur le même compte (spec §5.2 : un journal par compte).
    #[error("le compte {0} est déjà le compte du journal {1}")]
    CompteTresorerieDejaUtilise(NumeroCompte, CodeJournal),
    /// Le compte collectif d'un tiers actif doit exister, être collectif et actif.
    #[error("le compte {0} ne peut pas être le compte collectif d'un tiers actif")]
    CompteDeTiers(NumeroCompte),
    /// Ressource inconnue dans le dossier.
    #[error("ressource inconnue")]
    Inconnu,
    /// La demande ne change rien.
    #[error("aucun changement")]
    AucunChangement,
    /// Année ou date d'ouverture refusée (dossier repris dans une année future, début
    /// d'activité trop lointain).
    #[error("date d'ouverture refusée")]
    DateRefusee,
    /// Valeur invalide (numéro, code, intitulé).
    #[error("valeur invalide")]
    Valeur,
}

/// Plan, journaux et tiers d'un dossier : la partie du référentiel que [`muter`] modifie.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EtatReferentiel {
    /// Plan de comptes.
    pub plan: PlanComptes,
    /// Journaux.
    pub journaux: Journaux,
    /// Tiers, par identifiant.
    pub tiers: BTreeMap<Uuid, Tiers>,
}

/// Contrôle croisé du référentiel (note d'étude §5), qu'aucun type seul ne garantit :
/// - le compte de chaque journal de trésorerie reste un compte de mouvement actif
///   (`Journaux::verifier_plan` : un sous-compte sous le compte du journal CA, ou sa
///   désactivation, le rendrait illisible) ;
/// - deux journaux de trésorerie n'ont pas le même compte (spec §5.2, « un par compte ») ;
/// - le compte collectif de chaque tiers actif existe, est collectif et actif.
///
/// Que le compte collectif d'un tiers puisse être un compte de regroupement dépend de la
/// question 4 de la note d'étude (PR 13) : il n'est pas contrôlé ici.
///
/// # Errors
/// [`RefusReferentiel`] pour la première incohérence.
pub fn verifier(etat: &EtatReferentiel) -> Result<(), RefusReferentiel> {
    etat.journaux.verifier_plan(&etat.plan)?;
    let mut comptes: BTreeMap<&NumeroCompte, &CodeJournal> = BTreeMap::new();
    for journal in etat.journaux.journaux() {
        if let Some(compte) = journal.compte_tresorerie()
            && let Some(autre) = comptes.insert(compte, journal.code())
        {
            return Err(RefusReferentiel::CompteTresorerieDejaUtilise(
                compte.clone(),
                autre.clone(),
            ));
        }
    }
    for tiers in etat.tiers.values().filter(|t| t.actif) {
        let utilisable = etat
            .plan
            .compte(&tiers.compte_collectif)
            .is_some_and(|c| c.est_collectif() && c.est_actif());
        if !utilisable {
            return Err(RefusReferentiel::CompteDeTiers(
                tiers.compte_collectif.clone(),
            ));
        }
    }
    Ok(())
}

// --- Lignes plates (pur) -------------------------------------------------------------------

/// Ligne plate d'un compte, telle que stockée (sans son identifiant).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LigneCompte {
    /// Numéro.
    pub numero: NumeroCompte,
    /// Intitulé.
    pub intitule: TexteCourt,
    /// Origine.
    pub genre: GenreCompte,
    /// Sens normal.
    pub sens: SensNormal,
    /// Collectif (lui-même ou par un ascendant).
    pub collectif: bool,
    /// Admis comme compte d'un journal de trésorerie.
    pub tresorerie: bool,
    /// Actif.
    pub actif: bool,
}

/// Ligne plate d'un journal, telle que stockée (sans son identifiant).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LigneJournal {
    /// Code.
    pub code: CodeJournal,
    /// Intitulé.
    pub intitule: TexteCourt,
    /// Type.
    pub genre: TypeJournal,
    /// Admis pendant la clôture de l'exercice.
    pub admis_en_cloture: bool,
    /// Compte de trésorerie.
    pub compte_tresorerie: Option<NumeroCompte>,
}

/// Lignes relues qui ne décrivent pas un référentiel atteignable.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ErreurRelecture {
    /// Plan non reconstituable.
    #[error("plan relu : {0}")]
    Plan(#[from] ErreurPlan),
    /// Journaux non reconstituables.
    #[error("journaux relus : {0}")]
    Journal(#[from] ErreurJournal),
    /// Exercices non reconstituables.
    #[error("exercices relus : {0}")]
    Exercices(#[from] ErreurReconstitutionExercices),
    /// Le sens ou un caractère recalculé d'un compte diffère de la ligne relue.
    #[error("compte {0} : attributs relus incohérents")]
    Incoherence(NumeroCompte),
}

/// Lignes plates du plan, par ordre de numéro.
#[must_use]
pub fn aplatir_plan(plan: &PlanComptes) -> Vec<LigneCompte> {
    plan.comptes()
        .map(|c| LigneCompte {
            numero: c.numero().clone(),
            intitule: c.intitule().clone(),
            genre: c.genre(),
            sens: c.sens(),
            collectif: c.est_collectif(),
            tresorerie: c.est_admis_en_journal_de_tresorerie(),
            actif: c.est_actif(),
        })
        .collect()
}

/// Reconstitue un plan relu, lignes dans un ordre quelconque, par les seules fonctions
/// publiques de `compta-core` : comptes officiels par `PlanComptes::depuis_referentiel`, puis
/// comptes créés par le dossier dans l'ordre des numéros (un parent, préfixe de son enfant, le
/// précède), puis désactivations. Le sens et les caractères collectif et de trésorerie
/// recalculés doivent égaler les colonnes relues.
///
/// # Errors
/// [`ErreurRelecture`].
pub fn reconstituer_plan(
    lignes: impl IntoIterator<Item = LigneCompte>,
) -> Result<PlanComptes, ErreurRelecture> {
    let mut lignes: Vec<LigneCompte> = lignes.into_iter().collect();
    lignes.sort_by(|a, b| a.numero.cmp(&b.numero));
    let officiels = lignes
        .iter()
        .filter(|l| l.genre == GenreCompte::Officiel)
        .map(|l| {
            let reference = CompteReference::nouveau(
                l.numero.clone(),
                l.intitule.clone(),
                l.sens,
                l.collectif,
            )?;
            Ok(if l.tresorerie {
                reference.admettre_en_journal_de_tresorerie()
            } else {
                reference
            })
        })
        .collect::<Result<Vec<_>, ErreurPlan>>()?;
    let mut plan = PlanComptes::depuis_referentiel(officiels)?;
    for l in &lignes {
        let compte = match l.genre {
            GenreCompte::Officiel => plan
                .compte(&l.numero)
                .ok_or_else(|| ErreurPlan::Inconnu(l.numero.clone()))?,
            GenreCompte::SousCompte => {
                plan.ajouter_sous_compte(l.numero.clone(), l.intitule.clone())?
            }
            GenreCompte::Auxiliaire => {
                plan.ajouter_auxiliaire(l.numero.clone(), l.intitule.clone())?
            }
        };
        if (
            compte.sens(),
            compte.est_collectif(),
            compte.est_admis_en_journal_de_tresorerie(),
        ) != (l.sens, l.collectif, l.tresorerie)
        {
            return Err(ErreurRelecture::Incoherence(l.numero.clone()));
        }
    }
    for l in lignes.iter().filter(|l| !l.actif) {
        plan.desactiver(&l.numero)?;
    }
    Ok(plan)
}

/// Lignes plates des journaux, par ordre de code.
#[must_use]
pub fn aplatir_journaux(journaux: &Journaux) -> Vec<LigneJournal> {
    journaux
        .journaux()
        .map(|j| LigneJournal {
            code: j.code().clone(),
            intitule: j.intitule().clone(),
            genre: j.genre(),
            admis_en_cloture: j.admis_en_cloture(),
            compte_tresorerie: j.compte_tresorerie().cloned(),
        })
        .collect()
}

/// Reconstitue les journaux relus sur le plan relu, par les seules fonctions publiques de
/// `compta-core` (`Journal::nouveau`, `Journal::admettre_en_cloture`).
///
/// # Errors
/// [`ErreurRelecture`] : notamment un compte de trésorerie qui n'est plus un compte de
/// mouvement actif, ce que le contrôle croisé de [`muter`] empêche.
pub fn reconstituer_journaux(
    lignes: impl IntoIterator<Item = LigneJournal>,
    plan: &PlanComptes,
) -> Result<Journaux, ErreurRelecture> {
    let mut journaux = Journaux::new();
    for l in lignes {
        let journal = Journal::nouveau(l.code, l.intitule, l.genre, l.compte_tresorerie, plan)?;
        journaux.ajouter(if l.admis_en_cloture {
            journal.admettre_en_cloture()?
        } else {
            journal
        })?;
    }
    Ok(journaux)
}

// --- Noms stockés ---------------------------------------------------------------------------

/// Nom stocké d'une origine de compte.
#[must_use]
pub const fn nom_genre_compte(genre: GenreCompte) -> &'static str {
    match genre {
        GenreCompte::Officiel => "officiel",
        GenreCompte::SousCompte => "sous_compte",
        GenreCompte::Auxiliaire => "auxiliaire",
    }
}

/// Nom stocké d'un sens normal.
#[must_use]
pub const fn nom_sens(sens: SensNormal) -> &'static str {
    match sens {
        SensNormal::Debiteur => "debiteur",
        SensNormal::Crediteur => "crediteur",
        SensNormal::Variable => "variable",
    }
}

/// Nom stocké d'un type de journal (celui des données de `compta-core`).
#[must_use]
pub const fn nom_type_journal(genre: TypeJournal) -> &'static str {
    match genre {
        TypeJournal::Achats => "achats",
        TypeJournal::Ventes => "ventes",
        TypeJournal::Tresorerie => "tresorerie",
        TypeJournal::OperationsDiverses => "operations_diverses",
        TypeJournal::ANouveaux => "a_nouveaux",
    }
}

/// Nom stocké d'un statut d'exercice.
#[must_use]
pub const fn nom_statut_exercice(statut: StatutExercice) -> &'static str {
    match statut {
        StatutExercice::Ouvert => "ouvert",
        StatutExercice::EnCloture => "en_cloture",
        StatutExercice::Cloture => "cloture",
    }
}

/// Nom stocké d'un statut de période.
#[must_use]
pub const fn nom_statut_periode(statut: StatutPeriode) -> &'static str {
    match statut {
        StatutPeriode::Ouverte => "ouverte",
        StatutPeriode::Verrouillee => "verrouillee",
        StatutPeriode::Cloturee => "cloturee",
    }
}

fn donnee(_: impl Sized) -> ErreurBase {
    ErreurBase::Donnee("référentiel relu invalide")
}

fn genre_compte_lu(texte: &str) -> Result<GenreCompte, ErreurBase> {
    [
        GenreCompte::Officiel,
        GenreCompte::SousCompte,
        GenreCompte::Auxiliaire,
    ]
    .into_iter()
    .find(|g| nom_genre_compte(*g) == texte)
    .ok_or(ErreurBase::Donnee("origine de compte inconnue"))
}

fn sens_lu(texte: &str) -> Result<SensNormal, ErreurBase> {
    [
        SensNormal::Debiteur,
        SensNormal::Crediteur,
        SensNormal::Variable,
    ]
    .into_iter()
    .find(|s| nom_sens(*s) == texte)
    .ok_or(ErreurBase::Donnee("sens inconnu"))
}

fn statut_exercice_lu(texte: &str) -> Result<StatutExercice, ErreurBase> {
    [
        StatutExercice::Ouvert,
        StatutExercice::EnCloture,
        StatutExercice::Cloture,
    ]
    .into_iter()
    .find(|s| nom_statut_exercice(*s) == texte)
    .ok_or(ErreurBase::Donnee("statut d'exercice inconnu"))
}

fn statut_periode_lu(texte: &str) -> Result<StatutPeriode, ErreurBase> {
    [
        StatutPeriode::Ouverte,
        StatutPeriode::Verrouillee,
        StatutPeriode::Cloturee,
    ]
    .into_iter()
    .find(|s| nom_statut_periode(*s) == texte)
    .ok_or(ErreurBase::Donnee("statut de période inconnu"))
}

// --- Modèle officiel et instanciation (pur) -------------------------------------------------

/// Référentiel officiel prêt à copier : plan de `syscohada` et journaux par défaut, avec leur
/// version et leur empreinte. Chargé une fois ([`Modele::officiel`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Modele {
    version: String,
    empreinte: Empreinte,
    etat: EtatReferentiel,
}

/// Préfixe du contenu haché par [`Modele::empreinte`].
const ENTETE_EMPREINTE: &str = "sigma-books referentiel v1\n";

impl Modele {
    /// Le référentiel officiel embarqué (`syscohada`), chargé et validé une fois. Le serveur
    /// le charge au démarrage et refuse de démarrer s'il est invalide.
    ///
    /// # Errors
    /// [`ErreurBase::Donnee`] si les données embarquées sont invalides (garanti par les tests
    /// de `syscohada`).
    pub fn officiel() -> Result<&'static Self, ErreurBase> {
        static MODELE: OnceLock<Option<Modele>> = OnceLock::new();
        MODELE
            .get_or_init(|| Self::charger().ok())
            .as_ref()
            .ok_or(ErreurBase::Donnee("référentiel officiel invalide"))
    }

    fn charger() -> Result<Self, String> {
        let referentiel = syscohada::Referentiel::officiel().map_err(|e| e.to_string())?;
        let plan = referentiel.plan_comptes().map_err(|e| e.to_string())?;
        let journaux = referentiel
            .journaux_par_defaut(&plan)
            .map_err(|e| e.to_string())?;
        let etat = EtatReferentiel {
            plan,
            journaux,
            tiers: BTreeMap::new(),
        };
        verifier(&etat).map_err(|e| e.to_string())?;
        let version = referentiel.version().to_owned();
        Ok(Self {
            empreinte: empreinte(&version, &etat),
            version,
            etat,
        })
    }

    /// Version du référentiel (`syscohada`, `[source] version`).
    #[must_use]
    pub fn version(&self) -> &str {
        &self.version
    }

    /// Empreinte SHA-256 de ce qui est copié dans un nouveau dossier : la version, puis chaque
    /// compte et chaque journal, attribut par attribut (forme : [`empreinte`]). Elle ne dépend
    /// que du contenu copié, pas de la mise en forme des fichiers de données.
    #[must_use]
    pub fn empreinte(&self) -> Empreinte {
        self.empreinte
    }

    /// Plan et journaux copiés dans un nouveau dossier.
    #[must_use]
    pub fn etat(&self) -> &EtatReferentiel {
        &self.etat
    }
}

/// Empreinte d'un référentiel : SHA-256 de `sigma-books referentiel v1\n`, puis
/// `version=<version>\n`, puis une ligne par compte
/// (`compte=<numéro>;<origine>;<sens>;<collectif 0|1>;<trésorerie 0|1>;<actif 0|1>;<intitulé>`)
/// et une par journal (`journal=<code>;<type>;<clôture 0|1>;<compte ou vide>;<intitulé>`), dans
/// l'ordre des numéros et des codes. L'intitulé, seul champ libre, vient en dernier et ne
/// contient aucun caractère de contrôle : la forme est sans ambiguïté.
#[must_use]
pub fn empreinte(version: &str, etat: &EtatReferentiel) -> Empreinte {
    let b = |v: bool| if v { '1' } else { '0' };
    let mut lignes = vec![ENTETE_EMPREINTE.to_owned(), format!("version={version}\n")];
    for l in aplatir_plan(&etat.plan) {
        lignes.push(format!(
            "compte={};{};{};{};{};{};{}\n",
            l.numero,
            nom_genre_compte(l.genre),
            nom_sens(l.sens),
            b(l.collectif),
            b(l.tresorerie),
            b(l.actif),
            l.intitule
        ));
    }
    for l in aplatir_journaux(&etat.journaux) {
        lignes.push(format!(
            "journal={};{};{};{};{}\n",
            l.code,
            nom_type_journal(l.genre),
            b(l.admis_en_cloture),
            l.compte_tresorerie
                .as_ref()
                .map_or_else(String::new, ToString::to_string),
            l.intitule
        ));
    }
    Empreinte::calculer(lignes.concat().as_bytes())
}

/// Ouverture du premier exercice d'un nouveau dossier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ouverture {
    /// Entité nouvelle : premier exercice ouvert à son début d'activité (AUDCIF art. 7), avec
    /// l'option « second semestre ».
    Creation {
        /// Début d'activité.
        debut_activite: Date,
        /// Fin du premier exercice.
        fin: FinPremierExercice,
    },
    /// Entité déjà en activité : exercice civil choisi (art. 7 et 34), sans durée dérogatoire.
    Reprise {
        /// Année de l'exercice ouvert.
        annee: i32,
    },
}

impl Ouverture {
    /// Nom stocké de la nature du premier exercice.
    #[must_use]
    pub const fn nature(self) -> &'static str {
        match self {
            Self::Creation { .. } => "creation",
            Self::Reprise { .. } => "reprise",
        }
    }
}

/// Tout ce qu'un nouveau dossier reçoit, calculé en pur avant la première écriture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Instanciation {
    ouverture: Ouverture,
    version: String,
    empreinte: Empreinte,
    etat: EtatReferentiel,
    exercices: Exercices,
}

impl Instanciation {
    /// Prépare le référentiel d'un nouveau dossier : copie du modèle, premier exercice.
    ///
    /// Un dossier repris ne s'ouvre pas sur une année future. Le début d'activité d'une entité
    /// nouvelle peut précéder sa date réelle (dossier préparé à l'avance), au plus jusqu'à la
    /// fin de l'année suivante : au-delà, c'est une erreur de saisie, et un exercice ne se
    /// supprime pas.
    ///
    /// # Errors
    /// [`RefusReferentiel::DateRefusee`], refus de `Exercices` (prolongation au premier
    /// semestre, date hors plage).
    pub fn preparer(
        modele: &Modele,
        ouverture: Ouverture,
        aujourd_hui: Date,
    ) -> Result<Self, RefusReferentiel> {
        let annee = aujourd_hui.year();
        let mut exercices = Exercices::new();
        match ouverture {
            Ouverture::Reprise { annee: choisie } => {
                if choisie > annee {
                    return Err(RefusReferentiel::DateRefusee);
                }
                exercices.ouvrir_repris(choisie)?;
            }
            Ouverture::Creation {
                debut_activite,
                fin,
            } => {
                if debut_activite.year() > annee.saturating_add(1) {
                    return Err(RefusReferentiel::DateRefusee);
                }
                exercices.ouvrir_premier(debut_activite, fin)?;
            }
        }
        let etat = modele.etat.clone();
        verifier(&etat)?;
        Ok(Self {
            ouverture,
            version: modele.version.clone(),
            empreinte: modele.empreinte,
            etat,
            exercices,
        })
    }

    /// Ouverture demandée.
    #[must_use]
    pub fn ouverture(&self) -> Ouverture {
        self.ouverture
    }

    /// Plan et journaux du nouveau dossier.
    #[must_use]
    pub fn etat(&self) -> &EtatReferentiel {
        &self.etat
    }

    /// Exercices du nouveau dossier.
    #[must_use]
    pub fn exercices(&self) -> &Exercices {
        &self.exercices
    }

    /// Version du référentiel copié.
    #[must_use]
    pub fn version(&self) -> &str {
        &self.version
    }

    /// Empreinte du référentiel copié.
    #[must_use]
    pub fn empreinte(&self) -> Empreinte {
        self.empreinte
    }
}

// --- Référentiel relu --------------------------------------------------------------------

/// Référentiel d'un dossier relu de la base : état reconstitué et identifiants des lignes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReferentielDossier {
    etat: EtatReferentiel,
    exercices: Exercices,
    ids_comptes: BTreeMap<NumeroCompte, Uuid>,
    ids_journaux: BTreeMap<CodeJournal, Uuid>,
    ids_exercices: BTreeMap<CodeExercice, Uuid>,
    ids_periodes: BTreeMap<Date, Uuid>,
}

impl ReferentielDossier {
    /// Plan, journaux et tiers.
    #[must_use]
    pub fn etat(&self) -> &EtatReferentiel {
        &self.etat
    }

    /// Exercices et périodes.
    #[must_use]
    pub fn exercices(&self) -> &Exercices {
        &self.exercices
    }

    /// Identifiant d'un compte.
    #[must_use]
    pub fn id_compte(&self, numero: &NumeroCompte) -> Option<Uuid> {
        self.ids_comptes.get(numero).copied()
    }

    /// Compte d'identifiant donné.
    #[must_use]
    pub fn compte_par_id(&self, id: Uuid) -> Option<&Compte> {
        self.ids_comptes
            .iter()
            .find(|(_, i)| **i == id)
            .and_then(|(numero, _)| self.etat.plan.compte(numero))
    }

    /// Identifiant d'un journal.
    #[must_use]
    pub fn id_journal(&self, code: &CodeJournal) -> Option<Uuid> {
        self.ids_journaux.get(code).copied()
    }

    /// Identifiant d'un exercice.
    #[must_use]
    pub fn id_exercice(&self, code: &CodeExercice) -> Option<Uuid> {
        self.ids_exercices.get(code).copied()
    }

    /// Identifiant de la période commençant à `debut`.
    #[must_use]
    pub fn id_periode(&self, debut: Date) -> Option<Uuid> {
        self.ids_periodes.get(&debut).copied()
    }
}

/// Changement enregistré par [`muter`] : de quoi composer la trace d'audit (champs modifiés
/// seulement, ADR 0014) et la réponse.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Changement {
    /// Genre de la ressource (`compte`, `journal`, `tiers`).
    pub genre: &'static str,
    /// Identifiant de la ressource.
    pub id: Uuid,
    /// Vrai pour une création.
    pub cree: bool,
    /// Données citées (le numéro d'un compte modifié).
    pub donnees: Vec<(&'static str, String)>,
    /// Champs modifiés (nom, avant, après).
    pub champs: Vec<(&'static str, Option<String>, Option<String>)>,
}

/// Résultat d'une mutation enregistrée.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mutation<R> {
    /// Ce que la mutation pure a rendu.
    pub resultat: R,
    /// État après la mutation.
    pub etat: EtatReferentiel,
    /// Changements enregistrés, au moins un.
    pub changements: Vec<Changement>,
}

/// Échec d'une mutation.
#[derive(Debug, thiserror::Error)]
pub enum ErreurMutation {
    /// Le dossier du contexte n'a pas de référentiel.
    #[error("référentiel absent")]
    Absent,
    /// La mutation est refusée par les règles.
    #[error(transparent)]
    Refus(#[from] RefusReferentiel),
    /// Erreur d'accès à la base.
    #[error(transparent)]
    Base(#[from] ErreurBase),
}

impl From<sqlx::Error> for ErreurMutation {
    fn from(erreur: sqlx::Error) -> Self {
        Self::Base(ErreurBase::Sqlx(erreur))
    }
}

fn contexte(tx: &TransactionTenant) -> Result<(Uuid, Uuid), ErreurBase> {
    let dossier = tx.contexte().id_dossier().ok_or(ErreurBase::Donnee(
        "référentiel hors du contexte d'un dossier",
    ))?;
    Ok((tx.contexte().id_cabinet().uuid(), dossier.uuid()))
}

// --- Accès à la base -------------------------------------------------------------------------

/// Verrou du référentiel du dossier du contexte (`FOR NO KEY UPDATE` sur sa ligne `dossier`).
/// Faux si le dossier est absent du contexte.
///
/// # Errors
/// Contexte sans dossier, erreur de PostgreSQL.
pub async fn verrouiller(tx: &mut TransactionTenant) -> Result<bool, ErreurBase> {
    let (cabinet, dossier) = contexte(tx)?;
    let ligne = sqlx::query!(
        "SELECT id FROM dossier WHERE cabinet_id = $1 AND id = $2 FOR NO KEY UPDATE",
        cabinet,
        dossier,
    )
    .fetch_optional(tx.connexion())
    .await?;
    Ok(ligne.is_some())
}

/// Inscrit le référentiel préparé dans le dossier `dossier`, créé dans cette transaction :
/// le contexte est d'abord restreint à ce dossier (la RLS des tables du référentiel l'exige).
/// Une seule requête par table ; une seconde instanciation échoue (clé primaire de
/// `dossier_referentiel`).
///
/// # Errors
/// Erreur de PostgreSQL.
pub async fn instancier(
    tx: &mut TransactionTenant,
    dossier: IdDossier,
    instanciation: &Instanciation,
) -> Result<(), ErreurBase> {
    tx.restreindre_au_dossier(dossier).await?;
    let (cabinet, dossier) = contexte(tx)?;
    let debut_activite = match instanciation.ouverture {
        Ouverture::Creation { debut_activite, .. } => Some(debut_activite),
        Ouverture::Reprise { .. } => None,
    };
    sqlx::query!(
        "INSERT INTO dossier_referentiel \
             (cabinet_id, dossier_id, nature, debut_activite, referentiel_version, \
              referentiel_empreinte) \
         VALUES ($1, $2, $3, $4, $5, $6)",
        cabinet,
        dossier,
        instanciation.ouverture.nature(),
        debut_activite,
        instanciation.version,
        instanciation.empreinte.octets().as_slice(),
    )
    .execute(tx.connexion())
    .await?;
    let comptes: Vec<(Uuid, LigneCompte)> = aplatir_plan(&instanciation.etat.plan)
        .into_iter()
        .map(|l| (Uuid::new_v4(), l))
        .collect();
    inserer_comptes(tx, &comptes).await?;
    for (id, ligne) in aplatir_journaux(&instanciation.etat.journaux)
        .into_iter()
        .map(|l| (Uuid::new_v4(), l))
    {
        inserer_journal(tx, id, &ligne).await?;
    }
    inserer_exercices(tx, &instanciation.exercices).await
}

async fn inserer_comptes(
    tx: &mut TransactionTenant,
    comptes: &[(Uuid, LigneCompte)],
) -> Result<(), ErreurBase> {
    let (cabinet, dossier) = contexte(tx)?;
    let ids: Vec<Uuid> = comptes.iter().map(|(id, _)| *id).collect();
    let numeros: Vec<String> = comptes.iter().map(|(_, l)| l.numero.to_string()).collect();
    let parents: Vec<Option<String>> = comptes
        .iter()
        .map(|(_, l)| l.numero.parent().map(|p| p.to_string()))
        .collect();
    let intitules: Vec<String> = comptes
        .iter()
        .map(|(_, l)| l.intitule.as_str().to_owned())
        .collect();
    let genres: Vec<String> = comptes
        .iter()
        .map(|(_, l)| nom_genre_compte(l.genre).to_owned())
        .collect();
    let sens: Vec<String> = comptes
        .iter()
        .map(|(_, l)| nom_sens(l.sens).to_owned())
        .collect();
    let collectifs: Vec<bool> = comptes.iter().map(|(_, l)| l.collectif).collect();
    let tresoreries: Vec<bool> = comptes.iter().map(|(_, l)| l.tresorerie).collect();
    let actifs: Vec<bool> = comptes.iter().map(|(_, l)| l.actif).collect();
    sqlx::query!(
        "INSERT INTO compte \
             (cabinet_id, dossier_id, id, numero, parent_numero, intitule, genre, sens, \
              collectif, tresorerie, actif) \
         SELECT $1, $2, l.id, l.numero, l.parent, l.intitule, l.genre, l.sens, l.collectif, \
                l.tresorerie, l.actif \
         FROM UNNEST($3::uuid[], $4::text[], $5::text[], $6::text[], $7::text[], $8::text[], \
                     $9::bool[], $10::bool[], $11::bool[]) \
              AS l (id, numero, parent, intitule, genre, sens, collectif, tresorerie, actif)",
        cabinet,
        dossier,
        &ids,
        &numeros,
        &parents as &[Option<String>],
        &intitules,
        &genres,
        &sens,
        &collectifs,
        &tresoreries,
        &actifs,
    )
    .execute(tx.connexion())
    .await?;
    Ok(())
}

async fn inserer_journal(
    tx: &mut TransactionTenant,
    id: Uuid,
    l: &LigneJournal,
) -> Result<(), ErreurBase> {
    let (cabinet, dossier) = contexte(tx)?;
    sqlx::query!(
        "INSERT INTO journal \
             (cabinet_id, dossier_id, id, code, intitule, type, admis_en_cloture, \
              compte_tresorerie) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
        cabinet,
        dossier,
        id,
        l.code.as_str(),
        l.intitule.as_str(),
        nom_type_journal(l.genre),
        l.admis_en_cloture,
        l.compte_tresorerie.as_ref().map(NumeroCompte::as_str),
    )
    .execute(tx.connexion())
    .await?;
    Ok(())
}

async fn inserer_exercices(
    tx: &mut TransactionTenant,
    exercices: &Exercices,
) -> Result<(), ErreurBase> {
    let (cabinet, dossier) = contexte(tx)?;
    let (lignes, periodes) = exercices.aplatir();
    let mut ids: BTreeMap<CodeExercice, Uuid> = BTreeMap::new();
    for e in lignes {
        let id = Uuid::new_v4();
        sqlx::query!(
            "INSERT INTO exercice (cabinet_id, dossier_id, id, code, debut, fin, statut) \
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
            cabinet,
            dossier,
            id,
            e.code.as_str(),
            e.debut,
            e.fin,
            nom_statut_exercice(e.statut),
        )
        .execute(tx.connexion())
        .await?;
        ids.insert(e.code, id);
    }
    let mut ids_periodes = Vec::new();
    let mut exercices_periodes = Vec::new();
    let mut debuts = Vec::new();
    let mut fins = Vec::new();
    let mut statuts = Vec::new();
    for p in periodes {
        let exercice = ids
            .get(&p.exercice)
            .copied()
            .ok_or(ErreurBase::Donnee("période sans exercice"))?;
        ids_periodes.push(Uuid::new_v4());
        exercices_periodes.push(exercice);
        debuts.push(p.debut);
        fins.push(p.fin);
        statuts.push(nom_statut_periode(p.statut).to_owned());
    }
    sqlx::query!(
        "INSERT INTO periode (cabinet_id, dossier_id, id, exercice_id, debut, fin, statut) \
         SELECT $1, $2, p.id, p.exercice, p.debut, p.fin, p.statut \
         FROM UNNEST($3::uuid[], $4::uuid[], $5::date[], $6::date[], $7::text[]) \
              AS p (id, exercice, debut, fin, statut)",
        cabinet,
        dossier,
        &ids_periodes,
        &exercices_periodes,
        &debuts,
        &fins,
        &statuts,
    )
    .execute(tx.connexion())
    .await?;
    Ok(())
}

/// Relit et reconstitue le référentiel du dossier du contexte. `None` si le dossier n'en a
/// pas (dossier d'autrui, ou créé avant le référentiel).
///
/// # Errors
/// Contexte sans dossier, lignes relues incohérentes ([`ErreurBase::Donnee`]), erreur de
/// PostgreSQL.
pub async fn charger(tx: &mut TransactionTenant) -> Result<Option<ReferentielDossier>, ErreurBase> {
    let (cabinet, dossier) = contexte(tx)?;
    let present = sqlx::query!(
        "SELECT dossier_id FROM dossier_referentiel WHERE cabinet_id = $1 AND dossier_id = $2",
        cabinet,
        dossier,
    )
    .fetch_optional(tx.connexion())
    .await?;
    if present.is_none() {
        return Ok(None);
    }

    let (plan, journaux, ids_comptes, ids_journaux) =
        charger_plan_et_journaux(tx, cabinet, dossier).await?;
    let tiers = charger_tiers(tx, cabinet, dossier).await?;
    let (exercices, ids_exercices, ids_periodes) = charger_exercices(tx, cabinet, dossier).await?;

    let etat = EtatReferentiel {
        plan,
        journaux,
        tiers,
    };
    Ok(Some(ReferentielDossier {
        etat,
        exercices,
        ids_comptes,
        ids_journaux,
        ids_exercices,
        ids_periodes,
    }))
}

/// Plan et journaux relus, avec leurs identifiants.
type PlanEtJournauxRelus = (
    PlanComptes,
    Journaux,
    BTreeMap<NumeroCompte, Uuid>,
    BTreeMap<CodeJournal, Uuid>,
);

async fn charger_plan_et_journaux(
    tx: &mut TransactionTenant,
    cabinet: Uuid,
    dossier: Uuid,
) -> Result<PlanEtJournauxRelus, ErreurBase> {
    let comptes = sqlx::query!(
        "SELECT id, numero, intitule, genre, sens, collectif, tresorerie, actif FROM compte \
         WHERE cabinet_id = $1 AND dossier_id = $2",
        cabinet,
        dossier,
    )
    .fetch_all(tx.connexion())
    .await?;
    let mut ids_comptes = BTreeMap::new();
    let mut lignes = Vec::with_capacity(comptes.len());
    for c in comptes {
        let numero = NumeroCompte::parse(&c.numero).map_err(donnee)?;
        ids_comptes.insert(numero.clone(), c.id);
        lignes.push(LigneCompte {
            numero,
            intitule: TexteCourt::parse(&c.intitule).map_err(donnee)?,
            genre: genre_compte_lu(&c.genre)?,
            sens: sens_lu(&c.sens)?,
            collectif: c.collectif,
            tresorerie: c.tresorerie,
            actif: c.actif,
        });
    }
    let plan = reconstituer_plan(lignes).map_err(donnee)?;

    let journaux = sqlx::query!(
        "SELECT id, code, intitule, type, admis_en_cloture, compte_tresorerie FROM journal \
         WHERE cabinet_id = $1 AND dossier_id = $2",
        cabinet,
        dossier,
    )
    .fetch_all(tx.connexion())
    .await?;
    let mut ids_journaux = BTreeMap::new();
    let mut lignes = Vec::with_capacity(journaux.len());
    for j in journaux {
        let code = CodeJournal::parse(&j.code).map_err(donnee)?;
        ids_journaux.insert(code.clone(), j.id);
        lignes.push(LigneJournal {
            code,
            intitule: TexteCourt::parse(&j.intitule).map_err(donnee)?,
            genre: TypeJournal::parse(&j.r#type).map_err(donnee)?,
            admis_en_cloture: j.admis_en_cloture,
            compte_tresorerie: j
                .compte_tresorerie
                .as_deref()
                .map(NumeroCompte::parse)
                .transpose()
                .map_err(donnee)?,
        });
    }
    let journaux = reconstituer_journaux(lignes, &plan).map_err(donnee)?;
    Ok((plan, journaux, ids_comptes, ids_journaux))
}

async fn charger_tiers(
    tx: &mut TransactionTenant,
    cabinet: Uuid,
    dossier: Uuid,
) -> Result<BTreeMap<Uuid, Tiers>, ErreurBase> {
    let lignes_tiers = sqlx::query!(
        "SELECT id, nom, type, compte_collectif, actif FROM tiers \
         WHERE cabinet_id = $1 AND dossier_id = $2",
        cabinet,
        dossier,
    )
    .fetch_all(tx.connexion())
    .await?;
    let mut tiers = BTreeMap::new();
    for t in lignes_tiers {
        tiers.insert(
            t.id,
            Tiers {
                nom: TexteCourt::parse(&t.nom).map_err(donnee)?,
                genre: TypeTiers::parse(&t.r#type).ok_or(ErreurBase::Donnee("type de tiers"))?,
                compte_collectif: NumeroCompte::parse(&t.compte_collectif).map_err(donnee)?,
                actif: t.actif,
            },
        );
    }

    Ok(tiers)
}

/// Exercices relus, avec les identifiants des exercices et des périodes.
type ExercicesRelus = (
    Exercices,
    BTreeMap<CodeExercice, Uuid>,
    BTreeMap<Date, Uuid>,
);

async fn charger_exercices(
    tx: &mut TransactionTenant,
    cabinet: Uuid,
    dossier: Uuid,
) -> Result<ExercicesRelus, ErreurBase> {
    let lignes_exercices = sqlx::query!(
        "SELECT id, code, debut, fin, statut FROM exercice \
         WHERE cabinet_id = $1 AND dossier_id = $2",
        cabinet,
        dossier,
    )
    .fetch_all(tx.connexion())
    .await?;
    let mut ids_exercices = BTreeMap::new();
    let mut exercices = Vec::with_capacity(lignes_exercices.len());
    for e in lignes_exercices {
        let code = CodeExercice::parse(&e.code).map_err(donnee)?;
        ids_exercices.insert(code.clone(), e.id);
        exercices.push(LigneExercice {
            code,
            debut: e.debut,
            fin: e.fin,
            statut: statut_exercice_lu(&e.statut)?,
        });
    }
    let lignes_periodes = sqlx::query!(
        "SELECT p.id, e.code, p.debut, p.fin, p.statut FROM periode p \
         JOIN exercice e ON e.cabinet_id = p.cabinet_id AND e.dossier_id = p.dossier_id \
                        AND e.id = p.exercice_id \
         WHERE p.cabinet_id = $1 AND p.dossier_id = $2",
        cabinet,
        dossier,
    )
    .fetch_all(tx.connexion())
    .await?;
    let mut ids_periodes = BTreeMap::new();
    let mut periodes = Vec::with_capacity(lignes_periodes.len());
    for p in lignes_periodes {
        ids_periodes.insert(p.debut, p.id);
        periodes.push(LignePeriode {
            exercice: CodeExercice::parse(&p.code).map_err(donnee)?,
            debut: p.debut,
            fin: p.fin,
            statut: statut_periode_lu(&p.statut)?,
        });
    }
    let exercices = Exercices::reconstituer(exercices, periodes).map_err(donnee)?;
    Ok((exercices, ids_exercices, ids_periodes))
}

/// **Seule voie de mutation** du plan, des journaux et des tiers d'un dossier existant :
/// 1. verrou du référentiel ([`verrouiller`]) ;
/// 2. relecture ([`charger`]) ;
/// 3. `mutation`, pure, sur une **copie** de l'état relu ;
/// 4. contrôle croisé de la copie ([`verifier`]) ;
/// 5. enregistrement des seules différences (insertions, `UPDATE` des colonnes modifiables).
///
/// Rien n'est écrit si l'une des étapes échoue ; une mutation sans effet est refusée
/// ([`RefusReferentiel::AucunChangement`]).
///
/// # Errors
/// [`ErreurMutation`].
pub async fn muter<R>(
    tx: &mut TransactionTenant,
    mutation: impl FnOnce(&ReferentielDossier, &mut EtatReferentiel) -> Result<R, RefusReferentiel>,
) -> Result<Mutation<R>, ErreurMutation> {
    if !verrouiller(tx).await? {
        return Err(ErreurMutation::Absent);
    }
    let relu = charger(tx).await?.ok_or(ErreurMutation::Absent)?;
    let mut candidat = relu.etat.clone();
    let resultat = mutation(&relu, &mut candidat)?;
    verifier(&candidat)?;
    let changements = enregistrer(tx, &relu, &candidat).await?;
    if changements.is_empty() {
        return Err(RefusReferentiel::AucunChangement.into());
    }
    Ok(Mutation {
        resultat,
        etat: candidat,
        changements,
    })
}

/// Écart de deux valeurs pour un champ modifié : `None` si elles sont égales.
fn champ(
    nom: &'static str,
    avant: &impl ToString,
    apres: &impl ToString,
) -> Option<(&'static str, Option<String>, Option<String>)> {
    let (avant, apres) = (avant.to_string(), apres.to_string());
    (avant != apres).then_some((nom, Some(avant), Some(apres)))
}

/// Enregistre les différences entre l'état relu et la copie mutée. Une différence qu'aucune
/// colonne modifiable ne porte (suppression, attribut immuable changé) est une erreur interne :
/// aucune mutation de la PR 11a ne la produit.
async fn enregistrer(
    tx: &mut TransactionTenant,
    relu: &ReferentielDossier,
    apres: &EtatReferentiel,
) -> Result<Vec<Changement>, ErreurBase> {
    let mut changements = enregistrer_comptes(tx, relu, apres).await?;
    changements.extend(enregistrer_journaux(tx, relu, apres).await?);
    changements.extend(enregistrer_tiers(tx, relu, apres).await?);
    Ok(changements)
}

fn immuable() -> ErreurBase {
    ErreurBase::Donnee("modification d'un attribut immuable du référentiel")
}

async fn enregistrer_comptes(
    tx: &mut TransactionTenant,
    relu: &ReferentielDossier,
    apres: &EtatReferentiel,
) -> Result<Vec<Changement>, ErreurBase> {
    let (cabinet, dossier) = contexte(tx)?;
    let mut changements = Vec::new();
    // Comptes.
    if relu
        .etat
        .plan
        .comptes()
        .any(|c| apres.plan.compte(c.numero()).is_none())
    {
        return Err(immuable());
    }
    let mut nouveaux = Vec::new();
    for c in apres.plan.comptes() {
        match relu.etat.plan.compte(c.numero()) {
            None => {
                let id = Uuid::new_v4();
                changements.push(Changement {
                    genre: "compte",
                    id,
                    cree: true,
                    donnees: Vec::new(),
                    champs: vec![
                        ("numero", None, Some(c.numero().to_string())),
                        ("intitule", None, Some(c.intitule().to_string())),
                        ("genre", None, Some(nom_genre_compte(c.genre()).to_owned())),
                    ],
                });
                nouveaux.push((
                    id,
                    LigneCompte {
                        numero: c.numero().clone(),
                        intitule: c.intitule().clone(),
                        genre: c.genre(),
                        sens: c.sens(),
                        collectif: c.est_collectif(),
                        tresorerie: c.est_admis_en_journal_de_tresorerie(),
                        actif: c.est_actif(),
                    },
                ));
            }
            Some(avant) if avant == c => {}
            Some(avant) => {
                let identiques = (
                    avant.genre(),
                    avant.sens(),
                    avant.est_collectif(),
                    avant.est_admis_en_journal_de_tresorerie(),
                ) == (
                    c.genre(),
                    c.sens(),
                    c.est_collectif(),
                    c.est_admis_en_journal_de_tresorerie(),
                );
                let id = relu.id_compte(c.numero());
                let (true, Some(id)) = (identiques, id) else {
                    return Err(immuable());
                };
                sqlx::query!(
                    "UPDATE compte SET intitule = $4, actif = $5 \
                     WHERE cabinet_id = $1 AND dossier_id = $2 AND id = $3",
                    cabinet,
                    dossier,
                    id,
                    c.intitule().as_str(),
                    c.est_actif(),
                )
                .execute(tx.connexion())
                .await?;
                changements.push(Changement {
                    genre: "compte",
                    id,
                    cree: false,
                    donnees: vec![("numero", c.numero().to_string())],
                    champs: [
                        champ("intitule", avant.intitule(), c.intitule()),
                        champ("actif", &avant.est_actif(), &c.est_actif()),
                    ]
                    .into_iter()
                    .flatten()
                    .collect(),
                });
            }
        }
    }
    if !nouveaux.is_empty() {
        inserer_comptes(tx, &nouveaux).await?;
    }

    Ok(changements)
}

async fn enregistrer_journaux(
    tx: &mut TransactionTenant,
    relu: &ReferentielDossier,
    apres: &EtatReferentiel,
) -> Result<Vec<Changement>, ErreurBase> {
    let mut changements = Vec::new();
    // Journaux : création seule.
    if relu
        .etat
        .journaux
        .journaux()
        .any(|j| apres.journaux.journal(j.code()) != Some(j))
    {
        return Err(immuable());
    }
    for l in aplatir_journaux(&apres.journaux) {
        if relu.etat.journaux.journal(&l.code).is_some() {
            continue;
        }
        let id = Uuid::new_v4();
        inserer_journal(tx, id, &l).await?;
        let mut champs = vec![
            ("code", None, Some(l.code.to_string())),
            ("intitule", None, Some(l.intitule.to_string())),
            ("type", None, Some(nom_type_journal(l.genre).to_owned())),
        ];
        if let Some(compte) = &l.compte_tresorerie {
            champs.push(("compte_tresorerie", None, Some(compte.to_string())));
        }
        if l.admis_en_cloture {
            champs.push(("admis_en_cloture", None, Some(true.to_string())));
        }
        changements.push(Changement {
            genre: "journal",
            id,
            cree: true,
            donnees: Vec::new(),
            champs,
        });
    }

    Ok(changements)
}

async fn enregistrer_tiers(
    tx: &mut TransactionTenant,
    relu: &ReferentielDossier,
    apres: &EtatReferentiel,
) -> Result<Vec<Changement>, ErreurBase> {
    let (cabinet, dossier) = contexte(tx)?;
    let mut changements = Vec::new();
    // Tiers.
    if relu
        .etat
        .tiers
        .keys()
        .any(|id| !apres.tiers.contains_key(id))
    {
        return Err(immuable());
    }
    for (id, t) in &apres.tiers {
        match relu.etat.tiers.get(id) {
            None => {
                sqlx::query!(
                    "INSERT INTO tiers \
                         (cabinet_id, dossier_id, id, nom, type, compte_collectif, actif) \
                     VALUES ($1, $2, $3, $4, $5, $6, $7)",
                    cabinet,
                    dossier,
                    id,
                    t.nom.as_str(),
                    t.genre.nom(),
                    t.compte_collectif.as_str(),
                    t.actif,
                )
                .execute(tx.connexion())
                .await?;
                changements.push(Changement {
                    genre: "tiers",
                    id: *id,
                    cree: true,
                    donnees: Vec::new(),
                    champs: vec![
                        ("nom", None, Some(t.nom.to_string())),
                        ("type", None, Some(t.genre.nom().to_owned())),
                        (
                            "compte_collectif",
                            None,
                            Some(t.compte_collectif.to_string()),
                        ),
                    ],
                });
            }
            Some(avant) if avant == t => {}
            Some(avant) => {
                if (avant.genre, &avant.compte_collectif) != (t.genre, &t.compte_collectif) {
                    return Err(immuable());
                }
                sqlx::query!(
                    "UPDATE tiers SET nom = $4, actif = $5 \
                     WHERE cabinet_id = $1 AND dossier_id = $2 AND id = $3",
                    cabinet,
                    dossier,
                    id,
                    t.nom.as_str(),
                    t.actif,
                )
                .execute(tx.connexion())
                .await?;
                changements.push(Changement {
                    genre: "tiers",
                    id: *id,
                    cree: false,
                    donnees: Vec::new(),
                    champs: [
                        champ("nom", &avant.nom, &t.nom),
                        champ("actif", &avant.actif, &t.actif),
                    ]
                    .into_iter()
                    .flatten()
                    .collect(),
                });
            }
        }
    }
    Ok(changements)
}

#[cfg(test)]
mod tests;
