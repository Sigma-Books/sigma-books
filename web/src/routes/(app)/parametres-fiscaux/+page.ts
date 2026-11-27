// Paramètres fiscaux d'un dossier (`?cabinet=<uuid>&dossier=<uuid>[&date=AAAA-MM-JJ]`) : versions
// du pays et du dossier, version applicable à la date choisie (le jour du dossier par défaut).
import { lireApplicable, listerVersions } from '#lib/api/parametres-fiscaux.ts';
import { api } from '#lib/application.ts';
import { chargerDossier } from '#lib/contexte-dossier.ts';
import { estDateIso } from '#lib/dates.ts';
import type { PageLoad } from './$types';

export const load: PageLoad = ({ url }) =>
	chargerDossier(url, async ({ cabinet, dossier }) => {
		const demandee = url.searchParams.get('date');
		// Une date illisible dans l'adresse est ignorée : le jour du dossier s'applique.
		const date = demandee !== null && estDateIso(demandee) ? demandee : undefined;
		const [liste, applicable] = await Promise.all([
			listerVersions(api, cabinet, dossier),
			lireApplicable(api, cabinet, dossier, date)
		]);
		return { liste, applicable, dateChoisie: date ?? null };
	});
