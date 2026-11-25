// Administration d'un cabinet (`?cabinet=<uuid>`) : membres, invitations et réinitialisations en
// attente. Chaque liste est lue séparément : le refus du serveur (rôle insuffisant) ou une erreur
// sur l'une n'empêche pas d'afficher les autres. L'interface ne décide pas qui administre.
import { activerCabinet, api, exigerSessionComplete, membreDuCabinet } from '#lib/application.ts';
import { listerInvitations, listerMembres, listerReinitialisations } from '#lib/api/ressources.ts';
import { tenter } from '#lib/api/resultat.ts';
import { lireUuid } from '#lib/api/types.ts';
import type { PageLoad } from './$types';

export const load: PageLoad = async ({ url, depends, parent }) => {
	depends('sigma:administration');
	const cabinet = lireUuid(url.searchParams.get('cabinet'));
	if (cabinet === null) return { cabinet, contenu: null };
	const contenu = await tenter(async () => {
		await exigerSessionComplete(parent);
		await activerCabinet(cabinet);
		return membreDuCabinet(cabinet);
	});
	if (!contenu.ok) return { cabinet, contenu };
	const [membres, invitations, reinitialisations] = await Promise.all([
		tenter(() => listerMembres(api, cabinet)),
		tenter(() => listerInvitations(api, cabinet)),
		tenter(() => listerReinitialisations(api, cabinet))
	]);
	const fil = { cabinet: { id: cabinet, nom: contenu.valeur.nom } };
	return { cabinet, contenu, membres, invitations, reinitialisations, fil };
};
