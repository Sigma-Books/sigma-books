// Cérémonies WebAuthn dans le navigateur (`navigator.credentials`). Les conversions de format sont
// dans `webauthn.ts` (pures, testées) ; ce module n'est que l'appel au navigateur.
//
// Le navigateur est le seul à connaître l'authentificateur : l'interface ne voit jamais de clé
// privée, et le serveur vérifie tout (défi à usage unique, origine, vérification de l'utilisateur).

import { ErreurApi } from './api/erreurs.ts';
import {
	credentialCree,
	credentialDemande,
	optionsCreation,
	optionsDemande,
	type CredentialNavigateur
} from './webauthn.ts';

/** Issue d'une cérémonie qui n'a pas abouti côté navigateur. */
export type RaisonPasskey = 'non_prise_en_charge' | 'annulee';

export class ErreurPasskey extends Error {
	readonly raison: RaisonPasskey;

	constructor(raison: RaisonPasskey) {
		super(raison);
		this.name = 'ErreurPasskey';
		this.raison = raison;
	}
}

export function passkeysPrisesEnCharge(): boolean {
	return (
		typeof window !== 'undefined' && 'PublicKeyCredential' in window && !!navigator.credentials
	);
}

async function appeler(travail: () => Promise<Credential | null>): Promise<CredentialNavigateur> {
	if (!passkeysPrisesEnCharge()) throw new ErreurPasskey('non_prise_en_charge');
	let credential: Credential | null;
	try {
		credential = await travail();
	} catch (erreur) {
		// Refus de l'utilisateur, délai écoulé ou authentificateur absent : le navigateur ne les
		// distingue volontairement pas (`NotAllowedError`).
		if (erreur instanceof DOMException && erreur.name === 'NotSupportedError') {
			throw new ErreurPasskey('non_prise_en_charge');
		}
		throw new ErreurPasskey('annulee');
	}
	if (credential === null || credential.type !== 'public-key') throw new ErreurPasskey('annulee');
	return credential as unknown as CredentialNavigateur;
}

/** Authentification : réponse JSON du credential, à renvoyer au serveur. */
export async function authentifier(defi: unknown): Promise<Record<string, unknown>> {
	const options = optionsDemande(defi);
	if (options === null) throw new ErreurApi('reponse_invalide', null);
	const credential = await appeler(() => navigator.credentials.get({ publicKey: options }));
	const reponse = credentialDemande(credential);
	if (reponse === null) throw new ErreurApi('reponse_invalide', null);
	return reponse;
}

/** Enrôlement : réponse JSON du credential créé, à renvoyer au serveur. */
export async function enroler(defi: unknown): Promise<Record<string, unknown>> {
	const options = optionsCreation(defi);
	if (options === null) throw new ErreurApi('reponse_invalide', null);
	const credential = await appeler(() => navigator.credentials.create({ publicKey: options }));
	const reponse = credentialCree(credential);
	if (reponse === null) throw new ErreurApi('reponse_invalide', null);
	return reponse;
}
