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
// Espaces de groupement admis à la saisie, quel que soit le locale : espace, U+00A0 (insécable),
// U+202F (fine insécable, celle de `fr`).
const ESPACES = /[\s\u00a0\u202f]/g;
const FORME = new RegExp(`^(\\d{1,${CHIFFRES_ENTIERS_MAX}})(?:\\.(\\d{1,${ECHELLE}}))?$`);

/**
 * Montant saisi en chaîne canonique (`1500.5`), ou `null` si ce n'est pas un nombre décimal
 * positif ou nul. La chaîne vide donne `null` : c'est l'absence de montant.
 *
 * La saisie se lit selon le **locale du dossier**, comme le font Tryton (`sao`, `atof` : espaces
 * retirés, séparateur de milliers retiré, séparateur décimal remplacé par `.`) et Odoo : aucune
 * règle d'ambiguïté en plus. En `fr` (virgule décimale, espace fine pour les milliers) `1 500,50`
 * vaut 1500,5 et `1.500` vaut 1,5 ; en `en` (point décimal, virgule pour les milliers) `1,500.50`
 * vaut 1500,5 et `1,500` vaut 1500 ; en `pt` et `es` (point pour les milliers) `1.500,50` vaut
 * 1500,5 et `1.500` vaut 1500. Les séparateurs de milliers ne sont admis que dans la partie
 * entière ; le point reste lisible comme décimale (pavé numérique) quand il n'est ni le
 * séparateur décimal ni celui des milliers du locale. Un montant qui ne se lit pas ainsi est
 * refusé ; le serveur contrôle le reste (signe, bornes, échelle de la devise).
 */
export function lireMontant(saisie: string, r: Regional): string | null {
	const { decimal, groupe } = separateurs(localeDe(r));
	const texte = saisie.trim().replace(ESPACES, '');
	let sep = decimal;
	if (!texte.includes(decimal) && decimal !== '.' && groupe !== '.' && texte.includes('.')) {
		sep = '.';
	}
	const i = texte.indexOf(sep);
	const entier = i < 0 ? texte : texte.slice(0, i);
	const fraction = i < 0 ? '' : texte.slice(i + sep.length);
	const sansGroupes = /\s/.test(groupe) ? entier : entier.replaceAll(groupe, '');
	return lireCanonique(i < 0 ? sansGroupes : `${sansGroupes}.${fraction}`);
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
