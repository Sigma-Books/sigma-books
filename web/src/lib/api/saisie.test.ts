import assert from 'node:assert/strict';
import { test } from 'node:test';
import {
	corpsEcriture,
	equilibreIndicatif,
	indexDepuisPosition,
	ligneVide,
	soldePropose,
	type EnteteSaisie,
	type LigneSaisie
} from './saisie.ts';

const entete: EnteteSaisie = {
	journal: 'OD',
	date: '2026-03-15',
	libelle: ' Loyer ',
	dateValeur: ''
};
const l = (compte: string, debit: string, credit: string, tiers = ''): LigneSaisie => ({
	compte,
	tiers,
	debit,
	credit
});

test('le corps ne contient que des chaînes pour les montants', () => {
	const r = corpsEcriture(
		entete,
		[l('6222', '150 000,50', ''), ligneVide(), l('521', '', '150000.5')],
		3
	);
	assert.ok(r.ok);
	assert.deepEqual(r.corps, {
		version: 3,
		journal: 'OD',
		date: '2026-03-15',
		libelle: 'Loyer',
		lignes: [
			{ compte: '6222', sens: 'debit', montant: '150000.5' },
			{ compte: '521', sens: 'credit', montant: '150000.5' }
		]
	});
	assert.deepEqual(r.positions, [0, 2]);
	for (const ligne of r.corps.lignes) assert.equal(typeof ligne.montant, 'string');
});

test('tiers et date de valeur sont transmis quand ils sont saisis', () => {
	const r = corpsEcriture({ ...entete, dateValeur: '2026-03-10' }, [
		l('401', '', '10', 'id-tiers')
	]);
	assert.ok(r.ok);
	assert.equal(r.corps.date_valeur_comptable, '2026-03-10');
	assert.equal(r.corps.lignes[0]?.tiers, 'id-tiers');
	assert.equal('version' in r.corps, false);
});

test('les formes illisibles sont signalées avec la ligne de la grille', () => {
	assert.deepEqual(corpsEcriture(entete, [l('6222', '1', ''), l('521', '1', '2')]), {
		ok: false,
		probleme: 'debit_et_credit',
		ligne: 1
	});
	assert.deepEqual(corpsEcriture(entete, [ligneVide(), l('521', 'abc', '')]), {
		ok: false,
		probleme: 'montant_illisible',
		ligne: 1
	});
	assert.deepEqual(
		corpsEcriture(
			entete,
			[l('521', '', '')].map((x) => ({ ...x, tiers: 't' }))
		),
		{
			ok: false,
			probleme: 'montant_absent',
			ligne: 0
		}
	);
	assert.deepEqual(corpsEcriture(entete, [l('', '5', '')]), {
		ok: false,
		probleme: 'compte_absent',
		ligne: 0
	});
});

test('équilibre indicatif et solde proposé par =', () => {
	const grille = [l('6222', '100', ''), l('4452', '18', ''), l('521', '', '')];
	assert.equal(equilibreIndicatif(grille).sens, 'credit');
	assert.deepEqual(soldePropose(grille, 2), { champ: 'credit', montant: '118' });
	assert.equal(soldePropose([l('1', '5', ''), l('2', '', '5'), l('3', '', '')], 2), null);
	assert.deepEqual(soldePropose([l('1', '', '7'), l('2', '', '')], 1), {
		champ: 'debit',
		montant: '7'
	});
});

test('position du serveur → ligne de la grille', () => {
	assert.equal(indexDepuisPosition([0, 2, 3], 2), 2);
	assert.equal(indexDepuisPosition([0, 2, 3], 9), null);
});
