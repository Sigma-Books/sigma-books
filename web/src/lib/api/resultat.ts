// Résultat d'un chargement, pour qu'une page affiche l'erreur de l'API à sa place au lieu de
// basculer sur la page d'erreur générale.

import { ErreurApi } from './erreurs.ts';

export type Resultat<T> = { ok: true; valeur: T } | { ok: false; erreur: ErreurApi };

/** Exécute `travail` ; une {@link ErreurApi} devient un résultat, toute autre erreur remonte. */
export async function tenter<T>(travail: () => Promise<T>): Promise<Resultat<T>> {
	try {
		return { ok: true, valeur: await travail() };
	} catch (erreur) {
		if (erreur instanceof ErreurApi) return { ok: false, erreur };
		throw erreur;
	}
}
