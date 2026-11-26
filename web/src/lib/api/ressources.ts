// Appels typés des routes de l'API utilisées par l'interface.
//
// Le cabinet et le dossier sont **toujours** passés explicitement et figurent dans le chemin
// (ADR 0011, F1.14) : il n'existe aucun « cabinet courant » côté client ni côté serveur. Les
// identifiants sont des `Uuid` déjà vérifiés : un chemin ne peut pas être détourné.

import type { ClientApi } from './client.ts';
import { estPackPays, type CorpsCreationDossier, type PackPays } from './pays.ts';
import {
	estCabinetMembre,
	estCodesRecuperation,
	estConnexion,
	estDefi,
	estDossier,
	estFacteursMoi,
	estInvitation,
	estMembre,
	estObjetJson,
	estPasskeyEnrolee,
	estPreparationSecours,
	estReinitialisation,
	estSessionListee,
	listeDe,
	type CabinetMembre,
	type CodesRecuperation,
	type Connexion,
	type Defi,
	type Dossier,
	type FacteursMoi,
	type Invitation,
	type Membre,
	type PasskeyEnrolee,
	type PreparationSecours,
	type Reinitialisation,
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
export function creerDossier(
	api: ClientApi,
	cabinet: Uuid,
	corps: CorpsCreationDossier
): Promise<Dossier> {
	return api.muter('POST', `/api/cabinets/${cabinet}/dossiers`, estDossier, corps);
}

/** Pack pays : États, devises avec leur échelle, fuseaux, types d'identifiants (ADR 0027). */
export function lirePackPays(api: ClientApi, cabinet: Uuid): Promise<PackPays> {
	return api.lire(`/api/cabinets/${cabinet}/pays`, estPackPays);
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

// --- Administration du cabinet (PR 9a) ---------------------------------------------------------

/** Motifs d'une réinitialisation (liste fermée du serveur, `MOTIFS_REINITIALISATION`). */
export const MOTIFS_REINITIALISATION = [
	'appareil_perdu',
	'facteurs_perdus',
	'compte_compromis',
	'depart_reprise',
	'autre'
] as const;

export type MotifReinitialisation = (typeof MOTIFS_REINITIALISATION)[number];

const base = (cabinet: Uuid) => `/api/cabinets/${cabinet}`;

export function listerMembres(api: ClientApi, cabinet: Uuid): Promise<Membre[]> {
	return api.lire(`${base(cabinet)}/membres`, listeDe(estMembre));
}

export function changerRole(
	api: ClientApi,
	cabinet: Uuid,
	membre: Uuid,
	role: string
): Promise<Membre> {
	return api.muter('POST', `${base(cabinet)}/membres/${membre}/role`, estMembre, { role });
}

/** Suspension (`actif` faux) ou rétablissement (`actif` vrai) d'un membre. */
export function changerActivite(
	api: ClientApi,
	cabinet: Uuid,
	membre: Uuid,
	actif: boolean
): Promise<void> {
	const chemin = `${base(cabinet)}/membres/${membre}/suspension`;
	return actif
		? api.muterSansReponse('DELETE', chemin)
		: api.muter('POST', chemin, estMembre).then(() => undefined);
}

export function listerInvitations(api: ClientApi, cabinet: Uuid): Promise<Invitation[]> {
	return api.lire(`${base(cabinet)}/invitations`, listeDe(estInvitation));
}

export function inviter(
	api: ClientApi,
	cabinet: Uuid,
	invitation: { courriel: string; nom: string; role: string }
): Promise<Invitation> {
	return api.muter('POST', `${base(cabinet)}/invitations`, estInvitation, invitation);
}

export function renvoyerInvitation(api: ClientApi, cabinet: Uuid, invitation: Uuid): Promise<void> {
	return api
		.muter('POST', `${base(cabinet)}/invitations/${invitation}/renvoi`, estObjetJson)
		.then(() => undefined);
}

export function revoquerInvitation(api: ClientApi, cabinet: Uuid, invitation: Uuid): Promise<void> {
	return api
		.muter('DELETE', `${base(cabinet)}/invitations/${invitation}`, estObjetJson)
		.then(() => undefined);
}

export function listerReinitialisations(
	api: ClientApi,
	cabinet: Uuid
): Promise<Reinitialisation[]> {
	return api.lire(`${base(cabinet)}/reinitialisations`, listeDe(estReinitialisation));
}

export function reinitialiserAcces(
	api: ClientApi,
	cabinet: Uuid,
	membre: Uuid,
	demande: { motif_code: MotifReinitialisation; motif: string }
): Promise<void> {
	return api
		.muter('POST', `${base(cabinet)}/membres/${membre}/reinitialisation`, estObjetJson, {
			...demande,
			verification_hors_bande: true
		})
		.then(() => undefined);
}

/** Annulation par un administrateur. */
export function annulerReinitialisation(
	api: ClientApi,
	cabinet: Uuid,
	reinitialisation: Uuid
): Promise<void> {
	return api
		.muter('DELETE', `${base(cabinet)}/reinitialisations/${reinitialisation}`, estObjetJson)
		.then(() => undefined);
}

/** Contestation par le membre concerné de la réinitialisation de son propre accès. */
export function contesterReinitialisation(
	api: ClientApi,
	cabinet: Uuid,
	reinitialisation: Uuid
): Promise<void> {
	return api
		.muter(
			'POST',
			`${base(cabinet)}/reinitialisations/${reinitialisation}/contestation`,
			estObjetJson
		)
		.then(() => undefined);
}

export function changerSeparationTaches(
	api: ClientApi,
	cabinet: Uuid,
	active: boolean
): Promise<void> {
	const chemin = `${base(cabinet)}/separation-taches`;
	return active
		? api.muter('POST', chemin, estObjetJson).then(() => undefined)
		: api.muter('DELETE', chemin, estObjetJson).then(() => undefined);
}
