import assert from 'node:assert/strict';
import { describe, it } from 'node:test';
import { ErreurApi } from './api/erreurs.ts';
import { executerSensible } from './fraicheur.ts';

const refus = () => new ErreurApi('interdit', 403);

function monter(options: { fraiche?: boolean[]; assurer?: boolean[] } = {}) {
	const fraiche = [...(options.fraiche ?? [])];
	const assurer = [...(options.assurer ?? [])];
	const compteurs = { assurer: 0, estFraiche: 0 };
	return {
		compteurs,
		dependances: {
			assurer: async () => {
				compteurs.assurer += 1;
				return assurer.shift() ?? true;
			},
			estFraiche: async () => {
				compteurs.estFraiche += 1;
				return fraiche.shift() ?? true;
			}
		}
	};
}

describe('action sensible', () => {
	it('succès direct : aucun rejeu', async () => {
		const { dependances, compteurs } = monter();
		let appels = 0;
		const issue = await executerSensible(async () => (appels += 1), dependances);
		assert.deepEqual(issue, { fait: true, valeur: 1 });
		assert.equal(compteurs.assurer, 1);
	});

	it('ré-authentification annulée : l’action n’est pas lancée', async () => {
		const { dependances } = monter({ assurer: [false] });
		let appels = 0;
		const issue = await executerSensible(async () => (appels += 1), dependances);
		assert.deepEqual(issue, { fait: false });
		assert.equal(appels, 0);
	});

	it('403 avec session devenue non fraîche : ré-authentification puis un seul rejeu', async () => {
		const { dependances, compteurs } = monter({ fraiche: [false] });
		let appels = 0;
		const issue = await executerSensible(async () => {
			appels += 1;
			if (appels === 1) throw refus();
			return 'ok';
		}, dependances);
		assert.deepEqual(issue, { fait: true, valeur: 'ok' });
		assert.equal(appels, 2);
		assert.equal(compteurs.assurer, 2);
	});

	it('pas de boucle : un second 403 remonte', async () => {
		const { dependances } = monter({ fraiche: [false] });
		let appels = 0;
		await assert.rejects(
			() =>
				executerSensible(async () => {
					appels += 1;
					throw refus();
				}, dependances),
			ErreurApi
		);
		assert.equal(appels, 2);
	});

	it('403 avec session fraîche : refus ordinaire, sans rejeu', async () => {
		const { dependances } = monter({ fraiche: [true] });
		let appels = 0;
		await assert.rejects(
			() =>
				executerSensible(async () => {
					appels += 1;
					throw refus();
				}, dependances),
			ErreurApi
		);
		assert.equal(appels, 1);
	});

	it('autre erreur : remonte sans relire la session', async () => {
		const { dependances, compteurs } = monter();
		await assert.rejects(
			() =>
				executerSensible(async () => {
					throw new ErreurApi('conflit', 409);
				}, dependances),
			ErreurApi
		);
		assert.equal(compteurs.estFraiche, 0);
	});

	it('rejeu : ré-authentification annulée', async () => {
		const { dependances } = monter({ fraiche: [false], assurer: [true, false] });
		const issue = await executerSensible(async () => {
			throw refus();
		}, dependances);
		assert.deepEqual(issue, { fait: false });
	});
});
