// Page réservée : l'administration du cabinet arrive après la PR 9 du lot L2.
import { activerCabinet, membreDuCabinet } from '#lib/application.ts';
import { tenter } from '#lib/api/resultat.ts';
import { lireUuid } from '#lib/api/types.ts';
import type { PageLoad } from './$types';

export const load: PageLoad = async ({ url }) => {
	const cabinet = lireUuid(url.searchParams.get('cabinet'));
	if (cabinet === null) return { contenu: null };
	const contenu = await tenter(async () => {
		await activerCabinet(cabinet);
		return membreDuCabinet(cabinet);
	});
	const fil = contenu.ok ? { cabinet: { id: cabinet, nom: contenu.valeur.nom } } : {};
	return { contenu, fil };
};
