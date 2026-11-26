import assert from 'node:assert/strict';
import { describe, it } from 'node:test';
import type { ClientApi } from './client.ts';
import {
	estEtatIntegrite,
	estListeJetons,
	estRapportVerification,
	exporterIntegrite,
	lireEntetesExport
} from './integrite.ts';
import { lireUuid, type Uuid } from './types.ts';

const C = lireUuid('0b0f6c43-7c47-4a43-9d1e-2f4f6f0a1b2c') as Uuid;
const D = lireUuid('1b0f6c43-7c47-4a43-9d1e-2f4f6f0a1b2c') as Uuid;

const jeton = {
	id: C,
	nombre: 10,
	tete: 'ab'.repeat(32),
	autorite: 'tsa.example',
	date_jeton: '2026-11-24T00:00:00Z',
	obtenu_le: '2026-11-24T00:00:05Z',
	declencheur: 'quotidien'
};

const etat = {
	dossier: D,
	nombre: 12,
	tete: 'cd'.repeat(32),
	jetons: 1,
	dernier_jeton: jeton,
	ecritures_sans_jeton: 2,
	horodatage: { en_attente_depuis: null, retard: false },
	temoin: { contenu: 'sigma-temoin', nombre: 12, tete: 'cd'.repeat(32) }
};

describe('gardes de l’intégrité', () => {
	it('état', () => {
		assert.equal(estEtatIntegrite(etat), true);
		assert.equal(estEtatIntegrite({ ...etat, dernier_jeton: null }), true);
		assert.equal(estEtatIntegrite({ ...etat, nombre: '12' }), false);
		assert.equal(estEtatIntegrite({ ...etat, horodatage: { retard: false } }), false);
	});

	it('jetons : le DER n’est pas attendu, une forme inconnue est refusée', () => {
		assert.equal(estListeJetons({ total: 1, jetons: [jeton] }), true);
		assert.equal(estListeJetons({ total: 1, jetons: [{ ...jeton, nombre: 'x' }] }), false);
	});

	it('rapport de vérification', () => {
		const rapport = {
			integre: false,
			nombre: 12,
			tete: 'cd'.repeat(32),
			jetons: 1,
			jetons_valides: 1,
			jetons_non_verifies: 0,
			premiere_alteration: 7,
			anomalies: [{ code: 'empreinte_alteree', rang: 7, numero: 'VT-7', detail: null }],
			anomalies_omises: 3
		};
		assert.equal(estRapportVerification(rapport), true);
		assert.equal(estRapportVerification({ ...rapport, premiere_alteration: null }), true);
		assert.equal(estRapportVerification({ ...rapport, anomalies: [{ code: 1 }] }), false);
	});
});

describe('export', () => {
	it('en-têtes : seules les formes attendues sont lues', () => {
		const h = new Headers({
			'x-sigma-export-tronque': 'true',
			'x-sigma-jetons-total': '12000',
			'x-sigma-jetons-exportes': '10000'
		});
		assert.deepEqual(lireEntetesExport(h), {
			tronque: true,
			jetonsTotal: 12000,
			jetonsExportes: 10000
		});
		assert.equal(
			lireEntetesExport(new Headers({ 'x-sigma-export-tronque': 'false' })).tronque,
			false
		);
		assert.deepEqual(lireEntetesExport(new Headers({ 'x-sigma-jetons-total': '-1' })), {
			tronque: null,
			jetonsTotal: null,
			jetonsExportes: null
		});
	});

	it('nom de fichier construit ici, jamais repris du serveur', async () => {
		let methode = '';
		let chemin = '';
		const api = {
			telecharger: async (m: string, c: string) => {
				methode = m;
				chemin = c;
				return {
					contenu: new Blob(['x']),
					entetes: new Headers({ 'content-disposition': 'attachment; filename="../../x"' })
				};
			}
		} as unknown as ClientApi;
		const f = await exporterIntegrite(api, C, D);
		assert.equal(methode, 'POST');
		assert.equal(chemin, `/api/cabinets/${C}/dossiers/${D}/integrite/export`);
		assert.equal(f.nom, `integrite-dossier-${D}.txt`);
		assert.equal(f.tronque, null, 'en-tête absent : troncature inconnue');
	});
});
