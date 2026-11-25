// Montants en décimal exact, sous forme de chaînes (Règles absolues 2 et 4 : jamais de flottant).
//
// Un montant circule toujours comme une chaîne canonique (`1500.5`, sans zéro inutile) : celle que
// le serveur attend et renvoie. Les sommes de l'interface sont indicatives (l'équilibre fait foi
// côté serveur, à la validation) ; elles se font en entiers `BigInt` à l'échelle `ECHELLE`, jamais
// avec `number`. Aucun import de SvelteKit : testé par `node --test`.

/** Nombre maximal de décimales acceptées à la frappe. */
export const ECHELLE = 6;
/** Nombre maximal de chiffres entiers (borne de la base : strictement moins de 1e20). */
export const CHIFFRES_ENTIERS_MAX = 20;

const FACTEUR = 10n ** BigInt(ECHELLE);
// Espaces de groupement admis à la saisie : espace, U+00A0 (insécable), U+202F (fine insécable).
const ESPACES = /[ \u00a0\u202f]/g;
const FORME = new RegExp(`^(\\d{1,${CHIFFRES_ENTIERS_MAX}})(?:[.,](\\d{1,${ECHELLE}}))?$`);

/**
 * Montant saisi (`1 500,50`, `1500.5`) en chaîne canonique (`1500.5`), ou `null` si ce n'est pas
 * un nombre décimal positif ou nul. La chaîne vide donne `null` : c'est l'absence de montant.
 */
export function lireMontant(saisie: string): string | null {
	const m = FORME.exec(saisie.trim().replace(ESPACES, ''));
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

/** Somme de montants canoniques (les valeurs illisibles sont ignorées). */
export function additionner(montants: readonly string[]): string {
	let total = 0n;
	for (const m of montants) {
		const lu = lireMontant(m);
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
export function ecart(debits: readonly string[], credits: readonly string[]): Ecart {
	const d = versUnites(additionner(debits));
	const c = versUnites(additionner(credits));
	const diff = c - d;
	return {
		debit: depuisUnites(d),
		credit: depuisUnites(c),
		difference: depuisUnites(diff < 0n ? -diff : diff),
		sens: diff === 0n ? 'equilibre' : diff > 0n ? 'debit' : 'credit'
	};
}

/** Affichage français d'un montant canonique : groupes de milliers, virgule décimale. */
export function formaterMontant(canonique: string): string {
	const m = /^(-?)(\d+)(?:\.(\d+))?$/.exec(canonique);
	if (m === null) return canonique;
	const groupes = (m[2] ?? '').replace(/\B(?=(\d{3})+$)/g, ' ');
	return `${m[1] ?? ''}${groupes}${m[3] === undefined ? '' : `,${m[3]}`}`;
}
