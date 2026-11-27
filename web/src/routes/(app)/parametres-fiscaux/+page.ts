// Paramètres fiscaux d'un dossier (`?cabinet=<uuid>&dossier=<uuid>[&date=AAAA-MM-JJ]`) : versions
// du pays et du dossier, version applicable à la date choisie (le jour du dossier par défaut).
import { lireApplicable, listerVersions } from '#lib/api/parametres-fiscaux.ts';
import { listerMembres } from '#lib/api/ressources.ts';
import { tenter } from '#lib/api/resultat.ts';
import { api } from '#lib/application.ts';
import { chargerDossier, ROLES_CONTEXTE } from '#lib/contexte-dossier.ts';
import { estDateIso } from '#lib/dates.ts';
import type { PageLoad } from './$types';

export const load: PageLoad = ({ url }) =>
	chargerDossier(url, async ({ cabinet, dossier, membre }) => {
		const demandee = url.searchParams.get('date');
		// Une date illisible dans l'adresse est ignorée : le jour du dossier s'applique.
		const date = demandee !== null && estDateIso(demandee) ? demandee : undefined;
		const [liste, applicable] = await Promise.all([
			listerVersions(api, cabinet, dossier),
			lireApplicable(api, cabinet, dossier, date)
		]);
		// Les noms des auteurs ne sont lisibles que par l'administrateur (liste des membres) : sans
		// elle (autre rôle, ou erreur), l'identifiant abrégé s'affiche.
		const membres = ROLES_CONTEXTE.includes(membre.role)
			? await tenter(() => listerMembres(api, cabinet))
			: null;
		const auteurs = new Map<string, string>(
			membres?.ok ? membres.valeur.map((m) => [m.id, m.nom] as [string, string]) : []
		);
		return { liste, applicable, dateChoisie: date ?? null, auteurs };
	});
