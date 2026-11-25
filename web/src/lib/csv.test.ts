import assert from 'node:assert/strict';
import { describe, it } from 'node:test';
import type { Balance, GrandLivre } from './api/editions.ts';
import {
	csvBalance,
	csvGrandLivre,
	formaterCellule,
	neutraliser,
	nomFichierEdition,
	versCsv
} from './csv.ts';

describe('échappement des cellules', () => {
	it('neutralise tout début de formule', () => {
		for (const t of [
			'=1+1',
			'+33',
			'-cmd',
			'@SUM(A1)',
			'\tx',
			'\rx',
			' =1',
			'=HYPERLINK("http://x")'
		]) {
			assert.ok(neutraliser(t).startsWith("'"), t);
		}
		assert.equal(neutraliser('Loyer'), 'Loyer');
		assert.equal(neutraliser('Facture 12-3'), 'Facture 12-3');
	});

	it('protège séparateur, guillemets et retours à la ligne', () => {
		assert.equal(formaterCellule('a;b'), '"a;b"');
		assert.equal(formaterCellule('dit "oui"'), '"dit ""oui"""');
		assert.equal(formaterCellule('l1\nl2'), '"l1\nl2"');
		assert.equal(formaterCellule('=A;B'), `"'=A;B"`);
	});

	it('montant exact : virgule décimale, aucun nombre ; négatif conservé', () => {
		assert.equal(formaterCellule({ montant: '1500.5' }), '1500,5');
		assert.equal(formaterCellule({ montant: '-12' }), '-12');
		assert.equal(
			formaterCellule({ montant: '12345678901234567890.123456' }),
			'12345678901234567890,123456'
		);
	});

	it('un « montant » qui n’en est pas un est du texte neutralisé', () => {
		assert.equal(formaterCellule({ montant: '=1+1' }), "'=1+1");
	});

	it('document : BOM, séparateur point-virgule, fins de ligne CRLF', () => {
		assert.equal(versCsv([['a', { montant: '1.5' }], ['b']]), '﻿a;1,5\r\nb\r\n');
	});
});

describe('éditions en CSV', () => {
	const entete = {
		exercice: '2026',
		du: '2026-01-01',
		au: '2026-12-31',
		mode: 'definitif' as const,
		brouillons_exclus: { desequilibres: 0, illisibles: 0 }
	};

	it('balance : libellés de colonnes reçues, totaux du serveur', () => {
		const b: Balance = {
			...entete,
			format: 2,
			regroupement: 'compte',
			colonnes: ['solde_debiteur', 'solde_crediteur'],
			lignes: [{ cle: '411', valeurs: ['100.25', '0'] }],
			totaux: ['100.25', '100.25'],
			equilibree: true
		};
		assert.equal(
			csvBalance(b),
			'\uFEFFMode;définitif\r\nExercice;2026\r\nPériode;2026-01-01;2026-12-31\r\n\r\nCompte;Solde débiteur;Solde créditeur\r\n411;100,25;0\r\nTotaux;100,25;100,25\r\n'
		);
	});

	it('grand livre : un libellé piégé est neutralisé', () => {
		const g: GrandLivre = {
			...entete,
			comptes: [
				{
					compte: '411',
					solde_ouverture: '0',
					lignes: [
						{
							date: '2026-03-01',
							journal: 'VT',
							numero: 'VT-1',
							date_valeur_comptable: null,
							libelle: "=cmd|' /C calc'!A0",
							tiers: null,
							debit: '10',
							credit: '0',
							solde: '10'
						}
					],
					total_debit: '10',
					total_credit: '0',
					solde_final: '10'
				}
			]
		};
		const csv = csvGrandLivre(g);
		assert.ok(csv.includes("'=cmd|"));
		assert.ok(!/;=cmd/.test(csv));
	});

	it('en-tête : mode provisoire et brouillons exclus signalés', () => {
		const csv = csvBalance({
			...entete,
			mode: 'provisoire',
			brouillons_exclus: { desequilibres: 2, illisibles: 1 },
			format: 2,
			regroupement: 'compte',
			colonnes: ['solde_debiteur', 'solde_crediteur'],
			lignes: [{ cle: '411', valeurs: ['-0', '0'] }],
			totaux: ['0', '0'],
			equilibree: true
		});
		assert.ok(csv.startsWith('\uFEFFMode;provisoire\r\n'));
		assert.ok(csv.includes('Brouillons exclus;3\r\n'));
		assert.ok(csv.includes('411;0;0\r\n'), 'moins zéro normalisé');
	});

	it('nom de fichier sûr', () => {
		assert.equal(
			nomFichierEdition('grand-livre', '2026', 'provisoire'),
			'grand-livre-2026-provisoire.csv'
		);
		assert.equal(nomFichierEdition('../x', '20/26', 'a b'), 'x-2026-ab.csv');
	});
});
