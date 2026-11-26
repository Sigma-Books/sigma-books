// Avertissements d'une validation réussie (`crates/api/src/serveur/routes/brouillard/validation.rs`,
// `VueAvertissement`) : l'écriture est validée malgré tout, la réponse porte `avertissements`, une
// liste d'objets `{ code, periode?, echeance?, journal?, compte? }` (validation unitaire, par lot,
// contre-passation). L'interface les présente sans les bloquer. Module pur, testé sous `node --test`.
//
// Lecture défensive : une validation réussie ne doit jamais devenir illisible à cause d'un
// avertissement. Un élément mal formé est écarté, un code inconnu est affiché en termes généraux.

import { estDateIso, afficherDate } from '../dates.ts';
import { chercher, traduire } from '../i18n/index.ts';
import type { Regional } from '../regional.ts';
import { estCodeJournal } from './types.ts';

/** Avertissement lu dans une réponse du serveur. */
export interface Avertissement {
	/** Code stable (`echeance_de_cloture_depassee`, `compte_tresorerie_absent`…). */
	code: string;
	/** Premier jour de la période (`AAAA-MM-JJ`). */
	periode?: string;
	/** Échéance de clôture (`AAAA-MM-JJ`). */
	echeance?: string;
	journal?: string;
	compte?: string;
}

const MOTIF_CODE = /^[a-z0-9_]{1,64}$/;
const MOTIF_COMPTE = /^[1-9][0-9A-Za-z-]{0,31}$/;
/** Nombre d'avertissements affichés au plus (le reste est compté). */
export const AVERTISSEMENTS_AFFICHES_MAX = 10;

/** Avertissements d'une réponse (`avertissements` du serveur) : valeur inconnue ou absente → `[]`. */
export function lireAvertissements(brut: unknown): Avertissement[] {
	if (!Array.isArray(brut)) return [];
	const lus: Avertissement[] = [];
	for (const e of brut.slice(0, 1000)) {
		if (typeof e !== 'object' || e === null) continue;
		const o = e as Record<string, unknown>;
		if (typeof o.code !== 'string' || !MOTIF_CODE.test(o.code)) continue;
		const a: Avertissement = { code: o.code };
		if (typeof o.periode === 'string' && estDateIso(o.periode)) a.periode = o.periode;
		if (typeof o.echeance === 'string' && estDateIso(o.echeance)) a.echeance = o.echeance;
		if (typeof o.journal === 'string' && estCodeJournal(o.journal)) a.journal = o.journal;
		if (typeof o.compte === 'string' && MOTIF_COMPTE.test(o.compte)) a.compte = o.compte;
		lus.push(a);
	}
	return lus;
}

/**
 * Message d'un avertissement, dates dans le locale du dossier. Un code connu dont un détail manque,
 * ou un code inconnu, donne le message général : jamais de valeur du serveur citée sans contrôle.
 */
export function messageAvertissement(a: Avertissement, r: Regional): string {
	const general = traduire('avertissement.inconnu', undefined, r.langue);
	if (a.code === 'echeance_de_cloture_depassee') {
		if (a.periode === undefined || a.echeance === undefined) return general;
		return (
			chercher(
				`avertissement.${a.code}`,
				{ periode: afficherDate(a.periode, r), echeance: afficherDate(a.echeance, r) },
				r.langue
			) ?? general
		);
	}
	if (a.code === 'compte_tresorerie_absent') {
		if (a.journal === undefined || a.compte === undefined) return general;
		return (
			chercher(`avertissement.${a.code}`, { journal: a.journal, compte: a.compte }, r.langue) ??
			general
		);
	}
	return general;
}

/** Avertissements d'une écriture validée, avec son numéro (lot) ou sans (écriture unique). */
export interface AvertissementsDEcriture {
	numero: string;
	avertissements: Avertissement[];
}

export interface BandeauAvertissements {
	titre: string;
	lignes: string[];
	/** Avertissements non affichés au-delà de {@link AVERTISSEMENTS_AFFICHES_MAX} ; 0 si aucun. */
	suite: string | null;
}

/**
 * Bandeau d'information à afficher après un succès, ou `null` s'il n'y a aucun avertissement. Pour
 * une seule écriture, les messages sont donnés tels quels ; pour un lot, chacun est précédé du
 * numéro de l'écriture.
 */
export function bandeauAvertissements(
	ecritures: readonly AvertissementsDEcriture[],
	r: Regional,
	lot: boolean
): BandeauAvertissements | null {
	const tous = ecritures.flatMap((e) =>
		e.avertissements.map((a) => {
			const message = messageAvertissement(a, r);
			return lot
				? traduire('avertissement.ecriture', { numero: e.numero, message }, r.langue)
				: message;
		})
	);
	if (tous.length === 0) return null;
	const reste = tous.length - AVERTISSEMENTS_AFFICHES_MAX;
	return {
		titre: traduire(
			lot ? 'avertissement.titre.lot' : 'avertissement.titre.une',
			undefined,
			r.langue
		),
		lignes: tous.slice(0, AVERTISSEMENTS_AFFICHES_MAX),
		suite: reste > 0 ? traduire('avertissement.suite', { nombre: reste }, r.langue) : null
	};
}
