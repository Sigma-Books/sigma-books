// Choix du dossier d'un cabinet (`?cabinet=<uuid>`). Entrer dans un autre cabinet fait tourner la
// session avant tout appel (ADR 0012).
import { activerCabinet, api, exigerSessionComplete, membreDuCabinet } from '#lib/application.ts';
import { listerDossiers } from '#lib/api/ressources.ts';
import { tenter } from '#lib/api/resultat.ts';
import { lireUuid } from '#lib/api/types.ts';
import type { PageLoad } from './$types';

export const load: PageLoad = async ({ url, parent }) => {
	const cabinet = lireUuid(url.searchParams.get('cabinet'));
	if (cabinet === null) return { cabinet, contenu: null };
	const contenu = await tenter(async () => {
		await exigerSessionComplete(parent);
		await activerCabinet(cabinet);
		const [membre, dossiers] = await Promise.all([
			membreDuCabinet(cabinet),
			listerDossiers(api, cabinet)
		]);
		return { membre, dossiers };
	});
	const fil = contenu.ok ? { cabinet: { id: cabinet, nom: contenu.valeur.membre.nom } } : {};
	return { cabinet, contenu, fil };
};
