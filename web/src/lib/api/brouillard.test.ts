import assert from 'node:assert/strict';
import { test } from 'node:test';
import {
	estBrouillon,
	estEcriture,
	estLotValide,
	estPageBrouillard,
	parametresBrouillard
} from './brouillard.ts';
import { nomPieceSur } from './pieces.ts';
import { lireUuid } from './types.ts';

const ID = '0b6f2f0e-8a3a-4a39-9d0c-3d6a8d6d3c11';

const brouillon = {
	id: ID,
	journal: 'OD',
	date: '2026-03-15',
	libelle: 'Loyer',
	a_revoir: false,
	version: 1,
	cree_par: ID,
	cree_le: '2026-03-15T10:00:00Z',
	modifie_par: ID,
	modifie_le: '2026-03-15T10:00:00Z',
	contributeurs: [ID],
	lignes: [{ position: 1, compte: '6222', sens: 'debit', montant: '150000.5' }],
	pieces: []
};

test('garde du brouillon : montant en chaîne, sens fermé', () => {
	assert.equal(estBrouillon(brouillon), true);
	assert.equal(
		estBrouillon({ ...brouillon, lignes: [{ ...brouillon.lignes[0], montant: 150000.5 }] }),
		false
	);
	assert.equal(
		estBrouillon({ ...brouillon, lignes: [{ ...brouillon.lignes[0], sens: 'D' }] }),
		false
	);
	assert.equal(estBrouillon({ ...brouillon, id: 'pas-un-uuid' }), false);
	assert.equal(estBrouillon({ ...brouillon, version: 1.5 }), false);
});

test('garde de la page : curseur de forme sûre, absent en fin de liste', () => {
	const resume = {
		id: ID,
		journal: 'OD',
		date: '2026-03-15',
		libelle: 'x',
		a_revoir: true,
		version: 2,
		lignes: 2,
		debit: '10',
		credit: '10',
		cree_par: ID,
		cree_le: 'a',
		modifie_par: ID,
		modifie_le: 'b'
	};
	assert.equal(estPageBrouillard({ brouillons: [resume] }), true);
	assert.equal(estPageBrouillard({ brouillons: [resume], suivant: `2026-03-15_${ID}` }), true);
	assert.equal(estPageBrouillard({ brouillons: [resume], suivant: '../x?y=1' }), false);
	assert.equal(estPageBrouillard({ brouillons: [{ ...resume, lignes: '2' }] }), false);
});

test('garde de l’écriture validée : contre-passations en liste de numéros', () => {
	const e = {
		id: ID,
		numero: 'OD-2026-0001',
		journal: 'OD',
		exercice: '2026',
		date: '2026-03-15',
		libelle: 'x',
		rang: 1,
		empreinte: 'e',
		empreinte_precedente: 'p',
		format: 3,
		valide_par: ID,
		date_validation: '2026-03-16',
		valide_le: '2026-03-16T09:00:00Z',
		contre_passations: ['OD-2026-0002', 'OD-2026-0003'],
		lignes: [],
		pieces: [{ piece: ID, role: 'principale', empreinte: 'abc' }]
	};
	assert.equal(estEcriture(e), true);
	assert.equal(estEcriture({ ...e, contre_passations: 'OD-2026-0002' }), false);
	assert.equal(estLotValide({ ecritures: [], exclues_a_revoir: 2 }), true);
	assert.equal(estLotValide({ ecritures: [], exclues_a_revoir: '2' }), false);
});

test('paramètres du brouillard : formes invalides ignorées, pas d’injection', () => {
	assert.equal(parametresBrouillard({ journal: 'OD' }), `journal=OD&limite=50`);
	assert.equal(parametresBrouillard({ journal: 'od&x=1' }), 'limite=50');
	const periode = lireUuid(ID);
	assert.equal(
		parametresBrouillard({ periode, apres: `2026-03-15_${ID}` }, 20),
		`periode=${ID}&apres=2026-03-15_${ID}&limite=20`
	);
});

test('nom de pièce sûr : sans chemin, sans contrôle, borné en octets', () => {
	assert.equal(nomPieceSur('../../etc/passwd'), '.._.._etc_passwd');
	assert.equal(nomPieceSur('a\u0000b\nc.pdf'), 'abc.pdf');
	assert.ok(new TextEncoder().encode(nomPieceSur('é'.repeat(300))).length <= 255);
});
