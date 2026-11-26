// Contexte d'un dossier : pays, devise de tenue, fuseau, langue, forme juridique, identifiants
// (`?cabinet=<uuid>&dossier=<uuid>`). Le pack pays n'est lu que pour l'administrateur (masquage :
// le serveur décide).
import { lirePackPays } from '#lib/api/ressources.ts';
import { tenter } from '#lib/api/resultat.ts';
import { api } from '#lib/application.ts';
import { chargerDossier, ROLES_CONTEXTE } from '#lib/contexte-dossier.ts';
import type { PageLoad } from './$types';

export const load: PageLoad = ({ url }) =>
	chargerDossier(url, async ({ cabinet, membre }) =>
		ROLES_CONTEXTE.includes(membre.role) ? await tenter(() => lirePackPays(api, cabinet)) : null
	);
