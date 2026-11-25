// Éditions d'un dossier (`?cabinet=<uuid>&dossier=<uuid>[&edition=balance|grand-livre|journaux|centralisateur]`).
import { NATURES_EDITION, type NatureEdition } from '#lib/api/editions.ts';
import { listerExercices, listerJournaux } from '#lib/api/referentiel.ts';
import { tenter } from '#lib/api/resultat.ts';
import { api } from '#lib/application.ts';
import { chargerDossier } from '#lib/contexte-dossier.ts';
import type { PageLoad } from './$types';

export const load: PageLoad = async ({ url }) => {
	const demandee = url.searchParams.get('edition');
	const nature: NatureEdition = NATURES_EDITION.find((n) => n === demandee) ?? 'balance';
	const charge = await chargerDossier(url, async ({ cabinet, dossier }) => {
		// Exercices et journaux ne servent qu'à proposer des choix : leur absence n'empêche rien.
		const [exercices, journaux] = await Promise.all([
			tenter(() => listerExercices(api, cabinet, dossier)),
			tenter(() => listerJournaux(api, cabinet, dossier))
		]);
		return {
			exercices: exercices.ok ? exercices.valeur : [],
			journaux: journaux.ok ? journaux.valeur : []
		};
	});
	return { ...charge, nature };
};
