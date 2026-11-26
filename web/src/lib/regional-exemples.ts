// Exemples de contexte régional pour les tests (jamais importé par le code de l'application) :
// un dossier de référence aux valeurs explicites, que chaque test adapte champ par champ.

import type { Regional } from './regional.ts';

/** Un dossier d'exemple : les valeurs ne servent qu'aux tests, l'application n'en suppose aucune. */
export function regionalDe(surcharges: Partial<Regional> = {}): Regional {
	return {
		pays: 'SN',
		devise: 'XOF',
		decimales: 0,
		fuseau: 'Africa/Dakar',
		langue: 'fr',
		...surcharges
	};
}
