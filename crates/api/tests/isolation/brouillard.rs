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
        (Methode::Post, c) if c == sur("/suppression") || c == sur("/pieces/{piece}/retrait") => {
            vec![
                attendu(m.auditeur, a.cabinet, a.dossier, interdit()),
                attendu(a.collaborateur, a.cabinet, a.autre_dossier, absent()),
                attendu_croise(m.multi, b.cabinet, b.dossier, a.dossier, absent()),
                attendu(a.administrateur, a.cabinet, a.dossier, perime()),
            ]
        }
        (Methode::Post, c) if c == sur("/a-revoir") || c == sur("/revue-faite") => vec![
            attendu(m.auditeur, a.cabinet, a.dossier, interdit()),
            attendu(a.collaborateur, a.cabinet, a.autre_dossier, absent()),
            attendu_croise(m.multi, b.cabinet, b.dossier, a.dossier, absent()),
            attendu(m.valideur, a.cabinet, a.dossier, ok()),
        ],
        (Methode::Post, c) if c == sur("/pieces") => vec![
            attendu(m.auditeur, a.cabinet, a.dossier, interdit()),
            attendu(a.collaborateur, a.cabinet, a.autre_dossier, absent()),
            attendu_croise(m.multi, b.cabinet, b.dossier, a.dossier, absent()),
            // Pièce d'exemple inexistante : 404 du cas d'usage.
            attendu(a.administrateur, a.cabinet, a.dossier, absent()),
        ],
        _ => return None,
    })
}

/// « Cedar se trompe » (décision forcée à « autorisé ») : la requête et la RLS refusent encore
/// le brouillon d'un autre dossier ou cabinet, et le rôle relu en base refuse l'auditeur.
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
