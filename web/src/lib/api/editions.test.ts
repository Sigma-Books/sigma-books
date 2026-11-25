import assert from 'node:assert/strict';
import { describe, it } from 'node:test';
import {
	estAlertes,
	estBalance,
	estCentralisateur,
	estGrandLivre,
	estJournaux,
	estMontantChaine,
	FILTRE_VIDE,
	lireBalance,
	lireListe,
	parametresEdition,
	verifierFiltre
} from './editions.ts';
import type { ClientApi } from './client.ts';
import { lireUuid, type Uuid } from './types.ts';

const C = lireUuid('0b0f6c43-7c47-4a43-9d1e-2f4f6f0a1b2c') as Uuid;
const D = lireUuid('1b0f6c43-7c47-4a43-9d1e-2f4f6f0a1b2c') as Uuid;

const entete = {
	exercice: '2026',
	du: '2026-01-01',
	au: '2026-12-31',
	mode: 'definitif',
	brouillons_exclus: { desequilibres: 0, illisibles: 0 }
};

const balance = {
	...entete,
	format: 2,
	regroupement: 'compte',
	colonnes: ['solde_debiteur', 'solde_crediteur'],
	lignes: [{ cle: '411', valeurs: ['1500.5', '0'] }],
	totaux: ['1500.5', '1500.5'],
	equilibree: true
};

describe('montants', () => {
	it('accepte une chaîne décimale exacte, refuse nombre et formes douteuses', () => {
		assert.equal(estMontantChaine('1500.5'), true);
		assert.equal(estMontantChaine('-12'), true);
		assert.equal(estMontantChaine(1500.5), false);
		assert.equal(estMontantChaine('1'.repeat(60) + '.' + '5'.repeat(30)), true);
		assert.equal(estMontantChaine('1'.repeat(61)), false);
		assert.equal(estMontantChaine('1e5'), false);
		assert.equal(estMontantChaine('1 500'), false);
		assert.equal(estMontantChaine(''), false);
	});
});

describe('gardes des éditions', () => {
	it('balance : colonnes cohérentes avec le format, montants en chaînes', () => {
		assert.equal(estBalance(balance), true);
		assert.equal(estBalance({ ...balance, format: 4 }), false);
		assert.equal(estBalance({ ...balance, totaux: [1500.5, 1500.5] }), false);
		assert.equal(estBalance({ ...balance, lignes: [{ cle: '411', valeurs: ['1'] }] }), false);
		assert.equal(estBalance({ ...balance, mode: 'autre' }), false);
		assert.equal(estBalance({ ...balance, equilibree: 'oui' }), false);
	});

	it('grand livre, journaux, centralisateur', () => {
		const ligne = {
			date: '2026-03-01',
			journal: 'VT',
			numero: null,
			date_valeur_comptable: null,
			libelle: 'Facture',
			tiers: null,
			debit: '100',
			credit: '0',
			solde: '100'
		};
		const gl = {
			...entete,
			comptes: [
				{
					compte: '411',
					solde_ouverture: '0',
					lignes: [ligne],
					total_debit: '100',
					total_credit: '0',
					solde_final: '100'
				}
			]
		};
		assert.equal(estGrandLivre(gl), true);
		assert.equal(
			estGrandLivre({ ...gl, comptes: [{ ...gl.comptes[0], solde_final: 100 }] }),
			false
		);
		const jx = {
			...entete,
			journaux: [
				{
					journal: 'VT',
					ecritures: [
						{
							numero: 'VT-1',
							date: '2026-03-01',
							date_valeur_comptable: null,
							libelle: 'x',
							lignes: [{ compte: '411', debit: '100', credit: '0', tiers: null }]
						}
					],
					total_debit: '100',
					total_credit: '100'
				}
			]
		};
		assert.equal(estJournaux(jx), true);
		const mois = { annee: 2026, mois: 3, journal: 'VT', compte: null, debit: '1', credit: '0' };
		const rub = {
			par_journal: [mois],
			par_compte: [],
			par_mois: [],
			total_debit: '1',
			total_credit: '0'
		};
		assert.equal(estCentralisateur({ ...entete, mouvements: rub, a_nouveaux: rub }), true);
		assert.equal(estCentralisateur({ ...entete, mouvements: rub }), false);
	});

	it('alertes', () => {
		assert.equal(
			estAlertes({
				date_du_jour: '2026-11-25',
				brouillard_non_valide: [
					{
						journal: 'OD',
						debut_periode: '2026-01-01',
						fin_periode: '2026-01-31',
						nombre: 2,
						plus_ancienne: '2026-01-05',
						jours_de_retard: 300
					}
				],
				echeances_de_cloture: []
			}),
			true
		);
		assert.equal(estAlertes({ date_du_jour: 'x' }), false);
	});
});

describe('filtre', () => {
	it('liste : séparateurs variés, sans doublon', () => {
		assert.deepEqual(lireListe('411, 401;  411 512'), ['411', '401', '512']);
		assert.deepEqual(lireListe('  '), []);
	});

	it('paramètres : seulement ce que le serveur admet pour la nature', () => {
		const f = {
			...FILTRE_VIDE,
			exercice: '2026',
			du: '2026-02-01',
			mode: 'provisoire' as const,
			comptes: '411 401',
			journaux: 'VT',
			format: 8 as const,
			regroupement: 'tronc' as const
		};
		assert.equal(
			parametresEdition(f, 'balance'),
			'exercice=2026&du=2026-02-01&mode=provisoire&comptes=411%2C401&journaux=VT&format=8&regroupement=tronc'
		);
		assert.equal(
			parametresEdition(f, 'grand-livre'),
			'exercice=2026&du=2026-02-01&mode=provisoire&comptes=411%2C401&journaux=VT'
		);
		// Les journaux n'admettent pas de filtre de comptes.
		assert.equal(
			parametresEdition(f, 'journaux'),
			'exercice=2026&du=2026-02-01&mode=provisoire&journaux=VT'
		);
		assert.equal(parametresEdition(FILTRE_VIDE, 'balance'), '');
	});

	it('valeurs hostiles encodées, jamais recopiées telles quelles', () => {
		const q = parametresEdition({ ...FILTRE_VIDE, journaux: 'A&mode=x#' }, 'centralisateur');
		assert.ok(!q.includes('&mode='));
	});

	it('contrôles de confort', () => {
		assert.deepEqual(verifierFiltre(FILTRE_VIDE, 'balance'), []);
		assert.deepEqual(verifierFiltre({ ...FILTRE_VIDE, du: '2026-02-30' }, 'balance'), ['du']);
		assert.deepEqual(
			verifierFiltre({ ...FILTRE_VIDE, du: '2026-05-01', au: '2026-04-01' }, 'balance'),
			['bornes_inversees']
		);
		assert.deepEqual(verifierFiltre({ ...FILTRE_VIDE, exercice: '26' }, 'balance'), ['exercice']);
		assert.deepEqual(verifierFiltre({ ...FILTRE_VIDE, comptes: '0411' }, 'balance'), ['comptes']);
		assert.deepEqual(verifierFiltre({ ...FILTRE_VIDE, comptes: '0411' }, 'journaux'), []);
		assert.deepEqual(verifierFiltre({ ...FILTRE_VIDE, journaux: 'vt' }, 'balance'), ['journaux']);
		const beaucoup = Array.from({ length: 51 }, (_, i) => `J${i}`).join(',');
		assert.deepEqual(verifierFiltre({ ...FILTRE_VIDE, journaux: beaucoup }, 'balance'), [
			'trop_de_journaux'
		]);
	});
});

describe('appel', () => {
	it('lit la balance sur le chemin du cabinet et du dossier, avec la garde', async () => {
		let vu = '';
		const api = {
			lire: async (chemin: string, garde: (v: unknown) => boolean) => {
				vu = chemin;
				assert.equal(garde(balance), true);
				return balance;
			}
		} as unknown as ClientApi;
		await lireBalance(api, C, D, { ...FILTRE_VIDE, format: 2 });
		assert.equal(vu, `/api/cabinets/${C}/dossiers/${D}/editions/balance?format=2`);
	});
});
