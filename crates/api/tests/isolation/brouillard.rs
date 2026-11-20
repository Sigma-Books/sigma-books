//! Cas d'isolation des routes du module `brouillard` (PR 13a ; spec §20, ADR 0011).
//!
//! Chaque dossier du monde a un brouillon réel (version 1), sous-ressource `{brouillon}` : le
//! harnais vérifie en outre que le brouillon d'un autre dossier (de B, ou non affecté), placé
//! sous un dossier de A, donne le même 404 qu'un brouillon inexistant.
//!
//! Les cas sont **ordonnés** (les routes le sont aussi : la table suit l'ordre d'un parcours) :
//! la modification réussit une fois (version 1 → 2), si bien que la suppression et le retrait,
//! qui portent la version 1 dans leur corps d'exemple, atteignent le cas d'usage et y sont
//! refusés (409 `brouillon_perime`) : rien n'est supprimé, les routes suivantes gardent leur
//! brouillon. Le lien d'une pièce (corps d'exemple en version 2, celle d'après la modification) (pièce d'exemple inexistante) donne 404 ; les liens réels sont
//! dans `tests/brouillard.rs`.

use api::serveur::routes::{Methode, Route};
use axum::Router;
use axum::http::StatusCode;
use uuid::Uuid;

use super::{Attendu, CasModule, Monde, Resultat, attendu, attendu_croise, lire};

pub(super) const CAS: CasModule = CasModule {
    attentes,
    hors_cabinet: CasModule::AUCUN.hors_cabinet,
};

#[allow(clippy::too_many_lines)]
fn attentes(route: &Route, m: &Monde) -> Option<Vec<Attendu>> {
    use Resultat::{Cree, Statut};
    let (a, b) = (&m.a, &m.b);
    let ok = || Statut(StatusCode::OK);
    let interdit = || Statut(StatusCode::FORBIDDEN);
    let absent = || Statut(StatusCode::NOT_FOUND);
    let perime = || Statut(StatusCode::CONFLICT);
    let racine = "/api/cabinets/{cabinet}/dossiers/{dossier}/brouillard";
    let sur = |suffixe: &str| format!("{racine}/{{brouillon}}{suffixe}");
    Some(match (route.methode, route.chemin) {
        (Methode::Post, c) if c == racine => vec![
            attendu(m.auditeur, a.cabinet, a.dossier, interdit()),
            attendu(a.collaborateur, a.cabinet, a.autre_dossier, absent()),
            attendu(a.collaborateur, a.cabinet, a.dossier, Cree),
            attendu(m.valideur, a.cabinet, a.dossier, Cree),
            attendu(a.administrateur, a.cabinet, a.autre_dossier, Cree),
            attendu(m.multi, b.cabinet, b.dossier, Cree),
        ],
        (Methode::Get, c) if c == racine => vec![
            attendu(a.administrateur, a.cabinet, a.autre_dossier, ok()),
            attendu(m.auditeur, a.cabinet, a.dossier, ok()),
            attendu(a.collaborateur, a.cabinet, a.autre_dossier, absent()),
            attendu(m.multi, b.cabinet, b.dossier, ok()),
            attendu(m.multi, b.cabinet, b.autre_dossier, absent()),
        ],
        (Methode::Get, c) if c == sur("") => vec![
            attendu(a.administrateur, a.cabinet, a.dossier, ok()),
            attendu(m.auditeur, a.cabinet, a.dossier, ok()),
            attendu(a.collaborateur, a.cabinet, a.autre_dossier, absent()),
            attendu(m.multi, b.cabinet, b.dossier, ok()),
            attendu_croise(
                a.administrateur,
                a.cabinet,
                a.dossier,
                a.autre_dossier,
                absent(),
            ),
            attendu_croise(m.multi, b.cabinet, b.dossier, a.dossier, absent()),
        ],
        (Methode::Post, c) if c == sur("/modification") => vec![
            attendu(m.auditeur, a.cabinet, a.dossier, interdit()),
            attendu(a.collaborateur, a.cabinet, a.autre_dossier, absent()),
            attendu_croise(m.multi, b.cabinet, b.dossier, a.dossier, absent()),
            attendu(a.collaborateur, a.cabinet, a.dossier, ok()),
        ],
        (Methode::Post, c)
            if c == sur("/suppression")
                || c == sur("/pieces/{piece}/retrait")
                || c == sur("/pieces") =>
        {
            vec![
                attendu(m.auditeur, a.cabinet, a.dossier, interdit()),
                attendu(a.collaborateur, a.cabinet, a.autre_dossier, absent()),
                attendu_croise(m.multi, b.cabinet, b.dossier, a.dossier, absent()),
                attendu(a.administrateur, a.cabinet, a.dossier, perime()),
            ]
        }
        (Methode::Post, c) if c == sur("/a-revoir") || c == sur("/revue-faite") => {
            // Le valideur pose le drapeau ; l'administrateur le lève (pas le poseur).
            let acteur = if c == sur("/a-revoir") {
                m.valideur
            } else {
                a.administrateur
            };
            vec![
                attendu(m.auditeur, a.cabinet, a.dossier, interdit()),
                attendu(a.collaborateur, a.cabinet, a.autre_dossier, absent()),
                attendu_croise(m.multi, b.cabinet, b.dossier, a.dossier, absent()),
                attendu(acteur, a.cabinet, a.dossier, ok()),
            ]
        }
        // Validation (PR 13b) : le corps d'exemple porte la version 0, périmée : le valideur
        // atteint le cas d'usage et y est refusé (409), sans rien valider. Le créateur du
        // brouillon fixe (l'administrateur) est refusé par la séparation des tâches (403).
        (Methode::Post, c) if c == sur("/validation") => vec![
            attendu(m.auditeur, a.cabinet, a.dossier, interdit()),
            attendu(a.administrateur, a.cabinet, a.dossier, interdit()),
            attendu(a.collaborateur, a.cabinet, a.dossier, interdit()),
            attendu(a.collaborateur, a.cabinet, a.autre_dossier, absent()),
            attendu_croise(m.multi, b.cabinet, b.dossier, a.dossier, absent()),
            attendu(m.valideur, a.cabinet, a.dossier, perime()),
        ],
        (Methode::Get, "/api/cabinets/{cabinet}/dossiers/{dossier}/ecritures") => vec![
            attendu(a.administrateur, a.cabinet, a.autre_dossier, ok()),
            attendu(m.auditeur, a.cabinet, a.dossier, ok()),
            attendu(a.collaborateur, a.cabinet, a.autre_dossier, absent()),
            attendu(m.multi, b.cabinet, b.autre_dossier, absent()),
        ],
        (Methode::Get, "/api/cabinets/{cabinet}/dossiers/{dossier}/ecritures/{ecriture}") => {
            vec![
                attendu(a.administrateur, a.cabinet, a.dossier, ok()),
                attendu(m.auditeur, a.cabinet, a.dossier, ok()),
                attendu(a.collaborateur, a.cabinet, a.autre_dossier, absent()),
                attendu_croise(
                    a.administrateur,
                    a.cabinet,
                    a.dossier,
                    a.autre_dossier,
                    absent(),
                ),
                attendu_croise(m.multi, b.cabinet, b.dossier, a.dossier, absent()),
            ]
        }
        _ => return None,
    })
}

/// « Cedar se trompe » (décision forcée à « autorisé ») : la requête et la RLS refusent encore
/// le brouillon d'un autre dossier ou cabinet, et le rôle relu en base refuse l'auditeur.
#[allow(clippy::too_many_lines)]
pub(super) async fn quand_cedar_se_trompe(routeur: &Router, m: &Monde, reference: &super::Reponse) {
    let (a, b) = (&m.a, &m.b);
    let brouillon_de = |dossier: api::base::IdDossier| {
        super::SOUS_RESSOURCES
            .lock()
            .unwrap()
            .get(&(dossier.uuid(), "brouillon"))
            .copied()
            .unwrap()
    };
    let propre = brouillon_de(a.dossier);
    let autre_dossier = brouillon_de(a.autre_dossier);
    let de_b = brouillon_de(b.dossier);
    let base = |c: Uuid, d: Uuid| format!("/api/cabinets/{c}/dossiers/{d}/brouillard");
    // Brouillon d'un autre dossier de A, d'un dossier de B ou inexistant, sous le dossier de A.
    for id in [autre_dossier, de_b, Uuid::new_v4()] {
        let r = lire(
            routeur,
            &format!("{}/{id}", base(a.cabinet.uuid(), a.dossier.uuid())),
            Some(a.administrateur),
        )
        .await;
        super::verifier_absent(&r, reference, "brouillon d'autrui, Cedar forcé");
    }
    // Dossier non affecté.
    let r = lire(
        routeur,
        &format!(
            "{}/{autre_dossier}",
            base(a.cabinet.uuid(), a.autre_dossier.uuid())
        ),
        Some(a.collaborateur),
    )
    .await;
    super::verifier_absent(&r, reference, "dossier non affecté, Cedar forcé");
    // Auditeur : Cedar (forcé) autoriserait, le rôle relu en base refuse (403) sans rien changer.
    for (suffixe, corps) in [
        (
            "modification",
            crate::commun::brouillard::saisie("Pirate", Some(1)),
        ),
        ("suppression", r#"{"version":1}"#.to_owned()),
        ("a-revoir", "{}".to_owned()),
        ("validation", r#"{"version":1}"#.to_owned()),
    ] {
        let r = super::envoyer(
            routeur,
            &format!(
                "{}/{propre}/{suffixe}",
                base(a.cabinet.uuid(), a.dossier.uuid())
            ),
            Some(m.auditeur),
            &corps,
            None,
        )
        .await;
        assert_eq!(r.statut, StatusCode::FORBIDDEN, "{suffixe} : {}", r.texte());
    }
    // Validation, Cedar forcé : le créateur du brouillon fixe (l'administrateur) est refusé par la
    // séparation des tâches relue dans le cas d'usage, et le collaborateur sans délégation par le
    // rôle relu en base ; rien n'est validé.
    for u in [a.administrateur, a.collaborateur] {
        let r = super::envoyer(
            routeur,
            &format!(
                "{}/{propre}/validation",
                base(a.cabinet.uuid(), a.dossier.uuid())
            ),
            Some(u),
            r#"{"version":1}"#,
            None,
        )
        .await;
        assert_eq!(
            r.statut,
            StatusCode::FORBIDDEN,
            "validation Cedar forcé : {}",
            r.texte()
        );
    }
    // Modification et suppression du brouillon d'un autre dossier (de A non affecté, ou de B),
    // sous un dossier de A, au nom de l'administrateur de A : 404, rien n'est modifié.
    let racine = base(a.cabinet.uuid(), a.dossier.uuid());
    for cible in [autre_dossier, de_b] {
        for (suffixe, corps) in [
            (
                "modification",
                crate::commun::brouillard::saisie("Pirate", Some(1)),
            ),
            ("suppression", r#"{"version":1}"#.to_owned()),
        ] {
            let r = super::envoyer(
                routeur,
                &format!("{racine}/{cible}/{suffixe}"),
                Some(a.administrateur),
                &corps,
                None,
            )
            .await;
            super::verifier_absent(&r, reference, &format!("{suffixe} d'autrui, Cedar forcé"));
        }
    }
    for (dossier, cabinet, id) in [
        (a.autre_dossier, a.cabinet, autre_dossier),
        (b.dossier, b.cabinet, de_b),
    ] {
        let r = lire(
            routeur,
            &format!("{}/{id}", base(cabinet.uuid(), dossier.uuid())),
            Some(if cabinet == a.cabinet {
                a.administrateur
            } else {
                m.multi
            }),
        )
        .await;
        assert_eq!(r.statut, StatusCode::OK, "{}", r.texte());
        assert_eq!(r.json()["version"], 1);
        assert_eq!(r.json()["libelle"], "Brouillon fixe");
    }
    let r = super::envoyer(
        routeur,
        &base(a.cabinet.uuid(), a.dossier.uuid()),
        Some(m.auditeur),
        &crate::commun::brouillard::saisie("Pirate", None),
        None,
    )
    .await;
    assert_eq!(r.statut, StatusCode::FORBIDDEN);
}
