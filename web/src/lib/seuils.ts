// Seuils du serveur que l'interface cite dans ses phrases. L'API ne les renvoie pas (elle ne
// renvoie que le résultat : `horodatage.retard`, la liste des échéances) : ils sont donc recopiés
// ici, à un seul endroit, avec leur origine. Ils ne servent qu'au texte affiché, jamais à une
// décision : le serveur décide, et si l'un change côté serveur, seul le texte devient inexact.
// Quand l'API les exposera, ce fichier sera remplacé par leur lecture.

/** Retard d'horodatage signalé après ce délai : `horodatage en retard de plus de 24 h`, `crates/api/src/horodatage/tache.rs`. */
export const DELAI_RETARD_HORODATAGE_HEURES = 24;

/** Préavis des échéances de clôture de l'écran des alertes : `PREAVIS_ECHEANCE`, `crates/api/src/serveur/routes/editions.rs`. */
export const PREAVIS_ECHEANCE_JOURS = 30;
