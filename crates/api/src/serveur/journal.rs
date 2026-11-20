//! Journaux techniques en **liste blanche** de champs (F2.11, VP3 ; ASVS 4.0.3 V7.1.1, V7.1.2).
//!
//! Liste blanche ([`CHAMPS`]) : un socle commun, plus les champs que déclare chaque module (le
//! client d'horodatage, et chaque module de routes dans sa tranche). Un module n'ajoute ses
//! champs que dans son propre fichier.
//!
//! Une seule ligne par requête, émise à la fin du traitement sous la cible [`CIBLE`], avec les
//! seuls champs du socle :
//! - `methode` : méthode HTTP connue, sinon `autre` ;
//! - `route` : **modèle** de la route (`/api/cabinets/{cabinet}`), `statique` pour un fichier de
//!   l'interface, `aucune` hors table ; jamais le chemin réel, qui pourrait porter n'importe quoi ;
//! - `statut`, `duree_ms` ;
//! - `requete` : identifiant tiré par le serveur (UUID v4), renvoyé dans l'en-tête
//!   `x-sigma-requete` ; jamais repris du client (injection dans les journaux) ;
//! - `erreur` : catégorie d'une erreur interne (liste fermée, [`super::CategorieErreur`]).
//!
//! Le client d'horodatage ([`crate::horodatage`]) émet une ligne par autorité interrogée, sous
//! la même cible : `autorite` (hôte et port de la configuration, jamais le chemin), `statut`,
//! `duree_ms` et `issue` (liste fermée, [`crate::horodatage::EchecAutorite::issue`]) ; ses
//! champs propres sont déclarés par [`crate::horodatage::CHAMPS_JOURNAL`].
//!
//! Jamais de montant, de libellé, de nom, de jeton, de cookie, d'en-tête ni de corps. Les
//! journaux des dépendances (`sqlx` cite le texte des requêtes) sont écartés par [`filtre`].

use std::time::Instant;

use axum::extract::{MatchedPath, Request};
use axum::http::{HeaderName, HeaderValue, Method};
use axum::middleware::Next;
use axum::response::Response;
use tracing::{Level, Metadata};
use tracing_subscriber::filter::FilterFn;
use uuid::Uuid;

use super::erreur::CategorieErreur;
use super::routes::{MODULES, Module};

/// Cible `tracing` des journaux de Sigma Books.
pub const CIBLE: &str = "sigma";

/// Champs du socle : ligne de chaque requête, démarrage du serveur. `message` ne reçoit que des
/// textes constants.
const SOCLE: &[&str] = &[
    "message",
    "methode",
    "route",
    "statut",
    "duree_ms",
    "requete",
    "erreur",
    "adresse_ecoute",
];

/// Champs des modules hors routes : le socle, puis le client d'horodatage.
const HORS_ROUTES: &[&[&str]] = &[SOCLE, crate::horodatage::CHAMPS_JOURNAL];

/// Champs admis dans un événement de la cible [`CIBLE`] : le socle, ceux des modules hors routes
/// ([`crate::horodatage::CHAMPS_JOURNAL`]), puis ceux que chaque module de routes déclare dans sa
/// tranche ([`super::routes::Module::champs_journal`]). Assemblés à la compilation.
pub const CHAMPS: &[&str] = &TABLE_CHAMPS;

const NOMBRE_DE_CHAMPS: usize = nombre_de_champs(HORS_ROUTES, MODULES);
const TABLE_CHAMPS: [&str; NOMBRE_DE_CHAMPS] = assembler(HORS_ROUTES, MODULES);

const fn nombre_de_champs(hors_routes: &[&[&str]], modules: &[Module]) -> usize {
    let mut n = 0;
    let mut i = 0;
    while i < hors_routes.len() {
        n += hors_routes[i].len();
        i += 1;
    }
    let mut i = 0;
    while i < modules.len() {
        n += modules[i].champs_journal.len();
        i += 1;
    }
    n
}

/// Concatène les listes à la compilation ; chaque case est réécrite.
const fn assembler<const N: usize>(
    hors_routes: &[&[&'static str]],
    modules: &[Module],
) -> [&'static str; N] {
    let mut table = [""; N];
    let mut n = 0;
    let mut i = 0;
    while i < hors_routes.len() {
        let mut j = 0;
        while j < hors_routes[i].len() {
            table[n] = hors_routes[i][j];
            n += 1;
            j += 1;
        }
        i += 1;
    }
    let mut i = 0;
    while i < modules.len() {
        let mut j = 0;
        while j < modules[i].champs_journal.len() {
            table[n] = modules[i].champs_journal[j];
            n += 1;
            j += 1;
        }
        i += 1;
    }
    table
}

/// En-tête de réponse qui porte l'identifiant de la requête.
pub const EN_TETE_REQUETE: HeaderName = HeaderName::from_static("x-sigma-requete");

/// Filtre des journaux : la cible [`CIBLE`] **exactement** (pas `sigma_…` ni `sigma::…`, qu'une
/// dépendance pourrait utiliser), au niveau `INFO` ou plus grave ; rien d'autre.
#[must_use]
pub fn filtre() -> FilterFn<fn(&Metadata<'_>) -> bool> {
    FilterFn::new(admis)
}

fn admis(metadonnees: &Metadata<'_>) -> bool {
    metadonnees.target() == CIBLE && *metadonnees.level() <= Level::INFO
}

/// Modèle de route d'une requête statique (fichiers de l'interface).
#[derive(Debug, Clone, Copy)]
pub(crate) struct RouteStatique;

fn nom_methode(methode: &Method) -> &'static str {
    match *methode {
        Method::GET => "GET",
        Method::HEAD => "HEAD",
        Method::POST => "POST",
        Method::PUT => "PUT",
        Method::PATCH => "PATCH",
        Method::DELETE => "DELETE",
        Method::OPTIONS => "OPTIONS",
        _ => "autre",
    }
}

/// Intergiciel le plus externe : identifiant de requête, durée, ligne de journal.
pub async fn journaliser(requete: Request, suivant: Next) -> Response {
    let debut = Instant::now();
    let methode = nom_methode(requete.method());
    let id = Uuid::new_v4().hyphenated().to_string();
    let mut reponse = suivant.run(requete).await;
    // Le modèle de route est connu après le routage : le routeur le recopie dans la réponse.
    let route = if let Some(modele) = reponse.extensions().get::<MatchedPath>() {
        modele.as_str().to_owned()
    } else if reponse.extensions().get::<RouteStatique>().is_some() {
        "statique".to_owned()
    } else {
        "aucune".to_owned()
    };
    let erreur = reponse
        .extensions()
        .get::<CategorieErreur>()
        .map(|c| c.nom());
    let duree_ms = u64::try_from(debut.elapsed().as_millis()).unwrap_or(u64::MAX);
    tracing::info!(
        target: CIBLE,
        methode,
        route = route.as_str(),
        statut = reponse.status().as_u16(),
        duree_ms,
        requete = id.as_str(),
        erreur,
        "requête traitée"
    );
    if let Ok(valeur) = HeaderValue::from_str(&id) {
        reponse.headers_mut().insert(EN_TETE_REQUETE, valeur);
    }
    reponse
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn la_liste_blanche_est_l_assemblage_des_declarations() {
        let attendus: Vec<&str> = HORS_ROUTES
            .iter()
            .flat_map(|l| l.iter())
            .chain(MODULES.iter().flat_map(|m| m.champs_journal))
            .copied()
            .collect();
        assert_eq!(CHAMPS, attendus.as_slice());
        let mut vus = std::collections::BTreeSet::new();
        for champ in CHAMPS {
            assert_ne!(*champ, "");
            assert!(vus.insert(champ), "champ {champ} déclaré deux fois");
        }
        for champ in ["autorite", "issue"] {
            assert!(CHAMPS.contains(&champ), "{champ}");
        }
    }

    #[test]
    fn seule_la_cible_exacte_passe_le_filtre() {
        use tracing::callsite::Identifier;
        use tracing::field::FieldSet;
        use tracing::metadata::Kind;

        struct Site;
        impl tracing::Callsite for Site {
            fn set_interest(&self, _: tracing::subscriber::Interest) {}
            fn metadata(&self) -> &Metadata<'_> {
                unreachable!()
            }
        }
        static SITE: Site = Site;
        let meta = |cible: &'static str, niveau: Level| {
            Metadata::new(
                "e",
                cible,
                niveau,
                None,
                None,
                None,
                FieldSet::new(&[], Identifier(&SITE)),
                Kind::EVENT,
            )
        };
        assert!(admis(&meta("sigma", Level::INFO)));
        assert!(admis(&meta("sigma", Level::WARN)));
        assert!(!admis(&meta("sigma", Level::DEBUG)));
        for cible in ["sigma::requete", "sigma_tiers", "sqlx::query", "api"] {
            assert!(!admis(&meta(cible, Level::INFO)), "{cible}");
        }
    }
}
