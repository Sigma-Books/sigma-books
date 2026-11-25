// Brouillard d'un dossier (`?cabinet=<uuid>&dossier=<uuid>[&journal=<code>][&periode=<uuid>]`).
import { listerBrouillard } from '#lib/api/brouillard.ts';
import { listerExercices, listerJournaux } from '#lib/api/referentiel.ts';
import { lireUuid } from '#lib/api/types.ts';
import { api } from '#lib/application.ts';
import { chargerDossier } from '#lib/contexte-dossier.ts';
import type { PageLoad } from './$types';

export const load: PageLoad = async ({ url }) => {
	const journal = url.searchParams.get('journal') ?? '';
	const periode = lireUuid(url.searchParams.get('periode'));
	const charge = await chargerDossier(url, async ({ cabinet, dossier }) => {
		const [journaux, exercices, page] = await Promise.all([
			listerJournaux(api, cabinet, dossier),
			listerExercices(api, cabinet, dossier),
			listerBrouillard(api, cabinet, dossier, { journal, periode })
		]);
		return { journaux, exercices, page };
	});
	return { ...charge, filtre: { journal, periode } };
};
