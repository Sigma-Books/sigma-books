// Choix du cabinet : les cabinets dont l'utilisateur est membre actif.
import { api } from '#lib/application.ts';
import { listerCabinets } from '#lib/api/ressources.ts';
import { tenter } from '#lib/api/resultat.ts';
import type { PageLoad } from './$types';

export const load: PageLoad = async () => ({
	cabinets: await tenter(() => listerCabinets(api))
});
