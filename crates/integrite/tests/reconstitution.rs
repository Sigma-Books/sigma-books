//! Une écriture relue de la base et reconstituée ([`EcritureValidee::reconstituer`]) a le même
//! contenu canonique que l'écriture validée d'origine : sa place dans la chaîne de hachage
//! est inchangée (lot L2, « Tests exigés » ; ADR 0007).

// Fichier de test : les `unwrap` y sont voulus (une donnée de test invalide fait échouer le test).
#![allow(clippy::unwrap_used)]

mod commun;

use commun::dossier_de_reference;
use compta_core::{EcritureBrouillon, EcritureValidee};
use integrite::EcritureCanonique;

/// Reconstitue une écriture à partir de ses seuls champs, comme l'API la relira de la base.
fn relire(e: &EcritureValidee) -> EcritureValidee {
    let mut b = EcritureBrouillon::nouvelle(e.journal().clone(), e.date(), e.libelle().clone());
    for l in e.lignes() {
        b.ajouter_ligne(l.clone());
    }
    b.justifier(e.justification().clone());
    if let Some(version) = e.version_parametres() {
        b.utiliser_parametres(version.clone());
    }
    b.mentionner_date_valeur_comptable(e.date_valeur_comptable());
    EcritureValidee::reconstituer(
        b.valider_equilibre().unwrap(),
        e.numero().clone(),
        e.date_validation(),
        e.origine().cloned(),
    )
    .unwrap()
}

/// Dossier de référence : devise, tiers, pièces multiples, version de paramètres,
/// contre-passation, opération tardive (format v2).
#[test]
fn contenu_canonique_identique_apres_reconstitution() {
    let d = dossier_de_reference();
    assert!(d.ecritures().len() >= 6);
    for e in d.ecritures() {
        let relue = relire(e);
        assert_eq!(&relue, e, "{}", e.numero());
        assert_eq!(
            EcritureCanonique::depuis(&relue).serialiser(),
            EcritureCanonique::depuis(e).serialiser(),
            "{}",
            e.numero()
        );
    }
}
