// Synthèse de l'écran des alertes (module pur, testé sous `node --test`).

/** Résultat d'une lecture : valeur, ou erreur de l'API avec son code stable. */
export type LectureAlerte = { ok: true } | { ok: false; erreur: { code: string } };

/**
 * Vrai si l'état d'intégrité n'a pas pu être lu pour une autre raison qu'un refus de rôle : le
 * retard d'horodatage est alors inconnu, et l'écran ne doit jamais dire « aucune alerte ». Le refus
 * (403 `interdit`, rôle collaborateur) est normal : cet écran ne concerne pas ce rôle.
 */
export function integriteIndisponible(lecture: LectureAlerte | null): boolean {
	return lecture !== null && !lecture.ok && lecture.erreur.code !== 'interdit';
}

/** Vrai si l'on peut affirmer qu'il n'y a aucune alerte. */
export function peutDireAucuneAlerte(
	total: number,
	alertes: LectureAlerte | null,
	integrite: LectureAlerte | null
): boolean {
	return (
		total === 0 && alertes?.ok === true && integrite !== null && !integriteIndisponible(integrite)
	);
}
