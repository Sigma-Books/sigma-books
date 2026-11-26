import assert from 'node:assert/strict';
import { test } from 'node:test';
import { LANGUES, lireRegional, localeDe, REGIONAL_DEFAUT, separateurs } from './regional.ts';

test('sans contexte, le dossier est sénégalais : XOF, 0 décimale, fr, Africa/Dakar', () => {
	for (const brut of [undefined, null, {}, 'x', 42, []]) {
		assert.deepEqual(lireRegional(brut), REGIONAL_DEFAUT);
	}
	assert.deepEqual(REGIONAL_DEFAUT, {
		pays: 'SN',
		devise: 'XOF',
		decimales: 0,
		fuseau: 'Africa/Dakar',
		langue: 'fr'
	});
});

test('chaque champ valide est repris, chaque champ invalide prend sa valeur par défaut', () => {
	assert.deepEqual(
		lireRegional({
			pays: 'CD',
			devise: 'CDF',
			decimales: 2,
			fuseau: 'Africa/Kinshasa',
			langue: 'fr'
		}),
		{ pays: 'CD', devise: 'CDF', decimales: 2, fuseau: 'Africa/Kinshasa', langue: 'fr' }
	);
	const r = lireRegional({
		pays: 'sn',
		devise: 'xof',
		decimales: -1,
		fuseau: 'Mars/Olympus',
		langue: 'zz'
	});
	assert.deepEqual(r, REGIONAL_DEFAUT);
	assert.equal(lireRegional({ decimales: 1.5 }).decimales, 0);
	assert.equal(lireRegional({ decimales: '2' }).decimales, 0);
	assert.equal(lireRegional({ decimales: 7 }).decimales, 0);
	assert.equal(lireRegional({ langue: 'pt-GW' }).langue, 'pt');
	assert.equal(lireRegional({ langue: 'EN' }).langue, 'en');
});

test('les cinq devises et quatre langues de la zone OHADA sont lues', () => {
	for (const [devise, decimales] of [
		['XOF', 0],
		['XAF', 0],
		['KMF', 0],
		['GNF', 0],
		['CDF', 2]
	] as const) {
		assert.equal(lireRegional({ devise, decimales }).devise, devise);
	}
	for (const langue of LANGUES) assert.equal(lireRegional({ langue }).langue, langue);
});

test('le locale combine la langue et le pays', () => {
	assert.equal(localeDe({ langue: 'fr', pays: 'SN' }), 'fr-SN');
	assert.equal(localeDe({ langue: 'en', pays: 'CM' }), 'en-CM');
	assert.equal(localeDe({ langue: 'pt', pays: 'GW' }), 'pt-GW');
});

test('les séparateurs viennent d’Intl', () => {
	assert.deepEqual(separateurs('fr-SN'), { decimal: ',', groupe: ' ' });
	assert.deepEqual(separateurs('en-CM'), { decimal: '.', groupe: ',' });
	assert.deepEqual(separateurs('es-GQ'), { decimal: ',', groupe: '.' });
});
