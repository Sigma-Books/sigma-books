//! Tests des politiques Cedar : cas autorisés et refusés (spec §20, Q5,
//! ADR 0011, ADR 0012, ADR 0014 ; F1.7, F1.10, F1.14, F1.15, F1.16).

// Fichier de test : `expect` hors des fonctions `#[test]` (fixtures).
#![allow(clippy::expect_used)]

use std::collections::BTreeSet;
use std::str::FromStr;
use std::sync::LazyLock;

use authz::{
    AccesDossier, Action, Appartenance, Autorisation, Cabinet, Contexte, DUREE_FRAICHEUR, Decision,
    Dossier, EcritureBrouillon, EcritureValidee, Entites, ErreurIdentifiant, IdAccesDossier,
    IdAppartenance, IdCabinet, IdDossier, IdEcriture, IdUtilisateur, Ressource, Role, Session,
    TypeRessource,
};
use proptest::prelude::*;
use time::OffsetDateTime;
use time::macros::datetime;
use uuid::Uuid;

static AUTORISATION: LazyLock<Autorisation> =
    LazyLock::new(|| Autorisation::charger().expect("schéma et politiques valides"));

const OUI: bool = true;
const NON: bool = false;

/// Tableau attendu du lot L2, recopié de la spec §20 (colonnes : administrateur,
/// valideur, collaborateur, auditeur, client). Conditions : appartenance
/// active au cabinet de la ressource, dossier affecté, **sans** délégation
/// de validation, session fraîche, écriture saisie par un autre
/// utilisateur, séparation des tâches active.
const MATRICE_L2: &[(Action, [bool; 5])] = &[
    // Voir le cabinet (choix du cabinet actif, ADR 0012) : tout rôle du cabinet.
    (Action::AccederCabinet, [OUI, OUI, OUI, OUI, NON]),
    // « Gérer cabinet, utilisateurs, facturation »
    (Action::GererCabinet, [OUI, NON, NON, NON, NON]),
    (Action::ModifierDroits, [OUI, NON, NON, NON, NON]),
    (Action::ReinitialiserAcces, [OUI, NON, NON, NON, NON]),
    // Contester la réinitialisation de son accès : tout rôle du cabinet
    (Action::AnnulerSaReinitialisation, [OUI, OUI, OUI, OUI, NON]),
    (
        Action::ActiverSeparationDesTaches,
        [OUI, NON, NON, NON, NON],
    ),
    (
        Action::DesactiverSeparationDesTaches,
        [OUI, NON, NON, NON, NON],
    ),
    // Audit de portée cabinet : administrateur seul (ADR 0016)
    (Action::ConsulterAuditCabinet, [OUI, NON, NON, NON, NON]),
    (Action::VerifierAuditCabinet, [OUI, NON, NON, NON, NON]),
    (Action::ExporterAuditCabinet, [OUI, NON, NON, NON, NON]),
    // « Créer un dossier, reprise de données »
    (Action::CreerDossier, [OUI, OUI, NON, NON, NON]),
    // Voir le dossier : quiconque y a une action (« Consulter éditions »).
    (Action::AccederDossier, [OUI, OUI, OUI, OUI, NON]),
    // « Paramétrer plan, journaux, fiscalité »
    (Action::ParametrerDossier, [OUI, OUI, NON, NON, NON]),
    // Modifier le contexte du dossier (ADR 0027) : administrateur seul
    (Action::ModifierContexteDossier, [OUI, NON, NON, NON, NON]),
    // Paramètres fiscaux du dossier (ADR 0030) : lecture pour tout rôle qui voit le dossier,
    // création pour l'administrateur seul (session fraîche)
    (
        Action::ConsulterParametresFiscaux,
        [OUI, OUI, OUI, OUI, NON],
    ),
    (
        Action::CreerVersionParametresFiscaux,
        [OUI, NON, NON, NON, NON],
    ),
    // « Déposer des pièces » (client : portail, L4)
    (Action::DeposerPiece, [OUI, OUI, OUI, NON, NON]),
    // « Saisir, corriger dans le brouillard »
    (Action::SaisirBrouillard, [OUI, OUI, OUI, NON, NON]),
    (Action::CorrigerBrouillard, [OUI, OUI, OUI, NON, NON]),
    // « Valider une écriture » (collaborateur : selon délégation, absente ici)
    (Action::ValiderEcriture, [OUI, OUI, NON, NON, NON]),
    // « Validation par lot »
    (Action::ValiderParLot, [OUI, OUI, NON, NON, NON]),
    // « Contre-passer »
    (Action::ContrePasser, [OUI, OUI, NON, NON, NON]),
    // « Liquider la TVA, verrouiller une période »
    (Action::VerrouillerPeriode, [OUI, OUI, NON, NON, NON]),
    // « Déverrouiller une période »
    (Action::DeverrouillerPeriode, [OUI, NON, NON, NON, NON]),
    // « Clôturer un exercice » ; clôture informatique d'une période : idem
    (Action::CloturerExercice, [OUI, OUI, NON, NON, NON]),
    (Action::CloturerPeriode, [OUI, OUI, NON, NON, NON]),
    // Ouvrir l'exercice suivant : décision du mainteneur du 2026-11-19 (#172)
    (Action::OuvrirExerciceSuivant, [OUI, OUI, NON, NON, NON]),
    // « Consulter éditions et états »
    (Action::ConsulterEditions, [OUI, OUI, OUI, OUI, NON]),
    // « Vérifier l'intégrité, consulter l'audit »
    (Action::VerifierIntegrite, [OUI, OUI, NON, OUI, NON]),
    (Action::ConsulterAudit, [OUI, OUI, NON, OUI, NON]),
    // « Exporter (FEC, profils) »
    (Action::Exporter, [OUI, OUI, NON, OUI, NON]),
    (Action::ExporterComplet, [OUI, OUI, NON, OUI, NON]),
    (Action::ExporterIntegrite, [OUI, OUI, NON, OUI, NON]),
    // Lignes des lots ultérieurs : aucune politique avant leur lot.
    // Cible du §20 : facturation = administrateur ; reprise = admin, valideur (L8) ;
    // règle apprise = admin, valideur (L4) ; TVA = admin, valideur (L5) ; fin
    // d'exercice = admin, valideur, collaborateur au brouillard (L6) ; demandes =
    // client (L4). Lettrer : lot L3, voir `MATRICE_LETTRAGE`.
    (Action::GererFacturation, [NON, NON, NON, NON, NON]),
    (Action::ReprendreDonnees, [NON, NON, NON, NON, NON]),
    (Action::ActiverRegleApprise, [NON, NON, NON, NON, NON]),
    (Action::LiquiderTva, [NON, NON, NON, NON, NON]),
    (Action::TravauxFinExercice, [NON, NON, NON, NON, NON]),
    (Action::RepondreDemande, [NON, NON, NON, NON, NON]),
];

// Lot L3 : une matrice par domaine (voie de travail), pour que deux voies n'éditent pas les
// mêmes lignes. La spec §20 n'a pas de ligne pour ces actions : rôles proposés par le plan
// (`docs/lots/L3.md`, point C6 ; ADR 0031 à 0034), rattachés à la ligne du §20 la plus proche.

/// Domaine « tiers » (ADR 0031, 0033).
const MATRICE_TIERS: &[(Action, [bool; 5])] = &[
    // Lecture : comme « Consulter éditions et états » ; coordonnées bancaires sans l'auditeur
    // (minimisation, VP17).
    (Action::ConsulterTiers, [OUI, OUI, OUI, OUI, NON]),
    (
        Action::ConsulterCoordonneesBancairesTiers,
        [OUI, OUI, OUI, NON, NON],
    ),
    // Création, conditions de règlement : comme « Paramétrer ».
    (Action::CreerTiers, [OUI, OUI, NON, NON, NON]),
    (
        Action::ParametrerConditionsReglement,
        [OUI, OUI, NON, NON, NON],
    ),
    // Fiche courante, archivage (libre : ADR 0031 point 5, décision du mainteneur du
    // 2026-11-27) et révision d'échéance : comme « Saisir, corriger dans le brouillard ».
    (Action::ModifierTiers, [OUI, OUI, OUI, NON, NON]),
    (Action::ArchiverTiers, [OUI, OUI, OUI, NON, NON]),
    (Action::ReviserEcheance, [OUI, OUI, OUI, NON, NON]),
    // Fusion, rupture de fusion, anonymisation : administrateur, session fraîche (la rupture :
    // par interprétation de l'ADR 0031 point 6, à confirmer).
    (Action::FusionnerTiers, [OUI, NON, NON, NON, NON]),
    (Action::RompreFusionTiers, [OUI, NON, NON, NON, NON]),
    (Action::AnonymiserTiers, [OUI, NON, NON, NON, NON]),
    // Éditions de tiers : comme « Consulter éditions et états ».
    (Action::ConsulterEditionsTiers, [OUI, OUI, OUI, OUI, NON]),
];

/// Domaine « lettrage » (ADR 0032).
const MATRICE_LETTRAGE: &[(Action, [bool; 5])] = &[
    // « Lettrer / délettrer »
    (Action::Lettrer, [OUI, OUI, OUI, NON, NON]),
    (Action::Delettrer, [OUI, OUI, OUI, NON, NON]),
    // Règles automatiques : comme « Paramétrer ».
    (Action::ParametrerLettrage, [OUI, OUI, NON, NON, NON]),
];

/// Domaine « trésorerie » (ADR 0034).
const MATRICE_TRESORERIE: &[(Action, [bool; 5])] = &[
    // Lecture, auditeur compris : comme « Consulter éditions et états ».
    (Action::ConsulterTresorerie, [OUI, OUI, OUI, OUI, NON]),
    // Comptes de trésorerie, profils et règles de relevé : comme « Paramétrer ».
    (Action::ParametrerTresorerie, [OUI, OUI, NON, NON, NON]),
    (Action::ParametrerReleves, [OUI, OUI, NON, NON, NON]),
    // Import et pointage : comme « Saisir, corriger dans le brouillard ».
    (Action::DeposerReleve, [OUI, OUI, OUI, NON, NON]),
    (Action::ImporterReleve, [OUI, OUI, OUI, NON, NON]),
    (Action::ArreterCaisse, [OUI, OUI, OUI, NON, NON]),
    (Action::PointerReleve, [OUI, OUI, OUI, NON, NON]),
    (Action::RomprePointage, [OUI, OUI, OUI, NON, NON]),
    (Action::ProposerPrelevement, [OUI, OUI, OUI, NON, NON]),
    // Figement : valideur (et administrateur).
    (Action::FigerRapprochement, [OUI, OUI, NON, NON, NON]),
];

/// Les matrices de tous les domaines.
const MATRICES: &[&[(Action, [bool; 5])]] = &[
    MATRICE_L2,
    MATRICE_TIERS,
    MATRICE_LETTRAGE,
    MATRICE_TRESORERIE,
];

/// Actions du lot L3 : celles des matrices de ses domaines.
const MATRICES_L3: &[&[(Action, [bool; 5])]] =
    &[MATRICE_TIERS, MATRICE_LETTRAGE, MATRICE_TRESORERIE];

/// Toutes les lignes attendues, domaine après domaine.
fn matrice() -> impl Iterator<Item = &'static (Action, [bool; 5])> {
    MATRICES.iter().flat_map(|m| m.iter())
}

fn actions_l3() -> impl Iterator<Item = Action> {
    MATRICES_L3.iter().flat_map(|m| m.iter()).map(|(a, _)| *a)
}

/// Actions qui exigent une session fraîche (spec §21 ; ADR 0012).
const SENSIBLES: &[Action] = &[
    Action::ValiderParLot,
    Action::CloturerPeriode,
    Action::CloturerExercice,
    Action::OuvrirExerciceSuivant,
    Action::DeverrouillerPeriode,
    Action::ExporterComplet,
    Action::ExporterIntegrite,
    Action::ExporterAuditCabinet,
    Action::ModifierDroits,
    Action::ReinitialiserAcces,
    Action::DesactiverSeparationDesTaches,
    Action::CreerVersionParametresFiscaux,
    // Lot L3 (ADR 0031, plan C6).
    Action::FusionnerTiers,
    Action::RompreFusionTiers,
    Action::AnonymiserTiers,
];

// ---------------------------------------------------------------------------
// Identifiants : un UUID stable par nom lisible (FNV-1a, jamais nul).
// ---------------------------------------------------------------------------

fn uuid(nom: &str) -> Uuid {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in nom.bytes() {
        h = (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3);
    }
    Uuid::from_u128((u128::from(h) << 64) | 0x4000_8000_0000_0001)
}

fn ut(nom: &str) -> IdUtilisateur {
    IdUtilisateur::new(uuid(nom)).expect("UUID valide")
}
fn cab(nom: &str) -> IdCabinet {
    IdCabinet::new(uuid(nom)).expect("UUID valide")
}
fn app(nom: &str) -> IdAppartenance {
    IdAppartenance::new(uuid(nom)).expect("UUID valide")
}
fn dos(nom: &str) -> IdDossier {
    IdDossier::new(uuid(nom)).expect("UUID valide")
}
fn acc(nom: &str) -> IdAccesDossier {
    IdAccesDossier::new(uuid(nom)).expect("UUID valide")
}
fn ecr(nom: &str) -> IdEcriture {
    IdEcriture::new(uuid(nom)).expect("UUID valide")
}

// ---------------------------------------------------------------------------
// Sessions (ADR 0012)
// ---------------------------------------------------------------------------

const T0: OffsetDateTime = datetime!(2026-11-19 09:00:00 UTC);

fn fraiche() -> Session {
    Session::depuis(T0, T0)
}

fn ordinaire() -> Session {
    Session::depuis(T0, T0 + DUREE_FRAICHEUR)
}

// ---------------------------------------------------------------------------
// Monde de test : deux cabinets A et B, deux dossiers chacun.
// ---------------------------------------------------------------------------

fn cabinet(nom: &str, separation_des_taches: bool) -> Cabinet {
    Cabinet {
        id: cab(nom),
        separation_des_taches,
    }
}

fn appartenance(id: &str, utilisateur: &str, cabinet: &str, role: Role) -> Appartenance {
    Appartenance {
        id: app(id),
        utilisateur: ut(utilisateur),
        cabinet: cab(cabinet),
        role,
        actif: true,
    }
}

fn dossier(id: &str, cabinet: &str) -> Dossier {
    Dossier {
        id: dos(id),
        cabinet: cab(cabinet),
    }
}

fn acces(id: &str, appartenance: &str, dossier: &str, delegation: bool) -> AccesDossier {
    AccesDossier {
        id: acc(id),
        appartenance: app(appartenance),
        dossier: dos(dossier),
        delegation_validation: delegation,
    }
}

fn brouillon(id: &str, dossier: &str, createur: &str, correcteurs: &[&str]) -> EcritureBrouillon {
    EcritureBrouillon {
        id: ecr(id),
        dossier: dos(dossier),
        createur: ut(createur),
        correcteurs: correcteurs.iter().map(|c| ut(c)).collect(),
    }
}

fn validee(id: &str, dossier: &str) -> EcritureValidee {
    EcritureValidee {
        id: ecr(id),
        dossier: dos(dossier),
    }
}

/// Ressource de l'action dans le dossier `d` (ou son cabinet `c`).
fn ressource_dans(action: Action, c: &str, d: &str) -> Ressource {
    match action.type_ressource() {
        TypeRessource::Cabinet => Ressource::Cabinet(cab(c)),
        TypeRessource::Dossier => Ressource::Dossier(dos(d)),
        TypeRessource::EcritureBrouillon => {
            Ressource::EcritureBrouillon(ecr(&format!("brouillon-{d}")))
        }
        TypeRessource::EcritureValidee => Ressource::EcritureValidee(ecr(&format!("validee-{d}"))),
    }
}

/// Ressource de l'action dans le cabinet `c` (dossier `{c}1`).
fn ressource(action: Action, c: &str) -> Ressource {
    ressource_dans(action, c, &format!("{c}1"))
}

/// Toutes les entités des deux cabinets, hors appartenances et affectations.
/// Les brouillons sont créés par `createur` et corrigés par `correcteurs`.
fn monde(separation_a: bool, separation_b: bool, createur: &str, correcteurs: &[&str]) -> Entites {
    let mut entites = Entites::new()
        .avec(cabinet("A", separation_a))
        .avec(cabinet("B", separation_b));
    for c in ["A", "B"] {
        for n in 1..=2 {
            let d = format!("{c}{n}");
            entites.ajouter(dossier(&d, c));
            entites.ajouter(brouillon(
                &format!("brouillon-{d}"),
                &d,
                createur,
                correcteurs,
            ));
            entites.ajouter(validee(&format!("validee-{d}"), &d));
        }
    }
    entites
}

/// Part de l'utilisateur agissant (`moi`) dans le brouillon.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Part {
    Aucune,
    Createur,
    Correcteur,
}

/// Paramètres d'un cas : l'utilisateur `moi` agit au titre de son
/// appartenance au cabinet A.
// Un booléen par paramètre du cas : plus lisible ici que des énumérations.
#[allow(clippy::struct_excessive_bools)]
#[derive(Clone, Copy)]
struct Cas {
    role: Role,
    actif: bool,
    affecte: bool,
    delegation: bool,
    session: Session,
    separation: bool,
    part: Part,
}

impl Cas {
    fn nominal(role: Role) -> Self {
        Self {
            role,
            actif: true,
            affecte: true,
            delegation: false,
            session: fraiche(),
            separation: true,
            part: Part::Aucune,
        }
    }

    fn principal(self) -> Appartenance {
        Appartenance {
            actif: self.actif,
            ..appartenance("moi@A", "moi", "A", self.role)
        }
    }

    fn entites_et_contexte(self) -> (Entites, Contexte) {
        let (createur, correcteurs): (&str, &[&str]) = match self.part {
            Part::Aucune => ("autre", &[]),
            Part::Createur => ("moi", &[]),
            Part::Correcteur => ("autre", &["moi"]),
        };
        let mut entites = monde(self.separation, true, createur, correcteurs);
        let mut contexte = Contexte::new(self.session);
        if self.affecte {
            entites.ajouter(acces("acces-moi-A1", "moi@A", "A1", self.delegation));
            contexte = contexte.avec_acces(acc("acces-moi-A1"));
        }
        (entites, contexte)
    }

    /// Décision sur la ressource de `action` dans le dossier A1.
    fn decider(self, action: Action) -> Decision {
        let (entites, contexte) = self.entites_et_contexte();
        AUTORISATION.decider(
            &self.principal(),
            action,
            ressource(action, "A"),
            &entites,
            &contexte,
        )
    }

    /// Validation par lot, dans le dossier A1, des brouillons nommés.
    fn decider_lot(self, brouillons: &[&str]) -> Decision {
        let (entites, contexte) = self.entites_et_contexte();
        let ids: Vec<IdEcriture> = brouillons.iter().map(|b| ecr(b)).collect();
        AUTORISATION.decider_lot(&self.principal(), dos("A1"), &ids, &entites, &contexte)
    }
}

fn colonne(role: Role) -> usize {
    match role {
        Role::Administrateur => 0,
        Role::Valideur => 1,
        Role::Collaborateur => 2,
        Role::Auditeur => 3,
        Role::Client => 4,
    }
}

fn attendu(action: Action, role: Role) -> bool {
    matrice()
        .find(|(a, _)| *a == action)
        .map(|(_, ligne)| ligne[colonne(role)])
        .expect("action présente dans la matrice")
}

const fn decision(autorise: bool) -> Decision {
    if autorise {
        Decision::Autorise
    } else {
        Decision::Refuse
    }
}

// ---------------------------------------------------------------------------
// Matrice rôle × action : §20 pour le lot L2 (`MATRICE_L2`), plan L3 point C6
// pour les domaines de L3 (le §20 n'a pas de ligne pour eux)
// ---------------------------------------------------------------------------

#[test]
fn la_matrice_couvre_chaque_action_une_fois() {
    assert_eq!(matrice().count(), Action::TOUTES.len());
    for action in Action::TOUTES {
        assert_eq!(
            matrice().filter(|(a, _)| a == action).count(),
            1,
            "{action:?}"
        );
    }
}

/// Décisions obtenues = matrices attendues : `MATRICE_L2` recopie le §20, les matrices de L3
/// recopient les rôles du plan L3 (point C6) et les décisions du mainteneur, faute de ligne au §20.
#[test]
fn la_matrice_generee_est_celle_attendue() {
    let mut ecarts = Vec::new();
    for (action, ligne) in matrice() {
        for role in Role::TOUS {
            let obtenue = Cas::nominal(role).decider(*action);
            if obtenue != decision(ligne[colonne(role)]) {
                ecarts.push(format!("{action:?} / {role:?} : {obtenue:?}"));
            }
        }
    }
    assert!(
        ecarts.is_empty(),
        "écarts aux matrices attendues : {ecarts:#?}"
    );
}

// ---------------------------------------------------------------------------
// Lot L3 : tiers, lettrage, trésorerie (plan C6 ; ADR 0031 à 0034)
// ---------------------------------------------------------------------------

#[test]
fn chaque_action_de_l3_est_autorisee_a_un_role_et_refusee_aux_autres() {
    // Aucune action de L3 sans politique (sinon elle serait refusée à tous, en silence) ; aucune
    // ouverte au client (portail : L4) ; aucune écriture ouverte à l'auditeur.
    for (action, ligne) in MATRICES_L3.iter().flat_map(|m| m.iter()) {
        assert!(ligne.contains(&OUI), "{action:?} : aucun rôle autorisé");
        assert!(ligne.contains(&NON), "{action:?} : aucun rôle refusé");
        assert!(
            !ligne[colonne(Role::Client)],
            "{action:?} : client autorisé"
        );
        for role in Role::TOUS {
            assert_eq!(
                Cas::nominal(role).decider(*action),
                decision(ligne[colonne(role)]),
                "{role:?} {action:?}"
            );
        }
    }
}

/// Actions déclarées par les fragments de schéma des domaines de L3 (`tiers`, `lettrage`,
/// `tresorerie`) : source unique de la frontière entre L2 et L3.
fn actions_des_fragments_l3() -> BTreeSet<String> {
    authz::SCHEMA
        .iter()
        .filter(|(nom, _)| {
            [
                "schema/tiers.cedarschema",
                "schema/lettrage.cedarschema",
                "schema/tresorerie.cedarschema",
            ]
            .contains(nom)
        })
        .flat_map(|(_, texte)| texte.lines())
        .filter_map(|ligne| ligne.trim_start().strip_prefix("action "))
        .map(|reste| {
            reste
                .split_whitespace()
                .next()
                .unwrap_or_default()
                .to_owned()
        })
        .collect()
}

#[test]
fn la_frontiere_entre_les_matrices_de_l2_et_de_l3_est_celle_du_schema() {
    // Une action de L3 classée par erreur dans `MATRICE_L2` échapperait aux tests propres à L3 :
    // les matrices de L3 doivent contenir exactement les actions des fragments de L3, et
    // `MATRICE_L2` aucune.
    let fragments = actions_des_fragments_l3();
    assert!(!fragments.is_empty());
    let matrices_l3: BTreeSet<String> = actions_l3().map(|a| a.nom().to_owned()).collect();
    assert_eq!(matrices_l3, fragments);
    for (action, _) in MATRICE_L2 {
        assert!(
            !fragments.contains(action.nom()),
            "{action:?} : action de L3 dans MATRICE_L2"
        );
    }
}

#[test]
fn les_actions_de_l3_portent_sur_le_dossier_jamais_sur_une_ecriture() {
    // Lettrage, échéance, pointage et fusion n'écrivent que dans leurs tables (règle absolue 5,
    // plan P3) : aucune action de L3 ne s'applique à une écriture, validée ou non.
    for action in actions_l3() {
        assert_eq!(
            action.type_ressource(),
            TypeRessource::Dossier,
            "{action:?}"
        );
    }
}

#[test]
fn les_actions_de_l3_sont_refusees_dans_un_dossier_etranger() {
    for action in actions_l3() {
        for role in Role::TOUS {
            // Dossier d'un autre cabinet, avec une affectation forgée vers lui : refusé, par le
            // cloisonnement.
            let entites =
                monde(false, false, "autre", &[]).avec(acces("acces-forge", "moi@A", "B1", true));
            let verdict = AUTORISATION.verdict(
                &appartenance("moi@A", "moi", "A", role),
                action,
                Ressource::Dossier(dos("B1")),
                &entites,
                &Contexte::new(fraiche()).avec_acces(acc("acces-forge")),
            );
            assert_eq!(verdict.decision, Decision::Refuse, "{role:?} {action:?}");
            assert!(
                verdict
                    .politiques
                    .contains(&"invariant-cloisonnement-cabinet".to_string()),
                "{role:?} {action:?} : {verdict:?}"
            );
            // Dossier du même cabinet, non affecté : refusé, sauf à l'administrateur.
            let non_affecte = Cas {
                affecte: false,
                ..Cas::nominal(role)
            };
            let attendue = role == Role::Administrateur && attendu(action, role);
            assert_eq!(
                non_affecte.decider(action),
                decision(attendue),
                "{role:?} {action:?} non affecté"
            );
        }
        // Affecté à un autre dossier du cabinet : rien dans celui-ci.
        if attendu(action, Role::Valideur) {
            let a2 = acces("acces-moi-A2", "moi@A", "A2", true);
            assert_eq!(
                valideur_avec_acces(action, a2, monde(true, true, "autre", &[])),
                Decision::Refuse,
                "{action:?} affecté à A2"
            );
        }
    }
}

#[test]
fn l_auditeur_lit_les_tiers_sans_leurs_coordonnees_bancaires() {
    let auditeur = Cas::nominal(Role::Auditeur);
    for action in [
        Action::ConsulterTiers,
        Action::ConsulterEditionsTiers,
        Action::ConsulterTresorerie,
    ] {
        assert_eq!(auditeur.decider(action), Decision::Autorise, "{action:?}");
    }
    assert_eq!(
        auditeur.decider(Action::ConsulterCoordonneesBancairesTiers),
        Decision::Refuse
    );
    // Le collaborateur, qui tient la fiche, les lit.
    assert_eq!(
        Cas::nominal(Role::Collaborateur).decider(Action::ConsulterCoordonneesBancairesTiers),
        Decision::Autorise
    );
}

#[test]
fn fusion_et_anonymisation_d_un_tiers_exigent_l_administrateur_en_session_fraiche() {
    for action in [
        Action::FusionnerTiers,
        Action::RompreFusionTiers,
        Action::AnonymiserTiers,
    ] {
        assert!(SENSIBLES.contains(&action), "{action:?}");
        let admin = Cas::nominal(Role::Administrateur);
        assert_eq!(admin.decider(action), Decision::Autorise, "{action:?}");
        let verdict = {
            let cas = Cas {
                session: ordinaire(),
                ..admin
            };
            let (entites, contexte) = cas.entites_et_contexte();
            AUTORISATION.verdict(
                &cas.principal(),
                action,
                ressource(action, "A"),
                &entites,
                &contexte,
            )
        };
        assert_eq!(verdict.decision, Decision::Refuse, "{action:?}");
        assert!(
            verdict
                .politiques
                .contains(&"invariant-reauthentification".to_string()),
            "{action:?} : {verdict:?}"
        );
        for role in [Role::Valideur, Role::Collaborateur, Role::Auditeur] {
            assert_eq!(
                Cas::nominal(role).decider(action),
                Decision::Refuse,
                "{role:?} {action:?}"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Identifiants (UUID typés, forme canonique)
// ---------------------------------------------------------------------------

#[test]
fn les_identifiants_vides_nuls_ou_non_uuid_sont_impossibles() {
    for texte in ["", " ", "A", "pas-un-uuid", "moi@A"] {
        assert_eq!(
            IdCabinet::from_str(texte),
            Err(ErreurIdentifiant::Invalide),
            "{texte:?}"
        );
        assert_eq!(
            IdUtilisateur::from_str(texte),
            Err(ErreurIdentifiant::Invalide)
        );
        assert_eq!(
            IdAppartenance::from_str(texte),
            Err(ErreurIdentifiant::Invalide)
        );
        assert_eq!(IdDossier::from_str(texte), Err(ErreurIdentifiant::Invalide));
        assert_eq!(
            IdAccesDossier::from_str(texte),
            Err(ErreurIdentifiant::Invalide)
        );
        assert_eq!(
            IdEcriture::from_str(texte),
            Err(ErreurIdentifiant::Invalide)
        );
    }
    // Les UUID nul et maximal, valeurs par défaut typiques, sont refusés :
    // un principal et un cabinet « vides » ne peuvent plus coïncider et
    // obtenir `GererCabinet`.
    for special in [Uuid::nil(), Uuid::max()] {
        assert_eq!(IdCabinet::new(special), Err(ErreurIdentifiant::Reserve));
        assert_eq!(
            IdAppartenance::new(special),
            Err(ErreurIdentifiant::Reserve)
        );
        assert_eq!(IdUtilisateur::new(special), Err(ErreurIdentifiant::Reserve));
        assert_eq!(IdDossier::new(special), Err(ErreurIdentifiant::Reserve));
        assert_eq!(
            IdAccesDossier::new(special),
            Err(ErreurIdentifiant::Reserve)
        );
        assert_eq!(IdEcriture::new(special), Err(ErreurIdentifiant::Reserve));
        assert_eq!(
            IdCabinet::from_str(&special.hyphenated().to_string()),
            Err(ErreurIdentifiant::Reserve)
        );
    }
}

#[test]
fn la_forme_d_un_identifiant_est_canonique() {
    let minuscules = "6f1c2a9e-0b7d-4c3e-9a51-2d8e4f6a7b10";
    let majuscules = minuscules.to_uppercase();
    let a = IdUtilisateur::from_str(minuscules).expect("UUID");
    let b = IdUtilisateur::from_str(&majuscules).expect("UUID");
    let c = IdUtilisateur::from_str(&format!("{{{majuscules}}}")).expect("UUID");
    assert_eq!(a, b);
    assert_eq!(a, c);
    assert_eq!(b.to_string(), minuscules);
}

#[test]
fn la_casse_d_un_identifiant_ne_contourne_pas_la_separation_des_taches() {
    let minuscules = "6f1c2a9e-0b7d-4c3e-9a51-2d8e4f6a7b10";
    let createur = IdUtilisateur::from_str(&minuscules.to_uppercase()).expect("UUID");
    let principal = Appartenance {
        utilisateur: IdUtilisateur::from_str(minuscules).expect("UUID"),
        ..appartenance("moi@A", "moi", "A", Role::Administrateur)
    };
    let entites = Entites::new()
        .avec(cabinet("A", true))
        .avec(dossier("A1", "A"))
        .avec(EcritureBrouillon {
            id: ecr("brouillon-A1"),
            dossier: dos("A1"),
            createur,
            correcteurs: vec![],
        });
    assert_eq!(
        AUTORISATION.decider(
            &principal,
            Action::ValiderEcriture,
            Ressource::EcritureBrouillon(ecr("brouillon-A1")),
            &entites,
            &Contexte::new(fraiche()),
        ),
        Decision::Refuse
    );
}

// ---------------------------------------------------------------------------
// Cloisonnement entre cabinets (F1.7, F1.14)
// ---------------------------------------------------------------------------

#[test]
fn un_utilisateur_du_cabinet_a_est_refuse_sur_toute_ressource_de_b() {
    for role in Role::TOUS {
        for action in Action::TOUTES {
            // Pire cas : affectation forgée vers le dossier de B, avec
            // délégation, session fraîche, séparation désactivée dans B.
            let entites =
                monde(false, false, "autre", &[]).avec(acces("acces-forge", "moi@A", "B1", true));
            let contexte = Contexte::new(fraiche()).avec_acces(acc("acces-forge"));
            let verdict = AUTORISATION.verdict(
                &appartenance("moi@A", "moi", "A", role),
                *action,
                ressource(*action, "B"),
                &entites,
                &contexte,
            );
            assert_eq!(verdict.decision, Decision::Refuse, "{role:?} {action:?}");
            // Refus par la politique, pas par une anomalie.
            assert!(
                verdict
                    .politiques
                    .contains(&"invariant-cloisonnement-cabinet".to_string()),
                "{role:?} {action:?} : {verdict:?}"
            );
        }
    }
}

#[test]
fn un_membre_de_deux_cabinets_n_agit_que_selon_son_role_dans_le_cabinet_de_la_ressource() {
    // `moi` est administrateur de A et collaborateur de B, affecté à B1.
    let au_titre_de_a = appartenance("moi@A", "moi", "A", Role::Administrateur);
    let au_titre_de_b = appartenance("moi@B", "moi", "B", Role::Collaborateur);
    let entites = monde(true, true, "autre", &[]).avec(acces("acces-moi-B1", "moi@B", "B1", false));
    let contexte = Contexte::new(fraiche()).avec_acces(acc("acces-moi-B1"));
    for action in Action::TOUTES {
        let r = ressource(*action, "B");
        // Au titre de A : rien dans B, même administrateur.
        assert_eq!(
            AUTORISATION.decider(&au_titre_de_a, *action, r, &entites, &contexte),
            Decision::Refuse,
            "{action:?} au titre de A"
        );
        // Au titre de B : les droits d'un collaborateur, pas plus.
        assert_eq!(
            AUTORISATION.decider(&au_titre_de_b, *action, r, &entites, &contexte),
            decision(attendu(*action, Role::Collaborateur)),
            "{action:?} au titre de B"
        );
    }
}

// ---------------------------------------------------------------------------
// Audit de portée cabinet (ADR 0016)
// ---------------------------------------------------------------------------

#[test]
fn l_audit_du_cabinet_est_refuse_au_valideur_et_a_l_auditeur_affectes() {
    for role in [Role::Valideur, Role::Auditeur] {
        // Affecté au dossier A1, session fraîche : l'audit du dossier lui
        // est ouvert, pas celui du cabinet.
        let cas = Cas::nominal(role);
        assert_eq!(
            cas.decider(Action::ConsulterAudit),
            Decision::Autorise,
            "{role:?}"
        );
        assert_eq!(
            cas.decider(Action::VerifierIntegrite),
            Decision::Autorise,
            "{role:?}"
        );
        for action in [
            Action::ConsulterAuditCabinet,
            Action::VerifierAuditCabinet,
            Action::ExporterAuditCabinet,
        ] {
            assert_eq!(cas.decider(action), Decision::Refuse, "{role:?} {action:?}");
        }
    }
    // L'administrateur : oui, l'export seulement en session fraîche.
    let admin = Cas::nominal(Role::Administrateur);
    for action in [
        Action::ConsulterAuditCabinet,
        Action::VerifierAuditCabinet,
        Action::ExporterAuditCabinet,
    ] {
        assert_eq!(admin.decider(action), Decision::Autorise, "{action:?}");
    }
    let ordinaire = Cas {
        session: ordinaire(),
        ..admin
    };
    assert_eq!(
        ordinaire.decider(Action::ConsulterAuditCabinet),
        Decision::Autorise
    );
    assert_eq!(
        ordinaire.decider(Action::ExporterAuditCabinet),
        Decision::Refuse
    );
}

// ---------------------------------------------------------------------------
// Appartenance suspendue (ADR 0014)
// ---------------------------------------------------------------------------

#[test]
fn une_appartenance_suspendue_n_a_plus_aucune_action() {
    for role in Role::TOUS {
        for action in Action::TOUTES {
            let cas = Cas {
                actif: false,
                delegation: true,
                separation: false,
                ..Cas::nominal(role)
            };
            assert_eq!(
                cas.decider(*action),
                Decision::Refuse,
                "{role:?} {action:?}"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Affectation aux dossiers (Q5.1)
// ---------------------------------------------------------------------------

fn actions_de_dossier() -> impl Iterator<Item = Action> {
    Action::TOUTES
        .iter()
        .copied()
        .filter(|a| a.type_ressource() != TypeRessource::Cabinet)
}

#[test]
fn hors_administrateur_un_dossier_non_affecte_est_refuse() {
    for role in [
        Role::Valideur,
        Role::Collaborateur,
        Role::Auditeur,
        Role::Client,
    ] {
        for action in actions_de_dossier() {
            let cas = Cas {
                affecte: false,
                delegation: true,
                ..Cas::nominal(role)
            };
            assert_eq!(cas.decider(action), Decision::Refuse, "{role:?} {action:?}");
        }
    }
}

#[test]
fn l_administrateur_accede_a_un_dossier_non_affecte_de_son_cabinet() {
    for action in actions_de_dossier() {
        let cas = Cas {
            affecte: false,
            ..Cas::nominal(Role::Administrateur)
        };
        assert_eq!(
            cas.decider(action),
            decision(attendu(action, Role::Administrateur)),
            "{action:?}"
        );
    }
}

/// Décision pour un valideur de A sur une ressource de A1, avec l'affectation donnée.
fn valideur_avec_acces(action: Action, acces_fourni: AccesDossier, entites: Entites) -> Decision {
    let id = acces_fourni.id;
    AUTORISATION.decider(
        &appartenance("moi@A", "moi", "A", Role::Valideur),
        action,
        ressource(action, "A"),
        &entites.avec(acces_fourni),
        &Contexte::new(fraiche()).avec_acces(id),
    )
}

#[test]
fn une_affectation_a_un_autre_dossier_ne_vaut_rien() {
    for action in actions_de_dossier().filter(|a| attendu(*a, Role::Valideur)) {
        let a2 = acces("acces-moi-A2", "moi@A", "A2", true);
        assert_eq!(
            valideur_avec_acces(action, a2, monde(true, true, "autre", &[])),
            Decision::Refuse,
            "{action:?}"
        );
    }
}

#[test]
fn l_affectation_d_un_autre_utilisateur_ne_vaut_rien() {
    for action in actions_de_dossier().filter(|a| attendu(*a, Role::Valideur)) {
        let entites =
            monde(true, true, "autre", &[]).avec(appartenance("lui@A", "lui", "A", Role::Valideur));
        let sienne = acces("acces-lui-A1", "lui@A", "A1", true);
        assert_eq!(
            valideur_avec_acces(action, sienne, entites),
            Decision::Refuse,
            "{action:?}"
        );
    }
}

#[test]
fn une_affectation_annoncee_mais_non_fournie_est_refusee() {
    let contexte = Contexte::new(fraiche()).avec_acces(acc("fantome"));
    assert_eq!(
        AUTORISATION.decider(
            &appartenance("moi@A", "moi", "A", Role::Valideur),
            Action::ConsulterEditions,
            ressource(Action::ConsulterEditions, "A"),
            &monde(true, true, "autre", &[]),
            &contexte
        ),
        Decision::Refuse
    );
}

// ---------------------------------------------------------------------------
// Délégation de validation (Q5.2)
// ---------------------------------------------------------------------------

#[test]
fn un_collaborateur_valide_seulement_avec_la_delegation_du_dossier() {
    let avec = Cas {
        delegation: true,
        ..Cas::nominal(Role::Collaborateur)
    };
    assert_eq!(avec.decider(Action::ValiderEcriture), Decision::Autorise);
    assert_eq!(
        Cas::nominal(Role::Collaborateur).decider(Action::ValiderEcriture),
        Decision::Refuse
    );
    // La délégation n'ouvre ni la validation par lot ni la contre-passation.
    assert_eq!(avec.decider(Action::ValiderParLot), Decision::Refuse);
    assert_eq!(avec.decider_lot(&["brouillon-A1"]), Decision::Refuse);
    assert_eq!(avec.decider(Action::ContrePasser), Decision::Refuse);
}

#[test]
fn la_delegation_d_un_autre_dossier_n_ouvre_pas_la_validation() {
    let entites = monde(true, true, "autre", &[])
        .avec(acces("acces-moi-A1", "moi@A", "A1", false))
        .avec(acces("acces-moi-A2", "moi@A", "A2", true));
    for id in ["acces-moi-A1", "acces-moi-A2"] {
        assert_eq!(
            AUTORISATION.decider(
                &appartenance("moi@A", "moi", "A", Role::Collaborateur),
                Action::ValiderEcriture,
                ressource(Action::ValiderEcriture, "A"),
                &entites,
                &Contexte::new(fraiche()).avec_acces(acc(id))
            ),
            Decision::Refuse,
            "{id}"
        );
    }
}

#[test]
fn la_delegation_ne_donne_rien_a_l_auditeur_ni_au_client() {
    for role in [Role::Auditeur, Role::Client] {
        let cas = Cas {
            delegation: true,
            ..Cas::nominal(role)
        };
        assert_eq!(cas.decider(Action::ValiderEcriture), Decision::Refuse);
    }
}

// ---------------------------------------------------------------------------
// Séparation des tâches (Q5.3, F1.16)
// ---------------------------------------------------------------------------

#[test]
fn separation_active_ni_le_createur_ni_un_correcteur_ne_valident() {
    for role in [Role::Administrateur, Role::Valideur, Role::Collaborateur] {
        for part in [Part::Createur, Part::Correcteur] {
            let cas = Cas {
                delegation: true,
                part,
                ..Cas::nominal(role)
            };
            assert_eq!(
                cas.decider(Action::ValiderEcriture),
                Decision::Refuse,
                "{role:?}"
            );
            // Corriger le brouillon reste permis.
            assert_eq!(
                cas.decider(Action::CorrigerBrouillard),
                Decision::Autorise,
                "{role:?}"
            );
        }
        // La saisie d'autrui, à laquelle `moi` n'a pas contribué, reste validable.
        let autrui = Cas {
            delegation: true,
            ..Cas::nominal(role)
        };
        assert_eq!(
            autrui.decider(Action::ValiderEcriture),
            Decision::Autorise,
            "{role:?}"
        );
    }
}

#[test]
fn corriger_le_brouillon_d_autrui_puis_le_valider_est_refuse() {
    // Avant correction, X peut valider le brouillon de Y.
    let avant = Cas::nominal(Role::Valideur);
    assert_eq!(
        avant.decider(Action::CorrigerBrouillard),
        Decision::Autorise
    );
    assert_eq!(avant.decider(Action::ValiderEcriture), Decision::Autorise);
    // Après sa correction, X en est contributeur : validation refusée.
    let apres = Cas {
        part: Part::Correcteur,
        ..avant
    };
    assert_eq!(apres.decider(Action::ValiderEcriture), Decision::Refuse);
}

#[test]
fn separation_desactivee_un_contributeur_peut_valider() {
    for role in [Role::Administrateur, Role::Valideur, Role::Collaborateur] {
        for part in [Part::Createur, Part::Correcteur] {
            let cas = Cas {
                delegation: true,
                part,
                separation: false,
                ..Cas::nominal(role)
            };
            assert_eq!(
                cas.decider(Action::ValiderEcriture),
                Decision::Autorise,
                "{role:?}"
            );
        }
    }
}

#[test]
fn la_separation_appliquee_est_celle_du_cabinet_du_dossier() {
    // Même dossier A1, même brouillon créé par `moi` ; seul change le
    // paramètre de chaque cabinet. Seul celui de A (cabinet du dossier et de
    // l'appartenance) compte.
    let principal = appartenance("moi@A", "moi", "A", Role::Valideur);
    let r = ressource(Action::ValiderEcriture, "A");
    let contexte = Contexte::new(fraiche()).avec_acces(acc("acces-moi-A1"));
    let decide = |separation_a, separation_b| {
        let entites = monde(separation_a, separation_b, "moi", &[]).avec(acces(
            "acces-moi-A1",
            "moi@A",
            "A1",
            false,
        ));
        AUTORISATION.decider(&principal, Action::ValiderEcriture, r, &entites, &contexte)
    };
    assert_eq!(decide(true, false), Decision::Refuse);
    assert_eq!(decide(false, true), Decision::Autorise);
}

#[test]
fn sans_l_entite_cabinet_la_validation_est_refusee() {
    // La séparation ne peut être évaluée : refus, même pour la saisie d'autrui.
    let entites = Entites::new()
        .avec(dossier("A1", "A"))
        .avec(brouillon("brouillon-A1", "A1", "autre", &[]))
        .avec(acces("acces-moi-A1", "moi@A", "A1", false));
    assert_eq!(
        AUTORISATION.decider(
            &appartenance("moi@A", "moi", "A", Role::Valideur),
            Action::ValiderEcriture,
            ressource(Action::ValiderEcriture, "A"),
            &entites,
            &Contexte::new(fraiche()).avec_acces(acc("acces-moi-A1"))
        ),
        Decision::Refuse
    );
}

// ---------------------------------------------------------------------------
// Validation par lot
// ---------------------------------------------------------------------------

#[test]
fn un_lot_contenant_une_ecriture_du_valideur_est_refuse_en_entier() {
    let admin = Cas {
        part: Part::Createur,
        ..Cas::nominal(Role::Administrateur)
    };
    // Décider seulement `ValiderParLot` laisserait passer…
    assert_eq!(admin.decider(Action::ValiderParLot), Decision::Autorise);
    // … mais le lot est refusé : chaque brouillon est créé par `moi`.
    assert_eq!(admin.decider_lot(&["brouillon-A1"]), Decision::Refuse);
    // Séparation désactivée : accepté.
    let sans_separation = Cas {
        separation: false,
        ..admin
    };
    assert_eq!(
        sans_separation.decider_lot(&["brouillon-A1"]),
        Decision::Autorise
    );
}

#[test]
fn un_seul_brouillon_refuse_refuse_tout_le_lot() {
    let valideur = Cas::nominal(Role::Valideur);
    assert_eq!(valideur.decider_lot(&["brouillon-A1"]), Decision::Autorise);
    // Un brouillon d'un dossier non affecté (A2) dans le lot.
    assert_eq!(
        valideur.decider_lot(&["brouillon-A1", "brouillon-A2"]),
        Decision::Refuse
    );
    // Un brouillon inconnu.
    assert_eq!(
        valideur.decider_lot(&["brouillon-A1", "fantome"]),
        Decision::Refuse
    );
    // Lot vide.
    assert_eq!(valideur.decider_lot(&[]), Decision::Refuse);
    // Session non fraîche.
    let ordinaire = Cas {
        session: ordinaire(),
        ..valideur
    };
    assert_eq!(ordinaire.decider_lot(&["brouillon-A1"]), Decision::Refuse);
}

// ---------------------------------------------------------------------------
// Ré-authentification (ADR 0012)
// ---------------------------------------------------------------------------

#[test]
fn une_session_est_fraiche_moins_de_cinq_minutes() {
    let secondes = time::Duration::seconds;
    assert!(Session::depuis(T0, T0).est_fraiche());
    assert!(Session::depuis(T0, T0 + DUREE_FRAICHEUR - secondes(1)).est_fraiche());
    assert!(!Session::depuis(T0, T0 + DUREE_FRAICHEUR).est_fraiche());
    assert!(!Session::depuis(T0, T0 + time::Duration::hours(1)).est_fraiche());
    // Vérification datée du futur : refus en cas de doute.
    assert!(!Session::depuis(T0 + secondes(1), T0).est_fraiche());
    assert_eq!(DUREE_FRAICHEUR, time::Duration::minutes(5));
}

#[test]
fn les_actions_sensibles_exigent_une_session_fraiche() {
    for action in SENSIBLES {
        for role in Role::TOUS {
            let cas = Cas {
                session: ordinaire(),
                ..Cas::nominal(role)
            };
            assert_eq!(
                cas.decider(*action),
                Decision::Refuse,
                "{role:?} {action:?}"
            );
        }
        assert_eq!(
            Cas::nominal(Role::Administrateur).decider(*action),
            Decision::Autorise,
            "{action:?}"
        );
    }
}

#[test]
fn les_autres_actions_n_exigent_pas_de_session_fraiche() {
    for action in Action::TOUTES.iter().filter(|a| !SENSIBLES.contains(a)) {
        for role in Role::TOUS {
            let cas = Cas {
                session: ordinaire(),
                ..Cas::nominal(role)
            };
            assert_eq!(
                cas.decider(*action),
                decision(attendu(*action, role)),
                "{role:?} {action:?}"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Client (portail, L4) et élévation (F1.10)
// ---------------------------------------------------------------------------

#[test]
fn le_client_n_obtient_aucune_action() {
    for action in Action::TOUTES {
        for (affecte, delegation, separation) in [
            (true, true, false),
            (true, false, true),
            (false, false, false),
        ] {
            let cas = Cas {
                affecte,
                delegation,
                separation,
                ..Cas::nominal(Role::Client)
            };
            assert_eq!(cas.decider(*action), Decision::Refuse, "{action:?}");
        }
    }
}

#[test]
fn seul_l_administrateur_modifie_les_droits() {
    for role in [
        Role::Valideur,
        Role::Collaborateur,
        Role::Auditeur,
        Role::Client,
    ] {
        let cas = Cas {
            delegation: true,
            separation: false,
            ..Cas::nominal(role)
        };
        for action in [
            Action::ModifierDroits,
            Action::GererCabinet,
            Action::DesactiverSeparationDesTaches,
        ] {
            assert_eq!(cas.decider(action), Decision::Refuse, "{role:?} {action:?}");
        }
    }
}

// ---------------------------------------------------------------------------
// Écriture validée définitive (F1.15)
// ---------------------------------------------------------------------------

#[test]
fn aucune_action_de_modification_ne_s_applique_a_une_ecriture_validee() {
    let sur_validee: Vec<Action> = Action::TOUTES
        .iter()
        .copied()
        .filter(|a| a.type_ressource() == TypeRessource::EcritureValidee)
        .collect();
    // La contre-passation crée une écriture miroir ; elle ne modifie pas
    // l'écriture validée (spec §6.1).
    assert_eq!(sur_validee, vec![Action::ContrePasser]);

    // Une écriture validée présentée à une autre action est refusée, quel
    // que soit le rôle (le schéma rejette la requête).
    for role in Role::TOUS {
        for action in Action::TOUTES {
            if *action == Action::ContrePasser {
                continue;
            }
            let entites =
                monde(false, false, "moi", &[]).avec(acces("acces-moi-A1", "moi@A", "A1", true));
            assert_eq!(
                AUTORISATION.decider(
                    &appartenance("moi@A", "moi", "A", role),
                    *action,
                    Ressource::EcritureValidee(ecr("validee-A1")),
                    &entites,
                    &Contexte::new(fraiche()).avec_acces(acc("acces-moi-A1"))
                ),
                Decision::Refuse,
                "{role:?} {action:?}"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Anomalies : refus
// ---------------------------------------------------------------------------

fn admin_sur(action: Action, ressource: Ressource, entites: &Entites) -> Decision {
    AUTORISATION.decider(
        &appartenance("moi@A", "moi", "A", Role::Administrateur),
        action,
        ressource,
        entites,
        &Contexte::new(fraiche()),
    )
}

#[test]
fn une_ressource_du_mauvais_type_est_refusee() {
    let r = Ressource::Dossier(dos("A1"));
    assert_eq!(
        admin_sur(Action::GererCabinet, r, &monde(true, true, "autre", &[])),
        Decision::Refuse
    );
}

#[test]
fn une_entite_en_double_est_refusee() {
    let r = ressource(Action::ConsulterEditions, "A");
    // Doublon identique, puis doublon contradictoire (dossier placé dans B).
    for cabinet_double in ["A", "B"] {
        let entites = monde(true, true, "autre", &[]).avec(dossier("A1", cabinet_double));
        assert_eq!(
            admin_sur(Action::ConsulterEditions, r, &entites),
            Decision::Refuse,
            "{cabinet_double}"
        );
    }
}

#[test]
fn le_principal_ne_peut_pas_etre_redefini_dans_les_entites() {
    // Le principal est ajouté par `decider` ; une seconde version (rôle
    // élevé) fournie dans les entités est une anomalie.
    let principal = appartenance("moi@A", "moi", "A", Role::Auditeur);
    let entites = monde(true, true, "autre", &[]).avec(appartenance(
        "moi@A",
        "moi",
        "A",
        Role::Administrateur,
    ));
    assert_eq!(
        AUTORISATION.decider(
            &principal,
            Action::GererCabinet,
            Ressource::Cabinet(cab("A")),
            &entites,
            &Contexte::new(fraiche())
        ),
        Decision::Refuse
    );
}

#[test]
fn une_ressource_sans_son_dossier_est_refusee() {
    // Sans l'entité Dossier, l'écriture ne descend d'aucun cabinet.
    let entites = Entites::new()
        .avec(cabinet("A", false))
        .avec(validee("validee-A1", "A1"));
    let r = ressource(Action::ContrePasser, "A");
    assert_eq!(
        admin_sur(Action::ContrePasser, r, &entites),
        Decision::Refuse
    );
}

#[test]
fn le_verdict_cite_les_politiques_determinantes() {
    let entites = monde(true, true, "moi", &[]);
    let principal = appartenance("moi@A", "moi", "A", Role::Administrateur);
    let contexte = Contexte::new(fraiche());
    let refus = AUTORISATION.verdict(
        &principal,
        Action::ValiderEcriture,
        ressource(Action::ValiderEcriture, "A"),
        &entites,
        &contexte,
    );
    assert_eq!(refus.decision, Decision::Refuse);
    assert_eq!(refus.politiques, vec!["invariant-separation-des-taches"]);

    let accord = AUTORISATION.verdict(
        &principal,
        Action::DeverrouillerPeriode,
        ressource(Action::DeverrouillerPeriode, "A"),
        &entites,
        &contexte,
    );
    assert_eq!(accord.decision, Decision::Autorise);
    assert_eq!(accord.politiques, vec!["s20-deverrouiller-periode"]);
}

// ---------------------------------------------------------------------------
// Propriétés
// ---------------------------------------------------------------------------

fn role() -> impl Strategy<Value = Role> {
    prop::sample::select(Role::TOUS.to_vec())
}

fn action() -> impl Strategy<Value = Action> {
    prop::sample::select(Action::TOUTES.to_vec())
}

proptest! {
    /// Aucune combinaison d'entités ne donne à une appartenance du cabinet A
    /// un accès à une ressource du cabinet B : rôle, affectations (vraies ou
    /// forgées, toutes présentes), délégations, appartenances du même
    /// utilisateur et d'autrui à B (présentes), séparation des tâches,
    /// session et contributeurs quelconques. Le refus vient de la politique
    /// de cloisonnement, jamais d'une anomalie.
    #[test]
    fn aucun_acces_a_une_ressource_d_un_autre_cabinet(
        role_a in role(),
        role_b in role(),
        role_lui in role(),
        action in action(),
        acces_choisi in 0usize..5,
        delegation in any::<bool>(),
        fraiche_ou_non in any::<bool>(),
        separation_a in any::<bool>(),
        separation_b in any::<bool>(),
        createur_moi in any::<bool>(),
        dossier_b in 1usize..=2,
    ) {
        let createur = if createur_moi { "moi" } else { "autre" };
        let mut entites = monde(separation_a, separation_b, createur, &[])
            .avec(appartenance("moi@B", "moi", "B", role_b))
            .avec(appartenance("lui@B", "lui", "B", role_lui));
        let cible = format!("B{dossier_b}");
        let candidats = [
            None,
            Some(acces("x", "moi@A", &cible, delegation)),
            Some(acces("x", "moi@A", "A1", delegation)),
            Some(acces("x", "moi@B", &cible, delegation)),
            Some(acces("x", "lui@B", &cible, delegation)),
        ];
        let session = if fraiche_ou_non { fraiche() } else { ordinaire() };
        let mut contexte = Contexte::new(session);
        if let Some(a) = candidats[acces_choisi].clone() {
            entites.ajouter(a);
            contexte = contexte.avec_acces(acc("x"));
        }
        let verdict = AUTORISATION.verdict(
            &appartenance("moi@A", "moi", "A", role_a),
            action,
            ressource_dans(action, "B", &cible),
            &entites,
            &contexte,
        );
        prop_assert_eq!(verdict.decision, Decision::Refuse);
        prop_assert!(
            verdict.politiques.contains(&"invariant-cloisonnement-cabinet".to_string()),
            "{:?}", verdict
        );
    }

    /// Hors administrateur, aucune action sur un dossier sans l'affectation
    /// de l'appartenance agissante à ce dossier.
    #[test]
    fn aucun_acces_a_un_dossier_non_affecte(
        role_a in role().prop_filter("hors administrateur", |r| *r != Role::Administrateur),
        action in action().prop_filter("action de dossier", |a| a.type_ressource() != TypeRessource::Cabinet),
        acces_choisi in 0usize..3,
        delegation in any::<bool>(),
    ) {
        let mut entites = monde(false, false, "autre", &[])
            .avec(appartenance("lui@A", "lui", "A", Role::Administrateur));
        let candidats = [
            None,
            Some(acces("x", "moi@A", "A2", delegation)),
            Some(acces("x", "lui@A", "A1", delegation)),
        ];
        let mut contexte = Contexte::new(fraiche());
        if let Some(a) = candidats[acces_choisi].clone() {
            entites.ajouter(a);
            contexte = contexte.avec_acces(acc("x"));
        }
        prop_assert_eq!(
            AUTORISATION.decider(
                &appartenance("moi@A", "moi", "A", role_a),
                action,
                ressource(action, "A"),
                &entites,
                &contexte
            ),
            Decision::Refuse
        );
    }
}
