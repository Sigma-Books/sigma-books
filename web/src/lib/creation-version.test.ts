import assert from 'node:assert/strict';
import { describe, it } from 'node:test';
import { ErreurApi } from './api/erreurs.ts';
import type { Applicable, CorpsVersion, VersionCreee } from './api/parametres-fiscaux.ts';
import {
	CreationVersion,
	versionPossiblementCreee,
	type DependancesCreation
} from './creation-version.ts';
import { executerSensible } from './fraicheur.ts';
import { formulaireVersionVide, type FormulaireVersion } from './parametres-fiscaux.ts';

// Données d'exemple pour les tests : valeurs inventées.
const BASE: Applicable = {
	date: '2026-12-01',
	version: {
		identifiant: 'xx-2026.1',
		portee: 'pays',
		version: '2026.1',
		date_effet: '2026-01-01'
	},
	taux: [{ role: 'normal', valeur: '20', source: 'Exemple' }],
	regimes: [],
	motifs: [],
	retention_annees: null
};
const CREEE: VersionCreee = {
	identifiant: 'xx-2026.1+d1',
	portee: 'dossier',
	version: 'xx-2026.1',
	numero: 1,
	date_effet: '2026-12-01',
	avertissements: [{ code: 'ecritures_deja_validees', nombre: 2 }]
};

/** Une promesse que le test résout à la main (pour ordonner les réponses). */
function differee<T>() {
	let resoudre!: (v: T) => void;
	let rejeter!: (e: unknown) => void;
	const promesse = new Promise<T>((res, rej) => {
		resoudre = res;
		rejeter = rej;
	});
	return { promesse, resoudre, rejeter };
}

function monter(surcharge: Partial<DependancesCreation> = {}) {
	const f: { courant: FormulaireVersion } = {
		courant: { ...formulaireVersionVide(), motif: 'Décision' }
	};
	const appels = {
		lire: [] as string[],
		creer: [] as CorpsVersion[],
		actualiser: 0,
		resets: [] as boolean[]
	};
	const deps: DependancesCreation = {
		lireBase: async (date) => {
			appels.lire.push(date);
			return { ...BASE, date };
		},
		creer: async (corps) => {
			appels.creer.push(corps);
			return CREEE;
		},
		executerSensible: (travail) =>
			executerSensible(travail, { assurer: async () => true, estFraiche: async () => true }),
		actualiser: async () => {
			appels.actualiser += 1;
		},
		formulaire: () => f.courant,
		reinitialiserSaisies: (tout) => {
			appels.resets.push(tout);
			if (tout) f.courant = formulaireVersionVide();
			else f.courant = { ...f.courant, taux: {}, retires: [] };
		},
		aujourdhui: () => '2026-11-27',
		versions: () => [],
		regional: () => ({ langue: 'fr', pays: 'SN' }),
		...surcharge
	};
	let notifications = 0;
	const ctrl = new CreationVersion(deps, () => (notifications += 1));
	return { ctrl, f, appels, notifications: () => notifications };
}

describe('lecture de la version de base', () => {
	it('une lecture périmée (autre date) n’écrit jamais l’état', async () => {
		const a = differee<Applicable>();
		const b = differee<Applicable>();
		const reponses = [a, b];
		const { ctrl, f } = monter({ lireBase: () => (reponses.shift() as typeof a).promesse });
		f.courant.dateEffet = '2026-12-01';
		const pa = ctrl.choisirDate('2026-12-01');
		f.courant.dateEffet = '2027-01-15';
		const pb = ctrl.choisirDate('2027-01-15');
		b.resoudre({ ...BASE, date: '2027-01-15' });
		await pb;
		assert.equal(ctrl.vue.base?.date, '2027-01-15');
		// La réponse de la première date arrive après : sans effet.
		a.resoudre({ ...BASE, date: '2026-12-01' });
		await pa;
		assert.equal(ctrl.vue.base?.date, '2027-01-15');
		assert.equal(ctrl.vue.lectureEnCours, false);
	});

	it('une lecture en vol d’un contrôleur détruit (autre dossier) n’écrit plus rien', async () => {
		const a = differee<Applicable>();
		const { ctrl, notifications } = monter({ lireBase: () => a.promesse });
		const p = ctrl.choisirDate('2026-12-01');
		const avant = notifications();
		ctrl.detruire();
		a.resoudre(BASE);
		await p;
		assert.equal(ctrl.vue.base, null);
		assert.equal(notifications(), avant);
	});

	it('une date illisible vide la base sans appeler l’API', async () => {
		const { ctrl, appels } = monter();
		await ctrl.choisirDate('2026-12-01');
		assert.notEqual(ctrl.vue.base, null);
		await ctrl.choisirDate('');
		assert.equal(ctrl.vue.base, null);
		assert.deepEqual(appels.lire, ['2026-12-01']);
	});

	it('une erreur de lecture est gardée ; le changement de base remet à zéro les saisies', async () => {
		const { ctrl, appels } = monter({
			lireBase: async () => {
				throw new ErreurApi('reseau', null);
			}
		});
		await ctrl.choisirDate('2026-12-01');
		assert.equal(ctrl.vue.erreurBase?.code, 'reseau');
		assert.equal(ctrl.vue.lectureEnCours, false);
		const ok = monter();
		await ok.ctrl.choisirDate('2026-12-01');
		assert.deepEqual(ok.appels.resets, [false]);
		assert.deepEqual(appels.resets, []);
	});
});

describe('vérification', () => {
	it('saisie de la date puis « Vérifier » sans attendre : la lecture est attendue', async () => {
		const a = differee<Applicable>();
		const { ctrl, f, appels } = monter({
			lireBase: async (date) => {
				appels.lire.push(date);
				return a.promesse;
			}
		});
		f.courant.dateEffet = '2026-12-01';
		void ctrl.choisirDate('2026-12-01');
		const v = ctrl.verifier();
		a.resoudre({ ...BASE });
		await v;
		// Une seule lecture, attendue : le seul problème est l'absence de taux saisi, pas une
		// « version applicable » manquante.
		assert.deepEqual(appels.lire, ['2026-12-01']);
		assert.deepEqual(
			ctrl.vue.problemes.map((p) => p.cle),
			['aucun_changement']
		);
		assert.notEqual(ctrl.vue.base, null);
	});

	it('« Vérifier » sans lecture préalable la lance', async () => {
		const { ctrl, f, appels } = monter();
		f.courant.dateEffet = '2026-12-01';
		await ctrl.verifier();
		assert.deepEqual(appels.lire, ['2026-12-01']);
		assert.notEqual(ctrl.vue.base, null);
		f.courant.taux = { normal: '19' };
		await ctrl.verifier();
		assert.equal(ctrl.vue.phase, 'confirmation');
		assert.deepEqual(appels.lire, ['2026-12-01']);
	});

	it('une date déjà lue n’est pas relue à la vérification, les taux saisis sont conservés', async () => {
		const { ctrl, f, appels } = monter();
		f.courant.dateEffet = '2026-12-01';
		await ctrl.choisirDate('2026-12-01');
		f.courant.taux = { normal: '19' };
		await ctrl.verifier();
		assert.deepEqual(appels.lire, ['2026-12-01']);
		assert.equal(ctrl.vue.phase, 'confirmation');
	});

	it('la lecture échouée s’affiche seule, sans « aucune version applicable »', async () => {
		const { ctrl, f } = monter({
			lireBase: async () => {
				throw new ErreurApi('delai_depasse', 503);
			}
		});
		f.courant.dateEffet = '2026-12-01';
		await ctrl.verifier();
		assert.equal(ctrl.vue.erreurBase?.code, 'delai_depasse');
		assert.deepEqual(ctrl.vue.problemes, []);
		assert.equal(ctrl.vue.phase, 'saisie');
	});

	it('un contrôle de forme échoué reste en saisie avec ses problèmes', async () => {
		const { ctrl, f } = monter();
		f.courant.dateEffet = '2026-12-01';
		await ctrl.choisirDate('2026-12-01');
		f.courant.taux = { normal: '19' };
		f.courant.motif = '';
		await ctrl.verifier();
		assert.equal(ctrl.vue.phase, 'saisie');
		assert.deepEqual(
			ctrl.vue.problemes.map((p) => p.cle),
			['motif_invalide']
		);
	});
});

describe('création', () => {
	async function pret(surcharge: Partial<DependancesCreation> = {}) {
		const m = monter(surcharge);
		m.f.courant.dateEffet = '2026-12-01';
		await m.ctrl.choisirDate('2026-12-01');
		m.f.courant.taux = { normal: '19' };
		await m.ctrl.verifier();
		assert.equal(m.ctrl.vue.phase, 'confirmation');
		return m;
	}

	it('crée, rend les avertissements du serveur, recharge la liste et vide le formulaire', async () => {
		const { ctrl, appels, f } = await pret();
		await ctrl.creer();
		assert.equal(ctrl.vue.phase, 'fait');
		assert.equal(ctrl.vue.creee?.identifiant, 'xx-2026.1+d1');
		assert.deepEqual(ctrl.vue.creee?.avertissements, [
			{ code: 'ecritures_deja_validees', nombre: 2 }
		]);
		assert.deepEqual(appels.creer, [
			{ date_effet: '2026-12-01', motif: 'Décision', taux: { normal: '19' } }
		]);
		assert.equal(appels.actualiser, 1);
		assert.equal(f.courant.motif, '');
		assert.equal(ctrl.vue.enCours, false);
	});

	it('une seconde demande pendant la première est ignorée (double soumission)', async () => {
		const attente = differee<VersionCreee>();
		const { ctrl, appels } = await pret({
			creer: async (corps) => {
				appels.creer.push(corps);
				return attente.promesse;
			}
		});
		const premiere = ctrl.creer();
		await ctrl.creer();
		await ctrl.creer();
		attente.resoudre(CREEE);
		await premiere;
		assert.equal(appels.creer.length, 1);
	});

	it('session non fraîche : ré-authentification, puis la requête est rejouée une seule fois', async () => {
		let essais = 0;
		let fraiche = false;
		let verifications = 0;
		const { ctrl, appels } = await pret({
			creer: async (corps) => {
				essais += 1;
				appels.creer.push(corps);
				if (!fraiche) throw new ErreurApi('interdit', 403);
				return CREEE;
			},
			executerSensible: (travail) =>
				executerSensible(travail, {
					assurer: async () => {
						// Première vérification : la session paraît fraîche ; la seconde (après un 403) a
						// ouvert la boîte de ré-authentification, qui l'a rafraîchie.
						verifications += 1;
						if (verifications > 1) fraiche = true;
						return true;
					},
					estFraiche: async () => false
				})
		});
		// La session semble fraîche au départ (assurer), puis le serveur refuse : un seul rejeu.
		await ctrl.creer();
		assert.equal(essais, 2);
		assert.equal(verifications, 2);
		assert.equal(ctrl.vue.phase, 'fait');
	});

	it('un second 403 après le rejeu est affiché, sans troisième essai', async () => {
		let essais = 0;
		const { ctrl } = await pret({
			creer: async () => {
				essais += 1;
				throw new ErreurApi('interdit', 403);
			},
			executerSensible: (travail) =>
				executerSensible(travail, { assurer: async () => true, estFraiche: async () => false })
		});
		await ctrl.creer();
		assert.equal(essais, 2);
		assert.equal(ctrl.vue.erreur?.statut, 403);
		assert.equal(ctrl.vue.phase, 'saisie');
	});

	it('ré-authentification annulée : rien n’est envoyé, la saisie est conservée', async () => {
		const { ctrl, appels, f } = await pret({
			executerSensible: async () => ({ fait: false })
		});
		await ctrl.creer();
		assert.equal(ctrl.vue.reauthAnnulee, true);
		assert.equal(ctrl.vue.phase, 'saisie');
		assert.deepEqual(appels.creer, []);
		assert.equal(f.courant.motif, 'Décision');
	});

	it('un refus du serveur est affiché et la saisie conservée', async () => {
		const { ctrl, f } = await pret({
			creer: async () => {
				throw new ErreurApi('conflit', 409, null, null, {
					motif: 'parametres_date_effet_anterieure',
					nombre: null,
					ligne: null,
					refus: []
				});
			}
		});
		await ctrl.creer();
		assert.equal(ctrl.vue.erreur?.detail?.motif, 'parametres_date_effet_anterieure');
		assert.equal(ctrl.vue.phase, 'saisie');
		assert.equal(f.courant.motif, 'Décision');
		assert.equal(ctrl.vue.enCours, false);
	});

	it('201 avec une réponse illisible : « peut-être créée », liste rechargée, pas de nouvel essai', async () => {
		const { ctrl, appels } = await pret({
			creer: async () => {
				throw new ErreurApi('reponse_invalide', 201);
			}
		});
		await ctrl.creer();
		assert.equal(ctrl.vue.incertaine, true);
		assert.equal(ctrl.vue.phase, 'fait');
		assert.equal(ctrl.vue.erreur, null);
		assert.equal(appels.actualiser, 1);
		assert.equal(versionPossiblementCreee(new ErreurApi('reponse_invalide', 200)), false);
		assert.equal(versionPossiblementCreee(new ErreurApi('reseau', null)), false);
	});

	it('l’échec du rechargement de la liste ne fait pas échouer une création réussie', async () => {
		const { ctrl } = await pret({
			actualiser: async () => {
				throw new TypeError('Failed to fetch');
			}
		});
		await ctrl.creer();
		assert.equal(ctrl.vue.phase, 'fait');
		assert.equal(ctrl.vue.erreur, null);
	});

	it('un contrôleur détruit pendant la création n’écrit plus rien (autre dossier)', async () => {
		const attente = differee<VersionCreee>();
		const { ctrl, notifications } = await pret({ creer: () => attente.promesse });
		const p = ctrl.creer();
		ctrl.detruire();
		const avant = notifications();
		attente.resoudre(CREEE);
		await p;
		assert.equal(notifications(), avant);
		assert.equal(ctrl.vue.creee, null);
	});

	it('Modifier revient à la saisie ; Recommencer efface le résultat', async () => {
		const { ctrl } = await pret();
		ctrl.modifier();
		assert.equal(ctrl.vue.phase, 'saisie');
		const m = await pret();
		await m.ctrl.creer();
		m.ctrl.recommencer();
		assert.equal(m.ctrl.vue.phase, 'saisie');
		assert.equal(m.ctrl.vue.creee, null);
	});
});
