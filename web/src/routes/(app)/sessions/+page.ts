// Mes sessions : liste des sessions actives de l'utilisateur (spec §21).
import { api } from '#lib/application.ts';
import { listerSessions } from '#lib/api/ressources.ts';
import { tenter } from '#lib/api/resultat.ts';
import type { PageLoad } from './$types';

export const load: PageLoad = async ({ depends }) => {
	depends('sigma:sessions');
	return { sessions: await tenter(() => listerSessions(api)) };
};
