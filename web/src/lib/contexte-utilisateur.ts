// Contexte des écrans hors dossier (méthodes de connexion, sessions, administration du cabinet) :
// langue et fuseau du navigateur. Aller sur un de ces écrans ouvre une nouvelle navigation, de sorte
// qu'un chargement de dossier encore en cours ne puisse plus changer la langue.

import { fixerLangue, nouvelleNavigation } from './i18n/index.ts';
import { regionalUtilisateur, type Affichage } from './regional.ts';

/** Adopte la langue du navigateur pour cet écran et rend le contexte régional de l'utilisateur. */
export function adopterRegionalUtilisateur(): Affichage {
	const regional = regionalUtilisateur();
	fixerLangue(regional.langue, nouvelleNavigation());
	return regional;
}
