//! Propriétés de l'analyseur de relevés (plan L3, « Tests ») :
//! - `analyser(serialiser(m)) == m` pour tout profil valide engendré ;
//! - aucune panique sur des octets quelconques, bornes respectées ;
//! - l'ordre des lignes d'un fichier ne change pas l'ensemble des (empreinte, rang).

use proptest::prelude::*;
use rust_decimal::Decimal;
use time::macros::date;
use time::{Date, Duration};

use formats::releve::{
    Colonne, ColonnesBrutes, ErreurReleve, Limites, Mouvement, ProfilBrut, ProfilReleve, analyser,
    decoder, empreintes_rangees, serialiser,
};

const LARGES: Limites = Limites {
    taille_fichier: 1 << 22,
    lignes: 10_000,
    colonnes: 50,
    longueur_ligne: 1 << 20,
    longueur_cellule: 10_000,
};

const ETROITES: Limites = Limites {
    taille_fichier: 4_096,
    lignes: 12,
    colonnes: 6,
    longueur_ligne: 80,
    longueur_cellule: 16,
};

const FORMATS_DATE: [&str; 6] = [
    "%d/%m/%Y",
    "%Y-%m-%d",
    "%d.%m.%y",
    "%d%m%Y",
    "%Y-%m-%d %H:%M:%S",
    "%m/%d/%Y",
];

/// Champs facultatifs, dans l'ordre : date de valeur, libellé, référence, solde, devise,
/// identifiant.
#[derive(Debug, Clone)]
struct Plan {
    encodage: &'static str,
    delimiteur: char,
    guillemet: Option<char>,
    lignes_entete: usize,
    lignes_pied: usize,
    format_date: &'static str,
    decimal: char,
    milliers: Option<char>,
    signe_inverse: bool,
    debit_credit: bool,
    facultatifs: [bool; 6],
    titres: Vec<bool>,
    rangs: Vec<usize>,
}

fn plan() -> impl Strategy<Value = Plan> {
    (
        prop::sample::select(vec!["utf-8", "windows-1252", "iso-8859-15"]),
        prop::sample::select(vec![',', ';', '\t', '|']),
        prop::sample::select(vec![Some('"'), Some('\''), None]),
        0usize..3,
        0usize..3,
        prop::sample::select(FORMATS_DATE.to_vec()),
        prop::sample::select(vec!['.', ',']),
        prop::sample::select(vec![None, Some(' '), Some('\u{a0}'), Some('\''), Some('.')]),
        any::<bool>(),
        any::<bool>(),
        any::<[bool; 6]>(),
        (
            prop::collection::vec(any::<bool>(), 9),
            Just((0..9).collect::<Vec<usize>>()).prop_shuffle(),
        ),
    )
        .prop_map(
            |(e, d, g, entete, pied, f, dec, mil, inv, dc, fac, (titres, rangs))| Plan {
                encodage: e,
                delimiteur: d,
                guillemet: g,
                lignes_entete: entete,
                lignes_pied: pied,
                format_date: f,
                decimal: dec,
                milliers: mil.filter(|&m| m != dec),
                signe_inverse: inv,
                debit_credit: dc,
                facultatifs: fac,
                titres,
                rangs,
            },
        )
        // Sans guillemet, la virgule décimale ne peut pas être aussi le délimiteur.
        .prop_filter("décimale et délimiteur", |p| {
            p.guillemet.is_some() || p.decimal != p.delimiteur
        })
}

impl Plan {
    /// Profil du plan ; `None` s'il est refusé (aucun ne doit l'être : voir `aller_retour`).
    fn profil(&self) -> Option<ProfilReleve> {
        let mut n = 0;
        let mut suivante = || -> Colonne {
            let i = n;
            n += 1;
            if self.lignes_entete > 0 && self.titres[i] {
                Colonne::Titre(format!("Titre {i}"))
            } else {
                Colonne::Rang(self.rangs[i])
            }
        };
        let date = suivante();
        let mut col = |present: bool| present.then(&mut suivante);
        let [dv, lib, refe, solde, devise, id] = self.facultatifs;
        let colonnes = ColonnesBrutes {
            date,
            date_valeur: col(dv),
            libelle: col(lib),
            reference: col(refe),
            montant: col(!self.debit_credit),
            debit: col(self.debit_credit),
            credit: col(self.debit_credit),
            solde: col(solde),
            devise: col(devise),
            identifiant_transaction: col(id),
        };
        ProfilReleve::try_from(ProfilBrut {
            encodage: self.encodage.to_owned(),
            delimiteur: self.delimiteur.to_string(),
            guillemet: self.guillemet.map_or_default(String::from),
            lignes_entete: self.lignes_entete,
            lignes_pied: self.lignes_pied,
            format_date: self.format_date.to_owned(),
            separateur_decimal: self.decimal.to_string(),
            separateur_milliers: self.milliers.map(String::from),
            signe_inverse: self.signe_inverse,
            colonnes,
        })
        .ok()
    }

    /// Texte représentable : encodable dans les trois encodages, sans caractère de contrôle
    /// hors tabulation et fins de ligne, sans délimiteur ni fin de ligne s'il n'y a pas de
    /// guillemet.
    fn texte(&self) -> impl Strategy<Value = String> + use<> {
        let mut alphabet: Vec<char> = " azAZ09-+=@.,;|'\"\téèàçô€".chars().collect();
        if self.guillemet.is_some() {
            alphabet.extend(['\n', '\r']);
        } else {
            alphabet.retain(|&c| c != self.delimiteur);
        }
        prop::collection::vec(prop::sample::select(alphabet), 0..12)
            .prop_map(|v| v.into_iter().collect())
    }

    fn mouvement(&self) -> impl Strategy<Value = Mouvement> + use<> {
        let [dv, lib, refe, solde, devise, id] = self.facultatifs;
        let non_vide = |s: String| (!s.is_empty()).then_some(s);
        (
            date(),
            prop::option::of(date()),
            self.texte(),
            self.texte(),
            montant(),
            prop::option::of(montant()),
            self.texte(),
            self.texte(),
        )
            .prop_map(move |(d, v, l, r, m, s, dev, i)| Mouvement {
                date: d,
                date_valeur: v.filter(|_| dv),
                libelle: if lib { l } else { String::new() },
                reference: non_vide(r).filter(|_| refe),
                montant: m,
                solde: s.filter(|_| solde),
                devise: non_vide(dev).filter(|_| devise),
                identifiant_transaction: non_vide(i).filter(|_| id),
            })
    }
}

/// Dates de 1970 à 2060 (année sur deux chiffres : pivot de 1969 à 2068).
fn date() -> impl Strategy<Value = Date> {
    (0i64..33_000).prop_map(|j| date!(1970 - 01 - 01) + Duration::days(j))
}

fn montant() -> impl Strategy<Value = Decimal> {
    (-1_000_000_000_000_000i64..1_000_000_000_000_000, 0u32..5)
        .prop_map(|(m, e)| Decimal::new(m, e))
}

/// Plan, profil (s'il est accepté) et mouvements.
fn profil_et_mouvements() -> impl Strategy<Value = (Plan, Option<ProfilReleve>, Vec<Mouvement>)> {
    plan().prop_flat_map(|p| {
        let profil = p.profil();
        let mouvements = prop::collection::vec(p.mouvement(), 0..8);
        (Just(p), Just(profil), mouvements)
    })
}

fn profils_fixes() -> Vec<ProfilReleve> {
    [
        "encodage = \"utf-8\"\ndelimiteur = \",\"\nlignes_entete = 1\nformat_date = \"%Y-%m-%d\"\nseparateur_decimal = \".\"\n[colonnes]\ndate = \"Date\"\nlibelle = 1\nmontant = \"Montant\"\nsolde = 3\n",
        "encodage = \"windows-1252\"\ndelimiteur = \";\"\nguillemet = \"\"\nlignes_pied = 1\nformat_date = \"%d/%m/%y\"\nseparateur_decimal = \",\"\nseparateur_milliers = \" \"\n[colonnes]\ndate = 0\ndebit = 1\ncredit = 2\ndate_valeur = 3\n",
    ]
    .iter()
    .filter_map(|t| ProfilReleve::depuis_toml(t).ok())
    .collect()
}

/// Octets proches d'un CSV, pour aller au-delà du décodage.
fn octets_csv() -> impl Strategy<Value = Vec<u8>> {
    prop::collection::vec(
        prop::sample::select(
            b"0123456789/-.,; \t\"\r\nDateMontantabc\xc3\xa9\xe9\x00\x1b".to_vec(),
        ),
        0..600,
    )
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    #[test]
    fn aller_retour((plan, profil, mouvements) in profil_et_mouvements()) {
        let Some(profil) = profil else {
            return Err(TestCaseError::fail(format!("profil refusé : {plan:?}")));
        };
        let octets = serialiser(&mouvements, &profil).unwrap();
        let releve = analyser(&octets, &profil, &LARGES).unwrap();
        let relus: Vec<_> = releve.mouvements().cloned().collect();
        prop_assert_eq!(relus, mouvements);
    }

    #[test]
    fn aucune_panique_et_bornes(
        octets in prop_oneof![prop::collection::vec(any::<u8>(), 0..600), octets_csv()],
    ) {
        let profils = profils_fixes();
        prop_assert_eq!(profils.len(), 2);
        for profil in profils {
            match analyser(&octets, &profil, &ETROITES) {
                Ok(r) => {
                    prop_assert!(r.lignes().len() <= ETROITES.lignes);
                    for l in r.lignes() {
                        prop_assert!(l.ligne() >= 1);
                    }
                }
                Err(e) => prop_assert!(e.ligne().is_none_or(|l| l >= 1)),
            }
        }
    }

    #[test]
    fn decodage_borne_et_sans_controle(octets in prop::collection::vec(any::<u8>(), 0..600)) {
        for libelle in ["utf-8", "windows-1252", "iso-8859-7"] {
            let enc = formats::releve::Encodage::depuis_libelle(libelle).unwrap();
            if let Ok(t) = decoder(&octets, enc, &ETROITES) {
                prop_assert!(t.len() <= 3 * octets.len());
                prop_assert!(!t.chars().any(|c| c.is_control() && !matches!(c, '\t' | '\r' | '\n')));
                for l in t.split(['\r', '\n']) {
                    prop_assert!(l.chars().count() <= ETROITES.longueur_ligne);
                }
            }
        }
    }

    #[test]
    fn taille_au_dela_de_la_limite_refusee(surplus in 1usize..64, octet in any::<u8>()) {
        let octets = vec![octet; ETROITES.taille_fichier + surplus];
        for profil in profils_fixes() {
            let refus = matches!(
                analyser(&octets, &profil, &ETROITES),
                Err(ErreurReleve::FichierTropGrand { .. })
            );
            prop_assert!(refus);
        }
    }

    #[test]
    fn ordre_des_lignes_sans_effet_sur_les_empreintes(
        mouvements in prop::collection::vec(
            (0i64..3, prop::sample::select(vec!["a", "b", " a  "]), -2i64..2).prop_map(|(j, l, m)| {
                Mouvement {
                    date: date!(2026 - 03 - 01) + Duration::days(j),
                    date_valeur: None,
                    libelle: l.to_owned(),
                    reference: None,
                    montant: Decimal::from(m),
                    solde: None,
                    devise: None,
                    identifiant_transaction: None,
                }
            }),
            0..20,
        ).prop_shuffle().prop_flat_map(|v| (Just(v.clone()), Just(v).prop_shuffle())),
    ) {
        let (a, b) = mouvements;
        let mut ea = empreintes_rangees("compte", &a);
        let mut eb = empreintes_rangees("compte", &b);
        ea.sort();
        eb.sort();
        prop_assert_eq!(ea, eb);
    }
}
