import assert from 'node:assert/strict';
import { test } from 'node:test';
import { longueurOctets } from './texte.ts';

test('la longueur est comptée en octets UTF-8, pas en unités UTF-16', () => {
	assert.equal(longueurOctets('abc'), 3);
	assert.equal(longueurOctets('é'.repeat(300)), 600);
	assert.equal('é'.repeat(300).length, 300);
	assert.equal(longueurOctets('€'), 3);
	assert.equal(longueurOctets('😀'), 4);
	assert.ok(longueurOctets('é'.repeat(300)) > 400);
});
