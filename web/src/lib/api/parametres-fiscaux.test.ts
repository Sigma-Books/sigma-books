import assert from 'node:assert/strict';
import { describe, it } from 'node:test';
import { chercher, fixerLangue } from '../i18n/index.ts';
import {
	estApplicable,
	estListeVersions,
	estVersionCreee,
	lireApplicable,
	type Applicable,
	type VersionFiscale
} from './parametres-fiscaux.ts';
import type { Uuid } from './types.ts';

// Données d'exemple pour les tests : codes, taux et libellés inventés, qui ne sont pas ceux d'un
// pays réel (l'application n'en connaît aucun).
const PAYS: VersionFiscale = {
	identifiant: 'xx-2026.1',
	portee: 'pays',
	version: '2026.1',
	date_effet: '2026-01-01'
};
const PROPRE: VersionFiscale = {
	identifiant: 'xx-2026.1+d1',
	portee: 'dossier',
	version: 'xx-2026.1',
	numero: 1,
	date_effet: '2026-06-01',
	motif: 'Convention',
	cree_par: '0b0f6c43-7c47-4a43-9d1e-2f4f6f0a1b2c',
	cree_le: '2026-05-30T10:00:00Z',
	empreinte: 'ab'.repeat(32)
};
const BASE: Applicable = {
	date: '2026-07-01',
	version: PROPRE,
	taux: [
		{ role: 'normal', valeur: '20', source: 'Exemple' },
		{ role: 'reduit', valeur: '7.5', source: 'Exemple' }
	],
	regimes: [
		{
			code: 'R_TAXABLE',
			libelle: 'Régime taxable',
			exonere: false,
			ligne_taxe: true,
			taux: ['20'],
			exigibilites: ['livraison'],
			deduction: 'oui'
		}
	],
	motifs: [{ code: 'R_TAXABLE-1', regime: 'R_TAXABLE', libelle: 'Motif' }],
	retention_annees: 10
};

describe('réponses de l’API', () => {
	it('une réponse complète est reconnue ; sans version, rien n’est détaillé', () => {
		assert.equal(estApplicable(BASE), true);
		assert.equal(
			estApplicable({ ...BASE, version: null, taux: [], regimes: [], motifs: [] }),
			true
		);
		assert.equal(estApplicable({ ...BASE, version: null }), false);
		assert.equal(estListeVersions({ pays: 'XX', versions: [PAYS, PROPRE] }), true);
		assert.equal(estListeVersions({ pays: 'XX', versions: [] }), true);
	});

	it('un champ supplémentaire du serveur ne casse pas la lecture', () => {
		assert.equal(estApplicable({ ...BASE, champ_futur: 1 }), true);
		assert.equal(estListeVersions({ pays: 'XX', versions: [PAYS], champ_futur: [] }), true);
	});

	it('une forme invalide ou hostile est refusée', () => {
		const refuses: unknown[] = [
			null,
			[],
			{ ...BASE, date: '2026-02-30' },
			{ ...BASE, taux: [{ role: '<x>', valeur: '20', source: 's' }] },
			{ ...BASE, taux: [{ role: 'normal', valeur: '20', source: 'x'.repeat(2001) }] },
			{ ...BASE, regimes: [{ ...BASE.regimes[0], exonere: 'non' }] },
			{ ...BASE, regimes: [{ ...BASE.regimes[0], code: '' }] },
			{ ...BASE, regimes: [{ ...BASE.regimes[0], code: '../x' }] },
			{ ...BASE, motifs: [{ code: 'M', regime: 'R' }] },
			{ ...BASE, retention_annees: '10' },
			{ ...BASE, version: { ...PAYS, portee: 'autre' } },
			{ ...BASE, version: { ...PAYS, date_effet: 'hier' } },
			{ ...BASE, version: { ...PAYS, date_fin: 'demain' } }
		];
		for (const v of refuses) assert.equal(estApplicable(v), false, JSON.stringify(v));
		assert.equal(
			estListeVersions({ pays: 'XX', versions: [{ ...PROPRE, id: 'pas-un-uuid' }] }),
			false
		);
		assert.equal(estListeVersions({ pays: 'XX', versions: [{ ...PROPRE, numero: 1.5 }] }), false);
		assert.equal(estListeVersions({ pays: 'XX' }), false);
	});

	it('la création rend ses avertissements', () => {
		assert.equal(estVersionCreee({ ...PROPRE, avertissements: [] }), true);
		assert.equal(
			estVersionCreee({
				...PROPRE,
				avertissements: [{ code: 'ecritures_deja_validees', nombre: 3 }]
			}),
			true
		);
		assert.equal(estVersionCreee({ ...PROPRE }), false);
		assert.equal(estVersionCreee({ ...PROPRE, avertissements: [{ code: 'x' }] }), false);
		assert.equal(
			estVersionCreee({ ...PROPRE, avertissements: [{ code: 'x', nombre: 1.5 }] }),
			false
		);
	});

	it('la date demandée est vérifiée avant de former l’adresse', () => {
		const api = {} as never;
		const c = '0b0f6c43-7c47-4a43-9d1e-2f4f6f0a1b2c' as Uuid;
		for (const date of ['../x', '2026-13-01', '2026-07-01&x=1', '']) {
			assert.throws(() => lireApplicable(api, c, c, date), TypeError, date);
		}
	});
});

describe('messages', () => {
	it('les notices de la page ont leur message', () => {
		fixerLangue('fr');
		for (const cle of ['parametres.sans_fin', 'parametres.immuable']) {
			assert.ok((chercher(cle) ?? '') !== '', cle);
		}
	});
});
