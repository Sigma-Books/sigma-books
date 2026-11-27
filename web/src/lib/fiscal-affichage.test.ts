import assert from 'node:assert/strict';
import { describe, it } from 'node:test';
import { afficherTaux, libelleAuteur } from './fiscal-affichage.ts';
import { regionalDe } from './regional-exemples.ts';

describe('taux', () => {
	it('le séparateur décimal est celui du locale du dossier, la chaîne reste exacte', () => {
		assert.equal(afficherTaux('7.5', regionalDe()), '7,5');
		assert.equal(afficherTaux('18', regionalDe()), '18');
		assert.equal(afficherTaux('7.5', regionalDe({ langue: 'en', pays: 'CM' })), '7.5');
		assert.equal(afficherTaux('0.000001', regionalDe()), '0,000001');
		// Aucune conversion en nombre : un grand nombre de décimales n'est ni arrondi ni tronqué.
		assert.equal(afficherTaux('99.999999', regionalDe()), '99,999999');
	});

	it('une forme inattendue est rendue telle quelle', () => {
		for (const brut of ['', 'abc', '7,5', '1e2', '-3', '7.1234567', '<b>']) {
			assert.equal(afficherTaux(brut, regionalDe()), brut);
		}
	});
});

describe('auteur', () => {
	const ID = '0b0f6c43-7c47-4a43-9d1e-2f4f6f0a1b2c';

	it('le nom s’il est connu, sinon l’identifiant abrégé', () => {
		assert.equal(libelleAuteur(ID, new Map([[ID, 'Awa Diop']])), 'Awa Diop');
		assert.equal(libelleAuteur(ID, new Map()), '0b0f6c43…');
		assert.equal(libelleAuteur('abc', new Map()), 'abc');
	});
});
