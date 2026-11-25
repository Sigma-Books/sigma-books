import assert from 'node:assert/strict';
import { describe, it } from 'node:test';
import { ErreurApi, lireDetailRefus } from './erreurs.ts';
import {
	detailAnomalie,
	estEditionTropVolumineuse,
	messageAnomalie,
	messageEdition,
	messageIntegrite
} from './messages-editions.ts';

const trop = new ErreurApi(
	'conflit',
	409,
	null,
	null,
	lireDetailRefus({ erreur: 'edition_trop_volumineuse', nombre: 20000 })
);

describe('messages des éditions', () => {
	it('409 edition_trop_volumineuse : demande de restreindre la période', () => {
		assert.equal(estEditionTropVolumineuse(trop), true);
		const m = messageEdition(trop);
		assert.match(m, /restreignez la période/);
		assert.match(m, /20000 écritures/);
	});

	it('un autre 409 n’est pas pris pour une édition trop volumineuse', () => {
		const autre = new ErreurApi('conflit', 409, null, null, lireDetailRefus({ erreur: 'autre' }));
		assert.equal(estEditionTropVolumineuse(autre), false);
		assert.equal(messageEdition(autre), 'Action impossible dans l’état actuel.');
	});

	it('400 et 403 : messages propres, sans donnée de la requête', () => {
		assert.match(messageEdition(new ErreurApi('requete_invalide', 400)), /filtre/);
		assert.match(messageEdition(new ErreurApi('interdit', 403)), /rôle/);
		assert.match(messageIntegrite(new ErreurApi('interdit', 403)), /session fraîche/);
		assert.equal(
			messageEdition(new ErreurApi('reseau', null)),
			'Serveur injoignable. Vérifiez votre connexion.'
		);
	});
});

describe('anomalies de la chaîne', () => {
	it('codes connus traduits, inconnus bornés', () => {
		assert.match(messageAnomalie('empreinte_alteree'), /empreinte/);
		assert.equal(messageAnomalie('nouveau_code'), 'Anomalie « nouveau_code ».');
		assert.equal(messageAnomalie('<img src=x>'), 'Anomalie inconnue.');
		assert.equal(messageAnomalie('constructor'), 'Anomalie « constructor ».');
	});

	it('détail : seulement un code stable', () => {
		assert.equal(detailAnomalie('libelle'), 'libelle');
		assert.equal(detailAnomalie('un libellé <b>'), null);
		assert.equal(detailAnomalie(null), null);
	});
});
