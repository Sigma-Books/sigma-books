// @ts-check
import { defineConfig } from 'astro/config';

// Site public statique, zéro JavaScript par défaut (ADR 0002).
export default defineConfig({
	output: 'static',
	// CSP stricte injectée en <meta> : Astro y ajoute les empreintes des scripts et styles
	// qu'il émet (script-src, style-src), sans 'unsafe-inline'. Tout le reste est refusé par défaut.
	security: {
		csp: {
			directives: [
				"default-src 'none'",
				"img-src 'self'",
				"font-src 'self'",
				"base-uri 'none'",
				"form-action 'none'",
				"object-src 'none'"
			]
		}
	}
});
