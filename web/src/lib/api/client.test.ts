// Tests du client d'API : jeton anti-CSRF en mémoire, rotation, reprises, traduction des erreurs.
// Exécutés par `node --test` (types retirés par Node), contre un faux serveur qui reproduit le
// contrat de crates/api (session.rs, moi.rs, csrf.rs, erreur.rs).

import assert from 'node:assert/strict';
import { afterEach, beforeEach, describe, it, mock } from 'node:test';
import { creerClient, EN_TETE_CSRF } from './client.ts';
import { ErreurApi } from './erreurs.ts';
import { creerDossier, listerDossiers, revoquerSession } from './ressources.ts';
import { estDossier, lireUuid, type Uuid } from './types.ts';

const CABINET = lireUuid('0b0f6c43-7c47-4a43-9d1e-2f4f6f0a1b2c') as Uuid;

interface Appel {
	methode: string;
	chemin: string;
	csrf: string | null;
	/** Jeton du cookie au moment de l'envoi (le navigateur le joint seul). */
	cookie: string;
	init: RequestInit;
}

function json(statut: number, corps: unknown, entetes: Record<string, string> = {}): Response {
	return new Response(JSON.stringify(corps), {
		status: statut,
		headers: { 'content-type': 'application/json', ...entetes }
	});
}

const csrfDe = (jeton: string) => `csrf-${jeton}`;

const SESSION_1 = '11111111-2222-4333-8444-555555555555';

function etatSession(jeton: string, id: string) {
	return {
		id,
		nature: 'complete',
		methode: 'passkey',
		cree_le: '2026-11-19T08:00:00.000000Z',
		fin_absolue: '2026-11-19T20:00:00.000000Z',
		fin_inactivite: '2026-11-19T08:30:00.000000Z',
		derniere_verification: '2026-11-19T08:00:00.000000Z',
		fraiche: true,
		fraiche_jusqu_a: '2026-11-19T08:05:00.000000Z',
		csrf: csrfDe(jeton)
	};
}

/**
 * Faux serveur à l'image du vrai : cookie de session (jeton), anti-CSRF dérivé du jeton, rotation
 * qui remplace le jeton, 401 pour un cookie inconnu, 403 pour un anti-CSRF faux.
 */
function fauxServeur(
	routes: (appel: Appel) => Response | Promise<Response> | undefined = () => undefined
) {
	const etat = { jeton: 'j1', idSession: SESSION_1, session: true, rotations: 0 };
	const appels: Appel[] = [];
	const fetch: typeof globalThis.fetch = async (entree, init = {}) => {
		const entetes = new Headers(init.headers);
		const appel: Appel = {
			methode: init.method ?? 'GET',
			chemin: String(entree),
			csrf: entetes.get(EN_TETE_CSRF),
			cookie: etat.jeton,
			init
		};
		appels.push(appel);
		const reponse = await routes(appel);
		if (reponse !== undefined) return reponse;
		if (!etat.session || appel.cookie !== etat.jeton)
			return json(401, { erreur: 'non_authentifie' });
		if (appel.methode !== 'GET' && appel.csrf !== csrfDe(etat.jeton)) {
			return json(403, { erreur: 'interdit' });
		}
		if (appel.chemin === '/api/session') return json(200, etatSession(etat.jeton, etat.idSession));
		if (appel.chemin === '/api/session/rotation') {
			etat.rotations += 1;
			etat.jeton = `j${etat.rotations + 1}`;
			return json(200, { csrf: csrfDe(etat.jeton) });
		}
		if (appel.chemin === '/api/session/deconnexion') {
			etat.session = false;
			return new Response(null, { status: 204 });
		}
		if (appel.chemin === `/api/cabinets/${CABINET}/dossiers` && appel.methode === 'GET') {
			return json(200, []);
		}
		if (appel.chemin === `/api/cabinets/${CABINET}/dossiers` && appel.methode === 'POST') {
			return json(201, { id: 'aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee', nom: 'SARL Exemple' });
		}
		if (appel.chemin.startsWith('/api/moi/sessions/') && appel.methode === 'DELETE') {
			return new Response(null, { status: 204 });
		}
		return json(404, { erreur: 'introuvable' });
	};
	return { etat, appels, fetch };
}

/** Promesse résolue de l'extérieur, pour retenir une réponse du faux serveur. */
function differe<T>() {
	let resoudre!: (valeur: T) => void;
	const promesse = new Promise<T>((r) => (resoudre = r));
	return { promesse, resoudre };
}

const espions: ReturnType<typeof mock.method>[] = [];
beforeEach(() => {
	for (const nom of ['log', 'info', 'warn', 'error', 'debug'] as const) {
		espions.push(mock.method(console, nom));
	}
});
afterEach(() => {
	// Aucun appel du client n'écrit dans la console (jetons, réponses).
	for (const espion of espions.splice(0)) {
		assert.equal(espion.mock.callCount(), 0);
		espion.mock.restore();
	}
});

describe('session et jeton anti-CSRF', () => {
	it('lit la session de la même origine et ne rend pas le jeton anti-CSRF', async () => {
		const serveur = fauxServeur();
		const api = creerClient({ fetch: serveur.fetch });
		assert.equal(api.aJetonCsrf(), false);

		const etat = await api.ouvrirSession();

		assert.equal(etat.fin_inactivite, '2026-11-19T08:30:00.000000Z');
		assert.equal('csrf' in etat, false);
		assert.equal(api.aJetonCsrf(), true);
		const [appel] = serveur.appels;
		assert.equal(appel?.init.credentials, 'same-origin');
		assert.equal(appel?.csrf, null, 'aucun jeton sur une lecture');
		// Le jeton ne sort par aucune voie : ni dans l'état rendu, ni par une propriété du client,
		// qui n'expose que ses méthodes.
		assert.doesNotMatch(JSON.stringify(etat), /csrf-j1/);
		assert.deepEqual(Object.keys(api).sort(), [
			'aJetonCsrf',
			'deconnecter',
			'lire',
			'muter',
			'muterSansReponse',
			'muterSansSession',
			'ouvrirSession',
			'renouveler',
			'telecharger',
			'tourner'
		]);
		assert.ok(Object.values(api).every((v) => typeof v === 'function'));
	});

	it('envoie le jeton gardé en mémoire sur toute mutation, et le lit d’abord si besoin', async () => {
		const serveur = fauxServeur();
		const api = creerClient({ fetch: serveur.fetch });

		await creerDossier(api, CABINET, 'SARL Exemple');
		await revoquerSession(api, lireUuid('aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee') as Uuid);

		assert.deepEqual(
			serveur.appels.map((a) => [a.methode, a.chemin, a.csrf]),
			[
				['GET', '/api/session', null],
				['POST', `/api/cabinets/${CABINET}/dossiers`, 'csrf-j1'],
				['DELETE', '/api/moi/sessions/aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee', 'csrf-j1']
			]
		);
		const post = serveur.appels[1];
		assert.equal(post?.init.body, JSON.stringify({ nom: 'SARL Exemple' }));
		assert.equal(new Headers(post?.init.headers).get('content-type'), 'application/json');
	});

	it('ne lit qu’une fois la session pour des mutations simultanées', async () => {
		const serveur = fauxServeur();
		const api = creerClient({ fetch: serveur.fetch });

		await Promise.all([creerDossier(api, CABINET, 'A'), creerDossier(api, CABINET, 'B')]);

		assert.equal(serveur.appels.filter((a) => a.chemin === '/api/session').length, 1);
	});

	it('oublie le jeton à la déconnexion', async () => {
		const serveur = fauxServeur();
		const api = creerClient({ fetch: serveur.fetch });
		await api.ouvrirSession();

		await api.deconnecter();

		assert.equal(api.aJetonCsrf(), false);
		const dernier = serveur.appels.at(-1);
		assert.deepEqual(
			[dernier?.methode, dernier?.chemin, dernier?.csrf],
			['POST', '/api/session/deconnexion', 'csrf-j1']
		);
	});
});

describe('rotation', () => {
	it('renouvelle le jeton anti-CSRF après la rotation', async () => {
		const serveur = fauxServeur();
		const api = creerClient({ fetch: serveur.fetch });
		await api.ouvrirSession();

		await api.tourner(CABINET);
		await creerDossier(api, CABINET, 'SARL Exemple');

		const rotation = serveur.appels.find((a) => a.chemin === '/api/session/rotation');
		assert.equal(rotation?.methode, 'POST');
		assert.equal(rotation?.csrf, 'csrf-j1', 'la rotation est une mutation : ancien jeton');
		assert.equal(serveur.appels.at(-1)?.csrf, 'csrf-j2');
		assert.equal(serveur.etat.rotations, 1);
	});

	it('attend les requêtes en cours et retient les suivantes', async () => {
		const retenue = differe<Response>();
		let premiere = true;
		const serveur = fauxServeur((appel) => {
			if (appel.chemin === `/api/cabinets/${CABINET}/dossiers` && premiere) {
				premiere = false;
				return retenue.promesse;
			}
			return undefined;
		});
		const api = creerClient({ fetch: serveur.fetch });
		await api.ouvrirSession();

		const enCours = listerDossiers(api, CABINET);
		await new Promise((r) => setImmediate(r));
		const rotation = api.tourner(CABINET);
		const suivante = listerDossiers(api, CABINET);
		await new Promise((r) => setImmediate(r));

		// Rien n'est parti tant que la requête en cours n'a pas abouti.
		assert.deepEqual(
			serveur.appels.map((a) => a.chemin),
			['/api/session', `/api/cabinets/${CABINET}/dossiers`]
		);
		retenue.resoudre(json(200, []));
		await Promise.all([enCours, rotation, suivante]);

		assert.deepEqual(
			serveur.appels.map((a) => [a.chemin, a.cookie]),
			[
				['/api/session', 'j1'],
				[`/api/cabinets/${CABINET}/dossiers`, 'j1'],
				['/api/session/rotation', 'j1'],
				[`/api/cabinets/${CABINET}/dossiers`, 'j2']
			]
		);
	});

	it('partage une rotation en cours vers le même cabinet', async () => {
		const serveur = fauxServeur();
		const api = creerClient({ fetch: serveur.fetch });
		await api.ouvrirSession();

		await Promise.all([api.tourner('A'), api.tourner('A')]);

		assert.equal(serveur.etat.rotations, 1);
	});

	it('deux cabinets demandés coup sur coup : deux rotations, l’une après l’autre', async () => {
		const serveur = fauxServeur();
		const api = creerClient({ fetch: serveur.fetch });
		await api.ouvrirSession();

		await Promise.all([api.tourner('A'), api.tourner('B')]);

		assert.equal(serveur.etat.rotations, 2);
		assert.deepEqual(
			serveur.appels
				.filter((a) => a.chemin === '/api/session/rotation')
				.map((a) => [a.cookie, a.csrf]),
			[
				['j1', 'csrf-j1'],
				['j2', 'csrf-j2']
			]
		);
	});

	it('rotation concurrente d’un autre onglet : 401 sur la rotation, puis reprise', async () => {
		let premiere = true;
		const serveur = fauxServeur((appel) => {
			if (appel.chemin === '/api/session/rotation' && premiere) {
				// L'autre onglet a tourné le jeton juste avant : le cookie envoyé est périmé.
				premiere = false;
				serveur.etat.jeton = 'j7';
				return json(401, { erreur: 'non_authentifie' });
			}
			return undefined;
		});
		const surNonAuthentifie = mock.fn();
		const api = creerClient({ fetch: serveur.fetch, surNonAuthentifie });
		await api.ouvrirSession();

		await api.tourner(CABINET);
		await creerDossier(api, CABINET, 'X');

		assert.equal(surNonAuthentifie.mock.callCount(), 0);
		assert.deepEqual(
			serveur.appels.map((a) => [a.methode, a.chemin, a.cookie, a.csrf]),
			[
				['GET', '/api/session', 'j1', null],
				['POST', '/api/session/rotation', 'j1', 'csrf-j1'],
				['GET', '/api/session', 'j7', null],
				['POST', '/api/session/rotation', 'j7', 'csrf-j7'],
				['POST', `/api/cabinets/${CABINET}/dossiers`, 'j2', 'csrf-j2']
			]
		);
	});
});

describe('reprises après une rotation faite ailleurs (autre onglet)', () => {
	it('403 sur une mutation avec un jeton périmé : relit la session et reprend une fois', async () => {
		const serveur = fauxServeur();
		const api = creerClient({ fetch: serveur.fetch });
		await api.ouvrirSession();
		serveur.etat.jeton = 'j9'; // le cookie a tourné dans un autre onglet

		const dossier = await creerDossier(api, CABINET, 'SARL Exemple');

		assert.equal(dossier.nom, 'SARL Exemple');
		assert.deepEqual(
			serveur.appels.slice(1).map((a) => [a.methode, a.chemin, a.csrf]),
			[
				['POST', `/api/cabinets/${CABINET}/dossiers`, 'csrf-j1'],
				['GET', '/api/session', null],
				['POST', `/api/cabinets/${CABINET}/dossiers`, 'csrf-j9']
			]
		);
	});

	it('403 avec un jeton à jour : refus rendu tel quel, sans reprise', async () => {
		const serveur = fauxServeur((appel) =>
			appel.methode === 'POST' && appel.chemin.endsWith('/dossiers')
				? json(403, { erreur: 'interdit' })
				: undefined
		);
		const api = creerClient({ fetch: serveur.fetch });
		await api.ouvrirSession();

		await assert.rejects(creerDossier(api, CABINET, 'X'), (e: unknown) => {
			assert.ok(e instanceof ErreurApi);
			assert.equal(e.code, 'interdit');
			assert.equal(e.statut, 403);
			return true;
		});
		assert.equal(serveur.appels.filter((a) => a.methode === 'POST').length, 1);
	});

	it('401 sur une requête partie avec l’ancien cookie : reprise si la session est valide', async () => {
		let tournee = false;
		const serveur = fauxServeur((appel) => {
			if (appel.chemin.endsWith('/dossiers') && !tournee) {
				// La rotation d'un autre onglet aboutit pendant que la requête est en route.
				tournee = true;
				serveur.etat.jeton = 'j2';
				return json(401, { erreur: 'non_authentifie' });
			}
			return undefined;
		});
		const surNonAuthentifie = mock.fn();
		const api = creerClient({ fetch: serveur.fetch, surNonAuthentifie });
		await api.ouvrirSession();

		assert.deepEqual(await listerDossiers(api, CABINET), []);
		assert.equal(surNonAuthentifie.mock.callCount(), 0);
		assert.deepEqual(
			serveur.appels.map((a) => [a.chemin, a.cookie]),
			[
				['/api/session', 'j1'],
				[`/api/cabinets/${CABINET}/dossiers`, 'j1'],
				['/api/session', 'j2'],
				[`/api/cabinets/${CABINET}/dossiers`, 'j2']
			]
		);
	});
});

describe('autre session (déconnexion puis connexion dans un autre onglet)', () => {
	it('mutation refusée : jamais rejouée sous la nouvelle session', async () => {
		const serveur = fauxServeur();
		const surNonAuthentifie = mock.fn();
		const api = creerClient({ fetch: serveur.fetch, surNonAuthentifie });
		await api.ouvrirSession();
		// Autre onglet : l'utilisateur 1 se déconnecte, l'utilisateur 2 se connecte.
		serveur.etat.jeton = 'u2';
		serveur.etat.idSession = '22222222-3333-4444-8555-666666666666';

		await assert.rejects(creerDossier(api, CABINET, 'Dossier de l’utilisateur 1'), {
			code: 'non_authentifie'
		});

		assert.equal(serveur.appels.filter((a) => a.methode === 'POST').length, 1);
		assert.equal(surNonAuthentifie.mock.callCount(), 1);
		assert.equal(api.aJetonCsrf(), false);
	});

	it('lecture refusée (401) : pas de reprise non plus', async () => {
		let premiere = true;
		const serveur = fauxServeur((appel) => {
			if (appel.chemin.endsWith('/dossiers') && premiere) {
				premiere = false;
				serveur.etat.jeton = 'u2';
				serveur.etat.idSession = '22222222-3333-4444-8555-666666666666';
				return json(401, { erreur: 'non_authentifie' });
			}
			return undefined;
		});
		const surNonAuthentifie = mock.fn();
		const api = creerClient({ fetch: serveur.fetch, surNonAuthentifie });
		await api.ouvrirSession();

		await assert.rejects(listerDossiers(api, CABINET), { code: 'non_authentifie' });
		assert.equal(serveur.appels.filter((a) => a.chemin.endsWith('/dossiers')).length, 1);
		assert.equal(surNonAuthentifie.mock.callCount(), 1);
	});
});

describe('non authentifié', () => {
	it('401 sur une mutation : ni reprise ni nouvel envoi, connexion demandée', async () => {
		const serveur = fauxServeur();
		const surNonAuthentifie = mock.fn();
		const api = creerClient({ fetch: serveur.fetch, surNonAuthentifie });
		await api.ouvrirSession();
		serveur.etat.session = false;

		await assert.rejects(creerDossier(api, CABINET, 'X'), { code: 'non_authentifie', statut: 401 });

		assert.deepEqual(
			serveur.appels.slice(1).map((a) => [a.methode, a.chemin]),
			[
				['POST', `/api/cabinets/${CABINET}/dossiers`],
				['GET', '/api/session']
			]
		);
		assert.equal(api.aJetonCsrf(), false);
		assert.equal(surNonAuthentifie.mock.callCount(), 1);
	});

	it('401 définitif : jeton oublié et page de connexion demandée une fois', async () => {
		const serveur = fauxServeur();
		const surNonAuthentifie = mock.fn();
		const api = creerClient({ fetch: serveur.fetch, surNonAuthentifie });
		await api.ouvrirSession();
		serveur.etat.session = false;

		await assert.rejects(listerDossiers(api, CABINET), { code: 'non_authentifie', statut: 401 });
		assert.equal(api.aJetonCsrf(), false);
		assert.equal(surNonAuthentifie.mock.callCount(), 1);
	});

	it('401 à la lecture de la session au démarrage', async () => {
		const serveur = fauxServeur();
		serveur.etat.session = false;
		const surNonAuthentifie = mock.fn();
		const api = creerClient({ fetch: serveur.fetch, surNonAuthentifie });

		await assert.rejects(api.ouvrirSession(), { code: 'non_authentifie' });
		assert.equal(surNonAuthentifie.mock.callCount(), 1);
		assert.equal(serveur.appels.length, 1);
	});
});

describe('traduction des erreurs', () => {
	async function erreurPour(reponse: Response | Error): Promise<ErreurApi> {
		const api = creerClient({
			fetch: async () => {
				if (reponse instanceof Error) throw reponse;
				return reponse;
			}
		});
		try {
			await listerDossiers(api, CABINET);
		} catch (e) {
			assert.ok(e instanceof ErreurApi);
			return e;
		}
		assert.fail('une erreur était attendue');
	}

	it('404 : « introuvable », que la ressource soit absente ou d’autrui', async () => {
		const e = await erreurPour(json(404, { erreur: 'introuvable' }));
		assert.deepEqual([e.code, e.statut], ['introuvable', 404]);
	});

	it('le statut décide pour 403 et 404, quel que soit le corps', async () => {
		assert.equal((await erreurPour(json(403, { erreur: 'introuvable' }))).code, 'interdit');
		assert.equal((await erreurPour(json(404, { erreur: 'interdit' }))).code, 'introuvable');
	});

	it('code connu du corps retenu, code inconnu ou corps illisible : le statut', async () => {
		assert.equal((await erreurPour(json(503, { erreur: 'delai_depasse' }))).code, 'delai_depasse');
		assert.equal((await erreurPour(json(500, { erreur: 'autre' }))).code, 'erreur_interne');
		assert.equal(
			(await erreurPour(new Response('<html>', { status: 502 }))).code,
			'erreur_interne'
		);
		assert.equal((await erreurPour(json(413, {}))).code, 'corps_trop_grand');
	});

	it('429 : délai de `Retry-After`', async () => {
		const e = await erreurPour(json(429, { erreur: 'trop_de_requetes' }, { 'retry-after': '7' }));
		assert.deepEqual([e.code, e.reessayerDans], ['trop_de_requetes', 7]);
	});

	it('serveur injoignable : « reseau »', async () => {
		const e = await erreurPour(new TypeError('Failed to fetch'));
		assert.deepEqual([e.code, e.statut], ['reseau', null]);
	});

	it('réponse d’une forme inattendue : « reponse_invalide »', async () => {
		assert.equal(
			(await erreurPour(json(200, [{ id: '../x', nom: 'X' }]))).code,
			'reponse_invalide'
		);
		assert.equal((await erreurPour(new Response('pas du json'))).code, 'reponse_invalide');
	});

	it('le message de l’erreur ne porte que son code', async () => {
		const serveur = fauxServeur((appel) =>
			appel.methode === 'POST' ? json(403, { erreur: 'interdit', csrf: 'csrf-j1' }) : undefined
		);
		const api = creerClient({ fetch: serveur.fetch });
		await assert.rejects(creerDossier(api, CABINET, 'X'), (e: unknown) => {
			assert.ok(e instanceof ErreurApi);
			assert.equal(e.message, 'interdit');
			assert.doesNotMatch(JSON.stringify(e) + String(e), /csrf-|j1/);
			return true;
		});
	});
});

describe('chemins', () => {
	it('refuse un chemin hors de l’API', async () => {
		const serveur = fauxServeur();
		const api = creerClient({ fetch: serveur.fetch });
		await assert.rejects(api.lire('https://exemple.invalid/api/x', estDossier), TypeError);
		await assert.rejects(api.lire('/api/../connexion', estDossier), TypeError);
		assert.equal(serveur.appels.length, 0);
	});
});

describe('mutations sans session et ré-authentification', () => {
	it('connexion : en-tête factice, 401 ordinaire sans renvoi vers la connexion', async () => {
		let renvois = 0;
		const appels: Appel[] = [];
		const { fetch } = fauxServeur((appel) => {
			appels.push(appel);
			return json(401, { erreur: 'non_authentifie' });
		});
		const client = creerClient({ fetch, surNonAuthentifie: () => (renvois += 1) });
		await assert.rejects(
			() => client.muterSansSession('/api/connexion/secours', estDossier, { courriel: 'a@b.sn' }),
			(e: unknown) => e instanceof ErreurApi && e.code === 'non_authentifie'
		);
		assert.equal(renvois, 0);
		assert.equal(appels.length, 1);
		assert.ok(appels[0]?.csrf);
	});

	it('refuse les chemins hors connexion et lien', async () => {
		const { fetch } = fauxServeur();
		const client = creerClient({ fetch });
		await assert.rejects(
			() => client.muterSansSession('/api/moi/facteurs', estDossier, {}),
			TypeError
		);
	});

	it('renouveler : le jeton anti-CSRF de la réponse sert aux mutations suivantes', async () => {
		const { fetch, appels } = fauxServeur((appel) => {
			if (appel.chemin === '/api/session/reauthentification') {
				return json(200, { csrf: 'csrf-neuf', fraiche_jusqu_a: '2026-11-19T08:10:00.000000Z' });
			}
			return undefined;
		});
		const client = creerClient({ fetch });
		await client.ouvrirSession();
		await client.renouveler('/api/session/reauthentification', { mot_de_passe: 'x', code: '1' });
		await client.muterSansReponse('DELETE', `/api/moi/sessions/${SESSION_1}`).catch(() => {});
		assert.equal(appels.find((a) => a.methode === 'DELETE')?.csrf, 'csrf-neuf');
	});
});
