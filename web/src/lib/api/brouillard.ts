// Brouillard, validation, écritures validées et contre-passation : types, gardes et appels
// (`crates/api/src/serveur/routes/brouillard*.rs`). Toutes les mutations sont des POST (le client
// n'envoie ni PUT ni PATCH) ; le cabinet et le dossier sont des `Uuid` déjà vérifiés.

import type { ClientApi } from './client.ts';
import type { CorpsEcriture } from './saisie.ts';
import {
	chaineFacultative,
	chaines,
	curseurOuNul,
	entier,
	estId,
	estObjet,
	tableauDe
} from './gardes.ts';
import { traduire } from '../i18n/index.ts';
import type { Langue } from '../regional.ts';
import { longueurOctets } from './texte.ts';
import { estCodeJournal, type Garde, type Uuid } from './types.ts';

export type Sens = 'debit' | 'credit';

export interface LigneVue {
	position: number;
	compte: string;
	sens: Sens;
	/** Montant décimal exact, chaîne canonique. */
	montant: string;
	tiers?: string;
}

export interface PieceLiee {
	piece: string;
	role: string;
	lie_par: string;
}

export interface Brouillon {
	id: Uuid;
	journal: string;
	date: string;
	date_valeur_comptable?: string;
	libelle: string;
	a_revoir: boolean;
	a_revoir_par?: string;
	version: number;
	cree_par: string;
	cree_le: string;
	modifie_par: string;
	modifie_le: string;
	contributeurs: string[];
	lignes: LigneVue[];
	pieces: PieceLiee[];
}

export interface ResumeBrouillon {
	id: Uuid;
	journal: string;
	date: string;
	date_valeur_comptable?: string;
	libelle: string;
	a_revoir: boolean;
	version: number;
	lignes: number;
	debit: string;
	credit: string;
	cree_par: string;
	cree_le: string;
	modifie_par: string;
	modifie_le: string;
}

export interface PageBrouillard {
	brouillons: ResumeBrouillon[];
	/** Curseur de la page suivante ; absent ou nul en fin de liste. */
	suivant?: string | null;
}

export interface Validation {
	id: Uuid;
	numero: string;
	rang: number;
	empreinte: string;
	format: number;
	date_validation: string;
}

export interface LotValide {
	ecritures: Validation[];
	exclues_a_revoir: number;
}

export interface PieceEcriture {
	piece: string;
	role: string;
	empreinte: string;
}

export interface Ecriture {
	id: Uuid;
	numero: string;
	journal: string;
	exercice: string;
	date: string;
	date_valeur_comptable?: string;
	libelle: string;
	rang: number;
	empreinte: string;
	empreinte_precedente: string;
	format: number;
	valide_par: string;
	date_validation: string;
	valide_le: string;
	origine?: string;
	/** Numéros des contre-passations de l'écriture, dans l'ordre de la chaîne (zéro ou plusieurs). */
	contre_passations: string[];
	lignes: LigneVue[];
	pieces: PieceEcriture[];
}

export interface ResumeEcriture {
	id: Uuid;
	numero: string;
	date: string;
	libelle: string;
	rang: number;
	empreinte: string;
	total: string;
}

export interface PageEcritures {
	ecritures: ResumeEcriture[];
	/** Curseur de la page suivante ; absent ou nul en fin de liste. */
	suivant?: string | null;
}

export interface ContrePassation {
	id: Uuid;
	numero: string;
	origine: string;
	origine_numero: string;
	rang: number;
	empreinte: string;
	format: number;
	date_validation: string;
}

const estSens = (v: unknown): v is Sens => v === 'debit' || v === 'credit';

function estLigneVue(v: unknown): v is LigneVue {
	return (
		estObjet(v) &&
		entier(v.position) &&
		chaines(v, ['compte', 'montant']) &&
		estSens(v.sens) &&
		chaineFacultative(v, 'tiers')
	);
}

function estPieceLiee(v: unknown): v is PieceLiee {
	return estObjet(v) && chaines(v, ['piece', 'role', 'lie_par']);
}

function estPieceEcriture(v: unknown): v is PieceEcriture {
	return estObjet(v) && chaines(v, ['piece', 'role', 'empreinte']);
}

const estChaine = (v: unknown): v is string => typeof v === 'string';

export const estBrouillon: Garde<Brouillon> = (v): v is Brouillon =>
	estObjet(v) &&
	estId(v.id) &&
	chaines(v, ['journal', 'date', 'libelle', 'cree_par', 'cree_le', 'modifie_par', 'modifie_le']) &&
	chaineFacultative(v, 'date_valeur_comptable') &&
	chaineFacultative(v, 'a_revoir_par') &&
	typeof v.a_revoir === 'boolean' &&
	entier(v.version) &&
	tableauDe(v.contributeurs, estChaine) &&
	tableauDe(v.lignes, estLigneVue) &&
	tableauDe(v.pieces, estPieceLiee);

function estResumeBrouillon(v: unknown): v is ResumeBrouillon {
	return (
		estObjet(v) &&
		estId(v.id) &&
		chaines(v, [
			'journal',
			'date',
			'libelle',
			'debit',
			'credit',
			'cree_par',
			'cree_le',
			'modifie_par',
			'modifie_le'
		]) &&
		chaineFacultative(v, 'date_valeur_comptable') &&
		typeof v.a_revoir === 'boolean' &&
		entier(v.version) &&
		entier(v.lignes)
	);
}

export const estPageBrouillard: Garde<PageBrouillard> = (v): v is PageBrouillard =>
	estObjet(v) && tableauDe(v.brouillons, estResumeBrouillon) && curseurOuNul(v.suivant);

export const estValidation: Garde<Validation> = (v): v is Validation =>
	estObjet(v) &&
	estId(v.id) &&
	chaines(v, ['numero', 'empreinte', 'date_validation']) &&
	entier(v.rang) &&
	entier(v.format);

export const estLotValide: Garde<LotValide> = (v): v is LotValide =>
	estObjet(v) && tableauDe(v.ecritures, estValidation) && entier(v.exclues_a_revoir);

export const estEcriture: Garde<Ecriture> = (v): v is Ecriture =>
	estObjet(v) &&
	estId(v.id) &&
	chaines(v, [
		'numero',
		'journal',
		'exercice',
		'date',
		'libelle',
		'empreinte',
		'empreinte_precedente',
		'valide_par',
		'date_validation',
		'valide_le'
	]) &&
	chaineFacultative(v, 'date_valeur_comptable') &&
	chaineFacultative(v, 'origine') &&
	entier(v.rang) &&
	entier(v.format) &&
	tableauDe(v.contre_passations, estChaine) &&
	tableauDe(v.lignes, estLigneVue) &&
	tableauDe(v.pieces, estPieceEcriture);

function estResumeEcriture(v: unknown): v is ResumeEcriture {
	return (
		estObjet(v) &&
		estId(v.id) &&
		chaines(v, ['numero', 'date', 'libelle', 'empreinte', 'total']) &&
		entier(v.rang)
	);
}

export const estPageEcritures: Garde<PageEcritures> = (v): v is PageEcritures =>
	estObjet(v) && tableauDe(v.ecritures, estResumeEcriture) && curseurOuNul(v.suivant);

export const estContrePassation: Garde<ContrePassation> = (v): v is ContrePassation =>
	estObjet(v) &&
	estId(v.id) &&
	chaines(v, ['numero', 'origine', 'origine_numero', 'empreinte', 'date_validation']) &&
	entier(v.rang) &&
	entier(v.format);

/** Lot de 100 écritures au plus (`LOT_MAX` du serveur). */
export const LOT_MAX = 100;

function base(cabinet: Uuid, dossier: Uuid): string {
	return `/api/cabinets/${cabinet}/dossiers/${dossier}`;
}

export interface FiltreBrouillard {
	/** Code du journal (voir `estCodeJournal`), vide pour tous. */
	journal?: string;
	periode?: Uuid | null;
	apres?: string | null;
}

/** Paramètres de la liste ; une valeur de forme invalide est ignorée plutôt que d'être envoyée. */
export function parametresBrouillard(filtre: FiltreBrouillard, limite = 50): string {
	const p = new URLSearchParams();
	if (filtre.journal !== undefined && estCodeJournal(filtre.journal))
		p.set('journal', filtre.journal);
	if (filtre.periode) p.set('periode', filtre.periode);
	if (filtre.apres) p.set('apres', filtre.apres);
	p.set('limite', String(limite));
	return p.toString();
}

export function listerBrouillard(
	api: ClientApi,
	cabinet: Uuid,
	dossier: Uuid,
	filtre: FiltreBrouillard = {}
): Promise<PageBrouillard> {
	return api.lire(
		`${base(cabinet, dossier)}/brouillard?${parametresBrouillard(filtre)}`,
		estPageBrouillard
	);
}

export function lireBrouillon(
	api: ClientApi,
	cabinet: Uuid,
	dossier: Uuid,
	brouillon: Uuid
): Promise<Brouillon> {
	return api.lire(`${base(cabinet, dossier)}/brouillard/${brouillon}`, estBrouillon);
}

export function creerBrouillon(
	api: ClientApi,
	cabinet: Uuid,
	dossier: Uuid,
	corps: CorpsEcriture
): Promise<Brouillon> {
	return api.muter('POST', `${base(cabinet, dossier)}/brouillard`, estBrouillon, corps);
}

export function modifierBrouillon(
	api: ClientApi,
	cabinet: Uuid,
	dossier: Uuid,
	brouillon: Uuid,
	corps: CorpsEcriture
): Promise<Brouillon> {
	return api.muter(
		'POST',
		`${base(cabinet, dossier)}/brouillard/${brouillon}/modification`,
		estBrouillon,
		corps
	);
}

const estSupprime = (v: unknown): v is { id: string; supprime: true } =>
	estObjet(v) && typeof v.id === 'string' && v.supprime === true;

export async function supprimerBrouillon(
	api: ClientApi,
	cabinet: Uuid,
	dossier: Uuid,
	brouillon: Uuid,
	version: number
): Promise<void> {
	await api.muter(
		'POST',
		`${base(cabinet, dossier)}/brouillard/${brouillon}/suppression`,
		estSupprime,
		{ version }
	);
}

/** Pose (`true`) ou lève (`false`) le drapeau « à revoir ». */
export function changerDrapeau(
	api: ClientApi,
	cabinet: Uuid,
	dossier: Uuid,
	brouillon: Uuid,
	poser: boolean
): Promise<Brouillon> {
	const action = poser ? 'a-revoir' : 'revue-faite';
	return api.muter(
		'POST',
		`${base(cabinet, dossier)}/brouillard/${brouillon}/${action}`,
		estBrouillon,
		{}
	);
}

export type RolePieceJointe = 'principale' | 'complementaire';

export function lierPiece(
	api: ClientApi,
	cabinet: Uuid,
	dossier: Uuid,
	brouillon: Uuid,
	piece: Uuid,
	role: RolePieceJointe,
	version: number
): Promise<Brouillon> {
	return api.muter(
		'POST',
		`${base(cabinet, dossier)}/brouillard/${brouillon}/pieces`,
		estBrouillon,
		{ piece, role, version }
	);
}

export function retirerPiece(
	api: ClientApi,
	cabinet: Uuid,
	dossier: Uuid,
	brouillon: Uuid,
	piece: Uuid,
	version: number
): Promise<Brouillon> {
	return api.muter(
		'POST',
		`${base(cabinet, dossier)}/brouillard/${brouillon}/pieces/${piece}/retrait`,
		estBrouillon,
		{ version }
	);
}

export function validerBrouillon(
	api: ClientApi,
	cabinet: Uuid,
	dossier: Uuid,
	brouillon: Uuid,
	version: number
): Promise<Validation> {
	return api.muter(
		'POST',
		`${base(cabinet, dossier)}/brouillard/${brouillon}/validation`,
		estValidation,
		{ version }
	);
}

export interface ElementLot {
	ecriture: Uuid;
	version: number;
}

/** Validation d'un lot (1 à {@link LOT_MAX}), tout ou rien ; session fraîche exigée. */
export function validerParLot(
	api: ClientApi,
	cabinet: Uuid,
	dossier: Uuid,
	ecritures: readonly ElementLot[]
): Promise<LotValide> {
	return api.muter(
		'POST',
		`${base(cabinet, dossier)}/brouillard/validation-par-lot`,
		estLotValide,
		{ ecritures }
	);
}

export function listerEcritures(
	api: ClientApi,
	cabinet: Uuid,
	dossier: Uuid,
	apres: string | null = null
): Promise<PageEcritures> {
	const p = new URLSearchParams({ limite: '50' });
	if (apres !== null) p.set('apres', apres);
	return api.lire(`${base(cabinet, dossier)}/ecritures?${p.toString()}`, estPageEcritures);
}

export function lireEcriture(
	api: ClientApi,
	cabinet: Uuid,
	dossier: Uuid,
	ecriture: Uuid
): Promise<Ecriture> {
	return api.lire(`${base(cabinet, dossier)}/ecritures/${ecriture}`, estEcriture);
}

/** Longueur maximale du motif, en octets UTF-8 (`MOTIF_MAX` de `contre_passation.rs`). */
export const MOTIF_CONTRE_PASSATION_MAX = 400;

/**
 * Motif envoyé pour une contre-passation : la saisie **rognée**, ou, si elle est vide, un motif par
 * défaut « Contre-passation de <numéro> » dans la langue **demandée** (celle du dossier, passée en
 * paramètre : aucun état global). Le serveur exige aujourd'hui un motif non blanc (journal
 * d'audit) ; l'interface le rend facultatif, comme Odoo (`account.move.reversal`, raison
 * facultative) et Tryton (`CancelMoves`, description facultative).
 *
 * À terme, l'API fournira un défaut côté serveur quand le motif est vide (un autre chantier
 * assouplit l'API) : l'interface pourra alors envoyer un motif vide et supprimer ce défaut.
 */
export function motifContrePassation(saisie: string, numero: string, langue: Langue): string {
	const net = saisie.trim();
	return net === '' ? traduire('contre_passation.motif_defaut', { numero }, langue) : net;
}

/**
 * Vrai si le serveur acceptera ce motif : non blanc, au plus {@link MOTIF_CONTRE_PASSATION_MAX}
 * octets UTF-8 et sans caractère de contrôle (confort : le serveur contrôle de toute façon).
 */
export function motifAdmis(motif: string): boolean {
	return (
		motif.trim() !== '' &&
		longueurOctets(motif) <= MOTIF_CONTRE_PASSATION_MAX &&
		// eslint-disable-next-line no-control-regex
		!/[\u0000-\u001f\u007f-\u009f]/.test(motif)
	);
}

export interface CorpsContrePassation {
	/** `AAAA-MM-JJ`. */
	date: string;
	libelle: string;
	/** Motif non blanc (400 octets au plus), versé au journal d'audit : voir `motifContrePassation`. */
	motif: string;
}

export function contrePasser(
	api: ClientApi,
	cabinet: Uuid,
	dossier: Uuid,
	ecriture: Uuid,
	corps: CorpsContrePassation
): Promise<ContrePassation> {
	return api.muter(
		'POST',
		`${base(cabinet, dossier)}/ecritures/${ecriture}/contre-passation`,
		estContrePassation,
		corps
	);
}
