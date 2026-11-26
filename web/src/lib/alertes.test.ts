import assert from 'node:assert/strict';
import { describe, it } from 'node:test';
import { integriteIndisponible, peutDireAucuneAlerte } from './alertes.ts';

const ok = { ok: true } as const;
const refus = { ok: false, erreur: { code: 'interdit' } } as const;
const panne = { ok: false, erreur: { code: 'erreur_interne' } } as const;
const reseau = { ok: false, erreur: { code: 'reseau' } } as const;

describe('synthèse des alertes', () => {
	it('seul le refus de rôle est tu', () => {
		assert.equal(integriteIndisponible(ok), false);
		assert.equal(integriteIndisponible(refus), false);
		assert.equal(integriteIndisponible(panne), true);
		assert.equal(integriteIndisponible(reseau), true);
		assert.equal(integriteIndisponible(null), false);
	});

	it('« aucune alerte » jamais affirmé si une lecture a échoué', () => {
		assert.equal(peutDireAucuneAlerte(0, ok, ok), true);
		assert.equal(peutDireAucuneAlerte(0, ok, refus), true);
		assert.equal(peutDireAucuneAlerte(0, ok, panne), false);
		assert.equal(peutDireAucuneAlerte(0, panne, ok), false);
		assert.equal(peutDireAucuneAlerte(1, ok, ok), false);
	});
});
