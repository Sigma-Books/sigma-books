import assert from 'node:assert/strict';
import { test } from 'node:test';
import { afficherDate, dateDuJour, estDateIso } from './dates.ts';
import { regionalDe } from './regional-exemples.ts';

const FR = regionalDe({});

test('dateDuJour donne la date du fuseau du dossier, pas celle du navigateur', () => {
	// 23 h 30 UTC : encore le 5 à Dakar (UTC), déjà le 6 à Douala et à Moroni (UTC+1 et UTC+3).
	const instant = new Date(Date.UTC(2026, 2, 5, 23, 30));
	assert.equal(dateDuJour('Africa/Dakar', instant), '2026-03-05');
	assert.equal(dateDuJour('Africa/Douala', instant), '2026-03-06');
	assert.equal(dateDuJour('Indian/Comoro', instant), '2026-03-06');
	assert.equal(dateDuJour('UTC', new Date(Date.UTC(2026, 11, 31, 0, 0))), '2026-12-31');
});

test('estDateIso refuse les jours inexistants', () => {
	assert.equal(estDateIso('2026-02-28'), true);
	assert.equal(estDateIso('2024-02-29'), true);
	assert.equal(estDateIso('2026-02-30'), false);
	assert.equal(estDateIso('2026-13-01'), false);
	assert.equal(estDateIso('26-1-1'), false);
});

test('afficherDate suit le locale du dossier', () => {
	assert.equal(afficherDate('2026-03-05', FR), '05/03/2026');
	assert.equal(afficherDate('2026-11-19', regionalDe({ langue: 'en', pays: 'CM' })), '19/11/2026');
	assert.equal(afficherDate('2026-11-19', regionalDe({ langue: 'en', pays: 'US' })), '11/19/2026');
	assert.equal(afficherDate('2026-11-19', regionalDe({ langue: 'pt', pays: 'GW' })), '19/11/2026');
	assert.equal(afficherDate('n’importe quoi', FR), 'n’importe quoi');
	assert.equal(afficherDate('2026-02-30', FR), '2026-02-30');
});
