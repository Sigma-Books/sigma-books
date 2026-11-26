import assert from 'node:assert/strict';
import { test } from 'node:test';
import {
	estDecimales,
	estDevise,
	estFuseau,
	estLangue,
	estPays,
	LANGUES,
	langueSupportee,
	localeDe,
	regionalDuDossier,
	regionalUtilisateur,
	separateurs
} from './regional.ts';
import { regionalDe } from './regional-exemples.ts';

test('le pays est un code ISO à deux majuscules, la devise un code à trois majuscules', () => {
	for (const bon of ['SN', 'CD', 'GQ']) assert.equal(estPays(bon), true, bon);
	for (const mauvais of ['sn', 'SEN', 'S', '', 'S1', undefined, null, 1, ['SN']]) {
		assert.equal(estPays(mauvais), false, String(mauvais));
	}
	for (const bon of ['XOF', 'CDF', 'EUR']) assert.equal(estDevise(bon), true, bon);
	for (const mauvais of ['xof', 'XO', 'XOFF', '', 'X0F', undefined, 3]) {
		assert.equal(estDevise(mauvais), false, String(mauvais));
	}
});

test('l’échelle est un entier de 0 à 4 : toute autre valeur est refusée', () => {
	for (const bon of [0, 1, 2, 3, 4]) assert.equal(estDecimales(bon), true, String(bon));
	for (const mauvais of [-1, 5, 1.5, NaN, Infinity, '2', null, undefined, 2n]) {
		assert.equal(estDecimales(mauvais), false, String(mauvais));
	}
});

test('un fuseau n’est admis que si Intl le connaît, et sous une longueur bornée', () => {
	for (const bon of ['Africa/Kinshasa', 'Africa/Douala', 'UTC']) {
		assert.equal(estFuseau(bon), true, bon);
	}
	for (const mauvais of ['Mars/Olympus', '', ' ', '../etc', 'A'.repeat(65), undefined, 1]) {
		assert.equal(estFuseau(mauvais), false, String(mauvais));
	}
});

test('la langue a la forme BCP 47 du serveur et une longueur bornée', () => {
	for (const bonne of ['fr', 'pt-GW', 'fr-SN', 'sw', 'zh-Hant-TW']) {
		assert.equal(estLangue(bonne), true, bonne);
	}
	for (const mauvaise of ['', 'f', 'FR', 'fr_SN', 'fr-', 'fr--SN', 'fr-<x>', 'a'.repeat(36), 7]) {
		assert.equal(estLangue(mauvaise), false, String(mauvaise));
	}
});

test('la langue de l’interface est la sous-étiquette primaire, si elle a un catalogue', () => {
	assert.equal(langueSupportee('pt-GW'), 'pt');
	assert.equal(langueSupportee('EN'), 'en');
	assert.equal(langueSupportee('sw'), null);
	assert.equal(langueSupportee('__proto__'), null);
	for (const langue of LANGUES) assert.equal(langueSupportee(langue), langue);
});

test('le contexte du dossier ne reprend que les cinq réglages', () => {
	const dossier = { ...regionalDe({ pays: 'CD', devise: 'CDF', decimales: 2 }), nom: 'X' };
	assert.deepEqual(regionalDuDossier(dossier), {
		pays: 'CD',
		devise: 'CDF',
		decimales: 2,
		fuseau: 'Africa/Dakar',
		langue: 'fr'
	});
});

test('le locale combine la langue et le pays, sauf région déjà portée ou pays inconnu', () => {
	assert.equal(localeDe({ langue: 'fr', pays: 'SN' }), 'fr-SN');
	assert.equal(localeDe({ langue: 'en', pays: 'CM' }), 'en-CM');
	assert.equal(localeDe({ langue: 'pt', pays: 'GW' }), 'pt-GW');
	assert.equal(localeDe({ langue: 'fr-BE', pays: 'SN' }), 'fr-BE');
	assert.equal(localeDe({ langue: 'fr-FR' }), 'fr-FR');
	assert.equal(localeDe({ langue: 'fr' }), 'fr');
});

test('hors dossier, la langue et le fuseau sont ceux du navigateur, de forme valide', () => {
	const r = regionalUtilisateur();
	assert.equal(estLangue(r.langue), true);
	assert.equal(estFuseau(r.fuseau), true);
});

test('les séparateurs viennent d’Intl', () => {
	assert.deepEqual(separateurs('fr-SN'), { decimal: ',', groupe: '\u202f' });
	assert.deepEqual(separateurs('en-CM'), { decimal: '.', groupe: ',' });
	assert.deepEqual(separateurs('es-GQ'), { decimal: ',', groupe: '.' });
});
