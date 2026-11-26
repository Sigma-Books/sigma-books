import assert from 'node:assert/strict';
import { test } from 'node:test';
import {
	additionner,
	depuisUnites,
	ecart,
	estNul,
	formaterMontant,
	formaterMontantDevise,
	formaterNombre,
	lireCanonique,
	lireMontant,
	versSaisie,
	versUnites
} from './montants.ts';
import { lireRegional, type Regional } from './regional.ts';

const FR = lireRegional({ langue: 'fr', pays: 'SN' });
const EN = lireRegional({ langue: 'en', pays: 'CM', devise: 'XAF' });
const PT = lireRegional({ langue: 'pt', pays: 'BR' });
const ES = lireRegional({ langue: 'es', pays: 'GQ', devise: 'XAF' });
const CDF = lireRegional({ langue: 'fr', pays: 'CD', devise: 'CDF', decimales: 2 });

test('lireMontant normalise les saisies valides en chaîne canonique', () => {
	assert.equal(lireMontant('1500', FR), '1500');
	assert.equal(lireMontant('1 500,50', FR), '1500.5');
	assert.equal(lireMontant('1 500.500', FR), '1500.5');
	assert.equal(lireMontant('0012', FR), '12');
	assert.equal(lireMontant('0', FR), '0');
	assert.equal(lireMontant('0,10', FR), '0.1');
	assert.equal(lireMontant('  42  ', FR), '42');
	assert.equal(lireMontant('100.00', FR), '100');
});

test('lireMontant refuse ce qui n’est pas un décimal positif borné', () => {
	for (const mauvais of [
		'',
		' ',
		'-5',
		'+5',
		'1e3',
		'1,2,3',
		'12a',
		'.5',
		'5.',
		'0x10',
		'1.1234567'
	]) {
		assert.equal(lireMontant(mauvais, FR), null, mauvais);
	}
	assert.equal(lireMontant('9'.repeat(20), FR), '9'.repeat(20));
	assert.equal(lireMontant('9'.repeat(21), FR), null);
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

test('l’écart se calcule sur des saisies lues dans le locale du dossier', () => {
	const lire = (t: string) => lireMontant(t, FR);
	const e = ecart(['1 500,50'], ['1000,25', '500,25'], lire);
	assert.equal(e.sens, 'equilibre');
	assert.equal(additionner(['1,5', '2,25'], lire), '3.75');
});

test('formaterMontant suit la devise, l’échelle et le locale du dossier', () => {
	const U202F = '\u202f';
	// XOF : zéro décimale, groupes séparés par l'espace fine insécable (fr).
	assert.equal(formaterMontant('1234567', FR), `1${U202F}234${U202F}567`);
	assert.equal(formaterMontant('1500.5', FR), `1${U202F}500,5`);
	assert.equal(formaterMontant('1500.0', FR), `1${U202F}500`);
	// XAF et KMF, GNF : zéro décimale également.
	for (const devise of ['XAF', 'KMF', 'GNF']) {
		const r = lireRegional({ langue: 'fr', devise, decimales: 0 });
		assert.equal(formaterMontant('12000', r), `12${U202F}000`, devise);
	}
	// CDF : deux décimales, toujours affichées.
	assert.equal(formaterMontant('1500', CDF), `1${U202F}500,00`);
	assert.equal(formaterMontant('1500.5', CDF), `1${U202F}500,50`);
	assert.equal(formaterMontant('0.1', CDF), '0,10');
	assert.equal(formaterMontant('-1000.25', CDF), `-1${U202F}000,25`);
	assert.equal(formaterMontant('1000.250000', CDF), `1${U202F}000,25`);
	assert.equal(formaterMontant('pas un nombre', FR), 'pas un nombre');
});

test('un chiffre significatif au-delà de l’échelle n’est jamais masqué', () => {
	assert.equal(formaterMontant('1500.5', FR), `1\u202f500,5`);
	assert.equal(formaterMontant('0.000001', CDF), '0,000001');
});

test('les séparateurs viennent du locale (en, pt, es)', () => {
	assert.equal(formaterMontant('1234567', EN), '1,234,567');
	assert.equal(formaterMontant('1234.5', { ...EN, decimales: 2 }), '1,234.50');
	assert.equal(formaterMontant('1234567', PT), '1.234.567');
	assert.equal(formaterMontant('1500', ES), '1.500'); // `es` groupe aussi les quatre chiffres
	assert.equal(formaterMontant('1500.5', { ...ES, decimales: 2 }), '1.500,50');
});

test('formaterMontantDevise ajoute le code de la devise après une espace insécable', () => {
	assert.equal(formaterMontantDevise('1500', FR), '1\u202f500\u00a0XOF');
	assert.equal(formaterMontantDevise('1500', CDF), '1\u202f500,00\u00a0CDF');
});

test('formaterNombre : décimale et groupes choisis par l’appelant (CSV)', () => {
	assert.equal(formaterNombre('1500.5', 2, ',', null), '1500,50');
	assert.equal(formaterNombre('1500.5', 0, '.', null), '1500.5');
	assert.equal(formaterNombre('-0.5', 2, '.', ','), '-0.50');
});

test('versSaisie remet le montant canonique dans le locale de saisie', () => {
	assert.equal(versSaisie('1500.5', FR), '1500,5');
	assert.equal(versSaisie('1500.5', EN), '1500.5');
	assert.equal(versSaisie('1500', PT), '1500');
});

test('lireMontant lit les quatre écritures courantes selon le locale, sans règle d’ambiguïté', () => {
	// fr (décimale « , », milliers : espace) : 1.500 reste une décimale (comme Tryton et Odoo).
	assert.equal(lireMontant('1 500,50', FR), '1500.5');
	assert.equal(lireMontant('1.500', FR), '1.5');
	assert.equal(lireMontant('1,500', FR), '1.5');
	assert.equal(lireMontant('1,500.50', FR), null);
	assert.equal(lireMontant('1.500,50', FR), null);
	// en (décimale « . », milliers : « , »).
	assert.equal(lireMontant('1,500.50', EN), '1500.5');
	assert.equal(lireMontant('1,500', EN), '1500');
	assert.equal(lireMontant('1.500', EN), '1.5');
	assert.equal(lireMontant('1 500.50', EN), '1500.5');
	assert.equal(lireMontant('1.500,50', EN), null);
	// pt et es (décimale « , », milliers : « . »).
	for (const r of [PT, ES]) {
		assert.equal(lireMontant('1.500,50', r), '1500.5');
		assert.equal(lireMontant('1.500', r), '1500');
		assert.equal(lireMontant('1,500', r), '1.5');
		assert.equal(lireMontant('1,500.50', r), null);
		assert.equal(lireMontant('1 500,50', r), '1500.5');
	}
});

test('les séparateurs de milliers ne sont admis que dans la partie entière', () => {
	assert.equal(lireMontant('1.500,5,0', PT), null);
	assert.equal(lireMontant('1.5,00', EN), null);
	assert.equal(lireMontant('5,', FR), null);
	assert.equal(lireMontant(',5', FR), null);
});

test('un montant ne devient jamais un flottant (grands montants exacts)', () => {
	assert.equal(lireMontant('99 999 999 999 999 999 999,000001', FR), '99999999999999999999.000001');
	assert.equal(lireCanonique('0.1'), '0.1');
	assert.equal(lireCanonique('0,1'), null);
});

test('lireMontant(…) reste indépendant du dossier pour les espaces de groupement', () => {
	const U202F = '\u202f';
	assert.equal(lireMontant(`1${U202F}500`, FR), '1500');
	assert.equal(lireMontant('1\u00a0500', EN), '1500');
	assert.equal(lireMontant('1 500', PT), '1500');
});

test('le contexte régional par défaut est XOF, 0 décimale, fr', () => {
	const r: Regional = lireRegional(undefined);
	assert.equal(r.devise, 'XOF');
	assert.equal(r.decimales, 0);
	assert.equal(r.langue, 'fr');
});
