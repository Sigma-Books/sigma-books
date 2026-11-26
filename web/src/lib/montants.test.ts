import assert from 'node:assert/strict';
import { test } from 'node:test';
import {
	additionner,
	depuisUnites,
	diagnostiquerMontant,
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
import { regionalDe } from './regional-exemples.ts';

const FR = regionalDe({ langue: 'fr', pays: 'SN' });
const EN = regionalDe({ langue: 'en', pays: 'CM', devise: 'XAF' });
const PT = regionalDe({ langue: 'pt', pays: 'BR' });
const ES = regionalDe({ langue: 'es', pays: 'GQ', devise: 'XAF' });
const CDF = regionalDe({ langue: 'fr', pays: 'CD', devise: 'CDF', decimales: 2 });
// Dossiers à deux décimales, pour lire des saisies avec décimales (XOF, XAF… n'en ont pas).
const FR2 = regionalDe({ langue: 'fr', pays: 'SN', decimales: 2 });
const EN2 = regionalDe({ langue: 'en', pays: 'CM', devise: 'XAF', decimales: 2 });
const PT2 = regionalDe({ langue: 'pt', pays: 'BR', decimales: 2 });
const ES2 = regionalDe({ langue: 'es', pays: 'GQ', devise: 'XAF', decimales: 2 });
const PTGW = regionalDe({ langue: 'pt', pays: 'GW', decimales: 2 });
const lire = (t: string, r = FR2) => diagnostiquerMontant(t, r);

test('lireMontant normalise les saisies valides en chaîne canonique', () => {
	assert.equal(lireMontant('1500', FR), '1500');
	assert.equal(lireMontant('1 500,50', FR2), '1500.5');
	assert.equal(lireMontant('1 500.50', FR2), '1500.5');
	assert.equal(lireMontant('0012', FR), '12');
	assert.equal(lireMontant('0', FR), '0');
	assert.equal(lireMontant('0,10', FR2), '0.1');
	assert.equal(lireMontant('  42  ', FR), '42');
	assert.equal(lireMontant('100.00', FR2), '100');
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
		'1,1234567'
	]) {
		assert.equal(lireMontant(mauvais, FR2), null, mauvais);
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
	const lire = (t: string) => lireMontant(t, FR2);
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
		const r = regionalDe({ langue: 'fr', devise, decimales: 0 });
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

test('lireMontant lit les quatre écritures courantes selon le locale', () => {
	// fr (décimale « , », milliers : espace), 2 décimales.
	assert.equal(lireMontant('1 500,50', FR2), '1500.5');
	assert.equal(lireMontant('1500,50', FR2), '1500.5');
	assert.equal(lireMontant('1 500', FR2), '1500');
	assert.equal(lireMontant('1,5', FR2), '1.5');
	assert.equal(lireMontant('1.5', FR2), '1.5'); // pavé numérique
	assert.equal(lireMontant('1,500.50', FR2), null);
	assert.equal(lireMontant('1.500,50', FR2), null);
	// en (décimale « . », milliers : « , »).
	assert.equal(lireMontant('1,500.50', EN2), '1500.5');
	assert.equal(lireMontant('1,500', EN2), '1500');
	assert.equal(lireMontant('1 500.50', EN2), '1500.5');
	assert.equal(lireMontant('1.500,50', EN2), null);
	// pt et es (décimale « , », milliers : « . »).
	for (const r of [PT2, ES2]) {
		assert.equal(lireMontant('1.500,50', r), '1500.5');
		assert.equal(lireMontant('1.500', r), '1500');
		assert.equal(lireMontant('1,50', r), '1.5');
		assert.equal(lireMontant('1,500.50', r), null);
		assert.equal(lireMontant('1 500,50', r), '1500.5');
	}
});

test('un séparateur de milliers n’est admis que suivi de groupes de exactement trois chiffres', () => {
	// Ces saisies seraient lues 10 à 1000 fois trop petites ou trop grandes si on les tolérait.
	assert.equal(lireMontant('1500,50', EN2), null);
	assert.equal(lireMontant('12,5', EN2), null);
	assert.equal(lireMontant('1,50', EN2), null);
	assert.equal(lireMontant('1.5', PT2), null);
	assert.equal(lireMontant('1.5', ES2), null);
	assert.equal(lireMontant('12.50', PT2), null);
	assert.equal(lireMontant('1,5000', EN2), null);
	assert.equal(lireMontant('1,500,00', EN2), null);
	assert.equal(lireMontant('1234,567', EN2), null);
	assert.equal(lireMontant(',500', EN2), null);
	assert.equal(lireMontant('1 5', FR2), null);
	assert.equal(lireMontant('1500 000', FR2), null);
	assert.equal(lireMontant('1 500 000', FR2), '1500000');
	assert.equal(lireMontant('1,234,567.5', EN2), '1234567.5');
	assert.equal(lireMontant('1.234.567,5', PT2), '1234567.5');
});

test('pt-GW (et pt-MZ) : l’espace est le séparateur de milliers, la virgule la décimale', () => {
	assert.deepEqual(lire('1 500,50', PTGW), { ok: true, montant: '1500.5' });
	assert.deepEqual(lire('1\u00a0500,50', PTGW), { ok: true, montant: '1500.5' });
	assert.deepEqual(lire('1500,50', PTGW), { ok: true, montant: '1500.5' });
	assert.deepEqual(lire('1 5', PTGW), { ok: false, motif: 'illisible' });
	assert.deepEqual(lire('1,500', PTGW), { ok: false, motif: 'trop_de_decimales' });
	assert.equal(
		lireMontant('1 500,50', regionalDe({ langue: 'pt', pays: 'MZ', decimales: 2 })),
		'1500.5'
	);
});

test('une saisie qui a plus de décimales que la devise est refusée, jamais tronquée', () => {
	const trop = { ok: false, motif: 'trop_de_decimales' };
	// XOF, XAF, GNF, KMF : échelle 0, donc 1,500 et 1.500 ne sont pas lus 1,5.
	for (const devise of ['XOF', 'XAF', 'GNF', 'KMF']) {
		const r = regionalDe({ langue: 'fr', devise });
		assert.equal(r.decimales, 0, devise);
		assert.deepEqual(lire('1,500', r), trop, `${devise} 1,500`);
		assert.deepEqual(lire('1.500', r), trop, `${devise} 1.500`);
		assert.deepEqual(lire('1,5', r), trop, `${devise} 1,5`);
		assert.deepEqual(lire('1 500', r), { ok: true, montant: '1500' }, `${devise} 1 500`);
	}
	// CDF : deux décimales.
	assert.deepEqual(lire('1,500', CDF), trop);
	assert.deepEqual(lire('1.500', CDF), trop);
	assert.deepEqual(lire('1,50', CDF), { ok: true, montant: '1.5' });
	assert.deepEqual(lire('1,5', CDF), { ok: true, montant: '1.5' });
	assert.deepEqual(lire('1500,500', CDF), trop);
	// Zéros inutiles compris : on ne tronque pas en silence.
	assert.deepEqual(lire('1500,000', CDF), trop);
	// Les grandes devises à trois décimales (échelle 3) gardent leurs trois chiffres.
	assert.deepEqual(lire('1,500', regionalDe({ devise: 'XOF', decimales: 3 })), {
		ok: true,
		montant: '1.5'
	});
	// Montant illisible et montant à trop de décimales se distinguent.
	assert.deepEqual(lire('abc', CDF), { ok: false, motif: 'illisible' });
	assert.equal(lireMontant('1,500', FR), null);
});

test('les séparateurs de milliers ne sont admis que dans la partie entière', () => {
	assert.equal(lireMontant('1.500,5,0', PT2), null);
	assert.equal(lireMontant('1.5,00', EN2), null);
	assert.equal(lireMontant('5,', FR2), null);
	assert.equal(lireMontant(',5', FR2), null);
});

test('un montant ne devient jamais un flottant (grands montants exacts)', () => {
	assert.equal(
		lireMontant('99 999 999 999 999 999 999,000001', regionalDe({ decimales: 6 })),
		'99999999999999999999.000001'
	);
	assert.equal(lireCanonique('0.1'), '0.1');
	assert.equal(lireCanonique('0,1'), null);
});

test('les espaces de groupement (espace, insécable, fine insécable) sont admis dans tous les locales', () => {
	assert.equal(lireMontant('1\u202f500', FR), '1500');
	assert.equal(lireMontant('1\u00a0500', EN), '1500');
	assert.equal(lireMontant('1 500', PT), '1500');
});
