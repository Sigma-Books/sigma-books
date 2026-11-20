// Erreurs de l'API traduites en types (crates/api/src/serveur/erreur.rs).
//
// Le serveur répond toujours `{"erreur":"<code>"}`, code pris dans une liste fermée. Le client
// n'en garde que ce code et le statut : jamais le corps brut, jamais l'URL ni un identifiant.
// Une ressource d'autrui et une ressource absente donnent le même 404 (ADR 0011) : l'interface
// les présente de la même façon (« introuvable »), sans rien supposer de plus.

/** Codes émis par le serveur (`ErreurApi::code`). */
export const CODES_SERVEUR = [
	'requete_invalide',
	'non_authentifie',
	'interdit',
	'introuvable',
	'methode_non_admise',
	'corps_trop_grand',
	'trop_de_requetes',
	'erreur_interne',
	'delai_depasse'
] as const;

export type CodeServeur = (typeof CODES_SERVEUR)[number];

/** Codes propres au client : serveur injoignable, réponse d'une forme inattendue. */
export type CodeClient = 'reseau' | 'reponse_invalide';

export type CodeErreur = CodeServeur | CodeClient;

/** Erreur d'un appel à l'API. Son message est le code seul : aucune donnée n'y figure. */
export class ErreurApi extends Error {
	readonly code: CodeErreur;
	/** Statut HTTP, ou `null` si aucune réponse n'a été reçue. */
	readonly statut: number | null;
	/** Délai avant de réessayer, en secondes (`Retry-After` d'un 429). */
	readonly reessayerDans: number | null;

	constructor(code: CodeErreur, statut: number | null, reessayerDans: number | null = null) {
		super(code);
		this.name = 'ErreurApi';
		this.code = code;
		this.statut = statut;
		this.reessayerDans = reessayerDans;
	}
}

export function estCodeServeur(valeur: unknown): valeur is CodeServeur {
	return typeof valeur === 'string' && (CODES_SERVEUR as readonly string[]).includes(valeur);
}

/** Code déduit du seul statut, comme le fait le serveur (`erreur::pour_statut`). */
export function codeDepuisStatut(statut: number): CodeServeur {
	switch (statut) {
		case 400:
			return 'requete_invalide';
		case 401:
			return 'non_authentifie';
		case 403:
			return 'interdit';
		case 404:
			return 'introuvable';
		case 405:
			return 'methode_non_admise';
		case 413:
			return 'corps_trop_grand';
		case 429:
			return 'trop_de_requetes';
		case 503:
			return 'delai_depasse';
		default:
			return statut >= 500 ? 'erreur_interne' : 'requete_invalide';
	}
}

/** `Retry-After` en secondes entières ; toute autre forme est ignorée. */
function lireReessayer(valeur: string | null): number | null {
	if (valeur === null || !/^\d{1,6}$/.test(valeur.trim())) return null;
	return Number.parseInt(valeur.trim(), 10);
}

/** Statuts dont l'interface tire une conduite (connexion, refus, absence) : le statut décide. */
const STATUTS_DECISIFS = new Set([401, 403, 404, 429]);

/**
 * Traduit une réponse d'erreur en {@link ErreurApi}. Pour 401, 403, 404 et 429, le statut seul
 * fixe le code : un corps inattendu ne change jamais la conduite de l'interface. Pour les autres
 * statuts, le code du corps est retenu s'il appartient à la liste fermée, sinon le statut décide.
 */
export async function traduireReponse(reponse: Response): Promise<ErreurApi> {
	let code: CodeServeur = codeDepuisStatut(reponse.status);
	if (!STATUTS_DECISIFS.has(reponse.status)) {
		try {
			const corps: unknown = await reponse.json();
			if (typeof corps === 'object' && corps !== null && 'erreur' in corps) {
				if (estCodeServeur(corps.erreur)) code = corps.erreur;
			}
		} catch {
			// Corps absent ou non JSON : le statut suffit.
		}
	}
	const reessayer =
		code === 'trop_de_requetes' ? lireReessayer(reponse.headers.get('retry-after')) : null;
	return new ErreurApi(code, reponse.status, reessayer);
}

/** Message en français, sans aucune donnée de la requête ni de la réponse. */
export function messageErreur(erreur: ErreurApi): string {
	switch (erreur.code) {
		case 'requete_invalide':
			return 'La demande est invalide.';
		case 'non_authentifie':
			return 'Votre session est fermée ou a expiré. Reconnectez-vous.';
		case 'interdit':
			return 'Action refusée par le serveur.';
		case 'introuvable':
			return 'Élément introuvable.';
		case 'methode_non_admise':
			return 'Opération non prise en charge par le serveur.';
		case 'corps_trop_grand':
			return 'La demande est trop volumineuse.';
		case 'trop_de_requetes':
			return erreur.reessayerDans === null
				? 'Trop de demandes. Réessayez dans un instant.'
				: `Trop de demandes. Réessayez dans ${erreur.reessayerDans} s.`;
		case 'erreur_interne':
			return 'Erreur du serveur. Réessayez plus tard.';
		case 'delai_depasse':
			return 'Le serveur n’a pas répondu à temps. Réessayez.';
		case 'reseau':
			return 'Serveur injoignable. Vérifiez votre connexion.';
		case 'reponse_invalide':
			return 'Réponse inattendue du serveur.';
	}
}
