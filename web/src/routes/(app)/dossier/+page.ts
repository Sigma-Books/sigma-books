// Écran d'un dossier (`?cabinet=<uuid>&dossier=<uuid>`).
import { activerCabinet, api, membreDuCabinet } from '#lib/application.ts';
import { lireDossier } from '#lib/api/ressources.ts';
import { tenter } from '#lib/api/resultat.ts';
import { lireUuid } from '#lib/api/types.ts';
import type { PageLoad } from './$types';

export const load: PageLoad = async ({ url }) => {
	const cabinet = lireUuid(url.searchParams.get('cabinet'));
	const dossier = lireUuid(url.searchParams.get('dossier'));
	if (cabinet === null || dossier === null) return { contenu: null };
	const contenu = await tenter(async () => {
		await activerCabinet(cabinet);
		const [membre, lu] = await Promise.all([
			membreDuCabinet(cabinet),
			lireDossier(api, cabinet, dossier)
		]);
		return { membre, dossier: lu };
	});
	const fil = contenu.ok
		? {
				cabinet: { id: cabinet, nom: contenu.valeur.membre.nom },
				dossier: { id: dossier, nom: contenu.valeur.dossier.nom }
			}
		: {};
	return { contenu, fil };
};
