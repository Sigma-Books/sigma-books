//! Journal d'audit chaîné (ADR 0014, ADR 0016) : format canonique **audit v1** d'une entrée,
//! chaîne SHA-256 par portée (cabinet ou dossier), vérification et export d'audit.
//!
//! Module pur, comme le reste du crate : aucune entrée-sortie hors des flux qu'on lui passe,
//! aucune horloge (l'instant d'une entrée est fourni par `api`), aucun aléa. La persistance
//! (tables, têtes verrouillées, transaction de l'action) vit dans le crate `api`.
//!
//! Format décrit dans `docs/integrite/audit-v1.md`, figé et vérifié par les vecteurs de
//! `tests/vecteurs/audit-v1/`.

mod chaine;
mod entree;
mod export;

pub use chaine::{AnomalieAudit, ChaineAudit, ErreurAjout, RapportAudit, Verificateur, genese};
pub use entree::{
    Acteur, Champ, DecisionCedar, ENTETE, ENTREE_MAX, EntreeAudit, ErreurAudit, Evenement, FIN,
    Portee, Ressource, SYMBOLE_MAX, SYSTEME, Symbole, TEXTE_MAX, Texte, Uuid, formater_instant,
};
pub use export::{
    ANNEXE, AttentesAudit, ENTETE_EXPORT_AUDIT, EcrivainExportAudit, FIN_ANNEXE,
    RapportExportAudit, verifier_export_audit,
};
