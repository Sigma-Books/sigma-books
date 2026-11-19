// Session lue au démarrage (`GET /api/session`) : échéances affichées et jeton anti-CSRF retenu
// en mémoire par le client. Un 401 mène à la page de connexion (`$lib/application`).
import { api } from '#lib/application.ts';
import { tenter } from '#lib/api/resultat.ts';
import type { LayoutLoad } from './$types';

export const load: LayoutLoad = async ({ depends }) => {
	depends('sigma:session');
	return { session: await tenter(() => api.ouvrirSession()) };
};
