// Alertes d'un dossier (`?cabinet=<uuid>&dossier=<uuid>`) : brouillard en retard, échéances de
// clôture et retard d'horodatage.
import { lireAlertes } from '#lib/api/editions.ts';
import { lireEtatIntegrite } from '#lib/api/integrite.ts';
import { tenter } from '#lib/api/resultat.ts';
import { api } from '#lib/application.ts';
import { chargerDossier } from '#lib/contexte-dossier.ts';
import type { PageLoad } from './$types';

export const load: PageLoad = async ({ url }) =>
	chargerDossier(url, async ({ cabinet, dossier }) => {
		const [alertes, integrite] = await Promise.all([
			tenter(() => lireAlertes(api, cabinet, dossier)),
			// Le collaborateur ne consulte pas l'intégrité : son refus n'est pas une alerte.
			tenter(() => lireEtatIntegrite(api, cabinet, dossier))
		]);
		return { alertes, integrite };
	});
