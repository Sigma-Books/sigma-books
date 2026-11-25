import assert from 'node:assert/strict';
import { test } from 'node:test';
import {
	additionner,
	depuisUnites,
	ecart,
	estNul,
	formaterMontant,
	lireMontant,
	versUnites
} from './montants.ts';

test('lireMontant normalise les saisies valides en chaîne canonique', () => {
	assert.equal(lireMontant('1500'), '1500');
	assert.equal(lireMontant('1 500,50'), '1500.5');
	assert.equal(lireMontant('1 500.500'), '1500.5');
	assert.equal(lireMontant('0012'), '12');
	assert.equal(lireMontant('0'), '0');
	assert.equal(lireMontant('0,10'), '0.1');
	assert.equal(lireMontant('  42  '), '42');
	assert.equal(lireMontant('100.00'), '100');
});

test('lireMontant refuse ce qui n’est pas un décimal positif borné', () => {
	for (const mauvais of ['', ' ', '-5', '+5', '1e3', '1,2,3', '12a', '.5', '5.', '0x10', '1.1234567']) {
		assert.equal(lireMontant(mauvais), null, mauvais);
	}
	assert.equal(lireMontant('9'.repeat(20)), '9'.repeat(20));
	assert.equal(lireMontant('9'.repeat(21)), null);
});

test('les sommes sont exactes (pas de flottant)', () => {
	assert.equal(additionner(['0.1', '0.2']), '0.3');
	assert.equal(additionner(['99999999999999999999', '1']), '100000000000000000000');
	assert.equal(additionner(['0.000001', '0.000002']), '0.000003');
	assert.equal(additionner([]), '0');
	assert.equal(additionner(['abc', '5']), '5');
	assert.equal(depuisUnites(versUnites('123.456')), '123.456');
	assert.equal(depuisUnites(-1500000n), '-1.5');
});

test('estNul', () => {
	assert.equal(estNul('0'), true);
	assert.equal(estNul('0.000001'), false);
});

test('ecart indique le sens à porter pour équilibrer', () => {
	assert.deepEqual(ecart(['100'], ['100']), {
		debit: '100',
		credit: '100',
		difference: '0',
		sens: 'equilibre'
	});
	const e = ecart(['100.10', ''], ['60', '40']);
	assert.equal(e.difference, '0.1');
	assert.equal(e.sens, 'credit');
	assert.equal(ecart(['10'], ['25']).sens, 'debit');
	assert.equal(ecart(['0.1', '0.2'], ['0.3']).sens, 'equilibre');
});

test('formaterMontant groupe les milliers et met la virgule', () => {
	assert.equal(formaterMontant('1234567.5'), '1 234 567,5');
	assert.equal(formaterMontant('12'), '12');
	assert.equal(formaterMontant('1000'), '1 000');
	assert.equal(formaterMontant('-1000.25'), '-1 000,25');
	assert.equal(formaterMontant('pas un nombre'), 'pas un nombre');
});
