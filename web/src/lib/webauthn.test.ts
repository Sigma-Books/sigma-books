// Conversions WebAuthn : formats du serveur (`webauthn-rs`) vers le navigateur, et retour.

import assert from 'node:assert/strict';
import { describe, it } from 'node:test';
import {
	base64urlVersOctets,
	credentialCree,
	credentialDemande,
	octetsVersBase64url,
	optionsCreation,
	optionsDemande
} from './webauthn.ts';

const tampon = (...o: number[]): ArrayBuffer => new Uint8Array(o).buffer;

describe('base64url', () => {
	it('aller-retour sans remplissage, alphabet URL', () => {
		const octets = new Uint8Array([251, 255, 254, 0, 1]);
		const texte = octetsVersBase64url(octets);
		assert.equal(texte, '-__-AAE');
		assert.deepEqual(base64urlVersOctets(texte), octets);
	});

	it('refuse tout caractère hors alphabet et toute longueur impossible', () => {
		assert.equal(base64urlVersOctets('ab+/'), null);
		assert.equal(base64urlVersOctets('ab=='), null);
		assert.equal(base64urlVersOctets('a'), null);
		assert.deepEqual(base64urlVersOctets(''), new Uint8Array());
	});
});

describe('options', () => {
	const demande = {
		publicKey: {
			challenge: 'AQID',
			timeout: 300000,
			rpId: 'compta.exemple.sn',
			allowCredentials: [{ type: 'public-key', id: 'BAUG', transports: ['usb', 'inconnu'] }],
			userVerification: 'required',
			extensions: { inattendu: true }
		}
	};

	it('défi d’authentification', () => {
		const o = optionsDemande(demande);
		assert.ok(o);
		assert.deepEqual(o.challenge, new Uint8Array([1, 2, 3]));
		assert.equal(o.rpId, 'compta.exemple.sn');
		assert.deepEqual(o.allowCredentials, [
			{ type: 'public-key', id: new Uint8Array([4, 5, 6]), transports: ['usb'] }
		]);
		assert.equal(o.userVerification, 'required');
		assert.equal(o.timeout, 300000);
		assert.ok(!('extensions' in o), 'extensions non reprises');
	});

	it('forme invalide : null', () => {
		assert.equal(optionsDemande(null), null);
		assert.equal(optionsDemande({ publicKey: { ...demande.publicKey, challenge: '***' } }), null);
		assert.equal(optionsDemande({ publicKey: { ...demande.publicKey, rpId: 3 } }), null);
		assert.equal(
			optionsDemande({ publicKey: { ...demande.publicKey, allowCredentials: [{ type: 'x' }] } }),
			null
		);
	});

	const creation = {
		publicKey: {
			rp: { name: 'Sigma Books', id: 'compta.exemple.sn' },
			user: { id: 'AQID', name: 'awa@cabinet.sn', displayName: 'Awa' },
			challenge: 'BAUG',
			pubKeyCredParams: [{ type: 'public-key', alg: -7 }],
			excludeCredentials: [{ type: 'public-key', id: 'BwgJ' }],
			authenticatorSelection: {
				residentKey: 'discouraged',
				requireResidentKey: false,
				userVerification: 'required'
			},
			attestation: 'none'
		}
	};

	it('défi d’enrôlement', () => {
		const o = optionsCreation(creation);
		assert.ok(o);
		assert.deepEqual(o.user.id, new Uint8Array([1, 2, 3]));
		assert.deepEqual(o.pubKeyCredParams, [{ type: 'public-key', alg: -7 }]);
		assert.deepEqual(o.excludeCredentials, [{ type: 'public-key', id: new Uint8Array([7, 8, 9]) }]);
		assert.deepEqual(o.authenticatorSelection, {
			userVerification: 'required',
			residentKey: 'discouraged',
			requireResidentKey: false
		});
		assert.equal(o.attestation, 'none');
	});

	it('enrôlement : forme invalide', () => {
		const p = creation.publicKey;
		assert.equal(optionsCreation({ publicKey: { ...p, user: { ...p.user, id: '*' } } }), null);
		assert.equal(optionsCreation({ publicKey: { ...p, pubKeyCredParams: [{ alg: 'x' }] } }), null);
		assert.equal(optionsCreation({ publicKey: { ...p, rp: { name: 1 } } }), null);
	});
});

describe('credentials', () => {
	it('authentification : octets en base64url, userHandle facultatif', () => {
		const j = credentialDemande({
			id: 'AQ',
			rawId: tampon(1),
			type: 'public-key',
			response: {
				clientDataJSON: tampon(2),
				authenticatorData: tampon(3),
				signature: tampon(4),
				userHandle: null
			}
		});
		assert.deepEqual(j, {
			id: 'AQ',
			rawId: 'AQ',
			type: 'public-key',
			response: { authenticatorData: 'Aw', clientDataJSON: 'Ag', signature: 'BA' }
		});
	});

	it('réponse incomplète : null', () => {
		const base = { id: 'AQ', rawId: tampon(1), type: 'public-key' };
		assert.equal(credentialDemande({ ...base, response: { clientDataJSON: tampon(2) } }), null);
		assert.equal(credentialCree({ ...base, response: { clientDataJSON: tampon(2) } }), null);
	});

	it('enrôlement', () => {
		const j = credentialCree({
			id: 'AQ',
			rawId: tampon(1),
			type: 'public-key',
			response: { clientDataJSON: tampon(2), attestationObject: tampon(5) }
		});
		assert.deepEqual(j, {
			id: 'AQ',
			rawId: 'AQ',
			type: 'public-key',
			response: { attestationObject: 'BQ', clientDataJSON: 'Ag' }
		});
	});
});
