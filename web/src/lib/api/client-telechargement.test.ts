import assert from 'node:assert/strict';
import { describe, it } from 'node:test';
import { creerClient, EN_TETE_CSRF } from './client.ts';
import { ErreurApi } from './erreurs.ts';

const session = {
	id: '11111111-2222-4333-8444-555555555555',
	nature: 'complete',
	methode: 'passkey',
	cree_le: '2026-11-19T08:00:00.000000Z',
	fin_absolue: '2026-11-19T20:00:00.000000Z',
	fin_inactivite: '2026-11-19T08:30:00.000000Z',
	derniere_verification: '2026-11-19T08:00:00.000000Z',
	fraiche: true,
	fraiche_jusqu_a: '2026-11-19T08:05:00.000000Z',
	csrf: 'jeton-1'
};

describe('téléchargement d’un fichier', () => {
	it('POST : jeton anti-CSRF joint, corps lu en Blob, en-têtes rendus', async () => {
		const vus: { methode: string; chemin: string; csrf: string | null }[] = [];
		const fetch: typeof globalThis.fetch = async (entree, init = {}) => {
			const chemin = String(entree);
			const entetes = new Headers(init.headers);
			vus.push({ methode: init.method ?? 'GET', chemin, csrf: entetes.get(EN_TETE_CSRF) });
			if (chemin === '/api/session') return Response.json(session);
			return new Response('contenu', { headers: { 'x-sigma-export-tronque': 'false' } });
		};
		const api = creerClient({ fetch });
		const f = await api.telecharger('POST', '/api/cabinets/c/dossiers/d/integrite/export');
		assert.equal(await f.contenu.text(), 'contenu');
		assert.equal(f.entetes.get('x-sigma-export-tronque'), 'false');
		const export_ = vus.at(-1);
		assert.equal(export_?.methode, 'POST');
		assert.equal(export_?.csrf, 'jeton-1');
	});

	it('refus : erreur traduite, sans corps', async () => {
		const fetch: typeof globalThis.fetch = async (entree) =>
			String(entree) === '/api/session'
				? Response.json(session)
				: new Response('{"erreur":"interdit"}', { status: 403 });
		const api = creerClient({ fetch });
		await assert.rejects(api.telecharger('GET', '/api/x/temoin'), (e: unknown) => {
			assert.ok(e instanceof ErreurApi);
			assert.equal(e.statut, 403);
			return true;
		});
	});

	it('chemin hors de l’API refusé', async () => {
		const api = creerClient({ fetch: async () => new Response('') });
		await assert.rejects(api.telecharger('GET', 'https://exemple.test/x'), TypeError);
	});
});
