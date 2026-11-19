//! Autorisation par politiques Cedar, refus par défaut (ADR 0005, spec §20).
//!
//! Le schéma (`politiques/schema.cedarschema`) et les politiques
//! (`politiques/*.cedar`) sont embarqués dans le binaire. [`Autorisation::charger`]
//! les valide contre le schéma en mode strict et échoue sinon : le serveur
//! ne démarre pas avec des politiques invalides.
//!
//! # Modélisation
//! - Le **principal** est une [`Appartenance`] (un utilisateur dans un
//!   cabinet, avec un seul rôle), jamais l'utilisateur seul. L'appelant
//!   choisit l'appartenance d'après le cabinet explicite de la requête
//!   (ADR 0011) ; une décision ne vaut que si la ressource descend de ce
//!   cabinet (F1.14).
//! - Les ressources forment une hiérarchie stricte : [`Cabinet`] ⊃
//!   [`Dossier`] ⊃ [`EcritureBrouillon`] / [`EcritureValidee`].
//! - L'affectation à un dossier ([`AccesDossier`], Q5.1) et la délégation de
//!   validation (Q5.2) passent par le [`Contexte`] ; les politiques vérifient
//!   qu'elle appartient bien au principal et au dossier de la ressource.
//!
//! # Refus en cas de doute
//! Toute anomalie aboutit à [`Decision::Refuse`] : entités non conformes au
//! schéma ou en double, ressource du mauvais type pour l'action, erreur
//! d'évaluation d'une politique (Cedar ignore une politique en erreur : une
//! interdiction en erreur ne doit pas laisser passer une autorisation).
//!
//! # Ressource d'autrui
//! Cedar ne distingue pas « absente » de « refusée ». L'appelant décide
//! d'abord [`Action::AccederDossier`] : un refus donne la réponse d'une
//! ressource absente (404), un refus de l'action elle-même donne 403
//! (ADR 0011).
//!
//! # Validation par lot
//! L'API passe **toujours** par [`Autorisation::decider_lot`], qui exige
//! aussi [`Action::ValiderEcriture`] sur chaque brouillon : décider seulement
//! [`Action::ValiderParLot`] sur le dossier contournerait la séparation des
//! tâches.
//!
//! # Hors Cedar
//! La gestion de ses propres facteurs (passkeys, secours, codes de
//! récupération, ADR 0012) relève de l'utilisateur lui-même, hors de tout
//! cabinet : elle n'est pas une action Cedar ; l'authentification (PR 8) la
//! protège par la ré-authentification.
//!
//! # Rappels pour l'API (PR 6)
//! - L'appartenance (principal), son rôle, son état actif et son cabinet
//!   sont **lus en base** d'après l'utilisateur authentifié et le cabinet
//!   **du chemin** de la requête, jamais d'après un identifiant envoyé par
//!   le client (corps, en-tête, paramètre).
//! - Le parent `Dossier → Cabinet`, le dossier d'une écriture, ses
//!   contributeurs et l'affectation sont eux aussi lus en base, dans le
//!   contexte du cabinet du chemin.
//! - La fraîcheur de la session se calcule par [`Session::depuis`], à partir
//!   de l'instant de la dernière vérification de facteur enregistré en base.

#![forbid(unsafe_code)]

mod action;
mod entites;

use std::collections::{HashMap, HashSet};
use std::str::FromStr;

use cedar_policy::{
    Authorizer, Context, Entities, Entity, EntityId, EntityTypeName, EntityUid, PolicyId,
    PolicySet, Request, RestrictedExpression, Schema, ValidationMode, Validator,
};

pub use action::{Action, TypeRessource};
pub use entites::{
    AccesDossier, Appartenance, Cabinet, Contexte, DUREE_FRAICHEUR, Dossier, EcritureBrouillon,
    EcritureValidee, Entite, Entites, ErreurIdentifiant, IdAccesDossier, IdAppartenance, IdCabinet,
    IdDossier, IdEcriture, IdUtilisateur, Ressource, Role, Session,
};

/// Espace de noms Cedar des entités et des actions.
const ESPACE: &str = "Sigma";

/// Schéma Cedar, embarqué.
pub const SCHEMA: &str = include_str!("../politiques/schema.cedarschema");

/// Fichiers de politiques, embarqués : (nom, contenu).
pub const POLITIQUES: [(&str, &str); 2] = [
    (
        "invariants.cedar",
        include_str!("../politiques/invariants.cedar"),
    ),
    ("roles.cedar", include_str!("../politiques/roles.cedar")),
];

/// Échec du chargement du schéma ou des politiques.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ErreurChargement {
    /// Le schéma ne s'analyse pas ou produit des avertissements.
    #[error("schéma Cedar invalide : {0}")]
    Schema(String),
    /// Un fichier de politiques ne s'analyse pas.
    #[error("politiques invalides ({fichier}) : {detail}")]
    Politiques {
        /// Fichier en cause.
        fichier: &'static str,
        /// Message de Cedar.
        detail: String,
    },
    /// Une politique n'a pas d'annotation `@id`, ou elle est en double.
    #[error("identifiant de politique absent ou en double ({fichier}) : {detail}")]
    Identifiant {
        /// Fichier en cause.
        fichier: &'static str,
        /// Détail.
        detail: String,
    },
    /// La validation des politiques contre le schéma échoue ou avertit.
    #[error("politiques non conformes au schéma : {0}")]
    Validation(String),
    /// L'énumération [`Action`] et le schéma divergent.
    #[error("actions du schéma et du code divergentes : {0}")]
    Actions(String),
}

/// Décision d'autorisation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[must_use]
pub enum Decision {
    /// Au moins une autorisation s'applique, aucune interdiction.
    Autorise,
    /// Refus : par défaut, par une interdiction, ou en cas d'anomalie.
    Refuse,
}

/// Décision accompagnée des politiques déterminantes (`@id`), pour le
/// journal d'audit (ADR 0014, clé `politique=`) et les journaux techniques.
#[derive(Debug, Clone, PartialEq, Eq)]
#[must_use]
pub struct Verdict {
    /// La décision.
    pub decision: Decision,
    /// Politiques déterminantes : autorisations appliquées si
    /// [`Decision::Autorise`], interdictions appliquées si refus (vide pour
    /// un refus par défaut ou une anomalie).
    pub politiques: Vec<String>,
}

impl Verdict {
    fn refus() -> Self {
        Self {
            decision: Decision::Refuse,
            politiques: Vec::new(),
        }
    }
}

/// Noms des types d'entités, analysés une fois au chargement.
#[derive(Debug, Clone)]
struct Types {
    role: EntityTypeName,
    utilisateur: EntityTypeName,
    cabinet: EntityTypeName,
    appartenance: EntityTypeName,
    dossier: EntityTypeName,
    acces: EntityTypeName,
    brouillon: EntityTypeName,
    validee: EntityTypeName,
}

impl Types {
    fn analyser() -> Result<Self, ErreurChargement> {
        let t = |nom: &str| {
            EntityTypeName::from_str(&format!("{ESPACE}::{nom}"))
                .map_err(|e| ErreurChargement::Schema(e.to_string()))
        };
        Ok(Self {
            role: t("Role")?,
            utilisateur: t("Utilisateur")?,
            cabinet: t("Cabinet")?,
            appartenance: t("Appartenance")?,
            dossier: t("Dossier")?,
            acces: t("AccesDossier")?,
            brouillon: t(TypeRessource::EcritureBrouillon.nom())?,
            validee: t(TypeRessource::EcritureValidee.nom())?,
        })
    }

    fn ressource(&self, type_ressource: TypeRessource) -> &EntityTypeName {
        match type_ressource {
            TypeRessource::Cabinet => &self.cabinet,
            TypeRessource::Dossier => &self.dossier,
            TypeRessource::EcritureBrouillon => &self.brouillon,
            TypeRessource::EcritureValidee => &self.validee,
        }
    }
}

/// Identifiant Cedar d'une entité : l'UUID sous sa forme canonique unique
/// (à tirets, en minuscules), pour que deux mentions du même UUID soient
/// toujours égales (séparation des tâches, affectation).
fn uid(type_entite: &EntityTypeName, id: impl std::fmt::Display) -> EntityUid {
    EntityUid::from_type_name_and_id(type_entite.clone(), EntityId::new(id.to_string()))
}

fn reference(type_entite: &EntityTypeName, id: impl std::fmt::Display) -> RestrictedExpression {
    RestrictedExpression::new_entity_uid(uid(type_entite, id))
}

/// Le moteur d'autorisation : schéma et politiques validés.
#[derive(Debug, Clone)]
pub struct Autorisation {
    schema: Schema,
    politiques: PolicySet,
    autorisateur: Authorizer,
    types: Types,
    actions: HashMap<Action, EntityUid>,
}

impl Autorisation {
    /// Charge le schéma et les politiques embarqués et les valide.
    ///
    /// # Errors
    /// [`ErreurChargement`] si le schéma ou une politique ne s'analyse pas,
    /// si une politique n'a pas d'`@id` unique, si la validation stricte
    /// contre le schéma échoue ou avertit, ou si [`Action`] diverge du schéma.
    /// L'appelant doit alors refuser de démarrer.
    pub fn charger() -> Result<Self, ErreurChargement> {
        Self::charger_depuis(SCHEMA, &POLITIQUES)
    }

    fn charger_depuis(
        schema_src: &str,
        fichiers: &[(&'static str, &str)],
    ) -> Result<Self, ErreurChargement> {
        let (schema, avertissements) = Schema::from_cedarschema_str(schema_src)
            .map_err(|e| ErreurChargement::Schema(format!("{e:?}")))?;
        let avertissements: Vec<String> = avertissements.map(|a| a.to_string()).collect();
        if !avertissements.is_empty() {
            return Err(ErreurChargement::Schema(avertissements.join(" ; ")));
        }

        let mut politiques = PolicySet::new();
        for (fichier, texte) in fichiers {
            let ensemble =
                PolicySet::from_str(texte).map_err(|e| ErreurChargement::Politiques {
                    fichier,
                    detail: format!("{e:?}"),
                })?;
            if ensemble.templates().next().is_some() {
                return Err(ErreurChargement::Politiques {
                    fichier,
                    detail: "les modèles de politiques ne sont pas admis".into(),
                });
            }
            for politique in ensemble.policies() {
                let id =
                    politique
                        .annotation("id")
                        .ok_or_else(|| ErreurChargement::Identifiant {
                            fichier,
                            detail: format!("politique sans @id : {politique}"),
                        })?;
                politiques
                    .add(politique.new_id(PolicyId::new(id)))
                    .map_err(|e| ErreurChargement::Identifiant {
                        fichier,
                        detail: e.to_string(),
                    })?;
            }
        }

        let resultat = Validator::new(schema.clone()).validate(&politiques, ValidationMode::Strict);
        if !resultat.validation_passed_without_warnings() {
            let messages: Vec<String> = resultat
                .validation_errors()
                .map(ToString::to_string)
                .chain(resultat.validation_warnings().map(ToString::to_string))
                .collect();
            return Err(ErreurChargement::Validation(messages.join(" ; ")));
        }

        let types = Types::analyser()?;
        let actions = Self::verifier_actions(&schema, &types)?;
        Ok(Self {
            schema,
            politiques,
            autorisateur: Authorizer::new(),
            types,
            actions,
        })
    }

    /// Vérifie que [`Action`] et le schéma déclarent les mêmes actions, sur
    /// les mêmes types de ressources.
    fn verifier_actions(
        schema: &Schema,
        types: &Types,
    ) -> Result<HashMap<Action, EntityUid>, ErreurChargement> {
        let type_action = EntityTypeName::from_str(&format!("{ESPACE}::Action"))
            .map_err(|e| ErreurChargement::Actions(e.to_string()))?;
        let declarees: HashSet<&EntityUid> = schema.actions().collect();
        if declarees.len() != Action::TOUTES.len() {
            return Err(ErreurChargement::Actions(format!(
                "{} actions au schéma, {} dans le code",
                declarees.len(),
                Action::TOUTES.len()
            )));
        }
        let mut actions = HashMap::new();
        for action in Action::TOUTES {
            let id = uid(&type_action, action.nom());
            let applicables: HashSet<&EntityUid> = schema
                .actions_for_principal_and_resource(
                    &types.appartenance,
                    types.ressource(action.type_ressource()),
                )
                .collect();
            if !declarees.contains(&id) || !applicables.contains(&id) {
                return Err(ErreurChargement::Actions(format!(
                    "{} : absente du schéma ou mauvais type de ressource",
                    action.nom()
                )));
            }
            actions.insert(*action, id);
        }
        Ok(actions)
    }

    /// Décide si l'appartenance `principal` peut effectuer `action` sur
    /// `ressource`. L'entité du principal est ajoutée ici : elle ne doit pas
    /// figurer dans `entites`. Toute anomalie donne [`Decision::Refuse`].
    pub fn decider(
        &self,
        principal: &Appartenance,
        action: Action,
        ressource: Ressource,
        entites: &Entites,
        contexte: &Contexte,
    ) -> Decision {
        self.verdict(principal, action, ressource, entites, contexte)
            .decision
    }

    /// Comme [`Autorisation::decider`], avec les politiques déterminantes.
    pub fn verdict(
        &self,
        principal: &Appartenance,
        action: Action,
        ressource: Ressource,
        entites: &Entites,
        contexte: &Contexte,
    ) -> Verdict {
        self.evaluer(principal, action, ressource, entites, contexte)
            .unwrap_or_else(|()| Verdict::refus())
    }

    /// Validation par lot (§20) : **seul point d'entrée admis** pour valider
    /// plusieurs brouillons à la fois. Exige [`Action::ValiderParLot`] sur le
    /// dossier **et** [`Action::ValiderEcriture`] sur chaque brouillon
    /// (séparation des tâches, délégation, affectation). Un seul refus, ou
    /// un lot vide, refuse tout le lot. Chaque brouillon doit figurer dans
    /// `entites`, avec ses contributeurs.
    pub fn decider_lot(
        &self,
        principal: &Appartenance,
        dossier: IdDossier,
        brouillons: &[IdEcriture],
        entites: &Entites,
        contexte: &Contexte,
    ) -> Decision {
        if brouillons.is_empty() {
            return Decision::Refuse;
        }
        let lot = self.decider(
            principal,
            Action::ValiderParLot,
            Ressource::Dossier(dossier),
            entites,
            contexte,
        );
        if lot == Decision::Refuse {
            return Decision::Refuse;
        }
        let tous = brouillons.iter().all(|brouillon| {
            self.decider(
                principal,
                Action::ValiderEcriture,
                Ressource::EcritureBrouillon(*brouillon),
                entites,
                contexte,
            ) == Decision::Autorise
        });
        if tous {
            Decision::Autorise
        } else {
            Decision::Refuse
        }
    }

    fn evaluer(
        &self,
        principal: &Appartenance,
        action: Action,
        ressource: Ressource,
        entites: &Entites,
        contexte: &Contexte,
    ) -> Result<Verdict, ()> {
        let t = &self.types;
        let id_principal = uid(&t.appartenance, principal.id);
        let action = self.actions.get(&action).ok_or(())?.clone();
        let ressource = match ressource {
            Ressource::Cabinet(id) => uid(&t.cabinet, id),
            Ressource::Dossier(id) => uid(&t.dossier, id),
            Ressource::EcritureBrouillon(id) => uid(&t.brouillon, id),
            Ressource::EcritureValidee(id) => uid(&t.validee, id),
        };
        let contexte = self.contexte(contexte)?;
        // Le schéma vérifie la requête : type du principal, type de la
        // ressource pour l'action, forme du contexte.
        let requete = Request::new(
            id_principal,
            action,
            ressource,
            contexte,
            Some(&self.schema),
        )
        .map_err(|_| ())?;
        let entites = self.entites(principal, entites)?;
        let reponse = self
            .autorisateur
            .is_authorized(&requete, &self.politiques, &entites);
        if reponse.diagnostics().errors().next().is_some() {
            return Err(());
        }
        let politiques = reponse
            .diagnostics()
            .reason()
            .map(ToString::to_string)
            .collect();
        let decision = match reponse.decision() {
            cedar_policy::Decision::Allow => Decision::Autorise,
            cedar_policy::Decision::Deny => Decision::Refuse,
        };
        Ok(Verdict {
            decision,
            politiques,
        })
    }

    fn contexte(&self, contexte: &Contexte) -> Result<Context, ()> {
        let mut paires = vec![(
            "session_fraiche".to_string(),
            RestrictedExpression::new_bool(contexte.session().est_fraiche()),
        )];
        if let Some(acces) = contexte.acces() {
            paires.push((
                "acces".to_string(),
                RestrictedExpression::new_entity_uid(uid(&self.types.acces, acces)),
            ));
        }
        Context::from_pairs(paires).map_err(|_| ())
    }

    fn entites(&self, principal: &Appartenance, entites: &Entites) -> Result<Entities, ()> {
        let t = &self.types;
        let principal = Entite::Appartenance(principal.clone());
        let toutes = std::iter::once(&principal).chain(entites.liste());
        let mut cedar = Vec::with_capacity(entites.liste().len() + 1);
        let mut vus = HashSet::with_capacity(entites.liste().len() + 1);
        for entite in toutes {
            let (id, attributs, parents) = match entite {
                Entite::Cabinet(c) => (
                    uid(&t.cabinet, c.id),
                    vec![(
                        "separation_des_taches",
                        RestrictedExpression::new_bool(c.separation_des_taches),
                    )],
                    vec![],
                ),
                Entite::Appartenance(a) => (
                    uid(&t.appartenance, a.id),
                    vec![
                        ("utilisateur", reference(&t.utilisateur, a.utilisateur)),
                        ("cabinet", reference(&t.cabinet, a.cabinet)),
                        (
                            "role",
                            RestrictedExpression::new_entity_uid(EntityUid::from_type_name_and_id(
                                t.role.clone(),
                                EntityId::new(a.role.nom()),
                            )),
                        ),
                        ("actif", RestrictedExpression::new_bool(a.actif)),
                    ],
                    vec![],
                ),
                Entite::Dossier(d) => (
                    uid(&t.dossier, d.id),
                    vec![],
                    vec![uid(&t.cabinet, d.cabinet)],
                ),
                Entite::AccesDossier(a) => (
                    uid(&t.acces, a.id),
                    vec![
                        ("appartenance", reference(&t.appartenance, a.appartenance)),
                        ("dossier", reference(&t.dossier, a.dossier)),
                        (
                            "delegation_validation",
                            RestrictedExpression::new_bool(a.delegation_validation),
                        ),
                    ],
                    vec![],
                ),
                Entite::EcritureBrouillon(e) => (
                    uid(&t.brouillon, e.id),
                    vec![(
                        "contributeurs",
                        RestrictedExpression::new_set(
                            e.contributeurs().map(|u| reference(&t.utilisateur, u)),
                        ),
                    )],
                    vec![uid(&t.dossier, e.dossier)],
                ),
                Entite::EcritureValidee(e) => (
                    uid(&t.validee, e.id),
                    vec![],
                    vec![uid(&t.dossier, e.dossier)],
                ),
            };
            // Une entité en double, même identique (principal compris), est
            // une anomalie de l'appelant : refus.
            if !vus.insert(id.clone()) {
                return Err(());
            }
            let attributs = attributs
                .into_iter()
                .map(|(cle, valeur)| (cle.to_string(), valeur))
                .collect();
            cedar.push(Entity::new(id, attributs, parents.into_iter().collect()).map_err(|_| ())?);
        }
        // Le schéma vérifie chaque entité : attributs, types des parents.
        Entities::from_entities(cedar, Some(&self.schema)).map_err(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn les_politiques_embarquees_sont_valides_contre_le_schema() {
        let autorisation = Autorisation::charger();
        assert!(autorisation.is_ok(), "{autorisation:?}");
    }

    #[test]
    fn une_politique_non_conforme_au_schema_empeche_le_chargement() {
        let fichiers = [(
            "faux.cedar",
            r#"@id("faux") permit (principal, action == Sigma::Action::"GererCabinet", resource)
               when { principal.grade == "chef" };"#,
        )];
        assert!(matches!(
            Autorisation::charger_depuis(SCHEMA, &fichiers),
            Err(ErreurChargement::Validation(_))
        ));
    }

    #[test]
    fn une_action_inconnue_empeche_le_chargement() {
        let fichiers = [(
            "faux.cedar",
            r#"@id("faux") permit (principal, action == Sigma::Action::"ModifierEcritureValidee", resource);"#,
        )];
        assert!(matches!(
            Autorisation::charger_depuis(SCHEMA, &fichiers),
            Err(ErreurChargement::Validation(_))
        ));
    }

    #[test]
    fn une_politique_sans_identifiant_empeche_le_chargement() {
        let fichiers = [(
            "faux.cedar",
            r#"permit (principal, action == Sigma::Action::"GererCabinet", resource);"#,
        )];
        assert!(matches!(
            Autorisation::charger_depuis(SCHEMA, &fichiers),
            Err(ErreurChargement::Identifiant { .. })
        ));
    }

    #[test]
    fn un_identifiant_en_double_empeche_le_chargement() {
        let fichiers = [(
            "faux.cedar",
            r#"@id("x") permit (principal, action == Sigma::Action::"GererCabinet", resource);
               @id("x") permit (principal, action == Sigma::Action::"CreerDossier", resource);"#,
        )];
        assert!(matches!(
            Autorisation::charger_depuis(SCHEMA, &fichiers),
            Err(ErreurChargement::Identifiant { .. })
        ));
    }

    #[test]
    fn une_action_du_schema_absente_du_code_empeche_le_chargement() {
        let schema = SCHEMA.replacen(
            "namespace Sigma {",
            "namespace Sigma {\n  action ActionFantome appliesTo { principal: Appartenance, resource: Cabinet, context: Contexte };",
            1,
        );
        assert!(matches!(
            Autorisation::charger_depuis(&schema, &POLITIQUES),
            Err(ErreurChargement::Actions(_))
        ));
    }

    #[test]
    fn un_schema_invalide_empeche_le_chargement() {
        assert!(matches!(
            Autorisation::charger_depuis("namespace Sigma { entity ; }", &POLITIQUES),
            Err(ErreurChargement::Schema(_))
        ));
    }
}
