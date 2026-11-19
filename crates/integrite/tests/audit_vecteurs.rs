//! Vecteurs figés du format **audit v1** (`tests/vecteurs/audit-v1/`, `docs/integrite/audit-v1.md`).
//! Un échec ici signale un changement de format : il faut créer une nouvelle version, jamais
//! modifier ces vecteurs (`crates/integrite/CLAUDE.md`).
//!
//! Les vecteurs et les empreintes ci-dessous ont été produits **indépendamment** du code Rust
//! (Python, `hashlib`), à partir de `docs/integrite/audit-v1.md` :
//! - `entree-cabinet.txt` : première entrée de la chaîne d'un cabinet (création d'un dossier) ;
//! - `export-dossier.txt` : export de la chaîne d'un dossier de trois entrées (création, tâche
//!   `systeme` avec motif, export d'audit qui cite la tête et le nombre exportés), avec son
//!   annexe d'acteurs (un nom connu, un nom absent).

#![allow(clippy::unwrap_used)]

use std::collections::BTreeMap;

use integrite::Empreinte;
use integrite::audit::{
    Acteur, AnomalieAudit, AttentesAudit, ChaineAudit, EcrivainExportAudit, EntreeAudit, Portee,
    Texte, Uuid, genese, verifier_export_audit,
};

const ENTREE_CABINET: &str = include_str!("vecteurs/audit-v1/entree-cabinet.txt");
const EXPORT_DOSSIER: &str = include_str!("vecteurs/audit-v1/export-dossier.txt");

const CABINET: &str = "0193e0c2-7d1a-4c4e-9f00-1a2b3c4d5e6f";
const DOSSIER: &str = "0193e0c3-1111-4a2b-8c3d-000000000001";
const GENESE_CABINET: &str = "e70a220d436e4b916a792475764038b59b0841fbd87c2a55c43a7b7268a17be1";
const EMPREINTE_CABINET_1: &str =
    "f9b4a3ba98a2a51caaec1fba7a5f7285f22a7411833d6d18c6c7c98241e082d9";
const GENESE_DOSSIER: &str = "ae40da4dc7a1266cde2ee6bf4b9e733bfbc0ae7c1440dc23bf23938c85660c01";
const TETE_DOSSIER: &str = "383591bb3c62f7f173bb7d0240c89aa9e336de54ae972721263740f3a1434509";

fn portee_cabinet() -> Portee {
    Portee::Cabinet {
        cabinet: Uuid::parse(CABINET).unwrap(),
    }
}

fn portee_dossier() -> Portee {
    Portee::Dossier {
        cabinet: Uuid::parse(CABINET).unwrap(),
        dossier: Uuid::parse(DOSSIER).unwrap(),
    }
}

#[test]
fn empreintes_de_depart_figees() {
    assert_eq!(genese(&portee_cabinet()).to_string(), GENESE_CABINET);
    assert_eq!(genese(&portee_dossier()).to_string(), GENESE_DOSSIER);
}

#[test]
fn entree_du_cabinet_relue_et_chainee_a_l_identique() {
    let entree = EntreeAudit::lire(ENTREE_CABINET).unwrap();
    assert_eq!(entree.serialiser(), ENTREE_CABINET);
    assert_eq!(*entree.portee(), portee_cabinet());
    let mut chaine = ChaineAudit::ouvrir(portee_cabinet());
    let (contenu, empreinte) = chaine.ajouter(&entree).unwrap();
    assert_eq!(contenu, ENTREE_CABINET);
    assert_eq!(empreinte.to_string(), EMPREINTE_CABINET_1);
}

/// Les entrées d'un export, telles qu'écrites.
fn entrees(export: &str) -> Vec<(String, Empreinte)> {
    let mut sortie = Vec::new();
    let mut reste = export;
    // L'en-tête « sigma-books export-audit v1 » contient lui aussi « audit v1 » : une entrée
    // commence en début de ligne.
    while let Some(debut) = reste.find("\naudit v1\n").map(|i| i + 1) {
        let fin = reste[debut..].find("\nfin\n").unwrap() + debut + 5;
        let contenu = reste[debut..fin].to_owned();
        let ligne = reste[fin..].lines().next().unwrap();
        let empreinte = Empreinte::parse(ligne.strip_prefix("empreinte=").unwrap()).unwrap();
        sortie.push((contenu, empreinte));
        reste = &reste[fin - 1..];
    }
    sortie
}

#[test]
fn export_du_dossier_verifie_et_regenere_a_l_identique() {
    let tete = Empreinte::parse(TETE_DOSSIER).unwrap();
    let rapport = verifier_export_audit(
        EXPORT_DOSSIER.as_bytes(),
        &AttentesAudit {
            tete: Some(tete),
            cabinet: Some(Uuid::parse(CABINET).unwrap()),
            dossier: Some(Uuid::parse(DOSSIER).unwrap()),
        },
    )
    .unwrap();
    assert!(rapport.est_integre(), "{rapport}");
    assert_eq!(rapport.chaine.nombre, 3);
    assert_eq!(rapport.chaine.tete, tete);

    // Le code Rust recalcule les mêmes empreintes et réécrit le même export, octet pour octet.
    let mut chaine = ChaineAudit::ouvrir(portee_dossier());
    let mut ecrivain = EcrivainExportAudit::nouveau(portee_dossier(), Vec::new()).unwrap();
    let mut noms = BTreeMap::new();
    for (contenu, empreinte) in entrees(EXPORT_DOSSIER) {
        let entree = EntreeAudit::lire(&contenu).unwrap();
        assert_eq!(
            chaine.ajouter(&entree).unwrap(),
            (contenu.clone(), empreinte)
        );
        ecrivain.entree(&contenu, &empreinte).unwrap();
        if let Acteur::Utilisateur { id, role, .. } = entree.evenement().acteur()
            && role.as_str() == "administrateur"
        {
            noms.insert(*id, Texte::parse("Awa Ndiaye").unwrap());
        }
    }
    assert_eq!(*chaine.tete(), tete);
    let (sortie, nombre) = ecrivain.terminer(&tete, &noms).unwrap();
    assert_eq!(nombre, 3);
    assert_eq!(String::from_utf8(sortie).unwrap(), EXPORT_DOSSIER);
}

#[test]
fn un_octet_modifie_dans_le_vecteur_est_detecte() {
    for (avant, apres) in [
        ("apres=horodaté", "apres=horodate"),
        ("rang=2\n", "rang=3\n"),
        ("reauthentification=oui", "reauthentification=non"),
    ] {
        let modifie = EXPORT_DOSSIER.replacen(avant, apres, 1);
        let r = verifier_export_audit(modifie.as_bytes(), &AttentesAudit::default()).unwrap();
        assert!(!r.est_integre(), "{avant}");
        assert!(
            r.chaine
                .anomalies
                .iter()
                .any(|a| matches!(a, AnomalieAudit::EmpreinteAlteree { .. })),
            "{avant} : {r}"
        );
    }
}
