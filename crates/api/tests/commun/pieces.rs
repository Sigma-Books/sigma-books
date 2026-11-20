//! Stockage des pièces **en mémoire**, pour les tests sans service S3 : même contrat que le
//! stockage S3 (clé jamais écrasée, version rendue, lecture d'une version bornée). Les tests
//! contre un vrai service S3 avec verrouillage d'objet sont dans `tests/pieces_s3.rs`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use api::pieces::stockage::Futur;
use api::pieces::{
    CleObjet, Depot, ErreurStockage, ObjetADeposer, Pieces, ReglagesPieces, Stockage,
};
use uuid::Uuid;

/// Objets par (clé, version).
type Objets = HashMap<(String, String), Vec<u8>>;

/// Stockage en mémoire, partagé entre le test et l'application.
#[derive(Debug, Clone, Default)]
pub struct StockageMemoire(pub Arc<Mutex<Objets>>);

impl StockageMemoire {
    /// Remplace le contenu de tous les objets dont la clé se termine par `suffixe` (altération
    /// « à la main »). Rend le nombre d'objets altérés.
    pub fn alterer(&self, cle: &str, contenu: &[u8]) -> usize {
        let mut objets = self.0.lock().expect("objets");
        let mut n = 0;
        for ((c, _), v) in objets.iter_mut() {
            if c == cle {
                *v = contenu.to_vec();
                n += 1;
            }
        }
        n
    }

    /// Retire tous les objets de clé `cle`.
    pub fn supprimer(&self, cle: &str) {
        self.0.lock().expect("objets").retain(|(c, _), _| c != cle);
    }

    /// Nombre d'objets.
    pub fn nombre(&self) -> usize {
        self.0.lock().expect("objets").len()
    }
}

impl Stockage for StockageMemoire {
    fn deposer<'a>(&'a self, objet: ObjetADeposer<'a>) -> Futur<'a, Result<Depot, ErreurStockage>> {
        Box::pin(async move {
            let mut objets = self.0.lock().expect("objets");
            if objets.keys().any(|(c, _)| c == objet.cle.as_str()) {
                return Err(ErreurStockage::Refus(412));
            }
            let version = Uuid::new_v4().to_string();
            objets.insert(
                (objet.cle.as_str().to_owned(), version.clone()),
                objet.contenu.to_vec(),
            );
            Ok(Depot { version })
        })
    }

    fn lire<'a>(
        &'a self,
        cle: &'a CleObjet,
        version: &'a str,
        taille_max: usize,
    ) -> Futur<'a, Result<Vec<u8>, ErreurStockage>> {
        Box::pin(async move {
            let objets = self.0.lock().expect("objets");
            let contenu = objets
                .get(&(cle.as_str().to_owned(), version.to_owned()))
                .ok_or(ErreurStockage::Absent)?;
            if contenu.len() > taille_max {
                return Err(ErreurStockage::TropGrand);
            }
            Ok(contenu.clone())
        })
    }
}

/// Service des pièces en mémoire, avec ses réglages par défaut.
pub fn pieces_memoire(stockage: &StockageMemoire) -> Pieces {
    Pieces::new(Arc::new(stockage.clone()), ReglagesPieces::default())
}
