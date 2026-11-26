// Les seuils de `seuils.ts` sont recopiés du serveur (l'API ne les expose pas) : ce test échoue
// s'ils divergent de leur valeur de référence dans le code Rust, qu'il relit. Sans le dépôt complet
// (par exemple `web/` seul), il est sauté.

import assert from 'node:assert/strict';
import { existsSync, readFileSync } from 'node:fs';
import { test } from 'node:test';
import { DELAI_RETARD_HORODATAGE_HEURES, PREAVIS_ECHEANCE_JOURS } from './seuils.ts';

const RACINE = new URL('../../../', import.meta.url);
const EDITIONS = new URL('crates/api/src/serveur/routes/editions.rs', RACINE);
const TACHE = new URL('crates/api/src/horodatage/tache.rs', RACINE);
const present = existsSync(EDITIONS) && existsSync(TACHE);

test(
	'PREAVIS_ECHEANCE_JOURS vaut PREAVIS_ECHEANCE du serveur',
	{ skip: !present && 'sources du serveur absentes' },
	() => {
		const source = readFileSync(EDITIONS, 'utf8');
		const m = /pub const PREAVIS_ECHEANCE: i64 = (\d+);/.exec(source);
		assert.ok(
			m,
			'PREAVIS_ECHEANCE introuvable dans editions.rs : mettre à jour seuils.ts et ce test'
		);
		assert.equal(PREAVIS_ECHEANCE_JOURS, Number(m[1]));
	}
);

test(
	'DELAI_RETARD_HORODATAGE_HEURES vaut le délai d’alerte par défaut du serveur',
	{ skip: !present && 'sources du serveur absentes' },
	() => {
		const source = readFileSync(TACHE, 'utf8');
		const m = /delai_alerte: time::Duration::hours\((\d+)\)/.exec(source);
		assert.ok(m, 'délai d’alerte introuvable dans tache.rs : mettre à jour seuils.ts et ce test');
		assert.equal(DELAI_RETARD_HORODATAGE_HEURES, Number(m[1]));
	}
);
