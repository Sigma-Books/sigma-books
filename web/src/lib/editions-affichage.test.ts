import assert from 'node:assert/strict';
import { describe, it } from 'node:test';
import {
	libelleColonne,
	libelleMois,
	messageAlerteBrouillard,
	messageAlerteEcheance,
	messageBrouillonsExclus
} from './editions-affichage.ts';
import { estNomFichierSur } from './telechargement.ts';

describe('affichage des éditions', () => {
	it('colonnes et mois', () => {
		assert.equal(libelleColonne('solde_debiteur'), 'Solde débiteur');
		assert.equal(libelleColonne('inconnue'), 'inconnue');
		assert.equal(libelleColonne('constructor'), 'constructor');
		assert.equal(libelleMois({ annee: 2026, mois: 3 }), 'mars 2026');
		assert.equal(libelleMois({ annee: 2026, mois: 13 }), '2026-13');
	});

	it('brouillons exclus signalés seulement s’il y en a', () => {
		assert.equal(messageBrouillonsExclus({ desequilibres: 0, illisibles: 0 }), null);
		assert.match(messageBrouillonsExclus({ desequilibres: 2, illisibles: 1 }) ?? '', /3 brouillon/);
	});

	it('alertes', () => {
		assert.match(
			messageAlerteBrouillard({
				journal: 'OD',
				debut_periode: '2026-01-01',
				fin_periode: '2026-01-31',
				nombre: 2,
				plus_ancienne: '2026-01-05',
				jours_de_retard: 1
			}),
			/1 jour\b.*OD|Journal OD.*1 jour/
		);
		const e = {
			exercice: '2026',
			debut: '2026-01-01',
			fin: '2026-01-31',
			echeance: '2026-03-31',
			jours_restants: -3,
			depassee: true,
			statut: 'ouverte'
		};
		assert.match(messageAlerteEcheance(e), /dépassée de 3 jours/);
		assert.match(
			messageAlerteEcheance({ ...e, depassee: false, jours_restants: 12 }),
			/dans 12 jours/
		);
	});
});

describe('nom de fichier', () => {
	it('refuse chemins et extensions douteuses', () => {
		assert.equal(
			estNomFichierSur('integrite-dossier-0b0f6c43-7c47-4a43-9d1e-2f4f6f0a1b2c.txt'),
			true
		);
		assert.equal(estNomFichierSur('balance-2026-definitif.csv'), true);
		for (const n of ['../x.txt', 'a/b.txt', 'a..b.txt', '.txt', 'x.html', 'x.exe', 'a b.txt', '']) {
			assert.equal(estNomFichierSur(n), false, n);
		}
	});
});
