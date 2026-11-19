// Types globaux de SvelteKit : https://svelte.dev/docs/kit/types#app.d.ts
import type { Uuid } from '#lib/api/types.ts';

/** Cabinet ou dossier d'une page, pour le fil d'Ariane de la coquille. */
interface Etape {
	id: Uuid;
	nom: string;
}

declare global {
	namespace App {
		// interface Error {}
		// interface Locals {}
		interface PageData {
			/** Fil d'Ariane : cabinet et dossier lus dans l'URL, nommés par l'API. */
			fil?: { cabinet?: Etape; dossier?: Etape };
		}
		// interface PageState {}
		// interface Platform {}
	}
}

export {};
