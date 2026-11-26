// Intégrité d'un dossier (`?cabinet=<uuid>&dossier=<uuid>`) : état de la chaîne et jetons.
import { lireEtatIntegrite, listerJetons } from '#lib/api/integrite.ts';
import { tenter } from '#lib/api/resultat.ts';
import { api } from '#lib/application.ts';
import { chargerDossier } from '#lib/contexte-dossier.ts';
import type { PageLoad } from './$types';

export const load: PageLoad = async ({ url }) =>
	chargerDossier(url, async ({ cabinet, dossier }) => {
		// Séparés : un refus sur l'un (rôle, affectation) n'empêche pas d'afficher l'autre.
		const [etat, jetons] = await Promise.all([
			tenter(() => lireEtatIntegrite(api, cabinet, dossier)),
			tenter(() => listerJetons(api, cabinet, dossier))
		]);
		return { etat, jetons };
	});
