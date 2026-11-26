import assert from 'node:assert/strict';
import { describe, it } from 'node:test';
import { messageMetier } from './api/messages.ts';
import { ErreurApi } from './api/erreurs.ts';
import { MOTIFS_CONTEXTE, type PackPays } from './api/pays.ts';
import {
	BORNES_PROBLEMES,
	choisirPays,
	CLES_PROBLEME,
	corpsCreation,
	diagnostiquer,
	diagnostiquerNom,
	diagnostiquerOuverture,
	formulaireVide,
	identifiantsSaisis,
	libelleDecalage,
	libelleLangue,
	ouvertureParDefaut,
	type FormulaireContexte
} from './contexte-saisie.ts';
import { chercher, fixerLangue } from './i18n/index.ts';

/** Un pack d'exemple : deux pays, trois devises, trois fuseaux. Les valeurs ne servent qu'aux tests. */
const PACK: PackPays = {
	pays: [
		{
			code: 'SN',
			nom: 'Sénégal',
			devise: 'XOF',
			fuseau: 'Africa/Dakar',
			langue: 'fr',
			identifiants: [
				{ code: 'NINEA', libelle: 'Numéro national' },
				{ code: 'RCCM', libelle: 'Registre du commerce' },
				{ code: 'AUTRE', libelle: 'Autre' }
			]
		},
		{
			code: 'CD',
			nom: 'RD Congo',
			devise: 'CDF',
			fuseau: 'Africa/Kinshasa',
			langue: 'fr',
			identifiants: [
				{ code: 'RCCM', libelle: 'Registre du commerce' },
				{ code: 'AUTRE', libelle: 'Autre' }
			]
		}
	],
	devises: [
		{ code: 'XOF', intitule: 'Franc CFA', decimales: 0 },
		{ code: 'CDF', intitule: 'Franc congolais', decimales: 2 },
		{ code: 'EUR', intitule: 'Euro', decimales: 2 }
	],
	fuseaux: [
		{ nom: 'Africa/Dakar', decalage_minutes: 0 },
		{ nom: 'Africa/Kinshasa', decalage_minutes: 60 },
		{ nom: 'Africa/Lubumbashi', decalage_minutes: 120 }
	]
};

const REPRISE = ouvertureParDefaut('2026-11-26');

function formulaireSn(): FormulaireContexte {
	return choisirPays(formulaireVide(), PACK, 'SN');
}

describe('choix du pays : valeurs par défaut du pack', () => {
	it('le pays fixe devise, fuseau et langue par défaut, et les types d’identifiants', () => {
		const f = formulaireSn();
		assert.deepEqual([f.pays, f.devise, f.fuseau, f.langue], ['SN', 'XOF', 'Africa/Dakar', 'fr']);
		assert.deepEqual(Object.keys(f.identifiants), ['NINEA', 'RCCM', 'AUTRE']);
	});

	it('un changement de pays suit les défauts du nouveau pays', () => {
		const f = choisirPays(formulaireSn(), PACK, 'CD');
		assert.deepEqual([f.devise, f.fuseau], ['CDF', 'Africa/Kinshasa']);
	});

	it('une devise choisie à la main est conservée quand le pays change', () => {
		const choisie = { ...formulaireSn(), devise: 'EUR', fuseau: 'Africa/Lubumbashi' };
		const f = choisirPays(choisie, PACK, 'CD');
		assert.deepEqual([f.devise, f.fuseau], ['EUR', 'Africa/Lubumbashi']);
	});

	it('les valeurs saisies des types encore admis sont gardées, les autres disparaissent', () => {
		const saisi = { ...formulaireSn(), identifiants: { NINEA: '123', RCCM: 'R1', AUTRE: '' } };
		const f = choisirPays(saisi, PACK, 'CD');
		assert.deepEqual(f.identifiants, { RCCM: 'R1', AUTRE: '' });
	});

	it('un pays hors du pack vide les réglages et sera refusé', () => {
		const f = choisirPays(formulaireSn(), PACK, 'ZZ');
		assert.deepEqual([f.pays, f.devise, f.fuseau, f.langue], ['ZZ', '', '', '']);
		assert.ok(diagnostiquer(f, PACK).some((p) => p.cle === 'pays_inconnu'));
	});
});

describe('contrôles de forme du contexte', () => {
	it('un formulaire vide demande le pays', () => {
		const cles = diagnostiquer(formulaireVide(), PACK).map((p) => p.cle);
		assert.ok(cles.includes('pays_absent'));
	});

	it('un contexte complet est sans problème', () => {
		assert.deepEqual(diagnostiquer(formulaireSn(), PACK), []);
	});

	it('devise ou fuseau absents des listes du pack, langue mal formée : refus', () => {
		const f = { ...formulaireSn(), devise: 'USD', fuseau: 'Mars/Olympus', langue: 'FR' };
		const champs = diagnostiquer(f, PACK).map((p) => p.champ);
		assert.deepEqual(champs, ['devise', 'fuseau', 'langue']);
	});

	it('forme juridique : 100 caractères au plus (en caractères, pas en octets), sans contrôle', () => {
		const ok = { ...formulaireSn(), formeJuridique: 'é'.repeat(100) };
		assert.deepEqual(diagnostiquer(ok, PACK), []);
		const long = { ...formulaireSn(), formeJuridique: 'é'.repeat(101) };
		assert.equal(diagnostiquer(long, PACK)[0]?.cle, 'forme_trop_longue');
		for (const hostile of ['SARL\u0000', 'SARL\nX', 'SARL\u007f', '\u0085']) {
			const f = { ...formulaireSn(), formeJuridique: hostile };
			assert.equal(
				diagnostiquer(f, PACK)[0]?.cle,
				'caractere_de_controle',
				JSON.stringify(hostile)
			);
		}
	});

	it('identifiant : 64 caractères au plus, sans contrôle, type du pays seulement', () => {
		const sn = formulaireSn();
		const long = { ...sn, identifiants: { ...sn.identifiants, NINEA: 'x'.repeat(65) } };
		assert.deepEqual(diagnostiquer(long, PACK), [
			{ champ: 'identifiant:NINEA', cle: 'valeur_trop_longue' }
		]);
		const ctrl = { ...sn, identifiants: { ...sn.identifiants, RCCM: 'a\tb' } };
		assert.deepEqual(diagnostiquer(ctrl, PACK), [
			{ champ: 'identifiant:RCCM', cle: 'caractere_de_controle' }
		]);
		const autre = { ...sn, identifiants: { ...sn.identifiants, IFU: '1' } };
		assert.deepEqual(diagnostiquer(autre, PACK), [
			{ champ: 'identifiant:IFU', cle: 'type_non_admis' }
		]);
		const pasLe = { ...sn, identifiants: { ...sn.identifiants, __proto__: 'x' } };
		assert.ok(Array.isArray(diagnostiquer(pasLe, PACK)));
	});

	it('au plus dix identifiants', () => {
		const types = Array.from({ length: 11 }, (_, i) => ({ code: `T${i}`, libelle: `Type ${i}` }));
		const pack: PackPays = {
			...PACK,
			pays: [{ ...(PACK.pays[0] as PackPays['pays'][number]), identifiants: types }]
		};
		const f = choisirPays(formulaireVide(), pack, 'SN');
		for (const t of types) f.identifiants[t.code] = 'v';
		assert.ok(diagnostiquer(f, pack).some((p) => p.cle === 'trop_d_identifiants'));
		f.identifiants['T10'] = '';
		assert.deepEqual(diagnostiquer(f, pack), []);
	});

	it('les valeurs sont rognées ; vide ou blanc : aucun identifiant', () => {
		const sn = formulaireSn();
		const f = { ...sn, identifiants: { NINEA: '  123  ', RCCM: '   ', AUTRE: '' } };
		assert.deepEqual(identifiantsSaisis(f, PACK), [{ type: 'NINEA', valeur: '123' }]);
	});

	it('nom du dossier : obligatoire, 200 caractères au plus', () => {
		assert.deepEqual(diagnostiquerNom('SARL Exemple'), []);
		assert.equal(diagnostiquerNom('   ')[0]?.cle, 'nom_absent');
		assert.equal(diagnostiquerNom('x'.repeat(201))[0]?.cle, 'nom_trop_long');
		assert.equal(diagnostiquerNom('a\u0000b')[0]?.cle, 'caractere_de_controle');
	});

	it('ouverture : reprise par défaut sur l’année du jour ; formes de date et d’année', () => {
		assert.deepEqual(REPRISE, {
			nature: 'reprise',
			debutActivite: '2026-11-26',
			finPremier: 'annee_de_debut',
			annee: '2026'
		});
		assert.deepEqual(diagnostiquerOuverture(REPRISE), []);
		assert.equal(diagnostiquerOuverture({ ...REPRISE, annee: '26' })[0]?.cle, 'annee_invalide');
		assert.equal(diagnostiquerOuverture({ ...REPRISE, annee: '2026 ' })[0]?.cle, 'annee_invalide');
		const creation = { ...REPRISE, nature: 'creation' as const };
		assert.deepEqual(diagnostiquerOuverture(creation), []);
		assert.equal(
			diagnostiquerOuverture({ ...creation, debutActivite: '2026-02-30' })[0]?.cle,
			'debut_invalide'
		);
	});
});

describe('corps de la création', () => {
	it('envoie le pays, les réglages choisis, la reprise et les identifiants saisis', () => {
		const sn = formulaireSn();
		const f = {
			...sn,
			formeJuridique: ' SARL ',
			identifiants: { ...sn.identifiants, NINEA: '123' }
		};
		assert.deepEqual(corpsCreation(' SARL Exemple ', f, REPRISE, PACK), {
			nom: 'SARL Exemple',
			pays: 'SN',
			devise: 'XOF',
			fuseau: 'Africa/Dakar',
			langue: 'fr',
			forme_juridique: 'SARL',
			identifiants_legaux: [{ type: 'NINEA', valeur: '123' }],
			nature: 'reprise',
			annee: 2026
		});
	});

	it('entité nouvelle : date de début et fin du premier exercice', () => {
		const corps = corpsCreation(
			'X',
			formulaireSn(),
			{ ...REPRISE, nature: 'creation', debutActivite: '2026-09-01', finPremier: 'annee_suivante' },
			PACK
		);
		assert.deepEqual(corps && 'debut_activite' in corps && corps, {
			nom: 'X',
			pays: 'SN',
			devise: 'XOF',
			fuseau: 'Africa/Dakar',
			langue: 'fr',
			nature: 'creation',
			debut_activite: '2026-09-01',
			fin_premier_exercice: 'annee_suivante'
		});
	});

	it('rien n’est envoyé si un contrôle de forme échoue', () => {
		assert.equal(corpsCreation('', formulaireSn(), REPRISE, PACK), null);
		assert.equal(corpsCreation('X', formulaireVide(), REPRISE, PACK), null);
		assert.equal(corpsCreation('X', formulaireSn(), { ...REPRISE, annee: 'abcd' }, PACK), null);
	});
});

describe('messages', () => {
	it('chaque contrôle de forme a son message, qui cite sa borne', () => {
		fixerLangue('fr');
		for (const cle of CLES_PROBLEME) {
			const message = chercher(`contexte.probleme.${cle}`, { max: BORNES_PROBLEMES[cle] ?? 0 });
			assert.ok(message !== null && message !== '', cle);
			if (BORNES_PROBLEMES[cle] !== undefined) {
				assert.ok(message.includes(String(BORNES_PROBLEMES[cle])), cle);
			}
		}
	});

	it('chaque motif de refus du contexte est traduit, aucune donnée n’est citée', () => {
		fixerLangue('fr');
		for (const motif of MOTIFS_CONTEXTE) {
			const detail = { motif, nombre: null, ligne: null, refus: [] };
			const statut = motif.endsWith('_inconnu') || motif.endsWith('_invalide') ? 422 : 409;
			const erreur = new ErreurApi(
				statut === 409 ? 'conflit' : 'refus',
				statut,
				null,
				null,
				detail
			);
			const message = messageMetier(erreur);
			assert.notEqual(message, '', motif);
			assert.ok(!message.startsWith('erreur.') && !message.includes('{'), motif);
			assert.ok(chercher(`saisie.motif.${motif}`) !== null, motif);
		}
	});

	it('les trois conflits de la modification expliquent quoi faire', () => {
		fixerLangue('fr');
		const dit = (motif: string) => chercher(`saisie.motif.${motif}`) ?? '';
		assert.match(dit('dossier_deja_ecritures'), /écriture validée/);
		assert.match(dit('dossier_brouillard_non_vide'), /brouillons/);
		assert.match(dit('brouillon_devise_tenue_perimee'), /devise de tenue/);
	});

	it('une langue s’affiche dans la langue demandée, ou par son code si Intl l’ignore', () => {
		assert.equal(libelleLangue('fr', 'fr'), 'Français');
		assert.equal(libelleLangue('pt', 'fr'), 'Portugais');
		assert.equal(libelleLangue('zz', 'fr'), 'zz');
	});
});

describe('libellés', () => {
	it('le décalage d’un fuseau s’écrit UTC±hh:mm', () => {
		assert.equal(libelleDecalage(0), 'UTC');
		assert.equal(libelleDecalage(60), 'UTC+01:00');
		assert.equal(libelleDecalage(-90), 'UTC-01:30');
		assert.equal(libelleDecalage(330), 'UTC+05:30');
	});
});
