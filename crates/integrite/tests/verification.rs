//! Vérification d'un export : dossier réel, altérations, réécritures (menaces F2.3, F7.1).

#![allow(clippy::unwrap_used)]

mod commun;

use std::fmt::Write as _;

use commun::{dossier_id, export_de_reference};
use integrite::{
    Anomalie, Element, Empreinte, IdDossier, LecteurExport, Options, Rapport, chainer, genese,
    verifier,
};
use proptest::prelude::*;

fn verifier_octets(octets: &[u8], options: &Options) -> Rapport {
    verifier(octets, options).unwrap()
}

/// Contenus canoniques d'un export.
fn contenus(octets: &[u8]) -> Vec<String> {
    let (mut lecteur, _) = LecteurExport::ouvrir(octets).unwrap();
    let mut sortie = Vec::new();
    while let Some(element) = lecteur.suivant().unwrap() {
        if let Element::Ecriture { contenu, .. } = element {
            sortie.push(contenu);
        }
    }
    sortie
}

/// Export « forgé » : des contenus quelconques, avec des empreintes, une tête et un nombre
/// recalculés. Simule un attaquant qui maîtrise tout l'export.
fn forger(dossier: &IdDossier, contenus: &[String]) -> Vec<u8> {
    let mut tete = genese(dossier);
    let mut s = format!("sigma-books export v3\ndossier={dossier}\n");
    for c in contenus {
        tete = chainer(&tete, c.as_bytes());
        s.push_str(c);
        writeln!(s, "empreinte={tete}").unwrap();
    }
    write!(s, "tete={tete}\nnombre={}\nfin-export\n", contenus.len()).unwrap();
    s.into_bytes()
}

#[test]
fn export_integre() {
    let export = export_de_reference();
    let rapport = verifier_octets(&export, &Options::default());
    assert!(rapport.est_integre(), "{rapport}");
    assert_eq!(rapport.nombre, 6);
    let (premiere, derniere) = rapport.bornes.clone().unwrap();
    assert_eq!(premiere.to_string(), "VT-2026-000001");
    assert_eq!(derniere.to_string(), "OD-2026-000002");
    let plages: Vec<String> = rapport
        .sequences
        .iter()
        .map(|((j, e), p)| format!("{j} {e} {}-{}", p.premier, p.dernier))
        .collect();
    assert_eq!(
        plages,
        ["BQ1 2026 1-1", "OD 2026 1-2", "VT 2026 1-2", "VT 2027 1-1"]
    );
    let texte = rapport.to_string();
    assert!(texte.contains("RÉSULTAT : INTÈGRE"), "{texte}");
    assert!(texte.contains("sans référence externe"), "{texte}");
}

#[test]
fn chaque_octet_modifie_est_detecte() {
    let export = export_de_reference();
    for position in 0..export.len() {
        for masque in [0x01u8, 0x20, 0x80] {
            let mut altere = export.clone();
            altere[position] ^= masque;
            let rapport = verifier_octets(&altere, &Options::default());
            assert!(
                !rapport.est_integre(),
                "octet {position} ^ {masque:#04x} non détecté"
            );
        }
    }
}

proptest! {
    #[test]
    fn toute_substitution_est_detectee(position in any::<prop::sample::Index>(), octet in any::<u8>()) {
        let export = export_de_reference();
        let i = position.index(export.len());
        prop_assume!(export[i] != octet);
        let mut altere = export;
        altere[i] = octet;
        prop_assert!(!verifier_octets(&altere, &Options::default()).est_integre());
    }

    #[test]
    fn toute_troncature_est_detectee(longueur in 0usize..1500) {
        let export = export_de_reference();
        prop_assume!(longueur < export.len());
        prop_assert!(!verifier_octets(&export[..longueur], &Options::default()).est_integre());
    }
}

#[test]
fn ecriture_alteree_localisee() {
    let export = String::from_utf8(export_de_reference()).unwrap();
    let altere = export.replacen(
        "libelle=Écriture d'ajustement",
        "libelle=Écriture d'ajustemant",
        1,
    );
    let rapport = verifier_octets(altere.as_bytes(), &Options::default());
    assert!(
        matches!(
            rapport.anomalies.as_slice(),
            [Anomalie::EmpreinteDifferente { numero, .. }, Anomalie::TeteDifferente { .. }]
                if numero.to_string() == "OD-2026-000001"
        ),
        "{rapport}"
    );
}

#[test]
fn suppression_reforgee_detectee_par_la_numerotation() {
    let mut c = contenus(&export_de_reference());
    c.remove(0); // VT-2026-000001 disparaît, VT-2026-000002 la contre-passe.
    let rapport = verifier_octets(&forger(&dossier_id(), &c), &Options::default());
    assert!(
        rapport.anomalies.iter().any(|a| matches!(
            a,
            Anomalie::NumerotationDiscontinue { attendu: 1, numero, .. }
                if numero.to_string() == "VT-2026-000002"
        )),
        "{rapport}"
    );
    assert!(
        rapport
            .anomalies
            .iter()
            .any(|a| matches!(a, Anomalie::OrigineInconnue { .. })),
        "{rapport}"
    );
}

#[test]
fn inversion_reforgee_detectee() {
    let mut c = contenus(&export_de_reference());
    c.swap(0, 3);
    let rapport = verifier_octets(&forger(&dossier_id(), &c), &Options::default());
    assert!(!rapport.est_integre(), "{rapport}");
}

#[test]
fn reecriture_complete_detectee_par_l_ancrage() {
    let export = export_de_reference();
    let tete = verifier_octets(&export, &Options::default()).tete.unwrap();
    // Réécriture cohérente : libellé modifié, chaîne entièrement recalculée.
    let c: Vec<String> = contenus(&export)
        .into_iter()
        .map(|c| c.replace("Facture 2027-001", "Facture 2027-002"))
        .collect();
    let forge = forger(&dossier_id(), &c);
    let sans_ancrage = verifier_octets(&forge, &Options::default());
    assert!(sans_ancrage.est_integre(), "limite connue : {sans_ancrage}");
    let ancre = Options {
        tete: Some(tete),
        ..Options::default()
    };
    let rapport = verifier_octets(&forge, &ancre);
    assert_eq!(
        rapport.anomalies,
        [Anomalie::AncrageDifferent { attendue: tete }]
    );
    let intact = verifier_octets(&export, &ancre);
    assert!(intact.est_integre() && intact.ancre);
    assert!(!intact.to_string().contains("sans référence externe"));
}

#[test]
fn greffe_sur_un_autre_dossier_detectee() {
    let export = String::from_utf8(export_de_reference()).unwrap();
    let autre = export.replacen("dossier=dossier-test", "dossier=autre", 1);
    let rapport = verifier_octets(autre.as_bytes(), &Options::default());
    assert!(matches!(
        rapport.anomalies.first(),
        Some(Anomalie::EmpreinteDifferente { .. })
    ));
    let attendu = Options {
        dossier: Some(IdDossier::parse("autre").unwrap()),
        ..Options::default()
    };
    let rapport = verifier_octets(&export_de_reference(), &attendu);
    assert!(matches!(
        rapport.anomalies.as_slice(),
        [Anomalie::DossierInattendu { .. }]
    ));
}

#[test]
fn ecriture_desequilibree_detectee() {
    let mut c = contenus(&export_de_reference());
    c[2] = c[2].replacen("montant=2500\n", "montant=2600\n", 1);
    let rapport = verifier_octets(&forger(&dossier_id(), &c), &Options::default());
    assert!(
        matches!(
            rapport.anomalies.as_slice(),
            [Anomalie::Desequilibre { .. }]
        ),
        "{rapport}"
    );
}

#[test]
fn rapport_borne_le_nombre_d_anomalies() {
    let c = contenus(&export_de_reference());
    let mut tous = Vec::new();
    for _ in 0..60 {
        tous.extend(c.iter().cloned());
    }
    let rapport = verifier_octets(&forger(&dossier_id(), &tous), &Options::default());
    assert_eq!(
        rapport.anomalies.len(),
        integrite::verification::ANOMALIES_DETAILLEES
    );
    assert!(rapport.anomalies_omises > 0);
    assert!(rapport.to_string().contains("autre(s)"));
}

#[test]
fn export_vide_et_tete() {
    let forge = forger(&dossier_id(), &[]);
    let rapport = verifier_octets(&forge, &Options::default());
    assert!(rapport.est_integre());
    assert_eq!(rapport.tete, Some(genese(&dossier_id())));
    assert!(rapport.bornes.is_none());
    let _: Empreinte = rapport.tete.unwrap();
}

/// Blocs `jeton … fin-jeton` de l'export horodaté de référence.
fn blocs_de_jetons() -> String {
    let export =
        String::from_utf8(commun::export_avec_jetons(&commun::jetons_de_reference())).unwrap();
    let debut = export.find("jeton\n").unwrap();
    let fin = export.rfind("fin-export\n").unwrap();
    export[debut..fin].to_owned()
}

/// Menace F2.4 : réécriture complète avec chaîne recalculée, jetons d'origine conservés. Le
/// jeton qui couvre l'écriture modifiée ne correspond plus ; celui qui la précède reste valable.
#[test]
fn reecriture_complete_detectee_par_les_jetons() {
    let export = export_de_reference();
    let c: Vec<String> = contenus(&export)
        .into_iter()
        .map(|c| c.replace("Facture 2027-001", "Facture 2027-002"))
        .collect();
    let forge = String::from_utf8(forger(&dossier_id(), &c))
        .unwrap()
        .replace(
            "fin-export\n",
            &format!("{}fin-export\n", blocs_de_jetons()),
        );
    let rapport = verifier_octets(forge.as_bytes(), &Options::default());
    // Les jetons sur 5 et 6 écritures couvrent l'écriture modifiée : ils ne correspondent plus.
    assert!(
        matches!(
            rapport.anomalies.as_slice(),
            [
                Anomalie::TemoinDifferent { nombre: 5, .. },
                Anomalie::TemoinDifferent { nombre: 6, .. }
            ]
        ),
        "{rapport}"
    );
    // Les 3 premières écritures, intactes, restent couvertes par le jeton Sectigo.
    assert_eq!(rapport.horodatages.len(), 1);
    assert_eq!(rapport.horodatages[0].nombre, 3);
}

#[test]
fn jeton_altere_ou_deplace_detecte() {
    let export =
        String::from_utf8(commun::export_avec_jetons(&commun::jetons_de_reference())).unwrap();
    // Un caractère base64 modifié au cœur du jeton DigiCert (le base64 reste valide).
    let position = export.rfind("der=").unwrap() - 300;
    let mut altere = export.clone().into_bytes();
    altere[position] = if altere[position] == b'A' { b'B' } else { b'A' };
    let rapport = verifier_octets(&altere, &Options::default());
    assert!(
        matches!(
            rapport.anomalies.as_slice(),
            [Anomalie::JetonInvalide { .. } | Anomalie::Illisible { .. }]
        ),
        "{rapport}"
    );
    // Jeton rattaché à un autre état de la chaîne.
    let deplace = export.replacen("jeton\nnombre=3\n", "jeton\nnombre=4\n", 1);
    let rapport = verifier_octets(deplace.as_bytes(), &Options::default());
    assert!(
        matches!(
            rapport.anomalies.as_slice(),
            [Anomalie::TemoinDifferent { nombre: 4, .. }]
        ),
        "{rapport}"
    );
    // Témoin au-delà de la chaîne.
    let au_dela = export.replacen("jeton\nnombre=5\n", "jeton\nnombre=6\n", 1);
    let rapport = verifier_octets(au_dela.as_bytes(), &Options::default());
    assert!(matches!(
        rapport.anomalies.as_slice(),
        [Anomalie::TemoinDifferent { nombre: 6, .. }]
    ));
}

#[test]
fn jeton_d_un_autre_dossier_refuse() {
    // Même chaîne, autre dossier : la genèse change, donc les têtes aussi ; le jeton ne peut
    // pas être réutilisé.
    let export =
        String::from_utf8(commun::export_avec_jetons(&commun::jetons_de_reference())).unwrap();
    let autre = export.replacen("dossier=dossier-test", "dossier=autre", 1);
    let rapport = verifier_octets(autre.as_bytes(), &Options::default());
    assert!(rapport.horodatages.is_empty(), "{rapport}");
    assert!(
        rapport
            .anomalies
            .iter()
            .any(|a| matches!(a, Anomalie::TemoinDifferent { .. }))
    );
}
