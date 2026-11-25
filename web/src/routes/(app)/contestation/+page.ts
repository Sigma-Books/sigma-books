// Contestation, par le membre, de la réinitialisation de son propre accès
// (`?cabinet=<uuid>&reinitialisation=<uuid>`).
import { activerCabinet, exigerSessionComplete } from '#lib/application.ts';
import { tenter } from '#lib/api/resultat.ts';
import { lireUuid } from '#lib/api/types.ts';
import type { PageLoad } from './$types';

export const load: PageLoad = async ({ url, parent }) => {
	const cabinet = lireUuid(url.searchParams.get('cabinet'));
	const reinitialisation = lireUuid(url.searchParams.get('reinitialisation'));
	if (cabinet === null || reinitialisation === null) return { contenu: null };
	return {
		cabinet,
		reinitialisation,
		contenu: await tenter(async () => {
			await exigerSessionComplete(parent);
			await activerCabinet(cabinet);
		})
	};
};
