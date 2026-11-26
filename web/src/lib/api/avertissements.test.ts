import assert from 'node:assert/strict';
import { test } from 'node:test';
import {
	AVERTISSEMENTS_AFFICHES_MAX,
	bandeauAvertissements,
	lireAvertissements,
	messageAvertissement
} from './avertissements.ts';
import { estLotValide, estValidation, estContrePassation } from './brouillard.ts';
import { regionalDe } from '../regional-exemples.ts';

const FR = regionalDe({});
const EN = regionalDe({ langue: 'en', pays: 'CM', devise: 'XAF' });
const ID = '0b0f6c43-7c47-4a43-9d1e-2f4f6f0a1b2c';
const echeance = {
	code: 'echeance_de_cloture_depassee',
	periode: '2026-01-01',
	echeance: '2026-03-31'
};
const tresorerie = { code: 'compte_tresorerie_absent', journal: 'BQ', compte: '521' };

test('les deux avertissements du serveur sont lus avec leurs détails', () => {
	assert.deepEqual(lireAvertissements([echeance, tresorerie]), [echeance, tresorerie]);
});

test('absent ou mal formé : liste vide, jamais d’erreur', () => {
	for (const brut of [undefined, null, 'x', 3, {}, [null], [3], ['x'], [{}], [{ code: 3 }]]) {
		assert.deepEqual(lireAvertissements(brut), [], String(brut));
	}
});

test('les détails hostiles sont écartés, le code valide est gardé', () => {
	const lus = lireAvertissements([
		{
			code: 'compte_tresorerie_absent',
			journal: '<script>',
			compte: 'x'.repeat(100),
			periode: '2026-13-45',
			echeance: 3
		},
		{ code: 'Code Hostile<b>' }
	]);
	assert.deepEqual(lus, [{ code: 'compte_tresorerie_absent' }]);
});

test('messages : dates dans le locale du dossier, langue du dossier', () => {
	assert.match(
		messageAvertissement(echeance, FR),
		/commençant le 01\/01\/2026 était le 31\/03\/2026/
	);
	assert.match(messageAvertissement(tresorerie, FR), /journal BQ .*compte de trésorerie 521/);
	// Catalogue anglais non traduit : repli sur le français, dates en locale `en-CM`.
	assert.match(messageAvertissement(echeance, EN), /01\/01\/2026/);
});

test('un code inconnu, ou un code connu sans détail, donne le message général', () => {
	const general = messageAvertissement({ code: 'code_futur' }, FR);
	assert.match(general, /ne sait pas expliquer/);
	assert.equal(messageAvertissement({ code: 'echeance_de_cloture_depassee' }, FR), general);
	assert.equal(
		messageAvertissement({ code: 'compte_tresorerie_absent', journal: 'BQ' }, FR),
		general
	);
});

test('bandeau : aucun avertissement, aucun bandeau', () => {
	assert.equal(bandeauAvertissements([], FR, false), null);
	assert.equal(bandeauAvertissements([{ numero: 'OD-1', avertissements: [] }], FR, true), null);
});

test('bandeau d’une écriture, puis d’un lot avec les numéros', () => {
	const une = bandeauAvertissements([{ numero: 'OD-1', avertissements: [tresorerie] }], FR, false);
	assert.equal(une?.titre, 'Écriture validée, avec avertissement :');
	assert.equal(une?.lignes.length, 1);
	assert.ok(!une?.lignes[0]?.startsWith('OD-1'));
	const lot = bandeauAvertissements(
		[
			{ numero: 'OD-1', avertissements: [echeance] },
			{ numero: 'OD-2', avertissements: [] },
			{ numero: 'OD-3', avertissements: [tresorerie] }
		],
		FR,
		true
	);
	assert.equal(lot?.titre, 'Écritures validées, avec avertissements :');
	assert.equal(lot?.lignes.length, 2);
	assert.match(lot?.lignes[0] ?? '', /^OD-1 : /);
	assert.match(lot?.lignes[1] ?? '', /^OD-3 : /);
	assert.equal(lot?.suite, null);
});

test('bandeau : au plus dix lignes, le reste est compté', () => {
	const beaucoup = Array.from({ length: 25 }, (_, i) => ({
		numero: `OD-${i}`,
		avertissements: [tresorerie]
	}));
	const b = bandeauAvertissements(beaucoup, FR, true);
	assert.equal(b?.lignes.length, AVERTISSEMENTS_AFFICHES_MAX);
	assert.match(b?.suite ?? '', /15 autre/);
});

test('les réponses du serveur avec avertissements restent lisibles (ancienne API comprise)', () => {
	const validation = {
		id: ID,
		numero: 'OD-2026-000001',
		rang: 1,
		empreinte: 'e',
		format: 1,
		date_validation: '2026-03-01'
	};
	assert.equal(estValidation(validation), true);
	assert.equal(estValidation({ ...validation, avertissements: [echeance] }), true);
	assert.equal(estValidation({ ...validation, avertissements: 'abc' }), true);
	assert.equal(
		estLotValide({ ecritures: [{ ...validation, avertissements: [] }], exclues_a_revoir: 0 }),
		true
	);
	const cp = {
		...validation,
		origine: ID,
		origine_numero: 'OD-2026-000000',
		avertissements: [tresorerie]
	};
	assert.equal(estContrePassation(cp), true);
});
