// Contrôle de la CSP des pages compilées (`build/**/index.html`) : `pnpm build && pnpm test`.
// Sans build, ces tests sont sautés : les autres tests unitaires ne dépendent pas de la compilation.
//
// Exigences : une balise <meta> CSP par page ; aucune source `'unsafe-*'`, à une exception près :
// `'unsafe-hashes'` dans `style-src-attr`, qui n'autorise que le style de l'annonceur de SvelteKit
// par son empreinte exacte (vite.config.ts) ; chaque script inline autorisé par son empreinte.

import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { existsSync, readdirSync, readFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, it } from 'node:test';
import { fileURLToPath } from 'node:url';

const BUILD = fileURLToPath(new URL('../build/', import.meta.url));

function pages(repertoire: string): string[] {
	return readdirSync(repertoire, { withFileTypes: true }).flatMap((entree) => {
		const chemin = join(repertoire, entree.name);
		if (entree.isDirectory()) return pages(chemin);
		return entree.name === 'index.html' ? [chemin] : [];
	});
}

function directives(csp: string): Map<string, string[]> {
	return new Map(
		csp
			.split(';')
			.map((d) => d.trim().split(/\s+/))
			.filter((d) => d[0] !== '')
			.map(([nom, ...sources]) => [nom ?? '', sources])
	);
}

const avecBuild = existsSync(BUILD);

describe('CSP des pages compilées', { skip: avecBuild ? false : 'build/ absent' }, () => {
	const fichiers = avecBuild ? pages(BUILD) : [];

	it('trouve les pages', () => {
		assert.ok(fichiers.length > 0);
	});

	for (const fichier of fichiers) {
		it(fichier.slice(BUILD.length), () => {
			const html = readFileSync(fichier, 'utf8');
			const metas = [
				...html.matchAll(/<meta http-equiv="content-security-policy" content="([^"]*)">/g)
			];
			assert.equal(metas.length, 1, 'une seule balise <meta> CSP');
			const csp = directives(metas[0]?.[1] ?? '');

			for (const [nom, sources] of csp) {
				for (const source of sources.filter((s) => s.startsWith("'unsafe-"))) {
					assert.ok(
						nom === 'style-src-attr' && source === "'unsafe-hashes'",
						`${nom} : ${source} interdit`
					);
				}
			}
			assert.deepEqual(csp.get('default-src'), ["'none'"]);
			assert.deepEqual(csp.get('object-src'), ["'none'"]);
			assert.deepEqual(csp.get('base-uri'), ["'none'"]);
			assert.deepEqual(csp.get('require-trusted-types-for'), ["'script'"]);

			const scripts = csp.get('script-src') ?? [];
			assert.ok(scripts.every((s) => s === "'self'" || /^'sha256-[A-Za-z0-9+/=]+'$/.test(s)));
			// Toute balise <script>, quelles que soient la casse et les attributs : un script inline
			// (sans `src`) doit avoir son empreinte dans `script-src`.
			for (const [, attributs, inline] of html.matchAll(
				/<script\b([^>]*)>([\s\S]*?)<\/script\s*>/gi
			)) {
				if (/\bsrc\s*=/i.test(attributs ?? '')) continue;
				const empreinte = createHash('sha256')
					.update(inline ?? '')
					.digest('base64');
				assert.ok(scripts.includes(`'sha256-${empreinte}'`), 'script inline non haché');
			}
		});
	}
});
