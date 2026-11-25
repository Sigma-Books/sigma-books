// Une écriture validée (`?cabinet=<uuid>&dossier=<uuid>&ecriture=<uuid>`) : consultation et
// contre-passation.
import { lireEcriture } from '#lib/api/brouillard.ts';
import { lireUuid } from '#lib/api/types.ts';
import { api } from '#lib/application.ts';
import { chargerDossier } from '#lib/contexte-dossier.ts';
import type { PageLoad } from './$types';

export const load: PageLoad = async ({ url }) => {
	const ecriture = lireUuid(url.searchParams.get('ecriture'));
	if (ecriture === null) return { contenu: null };
	return chargerDossier(url, ({ cabinet, dossier }) =>
		lireEcriture(api, cabinet, dossier, ecriture)
	);
};
