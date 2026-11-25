// Conversions WebAuthn entre le JSON du serveur (`webauthn-rs`, ADR 0018) et l'API du navigateur.
//
// Le serveur rend les options d'une cérémonie avec les octets en base64url sans remplissage ; le
// navigateur veut des `ArrayBuffer`. Dans l'autre sens, la réponse de l'authentificateur part en
// JSON, octets en base64url. Rien ici n'importe de SvelteKit ni n'appelle `navigator` : ces
// fonctions pures se testent sous `node --test` (la partie navigateur est `passkeys.ts`).
//
// Une option inattendue est ignorée, jamais transmise telle quelle : seules les clés connues sont
// reprises, avec leur type vérifié. Une forme invalide donne `null` (réponse invalide).

type Objet = Record<string, unknown>;

function estObjet(valeur: unknown): valeur is Objet {
	return typeof valeur === 'object' && valeur !== null && !Array.isArray(valeur);
}

const MOTIF_BASE64URL = /^[A-Za-z0-9_-]*$/;

/** Octets d'un texte base64url (sans remplissage), ou `null` si le texte n'en est pas un. */
export function base64urlVersOctets(texte: string): Uint8Array<ArrayBuffer> | null {
	if (!MOTIF_BASE64URL.test(texte) || texte.length % 4 === 1) return null;
	const standard = texte.replaceAll('-', '+').replaceAll('_', '/');
	const rempli = standard + '='.repeat((4 - (standard.length % 4)) % 4);
	const binaire = atob(rempli);
	const octets = new Uint8Array(binaire.length);
	for (let i = 0; i < binaire.length; i += 1) octets[i] = binaire.charCodeAt(i);
	return octets;
}

/** Texte base64url, sans remplissage, des octets donnés. */
export function octetsVersBase64url(octets: ArrayBuffer | Uint8Array): string {
	const vue = octets instanceof Uint8Array ? octets : new Uint8Array(octets);
	let binaire = '';
	for (const octet of vue) binaire += String.fromCharCode(octet);
	return btoa(binaire).replaceAll('+', '-').replaceAll('/', '_').replaceAll('=', '');
}

const TRANSPORTS = ['ble', 'hybrid', 'internal', 'nfc', 'usb'] as const;
const EXIGENCES_VERIFICATION = ['required', 'preferred', 'discouraged'] as const;
const ATTESTATIONS = ['none', 'indirect', 'direct', 'enterprise'] as const;
const CLES_RESIDENTES = ['discouraged', 'preferred', 'required'] as const;
const RATTACHEMENTS = ['platform', 'cross-platform'] as const;

function parmi<T extends string>(liste: readonly T[], valeur: unknown): T | undefined {
	return typeof valeur === 'string' ? liste.find((v) => v === valeur) : undefined;
}

function octets(valeur: unknown): Uint8Array<ArrayBuffer> | null {
	return typeof valeur === 'string' ? base64urlVersOctets(valeur) : null;
}

function descripteurs(valeur: unknown): PublicKeyCredentialDescriptor[] | null {
	if (valeur === undefined || valeur === null) return [];
	if (!Array.isArray(valeur)) return null;
	const liste: PublicKeyCredentialDescriptor[] = [];
	for (const element of valeur) {
		if (!estObjet(element) || element.type !== 'public-key') return null;
		const id = octets(element.id);
		if (id === null) return null;
		const transports = Array.isArray(element.transports)
			? element.transports.flatMap((t) => {
					const connu = parmi(TRANSPORTS, t);
					return connu === undefined ? [] : [connu];
				})
			: [];
		liste.push(
			transports.length > 0 ? { type: 'public-key', id, transports } : { type: 'public-key', id }
		);
	}
	return liste;
}

function delai(valeur: unknown): number | undefined {
	return typeof valeur === 'number' && Number.isFinite(valeur) && valeur > 0 ? valeur : undefined;
}

/** `{ publicKey: … }` du défi, ou `null` si le défi n'a pas la forme attendue. */
function cle(defi: unknown): Objet | null {
	return estObjet(defi) && estObjet(defi.publicKey) ? defi.publicKey : null;
}

/** Options de `navigator.credentials.get` d'un défi d'authentification. */
export function optionsDemande(defi: unknown): PublicKeyCredentialRequestOptions | null {
	const p = cle(defi);
	if (p === null) return null;
	const challenge = octets(p.challenge);
	const allowCredentials = descripteurs(p.allowCredentials);
	if (challenge === null || allowCredentials === null || typeof p.rpId !== 'string') return null;
	const options: PublicKeyCredentialRequestOptions = {
		challenge,
		rpId: p.rpId,
		allowCredentials,
		userVerification: parmi(EXIGENCES_VERIFICATION, p.userVerification) ?? 'required'
	};
	const timeout = delai(p.timeout);
	if (timeout !== undefined) options.timeout = timeout;
	return options;
}

/** Options de `navigator.credentials.create` d'un défi d'enrôlement. */
export function optionsCreation(defi: unknown): PublicKeyCredentialCreationOptions | null {
	const p = cle(defi);
	if (p === null) return null;
	const challenge = octets(p.challenge);
	const excludeCredentials = descripteurs(p.excludeCredentials);
	if (challenge === null || excludeCredentials === null) return null;
	if (!estObjet(p.rp) || typeof p.rp.name !== 'string' || typeof p.rp.id !== 'string') return null;
	if (
		!estObjet(p.user) ||
		typeof p.user.name !== 'string' ||
		typeof p.user.displayName !== 'string'
	) {
		return null;
	}
	const idUtilisateur = octets(p.user.id);
	if (idUtilisateur === null) return null;
	if (!Array.isArray(p.pubKeyCredParams)) return null;
	const pubKeyCredParams: PublicKeyCredentialParameters[] = [];
	for (const parametre of p.pubKeyCredParams) {
		if (
			!estObjet(parametre) ||
			parametre.type !== 'public-key' ||
			typeof parametre.alg !== 'number' ||
			!Number.isInteger(parametre.alg)
		) {
			return null;
		}
		pubKeyCredParams.push({ type: 'public-key', alg: parametre.alg });
	}
	const options: PublicKeyCredentialCreationOptions = {
		rp: { name: p.rp.name, id: p.rp.id },
		user: { id: idUtilisateur, name: p.user.name, displayName: p.user.displayName },
		challenge,
		pubKeyCredParams,
		excludeCredentials,
		attestation: parmi(ATTESTATIONS, p.attestation) ?? 'none'
	};
	const timeout = delai(p.timeout);
	if (timeout !== undefined) options.timeout = timeout;
	if (estObjet(p.authenticatorSelection)) {
		const s = p.authenticatorSelection;
		const selection: AuthenticatorSelectionCriteria = {
			userVerification: parmi(EXIGENCES_VERIFICATION, s.userVerification) ?? 'required'
		};
		const rattachement = parmi(RATTACHEMENTS, s.authenticatorAttachment);
		if (rattachement !== undefined) selection.authenticatorAttachment = rattachement;
		const residente = parmi(CLES_RESIDENTES, s.residentKey);
		if (residente !== undefined) selection.residentKey = residente;
		if (typeof s.requireResidentKey === 'boolean') {
			selection.requireResidentKey = s.requireResidentKey;
		}
		options.authenticatorSelection = selection;
	}
	return options;
}

/** Forme minimale d'un credential du navigateur, pour s'affranchir du DOM dans les tests. */
export interface CredentialNavigateur {
	id: string;
	rawId: ArrayBuffer;
	type: string;
	response: {
		clientDataJSON: ArrayBuffer;
		authenticatorData?: ArrayBuffer;
		signature?: ArrayBuffer;
		userHandle?: ArrayBuffer | null;
		attestationObject?: ArrayBuffer;
	};
}

/** Réponse d'authentification au format attendu par le serveur ; `null` si elle est incomplète. */
export function credentialDemande(c: CredentialNavigateur): Objet | null {
	const { authenticatorData, signature, userHandle, clientDataJSON } = c.response;
	if (authenticatorData === undefined || signature === undefined) return null;
	return {
		id: c.id,
		rawId: octetsVersBase64url(c.rawId),
		type: c.type,
		response: {
			authenticatorData: octetsVersBase64url(authenticatorData),
			clientDataJSON: octetsVersBase64url(clientDataJSON),
			signature: octetsVersBase64url(signature),
			...(userHandle ? { userHandle: octetsVersBase64url(userHandle) } : {})
		}
	};
}

/** Réponse d'enrôlement au format attendu par le serveur ; `null` si elle est incomplète. */
export function credentialCree(c: CredentialNavigateur): Objet | null {
	const { attestationObject, clientDataJSON } = c.response;
	if (attestationObject === undefined) return null;
	return {
		id: c.id,
		rawId: octetsVersBase64url(c.rawId),
		type: c.type,
		response: {
			attestationObject: octetsVersBase64url(attestationObject),
			clientDataJSON: octetsVersBase64url(clientDataJSON)
		}
	};
}
