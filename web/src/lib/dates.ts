// Dates de l'interface : `AAAA-MM-JJ`, la forme de l'API. Aucun calcul comptable ici.

import { localeDe, type Regional } from './regional.ts';

const FORME = /^(\d{4})-(\d{2})-(\d{2})$/;

/**
 * Date du jour au format `AAAA-MM-JJ` **dans le fuseau du dossier** (pour proposer « aujourd'hui » :
 * à minuit, la date du navigateur peut différer de celle de l'entité). `maintenant` est passé en
 * paramètre pour les tests.
 */
export function dateDuJour(fuseau: string, maintenant: Date = new Date()): string {
	const parties = new Intl.DateTimeFormat('en-CA', {
		timeZone: fuseau,
		year: 'numeric',
		month: '2-digit',
		day: '2-digit'
	}).formatToParts(maintenant);
	const v = (type: string) => parties.find((p) => p.type === type)?.value ?? '';
	return `${v('year').padStart(4, '0')}-${v('month')}-${v('day')}`;
}

/** Vrai si le texte est une date `AAAA-MM-JJ` du calendrier (jour existant). */
export function estDateIso(texte: string): boolean {
	const m = FORME.exec(texte);
	if (m === null) return false;
	const [annee, mois, jour] = [Number(m[1]), Number(m[2]), Number(m[3])];
	const d = new Date(Date.UTC(annee, mois - 1, jour));
	return d.getUTCFullYear() === annee && d.getUTCMonth() === mois - 1 && d.getUTCDate() === jour;
}

const FORMATS = new Map<string, Intl.DateTimeFormat>();

/**
 * `AAAA-MM-JJ` dans l'usage du locale du dossier (`19/11/2026` en `fr-SN`, `11/19/2026` en `en`),
 * année sur quatre chiffres ; toute autre forme est rendue telle quelle. Une date de calendrier n'a
 * pas de fuseau : le calcul est fait en UTC pour ne jamais la décaler d'un jour.
 */
export function afficherDate(
	texte: string,
	r: Pick<Regional, 'langue'> & { pays?: string }
): string {
	const m = FORME.exec(texte);
	if (m === null || !estDateIso(texte)) return texte;
	const locale = localeDe(r);
	let format = FORMATS.get(locale);
	if (format === undefined) {
		format = new Intl.DateTimeFormat(locale, {
			timeZone: 'UTC',
			day: '2-digit',
			month: '2-digit',
			year: 'numeric'
		});
		FORMATS.set(locale, format);
	}
	return format.format(new Date(Date.UTC(Number(m[1]), Number(m[2]) - 1, Number(m[3]))));
}
