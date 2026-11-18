//! Éditions de base (spec §17.1) : balance 2/4/6/8 colonnes, grand livre, journaux.

// Fichier de test : les `unwrap` y sont voulus (une donnée de test invalide fait échouer le test).
#![allow(clippy::unwrap_used)]
mod commun;

use commun::*;
use compta_core::{
    Balance, CodeExercice, ErreurEdition, Filtre, FormatBalance, Mode, Regroupement, Sens,
    grand_livre, journaux,
};
use rust_decimal::Decimal;
use time::macros::date;

fn d(texte: &str) -> Decimal {
    texte.parse().unwrap()
}

/// Janvier : achat 1 000 et vente 3 000 ; mars : règlement fournisseur 1 000 ; brouillard :
/// vente de 200 en mars.
fn dossier() -> (Dossier, Vec<compta_core::EcritureEquilibree>) {
    let mut dos = Dossier::nouveau();
    dos.valider(equilibree(
        "AC",
        date!(2026 - 01 - 15),
        "Achat",
        vec![
            ligne("6052", Sens::Debit, "1000"),
            ligne_tiers("4011", Sens::Credit, "1000", "F1"),
        ],
    ))
    .unwrap();
    dos.valider(equilibree(
        "VT",
        date!(2026 - 01 - 20),
        "Vente",
        vec![
            ligne_tiers("4111", Sens::Debit, "3000", "C1"),
            ligne("7011", Sens::Credit, "3000"),
        ],
    ))
    .unwrap();
    dos.valider(equilibree(
        "BQ1",
        date!(2026 - 03 - 05),
        "Règlement",
        vec![
            ligne_tiers("4011", Sens::Debit, "1000", "F1"),
            ligne("521", Sens::Credit, "1000"),
        ],
    ))
    .unwrap();
    let brouillard = vec![equilibree(
        "VT",
        date!(2026 - 03 - 06),
        "Vente provisoire",
        vec![
            ligne_tiers("4111", Sens::Debit, "200", "C2"),
            ligne("7011", Sens::Credit, "200"),
        ],
    )];
    (dos, brouillard)
}

fn filtre_mars(dos: &Dossier) -> Filtre {
    let exercice = dos
        .exercices
        .exercice(&CodeExercice::parse("2026").unwrap())
        .unwrap();
    Filtre::nouveau(
        exercice,
        &dos.journaux,
        date!(2026 - 03 - 01),
        date!(2026 - 03 - 31),
    )
    .unwrap()
}

#[test]
fn balance_huit_colonnes_et_totaux() {
    let (dos, brouillard) = dossier();
    let filtre = filtre_mars(&dos);
    let balance = Balance::calculer(
        dos.livre.ecritures(),
        &brouillard,
        Mode::Definitif,
        &filtre,
        Regroupement::ParCompte,
    )
    .unwrap();
    let cles: Vec<&str> = balance
        .lignes()
        .iter()
        .map(compta_core::LigneBalance::cle)
        .collect();
    assert_eq!(cles, ["4011", "4111", "521", "6052", "7011"]);
    let f4011 = &balance.lignes()[0];
    // Ouverture créditrice 1 000 ; règlement de 1 000 au débit en mars ; solde nul.
    assert_eq!(
        f4011.colonnes(FormatBalance::Huit).unwrap(),
        [
            d("0"),
            d("1000"),
            d("1000"),
            d("0"),
            d("1000"),
            d("1000"),
            d("0"),
            d("0")
        ]
    );
    let banque = &balance.lignes()[2];
    assert_eq!(
        banque.colonnes(FormatBalance::Six).unwrap(),
        [d("0"), d("0"), d("0"), d("1000"), d("0"), d("1000")]
    );
    assert_eq!(
        banque.colonnes(FormatBalance::Deux).unwrap(),
        [d("0"), d("1000")]
    );
    assert_eq!(
        banque.colonnes(FormatBalance::Quatre).unwrap(),
        [d("0"), d("1000"), d("0"), d("1000")]
    );
    for format in [
        FormatBalance::Deux,
        FormatBalance::Quatre,
        FormatBalance::Six,
        FormatBalance::Huit,
    ] {
        let totaux = balance.totaux(format).unwrap();
        for paire in totaux.chunks(2) {
            assert_eq!(paire[0], paire[1], "{format:?} : {totaux:?}");
        }
    }
    assert_eq!(
        balance.totaux(FormatBalance::Six).unwrap(),
        [
            d("4000"),
            d("4000"),
            d("1000"),
            d("1000"),
            d("4000"),
            d("4000")
        ]
    );
}

#[test]
fn mode_provisoire_ajoute_le_brouillard() {
    let (dos, brouillard) = dossier();
    let filtre = filtre_mars(&dos);
    let provisoire = Balance::calculer(
        dos.livre.ecritures(),
        &brouillard,
        Mode::Provisoire,
        &filtre,
        Regroupement::ParCompte,
    )
    .unwrap();
    let ventes = provisoire
        .lignes()
        .iter()
        .find(|l| l.cle() == "7011")
        .unwrap();
    assert_eq!(ventes.mouvements(), (d("0"), d("200")));
    let definitif = Balance::calculer(
        dos.livre.ecritures(),
        &brouillard,
        Mode::Definitif,
        &filtre,
        Regroupement::ParCompte,
    )
    .unwrap();
    let ventes = definitif
        .lignes()
        .iter()
        .find(|l| l.cle() == "7011")
        .unwrap();
    assert_eq!(ventes.mouvements(), (d("0"), d("0")));
}

#[test]
fn agregation_par_tronc_et_filtres() {
    let (mut dos, _) = dossier();
    dos.plan
        .ajouter_auxiliaire(n("4111-C9"), t("Client 9"))
        .unwrap();
    dos.valider(equilibree(
        "VT",
        date!(2026 - 03 - 20),
        "Vente C9",
        vec![
            ligne_tiers("4111-C9", Sens::Debit, "50", "C9"),
            ligne("7011", Sens::Credit, "50"),
        ],
    ))
    .unwrap();
    let filtre = filtre_mars(&dos);
    let par_tronc = Balance::calculer(
        dos.livre.ecritures(),
        &[],
        Mode::Definitif,
        &filtre,
        Regroupement::ParTronc,
    )
    .unwrap();
    let clients = par_tronc
        .lignes()
        .iter()
        .find(|l| l.cle() == "4111")
        .unwrap();
    // 3 000 (janvier, ouverture) sur 4111 + 50 (mars) sur 4111-C9, agrégés sous le tronc.
    assert_eq!(
        clients.colonnes(FormatBalance::Deux).unwrap(),
        [d("3050"), d("0")]
    );

    let filtre_comptes = filtre_mars(&dos).comptes([n("41")]);
    let clients = Balance::calculer(
        dos.livre.ecritures(),
        &[],
        Mode::Definitif,
        &filtre_comptes,
        Regroupement::ParCompte,
    )
    .unwrap();
    let cles: Vec<&str> = clients
        .lignes()
        .iter()
        .map(compta_core::LigneBalance::cle)
        .collect();
    assert_eq!(cles, ["4111", "4111-C9"]);

    let filtre_journaux = filtre_mars(&dos).journaux([j("BQ1")]);
    let banque = Balance::calculer(
        dos.livre.ecritures(),
        &[],
        Mode::Definitif,
        &filtre_journaux,
        Regroupement::ParCompte,
    )
    .unwrap();
    let cles: Vec<&str> = banque
        .lignes()
        .iter()
        .map(compta_core::LigneBalance::cle)
        .collect();
    assert_eq!(cles, ["4011", "521"]);
}

#[test]
fn filtre_refuse_des_bornes_incoherentes() {
    let dos = Dossier::nouveau();
    let exercice = dos
        .exercices
        .exercice(&CodeExercice::parse("2026").unwrap())
        .unwrap();
    assert!(matches!(
        Filtre::nouveau(
            exercice,
            &dos.journaux,
            date!(2026 - 03 - 31),
            date!(2026 - 03 - 01)
        ),
        Err(ErreurEdition::BornesInversees { .. })
    ));
    assert_eq!(
        Filtre::nouveau(
            exercice,
            &dos.journaux,
            date!(2026 - 03 - 01),
            date!(2027 - 01 - 01)
        ),
        Err(ErreurEdition::HorsExercice(date!(2027 - 01 - 01)))
    );
}

#[test]
fn grand_livre_solde_progressif() {
    let (dos, brouillard) = dossier();
    let exercice = dos
        .exercices
        .exercice(&CodeExercice::parse("2026").unwrap())
        .unwrap();
    let filtre = Filtre::nouveau(
        exercice,
        &dos.journaux,
        date!(2026 - 01 - 01),
        date!(2026 - 03 - 31),
    )
    .unwrap()
    .comptes([n("4011")]);
    let gl = grand_livre(
        dos.livre.ecritures(),
        &brouillard,
        Mode::Provisoire,
        &filtre,
    )
    .unwrap();
    assert_eq!(gl.len(), 1);
    let f = &gl[0];
    assert_eq!(f.compte, n("4011"));
    assert_eq!(f.solde_ouverture, d("0"));
    let soldes: Vec<Decimal> = f.lignes.iter().map(|l| l.solde).collect();
    assert_eq!(soldes, [d("-1000"), d("0")]);
    assert_eq!(
        f.lignes[0].numero.as_ref().unwrap().to_string(),
        "AC-2026-000001"
    );
    assert_eq!(f.lignes[0].tiers.as_ref().unwrap().as_str(), "F1");
    assert_eq!(
        (f.total_debit, f.total_credit, f.solde_final),
        (d("1000"), d("1000"), d("0"))
    );

    // Sur mars seul, janvier passe dans l'ouverture.
    let gl = grand_livre(
        dos.livre.ecritures(),
        &brouillard,
        Mode::Provisoire,
        &filtre_mars(&dos).comptes([n("4011")]),
    )
    .unwrap();
    assert_eq!(gl[0].solde_ouverture, d("-1000"));
    assert_eq!(gl[0].lignes.len(), 1);
    assert_eq!(gl[0].solde_final, d("0"));
}

#[test]
fn grand_livre_brouillard_sans_numero_en_fin_de_journee() {
    let (dos, mut brouillard) = dossier();
    brouillard.push(equilibree(
        "BQ1",
        date!(2026 - 03 - 05),
        "Autre règlement (brouillard)",
        vec![
            ligne_tiers("4011", Sens::Debit, "1", "F1"),
            ligne("521", Sens::Credit, "1"),
        ],
    ));
    let gl = grand_livre(
        dos.livre.ecritures(),
        &brouillard,
        Mode::Provisoire,
        &filtre_mars(&dos).comptes([n("521")]),
    )
    .unwrap();
    let numeros: Vec<Option<String>> = gl[0]
        .lignes
        .iter()
        .map(|l| l.numero.as_ref().map(ToString::to_string))
        .collect();
    assert_eq!(numeros, [Some("BQ1-2026-000001".to_owned()), None]);
}

#[test]
fn edition_des_journaux() {
    let (dos, brouillard) = dossier();
    let exercice = dos
        .exercices
        .exercice(&CodeExercice::parse("2026").unwrap())
        .unwrap();
    let filtre = Filtre::nouveau(
        exercice,
        &dos.journaux,
        date!(2026 - 01 - 01),
        date!(2026 - 12 - 31),
    )
    .unwrap();
    let editions = journaux(
        dos.livre.ecritures(),
        &brouillard,
        Mode::Provisoire,
        &filtre,
    )
    .unwrap();
    let codes: Vec<&str> = editions.iter().map(|e| e.journal.as_str()).collect();
    assert_eq!(codes, ["AC", "BQ1", "VT"]);
    let ventes = &editions[2];
    assert_eq!(ventes.ecritures.len(), 2);
    assert!(ventes.ecritures[1].numero.is_none(), "brouillard");
    assert_eq!(
        (ventes.total_debit, ventes.total_credit),
        (d("3200"), d("3200"))
    );
    let definitif = journaux(dos.livre.ecritures(), &brouillard, Mode::Definitif, &filtre).unwrap();
    assert_eq!(definitif[2].ecritures.len(), 1);
}

/// AUDCIF art. 19 et Guide d'application, Application 127 : dans la balance générale (format 6,
/// depuis le début de l'exercice), les à-nouveaux forment la colonne d'ouverture, jamais les
/// mouvements, bien qu'ils soient datés du premier jour de l'exercice.
#[test]
fn a_nouveaux_dans_la_colonne_d_ouverture() {
    let mut dos = Dossier::nouveau();
    dos.valider(equilibree(
        "AC",
        date!(2026 - 01 - 01),
        "Achat du 1er janvier",
        vec![
            ligne("6052", Sens::Debit, "100"),
            ligne("521", Sens::Credit, "100"),
        ],
    ))
    .unwrap();
    // Le journal AN refuse la saisie (générés en L6) : l'à-nouveau est pris au brouillard.
    let a_nouveau = equilibree(
        "AN",
        date!(2026 - 01 - 01),
        "À-nouveaux",
        vec![
            ligne("521", Sens::Debit, "1000"),
            ligne_tiers("4011", Sens::Credit, "1000", "FOURNISSEUR-1"),
        ],
    );
    let exercice = dos
        .exercices
        .exercice(&CodeExercice::parse("2026").unwrap())
        .unwrap();
    let filtre = Filtre::exercice(exercice, &dos.journaux, date!(2026 - 12 - 31)).unwrap();
    let balance = Balance::calculer(
        dos.livre.ecritures(),
        &[a_nouveau],
        Mode::Provisoire,
        &filtre,
        Regroupement::ParCompte,
    )
    .unwrap();
    let colonnes = |compte: &str| {
        balance
            .lignes()
            .iter()
            .find(|l| l.cle() == compte)
            .unwrap()
            .colonnes(FormatBalance::Six)
            .unwrap()
    };
    let d = |texte: &str| texte.parse::<Decimal>().unwrap();
    let z = Decimal::ZERO;
    // 521 : ouverture 1 000 D ; mouvement 100 C (achat du 1er janvier) ; solde 900 D.
    assert_eq!(colonnes("521"), [d("1000"), z, z, d("100"), d("900"), z]);
    assert_eq!(colonnes("4011"), [z, d("1000"), z, z, z, d("1000")]);
    assert_eq!(colonnes("6052"), [z, z, d("100"), z, d("100"), z]);
    // Le journal AN reste édité à sa date.
    let journaux = compta_core::journaux(
        dos.livre.ecritures(),
        &[equilibree(
            "AN",
            date!(2026 - 01 - 01),
            "À-nouveaux",
            vec![
                ligne("521", Sens::Debit, "1000"),
                ligne_tiers("4011", Sens::Credit, "1000", "FOURNISSEUR-1"),
            ],
        )],
        Mode::Provisoire,
        &filtre,
    )
    .unwrap();
    assert!(journaux.iter().any(|j| j.journal.as_str() == "AN"));
}
