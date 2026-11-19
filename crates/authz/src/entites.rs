//! Constructeurs typés des entités, de la ressource et du contexte.
//!
//! L'appelant (l'API, à partir de la base) ne manipule jamais le JSON ni les
//! types de Cedar : il décrit les entités utiles à une décision avec ces
//! types, et [`crate::Autorisation`] les traduit et les valide contre le
//! schéma. Toute incohérence (doublon, entité manquante, ressource du
//! mauvais type) aboutit à un refus.

use std::fmt;
use std::str::FromStr;

use time::{Duration, OffsetDateTime};
use uuid::Uuid;

/// Identifiant refusé.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ErreurIdentifiant {
    /// Le texte n'est pas un UUID (chaîne vide comprise).
    #[error("identifiant : UUID attendu")]
    Invalide,
    /// UUID nul ou maximal : jamais attribué (ADR 0011, UUID v4), il
    /// trahirait une valeur par défaut.
    #[error("identifiant : UUID nul ou maximal refusé")]
    Reserve,
}

macro_rules! identifiant {
    ($(#[$doc:meta])* $nom:ident) => {
        $(#[$doc])*
        ///
        /// Toujours un UUID ni nul ni maximal ; sa forme dans Cedar est
        /// unique (forme à tirets, en minuscules), quelle que soit la casse
        /// ou la forme reçue.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub struct $nom(Uuid);

        impl $nom {
            /// Identifiant à partir d'un UUID.
            ///
            /// # Errors
            /// [`ErreurIdentifiant::Reserve`] pour l'UUID nul ou maximal.
            pub fn new(uuid: Uuid) -> Result<Self, ErreurIdentifiant> {
                if uuid.is_nil() || uuid.is_max() {
                    Err(ErreurIdentifiant::Reserve)
                } else {
                    Ok(Self(uuid))
                }
            }

            /// L'UUID.
            #[must_use]
            pub const fn uuid(self) -> Uuid {
                self.0
            }
        }

        impl FromStr for $nom {
            type Err = ErreurIdentifiant;

            /// Analyse un UUID, quelle que soit sa casse.
            fn from_str(texte: &str) -> Result<Self, Self::Err> {
                Uuid::try_parse(texte)
                    .map_err(|_| ErreurIdentifiant::Invalide)
                    .and_then(Self::new)
            }
        }

        impl fmt::Display for $nom {
            /// Forme canonique : à tirets, en minuscules.
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                fmt::Display::fmt(&self.0.hyphenated(), f)
            }
        }
    };
}

identifiant!(
    /// Identifiant d'un utilisateur (une personne).
    IdUtilisateur
);
identifiant!(
    /// Identifiant d'un cabinet (tenant).
    IdCabinet
);
identifiant!(
    /// Identifiant d'une appartenance d'un utilisateur à un cabinet.
    IdAppartenance
);
identifiant!(
    /// Identifiant d'un dossier.
    IdDossier
);
identifiant!(
    /// Identifiant d'une affectation à un dossier.
    IdAccesDossier
);
identifiant!(
    /// Identifiant d'une écriture (brouillon ou validée).
    IdEcriture
);
identifiant!(
    /// Identifiant public d'une session (ADR 0012).
    IdSession
);

/// Rôle par défaut du §20. Pas de rôle dérivé en L2 (Q5.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Role {
    /// Administrateur du cabinet.
    Administrateur,
    /// Comptable valideur, expert-comptable.
    Valideur,
    /// Collaborateur comptable.
    Collaborateur,
    /// Auditeur, commissaire aux comptes : lecture seule.
    Auditeur,
    /// Dirigeant client (portail, L4) : aucun droit en L2.
    Client,
}

impl Role {
    /// Tous les rôles.
    pub const TOUS: [Role; 5] = [
        Role::Administrateur,
        Role::Valideur,
        Role::Collaborateur,
        Role::Auditeur,
        Role::Client,
    ];

    /// Identifiant du rôle dans le schéma (`Sigma::Role::"…"`).
    #[must_use]
    pub const fn nom(self) -> &'static str {
        match self {
            Role::Administrateur => "administrateur",
            Role::Valideur => "valideur",
            Role::Collaborateur => "collaborateur",
            Role::Auditeur => "auditeur",
            Role::Client => "client",
        }
    }
}

/// Un cabinet et ses paramètres d'autorisation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cabinet {
    /// Identifiant.
    pub id: IdCabinet,
    /// Séparation des tâches active (Q5.3 ; vrai par défaut à la création).
    pub separation_des_taches: bool,
}

/// Appartenance d'un utilisateur à un cabinet, avec son rôle : c'est le
/// principal de toute décision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Appartenance {
    /// Identifiant.
    pub id: IdAppartenance,
    /// L'utilisateur.
    pub utilisateur: IdUtilisateur,
    /// Le cabinet.
    pub cabinet: IdCabinet,
    /// Son rôle dans ce cabinet.
    pub role: Role,
    /// Faux si l'appartenance est suspendue (ADR 0014) : plus aucune action.
    pub actif: bool,
}

/// Un dossier, rattaché à un seul cabinet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dossier {
    /// Identifiant.
    pub id: IdDossier,
    /// Son cabinet.
    pub cabinet: IdCabinet,
}

/// Affectation d'une appartenance à un dossier (Q5.1), avec la délégation
/// du droit de valider (Q5.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccesDossier {
    /// Identifiant.
    pub id: IdAccesDossier,
    /// L'appartenance affectée.
    pub appartenance: IdAppartenance,
    /// Le dossier.
    pub dossier: IdDossier,
    /// Délégation du droit de valider (collaborateur, Q5.2).
    pub delegation_validation: bool,
}

/// Une écriture au brouillard.
///
/// Séparation des tâches (Q5.3) : la spec §20 interdit de valider une
/// écriture « qu'il a lui-même saisie ». Corriger un brouillon, c'est en
/// saisir une partie : le créateur **et** chaque utilisateur qui l'a
/// modifié en sont contributeurs, et aucun d'eux ne peut le valider tant
/// que la séparation est active. Le créateur est obligatoire : l'ensemble
/// des contributeurs n'est jamais vide.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EcritureBrouillon {
    /// Identifiant.
    pub id: IdEcriture,
    /// Son dossier.
    pub dossier: IdDossier,
    /// L'utilisateur qui l'a créé.
    pub createur: IdUtilisateur,
    /// Les utilisateurs qui l'ont modifié depuis (journal du brouillard).
    pub correcteurs: Vec<IdUtilisateur>,
}

impl EcritureBrouillon {
    /// Créateur et correcteurs.
    pub fn contributeurs(&self) -> impl Iterator<Item = IdUtilisateur> + '_ {
        std::iter::once(self.createur).chain(self.correcteurs.iter().copied())
    }
}

/// Une écriture validée. Elle ne porte aucun état modifiable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EcritureValidee {
    /// Identifiant.
    pub id: IdEcriture,
    /// Son dossier.
    pub dossier: IdDossier,
}

/// Une entité fournie à la décision, en plus du principal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Entite {
    /// Un cabinet.
    Cabinet(Cabinet),
    /// Une autre appartenance (celle qui détient une affectation, par
    /// exemple). Le principal est ajouté par [`crate::Autorisation`].
    Appartenance(Appartenance),
    /// Un dossier.
    Dossier(Dossier),
    /// Une affectation à un dossier.
    AccesDossier(AccesDossier),
    /// Une écriture au brouillard.
    EcritureBrouillon(EcritureBrouillon),
    /// Une écriture validée.
    EcritureValidee(EcritureValidee),
}

macro_rules! vers_entite {
    ($($type:ident),+) => {
        $(impl From<$type> for Entite {
            fn from(e: $type) -> Self {
                Entite::$type(e)
            }
        })+
    };
}
vers_entite!(
    Cabinet,
    Appartenance,
    Dossier,
    AccesDossier,
    EcritureBrouillon,
    EcritureValidee
);

/// Les entités utiles à une décision, hors principal : le cabinet, la
/// ressource et ses parents (dossier), l'affectation éventuelle.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Entites {
    liste: Vec<Entite>,
}

impl Entites {
    /// Ensemble vide.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Ajoute une entité (forme chaînée).
    #[must_use]
    pub fn avec(mut self, entite: impl Into<Entite>) -> Self {
        self.ajouter(entite);
        self
    }

    /// Ajoute une entité.
    pub fn ajouter(&mut self, entite: impl Into<Entite>) {
        self.liste.push(entite.into());
    }

    /// Les entités ajoutées.
    #[must_use]
    pub fn liste(&self) -> &[Entite] {
        &self.liste
    }
}

/// La ressource visée par une action.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Ressource {
    /// Le cabinet.
    Cabinet(IdCabinet),
    /// Un dossier.
    Dossier(IdDossier),
    /// Une écriture au brouillard.
    EcritureBrouillon(IdEcriture),
    /// Une écriture validée.
    EcritureValidee(IdEcriture),
}

/// La ressource d'une action de l'utilisateur sur lui-même, telle que l'API
/// la lit en base : [`crate::Autorisation::decider_utilisateur`] en construit
/// l'entité.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RessourceUtilisateur {
    /// Une session, avec son titulaire lu en base (sous RLS).
    Session {
        /// Identifiant public.
        id: IdSession,
        /// Titulaire.
        titulaire: IdUtilisateur,
    },
    /// Un utilisateur (la liste de ses sessions).
    Utilisateur(IdUtilisateur),
}

/// Durée pendant laquelle une vérification de facteur rend la session
/// « fraîche » (ADR 0012 : moins de 5 minutes).
pub const DUREE_FRAICHEUR: Duration = Duration::minutes(5);

/// État de la session de l'appelant au regard de la ré-authentification.
///
/// Seul [`Session::depuis`] le construit : la règle de l'ADR 0012 n'est
/// écrite qu'ici. Aucune lecture d'horloge : les instants sont fournis.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Session {
    fraiche: bool,
}

impl Session {
    /// Session fraîche si la dernière vérification de facteur date de moins
    /// de [`DUREE_FRAICHEUR`] à `maintenant`. Une vérification datée du
    /// futur ne compte pas (refus en cas de doute).
    #[must_use]
    pub fn depuis(derniere_verification: OffsetDateTime, maintenant: OffsetDateTime) -> Self {
        let ecart = maintenant - derniere_verification;
        Self {
            fraiche: !ecart.is_negative() && ecart < DUREE_FRAICHEUR,
        }
    }

    /// Vrai si la session est fraîche.
    #[must_use]
    pub const fn est_fraiche(self) -> bool {
        self.fraiche
    }
}

/// Contexte de la requête.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Contexte {
    session: Session,
    acces: Option<IdAccesDossier>,
}

impl Contexte {
    /// Contexte sans affectation à un dossier.
    #[must_use]
    pub fn new(session: Session) -> Self {
        Self {
            session,
            acces: None,
        }
    }

    /// Précise l'affectation de l'appartenance agissante au dossier de la
    /// ressource ; l'entité [`AccesDossier`] doit figurer dans les entités.
    #[must_use]
    pub fn avec_acces(mut self, acces: IdAccesDossier) -> Self {
        self.acces = Some(acces);
        self
    }

    /// État de la session.
    #[must_use]
    pub fn session(&self) -> Session {
        self.session
    }

    /// Affectation fournie.
    #[must_use]
    pub fn acces(&self) -> Option<IdAccesDossier> {
        self.acces
    }
}
