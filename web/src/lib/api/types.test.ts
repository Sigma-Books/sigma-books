import assert from 'node:assert/strict';
import { describe, it } from 'node:test';
import { estCabinetMembre, estReponseSession, estSessionListee, lireUuid } from './types.ts';

describe('identifiants lus dans l’URL', () => {
	it('accepte un UUID et le met en minuscules', () => {
		assert.equal(
			lireUuid('0B0F6C43-7C47-4A43-9D1E-2F4F6F0A1B2C'),
			'0b0f6c43-7c47-4a43-9d1e-2f4f6f0a1b2c'
		);
	});

	it('refuse tout ce qui pourrait détourner un chemin de l’API', () => {
		for (const valeur of [
			null,
			undefined,
			'',
			'..',
			'../audit',
			'0b0f6c43-7c47-4a43-9d1e-2f4f6f0a1b2c/audit',
			'0b0f6c43-7c47-4a43-9d1e-2f4f6f0a1b2c?x=1',
			' 0b0f6c43-7c47-4a43-9d1e-2f4f6f0a1b2c',
			'0b0f6c43-7c47-4a43-9d1e-2f4f6f0a1b2c\n',
			'%2e%2e'
		]) {
			assert.equal(lireUuid(valeur), null, String(valeur));
		}
	});
});

describe('gardes des réponses', () => {
	it('cabinet : identifiant canonique exigé', () => {
		const id = '0b0f6c43-7c47-4a43-9d1e-2f4f6f0a1b2c';
		assert.equal(estCabinetMembre({ id, nom: 'Cabinet', role: 'valideur' }), true);
		assert.equal(estCabinetMembre({ id: id.toUpperCase(), nom: 'C', role: 'valideur' }), false);
		assert.equal(estCabinetMembre({ id, nom: 'Cabinet' }), false);
		assert.equal(estCabinetMembre([]), false);
	});

	it('session : jeton anti-CSRF non vide exigé', () => {
		const base = {
			id: 'x',
			nature: 'complete',
			methode: 'passkey',
			cree_le: 'a',
			fin_absolue: 'b',
			fin_inactivite: 'c',
			derniere_verification: 'd',
			fraiche: false,
			fraiche_jusqu_a: 'e'
		};
		assert.equal(estReponseSession({ ...base, csrf: 'c' }), true);
		assert.equal(estReponseSession({ ...base, csrf: '' }), false);
		assert.equal(estReponseSession(base), false);
	});

	it('session listée : appareil et adresse facultatifs', () => {
		const s = {
			id: 'aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee',
			nature: 'complete',
			methode: 'passkey',
			cree_le: 'a',
			derniere_activite: 'b',
			echeance: 'c',
			appareil: null,
			adresse: null,
			courante: true
		};
		assert.equal(estSessionListee(s), true);
		assert.equal(estSessionListee({ ...s, appareil: 3 }), false);
		assert.equal(estSessionListee({ ...s, courante: 'oui' }), false);
	});
});
