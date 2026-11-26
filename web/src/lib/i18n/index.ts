// Mécanisme minimal de langue : un catalogue de messages par langue, le français par défaut.
//
// - `traduire(cle, params)` rend le message de la langue courante, ou celui du français si cette
//   langue ne l'a pas traduit ; `{nom}` est remplacé par le paramètre du même nom.
// - La langue courante est celle du dernier dossier chargé (`fixerLangue`, appelée par
//   `chargerDossier`) ; avant tout dossier, le français. Aucune dépendance : pas de bibliothèque d'i18n.
// Module pur, testé sous `node --test`.

import type { Langue } from '../regional.ts';
import { en } from './en.ts';
import { es } from './es.ts';
import { fr, type Cle } from './fr.ts';
import { pt } from './pt.ts';

export type { Cle } from './fr.ts';

export type Parametres = Readonly<Record<string, string | number>>;

type Catalogue = Readonly<Partial<Record<Cle, string>>>;

/** Catalogues par langue (le français est complet, les autres retombent sur lui). */
export const CATALOGUES: Readonly<Record<Langue, Catalogue>> = { fr, en, pt, es };

let courante: Langue = 'fr';

/** Fixe la langue des messages (celle du dossier affiché). */
export function fixerLangue(langue: Langue): void {
	courante = langue;
}

export function langueCourante(): Langue {
	return courante;
}

function remplir(modele: string, params: Parametres | undefined): string {
	if (params === undefined) return modele;
	return modele.replace(/\{(\w+)\}/g, (brut, nom: string) =>
		Object.hasOwn(params, nom) ? String(params[nom]) : brut
	);
}

function modeleDe(cle: string, langue: Langue): string | null {
	const propre = CATALOGUES[langue];
	if (Object.hasOwn(propre, cle)) return (propre as Record<string, string>)[cle] ?? null;
	return Object.hasOwn(fr, cle) ? ((fr as Record<string, string>)[cle] ?? null) : null;
}

/** Message d'une clé connue, dans la langue demandée (courante par défaut). */
export function traduire(cle: Cle, params?: Parametres, langue: Langue = courante): string {
	return remplir(modeleDe(cle, langue) ?? cle, params);
}

/**
 * Message d'une clé construite à l'exécution (code du serveur, valeur codée), ou `null` si le
 * catalogue ne la connaît pas : l'appelant décide alors de l'affichage (valeur brute, message
 * général).
 */
export function chercher(
	cle: string,
	params?: Parametres,
	langue: Langue = courante
): string | null {
	const modele = modeleDe(cle, langue);
	return modele === null ? null : remplir(modele, params);
}

/** `3 jours`, `1 jour` : l'unité s'accorde selon les règles de la langue (`Intl.PluralRules`). */
export function nombreEtUnite(
	nombre: number,
	unite: 'jour' | 'brouillon',
	langue: Langue = courante
): string {
	const categorie =
		new Intl.PluralRules(langue).select(Math.abs(nombre)) === 'one' ? 'one' : 'other';
	return `${nombre} ${traduire(`unite.${unite}.${categorie}`, undefined, langue)}`;
}
