//! Domaine « facteurs » (ADR 0012, ADR 0017 ; PR 8b, 8c) : connexion, méthode de secours, codes de
//! récupération, ré-authentification, événements d'authentification
//! (`politiques/schema/facteurs.cedarschema`, `politiques/facteurs.cedar`).
//!
//! # Deux principaux
//! - **Anonyme** ([`Autorisation::verdict_connexion`]) : le demandeur d'une connexion, sans
//!   session. Il ne peut que demander à se connecter à l'instance (`SeConnecter`) : action
//!   publique et bornée, décidée **avant** toute lecture en base, comme toute route. Elle ne
//!   donne accès à rien : la décision qui ouvre une session vient après la vérification d'un
//!   facteur.
//! - **Utilisateur** ([`Autorisation::verdict_facteur`]) : l'utilisateur dont un facteur vient
//!   d'être vérifié (`OuvrirSession`), ou celui d'une session authentifiée (ses facteurs, ses
//!   événements, sa ré-authentification). La ressource est l'utilisateur lui-même ; le contexte
//!   ([`ContexteFacteur`]) porte la nature et la méthode de la session, et sa fraîcheur.
//!
//! # Contrôles au chargement
//! Les actions de [`ActionFacteur`] et `SeConnecter` doivent exister au schéma avec ces
//! principaux et ces ressources ; le principal `Anonyme` n'est admis par aucune autre action,
//! sur aucun autre type ; la ressource `Instance` n'admet aucun autre principal. Sinon le
//! serveur refuse de démarrer.

use std::collections::{HashMap, HashSet};
use std::str::FromStr;

use cedar_policy::{
    Context, Entities, Entity, EntityTypeName, EntityUid, Request, RestrictedExpression, Schema,
};

use crate::{Autorisation, ESPACE, ErreurChargement, IdUtilisateur, Session, Types, Verdict, uid};

/// Nom de l'action de connexion au schéma (principal `Anonyme`, ressource `Instance`).
pub const ACTION_CONNEXION: &str = "SeConnecter";

macro_rules! actions_facteur {
    ($( $(#[$doc:meta])* $variante:ident ),+ $(,)?) => {
        /// Action de l'utilisateur sur ses propres facteurs (ADR 0012), de principal
        /// `Utilisateur`, de ressource l'utilisateur lui-même, avec un [`ContexteFacteur`].
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum ActionFacteur {
            $( $(#[$doc])* $variante, )+
        }

        impl ActionFacteur {
            /// Toutes les actions du domaine déclarées au schéma (hors `SeConnecter`).
            pub const TOUTES: &'static [ActionFacteur] = &[ $( ActionFacteur::$variante, )+ ];

            /// Identifiant de l'action dans le schéma (`Sigma::Action::"…"`), repris par le
            /// journal d'audit (ADR 0014, clé `action=`).
            #[must_use]
            pub const fn nom(self) -> &'static str {
                match self { $( ActionFacteur::$variante => stringify!($variante), )+ }
            }
        }
    };
}

actions_facteur! {
    /// Ouverture d'une session après la vérification d'un facteur.
    OuvrirSession,
    /// Ré-authentification par la méthode de secours ou par une passkey.
    SeReauthentifier,
    /// État de ses facteurs.
    ConsulterSesFacteurs,
    /// Ses événements d'authentification (ADR 0017).
    ConsulterSesEvenements,
    /// Enrôlement (ou remplacement) de la méthode de secours.
    EnrolerSecours,
    /// Nouvelle série de codes de récupération.
    RegenererCodesRecuperation,
    /// Enrôlement d'une passkey (WebAuthn, PR 8c), cérémonie comprise.
    EnrolerPasskey,
    /// Suppression d'une de ses passkeys.
    SupprimerPasskey,
    /// Acceptation, par un utilisateur connecté, d'une invitation à un autre cabinet (ADR 0012).
    AccepterInvitation,
}

/// Nature d'une session, telle que la voit Cedar (ADR 0012).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NatureDeSession {
    /// Ouverte par un facteur fort.
    Complete,
    /// Ouverte par un code de récupération : enrôlement seulement.
    Restreinte,
    /// Ouverte par un lien d'invitation ou de réinitialisation.
    Enrolement,
}

impl NatureDeSession {
    /// Valeur de `context.nature`.
    #[must_use]
    pub const fn nom(self) -> &'static str {
        match self {
            Self::Complete => "complete",
            Self::Restreinte => "restreinte",
            Self::Enrolement => "enrolement",
        }
    }
}

/// Facteur vérifié à l'ouverture d'une session, tel que le voit Cedar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MethodeDeSession {
    /// Passkey (WebAuthn, PR 8c).
    Passkey,
    /// Mot de passe et TOTP.
    Secours,
    /// Code de récupération.
    CodeRecuperation,
    /// Lien d'invitation ou de réinitialisation (PR 9).
    LienEnrolement,
}

impl MethodeDeSession {
    /// Valeur de `context.methode`.
    #[must_use]
    pub const fn nom(self) -> &'static str {
        match self {
            Self::Passkey => "passkey",
            Self::Secours => "secours",
            Self::CodeRecuperation => "code_recuperation",
            Self::LienEnrolement => "lien_enrolement",
        }
    }
}

/// Contexte d'une action sur ses facteurs : la session de la requête ou, pour
/// [`ActionFacteur::OuvrirSession`], celle à ouvrir (facteur vérifié à l'instant).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ContexteFacteur {
    /// Nature de la session.
    pub nature: NatureDeSession,
    /// Facteur vérifié à son ouverture.
    pub methode: MethodeDeSession,
    /// Fraîcheur ([`Session::depuis`]).
    pub session: Session,
}

/// Actions du domaine, vérifiées contre le schéma au chargement.
#[derive(Debug, Clone)]
pub(crate) struct ActionsFacteurs {
    actions: HashMap<ActionFacteur, EntityUid>,
    connexion: EntityUid,
    anonyme: EntityTypeName,
    instance: EntityTypeName,
}

/// Nombre d'actions du domaine au schéma : celles de [`ActionFacteur`] et `SeConnecter`.
pub(crate) const NOMBRE_ACTIONS: usize = ActionFacteur::TOUTES.len() + 1;

impl ActionsFacteurs {
    /// Vérifie les actions du domaine et le cloisonnement des principaux `Anonyme` et de la
    /// ressource `Instance`.
    pub(crate) fn verifier(schema: &Schema, types: &Types) -> Result<Self, ErreurChargement> {
        let t = |nom: &str| {
            EntityTypeName::from_str(&format!("{ESPACE}::{nom}"))
                .map_err(|e| ErreurChargement::Schema(e.to_string()))
        };
        let anonyme = t("Anonyme")?;
        let instance = t("Instance")?;
        let type_action = t("Action")?;
        let applicables = |principal: &EntityTypeName, ressource: &EntityTypeName| {
            schema
                .actions_for_principal_and_resource(principal, ressource)
                .cloned()
                .collect::<HashSet<EntityUid>>()
        };
        let de_l_utilisateur = applicables(&types.utilisateur, &types.utilisateur);
        let mut actions = HashMap::new();
        for action in ActionFacteur::TOUTES {
            let id = uid(&type_action, action.nom());
            if !de_l_utilisateur.contains(&id) {
                return Err(ErreurChargement::Actions(format!(
                    "{} : absente du schéma ou mauvais type de principal ou de ressource",
                    action.nom()
                )));
            }
            actions.insert(*action, id);
        }
        let connexion = uid(&type_action, ACTION_CONNEXION);
        if applicables(&anonyme, &instance) != HashSet::from([connexion.clone()]) {
            return Err(ErreurChargement::Actions(format!(
                "{ACTION_CONNEXION} : seule action admise du principal Anonyme sur l'instance"
            )));
        }
        // Le principal Anonyme n'agit sur rien d'autre ; aucun autre principal n'agit sur
        // l'instance (tous les types d'entités du schéma).
        for autre in schema.entity_types() {
            let couples = [
                (autre != &instance).then_some((&anonyme, autre)),
                (autre != &anonyme).then_some((autre, &instance)),
            ];
            for (principal, cible) in couples.into_iter().flatten() {
                if let Some(action) = applicables(principal, cible).into_iter().next() {
                    return Err(ErreurChargement::Actions(format!(
                        "{action} : principal {principal} admis sur {cible}"
                    )));
                }
            }
        }
        Ok(Self {
            actions,
            connexion,
            anonyme,
            instance,
        })
    }
}

impl Autorisation {
    /// Décide la demande de connexion d'un demandeur sans session (`SeConnecter`, principal
    /// `Anonyme`, ressource `Instance`). Toute anomalie donne un refus.
    pub fn verdict_connexion(&self) -> Verdict {
        self.evaluer_connexion()
            .unwrap_or_else(|()| Verdict::refus())
    }

    fn evaluer_connexion(&self) -> Result<Verdict, ()> {
        let f = &self.facteurs;
        let principal = uid(&f.anonyme, "anonyme");
        let ressource = uid(&f.instance, "instance");
        let requete = Request::new(
            principal.clone(),
            f.connexion.clone(),
            ressource.clone(),
            Context::empty(),
            Some(&self.schema),
        )
        .map_err(|_| ())?;
        let entites = Entities::from_entities(
            [
                Entity::new_no_attrs(principal, HashSet::new()),
                Entity::new_no_attrs(ressource, HashSet::new()),
            ],
            Some(&self.schema),
        )
        .map_err(|_| ())?;
        self.repondre(&requete, &entites)
    }

    /// Décide si l'utilisateur `principal` peut effectuer `action` sur les facteurs de
    /// l'utilisateur `ressource` (lui-même), dans le contexte donné. Toute anomalie donne un
    /// refus.
    pub fn verdict_facteur(
        &self,
        principal: IdUtilisateur,
        action: ActionFacteur,
        ressource: IdUtilisateur,
        contexte: ContexteFacteur,
    ) -> Verdict {
        self.evaluer_facteur(principal, action, ressource, contexte)
            .unwrap_or_else(|()| Verdict::refus())
    }

    fn evaluer_facteur(
        &self,
        principal: IdUtilisateur,
        action: ActionFacteur,
        ressource: IdUtilisateur,
        contexte: ContexteFacteur,
    ) -> Result<Verdict, ()> {
        let t = &self.types;
        let id_principal = uid(&t.utilisateur, principal);
        let id_ressource = uid(&t.utilisateur, ressource);
        let mut cedar = vec![Entity::new_no_attrs(id_principal.clone(), HashSet::new())];
        if id_ressource != id_principal {
            cedar.push(Entity::new_no_attrs(id_ressource.clone(), HashSet::new()));
        }
        let action = self.facteurs.actions.get(&action).ok_or(())?.clone();
        let contexte = Context::from_pairs([
            (
                "nature".to_string(),
                RestrictedExpression::new_string(contexte.nature.nom().to_owned()),
            ),
            (
                "methode".to_string(),
                RestrictedExpression::new_string(contexte.methode.nom().to_owned()),
            ),
            (
                "session_fraiche".to_string(),
                RestrictedExpression::new_bool(contexte.session.est_fraiche()),
            ),
        ])
        .map_err(|_| ())?;
        let requete = Request::new(
            id_principal,
            action,
            id_ressource,
            contexte,
            Some(&self.schema),
        )
        .map_err(|_| ())?;
        let entites = Entities::from_entities(cedar, Some(&self.schema)).map_err(|_| ())?;
        self.repondre(&requete, &entites)
    }
}

#[cfg(test)]
mod tests {
    use crate::{Autorisation, ErreurChargement, POLITIQUES, SCHEMA};

    /// Le schéma embarqué, `de` remplacé par `vers` dans le fragment des facteurs.
    fn schema_modifie(de: &str, vers: &str) -> Vec<(&'static str, String)> {
        SCHEMA
            .iter()
            .map(|(nom, texte)| {
                let texte = if nom.ends_with("facteurs.cedarschema") {
                    assert!(texte.contains(de), "{de}");
                    texte.replacen(de, vers, 1)
                } else {
                    (*texte).to_owned()
                };
                (*nom, texte)
            })
            .collect()
    }

    fn charger(schema: &[(&'static str, String)]) -> Result<Autorisation, ErreurChargement> {
        let fragments: Vec<(&str, &str)> = schema.iter().map(|(n, t)| (*n, t.as_str())).collect();
        // Sans politique : seul le contrôle des actions peut refuser.
        Autorisation::charger_depuis(&fragments, &[])
    }

    #[test]
    fn le_principal_anonyme_n_obtient_rien_d_autre() {
        assert!(charger(&schema_modifie("", "")).is_ok());
        for (de, vers) in [
            (
                "action SeConnecter appliesTo { principal: Anonyme, resource: Instance };",
                "action SeConnecter appliesTo { principal: Anonyme, resource: [Instance, Utilisateur] };",
            ),
            (
                "action SeConnecter appliesTo { principal: Anonyme, resource: Instance };",
                "action SeConnecter appliesTo { principal: [Anonyme, Utilisateur], resource: Instance };",
            ),
            (
                "action EnrolerSecours appliesTo {\n    principal: Utilisateur,",
                "action EnrolerSecours appliesTo {\n    principal: [Utilisateur, Anonyme],",
            ),
            (
                "action SeConnecter appliesTo { principal: Anonyme, resource: Instance };",
                "action SeConnecter appliesTo { principal: Anonyme, resource: Instance };\n  \
                 action Autre appliesTo { principal: Anonyme, resource: Instance };",
            ),
        ] {
            assert!(
                matches!(
                    charger(&schema_modifie(de, vers)),
                    Err(ErreurChargement::Actions(_))
                ),
                "{vers}"
            );
        }
        assert!(Autorisation::charger_depuis(&SCHEMA, &POLITIQUES).is_ok());
    }
}
