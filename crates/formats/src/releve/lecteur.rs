//! Découpage CSV d'un texte décodé en enregistrements, limites appliquées **pendant la
//! lecture** (F11.4) : nombre d'enregistrements, de colonnes, longueur de cellule.
//!
//! Comportement du module `csv` de Python (dialecte `excel`, `strict=False`), qu'utilisent
//! Tryton et OCA : séparateur et guillemet du profil ; guillemet doublé = guillemet littéral ;
//! un guillemet au milieu d'une cellule non entourée est littéral ; après le guillemet fermant,
//! les caractères suivants sont ajoutés à la cellule ; fins de ligne `\r\n`, `\n` ou `\r` ; une
//! fin de ligne entre guillemets appartient à la cellule ; une ligne vide donne un
//! enregistrement d'une cellule vide. **Écart** : la fin du fichier dans un champ entre
//! guillemets est une erreur (fichier tronqué, F11.4), là où Python rend le champ.

use super::{ErreurReleve, Limites};

/// Enregistrement lu : ligne physique de début, texte brut (sans la fin de ligne), cellules.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Enregistrement {
    pub(crate) ligne: usize,
    pub(crate) brut: String,
    pub(crate) cellules: Vec<String>,
}

impl Enregistrement {
    /// Toutes les cellules sont vides (OCA `skip_empty_lines`).
    pub(crate) fn est_vide(&self) -> bool {
        self.cellules.iter().all(String::is_empty)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Etat {
    DebutCellule,
    Cellule,
    EntreGuillemets,
    ApresGuillemet,
}

struct Lecteur<'a> {
    limites: &'a Limites,
    enregistrements: Vec<Enregistrement>,
    cellules: Vec<String>,
    cellule: String,
    caracteres: usize,
    ligne_debut: usize,
}

impl Lecteur<'_> {
    fn ajouter(&mut self, c: char) -> Result<(), ErreurReleve> {
        self.caracteres += 1;
        if self.caracteres > self.limites.longueur_cellule {
            return Err(ErreurReleve::CelluleTropLongue {
                ligne: self.ligne_debut,
                colonne: self.cellules.len(),
                limite: self.limites.longueur_cellule,
            });
        }
        self.cellule.push(c);
        Ok(())
    }

    fn fermer_cellule(&mut self) -> Result<(), ErreurReleve> {
        if self.cellules.len() >= self.limites.colonnes {
            return Err(ErreurReleve::TropDeColonnes {
                ligne: self.ligne_debut,
                limite: self.limites.colonnes,
            });
        }
        self.cellules.push(std::mem::take(&mut self.cellule));
        self.caracteres = 0;
        Ok(())
    }

    fn fermer_enregistrement(&mut self, brut: &str) -> Result<(), ErreurReleve> {
        self.fermer_cellule()?;
        if self.enregistrements.len() >= self.limites.lignes {
            return Err(ErreurReleve::TropDeLignes {
                ligne: self.ligne_debut,
                limite: self.limites.lignes,
            });
        }
        self.enregistrements.push(Enregistrement {
            ligne: self.ligne_debut,
            brut: brut.to_owned(),
            cellules: std::mem::take(&mut self.cellules),
        });
        Ok(())
    }
}

/// Lit tous les enregistrements de `texte`.
pub(crate) fn lire(
    texte: &str,
    delimiteur: char,
    guillemet: Option<char>,
    limites: &Limites,
) -> Result<Vec<Enregistrement>, ErreurReleve> {
    let mut l = Lecteur {
        limites,
        enregistrements: Vec::new(),
        cellules: Vec::new(),
        cellule: String::new(),
        caracteres: 0,
        ligne_debut: 1,
    };
    let mut etat = Etat::DebutCellule;
    let mut ligne = 1;
    let mut debut: Option<usize> = None;
    let mut caracteres = texte.char_indices().peekable();
    while let Some((i, c)) = caracteres.next() {
        let depart = *debut.get_or_insert_with(|| {
            l.ligne_debut = ligne;
            i
        });
        if etat == Etat::EntreGuillemets {
            if Some(c) == guillemet {
                etat = Etat::ApresGuillemet;
            } else {
                l.ajouter(c)?;
                if c == '\r' && caracteres.peek().map(|&(_, s)| s) == Some('\n') {
                    caracteres.next();
                    l.ajouter('\n')?;
                }
                if c == '\r' || c == '\n' {
                    ligne += 1;
                }
            }
            continue;
        }
        if etat == Etat::ApresGuillemet && Some(c) == guillemet {
            l.ajouter(c)?;
            etat = Etat::EntreGuillemets;
        } else if c == delimiteur {
            l.fermer_cellule()?;
            etat = Etat::DebutCellule;
        } else if c == '\r' || c == '\n' {
            l.fermer_enregistrement(texte.get(depart..i).unwrap_or_default())?;
            if c == '\r' && caracteres.peek().map(|&(_, s)| s) == Some('\n') {
                caracteres.next();
            }
            ligne += 1;
            debut = None;
            etat = Etat::DebutCellule;
        } else if etat == Etat::DebutCellule && Some(c) == guillemet {
            etat = Etat::EntreGuillemets;
        } else {
            l.ajouter(c)?;
            etat = Etat::Cellule;
        }
    }
    if etat == Etat::EntreGuillemets {
        return Err(ErreurReleve::GuillemetNonFerme {
            ligne: l.ligne_debut,
        });
    }
    if let Some(depart) = debut {
        l.fermer_enregistrement(texte.get(depart..).unwrap_or_default())?;
    }
    Ok(l.enregistrements)
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIMITES: Limites = Limites {
        taille_fichier: 1 << 20,
        lignes: 100,
        colonnes: 10,
        longueur_ligne: 1 << 16,
        longueur_cellule: 20,
    };

    fn cellules(texte: &str) -> Vec<Vec<String>> {
        lire(texte, ',', Some('"'), &LIMITES)
            .unwrap()
            .into_iter()
            .map(|e| e.cellules)
            .collect()
    }

    #[test]
    fn comme_le_module_csv_de_python() {
        assert_eq!(cellules("a,b\r\nc,d\n"), [["a", "b"], ["c", "d"]]);
        assert_eq!(cellules("\"a,b\",\"c\"\"d\""), [["a,b", "c\"d"]]);
        assert_eq!(cellules("a\"b,\"c\"d"), [["a\"b", "cd"]]);
        assert_eq!(cellules("a\n\nb"), [vec!["a"], vec![""], vec!["b"]]);
        assert_eq!(cellules("a\rb"), [["a"], ["b"]]);
        assert_eq!(cellules(",\n"), [["", ""]]);
        assert_eq!(cellules(""), Vec::<Vec<String>>::new());
    }

    #[test]
    fn fin_de_ligne_entre_guillemets_et_numeros_de_ligne() {
        let e = lire("x\n\"a\r\nb\",c\ny\n", ',', Some('"'), &LIMITES).unwrap();
        assert_eq!(e[1].cellules, ["a\r\nb", "c"]);
        assert_eq!(e[1].brut, "\"a\r\nb\",c");
        assert_eq!(e.iter().map(|e| e.ligne).collect::<Vec<_>>(), [1, 2, 4]);
    }

    #[test]
    fn sans_guillemet_tout_est_litteral() {
        let e = lire("\"a\";b", ';', None, &LIMITES).unwrap();
        assert_eq!(e[0].cellules, ["\"a\"", "b"]);
    }

    #[test]
    fn limites_appliquees_pendant_la_lecture() {
        assert_eq!(
            lire("a\nb\n\"c", ',', Some('"'), &LIMITES).unwrap_err(),
            ErreurReleve::GuillemetNonFerme { ligne: 3 }
        );
        assert_eq!(
            lire(&"a,".repeat(10), ',', None, &LIMITES).unwrap_err(),
            ErreurReleve::TropDeColonnes {
                ligne: 1,
                limite: 10
            }
        );
        assert_eq!(
            lire(&format!("a\nb,{}", "x".repeat(21)), ',', None, &LIMITES).unwrap_err(),
            ErreurReleve::CelluleTropLongue {
                ligne: 2,
                colonne: 1,
                limite: 20
            }
        );
        assert_eq!(
            lire(&"a\n".repeat(101), ',', None, &LIMITES).unwrap_err(),
            ErreurReleve::TropDeLignes {
                ligne: 101,
                limite: 100
            }
        );
        assert_eq!(
            lire(&"a\n".repeat(100), ',', None, &LIMITES).unwrap().len(),
            100
        );
    }
}
