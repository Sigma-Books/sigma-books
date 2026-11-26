// Intégrité d'un dossier : état de la chaîne, jetons d'horodatage, vérification depuis la base,
// témoin et export (`crates/api/src/serveur/routes/integrite.rs`, `serveur/integrite.rs`).
// Lecture seule, sauf l'export (POST, session fraîche). Le serveur décide de tout : l'interface
// affiche. Aucun jeton DER ni contenu d'écriture ne transite ici (l'export est un fichier brut).

import type { ClientApi } from './client.ts';
import { chaines, entier, estId, estObjet, tableauDe } from './gardes.ts';
import type { Garde, Uuid } from './types.ts';

export interface JetonHorodatage {
	id: Uuid;
	/** Nombre d'écritures couvertes. */
	nombre: number;
	/** Tête de chaîne horodatée (empreinte hexadécimale). */
	tete: string;
	autorite: string;
	date_jeton: string;
	obtenu_le: string;
	declencheur: string;
}

export interface EtatIntegrite {
	dossier: Uuid;
	nombre: number;
	tete: string;
	jetons: number;
	dernier_jeton: JetonHorodatage | null;
	ecritures_sans_jeton: number;
	horodatage: { en_attente_depuis: string | null; retard: boolean };
	temoin: { contenu: string; nombre: number; tete: string };
}

export interface ListeJetons {
	total: number;
	jetons: JetonHorodatage[];
}

export interface AnomalieChaine {
	code: string;
	rang: number | null;
	numero: string | null;
	detail: string | null;
}

export interface RapportVerification {
	integre: boolean;
	nombre: number;
	tete: string;
	jetons: number;
	jetons_valides: number;
	jetons_non_verifies: number;
	/** Rang de la première écriture altérée, s'il y en a une. */
	premiere_alteration: number | null;
	anomalies: AnomalieChaine[];
	/** Anomalies non listées (la liste est tronquée côté serveur). */
	anomalies_omises: number;
}

const nulOuChaine = (v: unknown): boolean => v === null || typeof v === 'string';
const nulOuEntier = (v: unknown): boolean => v === null || entier(v);

const estJeton = (v: unknown): v is JetonHorodatage =>
	estObjet(v) &&
	estId(v.id) &&
	entier(v.nombre) &&
	chaines(v, ['tete', 'autorite', 'date_jeton', 'obtenu_le', 'declencheur']);

export const estEtatIntegrite: Garde<EtatIntegrite> = (v): v is EtatIntegrite =>
	estObjet(v) &&
	estId(v.dossier) &&
	entier(v.nombre) &&
	typeof v.tete === 'string' &&
	entier(v.jetons) &&
	(v.dernier_jeton === null || estJeton(v.dernier_jeton)) &&
	entier(v.ecritures_sans_jeton) &&
	estObjet(v.horodatage) &&
	nulOuChaine(v.horodatage.en_attente_depuis) &&
	typeof v.horodatage.retard === 'boolean' &&
	estObjet(v.temoin) &&
	typeof v.temoin.contenu === 'string' &&
	entier(v.temoin.nombre) &&
	typeof v.temoin.tete === 'string';

export const estListeJetons: Garde<ListeJetons> = (v): v is ListeJetons =>
	estObjet(v) && entier(v.total) && tableauDe(v.jetons, estJeton);

const estAnomalie = (v: unknown): v is AnomalieChaine =>
	estObjet(v) &&
	typeof v.code === 'string' &&
	nulOuEntier(v.rang) &&
	nulOuChaine(v.numero) &&
	nulOuChaine(v.detail);

export const estRapportVerification: Garde<RapportVerification> = (v): v is RapportVerification =>
	estObjet(v) &&
	typeof v.integre === 'boolean' &&
	entier(v.nombre) &&
	typeof v.tete === 'string' &&
	entier(v.jetons) &&
	entier(v.jetons_valides) &&
	entier(v.jetons_non_verifies) &&
	nulOuEntier(v.premiere_alteration) &&
	tableauDe(v.anomalies, estAnomalie) &&
	entier(v.anomalies_omises);

const base = (cabinet: Uuid, dossier: Uuid) =>
	`/api/cabinets/${cabinet}/dossiers/${dossier}/integrite`;

export function lireEtatIntegrite(api: ClientApi, c: Uuid, d: Uuid): Promise<EtatIntegrite> {
	return api.lire(base(c, d), estEtatIntegrite);
}

export function listerJetons(api: ClientApi, c: Uuid, d: Uuid): Promise<ListeJetons> {
	return api.lire(`${base(c, d)}/jetons`, estListeJetons);
}

/** Recalcule la chaîne depuis la base (lecture seule, mais coûteuse : à lancer à la demande). */
export function verifierChaine(api: ClientApi, c: Uuid, d: Uuid): Promise<RapportVerification> {
	return api.lire(`${base(c, d)}/verification`, estRapportVerification);
}

/** Fichier prêt à être enregistré par le navigateur. */
export interface FichierIntegrite {
	contenu: Blob;
	/** Nom sûr, construit ici (jamais repris de l'en-tête du serveur). */
	nom: string;
	/**
	 * Vrai si l'export ne contient pas tous les jetons (les plus récents seulement) ; `null` si le
	 * serveur ne l'a pas dit (en-tête absent ou mal formé) : la troncature est alors inconnue.
	 */
	tronque: boolean | null;
	jetonsTotal: number | null;
	jetonsExportes: number | null;
}

function entierEntete(valeur: string | null): number | null {
	return valeur !== null && /^\d{1,9}$/.test(valeur) ? Number.parseInt(valeur, 10) : null;
}

function booleenEntete(valeur: string | null): boolean | null {
	return valeur === 'true' ? true : valeur === 'false' ? false : null;
}

/** Rôles auxquels l'interface propose l'intégrité (masquage seulement : le serveur décide, §20). */
export const ROLES_INTEGRITE: readonly string[] = ['administrateur', 'valideur', 'auditeur'];

/** Lecture des en-têtes `x-sigma-*` de l'export (toute forme inattendue est ignorée). */
export function lireEntetesExport(
	entetes: Pick<Headers, 'get'>
): Pick<FichierIntegrite, 'tronque' | 'jetonsTotal' | 'jetonsExportes'> {
	return {
		tronque: booleenEntete(entetes.get('x-sigma-export-tronque')),
		jetonsTotal: entierEntete(entetes.get('x-sigma-jetons-total')),
		jetonsExportes: entierEntete(entetes.get('x-sigma-jetons-exportes'))
	};
}

/**
 * Export d'intégrité : `POST`, session fraîche exigée par le serveur (à appeler via
 * `executerSensible`). Le fichier est livré tel que reçu.
 */
export async function exporterIntegrite(
	api: ClientApi,
	cabinet: Uuid,
	dossier: Uuid
): Promise<FichierIntegrite> {
	const { contenu, entetes } = await api.telecharger('POST', `${base(cabinet, dossier)}/export`);
	return { contenu, nom: `integrite-dossier-${dossier}.txt`, ...lireEntetesExport(entetes) };
}

/** Témoin externe courant : `GET`, à conserver hors de l'instance. */
export async function telechargerTemoin(
	api: ClientApi,
	cabinet: Uuid,
	dossier: Uuid
): Promise<{ contenu: Blob; nom: string }> {
	const { contenu } = await api.telecharger('GET', `${base(cabinet, dossier)}/temoin`);
	return { contenu, nom: `temoin-${dossier}.txt` };
}
