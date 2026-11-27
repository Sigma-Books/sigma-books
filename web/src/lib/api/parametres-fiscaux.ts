// Paramètres fiscaux d'un dossier (ADR 0030) : types, gardes et appels des routes
// `…/dossiers/{dossier}/parametres-fiscaux` (`crates/api/src/serveur/routes/parametres_fiscaux.rs`).
//
// Tout ce que l'interface affiche (versions, taux, régimes, motifs, leurs codes et libellés) vient
// de l'API : aucun code, taux, article ni libellé fiscal n'est connu d'ici. Les gardes vérifient la
// forme de la réponse ; le serveur décide de tout le reste.

import { estDateIso } from '../dates.ts';
import type { ClientApi } from './client.ts';
import { chaineFacultative, entier, estId, estObjet, tableauDe, type Objet } from './gardes.ts';
import type { Garde, Uuid } from './types.ts';

/** Code stable du serveur (régime, motif, rôle de taux, exigibilité, déduction). */
const CODE = /^[A-Za-z0-9_.+-]{1,64}$/;
const estCode = (v: unknown): v is string => typeof v === 'string' && CODE.test(v);
/** Texte d'affichage du serveur : non vide, borné (jamais interprété comme du HTML). */
const estTexte = (v: unknown): v is string => typeof v === 'string' && v.length <= 2000;
const estDate = (v: unknown): v is string => typeof v === 'string' && estDateIso(v);
const dateFacultative = (o: Objet, champ: string): boolean =>
	o[champ] === undefined || estDate(o[champ]);

/** Version de paramètres : celle du paquet du pays, ou une version propre au dossier. */
export interface VersionFiscale {
	/** Identifiant enregistré par les écritures (`<pays>-<AAAA.N>`, `…+d<N>` pour un dossier). */
	identifiant: string;
	portee: 'pays' | 'dossier';
	/** Version du paquet (celle de base, pour une version propre). */
	version: string;
	/** Numéro dans le dossier (versions propres). */
	numero?: number;
	id?: Uuid;
	date_effet: string;
	date_fin?: string;
	motif?: string;
	cree_par?: string;
	cree_le?: string;
	empreinte?: string;
}

export function estVersionFiscale(v: unknown): v is VersionFiscale {
	return (
		estObjet(v) &&
		estTexte(v.identifiant) &&
		v.identifiant !== '' &&
		(v.portee === 'pays' || v.portee === 'dossier') &&
		estTexte(v.version) &&
		(v.numero === undefined || entier(v.numero)) &&
		(v.id === undefined || estId(v.id)) &&
		estDate(v.date_effet) &&
		dateFacultative(v, 'date_fin') &&
		chaineFacultative(v, 'motif') &&
		chaineFacultative(v, 'cree_par') &&
		chaineFacultative(v, 'cree_le') &&
		chaineFacultative(v, 'empreinte')
	);
}

/** `GET …/parametres-fiscaux` : paquet du pays puis versions propres. */
export interface ListeVersions {
	pays: string;
	versions: VersionFiscale[];
}

export const estListeVersions: Garde<ListeVersions> = (v): v is ListeVersions =>
	estObjet(v) && estTexte(v.pays) && tableauDe(v.versions, estVersionFiscale);

export interface TauxFiscal {
	/** Rôle du taux (`normal`, `reduit`…), défini par le socle de l'API. */
	role: string;
	/** Pourcentage, décimal canonique. */
	valeur: string;
	source: string;
}

export interface RegimeFiscal {
	code: string;
	libelle: string;
	exonere: boolean;
	ligne_taxe: boolean;
	taux: string[];
	exigibilites: string[];
	deduction: string;
}

export interface MotifFiscal {
	code: string;
	regime: string;
	libelle: string;
}

/** `GET …/parametres-fiscaux/applicable` : la version à une date, avec ce qu'elle contient. */
export interface Applicable {
	date: string;
	/** `null` : aucune version ne s'applique à cette date. */
	version: VersionFiscale | null;
	taux: TauxFiscal[];
	regimes: RegimeFiscal[];
	motifs: MotifFiscal[];
	retention_annees: number | null;
}

function estTauxFiscal(v: unknown): v is TauxFiscal {
	return estObjet(v) && estCode(v.role) && estCode(v.valeur) && estTexte(v.source);
}

function estRegimeFiscal(v: unknown): v is RegimeFiscal {
	return (
		estObjet(v) &&
		estCode(v.code) &&
		estTexte(v.libelle) &&
		typeof v.exonere === 'boolean' &&
		typeof v.ligne_taxe === 'boolean' &&
		tableauDe(v.taux, estCode) &&
		tableauDe(v.exigibilites, estCode) &&
		estCode(v.deduction)
	);
}

function estMotifFiscal(v: unknown): v is MotifFiscal {
	return estObjet(v) && estCode(v.code) && estCode(v.regime) && estTexte(v.libelle);
}

export const estApplicable: Garde<Applicable> = (v): v is Applicable =>
	estObjet(v) &&
	estDate(v.date) &&
	(v.version === null || estVersionFiscale(v.version)) &&
	tableauDe(v.taux, estTauxFiscal) &&
	tableauDe(v.regimes, estRegimeFiscal) &&
	tableauDe(v.motifs, estMotifFiscal) &&
	(v.retention_annees === null || entier(v.retention_annees)) &&
	// Sans version, rien n'est détaillé.
	(v.version !== null || (v.taux.length === 0 && v.regimes.length === 0));

/** Avertissement d'une création réussie. */
export interface AvertissementVersion {
	code: string;
	nombre: number;
}

/** Réponse de `POST …/parametres-fiscaux` : la version, plus ses avertissements éventuels. */
export type VersionCreee = VersionFiscale & { avertissements: AvertissementVersion[] };

function estAvertissement(v: unknown): v is AvertissementVersion {
	return estObjet(v) && estCode(v.code) && entier(v.nombre);
}

export const estVersionCreee: Garde<VersionCreee> = (v): v is VersionCreee =>
	estVersionFiscale(v) && tableauDe((v as unknown as Objet).avertissements, estAvertissement);

/** Corps de `POST …/parametres-fiscaux` : une version propre dérivée de celle qui s'applique. */
export interface CorpsVersion {
	date_effet: string;
	date_fin?: string;
	motif: string;
	/** Taux remplacés, par rôle : décimal canonique dans ]0 ; 100]. */
	taux?: Record<string, string>;
	/** Codes des régimes retirés de l'offre du dossier à partir de `date_effet`. */
	regimes_retires?: string[];
}

/** Code de l'avertissement d'une création rétroactive (écritures déjà validées dans la période). */
export const AVERTISSEMENT_ECRITURES_VALIDEES = 'ecritures_deja_validees';

const base = (cabinet: Uuid, dossier: Uuid) =>
	`/api/cabinets/${cabinet}/dossiers/${dossier}/parametres-fiscaux`;

/** Versions du dossier (tout rôle qui voit le dossier). */
export function listerVersions(
	api: ClientApi,
	cabinet: Uuid,
	dossier: Uuid
): Promise<ListeVersions> {
	return api.lire(base(cabinet, dossier), estListeVersions);
}

/**
 * Version applicable à une date `AAAA-MM-JJ` (sans date : le jour du dossier). La date est
 * vérifiée avant de former l'adresse.
 */
export function lireApplicable(
	api: ClientApi,
	cabinet: Uuid,
	dossier: Uuid,
	date?: string
): Promise<Applicable> {
	if (date !== undefined && !estDateIso(date)) throw new TypeError('date invalide');
	const suite = date === undefined ? '' : `?date=${date}`;
	return api.lire(`${base(cabinet, dossier)}/applicable${suite}`, estApplicable);
}

/**
 * Création d'une version propre (administrateur, session fraîche : l'appelant passe par
 * `executerSensible`). Jamais une modification : une version publiée est immuable.
 */
export function creerVersion(
	api: ClientApi,
	cabinet: Uuid,
	dossier: Uuid,
	corps: CorpsVersion
): Promise<VersionCreee> {
	return api.muter('POST', base(cabinet, dossier), estVersionCreee, corps);
}

/**
 * Motifs de refus de la création d'une version et leur statut (422 : refus de forme ; 409 :
 * conflit d'état), avec les refus du contexte qu'elle entraîne. Chacun a son message
 * (`saisie.motif.<motif>`) ; les tests le vérifient en passant par `traduireReponse`.
 */
export const MOTIFS_PARAMETRES = [
	['parametres_date_effet_lointaine', 422],
	['parametres_aucun_changement', 422],
	['parametres_taux_invalide', 422],
	['parametres_role_inconnu', 422],
	['parametres_regime_inconnu', 422],
	['parametres_regime_non_retirable', 422],
	['parametres_motif_invalide', 422],
	['parametres_periode_invalide', 422],
	['parametres_aucune_base', 422],
	['parametres_pays_sans_paquet', 422],
	['parametres_date_effet_cloturee', 409],
	['parametres_date_effet_anterieure', 409],
	['parametres_regime_utilise', 409],
	['parametres_trop_de_versions', 409],
	['etiquette_invalide', 422],
	['parametres_fiscaux_absents', 422],
	['validation_parametres_fiscaux_absents', 422]
] as const;
