//! Référentiel des dossiers de test (PR 11) : les dossiers créés par [`super::creer_cabinet`]
//! (super-utilisateur, sans l'API) reçoivent le référentiel officiel par le code de production
//! ([`api::base::referentiel::instancier`]), puis un sous-compte et un tiers fixes par
//! [`api::base::referentiel::muter`], seule voie de mutation.
//!
//! Aucun numéro de compte écrit en dur : tous sont tirés du référentiel officiel
//! (`syscohada`) selon leurs caractères (collectif, trésorerie, mouvement).

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::sync::OnceLock;

use api::base::referentiel::{
    EtatReferentiel, Instanciation, Modele, Ouverture, Tiers, TypeTiers, charger, instancier,
    muter, types_de_tiers_admis,
};
use api::base::{ContexteTenant, IdCabinet, IdDossier, IdUtilisateur, ouvrir_transaction};
use compta_core::{Compte, NumeroCompte, TexteCourt};
use sqlx::PgPool;
use uuid::Uuid;

/// Comptes du référentiel officiel choisis pour les tests.
pub struct Comptes {
    /// Compte de mouvement ni collectif ni de trésorerie : parent des sous-comptes de test.
    pub base: NumeroCompte,
    /// Compte de mouvement admis en trésorerie, autre que celui du journal de caisse.
    pub banque: NumeroCompte,
    /// Compte de mouvement collectif qui admet un client (fiche 41) : compte des tiers de test.
    pub collectif: NumeroCompte,
    /// Compte de mouvement collectif qui admet un fournisseur (fiche 40).
    pub fournisseur: NumeroCompte,
    /// Compte du journal de caisse (données de `syscohada`).
    pub caisse: NumeroCompte,
}

/// Les comptes de test, tirés une fois du référentiel officiel.
pub fn comptes() -> &'static Comptes {
    static COMPTES: OnceLock<Comptes> = OnceLock::new();
    COMPTES.get_or_init(|| {
        let etat = Modele::officiel().unwrap().etat();
        let admet = |c: &Compte, genre: TypeTiers| {
            c.est_collectif()
                && types_de_tiers_admis(etat, c.numero()).is_some_and(|t| t.contains(&genre))
        };
        let plan = &etat.plan;
        let caisse = etat
            .journaux
            .journaux()
            .find_map(|j| j.compte_tresorerie().cloned())
            .unwrap();
        let mouvement = |c: &&Compte| plan.compte_de_mouvement_actif(c.numero()).is_ok();
        let premier = |filtre: &dyn Fn(&Compte) -> bool| {
            plan.comptes()
                .filter(mouvement)
                .find(|c| filtre(c))
                .unwrap()
                .numero()
                .clone()
        };
        Comptes {
            base: premier(&|c| !c.est_collectif() && !c.est_admis_en_journal_de_tresorerie()),
            banque: premier(&|c| c.est_admis_en_journal_de_tresorerie() && *c.numero() != caisse),
            collectif: premier(&|c| admet(c, TypeTiers::Client)),
            fournisseur: premier(&|c| admet(c, TypeTiers::Fournisseur)),
            caisse,
        }
    })
}

/// Numéro du sous-compte fixe d'un dossier de test.
pub fn sous_compte_fixe() -> NumeroCompte {
    NumeroCompte::parse(&format!("{}-FIXE", comptes().base)).unwrap()
}

/// Sous-ressources d'un dossier de test.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Fixtures {
    /// Sous-compte fixe (créé par le dossier, donc renommable).
    pub compte: Uuid,
    /// Tiers fixe.
    pub tiers: Uuid,
    /// Premier journal (par code).
    pub journal: Uuid,
    /// Premier exercice.
    pub exercice: Uuid,
    /// Première période du premier exercice (ouverte, et pas la dernière).
    pub periode: Uuid,
}

impl Fixtures {
    /// UUID de la sous-ressource `{nom}` d'un chemin de route.
    pub fn sous_ressource(&self, nom: &str) -> Uuid {
        match nom {
            "compte" => self.compte,
            "tiers" => self.tiers,
            "periode" => self.periode,
            autre => panic!("sous-ressource de test inconnue : {autre}"),
        }
    }
}

/// Dote un dossier de test existant du référentiel officiel (dossier repris sur l'année en
/// cours), d'un sous-compte et d'un tiers fixes.
pub async fn doter(
    pool: &PgPool,
    cabinet: IdCabinet,
    dossier: IdDossier,
    utilisateur: IdUtilisateur,
) -> Fixtures {
    let aujourd_hui = time::OffsetDateTime::now_utc().date();
    let instanciation = Instanciation::preparer(
        Modele::officiel().unwrap(),
        Ouverture::Reprise {
            annee: aujourd_hui.year(),
        },
        aujourd_hui,
    )
    .unwrap();
    let mut tx = ouvrir_transaction(pool, ContexteTenant::cabinet(cabinet, utilisateur))
        .await
        .unwrap();
    instancier(&mut tx, dossier, &instanciation).await.unwrap();
    let numero = sous_compte_fixe();
    let compte = muter(&mut tx, |_, etat: &mut EtatReferentiel| {
        etat.plan
            .ajouter_sous_compte(numero, TexteCourt::parse("Sous-compte fixe").unwrap())?;
        Ok(())
    })
    .await
    .unwrap()
    .changements[0]
        .id;
    let tiers = Uuid::new_v4();
    muter(&mut tx, |_, etat: &mut EtatReferentiel| {
        etat.tiers.insert(
            tiers,
            Tiers::nouveau(
                TexteCourt::parse("Tiers fixe").unwrap(),
                TypeTiers::Client,
                comptes().collectif.clone(),
            ),
        );
        Ok(())
    })
    .await
    .unwrap();
    let relu = charger(&mut tx).await.unwrap().unwrap();
    let journal = relu
        .id_journal(relu.etat().journaux.journaux().next().unwrap().code())
        .unwrap();
    let premier = &relu.exercices().exercices()[0];
    let exercice = relu.id_exercice(premier.code()).unwrap();
    let periode = relu.id_periode(premier.periodes()[0].debut()).unwrap();
    tx.confirmer().await.unwrap();
    Fixtures {
        compte,
        tiers,
        journal,
        exercice,
        periode,
    }
}

/// Corps de la création d'un dossier repris sur l'année en cours (horloge du système, celle
/// de l'application des tests).
pub fn corps_creation(nom: &str) -> String {
    format!(
        r#"{{"nom":"{nom}","pays":"SN","nature":"reprise","annee":{}}}"#,
        time::OffsetDateTime::now_utc().year()
    )
}

/// Corps d'exemple des mutations du module `referentiel` (`commun::serveur::corps_de`).
pub fn corps(cas: api::serveur::routes::referentiel::Cas) -> &'static str {
    use api::serveur::routes::referentiel::Cas;
    static COMPTE: OnceLock<String> = OnceLock::new();
    static JOURNAL: OnceLock<String> = OnceLock::new();
    static TIERS: OnceLock<String> = OnceLock::new();
    match cas {
        Cas::CreerCompte => COMPTE.get_or_init(|| {
            format!(
                r#"{{"numero":"{}-TEST","intitule":"Sous-compte du test","genre":"sous_compte"}}"#,
                comptes().base
            )
        }),
        Cas::RenommerCompte => r#"{"intitule":"Sous-compte renommé"}"#,
        Cas::CreerJournal => JOURNAL.get_or_init(|| {
            format!(
                r#"{{"code":"BQ1","intitule":"Banque du test","type":"tresorerie","compte_tresorerie":"{}"}}"#,
                comptes().banque
            )
        }),
        Cas::CreerTiers => TIERS.get_or_init(|| {
            format!(
                r#"{{"nom":"Client du test","type":"client","compte_collectif":"{}"}}"#,
                comptes().collectif
            )
        }),
        Cas::ModifierTiers => r#"{"nom":"Tiers renommé"}"#,
        Cas::DesactiverCompte
        | Cas::ReactiverCompte
        | Cas::VerrouillerPeriode
        | Cas::OuvrirExerciceSuivant => "{}",
        Cas::DeverrouillerPeriode => r#"{"motif":"Correction d'une erreur de saisie"}"#,
        Cas::CloturerPeriode => r#"{"confirmation":"irreversible"}"#,
        Cas::ListerComptes | Cas::ListerJournaux | Cas::ListerTiers | Cas::ListerExercices => "",
    }
}
