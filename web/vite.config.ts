import adapter from '@sveltejs/adapter-static';
import { sveltekit } from '@sveltejs/kit/vite';
import { defineConfig } from 'vite';

export default defineConfig({
	plugins: [
		sveltekit({
			compilerOptions: {
				// Mode runes imposé au projet (sauf bibliothèques).
				runes: ({ filename }) =>
					filename.split(/[/\\]/).includes('node_modules') ? undefined : true
			},

			// Fichiers statiques servis par le backend Rust : aucun serveur Node en production (ADR 0002).
			adapter: adapter({ strict: true }),

			// CSP stricte : aucun script ni style inline non haché, aucune ressource externe.
			// Les scripts d'amorçage de SvelteKit sont autorisés par leur empreinte (mode « hash »),
			// injectée dans une balise <meta> des pages prérendues. Les en-têtes non exprimables en
			// <meta> (frame-ancestors, etc.) seront posés par le backend.
			csp: {
				mode: 'hash',
				directives: {
					'default-src': ['none'],
					'script-src': ['self'],
					'style-src': ['self'],
					// Seul style inline admis : celui de l'annonceur d'accessibilité de SvelteKit
					// (#svelte-announcer), autorisé par son empreinte exacte. À recalculer si une mise à
					// jour de SvelteKit modifie ce style (la violation CSP apparaît dans la console).
					'style-src-attr': [
						'unsafe-hashes',
						'sha256-S8qMpvofolR8Mpjy4kQvEm7m1q8clzU4dfDH0AmvZjo='
					],
					'img-src': ['self'],
					'font-src': ['self'],
					'connect-src': ['self'],
					'manifest-src': ['self'],
					'base-uri': ['none'],
					'form-action': ['self'],
					'object-src': ['none'],
					// Trusted Types : seules les politiques de Svelte et SvelteKit sont autorisées.
					'require-trusted-types-for': ['script'],
					'trusted-types': ['svelte-trusted-html', 'sveltekit-trusted-url']
				}
			}
		})
	]
});
