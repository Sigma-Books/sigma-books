// Briques communes des gardes de réponse (types, `types.ts` pour les gardes de base).

import { lireUuid, type Garde } from './types.ts';

export type Objet = Record<string, unknown>;

export function estObjet(valeur: unknown): valeur is Objet {
	return typeof valeur === 'object' && valeur !== null && !Array.isArray(valeur);
}

export function chaines(o: Objet, champs: readonly string[]): boolean {
	return champs.every((c) => typeof o[c] === 'string');
}

/** Absent, ou chaîne. */
export function chaineFacultative(o: Objet, champ: string): boolean {
	return o[champ] === undefined || typeof o[champ] === 'string';
}

export function estId(valeur: unknown): boolean {
	return typeof valeur === 'string' && lireUuid(valeur) === valeur;
}

export function entier(valeur: unknown): valeur is number {
	return typeof valeur === 'number' && Number.isSafeInteger(valeur);
}

export function tableauDe<T>(valeur: unknown, garde: Garde<T>): valeur is T[] {
	return Array.isArray(valeur) && valeur.every(garde);
}

/** Curseur de pagination : chaîne de forme sûre, ou `null` en fin de liste. */
export function curseurOuNul(valeur: unknown): valeur is string | null | undefined {
	return (
		valeur === null ||
		valeur === undefined ||
		(typeof valeur === 'string' && /^[0-9A-Za-z_-]{1,80}$/.test(valeur))
	);
}
