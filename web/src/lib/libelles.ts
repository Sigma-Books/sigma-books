// Libellés des valeurs codées rendues par l'API (catalogue de messages, `i18n/`). Une valeur
// inconnue s'affiche telle quelle : l'interface ne l'interprète pas.

import { chercher } from './i18n/index.ts';

function libelle(prefixe: string, valeur: string): string {
	return chercher(`${prefixe}.${valeur}`) ?? valeur;
}

export const libelleRole = (role: string): string => libelle('role', role);
export const libelleMethode = (methode: string): string => libelle('methode', methode);
export const libelleNature = (nature: string): string => libelle('nature_session', nature);

/**
 * Rôles pour lesquels l'interface **affiche** la création d'un dossier (§20 « Créer un dossier »).
 * Simple masquage : le serveur décide (Cedar, puis la requête), et son refus est affiché.
 */
export const ROLES_CREATION_DOSSIER: readonly string[] = ['administrateur', 'valideur'];

/** Secret TOTP en groupes de quatre caractères, plus lisible à recopier. */
export function grouperSecret(secret: string): string {
	return (
		secret
			.replace(/\s+/g, '')
			.match(/.{1,4}/g)
			?.join(' ') ?? ''
	);
}

/** L'URI `otpauth` du serveur, si c'en est bien une (seul schéma admis pour le lien). */
export function uriOtpauthSure(uri: string): string | null {
	return uri.startsWith('otpauth://totp/') && !/[\s<>"']/.test(uri) ? uri : null;
}

/** Rôles proposés à la sélection (libellés seulement : le serveur valide le rôle). */
export const ROLES_PROPOSES: readonly string[] = [
	'administrateur',
	'valideur',
	'collaborateur',
	'auditeur'
];

export const libelleMotifReinitialisation = (motif: string): string =>
	libelle('reinitialisation', motif);
