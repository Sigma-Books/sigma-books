// Lectures du référentiel du dossier utiles à la saisie : comptes, journaux, tiers, exercices.
// Simple affichage et aide à la frappe : le serveur contrôle seul les comptes et tiers admis.

import type { ClientApi } from './client.ts';
import { chaines, curseurOuNul, estId, estObjet, tableauDe } from './gardes.ts';
import type { Garde, Uuid } from './types.ts';

export interface Compte {
	id: Uuid;
	numero: string;
	intitule: string;
	actif: boolean;
	/** Compte de mouvement (sans enfant) : seul un tel compte reçoit des écritures. */
	mouvement: boolean;
}

export interface Journal {
	id: Uuid;
	code: string;
	intitule: string;
}

export interface Tiers {
	id: Uuid;
	nom: string;
	actif: boolean;
}

export interface Periode {
	id: Uuid;
	debut: string;
	fin: string;
	statut: string;
}

export interface Exercice {
	id: Uuid;
	code: string;
	debut: string;
	fin: string;
	statut: string;
	periodes: Periode[];
}

const estCompte = (v: unknown): v is Compte =>
	estObjet(v) &&
	estId(v.id) &&
	chaines(v, ['numero', 'intitule']) &&
	typeof v.actif === 'boolean' &&
	typeof v.mouvement === 'boolean';

const estJournal = (v: unknown): v is Journal =>
	estObjet(v) && estId(v.id) && chaines(v, ['code', 'intitule']);

const estTiers = (v: unknown): v is Tiers =>
	estObjet(v) && estId(v.id) && typeof v.nom === 'string' && typeof v.actif === 'boolean';

const estPeriode = (v: unknown): v is Periode =>
	estObjet(v) && estId(v.id) && chaines(v, ['debut', 'fin', 'statut']);

const estExercice = (v: unknown): v is Exercice =>
	estObjet(v) &&
	estId(v.id) &&
	chaines(v, ['code', 'debut', 'fin', 'statut']) &&
	tableauDe(v.periodes, estPeriode);

function liste<T, K extends string>(cle: K, garde: (v: unknown) => v is T): Garde<Record<K, T[]>> {
	return (v): v is Record<K, T[]> => estObjet(v) && tableauDe(v[cle], garde);
}

const base = (cabinet: Uuid, dossier: Uuid) => `/api/cabinets/${cabinet}/dossiers/${dossier}`;

export async function listerComptes(
	api: ClientApi,
	cabinet: Uuid,
	dossier: Uuid
): Promise<Compte[]> {
	return (await api.lire(`${base(cabinet, dossier)}/comptes`, liste('comptes', estCompte))).comptes;
}

export async function listerJournaux(
	api: ClientApi,
	cabinet: Uuid,
	dossier: Uuid
): Promise<Journal[]> {
	return (await api.lire(`${base(cabinet, dossier)}/journaux`, liste('journaux', estJournal)))
		.journaux;
}

export async function listerExercices(
	api: ClientApi,
	cabinet: Uuid,
	dossier: Uuid
): Promise<Exercice[]> {
	return (await api.lire(`${base(cabinet, dossier)}/exercices`, liste('exercices', estExercice)))
		.exercices;
}

const estPageTiers: Garde<{ tiers: Tiers[]; suite?: string | null }> = (
	v
): v is { tiers: Tiers[]; suite?: string | null } =>
	estObjet(v) && tableauDe(v.tiers, estTiers) && curseurOuNul(v.suite);

/** Tous les tiers du dossier, page par page (200 par page, au plus 20 pages). */
export async function listerTiers(api: ClientApi, cabinet: Uuid, dossier: Uuid): Promise<Tiers[]> {
	const tous: Tiers[] = [];
	let apres: string | null = null;
	for (let page = 0; page < 20; page += 1) {
		const suffixe: string = apres === null ? '' : `&apres=${encodeURIComponent(apres)}`;
		const lue = await api.lire(
			`${base(cabinet, dossier)}/tiers?limite=200${suffixe}`,
			estPageTiers
		);
		tous.push(...lue.tiers);
		if (lue.suite === null || lue.suite === undefined) break;
		apres = lue.suite;
	}
	return tous;
}
