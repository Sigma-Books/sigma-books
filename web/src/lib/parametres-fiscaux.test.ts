import assert from 'node:assert/strict';
import { describe, it } from 'node:test';
import { ErreurApi, traduireReponse } from './api/erreurs.ts';
import { messageMetier } from './api/messages.ts';
import {
	MOTIFS_PARAMETRES,
	type Applicable,
	type VersionFiscale
} from './api/parametres-fiscaux.ts';
import { chercher, fixerLangue } from './i18n/index.ts';
import {
	ajouterJours,
	CLES_PROBLEME_VERSION,
	corpsVersion,
	dateEffetLimite,
	derniereDateEffetPropre,
	diagnostiquerVersion,
	formulaireVersionVide,
	messageProblemeVersion,
	motifSur,
	pourraitEtreRetroactive,
	regimesRetirables,
	tauxCanonique,
	tauxChanges,
	VERSIONS_PROPRES_MAX,
	type EtatVersions,
	type FormulaireVersion
} from './parametres-fiscaux.ts';

// Données d'exemple pour les tests : codes, taux et libellés inventés, qui ne sont pas ceux d'un
// pays réel (l'application n'en connaît aucun).
const PAYS: VersionFiscale = {
	identifiant: 'xx-2026.1',
	portee: 'pays',
	version: '2026.1',
	date_effet: '2026-01-01'
};
const PROPRE: VersionFiscale = {
	identifiant: 'xx-2026.1+d1',
	portee: 'dossier',
	version: 'xx-2026.1',
	numero: 1,
	date_effet: '2026-06-01',
	motif: 'Convention',
	cree_par: '0b0f6c43-7c47-4a43-9d1e-2f4f6f0a1b2c',
	cree_le: '2026-05-30T10:00:00Z',
	empreinte: 'ab'.repeat(32)
};
const BASE: Applicable = {
	date: '2026-07-01',
	version: PROPRE,
	taux: [
		{ role: 'normal', valeur: '20', source: 'Exemple' },
		{ role: 'reduit', valeur: '7.5', source: 'Exemple' }
	],
	regimes: [
		{
			code: 'R_TAXABLE',
			libelle: 'Régime taxable',
			exonere: false,
			ligne_taxe: true,
			taux: ['20'],
			exigibilites: ['livraison'],
			deduction: 'oui'
		},
		{
			code: 'R_EXO',
			libelle: 'Régime exonéré',
			exonere: true,
			ligne_taxe: false,
			taux: [],
			exigibilites: [],
			deduction: 'non'
		},
		{
			code: 'R_MOTIFS',
			libelle: 'Régime à motifs',
			exonere: false,
			ligne_taxe: false,
			taux: [],
			exigibilites: [],
			deduction: 'oui'
		}
	],
	motifs: [{ code: 'R_MOTIFS-1', regime: 'R_MOTIFS', libelle: 'Motif' }],
	retention_annees: 10
};
const ETAT: EtatVersions = { aujourdhui: '2026-11-27', versions: [PAYS], base: BASE };
const VALIDE: FormulaireVersion = {
	...formulaireVersionVide(),
	dateEffet: '2026-12-01',
	motif: 'Décision de l’administration',
	taux: { normal: '19' }
};

describe('motif', () => {
	it('1 à 400 octets UTF-8, non blanc', () => {
		assert.equal(motifSur('Décision'), true);
		assert.equal(motifSur(''), false);
		assert.equal(motifSur('   '), false);
		assert.equal(motifSur('a'.repeat(400)), true);
		assert.equal(motifSur('a'.repeat(401)), false);
		// 200 caractères de 2 octets : 400 octets, admis ; 201 : refusés.
		assert.equal(motifSur('é'.repeat(200)), true);
		assert.equal(motifSur('é'.repeat(201)), false);
	});

	it('caractères de contrôle et bidirectionnels refusés', () => {
		for (const c of ['\u0000', '\n', '\t', '\u007f', '\u0085', '\u009f']) {
			assert.equal(motifSur(`a${c}b`), false, JSON.stringify(c));
		}
		for (const point of [0x200e, 0x200f, 0x202a, 0x202e, 0x2066, 0x2069]) {
			const c = String.fromCodePoint(point);
			assert.equal(motifSur(`a${c}b`), false, point.toString(16));
		}
		// Les caractères voisins sont admis.
		assert.equal(motifSur('a⁥b c‍'), true);
	});
});

describe('taux', () => {
	it('décimal canonique dans ]0 ; 100]', () => {
		for (const bon of ['18', '5.5', '0.5', '100', '0.000001', '99.999999']) {
			assert.equal(tauxCanonique(bon), true, bon);
		}
		for (const mauvais of [
			'',
			'0',
			'0.0',
			'00',
			'018',
			'5.50',
			'5.',
			'.5',
			'-5',
			'+5',
			'1e2',
			'100.5',
			'101',
			'1000',
			'0.0000001',
			'5,5',
			' 5',
			'5 ',
			'１８'
		]) {
			assert.equal(tauxCanonique(mauvais), false, JSON.stringify(mauvais));
		}
	});
});

describe('dates', () => {
	it('un an après le jour du dossier : 366 jours', () => {
		assert.equal(ajouterJours('2026-11-27', 366), '2027-11-28');
		assert.equal(dateEffetLimite('2027-01-01'), '2028-01-02');
		assert.equal(ajouterJours('2026-12-31', 1), '2027-01-01');
	});

	it('la dernière version propre fixe la date plancher', () => {
		assert.equal(derniereDateEffetPropre([PAYS]), null);
		assert.equal(derniereDateEffetPropre([PAYS, PROPRE]), '2026-06-01');
		assert.equal(
			derniereDateEffetPropre([PROPRE, { ...PROPRE, numero: 2, date_effet: '2026-09-01' }]),
			'2026-09-01'
		);
	});

	it('une date déjà atteinte peut être rétroactive', () => {
		assert.equal(pourraitEtreRetroactive('2026-11-27', '2026-11-27'), true);
		assert.equal(pourraitEtreRetroactive('2026-01-01', '2026-11-27'), true);
		assert.equal(pourraitEtreRetroactive('2026-11-28', '2026-11-27'), false);
		assert.equal(pourraitEtreRetroactive('nimporte', '2026-11-27'), false);
	});
});

describe('régimes retirables', () => {
	it('ni exonérés ni à motifs : seulement ce que la version applicable offre', () => {
		assert.deepEqual(
			regimesRetirables(BASE).map((r) => r.code),
			['R_TAXABLE']
		);
	});
});

describe('contrôle de la création', () => {
	const cles = (f: FormulaireVersion, etat: EtatVersions = ETAT) =>
		diagnostiquerVersion(f, etat).map((p) => p.cle);

	it('une version valide ne pose aucun problème', () => {
		assert.deepEqual(cles(VALIDE), []);
	});

	it('date d’effet : valide, au plus 366 jours après le jour du dossier', () => {
		assert.deepEqual(cles({ ...VALIDE, dateEffet: '' }), ['date_effet_invalide']);
		assert.deepEqual(cles({ ...VALIDE, dateEffet: '2027-11-28' }), []);
		assert.deepEqual(cles({ ...VALIDE, dateEffet: '2027-11-29' }), ['date_effet_lointaine']);
		assert.deepEqual(cles({ ...VALIDE, dateEffet: '9999-12-31' }), ['date_effet_lointaine']);
		const [p] = diagnostiquerVersion({ ...VALIDE, dateEffet: '2027-11-29' }, ETAT);
		assert.match(messageProblemeVersion(p as NonNullable<typeof p>), /2027-11-28/);
	});

	it('date d’effet postérieure à la dernière version propre', () => {
		const etat = { ...ETAT, versions: [PAYS, PROPRE] };
		assert.deepEqual(cles({ ...VALIDE, dateEffet: '2026-06-01' }, etat), ['date_effet_anterieure']);
		assert.deepEqual(cles({ ...VALIDE, dateEffet: '2026-05-01' }, etat), ['date_effet_anterieure']);
		assert.deepEqual(cles({ ...VALIDE, dateEffet: '2026-06-02' }, etat), []);
	});

	it('date de fin : facultative, valide, pas avant la date d’effet', () => {
		assert.deepEqual(cles({ ...VALIDE, dateFin: '2027-01-01' }), []);
		assert.deepEqual(cles({ ...VALIDE, dateFin: '2026-12-01' }), []);
		assert.deepEqual(cles({ ...VALIDE, dateFin: '2026-11-30' }), ['date_fin_anterieure']);
		assert.deepEqual(cles({ ...VALIDE, dateFin: '2026-02-30' }), ['date_fin_invalide']);
	});

	it('motif obligatoire et sûr', () => {
		assert.deepEqual(cles({ ...VALIDE, motif: '' }), ['motif_invalide']);
		assert.deepEqual(cles({ ...VALIDE, motif: `x${String.fromCodePoint(0x202e)}y` }), [
			'motif_invalide'
		]);
		assert.deepEqual(cles({ ...VALIDE, motif: 'x'.repeat(401) }), ['motif_invalide']);
	});

	it('taux : forme canonique ; au moins un changement ; une valeur identique n’en est pas un', () => {
		assert.deepEqual(cles({ ...VALIDE, taux: { normal: '19,5' } }), ['taux_invalide']);
		assert.deepEqual(cles({ ...VALIDE, taux: { normal: '0' } }), ['taux_invalide']);
		assert.deepEqual(cles({ ...VALIDE, taux: {} }), ['aucun_changement']);
		assert.deepEqual(cles({ ...VALIDE, taux: { normal: '20' } }), ['aucun_changement']);
		assert.deepEqual(cles({ ...VALIDE, taux: {}, retires: ['R_TAXABLE'] }), []);
	});

	it('sans version applicable à la date, aucune base', () => {
		const vide: Applicable = { ...BASE, version: null, taux: [], regimes: [], motifs: [] };
		assert.deepEqual(cles(VALIDE, { ...ETAT, base: vide }), ['aucune_version_applicable']);
		assert.deepEqual(cles(VALIDE, { ...ETAT, base: null }), ['aucune_version_applicable']);
	});

	it('au plus 50 versions propres', () => {
		const versions = Array.from({ length: VERSIONS_PROPRES_MAX }, (_, i) => ({
			...PROPRE,
			numero: i + 1,
			date_effet: ajouterJours('2020-01-01', i)
		}));
		const etat = { ...ETAT, versions };
		assert.ok(cles(VALIDE, etat).includes('trop_de_versions'));
		const [p] = diagnostiquerVersion(VALIDE, etat);
		assert.match(messageProblemeVersion(p as NonNullable<typeof p>), /50/);
		assert.deepEqual(cles(VALIDE, { ...ETAT, versions: versions.slice(1) }), []);
	});

	it('chaque contrôle a son message, sans paramètre oublié', () => {
		fixerLangue('fr');
		for (const cle of CLES_PROBLEME_VERSION) {
			const message = messageProblemeVersion({
				champ: 'x',
				cle,
				params: { limite: '2027-01-01', derniere: '2026-01-01', max: 50 }
			});
			assert.notEqual(message, '', cle);
			assert.ok(!message.includes('{'), cle);
		}
	});
});

describe('corps de la création', () => {
	it('seuls les taux changés, les régimes offerts et la date de fin choisie partent', () => {
		const corps = corpsVersion(
			{
				...VALIDE,
				dateFin: '2027-12-31',
				motif: '  Décision  ',
				taux: { normal: '19', reduit: '7.5', inconnu: '3' },
				retires: ['R_TAXABLE', 'R_EXO', 'AUTRE']
			},
			ETAT
		);
		assert.deepEqual(corps, {
			date_effet: '2026-12-01',
			date_fin: '2027-12-31',
			motif: 'Décision',
			taux: { normal: '19' },
			regimes_retires: ['R_TAXABLE']
		});
	});

	it('sans date de fin ni retrait : ni date_fin ni regimes_retires', () => {
		assert.deepEqual(corpsVersion(VALIDE, ETAT), {
			date_effet: '2026-12-01',
			motif: 'Décision de l’administration',
			taux: { normal: '19' }
		});
	});

	it('un contrôle échoué : rien n’est envoyé', () => {
		assert.equal(corpsVersion({ ...VALIDE, motif: '' }, ETAT), null);
		assert.equal(corpsVersion(VALIDE, { ...ETAT, base: null }), null);
	});

	it('les écarts de taux ignorent les champs vides', () => {
		assert.deepEqual(tauxChanges({ ...VALIDE, taux: { normal: ' ', reduit: '8' } }, BASE.taux), [
			['reduit', '8']
		]);
	});
});

describe('messages des refus', () => {
	it('chaque motif de la surcharge, reçu du serveur, est traduit sans rien citer', async () => {
		fixerLangue('fr');
		for (const [motif, statut] of MOTIFS_PARAMETRES) {
			const reponse = new Response(JSON.stringify({ erreur: motif }), {
				status: statut,
				headers: { 'content-type': 'application/json' }
			});
			const erreur = await traduireReponse(reponse);
			assert.ok(erreur instanceof ErreurApi);
			assert.equal(erreur.detail?.motif, motif);
			const message = messageMetier(erreur);
			assert.ok(message !== '' && !message.startsWith('erreur.') && !message.includes('{'), motif);
			assert.ok(chercher(`saisie.motif.${motif}`) !== null, motif);
		}
	});

	it('les avertissements et notices de la création ont leur message', () => {
		fixerLangue('fr');
		assert.match(
			chercher('parametres.avertissement.ecritures_deja_validees', { nombre: 4 }) ?? '',
			/4 écriture/
		);
		for (const cle of [
			'parametres.retroactive_avant',
			'parametres.sans_fin',
			'parametres.immuable'
		]) {
			assert.ok((chercher(cle) ?? '') !== '', cle);
		}
	});
});
