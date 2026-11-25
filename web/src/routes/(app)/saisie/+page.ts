// Saisie manuelle d'une écriture (`?cabinet=<uuid>&dossier=<uuid>[&mode=kilometre][&brouillon=<uuid>]`).
import { lireBrouillon } from '#lib/api/brouillard.ts';
import { listerComptes, listerJournaux, listerTiers } from '#lib/api/referentiel.ts';
import { lireUuid } from '#lib/api/types.ts';
import { api } from '#lib/application.ts';
import { chargerDossier } from '#lib/contexte-dossier.ts';
import type { PageLoad } from './$types';

export const load: PageLoad = async ({ url }) => {
	const mode: 'piece' | 'kilometre' =
		url.searchParams.get('mode') === 'kilometre' ? 'kilometre' : 'piece';
	const brouillon = lireUuid(url.searchParams.get('brouillon'));
	const charge = await chargerDossier(url, async ({ cabinet, dossier }) => {
		const [comptes, journaux, tiers, ouvert] = await Promise.all([
			listerComptes(api, cabinet, dossier),
			listerJournaux(api, cabinet, dossier),
			listerTiers(api, cabinet, dossier),
			brouillon === null ? Promise.resolve(null) : lireBrouillon(api, cabinet, dossier, brouillon)
		]);
		return { comptes, journaux, tiers, ouvert };
	});
	return { ...charge, mode };
};
