// Instants rendus par l'API (`AAAA-MM-JJTHH:MM:SS.ffffffZ`, format de l'audit) et leur affichage.
//
// Les microsecondes dépassent la forme garantie par `Date.parse` (trois chiffres) : l'analyse est
// faite ici, tronquée à la milliseconde, et refuse toute autre forme.

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

const DATE_HEURE = new Intl.DateTimeFormat('fr-FR', { dateStyle: 'short', timeStyle: 'short' });
const HEURE = new Intl.DateTimeFormat('fr-FR', { hour: '2-digit', minute: '2-digit' });

/** Date et heure locales (« 19/11/2026 14:05 »), ou « — » si l'instant est illisible. */
export function dateHeure(texte: string): string {
	const date = lireInstant(texte);
	return date === null ? '—' : DATE_HEURE.format(date);
}

/** Heure locale (« 14:05 »), ou « — ». */
export function heure(texte: string): string {
	const date = lireInstant(texte);
	return date === null ? '—' : HEURE.format(date);
}

/** Minutes entières restantes avant l'instant (0 s'il est passé), ou `null` s'il est illisible. */
export function minutesRestantes(texte: string, maintenant: number): number | null {
	const date = lireInstant(texte);
	if (date === null) return null;
	return Math.max(0, Math.floor((date.getTime() - maintenant) / 60_000));
}
