//! Assistant `verifier_sans_valeur_metier` : insensible aux « 1000 » fortuits des UUID et
//! empreintes, mais strict sur les vraies valeurs métier.
// Fichier de test : les `expect`/`unwrap` y sont voulus.
#![allow(clippy::expect_used, clippy::unwrap_used)]

mod commun;

use commun::audit_valeurs::verifier_sans_valeur_metier;

const LIBELLES: &[&str] = &["Première"];
const MONTANTS: &[&str] = &["1000"];

const ENTREE: &str = "audit v1\nportee=dossier\nacteur=a1000b2c-1000-4000-8000-100000000000\n\
                      rang=7\nressource=ecriture:10001000-aaaa-bbbb-cccc-dddddddddddd\n\
                      donnee=empreinte\nvaleur=ab1000cd1000\nfin\n";

#[test]
fn un_1000_fortuit_dans_un_uuid_ou_une_empreinte_passe() {
    verifier_sans_valeur_metier(ENTREE, LIBELLES, MONTANTS);
}

#[test]
#[should_panic(expected = "montant 1000")]
fn un_montant_echoue() {
    let entree = format!("{ENTREE}donnee=montant\nvaleur=1000\n");
    verifier_sans_valeur_metier(&entree, LIBELLES, MONTANTS);
}

#[test]
#[should_panic(expected = "montant 1000")]
fn un_montant_decimal_equivalent_echoue() {
    let entree = format!("{ENTREE}donnee=montant\nvaleur=1000.00\n");
    verifier_sans_valeur_metier(&entree, LIBELLES, MONTANTS);
}

#[test]
#[should_panic(expected = "libellé")]
fn un_libelle_echoue() {
    let entree = format!("{ENTREE}donnee=libelle\nvaleur=Première écriture\n");
    verifier_sans_valeur_metier(&entree, LIBELLES, MONTANTS);
}
