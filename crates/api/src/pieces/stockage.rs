//! Stockage objet des pièces (ADR 0015) : le trait [`Stockage`] et la clé d'objet.
//!
//! Le binaire n'en connaît qu'une implémentation, compatible S3 ([`super::s3::StockageS3`]) ;
//! les tests sans service S3 en fournissent une en mémoire (`crates/api/tests/commun/`).

use std::fmt;
use std::future::Future;
use std::pin::Pin;

use integrite::Empreinte;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::base::{IdCabinet, IdDossier};

/// Futur rendu par un [`Stockage`].
pub type Futur<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Clé d'un objet : `<cabinet>/<dossier>/<UUID v4>`. Ni l'empreinte ni le nom de fichier n'y
/// figurent (ADR 0015) : la clé ne révèle rien du contenu et ne se devine pas.
#[derive(Clone, PartialEq, Eq)]
pub struct CleObjet(String);

impl CleObjet {
    /// Nouvelle clé, d'UUID v4 tiré au hasard, dans le dossier `dossier` du cabinet `cabinet`.
    #[must_use]
    pub fn nouvelle(cabinet: IdCabinet, dossier: IdDossier) -> Self {
        Self(format!("{cabinet}/{dossier}/{}", Uuid::new_v4()))
    }

    /// Clé relue en base, si elle a la forme exacte d'une clé de ce cabinet et de ce dossier.
    #[must_use]
    pub fn relue(texte: &str, cabinet: IdCabinet, dossier: IdDossier) -> Option<Self> {
        let reste = texte.strip_prefix(&format!("{cabinet}/{dossier}/"))?;
        let uuid = Uuid::try_parse(reste).ok()?;
        (uuid.to_string() == reste).then(|| Self(texte.to_owned()))
    }

    /// La clé.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for CleObjet {
    /// La clé n'est pas un secret, mais elle cite cabinet et dossier : jamais dans un journal.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("CleObjet(..)")
    }
}

/// Objet à déposer.
#[derive(Debug, Clone, Copy)]
pub struct ObjetADeposer<'a> {
    /// Clé neuve ([`CleObjet::nouvelle`]).
    pub cle: &'a CleObjet,
    /// Contenu, tel que reçu.
    pub contenu: &'a [u8],
    /// SHA-256 du contenu, calculée par le serveur ; le stockage la fait vérifier au service.
    pub empreinte: Empreinte,
    /// Type MIME reconnu par la signature.
    pub type_contenu: &'static str,
    /// Fin de la rétention posée sur l'objet (mode gouvernance).
    pub retenir_jusqu_au: OffsetDateTime,
}

/// Objet déposé : la version écrite, seule relue ensuite.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Depot {
    /// Identifiant de version rendu par le service (versionnement obligatoire).
    pub version: String,
}

/// Échec du stockage. Aucun message ne recopie une URL, un identifiant ni une réponse.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ErreurStockage {
    /// Service injoignable, délai dépassé ou échange interrompu.
    #[error("stockage objet injoignable")]
    Injoignable,
    /// Le service a refusé la requête (statut HTTP).
    #[error("stockage objet : requête refusée (statut {0})")]
    Refus(u16),
    /// Objet ou version absents.
    #[error("stockage objet : objet absent")]
    Absent,
    /// Objet plus grand qu'attendu.
    #[error("stockage objet : objet plus grand qu'attendu")]
    TropGrand,
    /// Réponse incomplète ou incohérente (version absente, par exemple).
    #[error("stockage objet : réponse inattendue")]
    ReponseInattendue,
    /// Versionnement absent ou suspendu sur le compartiment.
    #[error("compartiment sans versionnement activé")]
    SansVersionnement,
    /// Verrouillage d'objet absent sur le compartiment.
    #[error("compartiment sans verrouillage d'objet")]
    SansVerrouillage,
}

/// Stockage objet des pièces : écriture unique sous rétention, lecture d'une version précise.
pub trait Stockage: Send + Sync + 'static {
    /// Écrit un objet neuf sous rétention (mode gouvernance) ; refuse d'écraser une clé
    /// existante. Rend la version écrite.
    fn deposer<'a>(&'a self, objet: ObjetADeposer<'a>) -> Futur<'a, Result<Depot, ErreurStockage>>;

    /// Lit la version `version` de l'objet `cle`, au plus `taille_max` octets.
    fn lire<'a>(
        &'a self,
        cle: &'a CleObjet,
        version: &'a str,
        taille_max: usize,
    ) -> Futur<'a, Result<Vec<u8>, ErreurStockage>>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn la_cle_porte_le_tenant_et_un_uuid_neuf() {
        let (c, d) = (IdCabinet::nouveau(), IdDossier::nouveau());
        let a = CleObjet::nouvelle(c, d);
        let b = CleObjet::nouvelle(c, d);
        assert_ne!(a, b);
        assert!(a.as_str().starts_with(&format!("{c}/{d}/")));
        assert_eq!(CleObjet::relue(a.as_str(), c, d), Some(a.clone()));
        assert_eq!(CleObjet::relue(a.as_str(), c, IdDossier::nouveau()), None);
        assert_eq!(CleObjet::relue(&format!("{c}/{d}/x"), c, d), None);
        assert_eq!(
            CleObjet::relue(&format!("{c}/{d}/{}", Uuid::new_v4().simple()), c, d),
            None
        );
        assert_eq!(format!("{a:?}"), "CleObjet(..)");
    }
}
