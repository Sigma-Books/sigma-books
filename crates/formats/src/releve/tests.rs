//! Tests de l'analyse : profils, limites, fichiers hostiles, empreintes.

use std::str::FromStr;

use rust_decimal::Decimal;
use time::macros::date;

use super::*;

const LIMITES: Limites = Limites {
    taille_fichier: 1 << 16,
    lignes: 50,
    colonnes: 10,
    longueur_ligne: 256,
    longueur_cellule: 64,
};

fn d(s: &str) -> Decimal {
    Decimal::from_str(s).unwrap()
}

fn profil(toml: &str) -> ProfilReleve {
    ProfilReleve::depuis_toml(toml).unwrap()
}

fn signe() -> ProfilReleve {
    profil(
        r#"
encodage = "utf-8"
delimiteur = ","
lignes_entete = 1
format_date = "%Y-%m-%d"
separateur_decimal = "."
[colonnes]
date = "Date"
libelle = "Libelle"
montant = "Montant"
reference = 3
identifiant_transaction = 4
"#,
    )
}

#[test]
fn analyse_simple_et_lignes_brutes() {
    let f = b"Date,Libelle,Montant,Ref,Id\n2026-01-05,\"Virement, recu\",1500.50,R1,\n\n2026-01-06,Frais,-12,,T2\n\n";
    let r = analyser(f, &signe(), &LIMITES).unwrap();
    assert_eq!(r.empreinte(), empreinte_fichier(f));
    assert_eq!(r.lignes().len(), 2);
    let l = &r.lignes()[0];
    assert_eq!(l.ligne(), 2);
    assert_eq!(l.brut(), "2026-01-05,\"Virement, recu\",1500.50,R1,");
    assert_eq!(
        l.mouvement(),
        &Mouvement {
            date: date!(2026 - 01 - 05),
            date_valeur: None,
            libelle: "Virement, recu".into(),
            reference: Some("R1".into()),
            montant: d("1500.50"),
            solde: None,
            devise: None,
            identifiant_transaction: None,
        }
    );
    assert_eq!(r.lignes()[1].ligne(), 4);
    assert_eq!(r.lignes()[1].mouvement().montant, d("-12"));
    assert_eq!(
        r.lignes()[1].mouvement().identifiant_transaction.as_deref(),
        Some("T2")
    );
    assert_eq!(r.alertes(), []);
}

#[test]
fn erreurs_avec_rang_de_ligne() {
    let p = signe();
    let cas: [(&[u8], ErreurReleve); 8] = [
        (
            b"Date,Libelle,Montant,Ref,Id\n2026-02-30,a,1,,\n",
            ErreurReleve::DateInvalide {
                ligne: 2,
                champ: Champ::Date,
            },
        ),
        (
            b"Date,Libelle,Montant,Ref,Id\n2026-02-03,a,1e3,,\n",
            ErreurReleve::NombreInvalide {
                ligne: 2,
                champ: Champ::Montant,
            },
        ),
        (
            b"Date,Libelle,Montant,Ref,Id\n2026-02-03,a,,,\n",
            ErreurReleve::NombreInvalide {
                ligne: 2,
                champ: Champ::Montant,
            },
        ),
        (
            b"Date,Libelle,Montant,Ref,Id\n\n2026-02-03,a,1\n",
            ErreurReleve::ColonneAbsente {
                ligne: 3,
                champ: Champ::Reference,
            },
        ),
        (
            b"Date,Libelle,Somme,Ref,Id\n",
            ErreurReleve::TitreIntrouvable {
                ligne: 1,
                champ: Champ::Montant,
            },
        ),
        (b"", ErreurReleve::TitresAbsents),
        (
            b"Date,Libelle,Montant,Ref,Id\n2026-02-03,a\x00,1,,\n",
            ErreurReleve::OctetNul { ligne: 2 },
        ),
        (
            b"Date,Libelle,Montant,Ref,Id\r\n2026-02-03,\xC3(,1,,\r\n",
            ErreurReleve::EncodageInvalide { ligne: 2 },
        ),
    ];
    for (fichier, attendu) in cas {
        let e = analyser(fichier, &p, &LIMITES).unwrap_err();
        assert_eq!(e, attendu);
        // Les messages ne reprennent aucun contenu du fichier.
        assert!(!e.to_string().contains("2026"));
    }
}

#[test]
fn fichiers_hostiles() {
    let p = signe();
    let trop_grand = vec![b'a'; LIMITES.taille_fichier + 1];
    assert_eq!(
        analyser(&trop_grand, &p, &LIMITES).unwrap_err(),
        ErreurReleve::FichierTropGrand {
            taille: LIMITES.taille_fichier + 1,
            limite: LIMITES.taille_fichier
        }
    );
    let longue = format!("Date\n{}\n", "x".repeat(257));
    assert_eq!(
        analyser(longue.as_bytes(), &p, &LIMITES).unwrap_err(),
        ErreurReleve::LigneTropLongue {
            ligne: 2,
            limite: 256
        }
    );
    // Retours chariot seuls : fins de ligne, aucune ligne trop longue.
    let cr = "a\r".repeat(40);
    assert!(decoder(cr.as_bytes(), Encodage::UTF8, &LIMITES).is_ok());
    for controle in ["\u{1b}", "\u{7f}", "\u{85}", "\u{c}"] {
        let f = format!("Date\nx{controle}\n");
        assert_eq!(
            analyser(f.as_bytes(), &p, &LIMITES).unwrap_err(),
            ErreurReleve::CaractereDeControle { ligne: 2 }
        );
    }
    // UTF-16 (BOM puis octets nuls) : refusé comme texte non admis.
    assert_eq!(
        analyser(b"\xFF\xFED\x00a\x00", &p, &LIMITES).unwrap_err(),
        ErreurReleve::OctetNul { ligne: 1 }
    );
    let guillemet = b"Date,Libelle,Montant,Ref,Id\n2026-01-01,\"non ferme,1,,\n";
    assert_eq!(
        analyser(guillemet, &p, &LIMITES).unwrap_err(),
        ErreurReleve::GuillemetNonFerme { ligne: 2 }
    );
    let colonnes = format!("{}\n", ",".repeat(10));
    assert_eq!(
        analyser(colonnes.as_bytes(), &p, &LIMITES).unwrap_err(),
        ErreurReleve::TropDeColonnes {
            ligne: 1,
            limite: 10
        }
    );
    let lignes = "\n".repeat(51);
    assert_eq!(
        analyser(lignes.as_bytes(), &p, &LIMITES).unwrap_err(),
        ErreurReleve::TropDeLignes {
            ligne: 51,
            limite: 50
        }
    );
}

#[test]
fn bom_utf8_retire_et_encodage_a_un_octet() {
    let p = signe();
    let f = b"\xEF\xBB\xBFDate,Libelle,Montant,Ref,Id\n2026-01-05,a,1,,\n";
    assert_eq!(analyser(f, &p, &LIMITES).unwrap().lignes().len(), 1);
    let w = Encodage::depuis_libelle("Windows-1252").unwrap();
    assert_eq!(
        decoder(b"Caf\xE9 \x80 \xA0", w, &LIMITES).unwrap(),
        "Café € \u{a0}"
    );
    // Octet non défini en windows-1252 : caractère de contrôle C1, refusé.
    assert_eq!(
        decoder(b"a\n\x81", w, &LIMITES).unwrap_err(),
        ErreurReleve::CaractereDeControle { ligne: 2 }
    );
    // ISO-8859-3 : octet sans caractère.
    let i3 = Encodage::depuis_libelle("iso-8859-3").unwrap();
    assert_eq!(
        decoder(b"\xA5", i3, &LIMITES).unwrap_err(),
        ErreurReleve::EncodageInvalide { ligne: 1 }
    );
    // Le BOM n'est pas retiré hors UTF-8 : il est lu dans l'encodage du profil.
    assert_eq!(decoder(b"\xEF\xBB\xBFa", w, &LIMITES).unwrap(), "ï»¿a");
    assert!(Encodage::depuis_libelle("utf-16le").is_none());
    assert!(Encodage::depuis_libelle("x-user-defined").is_none());
}

fn debit_credit(entete: usize, pied: usize, inverse: bool) -> ProfilReleve {
    profil(&format!(
        r#"
encodage = "utf-8"
delimiteur = ";"
guillemet = ""
lignes_entete = {entete}
lignes_pied = {pied}
format_date = "%d/%m/%Y"
separateur_decimal = ","
separateur_milliers = "."
signe_inverse = {inverse}
[colonnes]
date = 0
libelle = 1
debit = 2
credit = 3
solde = 4
"#
    ))
}

#[test]
fn debit_credit_entete_pied_et_signe_inverse() {
    let f = "Banque fictive\nDate;Libellé;Débit;Crédit;Solde\n01/02/2026;Retrait;1.000,00;;-1.000,00\n02/02/2026;Dépôt;;2.500,5;1.500,5\n03/02/2026;Net;10;4;1.494,5\nSolde final;;;;1.494,5\n\n";
    let r = analyser(f.as_bytes(), &debit_credit(2, 1, false), &LIMITES).unwrap();
    let m: Vec<_> = r.mouvements().map(|m| (m.montant, m.solde)).collect();
    assert_eq!(
        m,
        [
            (d("-1000.00"), Some(d("-1000.00"))),
            (d("2500.5"), Some(d("1500.5"))),
            (d("-6"), Some(d("1494.5"))),
        ]
    );
    let r = analyser(f.as_bytes(), &debit_credit(2, 1, true), &LIMITES).unwrap();
    assert_eq!(r.lignes()[0].mouvement().montant, d("1000.00"));
    assert_eq!(r.lignes()[0].mouvement().solde, Some(d("1000.00")));
    // En-tête et pied plus longs que le fichier : aucune ligne, sans erreur.
    let r = analyser(f.as_bytes(), &debit_credit(4, 3, false), &LIMITES).unwrap();
    assert_eq!(r.lignes(), []);
}

#[test]
fn alertes_formule_et_bidirectionnel() {
    let f = "x\n01/01/2026;=SOMME(A1);1;;\n01/01/2026;+33 77;1;;\n01/01/2026;Normal;1;;\n01/01/2026;@cmd;1;;\n01/01/2026;fac\u{202E}fdp.exe;1;;\n";
    let r = analyser(f.as_bytes(), &debit_credit(1, 0, false), &LIMITES).unwrap();
    assert_eq!(r.lignes().len(), 5);
    assert_eq!(r.lignes()[0].mouvement().libelle, "=SOMME(A1)");
    assert_eq!(r.lignes()[4].mouvement().libelle, "fac\u{202E}fdp.exe");
    let codes = |r: &Releve| {
        r.alertes()
            .iter()
            .map(|a| (a.ligne, a.champ, a.nature.code()))
            .collect::<Vec<_>>()
    };
    assert_eq!(
        codes(&r),
        [
            (2, Champ::Libelle, "libelle_formule"),
            (3, Champ::Libelle, "libelle_formule"),
            (5, Champ::Libelle, "libelle_formule"),
            (6, Champ::Libelle, "caractere_bidirectionnel"),
        ]
    );
    // Référence, devise et identifiant sont aussi contrôlés.
    let f = "Date,Libelle,Montant,Ref,Id\n2026-01-05,a,1,@ref,\u{2067}T1\n2026-01-06,b,-1,,-T2\n";
    let r = analyser(f.as_bytes(), &signe(), &LIMITES).unwrap();
    assert_eq!(
        codes(&r),
        [
            (2, Champ::Reference, "libelle_formule"),
            (2, Champ::IdentifiantTransaction, "caractere_bidirectionnel"),
            (3, Champ::IdentifiantTransaction, "libelle_formule"),
        ]
    );
}

#[test]
fn signe_conserve_avec_separateur_de_milliers() {
    let p = profil(
        r#"
encodage = "utf-8"
delimiteur = ";"
format_date = "%d/%m/%Y"
separateur_decimal = ","
separateur_milliers = "."
[colonnes]
date = 0
montant = 1
"#,
    );
    let r = analyser(b"01/01/2026;-1.500,25\n02/01/2026;+2.000\n", &p, &LIMITES).unwrap();
    assert_eq!(
        r.mouvements().map(|m| m.montant).collect::<Vec<_>>(),
        [d("-1500.25"), d("2000")]
    );
}

fn mouvement(libelle: &str, montant: &str) -> Mouvement {
    Mouvement {
        date: date!(2026 - 01 - 05),
        date_valeur: None,
        libelle: libelle.into(),
        reference: None,
        montant: d(montant),
        solde: None,
        devise: None,
        identifiant_transaction: None,
    }
}

#[test]
fn empreinte_de_ligne() {
    let a = mouvement("Frais  de\ttenue ", "-1500.00");
    // Libellé normalisé (espaces), montant normalisé (échelle), casse conservée.
    assert_eq!(
        empreinte_ligne("c1", &a),
        empreinte_ligne("c1", &mouvement(" Frais de tenue", "-1500"))
    );
    assert_ne!(
        empreinte_ligne("c1", &a),
        empreinte_ligne("c1", &mouvement("FRAIS DE TENUE", "-1500"))
    );
    assert_ne!(empreinte_ligne("c1", &a), empreinte_ligne("c2", &a));
    let mut r = a.clone();
    r.reference = Some(String::new());
    assert_ne!(empreinte_ligne("c1", &a), empreinte_ligne("c1", &r));
    // L'identifiant de transaction prime sur les autres champs.
    let mut x = a.clone();
    x.identifiant_transaction = Some("T1".into());
    let mut y = mouvement("autre", "3");
    y.identifiant_transaction = Some("T1".into());
    assert_eq!(empreinte_ligne("c1", &x), empreinte_ligne("c1", &y));
    // Zéro négatif et zéro : même empreinte.
    assert_eq!(
        empreinte_ligne("c1", &mouvement("z", "-0.00")),
        empreinte_ligne("c1", &mouvement("z", "0"))
    );
    // Vecteur figé du format `releve-ligne-v1`.
    assert_eq!(empreinte_ligne("c1", &a).to_string(), VECTEUR_V1);
}

/// Vecteur figé : SHA-256 de
/// `sigma-books/releve-ligne/v1\nchamps\ncompte=2:c1\ndate=10:2026-01-05\nmontant=5:-1500\nlibelle=14:Frais de tenue\nreference=-\n`.
const VECTEUR_V1: &str = "da60fd982e5b3984cffadc5d0fde4334a3451ea8167a3b126522d94a0a586769";

#[test]
fn rangs_d_occurrence() {
    let m = [
        mouvement("a", "1"),
        mouvement("b", "1"),
        mouvement("a", "1"),
        mouvement("a", "1"),
    ];
    let e = empreintes_rangees("c", &m);
    assert_eq!(e.iter().map(|e| e.rang).collect::<Vec<_>>(), [1, 1, 2, 3]);
    assert_eq!(e[0].empreinte, e[2].empreinte);
}

#[test]
fn ecriture_refusee() {
    let p = debit_credit(0, 0, false);
    let mut m = mouvement("a;b", "1");
    assert_eq!(
        serialiser(std::slice::from_ref(&m), &p).unwrap_err(),
        ErreurEcriture::CelluleNonRepresentable
    );
    m.libelle = "€".into();
    let w = profil(
        r#"
encodage = "iso-8859-1"
delimiteur = ";"
format_date = "%d/%m/%Y"
separateur_decimal = ","
[colonnes]
date = 0
libelle = 1
montant = 2
"#,
    );
    // iso-8859-1 suit windows-1252 (WHATWG) : l'euro y existe.
    assert!(serialiser(std::slice::from_ref(&m), &w).is_ok());
    m.libelle = "Ω".into();
    assert_eq!(
        serialiser(&[m], &w).unwrap_err(),
        ErreurEcriture::CaractereNonEncodable
    );
    let partage = profil(
        r#"
encodage = "utf-8"
delimiteur = ";"
format_date = "%d/%m/%Y"
separateur_decimal = ","
[colonnes]
date = 0
libelle = 0
montant = 2
"#,
    );
    assert_eq!(
        serialiser(&[], &partage).unwrap_err(),
        ErreurEcriture::ColonnesPartagees
    );
}
