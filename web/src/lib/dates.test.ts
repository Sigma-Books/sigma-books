import assert from 'node:assert/strict';
import { test } from 'node:test';
import { afficherDate, dateIso, estDateIso } from './dates.ts';

test('dateIso met la date locale au format de l’API', () => {
	assert.equal(dateIso(new Date(2026, 2, 5)), '2026-03-05');
	assert.equal(dateIso(new Date(2026, 11, 31)), '2026-12-31');
});

test('estDateIso refuse les jours inexistants', () => {
	assert.equal(estDateIso('2026-02-28'), true);
	assert.equal(estDateIso('2024-02-29'), true);
	assert.equal(estDateIso('2026-02-30'), false);
	assert.equal(estDateIso('2026-13-01'), false);
	assert.equal(estDateIso('26-1-1'), false);
});

test('afficherDate', () => {
	assert.equal(afficherDate('2026-03-05'), '05/03/2026');
	assert.equal(afficherDate('n’importe quoi'), 'n’importe quoi');
});
