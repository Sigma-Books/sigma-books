//! Analyseur de relevés CSV sur des octets quelconques × un profil engendré (menace F11.4) :
//! aucune panique, bornes respectées, et un relevé accepté se réécrit puis se relit à
//! l'identique quand il est représentable.
//!
//! Entrée : les trois premiers octets choisissent le profil (encodage, délimiteur, guillemet,
//! en-tête, pied, format de date, séparateurs, signe, colonnes) ; le reste est le fichier.

#![no_main]
#![forbid(unsafe_code)]

use formats::releve::{
    Colonne, ColonnesBrutes, Limites, ProfilBrut, ProfilReleve, analyser, serialiser,
};
use libfuzzer_sys::fuzz_target;

const LIMITES: Limites = Limites {
    taille_fichier: 64 * 1024,
    lignes: 1_000,
    colonnes: 50,
    longueur_ligne: 4_096,
    longueur_cellule: 256,
};

const FORMATS_DATE: [&str; 6] = [
    "%d/%m/%Y",
    "%Y-%m-%d",
    "%d.%m.%y",
    "%d%m%Y",
    "%Y-%m-%d %H:%M:%S",
    "%m/%d/%Y",
];

fn profil(a: u8, b: u8, c: u8) -> Option<ProfilReleve> {
    let encodage = ["utf-8", "windows-1252", "iso-8859-15"][usize::from(a % 3)];
    let delimiteur = [",", ";", "\t", "|"][usize::from(a / 3 % 4)];
    let guillemet = ["\"", "'", ""][usize::from(a / 12 % 3)];
    let lignes_entete = usize::from(b % 3);
    let lignes_pied = usize::from(b / 3 % 3);
    let format_date = FORMATS_DATE[usize::from(b / 9 % 6)];
    let (decimal, milliers) = [
        (".", None),
        (",", Some(" ")),
        (",", Some(".")),
        (".", Some("'")),
    ][usize::from(a / 36 % 4)];
    let titres = lignes_entete > 0 && c & 0x80 != 0;
    let mut rang = 0;
    let mut col = |present: bool, titre: &str| {
        present.then(|| {
            rang += 1;
            if titres {
                Colonne::Titre(titre.to_owned())
            } else {
                Colonne::Rang(rang - 1)
            }
        })
    };
    let debit_credit = c & 1 != 0;
    let colonnes = ColonnesBrutes {
        date: col(true, "Date")?,
        date_valeur: col(c & 2 != 0, "Valeur"),
        libelle: col(c & 4 != 0, "Libelle"),
        reference: col(c & 8 != 0, "Ref"),
        montant: col(!debit_credit, "Montant"),
        debit: col(debit_credit, "Debit"),
        credit: col(debit_credit, "Credit"),
        solde: col(c & 16 != 0, "Solde"),
        devise: col(c & 32 != 0, "Devise"),
        identifiant_transaction: col(c & 64 != 0, "Id"),
    };
    ProfilReleve::try_from(ProfilBrut {
        encodage: encodage.to_owned(),
        delimiteur: delimiteur.to_owned(),
        guillemet: guillemet.to_owned(),
        lignes_entete,
        lignes_pied,
        format_date: format_date.to_owned(),
        separateur_decimal: decimal.to_owned(),
        separateur_milliers: milliers.map(str::to_owned),
        signe_inverse: b & 0x80 != 0,
        colonnes,
    })
    .ok()
}

fuzz_target!(|octets: &[u8]| {
    let [a, b, c, fichier @ ..] = octets else {
        return;
    };
    let Some(profil) = profil(*a, *b, *c) else {
        return;
    };
    let Ok(releve) = analyser(fichier, &profil, &LIMITES) else {
        return;
    };
    assert!(releve.lignes().len() <= LIMITES.lignes);
    assert!(releve.lignes().iter().all(|l| l.ligne() >= 1));
    let mouvements: Vec<_> = releve.mouvements().cloned().collect();
    if let Ok(ecrit) = serialiser(&mouvements, &profil) {
        let larges = Limites {
            taille_fichier: usize::MAX,
            lignes: usize::MAX,
            colonnes: usize::MAX,
            longueur_ligne: usize::MAX,
            longueur_cellule: usize::MAX,
        };
        let relu = analyser(&ecrit, &profil, &larges).expect("réécriture relisible");
        assert!(relu.mouvements().eq(mouvements.iter()));
    }
});
