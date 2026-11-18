//! `sigma-verifier` : vérification hors ligne de la chaîne de hachage d'un export.
//!
//! Usage : `sigma-verifier [--dossier ID] [--tete EMPREINTE] [--ancre FICHIER]… <export | ->`
//!
//! Codes de retour : 0 intègre, 1 anomalie constatée, 2 usage incorrect ou lecture impossible.
//! L'outil ne se connecte à rien : il ne lit que l'export.

#![forbid(unsafe_code)]

use std::fs::File;
use std::io::{self, BufReader, Write};
use std::process::ExitCode;

use integrite::{Ancre, Empreinte, IdDossier, Options, verifier};

const USAGE: &str = "\
Usage : sigma-verifier [--dossier ID] [--tete EMPREINTE] [--ancre FICHIER]… <export | ->

Vérifie hors ligne un export d'intégrité Sigma Books : recalcule la chaîne de hachage,
contrôle la numérotation, l'équilibre des écritures et les contre-passations, et vérifie les
jetons d'horodatage RFC 3161 de l'export.

  --dossier ID         identifiant du dossier attendu
  --tete EMPREINTE     empreinte de tête attendue (témoin externe)
  --ancre FICHIER      certificat racine supplémentaire d'une autorité d'horodatage (PEM ou
                       DER) ; les racines de DigiCert et de Sectigo sont intégrées
  -                    lit l'export sur l'entrée standard

Codes de retour : 0 intègre, 1 anomalie constatée, 2 usage incorrect ou lecture impossible.";

enum Commande {
    Verifier { chemin: String, options: Options },
    Aide,
    Version,
}

fn analyser(arguments: impl IntoIterator<Item = String>) -> Result<Commande, String> {
    let mut options = Options::default();
    let mut chemin = None;
    let mut arguments = arguments.into_iter();
    while let Some(a) = arguments.next() {
        match a.as_str() {
            "-h" | "--help" | "--aide" => return Ok(Commande::Aide),
            "-V" | "--version" => return Ok(Commande::Version),
            "--dossier" => {
                let v = arguments.next().ok_or("--dossier : valeur manquante")?;
                options.dossier =
                    Some(IdDossier::parse(&v).map_err(|e| format!("--dossier : {e}"))?);
            }
            "--tete" => {
                let v = arguments.next().ok_or("--tete : valeur manquante")?;
                options.tete = Some(Empreinte::parse(&v).map_err(|e| format!("--tete : {e}"))?);
            }
            "--ancre" => {
                let v = arguments.next().ok_or("--ancre : valeur manquante")?;
                let octets =
                    std::fs::read(&v).map_err(|e| format!("--ancre : lecture de « {v} » : {e}"))?;
                let ancres = if octets.starts_with(b"-----BEGIN") {
                    Ancre::depuis_pem(&octets)
                } else {
                    Ancre::depuis_der(&octets).map(|a| vec![a])
                }
                .map_err(|e| format!("--ancre : {e}"))?;
                options.ancres.extend(ancres);
            }
            option if option.starts_with("--") => {
                return Err(format!("option inconnue : {option}"));
            }
            _ if chemin.is_some() => return Err("un seul export à la fois".to_owned()),
            _ => chemin = Some(a),
        }
    }
    let chemin = chemin.ok_or("export à vérifier manquant")?;
    Ok(Commande::Verifier { chemin, options })
}

fn main() -> ExitCode {
    let (chemin, options) = match analyser(std::env::args().skip(1)) {
        Ok(Commande::Verifier { chemin, options }) => (chemin, options),
        Ok(Commande::Aide) => {
            println!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        Ok(Commande::Version) => {
            println!("sigma-verifier {}", env!("CARGO_PKG_VERSION"));
            return ExitCode::SUCCESS;
        }
        Err(message) => {
            eprintln!("sigma-verifier : {message}\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    let rapport = if chemin == "-" {
        verifier(io::stdin().lock(), &options)
    } else {
        File::open(&chemin).and_then(|f| verifier(BufReader::new(f), &options))
    };
    match rapport {
        Ok(rapport) => {
            // Une sortie fermée (tube interrompu) ne change pas le verdict.
            let _ = write!(io::stdout().lock(), "{rapport}");
            if rapport.est_integre() {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(1)
            }
        }
        Err(e) => {
            eprintln!("sigma-verifier : lecture de « {chemin} » impossible : {e}");
            ExitCode::from(2)
        }
    }
}
