// Montants en décimal exact, sous forme de chaînes (Règles absolues 2 et 4 : jamais de flottant).
//
// Un montant circule toujours comme une chaîne canonique (`1500.5`, sans zéro inutile) : celle que
// le serveur attend et renvoie. Les sommes de l'interface sont indicatives (l'équilibre fait foi
// côté serveur, à la validation) ; elles se font en entiers `BigInt` à l'échelle `ECHELLE`, jamais
// avec `number`. Aucun import de SvelteKit : testé par `node --test`.

import { localeDe, separateurs, type Regional } from './regional.ts';

/** Nombre maximal de décimales acceptées à la frappe. */
export const ECHELLE = 6;
/** Nombre maximal de chiffres entiers (borne de la base : strictement moins de 1e20). */
export const CHIFFRES_ENTIERS_MAX = 20;

const FACTEUR = 10n ** BigInt(ECHELLE);
const FORME = new RegExp(`^(\\d{1,${CHIFFRES_ENTIERS_MAX}})(?:\\.(\\d{1,${ECHELLE}}))?$`);

export type LectureMontant =
	{ ok: true; montant: string } | { ok: false; motif: 'illisible' | 'trop_de_decimales' };

const ECHAPPEMENT = /[.*+?^${}()|[\]\\]/g;
const GROUPE_DE_TROIS = /^\d{3}$/;
const GROUPE_INITIAL = /^\d{1,3}$/;

/**
 * Lit une saisie selon le **locale du dossier** et l'**échelle de sa devise**, et dit pourquoi elle
 * est refusée : aucune lecture douteuse n'est jamais devinée, ni tronquée en silence (un montant
 * faux mais équilibré passerait le contrôle du serveur).
 *
 * Comme Tryton (`sao`, `atof`) et Odoo : espaces retirés, séparateur de milliers retiré, séparateur
 * décimal remplacé par `.`, sans règle d'ambiguïté. En plus, deux garde-fous de lecture :
 * - le séparateur de milliers (une espace, ou le séparateur du locale : `,` en `en`, `.` en `pt` et
 *   `es`) n'est admis que dans la partie entière, entre des groupes d'**exactement trois chiffres**
 *   (`1,500` en `en`, pas `1500,50` ni `12,5`) ;
 * - la partie décimale ne dépasse pas l'échelle de la devise (`regional.decimales`) : en XOF, XAF,
 *   KMF et GNF (échelle 0) `1,500` et `1.500` sont refusés plutôt que lus 1,5 ; en CDF (échelle 2)
 *   `1,500` aussi. Le point reste une décimale (pavé numérique) quand il n'est ni le séparateur
 *   décimal ni celui des milliers du locale.
 */
export function diagnostiquerMontant(saisie: string, r: Regional): LectureMontant {
	const illisible = { ok: false, motif: 'illisible' } as const;
	const { decimal, groupe } = separateurs(localeDe(r));
	const texte = saisie.trim();
	let sep = decimal;
	if (!texte.includes(decimal) && decimal !== '.' && groupe !== '.' && texte.includes('.')) {
		sep = '.';
	}
	const i = texte.indexOf(sep);
	const entier = i < 0 ? texte : texte.slice(0, i);
	const fraction = i < 0 ? '' : texte.slice(i + sep.length);
	if (i >= 0 && !/^\d{1,6}$/.test(fraction)) return illisible;
	const separateurGroupe = /\s/.test(groupe)
		? /[\s\u00a0\u202f]+/
		: new RegExp(`[\\s\\u00a0\\u202f]+|${groupe.replace(ECHAPPEMENT, '\\$&')}`);
	const groupes = entier.split(separateurGroupe);
	if (groupes.length > 1) {
		const [premier = '', ...suite] = groupes;
		if (!GROUPE_INITIAL.test(premier) || !suite.every((g) => GROUPE_DE_TROIS.test(g))) {
			return illisible;
		}
	}
	if (fraction.length > r.decimales) return { ok: false, motif: 'trop_de_decimales' };
	const montant = lireCanonique(i < 0 ? groupes.join('') : `${groupes.join('')}.${fraction}`);
	return montant === null ? illisible : { ok: true, montant };
}

/** Montant saisi en chaîne canonique (`1500.5`), ou `null` : voir {@link diagnostiquerMontant}. */
export function lireMontant(saisie: string, r: Regional): string | null {
	const lecture = diagnostiquerMontant(saisie, r);
	return lecture.ok ? lecture.montant : null;
}

/** Chaîne canonique (`1500.5`) lue strictement, normalisée (zéros inutiles retirés), ou `null`. */
export function lireCanonique(texte: string): string | null {
	const m = FORME.exec(texte);
	if (m === null) return null;
	const entier = (m[1] ?? '').replace(/^0+(?=\d)/, '');
	const decimales = (m[2] ?? '').replace(/0+$/, '');
	return decimales === '' ? entier : `${entier}.${decimales}`;
}

/** Montant canonique en entiers à l'échelle {@link ECHELLE}. */
export function versUnites(canonique: string): bigint {
	const [entier = '0', decimales = ''] = canonique.split('.');
	return BigInt(entier) * FACTEUR + BigInt(decimales.padEnd(ECHELLE, '0'));
}

/** Entiers à l'échelle {@link ECHELLE} en chaîne canonique. */
export function depuisUnites(unites: bigint): string {
	const negatif = unites < 0n;
	const absolu = negatif ? -unites : unites;
	const entier = (absolu / FACTEUR).toString();
	const decimales = (absolu % FACTEUR).toString().padStart(ECHELLE, '0').replace(/0+$/, '');
	return `${negatif ? '-' : ''}${entier}${decimales === '' ? '' : `.${decimales}`}`;
}

/** Vrai si la chaîne canonique vaut zéro. */
export function estNul(canonique: string): boolean {
	return versUnites(canonique) === 0n;
}

/** Lecture d'un montant : saisie d'un locale, ou chaîne canonique (défaut). */
export type Lecture = (texte: string) => string | null;

/** Somme de montants (les valeurs illisibles sont ignorées), lus par `lire` (canoniques par défaut). */
export function additionner(montants: readonly string[], lire: Lecture = lireCanonique): string {
	let total = 0n;
	for (const m of montants) {
		const lu = lire(m);
		if (lu !== null) total += versUnites(lu);
	}
	return depuisUnites(total);
}

export type SensEcart = 'equilibre' | 'debit' | 'credit';

export interface Ecart {
	debit: string;
	credit: string;
	/** Valeur absolue de la différence. */
	difference: string;
	/** Sens à porter par une ligne pour équilibrer : `debit` si les crédits dépassent. */
	sens: SensEcart;
}

/** Équilibre indicatif d'une saisie (le serveur recalcule et décide à la validation). */
export function ecart(
	debits: readonly string[],
	credits: readonly string[],
	lire: Lecture = lireCanonique
): Ecart {
	const d = versUnites(additionner(debits, lire));
	const c = versUnites(additionner(credits, lire));
	const diff = c - d;
	return {
		debit: depuisUnites(d),
		credit: depuisUnites(c),
		difference: depuisUnites(diff < 0n ? -diff : diff),
		sens: diff === 0n ? 'equilibre' : diff > 0n ? 'debit' : 'credit'
	};
}

/**
 * Nombre canonique mis en forme : au moins `decimales` décimales (complétées par des zéros), jamais
 * moins que celles que le montant porte vraiment (un chiffre non nul n'est jamais arrondi ni
 * masqué), séparateur décimal `decimal` et, si `groupe` est donné, groupes de trois chiffres. Une
 * chaîne qui n'est pas un nombre canonique est rendue telle quelle. Aucun `number` : chaînes.
 */
export function formaterNombre(
	canonique: string,
	decimales: number,
	decimal: string,
	groupe: string | null
): string {
	const m = /^(-?)(\d+)(?:\.(\d+))?$/.exec(canonique);
	if (m === null) return canonique;
	const entier = m[2] ?? '';
	let fraction = m[3] ?? '';
	while (fraction.length > decimales && fraction.endsWith('0')) fraction = fraction.slice(0, -1);
	fraction = fraction.padEnd(decimales, '0');
	const groupes = groupe === null ? entier : entier.replace(/\B(?=(\d{3})+$)/g, groupe);
	return `${m[1] ?? ''}${groupes}${fraction === '' ? '' : `${decimal}${fraction}`}`;
}

/**
 * Affichage d'un montant canonique dans le format du dossier : échelle de sa devise, séparateurs de
 * son locale (espace fine insécable U+202F en `fr`, qui évite les coupures de ligne).
 */
export function formaterMontant(canonique: string, r: Regional): string {
	const { decimal, groupe } = separateurs(localeDe(r));
	return formaterNombre(canonique, r.decimales, decimal, groupe);
}

/** Espace insécable entre le montant et le code de la devise. */
const ESPACE_INSECABLE = '\u00a0';

/** Montant suivi du code de sa devise (`1 500 XOF`), pour les phrases et les totaux isolés. */
export function formaterMontantDevise(canonique: string, r: Regional): string {
	return `${formaterMontant(canonique, r)}${ESPACE_INSECABLE}${r.devise}`;
}

/** Montant canonique tel qu'on le saisit dans le locale du dossier (`1500,5` en `fr`), sans groupes. */
export function versSaisie(canonique: string, r: Regional): string {
	const { decimal } = separateurs(localeDe(r));
	return canonique.replace('.', decimal);
}
