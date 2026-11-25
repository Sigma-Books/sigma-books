// Gardes de l'administration et appels : chemins et corps conformes à `routes/administration.rs`.

import assert from 'node:assert/strict';
import { describe, it } from 'node:test';
import { creerClient } from './client.ts';
import {
	changerActivite,
	reinitialiserAcces,
	changerSeparationTaches,
	contesterReinitialisation
} from './ressources.ts';
import { estInvitation, estMembre, estReinitialisation, lireUuid, type Uuid } from './types.ts';

const ID = '0b0f6c43-7c47-4a43-9d1e-2f4f6f0a1b2c';
const CABINET = lireUuid(ID) as Uuid;

describe('gardes', () => {
	it('membre', () => {
		const m = { id: ID, nom: 'Awa', courriel: 'a@b.sn', role: 'valideur', actif: true };
		assert.ok(estMembre(m));
		assert.ok(!estMembre({ ...m, actif: 'oui' }));
		assert.ok(!estMembre({ ...m, id: '../x' }));
	});

	it('invitation et réinitialisation', () => {
		assert.ok(
			estInvitation({ id: ID, courriel: 'a@b.sn', nom: 'A', role: 'auditeur', cree_le: 'x' })
		);
		const r = {
			id: ID,
			membre: ID,
			demandee_par: null,
			motif: 'autre',
			cree_le: 'x',
			echeance: 'y'
		};
		assert.ok(estReinitialisation(r));
		assert.ok(!estReinitialisation({ ...r, membre: 'nope' }));
	});
});

describe('appels', () => {
	function client() {
		const appels: { methode: string; chemin: string; corps: unknown }[] = [];
		const fetch: typeof globalThis.fetch = async (entree, init = {}) => {
			const chemin = String(entree);
			if (chemin === '/api/session') {
				return Response.json({
					id: ID,
					nature: 'complete',
					methode: 'passkey',
					cree_le: 'x',
					fin_absolue: 'x',
					fin_inactivite: 'x',
					derniere_verification: 'x',
					fraiche: true,
					fraiche_jusqu_a: 'x',
					csrf: 'c'
				});
			}
			appels.push({
				methode: init.method ?? 'GET',
				chemin,
				corps: typeof init.body === 'string' ? JSON.parse(init.body) : undefined
			});
			return Response.json({ id: ID, nom: 'A', courriel: 'a@b', role: 'auditeur', actif: false });
		};
		return { api: creerClient({ fetch }), appels };
	}

	it('suspension en POST, rétablissement en DELETE', async () => {
		const { api, appels } = client();
		await changerActivite(api, CABINET, CABINET, false);
		await changerActivite(api, CABINET, CABINET, true);
		assert.deepEqual(
			appels.map((a) => a.methode),
			['POST', 'DELETE']
		);
		assert.ok(appels.every((a) => a.chemin.endsWith('/suspension')));
	});

	it('réinitialisation : la vérification hors bande part avec la demande', async () => {
		const { api, appels } = client();
		await reinitialiserAcces(api, CABINET, CABINET, { motif_code: 'autre', motif: 'ok' });
		assert.deepEqual(appels[0]?.corps, {
			motif_code: 'autre',
			motif: 'ok',
			verification_hors_bande: true
		});
	});

	it('séparation des tâches et contestation', async () => {
		const { api, appels } = client();
		await changerSeparationTaches(api, CABINET, true);
		await changerSeparationTaches(api, CABINET, false);
		await contesterReinitialisation(api, CABINET, CABINET);
		assert.deepEqual(
			appels.map((a) => `${a.methode} ${a.chemin.replace(`/api/cabinets/${CABINET}`, '')}`),
			[
				'POST /separation-taches',
				'DELETE /separation-taches',
				`POST /reinitialisations/${ID}/contestation`
			]
		);
	});
});
