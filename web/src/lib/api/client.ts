// Client de l'API (ADR 0012 ; F1.4 ; crates/api/src/serveur/csrf.rs, session.rs).
//
// - `fetch` de la même origine (`credentials: 'same-origin'`) : le cookie de session
//   `__Host-sigma_session` est `HttpOnly`, le script ne le voit jamais.
// - Le jeton anti-CSRF, reçu de `GET /api/session` ou de la rotation, vit **dans une variable de
//   cette fermeture seulement** : ni stockage du navigateur, ni état de l'interface, ni console,
//   ni message d'erreur. Il part dans l'en-tête `x-sigma-csrf` de toute mutation.
// - Rotation (`POST /api/session/rotation`) : elle remplace le cookie ; une requête partie avec
//   l'ancien serait refusée (401). La rotation attend donc la fin des requêtes en cours et retient
//   les suivantes jusqu'à ce que le nouveau jeton anti-CSRF soit connu.
// - Un autre onglet peut avoir tourné le jeton : sur un 401 (cookie remplacé pendant l'envoi) ou un
//   403 de mutation (jeton anti-CSRF périmé), la session est relue **une fois** ; la requête est
//   reprise seulement si la session est valide (et, pour le 403, si le jeton a changé). Un refus
//   ou une authentification échouée n'a aucun effet de bord côté serveur : la reprise est sûre.
// - 401 définitif : jeton oublié, puis `surNonAuthentifie` (page de connexion).
//
// Ce module n'importe rien de SvelteKit : il se teste sous `node --test`.

import { ErreurApi, traduireReponse } from './erreurs.ts';
import { estReponseRotation, estReponseSession, type EtatSession, type Garde } from './types.ts';

/** En-tête anti-CSRF exigé sur toute mutation (`csrf::EN_TETE_CSRF`). */
export const EN_TETE_CSRF = 'x-sigma-csrf';

export const CHEMIN_SESSION = '/api/session';
export const CHEMIN_ROTATION = '/api/session/rotation';
export const CHEMIN_DECONNEXION = '/api/session/deconnexion';

export type Mutation = 'POST' | 'DELETE';
type Methode = 'GET' | Mutation;

export interface OptionsClient {
	/** `fetch` à utiliser (celui du navigateur par défaut ; un faux dans les tests). */
	fetch?: typeof fetch;
	/** Appelé quand le serveur répond 401 de façon définitive. */
	surNonAuthentifie?: () => void;
}

export interface ClientApi {
	/** `GET` d'une ressource, dont la réponse doit passer la garde. */
	lire<T>(chemin: string, garde: Garde<T>): Promise<T>;
	/** Mutation dont la réponse JSON doit passer la garde. */
	muter<T>(methode: Mutation, chemin: string, garde: Garde<T>, corps?: unknown): Promise<T>;
	/** Mutation sans corps de réponse (204). */
	muterSansReponse(methode: Mutation, chemin: string, corps?: unknown): Promise<void>;
	/** Lit la session courante et retient son jeton anti-CSRF ; rend l'état sans le jeton. */
	ouvrirSession(): Promise<EtatSession>;
	/**
	 * Tourne le jeton de session et renouvelle le jeton anti-CSRF. À n'appeler que hors d'une
	 * requête de ce client (elle attend la fin des requêtes en cours).
	 */
	tourner(): Promise<void>;
	/** Ferme la session courante et oublie le jeton anti-CSRF. */
	deconnecter(): Promise<void>;
	/** Vrai si un jeton anti-CSRF est en mémoire. Le jeton lui-même n'est jamais exposé. */
	aJetonCsrf(): boolean;
}

export function creerClient(options: OptionsClient = {}): ClientApi {
	const appelFetch = options.fetch ?? ((entree, init) => globalThis.fetch(entree, init));
	const surNonAuthentifie = options.surNonAuthentifie ?? (() => {});

	let csrf: string | null = null;
	let lectureSession: Promise<EtatSession> | null = null;
	let rotation: Promise<void> | null = null;
	const enCours = new Set<Promise<unknown>>();

	function verifierChemin(chemin: string): void {
		if (!chemin.startsWith('/api/') || chemin.includes('..')) {
			throw new TypeError('chemin hors de l’API');
		}
	}

	/** Un aller-retour, sans reprise. Seule l'absence de réponse est convertie (`reseau`). */
	async function envoyer(
		methode: Methode,
		chemin: string,
		corps: unknown,
		jeton: string | null
	): Promise<Response> {
		const entetes: Record<string, string> = { accept: 'application/json' };
		if (jeton !== null) entetes[EN_TETE_CSRF] = jeton;
		const init: RequestInit = {
			method: methode,
			credentials: 'same-origin',
			cache: 'no-store',
			redirect: 'error',
			headers: entetes
		};
		if (corps !== undefined) {
			entetes['content-type'] = 'application/json';
			init.body = JSON.stringify(corps);
		}
		try {
			return await appelFetch(chemin, init);
		} catch {
			throw new ErreurApi('reseau', null);
		}
	}

	async function lireJson<T>(reponse: Response, garde: Garde<T>): Promise<T> {
		let valeur: unknown;
		try {
			valeur = await reponse.json();
		} catch {
			throw new ErreurApi('reponse_invalide', reponse.status);
		}
		if (!garde(valeur)) throw new ErreurApi('reponse_invalide', reponse.status);
		return valeur;
	}

	/** Relit la session (une seule lecture à la fois) et retient le jeton anti-CSRF. */
	function chargerSession(): Promise<EtatSession> {
		lectureSession ??= (async () => {
			try {
				const reponse = await envoyer('GET', CHEMIN_SESSION, undefined, null);
				if (!reponse.ok) {
					if (reponse.status === 401) csrf = null;
					throw await traduireReponse(reponse);
				}
				const { csrf: jeton, ...etat } = await lireJson(reponse, estReponseSession);
				csrf = jeton;
				return etat;
			} finally {
				lectureSession = null;
			}
		})();
		return lectureSession;
	}

	async function sessionValide(): Promise<boolean> {
		try {
			await chargerSession();
			return true;
		} catch {
			return false;
		}
	}

	async function jetonCsrf(): Promise<string> {
		if (csrf === null) await chargerSession();
		if (csrf === null) throw new ErreurApi('reponse_invalide', null);
		return csrf;
	}

	/** Requête avec au plus une reprise ; rend une réponse 2xx ou lève une {@link ErreurApi}. */
	async function executer(
		methode: Methode,
		chemin: string,
		corps: unknown,
		reprise = true
	): Promise<Response> {
		verifierChemin(chemin);
		const jeton = methode === 'GET' ? null : await jetonCsrf();
		const reponse = await envoyer(methode, chemin, corps, jeton);
		if (reponse.ok) return reponse;
		if (reprise && reponse.status === 401 && chemin !== CHEMIN_SESSION && (await sessionValide())) {
			return executer(methode, chemin, corps, false);
		}
		if (reprise && reponse.status === 403 && jeton !== null && (await sessionValide())) {
			if (csrf !== jeton) return executer(methode, chemin, corps, false);
		}
		const erreur = await traduireReponse(reponse);
		if (reponse.status === 401) {
			csrf = null;
			surNonAuthentifie();
		}
		throw erreur;
	}

	/** Retient la requête pendant une rotation, et la compte parmi les requêtes en cours. */
	async function suivre<T>(travail: () => Promise<T>): Promise<T> {
		while (rotation !== null) await rotation.catch(() => {});
		const promesse = travail();
		enCours.add(promesse);
		try {
			return await promesse;
		} finally {
			enCours.delete(promesse);
		}
	}

	return {
		lire<T>(chemin: string, garde: Garde<T>): Promise<T> {
			return suivre(async () => lireJson(await executer('GET', chemin, undefined), garde));
		},

		muter<T>(methode: Mutation, chemin: string, garde: Garde<T>, corps?: unknown): Promise<T> {
			return suivre(async () => lireJson(await executer(methode, chemin, corps), garde));
		},

		muterSansReponse(methode: Mutation, chemin: string, corps?: unknown): Promise<void> {
			return suivre(async () => {
				await executer(methode, chemin, corps);
			});
		},

		ouvrirSession(): Promise<EtatSession> {
			return suivre(async () => {
				try {
					return await chargerSession();
				} catch (erreur) {
					if (erreur instanceof ErreurApi && erreur.statut === 401) surNonAuthentifie();
					throw erreur;
				}
			});
		},

		tourner(): Promise<void> {
			if (rotation !== null) return rotation;
			const courante = (async () => {
				await Promise.allSettled([...enCours]);
				const reponse = await executer('POST', CHEMIN_ROTATION, undefined);
				csrf = (await lireJson(reponse, estReponseRotation)).csrf;
			})();
			rotation = courante;
			const liberer = () => {
				if (rotation === courante) rotation = null;
			};
			courante.then(liberer, liberer);
			return courante;
		},

		deconnecter(): Promise<void> {
			return suivre(async () => {
				try {
					await executer('POST', CHEMIN_DECONNEXION, undefined);
				} finally {
					csrf = null;
				}
			});
		},

		aJetonCsrf(): boolean {
			return csrf !== null;
		}
	};
}
