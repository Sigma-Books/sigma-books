import assert from 'node:assert/strict';
import { describe, it } from 'node:test';
import type { Balance, GrandLivre } from './api/editions.ts';
import {
	csvBalance,
	csvCentralisateur,
	csvGrandLivre,
	formaterCellule,
	neutraliser,
	nomFichierEdition,
	optionsCsvParDefaut,
	versCsv,
	type ContexteCsv,
	type OptionsCsv
} from './csv.ts';
import { regionalDe } from './regional-exemples.ts';

const FR = regionalDe({});
const EN = regionalDe({ langue: 'en', pays: 'CM', devise: 'XAF' });
const CDF = regionalDe({ langue: 'fr', pays: 'CD', devise: 'CDF', decimales: 2 });
const O: OptionsCsv = optionsCsvParDefaut(FR);
const contexte = (regional = FR, nomDossier = 'Société Test'): ContexteCsv => ({
	nomDossier,
	regional,
	exercices: [{ code: '2026', debut: '2026-01-01', fin: '2026-12-31' }],
	options: optionsCsvParDefaut(regional)
});

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
		assert.equal(formaterCellule('a;b', O), '"a;b"');
		assert.equal(formaterCellule('dit "oui"', O), '"dit ""oui"""');
		assert.equal(formaterCellule('l1\nl2', O), '"l1\nl2"');
		assert.equal(formaterCellule('=A;B', O), `"'=A;B"`);
	});

	it('montant exact : virgule décimale, aucun nombre ; négatif conservé', () => {
		assert.equal(formaterCellule({ montant: '1500.5' }, O), '1500,5');
		assert.equal(formaterCellule({ montant: '-12' }, O), '-12');
		assert.equal(
			formaterCellule({ montant: '12345678901234567890.123456' }, O),
			'12345678901234567890,123456'
		);
	});

	it('un « montant » qui n’en est pas un est du texte neutralisé', () => {
		assert.equal(formaterCellule({ montant: '=1+1' }, O), "'=1+1");
	});

	it('document : BOM, séparateur point-virgule, fins de ligne CRLF', () => {
		assert.equal(versCsv([['a', { montant: '1.5' }], ['b']], O), '﻿a;1,5\r\nb\r\n');
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
			csvBalance(b, contexte()),
			'\uFEFFDossier;Société Test\r\nDevise;XOF\r\nMode;définitif\r\nExercice;2026;2026-01-01;2026-12-31\r\nPériode;2026-01-01;2026-12-31\r\n\r\nCompte;Solde débiteur;Solde créditeur\r\n411;100,25;0\r\nTotaux;100,25;100,25\r\n'
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
		const csv = csvGrandLivre(g, contexte());
		assert.ok(csv.includes("'=cmd|"));
		assert.ok(!/;=cmd/.test(csv));
	});

	it('en-tête : mode provisoire et brouillons exclus signalés', () => {
		const csv = csvBalance(
			{
				...entete,
				mode: 'provisoire',
				brouillons_exclus: { desequilibres: 2, illisibles: 1 },
				format: 2,
				regroupement: 'compte',
				colonnes: ['solde_debiteur', 'solde_crediteur'],
				lignes: [{ cle: '411', valeurs: ['-0', '0'] }],
				totaux: ['0', '0'],
				equilibree: true
			},
			contexte()
		);
		assert.ok(csv.includes('Mode;provisoire\r\n'));
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

	it('en-tête : dossier, devise et dates de l’exercice (pas seulement son code)', () => {
		const csv = csvBalance(
			{
				...entete,
				format: 2,
				regroupement: 'compte',
				colonnes: ['solde_debiteur'],
				lignes: [],
				totaux: ['0'],
				equilibree: true
			},
			contexte(CDF, '=Société')
		);
		assert.ok(csv.includes("Dossier;'=Société\r\n"), 'nom du dossier neutralisé');
		assert.ok(csv.includes('Devise;CDF\r\n'));
		assert.ok(csv.includes('Exercice;2026;2026-01-01;2026-12-31\r\n'));
		const inconnu = csvBalance(
			{
				...entete,
				exercice: '2025',
				format: 2,
				regroupement: 'compte',
				colonnes: [],
				lignes: [],
				totaux: [],
				equilibree: true
			},
			contexte()
		);
		assert.ok(inconnu.includes('Exercice;2025\r\n'));
	});

	it('les montants suivent l’échelle de la devise (CDF : deux décimales)', () => {
		const csv = csvBalance(
			{
				...entete,
				format: 2,
				regroupement: 'compte',
				colonnes: ['solde_debiteur', 'solde_crediteur'],
				lignes: [{ cle: '411', valeurs: ['1500.5', '0'] }],
				totaux: ['1500.5', '0'],
				equilibree: true
			},
			contexte(CDF)
		);
		assert.ok(csv.includes('411;1500,50;0,00\r\n'));
	});

	it('le centralisateur écrit les mois dans le locale du dossier', () => {
		const rub = {
			par_journal: [],
			par_compte: [],
			par_mois: [{ annee: 2026, mois: 3, journal: null, compte: null, debit: '1', credit: '1' }],
			total_debit: '1',
			total_credit: '1'
		};
		const vide = { ...rub, par_mois: [], total_debit: '0', total_credit: '0' };
		const c = { ...entete, mouvements: rub, a_nouveaux: vide };
		assert.ok(csvCentralisateur(c, contexte()).includes('mars 2026'));
		assert.ok(csvCentralisateur(c, contexte(EN)).includes('March 2026'));
	});
});

describe('options du CSV', () => {
	it('valeurs par défaut déduites du locale du dossier', () => {
		assert.deepEqual(optionsCsvParDefaut(FR), {
			separateur: ';',
			separateurDecimal: ',',
			bom: true,
			decimales: 0
		});
		assert.deepEqual(optionsCsvParDefaut(EN), {
			separateur: ',',
			separateurDecimal: '.',
			bom: true,
			decimales: 0
		});
		assert.equal(optionsCsvParDefaut(CDF).decimales, 2);
		assert.equal(optionsCsvParDefaut(regionalDe({ langue: 'pt', pays: 'GW' })).separateur, ';');
	});

	it('séparateur, séparateur décimal et BOM sont réglables', () => {
		const lignes = [['a', { montant: '1500.5' }]];
		const virgule: OptionsCsv = {
			separateur: ',',
			separateurDecimal: '.',
			bom: false,
			decimales: 2
		};
		assert.equal(versCsv(lignes, virgule), 'a,1500.50\r\n');
		assert.equal(versCsv(lignes, { ...virgule, separateur: '\t' }), 'a\t1500.50\r\n');
		assert.equal(versCsv(lignes, { ...virgule, bom: true }), '\uFEFFa,1500.50\r\n');
	});

	it('un séparateur identique au décimal met le montant entre guillemets', () => {
		const o: OptionsCsv = { separateur: ',', separateurDecimal: ',', bom: false, decimales: 2 };
		assert.equal(versCsv([['a', { montant: '1.5' }]], o), 'a,"1,50"\r\n');
	});

	it('la neutralisation des formules est conservée avec toutes les options', () => {
		for (const separateur of [';', ',', '\t'] as const) {
			const o: OptionsCsv = { separateur, separateurDecimal: '.', bom: false, decimales: 0 };
			assert.equal(versCsv([['=1+1', '+3', '-x', '@a', '\tz']], o).startsWith("'=1+1"), true);
			assert.equal(formaterCellule('+3', o), "'+3");
			assert.equal(formaterCellule('@a', o), "'@a");
			assert.equal(formaterCellule('\rz', o), `"'\rz"`);
			assert.equal(formaterCellule(`a${separateur}b`, o), `"a${separateur}b"`);
		}
	});
});
