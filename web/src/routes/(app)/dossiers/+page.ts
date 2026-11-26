// Choix du dossier d'un cabinet (`?cabinet=<uuid>`). Entrer dans un autre cabinet fait tourner la
// session avant tout appel (ADR 0012).
import { activerCabinet, api, exigerSessionComplete, membreDuCabinet } from '#lib/application.ts';
import { lirePackPays, listerDossiers } from '#lib/api/ressources.ts';
import { tenter } from '#lib/api/resultat.ts';
import { lireUuid } from '#lib/api/types.ts';
import { ROLES_CREATION_DOSSIER } from '#lib/libelles.ts';
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
		// Le pack pays n'est lu que pour qui peut créer un dossier (simple masquage : le serveur
		// décide) ; son échec n'empêche pas d'afficher la liste des dossiers.
		const pack = ROLES_CREATION_DOSSIER.includes(membre.role)
			? await tenter(() => lirePackPays(api, cabinet))
			: null;
		return { membre, dossiers, pack };
	});
	const fil = contenu.ok ? { cabinet: { id: cabinet, nom: contenu.valeur.membre.nom } } : {};
	return { cabinet, contenu, fil };
};
