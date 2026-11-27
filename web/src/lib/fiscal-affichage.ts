// Affichage des paramètres fiscaux : taux dans le format du dossier, auteur d'une version. Module
// pur, testé sous `node --test`. Les taux sont des chaînes décimales exactes : jamais de `number`.

import { localeDe, separateurs, type Regional } from './regional.ts';

/**
 * Taux décimal exact (`7.5`) écrit avec le séparateur décimal du locale du dossier (`7,5` en
 * `fr`). La chaîne n'est jamais convertie en nombre ; une forme inattendue est rendue telle quelle.
 */
export function afficherTaux(
	valeur: string,
	r: Pick<Regional, 'langue'> & { pays?: string }
): string {
	if (!/^\d{1,3}(\.\d{1,6})?$/.test(valeur)) return valeur;
	return valeur.replace('.', separateurs(localeDe(r)).decimal);
}

/** Longueur de l'identifiant abrégé d'un auteur. */
const ABREGE = 8;

/** Auteur d'une version : son nom s'il est connu, sinon son identifiant abrégé. */
export function libelleAuteur(id: string, noms: ReadonlyMap<string, string>): string {
	return noms.get(id) ?? (id.length > ABREGE ? `${id.slice(0, ABREGE)}…` : id);
}
