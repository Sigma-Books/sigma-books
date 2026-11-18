//! Preuve d'inaltérabilité (spec §6.4, ADR 0007) : sérialisation canonique des écritures
//! validées, chaîne de hachage SHA-256 par dossier, export d'intégrité et vérification hors
//! ligne (`sigma-verifier`), horodatage RFC 3161 de la tête de chaîne. Formats décrits dans
//! `docs/integrite/`.

#![forbid(unsafe_code)]

pub mod canonique;
pub mod chaine;
pub mod export;
pub mod horodatage;
pub mod verification;

pub use canonique::{EcritureCanonique, ErreurCanonique};
pub use chaine::{Chaine, Empreinte, ErreurChaine, IdDossier, chainer, genese};
pub use export::{
    EcrivainExport, Element, ErreurExport, ErreurLecture, JetonExporte, LecteurExport, Version,
    exporter,
};
pub use horodatage::{
    Ancre, ErreurAncre, ErreurJeton, ErreurRequete, JetonVerifie, Temoin, ancres_par_defaut,
    requete, verifier_jeton, verifier_reponse,
};
pub use verification::{Anomalie, Horodatage, Options, Plage, Rapport, verifier};
