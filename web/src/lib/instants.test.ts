import assert from 'node:assert/strict';
import { describe, it } from 'node:test';
import { lireInstant, minutesRestantes } from './instants.ts';

describe('instants de l’API', () => {
	it('lit le format de l’audit, microsecondes tronquées à la milliseconde', () => {
		assert.equal(
			lireInstant('2026-11-19T08:30:00.123456Z')?.toISOString(),
			'2026-11-19T08:30:00.123Z'
		);
		assert.equal(lireInstant('2026-11-19T08:30:00Z')?.toISOString(), '2026-11-19T08:30:00.000Z');
	});

	it('refuse les autres formes et les dates impossibles', () => {
		for (const texte of [
			'',
			'2026-11-19',
			'2026-11-19T08:30:00',
			'2026-11-19T08:30:00+01:00',
			'2026-02-30T08:30:00Z',
			'2026-11-19T24:00:00Z'
		]) {
			assert.equal(lireInstant(texte), null, texte);
		}
	});

	it('minutes restantes, jamais négatives', () => {
		const maintenant = Date.UTC(2026, 10, 19, 8, 0, 0);
		assert.equal(minutesRestantes('2026-11-19T08:04:59.000000Z', maintenant), 4);
		assert.equal(minutesRestantes('2026-11-19T07:00:00.000000Z', maintenant), 0);
		assert.equal(minutesRestantes('illisible', maintenant), null);
	});
});
