// Appels typés des routes de l'API utilisées par l'interface.
//
// Le cabinet et le dossier sont **toujours** passés explicitement et figurent dans le chemin
// (ADR 0011, F1.14) : il n'existe aucun « cabinet courant » côté client ni côté serveur. Les
// identifiants sont des `Uuid` déjà vérifiés : un chemin ne peut pas être détourné.

import type { ClientApi } from './client.ts';
import {
	estCabinetMembre,
	estCodesRecuperation,
	estConnexion,
	estDefi,
	estDossier,
	estFacteursMoi,
	estObjetJson,
	estPasskeyEnrolee,
	estPreparationSecours,
	estSessionListee,
	listeDe,
	type CabinetMembre,
	type CodesRecuperation,
	type Connexion,
	type Defi,
	type Dossier,
	type FacteursMoi,
	type PasskeyEnrolee,
	type PreparationSecours,
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

// --- Connexion et méthodes de connexion (PR 8b, 8c) --------------------------------------------

/** Connexion par passkey, en deux appels : défi, puis réponse de l'authentificateur. */
export function defiConnexionPasskey(api: ClientApi, courriel: string): Promise<Defi> {
	return api.muterSansSession('/api/connexion/passkey/defi', estDefi, { courriel });
}

export function connecterParPasskey(
	api: ClientApi,
	courriel: string,
	credential: unknown
): Promise<Connexion> {
	return api.muterSansSession('/api/connexion/passkey', estConnexion, { courriel, credential });
}

export function connecterParSecours(
	api: ClientApi,
	courriel: string,
	motDePasse: string,
	code: string
): Promise<Connexion> {
	return api.muterSansSession('/api/connexion/secours', estConnexion, {
		courriel,
		mot_de_passe: motDePasse,
		code
	});
}

export function connecterParCodeRecuperation(
	api: ClientApi,
	courriel: string,
	code: string
): Promise<Connexion> {
	return api.muterSansSession('/api/connexion/code-recuperation', estConnexion, {
		courriel,
		code
	});
}

/** Usage d'un lien reçu par courriel (invitation, réinitialisation) : ouvre une session d'enrôlement. */
export function utiliserLien(api: ClientApi, jeton: string): Promise<Connexion> {
	return api.muterSansSession('/api/liens/utilisation', estConnexion, { jeton });
}

/** Invitation à un autre cabinet, acceptée par un utilisateur connecté. */
export function accepterInvitation(
	api: ClientApi,
	jeton: string
): Promise<Record<string, unknown>> {
	return api.muter('POST', '/api/moi/invitations/acceptation', estObjetJson, { jeton });
}

export function lireFacteurs(api: ClientApi): Promise<FacteursMoi> {
	return api.lire('/api/moi/facteurs', estFacteursMoi);
}

export function defiEnrolementPasskey(api: ClientApi): Promise<Defi> {
	return api.muter('POST', '/api/moi/facteurs/passkeys/defi', estDefi);
}

export function enrolerPasskey(
	api: ClientApi,
	nom: string,
	credential: unknown
): Promise<PasskeyEnrolee> {
	return api.muter('POST', '/api/moi/facteurs/passkeys', estPasskeyEnrolee, { nom, credential });
}

export function supprimerPasskey(api: ClientApi, passkey: Uuid): Promise<void> {
	return api.muterSansReponse('DELETE', `/api/moi/facteurs/passkeys/${passkey}`);
}

export function preparerSecours(api: ClientApi): Promise<PreparationSecours> {
	return api.muter('POST', '/api/moi/facteurs/secours/preparation', estPreparationSecours);
}

export function enrolerSecours(
	api: ClientApi,
	jetonEnrolement: string,
	motDePasse: string,
	code: string
): Promise<CodesRecuperation> {
	return api.muter('POST', '/api/moi/facteurs/secours', estCodesRecuperation, {
		jeton_enrolement: jetonEnrolement,
		mot_de_passe: motDePasse,
		code
	});
}

export function regenererCodes(api: ClientApi): Promise<CodesRecuperation> {
	return api.muter('POST', '/api/moi/facteurs/codes-recuperation', estCodesRecuperation);
}

// --- Ré-authentification (session fraîche) -----------------------------------------------------

export function reauthentifierParSecours(
	api: ClientApi,
	motDePasse: string,
	code: string
): Promise<void> {
	return api.renouveler('/api/session/reauthentification', {
		mot_de_passe: motDePasse,
		code
	});
}

export function defiReauthentificationPasskey(api: ClientApi): Promise<Defi> {
	return api.muter('POST', '/api/session/reauthentification/passkey/defi', estDefi);
}

export function reauthentifierParPasskey(api: ClientApi, credential: unknown): Promise<void> {
	return api.renouveler('/api/session/reauthentification/passkey', { credential });
}
