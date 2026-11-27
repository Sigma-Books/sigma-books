import assert from 'node:assert/strict';
import { test } from 'node:test';
import {
	estBrouillon,
	estEcriture,
	estLotValide,
	estPageBrouillard,
	MOTIF_CONTRE_PASSATION_MAX,
	motifAdmis,
	motifContrePassation,
	parametresBrouillard
} from './brouillard.ts';
import { fixerLangue, langueCourante } from '../i18n/index.ts';
import { nomPieceSur } from './pieces.ts';
import { CODE_JOURNAL_MAX, estCodeJournal, lireUuid } from './types.ts';

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
	assert.equal(estBrouillon({ ...brouillon, version_parametres: 'xx-2026.1' }), true);
	assert.equal(estBrouillon({ ...brouillon, version_parametres: ['x'] }), false);
	assert.equal(estBrouillon({ ...brouillon, champ_futur: 1 }), true);
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
	// Version de paramètres fiscaux : facultative (absente sans version applicable), toujours du texte.
	assert.equal(estEcriture({ ...e, version_parametres: 'xx-2026.1+d1' }), true);
	assert.equal(estEcriture({ ...e, version_parametres: 7 }), false);
	assert.equal(estEcriture({ ...e, version_parametres: null }), false);
	assert.equal(estEcriture({ ...e, champ_futur: true }), true);
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

test('le motif d’une contre-passation est facultatif : un motif par défaut est généré', () => {
	const n = 'OD-2026-000001';
	assert.equal(motifContrePassation('', n, 'fr'), 'Contre-passation de OD-2026-000001');
	assert.equal(motifContrePassation('  \t', n, 'fr'), 'Contre-passation de OD-2026-000001');
	assert.equal(motifContrePassation('Erreur de période', n, 'fr'), 'Erreur de période');
});

test('le motif saisi est rogné', () => {
	assert.equal(motifContrePassation('  Erreur de période \t', 'X', 'fr'), 'Erreur de période');
});

test('le motif par défaut suit la langue demandée, sans état global', () => {
	fixerLangue('fr');
	assert.equal(motifContrePassation('', 'N1', 'en'), 'Reversal of N1');
	assert.equal(motifContrePassation('', 'N1', 'pt'), 'Estorno de N1');
	assert.equal(motifContrePassation('', 'N1', 'es'), 'Contrapartida de N1');
	assert.equal(langueCourante(), 'fr');
});

test('le motif est borné à 400 octets UTF-8, pas à 400 caractères', () => {
	assert.equal(MOTIF_CONTRE_PASSATION_MAX, 400);
	assert.equal(motifAdmis('a'.repeat(400)), true);
	assert.equal(motifAdmis('a'.repeat(401)), false);
	assert.equal(motifAdmis('é'.repeat(200)), true); // 400 octets
	assert.equal(motifAdmis('é'.repeat(201)), false); // 402 octets, 201 caractères
	assert.equal(motifAdmis('€'.repeat(133)), true); // 399 octets
	assert.equal(motifAdmis('€'.repeat(134)), false); // 402 octets
	assert.equal(motifAdmis(''), false);
	assert.equal(motifAdmis('  '), false);
	assert.equal(motifAdmis('ligne\nsuivante'), false);
	assert.equal(motifAdmis('Erreur de période'), true);
});

test('le code de journal suit le contrat de l’API (1 à 8 majuscules ou chiffres)', () => {
	assert.equal(CODE_JOURNAL_MAX, 8);
	assert.equal(estCodeJournal('OD'), true);
	assert.equal(estCodeJournal('A'.repeat(8)), true);
	assert.equal(estCodeJournal('A'.repeat(9)), false);
	assert.equal(estCodeJournal(''), false);
	assert.equal(estCodeJournal('od'), false);
	assert.equal(parametresBrouillard({ journal: 'A'.repeat(9) }), 'limite=50');
});
