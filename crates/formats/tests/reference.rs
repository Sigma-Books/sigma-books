//! Fichiers de référence (« golden files ») des relevés, tous **synthétiques** (banques,
//! clients, références et identifiants inventés ; aucun IBAN). Pour chaque `NOM.csv` lu avec
//! `NOM.toml` :
//! - l'analyse rend exactement `NOM.attendu` (lignes, lignes brutes, alertes, empreintes) ;
//! - l'écriture des mouvements selon le profil rend, octet pour octet, `NOM.ecrit.csv` ;
//! - l'aller-retour `analyser(serialiser(mouvements)) == mouvements` tient.
//!
//! `SIGMA_BENIR=1 cargo test -p formats --test reference` réécrit `NOM.attendu` et
//! `NOM.ecrit.csv` (à relire avant tout commit).

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use formats::releve::{Limites, ProfilReleve, Releve, analyser, serialiser};

const LIMITES: Limites = Limites {
    taille_fichier: 5 * 1024 * 1024,
    lignes: 20_000,
    colonnes: 50,
    longueur_ligne: 64 * 1024,
    longueur_cellule: 1_000,
};

const NOMS: [&str; 4] = [
    "banque-utf8-bom",
    "banque-windows-1252",
    "carte-signe-inverse",
    "mobile-money",
];

fn chemin(nom: &str, extension: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/releves")
        .join(format!("{nom}.{extension}"))
}

fn option<T: ToString>(v: Option<&T>) -> String {
    v.map_or_else(|| "-".to_owned(), ToString::to_string)
}

fn rendu(releve: &Releve) -> String {
    let mut s = String::new();
    let _ = writeln!(s, "fichier {}", releve.empreinte());
    let empreintes = releve.empreintes_rangees("compte-fictif-1");
    for (l, e) in releve.lignes().iter().zip(&empreintes) {
        let m = l.mouvement();
        let _ = writeln!(s, "ligne {}", l.ligne());
        let _ = writeln!(s, "  brut {:?}", l.brut());
        let _ = writeln!(s, "  date {}", m.date);
        let _ = writeln!(s, "  date_valeur {}", option(m.date_valeur.as_ref()));
        let _ = writeln!(s, "  libelle {:?}", m.libelle);
        let _ = writeln!(s, "  reference {}", option(m.reference.as_ref()));
        let _ = writeln!(s, "  montant {}", m.montant);
        let _ = writeln!(s, "  solde {}", option(m.solde.as_ref()));
        let _ = writeln!(s, "  devise {}", option(m.devise.as_ref()));
        let _ = writeln!(
            s,
            "  identifiant {}",
            option(m.identifiant_transaction.as_ref())
        );
        let _ = writeln!(s, "  empreinte {} rang {}", e.empreinte, e.rang);
    }
    for a in releve.alertes() {
        let _ = writeln!(s, "alerte ligne {} {}", a.ligne, a.nature.code());
    }
    s
}

fn comparer(chemin: &Path, obtenu: &[u8]) {
    if std::env::var_os("SIGMA_BENIR").is_some() {
        std::fs::write(chemin, obtenu).unwrap_or_else(|e| panic!("{}: {e}", chemin.display()));
        return;
    }
    let attendu = std::fs::read(chemin).unwrap_or_else(|e| panic!("{}: {e}", chemin.display()));
    assert!(
        attendu == obtenu,
        "{} diffère :\n{}",
        chemin.display(),
        String::from_utf8_lossy(obtenu)
    );
}

#[test]
fn fichiers_de_reference() {
    for nom in NOMS {
        let profil =
            ProfilReleve::depuis_toml(&std::fs::read_to_string(chemin(nom, "toml")).unwrap())
                .unwrap();
        let octets = std::fs::read(chemin(nom, "csv")).unwrap();
        let releve = analyser(&octets, &profil, &LIMITES).unwrap();
        comparer(&chemin(nom, "attendu"), rendu(&releve).as_bytes());

        let mouvements: Vec<_> = releve.mouvements().cloned().collect();
        let ecrit = serialiser(&mouvements, &profil).unwrap();
        comparer(&chemin(nom, "ecrit.csv"), &ecrit);

        let relu = analyser(&ecrit, &profil, &LIMITES).unwrap();
        assert_eq!(
            relu.mouvements().cloned().collect::<Vec<_>>(),
            mouvements,
            "{nom}"
        );
    }
}
