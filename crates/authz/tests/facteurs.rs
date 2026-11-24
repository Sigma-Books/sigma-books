//! Tests du domaine « facteurs » (ADR 0012, ADR 0017 ; règle absolue 7) : connexion par un
//! principal anonyme, ouverture de session selon le facteur vérifié, ré-authentification,
//! enrôlement de la méthode de secours, codes de récupération, consultation. Cas autorisés et
//! refusés, et propriété : aucun utilisateur n'agit sur les facteurs d'un autre.

// Fichier de test : `expect` hors des fonctions `#[test]` (fixtures).
#![allow(clippy::expect_used)]

use std::sync::LazyLock;

use authz::{
    ActionFacteur, Autorisation, ContexteFacteur, Decision, IdUtilisateur, MethodeDeSession,
    NatureDeSession, Session,
};
use proptest::prelude::*;
use time::OffsetDateTime;
use uuid::Uuid;

static AUTORISATION: LazyLock<Autorisation> =
    LazyLock::new(|| Autorisation::charger().expect("schéma et politiques valides"));

fn utilisateur(n: u128) -> IdUtilisateur {
    IdUtilisateur::new(Uuid::from_u128(n)).expect("UUID non réservé")
}

fn session(fraiche: bool) -> Session {
    let t = OffsetDateTime::UNIX_EPOCH;
    if fraiche {
        Session::depuis(t, t)
    } else {
        Session::depuis(t, t + time::Duration::minutes(6))
    }
}

fn contexte(nature: NatureDeSession, methode: MethodeDeSession, fraiche: bool) -> ContexteFacteur {
    ContexteFacteur {
        nature,
        methode,
        session: session(fraiche),
    }
}

const NATURES: [NatureDeSession; 3] = [
    NatureDeSession::Complete,
    NatureDeSession::Restreinte,
    NatureDeSession::Enrolement,
];
const METHODES: [MethodeDeSession; 4] = [
    MethodeDeSession::Passkey,
    MethodeDeSession::Secours,
    MethodeDeSession::CodeRecuperation,
    MethodeDeSession::LienEnrolement,
];

/// Décision attendue sur soi, d'après l'ADR 0012.
fn attendue(action: ActionFacteur, c: ContexteFacteur) -> Decision {
    use MethodeDeSession as M;
    use NatureDeSession as N;
    let fraiche = c.session.est_fraiche();
    let admis = match action {
        ActionFacteur::OuvrirSession => matches!(
            (c.methode, c.nature),
            (M::Passkey | M::Secours, N::Complete) | (M::CodeRecuperation, N::Restreinte)
        ),
        ActionFacteur::SeReauthentifier
        | ActionFacteur::ConsulterSesFacteurs
        | ActionFacteur::ConsulterSesEvenements => c.nature == N::Complete,
        ActionFacteur::EnrolerSecours | ActionFacteur::EnrolerPasskey => {
            (c.nature == N::Complete && fraiche)
                || (c.nature == N::Restreinte && c.methode == M::CodeRecuperation)
        }
        ActionFacteur::RegenererCodesRecuperation | ActionFacteur::SupprimerPasskey => {
            c.nature == N::Complete && fraiche
        }
    };
    if admis {
        Decision::Autorise
    } else {
        Decision::Refuse
    }
}

/// Matrice complète : chaque action, chaque nature, chaque méthode, fraîche ou non.
#[test]
fn matrice_des_actions_sur_ses_facteurs() {
    let moi = utilisateur(1);
    for action in ActionFacteur::TOUTES {
        for nature in NATURES {
            for methode in METHODES {
                for fraiche in [true, false] {
                    let c = contexte(nature, methode, fraiche);
                    let verdict = AUTORISATION.verdict_facteur(moi, *action, moi, c);
                    assert_eq!(
                        verdict.decision,
                        attendue(*action, c),
                        "{action:?} {nature:?} {methode:?} fraîche={fraiche}"
                    );
                    if verdict.decision == Decision::Autorise {
                        assert_eq!(verdict.politiques.len(), 1, "{action:?}");
                    }
                }
            }
        }
    }
}

#[test]
fn un_code_de_recuperation_n_ouvre_qu_une_session_restreinte() {
    let moi = utilisateur(2);
    for nature in [NatureDeSession::Complete, NatureDeSession::Enrolement] {
        let verdict = AUTORISATION.verdict_facteur(
            moi,
            ActionFacteur::OuvrirSession,
            moi,
            contexte(nature, MethodeDeSession::CodeRecuperation, true),
        );
        assert_eq!(verdict.decision, Decision::Refuse);
        assert_eq!(
            verdict.politiques,
            vec!["invariant-recuperation-restreinte".to_owned()]
        );
    }
    let verdict = AUTORISATION.verdict_facteur(
        moi,
        ActionFacteur::OuvrirSession,
        moi,
        contexte(
            NatureDeSession::Restreinte,
            MethodeDeSession::CodeRecuperation,
            true,
        ),
    );
    assert_eq!(verdict.decision, Decision::Autorise);
    assert_eq!(
        verdict.politiques,
        vec!["adr0012-ouverture-session".to_owned()]
    );
}

#[test]
fn la_connexion_est_une_action_publique() {
    let verdict = AUTORISATION.verdict_connexion();
    assert_eq!(verdict.decision, Decision::Autorise);
    assert_eq!(
        verdict.politiques,
        vec!["adr0012-connexion-publique".to_owned()]
    );
}

#[test]
fn les_facteurs_d_un_autre_sont_refuses_par_l_invariant() {
    let (moi, autre) = (utilisateur(3), utilisateur(4));
    for action in ActionFacteur::TOUTES {
        let c = contexte(NatureDeSession::Complete, MethodeDeSession::Secours, true);
        let verdict = AUTORISATION.verdict_facteur(moi, *action, autre, c);
        assert_eq!(verdict.decision, Decision::Refuse, "{action:?}");
        assert!(
            verdict
                .politiques
                .contains(&"invariant-soi-meme".to_owned()),
            "{action:?} : {:?}",
            verdict.politiques
        );
    }
}

fn action() -> impl Strategy<Value = ActionFacteur> {
    prop::sample::select(ActionFacteur::TOUTES.to_vec())
}

proptest! {
    /// Aucun utilisateur n'agit sur les facteurs d'un autre, quels que soient le contexte et
    /// les identifiants.
    #[test]
    fn aucune_action_sur_les_facteurs_d_un_autre(
        a in 1u128..u128::MAX,
        b in 1u128..u128::MAX,
        action in action(),
        nature in prop::sample::select(NATURES.to_vec()),
        methode in prop::sample::select(METHODES.to_vec()),
        fraiche in any::<bool>(),
    ) {
        prop_assume!(a != b);
        let c = contexte(nature, methode, fraiche);
        prop_assert_eq!(
            AUTORISATION.verdict_facteur(utilisateur(a), action, utilisateur(b), c).decision,
            Decision::Refuse
        );
        prop_assert_eq!(
            AUTORISATION.verdict_facteur(utilisateur(a), action, utilisateur(a), c).decision,
            attendue(action, c)
        );
    }
}
