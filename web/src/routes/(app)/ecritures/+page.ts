// Écritures validées d'un dossier (`?cabinet=<uuid>&dossier=<uuid>`), en lecture seule.
import { listerEcritures } from '#lib/api/brouillard.ts';
import { api } from '#lib/application.ts';
import { chargerDossier } from '#lib/contexte-dossier.ts';
import type { PageLoad } from './$types';

export const load: PageLoad = ({ url }) =>
	chargerDossier(url, ({ cabinet, dossier }) => listerEcritures(api, cabinet, dossier));
