// Appels typés des routes de l'API utilisées par l'interface.
//
// Le cabinet et le dossier sont **toujours** passés explicitement et figurent dans le chemin
// (ADR 0011, F1.14) : il n'existe aucun « cabinet courant » côté client ni côté serveur. Les
// identifiants sont des `Uuid` déjà vérifiés : un chemin ne peut pas être détourné.

import type { ClientApi } from './client.ts';
import {
	estCabinetMembre,
	estDossier,
	estSessionListee,
	listeDe,
	type CabinetMembre,
	type Dossier,
	type SessionListee,
	type Uuid
} from './types.ts';

/** Cabinets dont l'utilisateur est membre actif, avec son rôle. */
export function listerCabinets(api: ClientApi): Promise<CabinetMembre[]> {
	return api.lire('/api/moi/cabinets', listeDe(estCabinetMembre));
}

/** Dossiers du cabinet visibles par l'utilisateur (tous pour l'administrateur, Q5.1). */
export function listerDossiers(api: ClientApi, cabinet: Uuid): Promise<Dossier[]> {
	return api.lire(`/api/cabinets/${cabinet}/dossiers`, listeDe(estDossier));
}

export function lireDossier(api: ClientApi, cabinet: Uuid, dossier: Uuid): Promise<Dossier> {
	return api.lire(`/api/cabinets/${cabinet}/dossiers/${dossier}`, estDossier);
}

/** Création d'un dossier ; le serveur décide (§20 : administrateur, valideur). */
export function creerDossier(api: ClientApi, cabinet: Uuid, nom: string): Promise<Dossier> {
	return api.muter('POST', `/api/cabinets/${cabinet}/dossiers`, estDossier, { nom });
}

/** Sessions actives de l'utilisateur. */
export function listerSessions(api: ClientApi): Promise<SessionListee[]> {
	return api.lire('/api/moi/sessions', listeDe(estSessionListee));
}

export function revoquerSession(api: ClientApi, session: Uuid): Promise<void> {
	return api.muterSansReponse('DELETE', `/api/moi/sessions/${session}`);
}
