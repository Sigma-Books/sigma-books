import assert from 'node:assert/strict';
import { test } from 'node:test';
import {
	actionClavier,
	normaliser,
	rechercherComptes,
	type EvenementClavier
} from './raccourcis.ts';

const touche = (key: string, plus: Partial<EvenementClavier> = {}): EvenementClavier => ({
	key,
	ctrlKey: false,
	metaKey: false,
	altKey: false,
	shiftKey: false,
	...plus
});

test('Entrée passe à la ligne suivante dans la grille, pas dans l’en-tête', () => {
	assert.equal(actionClavier(touche('Enter'), { champ: 'debit', vide: true }), 'ligne-suivante');
	assert.equal(actionClavier(touche('Enter'), { champ: 'libelle', vide: false }), 'ligne-suivante');
	assert.equal(actionClavier(touche('Enter'), { champ: 'entete', vide: false }), null);
});

test('Entrée valide la liste ouverte au lieu de changer de ligne', () => {
	assert.equal(
		actionClavier(touche('Enter'), { champ: 'compte', vide: false, listeOuverte: true }),
		null
	);
});

test('Ctrl+Entrée enregistre', () => {
	assert.equal(
		actionClavier(touche('Enter', { ctrlKey: true }), { champ: 'credit', vide: false }),
		'enregistrer'
	);
	assert.equal(
		actionClavier(touche('Enter', { ctrlKey: true }), { champ: 'entete', vide: false }),
		'enregistrer'
	);
});

test('= solde seulement dans un montant vide', () => {
	assert.equal(actionClavier(touche('='), { champ: 'debit', vide: true }), 'solde');
	assert.equal(actionClavier(touche('='), { champ: 'credit', vide: true }), 'solde');
	assert.equal(actionClavier(touche('='), { champ: 'debit', vide: false }), null);
	assert.equal(actionClavier(touche('='), { champ: 'compte', vide: true }), null);
});

test('F2 ouvre la recherche de compte dans le champ compte', () => {
	assert.equal(actionClavier(touche('F2'), { champ: 'compte', vide: true }), 'recherche-compte');
	assert.equal(actionClavier(touche('F2'), { champ: 'tiers', vide: true }), null);
});

test('les combinaisons avec Alt ou Méta sont laissées au navigateur', () => {
	assert.equal(
		actionClavier(touche('Enter', { altKey: true }), { champ: 'debit', vide: true }),
		null
	);
	assert.equal(
		actionClavier(touche('Enter', { metaKey: true }), { champ: 'debit', vide: true }),
		null
	);
	assert.equal(
		actionClavier(touche('Enter', { shiftKey: true }), { champ: 'debit', vide: true }),
		null
	);
});

const COMPTES = [
	{ numero: '411', intitule: 'Clients' },
	{ numero: '4111', intitule: 'Clients - ventes de biens' },
	{ numero: '521', intitule: 'Banques locales' },
	{ numero: '601', intitule: 'Achats de marchandises' },
	{ numero: '706', intitule: 'Prestations de services' }
];

test('normaliser retire accents et casse', () => {
	assert.equal(normaliser('  Écriture  DE  clôture '), 'ecriture de cloture');
});

test('rechercherComptes : numéro d’abord, puis intitulé', () => {
	assert.deepEqual(
		rechercherComptes(COMPTES, '411').map((c) => c.numero),
		['411', '4111']
	);
	assert.deepEqual(
		rechercherComptes(COMPTES, 'MARCHANDISES').map((c) => c.numero),
		['601']
	);
	assert.deepEqual(
		rechercherComptes(COMPTES, 'prestation serv').map((c) => c.numero),
		['706']
	);
	assert.deepEqual(rechercherComptes(COMPTES, ''), []);
	assert.equal(rechercherComptes(COMPTES, 'client', 1).length, 1);
});
