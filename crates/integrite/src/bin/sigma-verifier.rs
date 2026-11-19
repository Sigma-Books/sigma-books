//! `sigma-verifier` : vérification hors ligne de la chaîne de hachage d'un export d'intégrité
//! ou d'un export d'audit (`docs/integrite/audit-v1.md`), reconnu à sa première ligne.
//!
//! Usage : `sigma-verifier [--dossier ID] [--cabinet UUID] [--tete EMPREINTE] [--ancre FICHIER]…
//! <export | ->`
//!
//! Codes de retour : 0 intègre, 1 anomalie constatée, 2 usage incorrect ou lecture impossible.
//! L'outil ne se connecte à rien : il ne lit que l'export.

#![forbid(unsafe_code)]

use std::fs::File;
use std::io::{self, BufReader, Read, Write};
use std::process::ExitCode;

use integrite::audit::{AttentesAudit, ENTETE_EXPORT_AUDIT, Uuid, verifier_export_audit};
use integrite::{Ancre, Empreinte, IdDossier, Options, verifier};

const USAGE: &str = "\
Usage : sigma-verifier [--dossier ID] [--cabinet UUID] [--tete EMPREINTE] [--ancre FICHIER]…
                      <export | ->

Vérifie hors ligne un export d'intégrité Sigma Books : recalcule la chaîne de hachage,
contrôle la numérotation, l'équilibre des écritures et les contre-passations, et vérifie les
jetons d'horodatage RFC 3161 de l'export. Vérifie aussi un export d'audit (« sigma-books
export-audit v1 ») : forme des entrées, chaîne, rangs, tête, nombre et annexe des acteurs,
avec les options --dossier, --cabinet et --tete.

  --dossier ID         identifiant du dossier attendu (UUID pour un export d'audit)
  --cabinet UUID       cabinet attendu (export d'audit seulement)
  --tete EMPREINTE     empreinte de tête attendue (témoin externe)
  --ancre FICHIER      certificat racine supplémentaire d'une autorité d'horodatage (PEM ou
                       DER) ; les racines de DigiCert et de Sectigo sont intégrées
  -                    lit l'export sur l'entrée standard

Codes de retour : 0 intègre, 1 anomalie constatée, 2 usage incorrect ou lecture impossible.";

enum Commande {
    Verifier {
        chemin: String,
        options: Options,
        cabinet: Option<Uuid>,
    },
    Aide,
    Version,
}

fn analyser(arguments: impl IntoIterator<Item = String>) -> Result<Commande, String> {
    let mut options = Options::default();
    let mut cabinet = None;
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
            "--cabinet" => {
                let v = arguments.next().ok_or("--cabinet : valeur manquante")?;
                cabinet = Some(Uuid::parse(&v).map_err(|e| format!("--cabinet : {e}"))?);
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
    Ok(Commande::Verifier {
        chemin,
        options,
        cabinet,
    })
}

fn main() -> ExitCode {
    let (chemin, options, cabinet) = match analyser(std::env::args().skip(1)) {
        Ok(Commande::Verifier {
            chemin,
            options,
            cabinet,
        }) => (chemin, options, cabinet),
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
    let resultat = if chemin == "-" {
        verifier_selon_format(io::stdin().lock(), &options, cabinet)
    } else {
        File::open(&chemin).and_then(|f| verifier_selon_format(f, &options, cabinet))
    };
    match resultat {
        Ok((texte, integre)) => {
            // Une sortie fermée (tube interrompu) ne change pas le verdict.
            let _ = write!(io::stdout().lock(), "{texte}");
            if integre {
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

/// Vérifie un export d'intégrité ou d'audit, selon sa première ligne ; rend le rapport et le
/// verdict.
///
/// Le début du flux est lu jusqu'à la longueur de l'en-tête d'audit (ou jusqu'à la fin du flux,
/// quel que soit le découpage des lectures), puis rendu au vérificateur : rien n'est perdu.
fn verifier_selon_format<R: Read>(
    mut entree: R,
    options: &Options,
    cabinet: Option<Uuid>,
) -> io::Result<(String, bool)> {
    let entete = format!("{ENTETE_EXPORT_AUDIT}\n");
    let mut debut = Vec::with_capacity(entete.len());
    (&mut entree)
        .take(entete.len() as u64)
        .read_to_end(&mut debut)?;
    let audit = debut == entete.as_bytes();
    let flux = BufReader::new(io::Cursor::new(debut).chain(entree));
    if !audit {
        if cabinet.is_some() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "--cabinet ne s'applique qu'à un export d'audit",
            ));
        }
        let rapport = verifier(flux, options)?;
        return Ok((rapport.to_string(), rapport.est_integre()));
    }
    if !options.ancres.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "--ancre est sans objet pour un export d'audit",
        ));
    }
    let dossier = options
        .dossier
        .as_ref()
        .map(|d| Uuid::parse(d.as_str()))
        .transpose()
        .map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "--dossier : UUID canonique attendu pour un export d'audit",
            )
        })?;
    let attentes = AttentesAudit {
        tete: options.tete,
        cabinet,
        dossier,
    };
    // Comme pour l'export d'intégrité, un export mal formé est une anomalie (code 1).
    let rapport = match verifier_export_audit(flux, &attentes) {
        Ok(rapport) => rapport,
        Err(integrite::ErreurLecture::Io(e)) => return Err(e),
        Err(forme @ integrite::ErreurLecture::Forme { .. }) => {
            return Ok((format!("Export d'audit illisible : {forme}\n"), false));
        }
    };
    Ok((rapport.to_string(), rapport.est_integre()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Lecteur qui ne rend qu'un octet par lecture : la détection du format ne dépend pas du
    /// découpage du flux.
    struct OctetParOctet<'a>(&'a [u8]);

    impl Read for OctetParOctet<'_> {
        fn read(&mut self, tampon: &mut [u8]) -> io::Result<usize> {
            match (self.0.split_first(), tampon.first_mut()) {
                (Some((premier, reste)), Some(place)) => {
                    *place = *premier;
                    self.0 = reste;
                    Ok(1)
                }
                _ => Ok(0),
            }
        }
    }

    #[test]
    fn format_detecte_quel_que_soit_le_decoupage() {
        let audit = include_bytes!("../../tests/vecteurs/audit-v1/export-dossier.txt");
        let (texte, integre) =
            verifier_selon_format(OctetParOctet(audit), &Options::default(), None).unwrap();
        assert!(integre, "{texte}");
        assert!(texte.contains("chaîne d'audit intègre"), "{texte}");
        let integrite = include_bytes!("../../tests/vecteurs/export-v1.txt");
        let (texte, integre) =
            verifier_selon_format(OctetParOctet(integrite), &Options::default(), None).unwrap();
        assert!(integre, "{texte}");
        // Flux plus court que l'en-tête : lu jusqu'à la fin, traité en export d'intégrité.
        let (texte, integre) =
            verifier_selon_format(OctetParOctet(b"sigma"), &Options::default(), None).unwrap();
        assert!(!integre, "{texte}");
    }
}
