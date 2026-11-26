// Écran d'un dossier (`?cabinet=<uuid>&dossier=<uuid>`).
import { activerCabinet, api, exigerSessionComplete, membreDuCabinet } from '#lib/application.ts';
import { lirePackPays, lireDossier } from '#lib/api/ressources.ts';
import { tenter } from '#lib/api/resultat.ts';
import { lireUuid } from '#lib/api/types.ts';
import { ROLES_CREATION_DOSSIER } from '#lib/libelles.ts';
import type { PageLoad } from './$types';

export const load: PageLoad = async ({ url, parent }) => {
	const cabinet = lireUuid(url.searchParams.get('cabinet'));
	const dossier = lireUuid(url.searchParams.get('dossier'));
	if (cabinet === null || dossier === null) return { contenu: null };
	const contenu = await tenter(async () => {
		await exigerSessionComplete(parent);
		await activerCabinet(cabinet);
		const [membre, lu] = await Promise.all([
			membreDuCabinet(cabinet),
			lireDossier(api, cabinet, dossier)
		]);
		// Le pack ne sert qu'à nommer le pays et les types d'identifiants : sans lui (rôle sans droit
		// de création, ou erreur), l'écran affiche les codes.
		const pack = ROLES_CREATION_DOSSIER.includes(membre.role)
			? await tenter(() => lirePackPays(api, cabinet))
			: null;
		return { membre, dossier: lu, pack: pack?.ok ? pack.valeur : null };
	});
	const fil = contenu.ok
		? {
				cabinet: { id: cabinet, nom: contenu.valeur.membre.nom },
				dossier: { id: dossier, nom: contenu.valeur.dossier.nom }
			}
		: {};
	return { contenu, fil };
};
