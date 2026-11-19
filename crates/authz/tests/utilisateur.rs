//! Tests des actions de l'utilisateur sur lui-même (ADR 0012 ; règle
//! absolue 7) : principal `Utilisateur`, ressource `Session` (titulaire) ou
//! l'utilisateur lui-même. Cas autorisés et refusés, et propriété : aucun
//! utilisateur n'obtient d'action sur la session d'un autre.

// Fichier de test : `expect` hors des fonctions `#[test]` (fixtures).
#![allow(clippy::expect_used)]

use std::sync::LazyLock;

use authz::{
    ActionUtilisateur, Autorisation, Decision, IdSession, IdUtilisateur, RessourceUtilisateur,
    TypeRessourceUtilisateur,
};
use proptest::prelude::*;
use uuid::Uuid;

static AUTORISATION: LazyLock<Autorisation> =
    LazyLock::new(|| Autorisation::charger().expect("schéma et politiques valides"));

fn utilisateur(n: u128) -> IdUtilisateur {
    IdUtilisateur::new(Uuid::from_u128(n)).expect("UUID non réservé")
}

fn session(n: u128) -> IdSession {
    IdSession::new(Uuid::from_u128(n)).expect("UUID non réservé")
}

/// La ressource de `action` qui appartient à `titulaire`.
fn ressource(action: ActionUtilisateur, titulaire: IdUtilisateur) -> RessourceUtilisateur {
    match action.type_ressource() {
        TypeRessourceUtilisateur::Session => RessourceUtilisateur::Session {
            id: session(0xabc),
            titulaire,
        },
        TypeRessourceUtilisateur::Utilisateur => RessourceUtilisateur::Utilisateur(titulaire),
    }
}

/// Matrice : chaque action est autorisée sur ce qui est à soi, refusée sur
/// ce qui est à un autre.
#[test]
fn chaque_action_sur_soi_seulement() {
    let (moi, autre) = (utilisateur(1), utilisateur(2));
    for action in ActionUtilisateur::TOUTES {
        let verdict = AUTORISATION.verdict_utilisateur(moi, *action, ressource(*action, moi));
        assert_eq!(verdict.decision, Decision::Autorise, "{action:?}");
        let attendue = match action.type_ressource() {
            TypeRessourceUtilisateur::Session => "adr0012-sa-session",
            TypeRessourceUtilisateur::Utilisateur => "adr0012-ses-sessions",
        };
        assert_eq!(verdict.politiques, vec![attendue.to_owned()], "{action:?}");

        let verdict = AUTORISATION.verdict_utilisateur(moi, *action, ressource(*action, autre));
        assert_eq!(verdict.decision, Decision::Refuse, "{action:?}");
        // Refus par l'invariant du titulaire, jamais par une anomalie.
        let invariant = match action.type_ressource() {
            TypeRessourceUtilisateur::Session => "invariant-titulaire-session",
            TypeRessourceUtilisateur::Utilisateur => "invariant-soi-meme",
        };
        assert_eq!(verdict.politiques, vec![invariant.to_owned()], "{action:?}");
    }
}

#[test]
fn la_session_d_autrui_est_refusee() {
    let (a, b) = (utilisateur(10), utilisateur(11));
    let de_b = RessourceUtilisateur::Session {
        id: session(20),
        titulaire: b,
    };
    for action in [
        ActionUtilisateur::ConsulterSaSession,
        ActionUtilisateur::TournerSaSession,
        ActionUtilisateur::RevoquerSaSession,
    ] {
        assert_eq!(
            AUTORISATION.decider_utilisateur(a, action, de_b),
            Decision::Refuse
        );
        assert_eq!(
            AUTORISATION.decider_utilisateur(b, action, de_b),
            Decision::Autorise
        );
    }
}

#[test]
fn une_ressource_du_mauvais_type_est_refusee() {
    let moi = utilisateur(5);
    // La liste porte sur l'utilisateur ; une action de session sur un utilisateur
    // (ou l'inverse) est rejetée par le schéma : refus.
    assert_eq!(
        AUTORISATION.decider_utilisateur(
            moi,
            ActionUtilisateur::RevoquerSaSession,
            RessourceUtilisateur::Utilisateur(moi)
        ),
        Decision::Refuse
    );
    assert_eq!(
        AUTORISATION.decider_utilisateur(
            moi,
            ActionUtilisateur::ListerSesSessions,
            RessourceUtilisateur::Session {
                id: session(6),
                titulaire: moi
            }
        ),
        Decision::Refuse
    );
}

fn action() -> impl Strategy<Value = ActionUtilisateur> {
    prop::sample::select(ActionUtilisateur::TOUTES.to_vec())
}

proptest! {
    /// Aucun utilisateur n'obtient d'action sur la session d'un autre, ni sur
    /// un autre utilisateur, quels que soient les identifiants.
    #[test]
    fn aucune_action_sur_ce_qui_est_a_un_autre(
        a in 1u128..u128::MAX,
        b in 1u128..u128::MAX,
        s in 1u128..u128::MAX,
        action in action(),
    ) {
        prop_assume!(a != b);
        let (a, b) = (utilisateur(a), utilisateur(b));
        for ressource in [
            RessourceUtilisateur::Session { id: session(s), titulaire: b },
            RessourceUtilisateur::Utilisateur(b),
        ] {
            prop_assert_eq!(
                AUTORISATION.decider_utilisateur(a, action, ressource),
                Decision::Refuse
            );
        }
        // Sur ce qui est à lui, l'action de son type est autorisée.
        prop_assert_eq!(
            AUTORISATION.decider_utilisateur(a, action, ressource(action, a)),
            Decision::Autorise
        );
    }
}
