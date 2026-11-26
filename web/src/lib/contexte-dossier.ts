// Contexte commun des écrans d'un dossier : cabinet et dossier lus dans l'URL (validés par
// `lireUuid`), cabinet activé (rotation de session), rôle du membre, nom du dossier et contexte
// régional du dossier (pays, devise, échelle décimale, fuseau, langue : `regional.ts`).

import { activerCabinet, api, membreDuCabinet } from './application.ts';
import { lireDossier } from './api/ressources.ts';
import { tenter, type Resultat } from './api/resultat.ts';
import { lireUuid, type CabinetMembre, type Dossier, type Uuid } from './api/types.ts';
import { fixerLangue } from './i18n/index.ts';
import { lireRegional, type Regional } from './regional.ts';

export interface ContexteDossier {
	cabinet: Uuid;
	dossier: Uuid;
	membre: CabinetMembre;
	nomDossier: Dossier;
	/** Devise, échelle, locale, fuseau et langue du dossier (valeurs par défaut si l'API n'en dit rien). */
	regional: Regional;
}

/** Rôles auxquels l'interface propose la saisie (simple masquage : le serveur décide, §20). */
export const ROLES_SAISIE: readonly string[] = ['administrateur', 'valideur', 'collaborateur'];
/** Rôles auxquels elle propose la validation par lot et la contre-passation. */
export const ROLES_VALIDATION_LOT: readonly string[] = ['administrateur', 'valideur'];

/** Fil d'Ariane de la coquille (`page.data.fil`). */
export function filDepuis(c: ContexteDossier) {
	return {
		cabinet: { id: c.cabinet, nom: c.membre.nom },
		dossier: { id: c.dossier, nom: c.nomDossier.nom }
	};
}

/**
 * Charge le contexte, puis `suite` avec lui. `contenu` vaut `null` si l'adresse est invalide
 * (cabinet ou dossier absent ou mal formé).
 */
export async function chargerDossier<T>(
	url: URL,
	suite: (contexte: ContexteDossier) => Promise<T>
): Promise<{
	contenu: Resultat<{ contexte: ContexteDossier; valeur: T }> | null;
	fil?: ReturnType<typeof filDepuis>;
}> {
	const cabinet = lireUuid(url.searchParams.get('cabinet'));
	const dossier = lireUuid(url.searchParams.get('dossier'));
	if (cabinet === null || dossier === null) return { contenu: null };
	const contenu = await tenter(async () => {
		await activerCabinet(cabinet);
		const [membre, nomDossier] = await Promise.all([
			membreDuCabinet(cabinet),
			lireDossier(api, cabinet, dossier)
		]);
		// Seul point de raccordement du contexte régional : l'API de lecture du dossier.
		const regional = lireRegional(nomDossier);
		fixerLangue(regional.langue);
		const contexte = { cabinet, dossier, membre, nomDossier, regional };
		return { contexte, valeur: await suite(contexte) };
	});
	return contenu.ok ? { contenu, fil: filDepuis(contenu.valeur.contexte) } : { contenu };
}
