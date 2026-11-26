//! Vecteur figé de l'export v5 (devise de tenue, ADR 0027 ; `docs/integrite/format-v5.md`).
//! Un échec ici signale un changement de format : il faut créer une nouvelle version, jamais
//! modifier ce vecteur (`crates/integrite/CLAUDE.md`).
//!
//! `export-v5-reference.txt` : dossier tenu en CDF (deux décimales), trois écritures au format
//! canonique v4 : vente taxée avec centimes et étiquettes fiscales, achat en EUR dont la
//! contre-valeur est arrondie au centime, contre-passation de la vente. Produit
//! **indépendamment** du moteur (Python, `hashlib`) à partir de la documentation du format, puis
//! comparé à ce que produit le moteur.
//!
//! Le vecteur se recalcule avec `tests/vecteurs/generer-export-v5.py` (Python, `hashlib` seul,
//! écrit à partir de `docs/integrite/format-v5.md`).
//!
//! Les vecteurs v1 à v4 (dossier XOF) restent vérifiés par leurs propres tests : leurs
//! empreintes n'ont pas changé.

#![allow(clippy::unwrap_used)]

mod commun;

use integrite::{Anomalie, ErreurCanonique, ErreurExport, Options, Version, verifier};

const REFERENCE: &[u8] = include_bytes!("vecteurs/export-v5-reference.txt");
const TETE: &str = "3f4eb22f9321573164686b7ba8fb9afb7d5d065b43691662c6f4f666ce52756d";

fn texte() -> String {
    String::from_utf8(REFERENCE.to_vec()).unwrap()
}

#[test]
fn le_moteur_regenere_le_vecteur() {
    assert_eq!(
        String::from_utf8(commun::export_de_reference_v5()).unwrap(),
        texte()
    );
}

#[test]
fn vecteur_v5_integre() {
    let rapport = verifier(REFERENCE, &Options::default()).unwrap();
    assert!(rapport.est_integre(), "{rapport}");
    assert_eq!(rapport.version, Some(Version::V5));
    assert_eq!(rapport.nombre, 3);
    assert_eq!(rapport.tete.unwrap().to_string(), TETE);
}

#[test]
fn la_devise_de_tenue_est_dans_le_contenu_hache() {
    // Même export retouché en XAF (devise et centimes), sans recalcul de la chaîne : altération
    // localisée, jamais acceptée.
    let altere = texte().replace("devise-tenue=CDF", "devise-tenue=XAF");
    let rapport = verifier(altere.as_bytes(), &Options::default()).unwrap();
    assert!(!rapport.est_integre());
    assert!(
        rapport
            .anomalies
            .iter()
            .any(|a| matches!(a, Anomalie::EmpreinteDifferente { .. })),
        "{rapport}"
    );
}

#[test]
fn un_export_v4_n_admet_pas_l_ecriture_v4() {
    let ancien = texte().replacen("sigma-books export v5", "sigma-books export v4", 1);
    let rapport = verifier(ancien.as_bytes(), &Options::default()).unwrap();
    assert!(!rapport.est_integre());
    assert!(
        rapport
            .anomalies
            .iter()
            .any(|a| matches!(a, Anomalie::Illisible { .. })),
        "{rapport}"
    );
}

#[test]
fn echelle_et_contre_valeur_controlees_par_le_verificateur() {
    // La forme est contrôlée avant la chaîne : l'écriture retouchée est illisible.
    for (avant, apres, attendu) in [
        // Trois décimales pour une tenue à deux : refusé à la lecture.
        (
            "montant=118000.55",
            "montant=118000.555",
            ErreurCanonique::ValeurInvalide {
                ligne: 12,
                cle: "montant",
            },
        ),
        // Contre-valeur incohérente avec 100,55 EUR au cours de 2 827,3333.
        (
            "montant=284288.36\ndevise=EUR\nmontant-devise=100.55\ncours=2827.3333\nligne",
            "montant=284288.37\ndevise=EUR\nmontant-devise=100.55\ncours=2827.3333\nligne",
            ErreurCanonique::ContreValeurIncoherente { ligne: 12 },
        ),
    ] {
        let t = texte().replacen(avant, apres, 1);
        assert_ne!(t, texte(), "{avant}");
        let rapport = verifier(t.as_bytes(), &Options::default()).unwrap();
        assert!(!rapport.est_integre(), "{avant}");
        let erreur = rapport.anomalies.iter().find_map(|a| match a {
            Anomalie::Illisible { erreur, .. } => Some(erreur.clone()),
            _ => None,
        });
        assert!(
            matches!(erreur, Some(ErreurExport::Ecriture(e)) if e == attendu),
            "{avant} : {:?}",
            rapport.anomalies
        );
    }
}

/// Écritures (texte canonique, `fin\n` compris) d'un export, dans l'ordre.
fn ecritures_de(export: &str) -> Vec<String> {
    let mut sortie = Vec::new();
    let mut courante = String::new();
    for l in export.split_inclusive('\n') {
        if l.starts_with("ecriture v") {
            courante.clear();
        }
        if !courante.is_empty() || l.starts_with("ecriture v") {
            courante.push_str(l);
        }
        if l == "fin\n" {
            sortie.push(std::mem::take(&mut courante));
        }
    }
    sortie
}

/// La devise de tenue est celle du dossier et ne change pas : un export qui mêle une écriture en
/// CDF (v4) et une écriture en XOF (v1), pourtant chaîné sans défaut, est signalé.
#[test]
fn une_devise_de_tenue_qui_change_est_signalee() {
    use integrite::{IdDossier, chainer, genese};
    use std::fmt::Write as _;
    let cdf = ecritures_de(&texte());
    let xof = ecritures_de(
        std::str::from_utf8(include_bytes!("vecteurs/export-v4-reference.txt")).unwrap(),
    );
    // BQ1-2026-000001 : première écriture XOF du journal BQ1 (aucune contre-passation).
    let bq1 = xof
        .iter()
        .find(|e| e.contains("numero=BQ1-2026-000001\n"))
        .unwrap();
    let dossier = IdDossier::parse("dossier-cdf").unwrap();
    let mut export = format!("sigma-books export v5\ndossier={dossier}\n");
    let mut tete = genese(&dossier);
    for e in [&cdf[0], bq1] {
        tete = chainer(&tete, e.as_bytes());
        export.push_str(e);
        writeln!(export, "empreinte={tete}").unwrap();
    }
    write!(export, "tete={tete}\nnombre=2\nfin-export\n").unwrap();
    let rapport = verifier(export.as_bytes(), &Options::default()).unwrap();
    assert!(!rapport.est_integre());
    // Seule l'anomalie de devise : la chaîne, la numérotation et l'équilibre sont sains.
    assert_eq!(rapport.anomalies.len(), 1, "{rapport}");
    assert!(
        matches!(
            &rapport.anomalies[0],
            Anomalie::TenueChangeante { attendue, trouvee, .. }
                if attendue.code() == "CDF" && trouvee.code() == "XOF"
        ),
        "{rapport}"
    );
    assert!(
        rapport
            .to_string()
            .contains("la devise de tenue d'un dossier ne change pas")
    );
}
