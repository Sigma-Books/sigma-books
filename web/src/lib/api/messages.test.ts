import assert from 'node:assert/strict';
import { test } from 'node:test';
import { ErreurApi, lireDetailRefus, traduireReponse } from './erreurs.ts';
import { estConflitDeVersion, messageMetier, messageMotif } from './messages.ts';

const reponse = (statut: number, corps: unknown): Response =>
	new Response(JSON.stringify(corps), { status: statut });

test('un 422 de saisie garde son motif et sa ligne', async () => {
	const e = await traduireReponse(reponse(422, { erreur: 'compte_inactif', ligne: 3 }));
	assert.equal(e.detail?.motif, 'compte_inactif');
	assert.equal(e.detail?.ligne, 3);
	assert.equal(messageMetier(e), 'Ligne 3 : Ce compte est désactivé.');
});

test('un 409 de version périmée est reconnu', async () => {
	const e = await traduireReponse(reponse(409, { erreur: 'brouillon_perime' }));
	assert.equal(estConflitDeVersion(e), true);
	assert.match(messageMetier(e), /modifié par quelqu’un d’autre/);
});

test('un lot refusé liste les écritures et leur code', async () => {
	const e = await traduireReponse(
		reponse(409, {
			erreur: 'lot_refuse',
			refus: [
				{ ecriture: 'a', code: 'ecriture_desequilibree' },
				{ ecriture: 'b', code: 'Mauvais Code' },
				{ code: 'x' }
			]
		})
	);
	assert.deepEqual(e.detail?.refus, [{ ecriture: 'a', code: 'ecriture_desequilibree' }]);
});

test('un code inconnu retombe sur le message général', async () => {
	const e = await traduireReponse(reponse(422, { erreur: 'code_futur' }));
	assert.equal(messageMotif('code_futur'), null);
	assert.equal(messageMetier(e), 'La demande est invalide.');
});

test('le nombre cité par le serveur est lu, jamais une valeur douteuse', () => {
	assert.equal(lireDetailRefus({ erreur: 'brouillard_non_valide', nombre: 4 })?.nombre, 4);
	assert.equal(lireDetailRefus({ erreur: 'x', nombre: -1 })?.nombre, null);
	assert.equal(lireDetailRefus({ erreur: 'x', nombre: '4' })?.nombre, null);
	assert.equal(lireDetailRefus({ erreur: '<script>' }), null);
	assert.equal(lireDetailRefus('texte'), null);
});

test('un 403 ou un 404 ne porte aucun détail', async () => {
	const e = await traduireReponse(reponse(403, { erreur: 'periode_cloturee' }));
	assert.equal(e.code, 'interdit');
	assert.equal(e.detail, null);
	assert.ok(e instanceof ErreurApi);
});

test('tous les motifs du serveur ont un message', () => {
	for (const motif of [
		'journal_inconnu',
		'brouillon_perime',
		'lot_sans_ecriture_a_valider',
		'justification_manquante',
		'date_anterieure_a_l_origine',
		'periode_verrouillee',
		'revue_par_le_poseur'
	]) {
		assert.notEqual(messageMotif(motif), null, motif);
	}
});
