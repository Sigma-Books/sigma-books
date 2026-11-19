// Instance du client d'API de l'application, et changement de cabinet actif.
//
// Le « cabinet actif » n'est que le dernier cabinet pour lequel la session a tourné (ADR 0012 :
// rotation au changement de cabinet actif). Il vit en mémoire, n'est jamais envoyé au serveur et
// ne choisit aucune ressource : chaque appel prend le cabinet de l'URL de la page (ADR 0011,
// F1.14). Après un rechargement, la mémoire est vide : l'entrée dans un cabinet tourne à nouveau
// le jeton, ce qui est sans inconvénient.

import { goto } from '$app/navigation';
import { resolve } from '$app/paths';
import { creerClient } from './api/client.ts';
import { ErreurApi } from './api/erreurs.ts';
import { listerCabinets } from './api/ressources.ts';
import type { CabinetMembre, Uuid } from './api/types.ts';

let cabinetActif: Uuid | null = null;

function allerALaConnexion(): void {
	cabinetActif = null;
	const connexion = resolve('connexion/');
	if (window.location.pathname === connexion) return;
	// Une navigation interrompue (par exemple pendant le tout premier chargement) se rabat sur un
	// chargement complet de la page.
	goto(connexion, { replace: true }).catch(() => window.location.replace(connexion));
}

export const api = creerClient({ surNonAuthentifie: allerALaConnexion });

/**
 * À appeler avant tout appel à un cabinet : si l'utilisateur change de cabinet, la session tourne
 * (`POST /api/session/rotation`) et le jeton anti-CSRF est renouvelé.
 */
export async function activerCabinet(cabinet: Uuid): Promise<void> {
	if (cabinetActif === cabinet) return;
	await api.tourner();
	cabinetActif = cabinet;
}

/** Déconnexion : la session est révoquée, puis la page de connexion s'affiche. */
export async function seDeconnecter(): Promise<void> {
	await api.deconnecter();
	allerALaConnexion();
}

/**
 * Le cabinet de l'URL parmi ceux de l'utilisateur (`GET /api/moi/cabinets`), pour son nom et le
 * rôle affiché. Absent de la liste : « introuvable », comme le dirait le serveur (ADR 0011).
 */
export async function membreDuCabinet(cabinet: Uuid): Promise<CabinetMembre> {
	const membre = (await listerCabinets(api)).find((c) => c.id === cabinet);
	if (membre === undefined) throw new ErreurApi('introuvable', 404);
	return membre;
}
