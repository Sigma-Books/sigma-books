// Libellés français des valeurs codées rendues par l'API. Une valeur inconnue s'affiche telle
// quelle : l'interface ne l'interprète pas.

const ROLES: Record<string, string> = {
	administrateur: 'Administrateur',
	valideur: 'Valideur',
	collaborateur: 'Collaborateur',
	auditeur: 'Auditeur'
};

const METHODES: Record<string, string> = {
	passkey: 'Passkey',
	secours: 'Mot de passe et TOTP',
	code_recuperation: 'Code de récupération',
	lien_enrolement: 'Lien d’enrôlement'
};

const NATURES: Record<string, string> = {
	complete: 'Complète',
	restreinte: 'Restreinte (enrôlement d’une passkey)',
	enrolement: 'Enrôlement'
};

function libelle(table: Record<string, string>, valeur: string): string {
	return Object.hasOwn(table, valeur) ? (table[valeur] ?? valeur) : valeur;
}

export const libelleRole = (role: string): string => libelle(ROLES, role);
export const libelleMethode = (methode: string): string => libelle(METHODES, methode);
export const libelleNature = (nature: string): string => libelle(NATURES, nature);

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
