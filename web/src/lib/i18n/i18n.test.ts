import assert from 'node:assert/strict';
import { afterEach, test } from 'node:test';
import { MOTIFS_SERVEUR } from '../api/erreurs.ts';
import { LANGUES } from '../regional.ts';
import { en } from './en.ts';
import { fr } from './fr.ts';
import {
	CATALOGUES,
	chercher,
	fixerLangue,
	langueCourante,
	nombreEtUnite,
	nouvelleNavigation,
	traduire
} from './index.ts';

afterEach(() => fixerLangue('fr'));

test('le français est la langue par défaut', () => {
	assert.equal(langueCourante(), 'fr');
	assert.equal(traduire('role.administrateur'), 'Administrateur');
});

test('les paramètres {nom} sont remplacés, un paramètre absent est laissé tel quel', () => {
	assert.equal(
		traduire('contre_passation.libelle_defaut', { numero: 'OD-2026-000001' }),
		'Contre-passation de OD-2026-000001'
	);
	assert.equal(traduire('contre_passation.libelle_defaut', {}), 'Contre-passation de {numero}');
});

test('une langue qui n’a pas traduit un message retombe sur le français', () => {
	fixerLangue('en');
	assert.equal(traduire('taxe.sigle'), 'VAT');
	assert.equal(traduire('contre_passation.libelle_defaut', { numero: 'X' }), 'Reversal of X');
	assert.equal(traduire('role.valideur'), 'Valideur');
	assert.equal(traduire('role.valideur', undefined, 'pt'), 'Valideur');
	assert.equal(traduire('taxe.sigle', undefined, 'es'), 'IVA');
});

test('chercher rend null pour une clé inconnue et ignore le prototype', () => {
	assert.equal(chercher('role.inconnu'), null);
	assert.equal(chercher('constructor'), null);
	assert.equal(chercher('toString'), null);
	assert.equal(chercher('role.auditeur'), 'Auditeur');
});

test('chaque langue ne traduit que des clés du français, avec les mêmes paramètres', () => {
	const parametres = (m: string) => [...m.matchAll(/\{(\w+)\}/g)].map((x) => x[1]).sort();
	for (const langue of LANGUES) {
		for (const [cle, texte] of Object.entries(CATALOGUES[langue])) {
			assert.ok(Object.hasOwn(fr, cle), `${langue} : clé inconnue ${cle}`);
			assert.deepEqual(
				parametres(texte),
				parametres((fr as Record<string, string>)[cle] ?? ''),
				`${langue} : paramètres de ${cle}`
			);
		}
	}
	assert.ok(Object.keys(en).length > 0);
});

test('chaque motif et chaque code du serveur a son message', () => {
	for (const motif of MOTIFS_SERVEUR) assert.ok(Object.hasOwn(fr, `motif.${motif}`), motif);
});

test('le pluriel suit la langue', () => {
	assert.equal(nombreEtUnite(1, 'jour', 'fr'), '1 jour');
	assert.equal(nombreEtUnite(0, 'jour', 'fr'), '0 jour');
	assert.equal(nombreEtUnite(3, 'jour', 'fr'), '3 jours');
	assert.equal(nombreEtUnite(2, 'brouillon', 'fr'), '2 brouillons');
});

test('une langue périmée ne remplace pas celle de la navigation en cours (course A puis B)', () => {
	const a = nouvelleNavigation(); // chargement du dossier A (en)
	const b = nouvelleNavigation(); // chargement du dossier B (pt), commencé ensuite
	assert.equal(fixerLangue('pt', b), true); // B se termine d'abord
	assert.equal(fixerLangue('en', a), false); // A se termine après : ignoré
	assert.equal(langueCourante(), 'pt');
	// Sans jeton, la langue est celle de la navigation courante.
	assert.equal(fixerLangue('es'), true);
	assert.equal(langueCourante(), 'es');
});

test('fixerLangue met l’attribut lang du document à jour sans attendre la navigation', () => {
	const document = { documentElement: { lang: 'fr' } };
	Object.assign(globalThis, { document });
	try {
		fixerLangue('en');
		assert.equal(document.documentElement.lang, 'en');
		const vieux = nouvelleNavigation();
		nouvelleNavigation();
		fixerLangue('pt', vieux);
		assert.equal(document.documentElement.lang, 'en');
	} finally {
		Reflect.deleteProperty(globalThis, 'document');
	}
});
