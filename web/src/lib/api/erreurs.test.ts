import assert from 'node:assert/strict';
import { describe, it } from 'node:test';
import { ErreurApi, messageErreur, traduireReponse } from './erreurs.ts';

const reponse = (statut: number, corps: unknown) =>
	new Response(JSON.stringify(corps), { status: statut });

describe('motifs des refus 409 et 422', () => {
	it('retient un motif connu', async () => {
		const e = await traduireReponse(reponse(422, { erreur: 'dernier_facteur' }));
		assert.equal(e.code, 'refus');
		assert.equal(e.motif, 'dernier_facteur');
		assert.match(messageErreur(e), /dernier moyen de connexion/);
	});

	it('motif inconnu ou corps invalide : message du statut', async () => {
		const e = await traduireReponse(reponse(409, { erreur: 'inconnu<script>' }));
		assert.equal(e.code, 'conflit');
		assert.equal(e.motif, null);
		assert.equal(messageErreur(e), 'Action impossible dans l’état actuel.');
	});

	it('un motif n’est jamais pris pour un code hors 409 et 422', async () => {
		const e = await traduireReponse(reponse(400, { erreur: 'dernier_facteur' }));
		assert.equal(e.motif, null);
		assert.equal(e.code, 'requete_invalide');
	});

	it('constructeur : motif nul par défaut', () => {
		assert.equal(new ErreurApi('reseau', null).motif, null);
	});
});
