import assert from 'node:assert/strict';
import { describe, it } from 'node:test';
import { messageMetier } from './api/messages.ts';
import { ErreurApi, traduireReponse } from './api/erreurs.ts';
import { MOTIFS_CONTEXTE, type PackPays } from './api/pays.ts';
import type { Dossier, Uuid } from './api/types.ts';
import {
	BORNES_PROBLEMES,
	choisirPays,
	CLES_PROBLEME,
	corpsCreation,
	diagnostiquer,
	diagnostiquerNom,
	diagnostiquerOuverture,
	formulaireVide,
	creationPossiblementFaite,
	etatIdentifiants,
	formulaireDuDossier,
	preparerModification,
	messageProbleme,
	ouvertureSuivantFuseau,
	libelleDecalage,
	libelleLangue,
	ouvertureParDefaut,
	type FormulaireContexte
} from './contexte-saisie.ts';
import { dateDuJour } from './dates.ts';
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

const AUJOURDHUI = '2026-11-26';
const REPRISE = ouvertureParDefaut(AUJOURDHUI);

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

	it('un aller-retour entre deux pays ne perd aucune valeur d’identifiant', () => {
		const saisi = { ...formulaireSn(), identifiants: { NINEA: '123', RCCM: 'R1', AUTRE: '' } };
		const cd = choisirPays(saisi, PACK, 'CD');
		// Masquée et non envoyée pour CD, mais gardée dans le formulaire…
		assert.equal(cd.identifiants['NINEA'], '123');
		assert.deepEqual(etatIdentifiants(cd, PACK, null).effectifs, [{ type: 'RCCM', valeur: 'R1' }]);
		assert.deepEqual(etatIdentifiants(cd, PACK, null).retires, ['NINEA']);
		// … et retrouvée au retour sur le pays d'origine.
		const sn = choisirPays(cd, PACK, 'SN');
		assert.deepEqual(etatIdentifiants(sn, PACK, null).effectifs, [
			{ type: 'NINEA', valeur: '123' },
			{ type: 'RCCM', valeur: 'R1' }
		]);
		assert.deepEqual(etatIdentifiants(sn, PACK, null).retires, []);
	});

	it('la langue suit le pays, sauf si elle a été choisie à la main', () => {
		const pack: PackPays = {
			...PACK,
			pays: [
				...PACK.pays,
				{
					code: 'GW',
					nom: 'Guinée-Bissau',
					devise: 'XOF',
					fuseau: 'Africa/Dakar',
					langue: 'pt',
					identifiants: []
				}
			]
		};
		const f = choisirPays(choisirPays(formulaireVide(), pack, 'SN'), pack, 'GW');
		assert.equal(f.langue, 'pt');
		const choisie = { ...choisirPays(formulaireVide(), pack, 'SN'), langue: 'en' };
		assert.equal(choisirPays(choisie, pack, 'GW').langue, 'en');
		assert.equal(choisirPays(f, pack, 'SN').langue, 'fr');
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

	it('identifiant : 64 caractères au plus, sans caractère de contrôle', () => {
		const sn = formulaireSn();
		const long = { ...sn, identifiants: { ...sn.identifiants, NINEA: 'x'.repeat(65) } };
		assert.deepEqual(diagnostiquer(long, PACK), [
			{ champ: 'identifiant:NINEA', cle: 'valeur_trop_longue' }
		]);
		const ctrl = { ...sn, identifiants: { ...sn.identifiants, RCCM: 'a\tb' } };
		assert.deepEqual(diagnostiquer(ctrl, PACK), [
			{ champ: 'identifiant:RCCM', cle: 'caractere_de_controle' }
		]);
	});

	it('un type que le pays n’admet pas n’est jamais envoyé ni affiché (création)', () => {
		const sn = formulaireSn();
		const f = { ...sn, identifiants: { ...sn.identifiants, IFU: '1' } };
		const etat = etatIdentifiants(f, PACK, null);
		assert.deepEqual(etat.effectifs, []);
		assert.deepEqual(etat.retires, ['IFU']);
		assert.deepEqual(
			etat.affiches.map((t) => t.code),
			['NINEA', 'RCCM', 'AUTRE']
		);
		assert.deepEqual(diagnostiquer(f, PACK), []);
	});

	it('une clé hostile du formulaire ne devient pas un identifiant', () => {
		const sn = formulaireSn();
		const hostile = Object.fromEntries([...Object.entries(sn.identifiants), ['__proto__', 'x']]);
		assert.deepEqual(etatIdentifiants({ ...sn, identifiants: hostile }, PACK, null).effectifs, []);
	});

	it('modification : une valeur inscrite que le pack ne propose plus ne bloque que si on la change', () => {
		const stocke = { ...formulaireSn(), fuseau: 'Africa/Disparu', devise: 'USD' };
		// Rien n'a changé : aucun contrôle de ces champs (la modification d'un autre champ passe).
		assert.deepEqual(
			diagnostiquer({ ...stocke, langue: 'fr', formeJuridique: 'SA' }, PACK, stocke),
			[]
		);
		// Changé vers une valeur inconnue : refus.
		assert.deepEqual(
			diagnostiquer({ ...stocke, fuseau: 'Mars/Olympus' }, PACK, stocke).map((p) => p.cle),
			['fuseau_inconnu']
		);
	});

	it('modification : un type inscrit non admis par le pack est conservé tant que le pays reste', () => {
		const sn = formulaireSn();
		const f = { ...sn, identifiants: { ...sn.identifiants, IFU: '42' } };
		const etat = etatIdentifiants(f, PACK, 'SN');
		assert.deepEqual(etat.effectifs, [{ type: 'IFU', valeur: '42' }]);
		assert.deepEqual(etat.retires, []);
		assert.deepEqual(diagnostiquer(f, PACK, f), []);
		// Le pays change : le type n'est plus envoyé, et c'est signalé.
		const cd = choisirPays(f, PACK, 'CD');
		assert.deepEqual(etatIdentifiants(cd, PACK, 'SN').retires, ['IFU']);
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
		assert.deepEqual(etatIdentifiants(f, PACK, null).effectifs, [{ type: 'NINEA', valeur: '123' }]);
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
		const diag = (o: typeof REPRISE) => diagnostiquerOuverture(o, AUJOURDHUI);
		assert.deepEqual(diag(REPRISE), []);
		assert.equal(diag({ ...REPRISE, annee: '26' })[0]?.cle, 'annee_invalide');
		assert.equal(diag({ ...REPRISE, annee: '2026 ' })[0]?.cle, 'annee_invalide');
		const creation = { ...REPRISE, nature: 'creation' as const };
		assert.deepEqual(diag(creation), []);
		assert.equal(diag({ ...creation, debutActivite: '2026-02-30' })[0]?.cle, 'debut_invalide');
	});

	it('reprise : l’année en cours ou la précédente seulement, avec message précis', () => {
		const diag = (annee: string) => diagnostiquerOuverture({ ...REPRISE, annee }, AUJOURDHUI);
		assert.deepEqual(diag('2025'), []);
		assert.deepEqual(diag('2026'), []);
		for (const hors of ['2024', '2027']) {
			const [p] = diag(hors);
			assert.equal(p?.cle, 'annee_hors_bornes', hors);
			assert.match(messageProbleme(p as NonNullable<typeof p>), /2025 ou 2026/);
		}
	});

	it('entité nouvelle : début de l’année précédente à l’année suivante', () => {
		const diag = (
			debutActivite: string,
			finPremier: 'annee_de_debut' | 'annee_suivante' = 'annee_de_debut'
		) =>
			diagnostiquerOuverture(
				{ ...REPRISE, nature: 'creation', debutActivite, finPremier },
				AUJOURDHUI
			);
		assert.deepEqual(diag('2025-01-01'), []);
		assert.deepEqual(diag('2027-12-31'), []);
		assert.equal(diag('2024-12-31')[0]?.cle, 'debut_hors_bornes');
		assert.equal(diag('2028-01-01')[0]?.cle, 'debut_hors_bornes');
		assert.match(messageProbleme(diag('2028-01-01')[0] as never), /2025 et 2027/);
	});

	it('fin au 31 décembre de l’année suivante : seulement si le début est au second semestre', () => {
		const diag = (debutActivite: string) =>
			diagnostiquerOuverture(
				{ ...REPRISE, nature: 'creation', debutActivite, finPremier: 'annee_suivante' },
				AUJOURDHUI
			);
		assert.equal(diag('2026-06-30')[0]?.cle, 'prolongation_impossible');
		assert.deepEqual(diag('2026-07-01'), []);
	});

	it('l’année du jour se calcule dans le fuseau choisi (autour du 1er janvier)', () => {
		// À 23 h 30 UTC le 31 décembre, il est déjà 2027 à Lagos (UTC+1) mais pas à Dakar (UTC).
		const instant = new Date('2026-12-31T23:30:00Z');
		assert.equal(dateDuJour('Africa/Dakar', instant), '2026-12-31');
		assert.equal(dateDuJour('Africa/Lagos', instant), '2027-01-01');
		const reprise2027 = { ...REPRISE, annee: '2027' };
		assert.equal(
			diagnostiquerOuverture(reprise2027, dateDuJour('Africa/Dakar', instant))[0]?.cle,
			'annee_hors_bornes'
		);
		assert.deepEqual(diagnostiquerOuverture(reprise2027, dateDuJour('Africa/Lagos', instant)), []);
	});

	it('la date et l’année proposées suivent le fuseau, pas une valeur saisie', () => {
		const proposee = ouvertureParDefaut('2026-12-31');
		const suivie = ouvertureSuivantFuseau(proposee, '2026-12-31', '2027-01-01');
		assert.deepEqual([suivie.debutActivite, suivie.annee], ['2027-01-01', '2027']);
		const saisie = { ...proposee, annee: '2025', debutActivite: '2026-03-01' };
		const gardee = ouvertureSuivantFuseau(saisie, '2026-12-31', '2027-01-01');
		assert.deepEqual([gardee.debutActivite, gardee.annee], ['2026-03-01', '2025']);
	});

	it('201 avec une réponse illisible : la création a peut-être abouti', () => {
		assert.equal(creationPossiblementFaite(new ErreurApi('reponse_invalide', 201)), true);
		assert.equal(creationPossiblementFaite(new ErreurApi('reponse_invalide', 200)), false);
		assert.equal(creationPossiblementFaite(new ErreurApi('reseau', null)), false);
		assert.equal(creationPossiblementFaite(new ErreurApi('refus', 422)), false);
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
		assert.deepEqual(corpsCreation(' SARL Exemple ', f, REPRISE, PACK, AUJOURDHUI), {
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
			PACK,
			AUJOURDHUI
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
		assert.equal(corpsCreation('', formulaireSn(), REPRISE, PACK, AUJOURDHUI), null);
		assert.equal(corpsCreation('X', formulaireVide(), REPRISE, PACK, AUJOURDHUI), null);
		assert.equal(
			corpsCreation('X', formulaireSn(), { ...REPRISE, annee: 'abcd' }, PACK, AUJOURDHUI),
			null
		);
	});
});

describe('corps de la modification', () => {
	const dossier: Dossier = {
		id: '0b0f6c43-7c47-4a43-9d1e-2f4f6f0a1b2c' as Uuid,
		nom: 'SARL Exemple',
		pays: 'SN',
		devise: 'XOF',
		decimales: 0,
		fuseau: 'Africa/Dakar',
		langue: 'fr',
		forme_juridique: 'SARL',
		identifiants_legaux: [{ type: 'NINEA', valeur: '123' }]
	};
	const initial = formulaireDuDossier(dossier, PACK);

	it('le formulaire d’un dossier reprend son contexte et ses identifiants', () => {
		assert.equal(initial.formeJuridique, 'SARL');
		assert.deepEqual(initial.identifiants, { NINEA: '123', RCCM: '', AUTRE: '' });
	});

	it('sans changement : rien à envoyer', () => {
		assert.deepEqual(preparerModification(initial, { ...initial }, PACK), { etat: 'inchange' });
		// Une valeur seulement entourée d'espaces ne change rien.
		assert.deepEqual(
			preparerModification(initial, { ...initial, formeJuridique: ' SARL ' }, PACK),
			{ etat: 'inchange' }
		);
	});

	it('fuseau, langue, forme juridique : changement non sensible, champs changés seulement', () => {
		const m = preparerModification(
			initial,
			{ ...initial, fuseau: 'Africa/Kinshasa', langue: 'pt', formeJuridique: 'SA' },
			PACK
		);
		assert.deepEqual(m, {
			etat: 'a_envoyer',
			sensible: false,
			corps: { fuseau: 'Africa/Kinshasa', langue: 'pt', forme_juridique: 'SA' }
		});
	});

	it('forme juridique vidée : envoyée null pour l’effacer', () => {
		const m = preparerModification(initial, { ...initial, formeJuridique: '' }, PACK);
		assert.deepEqual(m, { etat: 'a_envoyer', sensible: false, corps: { forme_juridique: null } });
	});

	it('pays ou devise changés : changement sensible (session fraîche)', () => {
		const devise = preparerModification(initial, { ...initial, devise: 'EUR' }, PACK);
		assert.deepEqual(devise, { etat: 'a_envoyer', sensible: true, corps: { devise: 'EUR' } });
		const pays = preparerModification(initial, choisirPays(initial, PACK, 'CD'), PACK);
		assert.equal(pays.etat, 'a_envoyer');
		assert.equal(pays.etat === 'a_envoyer' && pays.sensible, true);
		assert.deepEqual(pays.etat === 'a_envoyer' && pays.corps, {
			pays: 'CD',
			devise: 'CDF',
			fuseau: 'Africa/Kinshasa',
			// Le NINEA n'existe pas pour ce pays : la liste complète est remplacée, sans lui.
			identifiants_legaux: []
		});
	});

	it('identifiants : la liste complète remplace l’existante', () => {
		const m = preparerModification(
			initial,
			{ ...initial, identifiants: { NINEA: '123', RCCM: 'R1', AUTRE: '' } },
			PACK
		);
		assert.deepEqual(m, {
			etat: 'a_envoyer',
			sensible: false,
			corps: {
				identifiants_legaux: [
					{ type: 'NINEA', valeur: '123' },
					{ type: 'RCCM', valeur: 'R1' }
				]
			}
		});
	});

	it('une valeur invalide n’est pas envoyée', () => {
		const m = preparerModification(initial, { ...initial, formeJuridique: 'x'.repeat(101) }, PACK);
		assert.deepEqual(m, {
			etat: 'invalide',
			problemes: [{ champ: 'forme_juridique', cle: 'forme_trop_longue' }]
		});
	});

	it('aller-retour SN → CD → SN : rien n’est perdu ni envoyé', () => {
		const aller = choisirPays(initial, PACK, 'CD');
		// Au retour, le NINEA est retrouvé : aucun changement.
		const retour = choisirPays(aller, PACK, 'SN');
		assert.deepEqual(preparerModification(initial, retour, PACK), { etat: 'inchange' });
		// Pendant le séjour en CD, le NINEA est signalé comme non enregistré.
		assert.deepEqual(etatIdentifiants(aller, PACK, 'SN').retires, ['NINEA']);
	});

	it('identifiants non modifiés : jamais envoyés avec un changement de fuseau', () => {
		const m = preparerModification(initial, { ...initial, fuseau: 'Africa/Kinshasa' }, PACK);
		assert.deepEqual(m, {
			etat: 'a_envoyer',
			sensible: false,
			corps: { fuseau: 'Africa/Kinshasa' }
		});
	});

	it('un type inscrit que le pack ne propose plus ne bloque pas la modification du fuseau', () => {
		const ancien: Dossier = {
			...dossier,
			fuseau: 'Africa/Disparu',
			identifiants_legaux: [{ type: 'IFU', valeur: '42' }]
		};
		const base = formulaireDuDossier(ancien, PACK);
		assert.equal(base.identifiants['IFU'], '42');
		const m = preparerModification(base, { ...base, langue: 'pt' }, PACK);
		assert.deepEqual(m, { etat: 'a_envoyer', sensible: false, corps: { langue: 'pt' } });
		// Le type est conservé tant que le pays ne change pas ; changer sa valeur l'envoie.
		const change = preparerModification(
			base,
			{ ...base, identifiants: { ...base.identifiants, IFU: '43' } },
			PACK
		);
		assert.deepEqual(change.etat === 'a_envoyer' && change.corps.identifiants_legaux, [
			{ type: 'IFU', valeur: '43' }
		]);
	});

	it('un type inscrit sous un nom hostile reste une simple clé', () => {
		const hostile: Dossier = {
			...dossier,
			identifiants_legaux: [{ type: '__proto__', valeur: 'x' }]
		};
		const f = formulaireDuDossier(hostile, PACK);
		assert.deepEqual(Object.keys(f.identifiants).includes('__proto__'), true);
		assert.deepEqual(preparerModification(f, { ...f }, PACK), { etat: 'inchange' });
	});
});

describe('messages', () => {
	it('chaque contrôle de forme a son message, qui cite sa borne ou ses années', () => {
		fixerLangue('fr');
		for (const cle of CLES_PROBLEME) {
			const borne = BORNES_PROBLEMES[cle];
			const params = borne === undefined ? { min: 2025, max: 2027 } : undefined;
			const message = messageProbleme({ champ: 'x', cle, params });
			assert.notEqual(message, '', cle);
			assert.ok(!message.includes('{'), `${cle} : paramètre non remplacé`);
			if (borne !== undefined) assert.ok(message.includes(String(borne)), cle);
		}
	});

	it('chaque motif de refus du contexte, reçu du serveur, est traduit sans rien citer', async () => {
		fixerLangue('fr');
		for (const [motif, statut] of MOTIFS_CONTEXTE) {
			const reponse = new Response(JSON.stringify({ erreur: motif }), {
				status: statut,
				headers: { 'content-type': 'application/json' }
			});
			const erreur = await traduireReponse(reponse);
			assert.equal(erreur.detail?.motif, motif);
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
