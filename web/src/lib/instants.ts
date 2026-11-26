// Instants rendus par l'API (`AAAA-MM-JJTHH:MM:SS.ffffffZ`, format de l'audit) et leur affichage.
//
// Les microsecondes dépassent la forme garantie par `Date.parse` (trois chiffres) : l'analyse est
// faite ici, tronquée à la milliseconde, et refuse toute autre forme.

import { localeDe, type Regional } from './regional.ts';

const MOTIF = /^(\d{4})-(\d{2})-(\d{2})T(\d{2}):(\d{2}):(\d{2})(?:\.(\d{1,9}))?Z$/;

/** Instant UTC lu, ou `null` si la forme ou la date est invalide. */
export function lireInstant(texte: string): Date | null {
	const m = MOTIF.exec(texte);
	if (m === null) return null;
	const [an, mois, jour, heure, minute, seconde] = m.slice(1, 7).map(Number) as [
		number,
		number,
		number,
		number,
		number,
		number
	];
	const millis = Number((m[7] ?? '0').padEnd(3, '0').slice(0, 3));
	const date = new Date(Date.UTC(an, mois - 1, jour, heure, minute, seconde, millis));
	const coherente =
		date.getUTCFullYear() === an &&
		date.getUTCMonth() === mois - 1 &&
		date.getUTCDate() === jour &&
		date.getUTCHours() === heure &&
		date.getUTCMinutes() === minute &&
		date.getUTCSeconds() === seconde;
	return coherente ? date : null;
}

const FORMATS = new Map<string, Intl.DateTimeFormat>();

function format(r: Regional, options: Intl.DateTimeFormatOptions): Intl.DateTimeFormat {
	const cle = `${localeDe(r)}|${r.fuseau}|${JSON.stringify(options)}`;
	let f = FORMATS.get(cle);
	if (f === undefined) {
		f = new Intl.DateTimeFormat(localeDe(r), { timeZone: r.fuseau, ...options });
		FORMATS.set(cle, f);
	}
	return f;
}

/** Date et heure du fuseau et du locale du dossier (`19/11/2026 14:05`), ou « — » si illisible. */
export function dateHeure(texte: string, r: Regional): string {
	const date = lireInstant(texte);
	return date === null
		? '—'
		: format(r, {
				day: '2-digit',
				month: '2-digit',
				year: 'numeric',
				hour: '2-digit',
				minute: '2-digit'
			}).format(date);
}

/** Heure du fuseau du dossier (`14:05`), ou « — ». */
export function heure(texte: string, r: Regional): string {
	const date = lireInstant(texte);
	return date === null ? '—' : format(r, { hour: '2-digit', minute: '2-digit' }).format(date);
}

/** Minutes entières restantes avant l'instant (0 s'il est passé), ou `null` s'il est illisible. */
export function minutesRestantes(texte: string, maintenant: number): number | null {
	const date = lireInstant(texte);
	if (date === null) return null;
	return Math.max(0, Math.floor((date.getTime() - maintenant) / 60_000));
}
