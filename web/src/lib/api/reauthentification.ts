// Ré-authentification de la session courante (`POST /api/session/reauthentification`), exigée
// avant la validation par lot (Cedar, invariant de ré-authentification). Mot de passe de secours
// et code TOTP ; la réponse tourne le jeton de session : la session est relue ensuite pour
// renouveler le jeton anti-CSRF gardé par le client.

import type { ClientApi } from './client.ts';
import { estObjet } from './gardes.ts';
import type { EtatSession } from './types.ts';

const estReponse = (v: unknown): v is { csrf: string; fraiche_jusqu_a: string } =>
	estObjet(v) && typeof v.csrf === 'string' && typeof v.fraiche_jusqu_a === 'string';

export async function reauthentifier(
	api: ClientApi,
	motDePasse: string,
	code: string
): Promise<EtatSession> {
	await api.muter('POST', '/api/session/reauthentification', estReponse, {
		mot_de_passe: motDePasse,
		code
	});
	return api.ouvrirSession();
}
