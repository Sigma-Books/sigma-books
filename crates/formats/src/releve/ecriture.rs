//! Écriture d'un relevé selon un profil : réciproque de l'analyse
//! (`analyser(serialiser(m)) == m`), pour les fichiers de référence et les tests.
//!
//! Forme produite : `lignes_entete` lignes d'en-tête (la dernière porte les titres des colonnes
//! désignées par titre, `-` sinon), une ligne par mouvement, `lignes_pied` lignes `-`, fins de
//! ligne `\n`, sans BOM. Les colonnes désignées par titre occupent les premiers rangs libres.
//! Montants sans séparateur de milliers ; dates selon le format du profil.
//!
//! **Réservé aux tests et aux fichiers de référence, jamais pour un export** : les cellules
//! sont écrites telles quelles, sans neutralisation des formules (F7.6 ; la neutralisation des
//! exports relève de L7).

use rust_decimal::Decimal;
use thiserror::Error;

use super::profil::{Colonne, ProfilReleve};
use super::{Champ, Mouvement};

/// Refus d'écriture.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ErreurEcriture {
    /// Deux champs désignent la même colonne : l'écriture ne serait pas réciproque.
    #[error("deux champs désignent la même colonne")]
    ColonnesPartagees,
    /// Cellule contenant le délimiteur ou une fin de ligne, sans guillemet dans le profil.
    #[error("valeur non représentable sans guillemet")]
    CelluleNonRepresentable,
    /// Caractère absent de l'encodage du profil.
    #[error("caractère non représentable dans l'encodage du profil")]
    CaractereNonEncodable,
}

/// Champ, rang de sa colonne et titre éventuel.
type Place<'a> = (Champ, usize, Option<&'a str>);

/// Rang de chaque colonne désignée, et titres.
fn disposition(profil: &ProfilReleve) -> Result<Vec<Place<'_>>, ErreurEcriture> {
    let colonnes = profil.toutes_colonnes();
    let mut pris: Vec<usize> = Vec::new();
    for (_, col) in &colonnes {
        if let Colonne::Rang(r) = col {
            if pris.contains(r) {
                return Err(ErreurEcriture::ColonnesPartagees);
            }
            pris.push(*r);
        }
    }
    let mut titres: Vec<&str> = Vec::new();
    let mut libre = 0;
    let mut sortie = Vec::new();
    for (champ, col) in colonnes {
        match col {
            Colonne::Rang(r) => sortie.push((champ, *r, None)),
            Colonne::Titre(t) => {
                if titres.contains(&t.trim()) {
                    return Err(ErreurEcriture::ColonnesPartagees);
                }
                titres.push(t.trim());
                while pris.contains(&libre) {
                    libre += 1;
                }
                pris.push(libre);
                sortie.push((champ, libre, Some(t.as_str())));
            }
        }
    }
    Ok(sortie)
}

fn cellule(profil: &ProfilReleve, valeur: &str, sortie: &mut String) -> Result<(), ErreurEcriture> {
    let speciale = |c: char| c == profil.delimiteur || c == '\r' || c == '\n';
    match profil.guillemet {
        Some(q) if valeur.contains(|c: char| speciale(c) || c == q) => {
            sortie.push(q);
            for c in valeur.chars() {
                if c == q {
                    sortie.push(q);
                }
                sortie.push(c);
            }
            sortie.push(q);
        }
        None if valeur.contains(speciale) => return Err(ErreurEcriture::CelluleNonRepresentable),
        _ => sortie.push_str(valeur),
    }
    Ok(())
}

fn enregistrement(
    profil: &ProfilReleve,
    cellules: &[String],
    sortie: &mut String,
) -> Result<(), ErreurEcriture> {
    for (i, c) in cellules.iter().enumerate() {
        if i > 0 {
            sortie.push(profil.delimiteur);
        }
        cellule(profil, c, sortie)?;
    }
    sortie.push('\n');
    Ok(())
}

fn nombre(profil: &ProfilReleve, d: Decimal) -> String {
    d.to_string()
        .replace('.', &profil.separateur_decimal.to_string())
}

/// Écrit des mouvements selon le profil, dans son encodage.
/// Réservé aux tests et aux fichiers de référence, **jamais pour un export** : aucune
/// neutralisation des formules (F7.6, L7).
///
/// # Errors
/// Voir [`ErreurEcriture`].
pub fn serialiser(
    mouvements: &[Mouvement],
    profil: &ProfilReleve,
) -> Result<Vec<u8>, ErreurEcriture> {
    let disposition = disposition(profil)?;
    let largeur = disposition.iter().map(|(_, r, _)| r + 1).max().unwrap_or(1);
    let mut texte = String::new();
    let tiret = ["-".to_owned()];
    for i in 0..profil.lignes_entete {
        let titres = disposition.iter().any(|(_, _, t)| t.is_some());
        if i + 1 == profil.lignes_entete && titres {
            let mut cellules = vec![String::new(); largeur];
            for (_, r, t) in &disposition {
                if let (Some(t), Some(c)) = (t, cellules.get_mut(*r)) {
                    (*t).clone_into(c);
                }
            }
            enregistrement(profil, &cellules, &mut texte)?;
        } else {
            enregistrement(profil, &tiret, &mut texte)?;
        }
    }
    for m in mouvements {
        let mut cellules = vec![String::new(); largeur];
        let (montant, solde) = if profil.signe_inverse {
            (-m.montant, m.solde.map(|s| -s))
        } else {
            (m.montant, m.solde)
        };
        for (champ, r, _) in &disposition {
            let valeur = match champ {
                Champ::Date => profil.format_date.ecrire(m.date),
                Champ::DateValeur => m
                    .date_valeur
                    .map_or_default(|d| profil.format_date.ecrire(d)),
                Champ::Libelle => m.libelle.clone(),
                Champ::Reference => m.reference.clone().unwrap_or_default(),
                Champ::Montant => nombre(profil, montant),
                Champ::Debit if montant < Decimal::ZERO => nombre(profil, -montant),
                Champ::Credit if montant >= Decimal::ZERO => nombre(profil, montant),
                Champ::Debit | Champ::Credit => String::new(),
                Champ::Solde => solde.map_or_default(|s| nombre(profil, s)),
                Champ::Devise => m.devise.clone().unwrap_or_default(),
                Champ::IdentifiantTransaction => {
                    m.identifiant_transaction.clone().unwrap_or_default()
                }
            };
            if let Some(c) = cellules.get_mut(*r) {
                *c = valeur;
            }
        }
        enregistrement(profil, &cellules, &mut texte)?;
    }
    for _ in 0..profil.lignes_pied {
        enregistrement(profil, &tiret, &mut texte)?;
    }
    if profil.encodage.est_utf8() {
        return Ok(texte.into_bytes());
    }
    let (octets, _, erreurs) = profil.encodage.table().encode(&texte);
    if erreurs {
        return Err(ErreurEcriture::CaractereNonEncodable);
    }
    Ok(octets.into_owned())
}
