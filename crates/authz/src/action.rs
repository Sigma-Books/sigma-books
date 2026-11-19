//! Actions du schéma Cedar (`politiques/schema.cedarschema`).
//!
//! La correspondance entre cette énumération et le schéma est vérifiée au
//! chargement : une action absente du schéma, ou appliquée à un autre type
//! de ressource, empêche le démarrage.

/// Type de ressource sur lequel porte une action.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TypeRessource {
    /// Le cabinet (tenant).
    Cabinet,
    /// Un dossier du cabinet.
    Dossier,
    /// Une écriture au brouillard.
    EcritureBrouillon,
    /// Une écriture validée, définitive.
    EcritureValidee,
}

impl TypeRessource {
    /// Nom du type d'entité dans le schéma, sans l'espace de noms.
    #[must_use]
    pub const fn nom(self) -> &'static str {
        match self {
            Self::Cabinet => "Cabinet",
            Self::Dossier => "Dossier",
            Self::EcritureBrouillon => "EcritureBrouillon",
            Self::EcritureValidee => "EcritureValidee",
        }
    }
}

macro_rules! actions {
    ($( $(#[$doc:meta])* $variante:ident => $ressource:ident ),+ $(,)?) => {
        /// Action soumise à autorisation (une par ligne utile du tableau de la
        /// spec §20, voir le schéma pour le détail).
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum Action {
            $( $(#[$doc])* $variante, )+
        }

        impl Action {
            /// Toutes les actions déclarées dans le schéma.
            pub const TOUTES: &'static [Action] = &[ $( Action::$variante, )+ ];

            /// Identifiant de l'action dans le schéma (`Sigma::Action::"…"`),
            /// repris par le journal d'audit (ADR 0014, clé `action=`).
            #[must_use]
            pub const fn nom(self) -> &'static str {
                match self { $( Action::$variante => stringify!($variante), )+ }
            }

            /// Type de ressource auquel l'action s'applique.
            #[must_use]
            pub const fn type_ressource(self) -> TypeRessource {
                match self { $( Action::$variante => TypeRessource::$ressource, )+ }
            }
        }
    };
}

actions! {
    /// Voir le cabinet : le choisir comme cabinet actif parmi les siens
    /// (`GET /api/moi/cabinets`, ADR 0012), tout rôle du cabinet.
    AccederCabinet => Cabinet,
    /// Paramètres du cabinet, utilisateurs, sessions (§20, administrateur).
    GererCabinet => Cabinet,
    /// Changements de droits : invitation, rôle, suspension, affectation,
    /// délégation (§20 ; ré-authentification).
    ModifierDroits => Cabinet,
    /// Activer la séparation des tâches (Q5.3).
    ActiverSeparationDesTaches => Cabinet,
    /// Désactiver la séparation des tâches (Q5.3 ; ré-authentification).
    DesactiverSeparationDesTaches => Cabinet,
    /// Consulter l'audit de portée cabinet (ADR 0014 ; administrateur, ADR 0016).
    ConsulterAuditCabinet => Cabinet,
    /// Vérifier la chaîne d'audit du cabinet (administrateur, ADR 0016).
    VerifierAuditCabinet => Cabinet,
    /// Exporter l'audit de portée cabinet (administrateur, ADR 0016 ;
    /// ré-authentification).
    ExporterAuditCabinet => Cabinet,
    /// Créer un dossier (§20).
    CreerDossier => Cabinet,
    /// Facturation (§20) : lot ultérieur, aucune politique.
    GererFacturation => Cabinet,
    /// Voir un dossier : distingue 403 (visible) de 404 (d'autrui, ADR 0011).
    AccederDossier => Dossier,
    /// Paramétrer plan, journaux, fiscalité (§20).
    ParametrerDossier => Dossier,
    /// Déposer des pièces (§20).
    DeposerPiece => Dossier,
    /// Créer un brouillon (§20 « Saisir dans le brouillard »).
    SaisirBrouillard => Dossier,
    /// Validation par lot (§20 ; ré-authentification). Chaque écriture du lot
    /// doit aussi être autorisée par [`Action::ValiderEcriture`].
    ValiderParLot => Dossier,
    /// Verrouiller une période (§20).
    VerrouillerPeriode => Dossier,
    /// Déverrouiller une période (§20 ; ré-authentification).
    DeverrouillerPeriode => Dossier,
    /// Clôture informatique d'une période (ré-authentification).
    CloturerPeriode => Dossier,
    /// Clôturer un exercice (§20 ; ré-authentification).
    CloturerExercice => Dossier,
    /// Consulter éditions et états (§20).
    ConsulterEditions => Dossier,
    /// Vérifier l'intégrité (§20).
    VerifierIntegrite => Dossier,
    /// Consulter l'audit du dossier (§20).
    ConsulterAudit => Dossier,
    /// Exporter une édition (§20).
    Exporter => Dossier,
    /// Export complet : FEC, livres, audit du dossier (§20 ; ré-authentification).
    ExporterComplet => Dossier,
    /// Export d'intégrité (§20 ; ré-authentification).
    ExporterIntegrite => Dossier,
    /// Reprise de données (§20) : L8, aucune politique.
    ReprendreDonnees => Dossier,
    /// Lettrer, délettrer (§20) : L3, aucune politique.
    Lettrer => Dossier,
    /// Activer une règle apprise (§20) : L4, aucune politique.
    ActiverRegleApprise => Dossier,
    /// Liquider la TVA (§20) : L5, aucune politique.
    LiquiderTva => Dossier,
    /// Travaux de fin d'exercice (§20) : L6, aucune politique.
    TravauxFinExercice => Dossier,
    /// Répondre aux demandes (§20, portail client) : L4, aucune politique.
    RepondreDemande => Dossier,
    /// Modifier ou supprimer un brouillon (§20 « corriger dans le brouillard »).
    CorrigerBrouillard => EcritureBrouillon,
    /// Valider une écriture (§20 ; délégation Q5.2 ; séparation des tâches Q5.3).
    ValiderEcriture => EcritureBrouillon,
    /// Contre-passer une écriture validée (§20).
    ContrePasser => EcritureValidee,
}
