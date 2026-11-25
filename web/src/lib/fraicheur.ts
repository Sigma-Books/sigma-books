// Action sensible : session fraîche exigée par le serveur (ADR 0012, F1.6).
//
// L'API ne distingue pas, dans sa réponse, « session non fraîche » d'un autre refus (403 `interdit`,
// sans motif : ni la décision Cedar ni sa cause ne sortent du serveur). Si la fraîcheur expire entre
// la vérification et l'action, l'interface relit donc l'état de la session : seulement si elle n'est
// plus fraîche, elle demande une ré-authentification et rejoue l'action **une seule fois**. Un autre
// 403, ou un second échec, est affiché tel quel. Module pur (testé sous `node --test`).

import { ErreurApi } from './api/erreurs.ts';

export interface DependancesFraicheur {
	/** Vrai si la session est fraîche, ou si l'utilisateur vient de se ré-authentifier. */
	assurer: () => Promise<boolean>;
	/** Vrai si la session est fraîche d'après le serveur (relecture de son état). */
	estFraiche: () => Promise<boolean>;
}

export type IssueSensible<T> = { fait: true; valeur: T } | { fait: false };

export async function executerSensible<T>(
	travail: () => Promise<T>,
	{ assurer, estFraiche }: DependancesFraicheur
): Promise<IssueSensible<T>> {
	if (!(await assurer())) return { fait: false };
	try {
		return { fait: true, valeur: await travail() };
	} catch (erreur) {
		const refus = erreur instanceof ErreurApi && erreur.statut === 403;
		if (!refus || (await estFraiche())) throw erreur;
		// La fraîcheur a expiré entre-temps : une ré-authentification, un seul rejeu.
		if (!(await assurer())) return { fait: false };
		return { fait: true, valeur: await travail() };
	}
}
