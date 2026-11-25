// Éditions (balance, grand livre, journaux, centralisateur) et alertes du dossier : types, gardes,
// paramètres et appels (`crates/api/src/serveur/routes/editions.rs`). Lectures seules.
//
// Le serveur calcule tout (totaux, soldes, équilibre) : l'interface affiche les chaînes décimales
// exactes reçues, sans jamais les convertir en `number` (Règles absolues 2 et 4).

import type { ClientApi } from './client.ts';
import { chaines, entier, estObjet, tableauDe } from './gardes.ts';
import type { Garde, Uuid } from './types.ts';
import { estDateIso } from '../dates.ts';

export type ModeEdition = 'definitif' | 'provisoire';
export type FormatBalance = 2 | 4 | 6 | 8;
export type Regroupement = 'compte' | 'tronc';
export type NatureEdition = 'balance' | 'grand-livre' | 'journaux' | 'centralisateur';

export const NATURES_EDITION: readonly NatureEdition[] = [
	'balance',
	'grand-livre',
	'journaux',
	'centralisateur'
];
export const FORMATS_BALANCE: readonly FormatBalance[] = [2, 4, 6, 8];

export interface BrouillonsExclus {
	desequilibres: number;
	illisibles: number;
}

export interface EnteteEdition {
	exercice: string;
	du: string;
	au: string;
	mode: ModeEdition;
	/** Brouillons que le mode provisoire n'a pas pu prendre en compte (zéros en définitif). */
	brouillons_exclus: BrouillonsExclus;
}

export interface LigneBalance {
	cle: string;
	valeurs: string[];
}

export interface Balance extends EnteteEdition {
	format: FormatBalance;
	regroupement: Regroupement;
	colonnes: string[];
	lignes: LigneBalance[];
	totaux: string[];
	equilibree: boolean;
}

export interface LigneGrandLivre {
	date: string;
	journal: string;
	numero: string | null;
	date_valeur_comptable: string | null;
	libelle: string;
	tiers: string | null;
	debit: string;
	credit: string;
	solde: string;
}

export interface CompteGrandLivre {
	compte: string;
	solde_ouverture: string;
	lignes: LigneGrandLivre[];
	total_debit: string;
	total_credit: string;
	solde_final: string;
}

export interface GrandLivre extends EnteteEdition {
	comptes: CompteGrandLivre[];
}

export interface LigneJournal {
	compte: string;
	debit: string;
	credit: string;
	tiers: string | null;
}

export interface EcritureJournal {
	numero: string | null;
	date: string;
	date_valeur_comptable: string | null;
	libelle: string;
	lignes: LigneJournal[];
}

export interface Journal {
	journal: string;
	ecritures: EcritureJournal[];
	total_debit: string;
	total_credit: string;
}

export interface Journaux extends EnteteEdition {
	journaux: Journal[];
}

export interface MoisCentralisateur {
	annee: number;
	mois: number;
	journal: string | null;
	compte: string | null;
	debit: string;
	credit: string;
}

export interface Rubrique {
	par_journal: MoisCentralisateur[];
	par_compte: MoisCentralisateur[];
	par_mois: MoisCentralisateur[];
	total_debit: string;
	total_credit: string;
}

export interface Centralisateur extends EnteteEdition {
	mouvements: Rubrique;
	a_nouveaux: Rubrique;
}

export interface AlerteBrouillard {
	journal: string;
	debut_periode: string;
	fin_periode: string;
	nombre: number;
	plus_ancienne: string;
	jours_de_retard: number;
}

export interface AlerteEcheance {
	exercice: string;
	debut: string;
	fin: string;
	echeance: string;
	/** Négatif : l'échéance est dépassée. */
	jours_restants: number;
	depassee: boolean;
	statut: string;
}

export interface Alertes {
	date_du_jour: string;
	brouillard_non_valide: AlerteBrouillard[];
	echeances_de_cloture: AlerteEcheance[];
}

// --- Gardes -----------------------------------------------------------------------------------

/** Montant décimal exact en chaîne (jamais un nombre JSON) : `-12`, `1500.5`. */
export function estMontantChaine(valeur: unknown): valeur is string {
	return typeof valeur === 'string' && /^-?\d{1,30}(\.\d{1,12})?$/.test(valeur);
}

const montants = (o: Record<string, unknown>, champs: readonly string[]): boolean =>
	champs.every((c) => estMontantChaine(o[c]));

const chaineOuNul = (v: unknown): boolean => v === null || typeof v === 'string';

const estExclus = (v: unknown): v is BrouillonsExclus =>
	estObjet(v) && entier(v.desequilibres) && entier(v.illisibles);

const estEntete = (v: Record<string, unknown>): boolean =>
	chaines(v, ['exercice', 'du', 'au']) &&
	(v.mode === 'definitif' || v.mode === 'provisoire') &&
	estExclus(v.brouillons_exclus);

const estTableauDeChaines = (v: unknown): v is string[] =>
	tableauDe(v, (x): x is string => typeof x === 'string');
const estTableauMontants = (v: unknown): v is string[] => tableauDe(v, estMontantChaine);

export const estBalance: Garde<Balance> = (v): v is Balance =>
	estObjet(v) &&
	estEntete(v) &&
	(v.format === 2 || v.format === 4 || v.format === 6 || v.format === 8) &&
	(v.regroupement === 'compte' || v.regroupement === 'tronc') &&
	estTableauDeChaines(v.colonnes) &&
	v.colonnes.length === v.format &&
	tableauDe(
		v.lignes,
		(l): l is LigneBalance =>
			estObjet(l) &&
			typeof l.cle === 'string' &&
			estTableauMontants(l.valeurs) &&
			l.valeurs.length === v.format
	) &&
	estTableauMontants(v.totaux) &&
	v.totaux.length === v.format &&
	typeof v.equilibree === 'boolean';

const estLigneGrandLivre = (l: unknown): l is LigneGrandLivre =>
	estObjet(l) &&
	chaines(l, ['date', 'journal', 'libelle']) &&
	chaineOuNul(l.numero) &&
	chaineOuNul(l.date_valeur_comptable) &&
	chaineOuNul(l.tiers) &&
	montants(l, ['debit', 'credit', 'solde']);

const estCompteGrandLivre = (c: unknown): c is CompteGrandLivre =>
	estObjet(c) &&
	typeof c.compte === 'string' &&
	montants(c, ['solde_ouverture', 'total_debit', 'total_credit', 'solde_final']) &&
	tableauDe(c.lignes, estLigneGrandLivre);

export const estGrandLivre: Garde<GrandLivre> = (v): v is GrandLivre =>
	estObjet(v) && estEntete(v) && tableauDe(v.comptes, estCompteGrandLivre);

const estLigneJournal = (l: unknown): l is LigneJournal =>
	estObjet(l) &&
	typeof l.compte === 'string' &&
	montants(l, ['debit', 'credit']) &&
	chaineOuNul(l.tiers);

const estEcritureJournal = (e: unknown): e is EcritureJournal =>
	estObjet(e) &&
	chaines(e, ['date', 'libelle']) &&
	chaineOuNul(e.numero) &&
	chaineOuNul(e.date_valeur_comptable) &&
	tableauDe(e.lignes, estLigneJournal);

const estJournal = (j: unknown): j is Journal =>
	estObjet(j) &&
	typeof j.journal === 'string' &&
	montants(j, ['total_debit', 'total_credit']) &&
	tableauDe(j.ecritures, estEcritureJournal);

export const estJournaux: Garde<Journaux> = (v): v is Journaux =>
	estObjet(v) && estEntete(v) && tableauDe(v.journaux, estJournal);

const estMois = (m: unknown): m is MoisCentralisateur =>
	estObjet(m) &&
	entier(m.annee) &&
	entier(m.mois) &&
	chaineOuNul(m.journal) &&
	chaineOuNul(m.compte) &&
	montants(m, ['debit', 'credit']);

const estRubrique = (r: unknown): r is Rubrique =>
	estObjet(r) &&
	tableauDe(r.par_journal, estMois) &&
	tableauDe(r.par_compte, estMois) &&
	tableauDe(r.par_mois, estMois) &&
	montants(r, ['total_debit', 'total_credit']);

export const estCentralisateur: Garde<Centralisateur> = (v): v is Centralisateur =>
	estObjet(v) && estEntete(v) && estRubrique(v.mouvements) && estRubrique(v.a_nouveaux);

export const estAlertes: Garde<Alertes> = (v): v is Alertes =>
	estObjet(v) &&
	typeof v.date_du_jour === 'string' &&
	tableauDe(
		v.brouillard_non_valide,
		(a): a is AlerteBrouillard =>
			estObjet(a) &&
			chaines(a, ['journal', 'debut_periode', 'fin_periode', 'plus_ancienne']) &&
			entier(a.nombre) &&
			entier(a.jours_de_retard)
	) &&
	tableauDe(
		v.echeances_de_cloture,
		(a): a is AlerteEcheance =>
			estObjet(a) &&
			chaines(a, ['exercice', 'debut', 'fin', 'echeance', 'statut']) &&
			entier(a.jours_restants) &&
			typeof a.depassee === 'boolean'
	);

// --- Paramètres ---------------------------------------------------------------------------------

/** Filtre saisi à l'écran (champs vides : valeur par défaut du serveur). */
export interface FiltreEdition {
	exercice: string;
	du: string;
	au: string;
	mode: ModeEdition;
	/** Numéros séparés par des virgules ou des espaces. */
	comptes: string;
	/** Codes séparés par des virgules ou des espaces. */
	journaux: string;
	format: FormatBalance;
	regroupement: Regroupement;
}

export const FILTRE_VIDE: FiltreEdition = {
	exercice: '',
	du: '',
	au: '',
	mode: 'definitif',
	comptes: '',
	journaux: '',
	format: 6,
	regroupement: 'compte'
};

/** Nombre maximal d'éléments d'une liste de filtre (`LISTE_MAX` du serveur). */
export const LISTE_MAX = 50;

const MOTIF_COMPTE = /^[1-9][0-9A-Za-z-]{0,31}$/;
const MOTIF_JOURNAL = /^[A-Z0-9]{1,16}$/;
const MOTIF_EXERCICE = /^\d{4}$/;

/** Éléments d'une liste saisie (virgules et espaces), sans doublon, dans l'ordre. */
export function lireListe(saisie: string): string[] {
	return [...new Set(saisie.split(/[\s,;]+/).filter((x) => x !== ''))];
}

export type ProblemeFiltre =
	| 'exercice'
	| 'du'
	| 'au'
	| 'bornes_inversees'
	| 'comptes'
	| 'journaux'
	| 'trop_de_comptes'
	| 'trop_de_journaux';

export const MESSAGES_FILTRE: Readonly<Record<ProblemeFiltre, string>> = {
	exercice: 'Le code de l’exercice est une année à quatre chiffres (2026).',
	du: 'La date de début n’est pas une date valide (AAAA-MM-JJ).',
	au: 'La date de fin n’est pas une date valide (AAAA-MM-JJ).',
	bornes_inversees: 'La date de début est postérieure à la date de fin.',
	comptes: 'Un numéro de compte du filtre est illisible.',
	journaux: 'Un code de journal du filtre est illisible (majuscules et chiffres).',
	trop_de_comptes: `Au plus ${LISTE_MAX} comptes dans le filtre.`,
	trop_de_journaux: `Au plus ${LISTE_MAX} journaux dans le filtre.`
};

/**
 * Contrôles de confort du filtre (le serveur valide de toute façon : 400 `requete_invalide`).
 * Les journaux n'ont pas de filtre de comptes (une écriture figure en entier dans son journal).
 */
export function verifierFiltre(f: FiltreEdition, nature: NatureEdition): ProblemeFiltre[] {
	const problemes: ProblemeFiltre[] = [];
	const exercice = f.exercice.trim();
	if (exercice !== '' && !MOTIF_EXERCICE.test(exercice)) problemes.push('exercice');
	const du = f.du.trim();
	const au = f.au.trim();
	if (du !== '' && !estDateIso(du)) problemes.push('du');
	if (au !== '' && !estDateIso(au)) problemes.push('au');
	if (du !== '' && au !== '' && estDateIso(du) && estDateIso(au) && du > au) {
		problemes.push('bornes_inversees');
	}
	if (nature !== 'journaux') {
		const comptes = lireListe(f.comptes);
		if (comptes.length > LISTE_MAX) problemes.push('trop_de_comptes');
		else if (!comptes.every((c) => MOTIF_COMPTE.test(c))) problemes.push('comptes');
	}
	const journaux = lireListe(f.journaux);
	if (journaux.length > LISTE_MAX) problemes.push('trop_de_journaux');
	else if (!journaux.every((j) => MOTIF_JOURNAL.test(j))) problemes.push('journaux');
	return problemes;
}

/**
 * Chaîne de requête de l'édition (sans `?`), limitée aux paramètres admis par le serveur pour
 * cette nature : un paramètre inconnu y serait refusé (400). Les valeurs sont encodées.
 */
export function parametresEdition(f: FiltreEdition, nature: NatureEdition): string {
	const p = new URLSearchParams();
	const exercice = f.exercice.trim();
	if (exercice !== '') p.set('exercice', exercice);
	if (f.du.trim() !== '') p.set('du', f.du.trim());
	if (f.au.trim() !== '') p.set('au', f.au.trim());
	if (f.mode === 'provisoire') p.set('mode', 'provisoire');
	if (nature !== 'journaux') {
		const comptes = lireListe(f.comptes);
		if (comptes.length > 0) p.set('comptes', comptes.join(','));
	}
	const journaux = lireListe(f.journaux);
	if (journaux.length > 0) p.set('journaux', journaux.join(','));
	if (nature === 'balance') {
		if (f.format !== 6) p.set('format', String(f.format));
		if (f.regroupement !== 'compte') p.set('regroupement', f.regroupement);
	}
	return p.toString();
}

// --- Appels -------------------------------------------------------------------------------------

const base = (cabinet: Uuid, dossier: Uuid) => `/api/cabinets/${cabinet}/dossiers/${dossier}`;

function chemin(cabinet: Uuid, dossier: Uuid, nature: NatureEdition, f: FiltreEdition): string {
	const requete = parametresEdition(f, nature);
	return `${base(cabinet, dossier)}/editions/${nature}${requete === '' ? '' : `?${requete}`}`;
}

export function lireBalance(api: ClientApi, c: Uuid, d: Uuid, f: FiltreEdition): Promise<Balance> {
	return api.lire(chemin(c, d, 'balance', f), estBalance);
}

export function lireGrandLivre(
	api: ClientApi,
	c: Uuid,
	d: Uuid,
	f: FiltreEdition
): Promise<GrandLivre> {
	return api.lire(chemin(c, d, 'grand-livre', f), estGrandLivre);
}

export function lireJournaux(
	api: ClientApi,
	c: Uuid,
	d: Uuid,
	f: FiltreEdition
): Promise<Journaux> {
	return api.lire(chemin(c, d, 'journaux', f), estJournaux);
}

export function lireCentralisateur(
	api: ClientApi,
	c: Uuid,
	d: Uuid,
	f: FiltreEdition
): Promise<Centralisateur> {
	return api.lire(chemin(c, d, 'centralisateur', f), estCentralisateur);
}

export function lireAlertes(api: ClientApi, cabinet: Uuid, dossier: Uuid): Promise<Alertes> {
	return api.lire(`${base(cabinet, dossier)}/alertes`, estAlertes);
}
