// Export CSV de l'édition affichée, côté navigateur : mise en forme des colonnes **reçues**, sans
// aucun calcul (le serveur a calculé les totaux et les soldes).
//
// Séparateur de colonnes, séparateur décimal et marque d'ordre des octets (BOM) sont des options ;
// leurs valeurs par défaut sont déduites du locale du dossier (virgule décimale et point-virgule en
// `fr`, `es`, `pt` ; point décimal et virgule en `en` ; BOM UTF-8 pour la lecture directe par
// Excel). Les montants restent exacts : chaîne décimale reçue, mise à l'échelle de la devise du
// dossier, aucun `number`. Toute cellule de texte est neutralisée contre l'injection de formules
// (OWASP « CSV injection ») : un texte qui commence par `=`, `+`, `-`, `@`, tabulation ou retour
// chariot est précédé d'une apostrophe. Module pur, testé sous `node --test`.

import type { Balance, Centralisateur, GrandLivre, Journaux, Rubrique } from './api/editions.ts';
import { estMontantChaine } from './api/editions.ts';
import type { EnteteEdition } from './api/editions.ts';
import { libelleColonne, libelleMois, normaliserZero } from './editions-affichage.ts';
import { traduire } from './i18n/index.ts';
import { formaterNombre } from './montants.ts';
import { localeDe, separateurs, type Regional } from './regional.ts';

/** Cellule : texte libre (neutralisé) ou montant décimal exact (reçu du serveur). */
export type Cellule = string | { montant: string };

export const SEPARATEURS_CSV = [';', ',', '\t'] as const;
export const SEPARATEURS_DECIMAUX = [',', '.'] as const;

export interface OptionsCsv {
	/** Séparateur de colonnes. */
	separateur: (typeof SEPARATEURS_CSV)[number];
	/** Séparateur décimal des montants. */
	separateurDecimal: (typeof SEPARATEURS_DECIMAUX)[number];
	/** Marque d'ordre des octets UTF-8 en tête du fichier. */
	bom: boolean;
	/** Échelle décimale des montants (celle de la devise du dossier). */
	decimales: number;
}

/**
 * Options avant qu'un dossier soit chargé : l'écran ne montre alors aucun formulaire, et les options
 * du dossier (locale, échelle de sa devise) les remplacent dès qu'il l'est.
 */
export const OPTIONS_CSV_SANS_DOSSIER: Readonly<OptionsCsv> = {
	separateur: ';',
	separateurDecimal: ',',
	bom: true,
	decimales: 0
};

/** Options par défaut d'un dossier : celles qu'attend un tableur réglé sur son locale. */
export function optionsCsvParDefaut(r: Regional): OptionsCsv {
	const decimal = separateurs(localeDe(r)).decimal === '.' ? '.' : ',';
	return {
		separateur: decimal === ',' ? ';' : ',',
		separateurDecimal: decimal,
		bom: true,
		decimales: r.decimales
	};
}

const FIN_DE_LIGNE = '\r\n';
const BOM = '﻿';
const DECLENCHEURS_FORMULE = /^[\s]*[=+\-@\t\r]/;

/** Texte neutralisé : apostrophe devant tout début de formule possible. */
export function neutraliser(texte: string): string {
	return DECLENCHEURS_FORMULE.test(texte) ? `'${texte}` : texte;
}

function proteger(texte: string, separateur: string): string {
	return texte.includes(separateur) || /["\r\n]/.test(texte)
		? `"${texte.replaceAll('"', '""')}"`
		: texte;
}

/** Une cellule en CSV. Un « montant » illisible est traité comme du texte (donc neutralisé). */
export function formaterCellule(cellule: Cellule, o: OptionsCsv): string {
	if (typeof cellule === 'string') return proteger(neutraliser(cellule), o.separateur);
	if (!estMontantChaine(cellule.montant)) {
		return proteger(neutraliser(cellule.montant), o.separateur);
	}
	return proteger(
		formaterNombre(cellule.montant, o.decimales, o.separateurDecimal, null),
		o.separateur
	);
}

/** Document CSV : lignes de cellules, fins de ligne `\r\n`, BOM en tête si demandé. */
export function versCsv(lignes: readonly (readonly Cellule[])[], o: OptionsCsv): string {
	return (
		(o.bom ? BOM : '') +
		lignes.map((l) => l.map((c) => formaterCellule(c, o)).join(o.separateur)).join(FIN_DE_LIGNE) +
		FIN_DE_LIGNE
	);
}

/** Exercice du dossier (code, début et fin), tel que le serveur le renvoie. */
export interface ExerciceCsv {
	code: string;
	debut: string;
	fin: string;
}

/** Ce que le fichier cite de l'entité : dossier, devise, exercices, options de mise en forme. */
export interface ContexteCsv {
	nomDossier: string;
	regional: Regional;
	exercices: readonly ExerciceCsv[];
	options: OptionsCsv;
}

const m = (montant: string): Cellule => ({ montant: normaliserZero(montant) });
const t = traduire;

/**
 * Lignes d'en-tête du fichier : dossier, devise, mode, exercice avec ses dates de début et de fin
 * (s'il est connu du dossier), période, brouillons exclus (si signalés), ligne vide.
 */
export function enteteCsv(e: EnteteEdition, c: ContexteCsv): Cellule[][] {
	const exclus = e.brouillons_exclus.desequilibres + e.brouillons_exclus.illisibles;
	const exercice = c.exercices.find((x) => x.code === e.exercice);
	return [
		[t('csv.dossier'), c.nomDossier],
		[t('csv.devise'), c.regional.devise],
		[t('csv.mode'), e.mode === 'provisoire' ? t('csv.mode.provisoire') : t('csv.mode.definitif')],
		exercice === undefined
			? [t('csv.exercice'), e.exercice]
			: [t('csv.exercice'), e.exercice, exercice.debut, exercice.fin],
		[t('csv.periode'), e.du, e.au],
		...(exclus > 0 ? [[t('csv.brouillons_exclus'), String(exclus)]] : []),
		['']
	];
}

export function csvBalance(b: Balance, c: ContexteCsv): string {
	const cle = b.regroupement === 'tronc' ? t('csv.tronc') : t('csv.compte');
	return versCsv(
		[
			...enteteCsv(b, c),
			[cle, ...b.colonnes.map(libelleColonne)],
			...b.lignes.map((l) => [l.cle, ...l.valeurs.map(m)]),
			[t('csv.totaux'), ...b.totaux.map(m)]
		],
		c.options
	);
}

export function csvGrandLivre(g: GrandLivre, c: ContexteCsv): string {
	const lignes: Cellule[][] = [
		...enteteCsv(g, c),
		[
			t('csv.compte'),
			t('csv.date'),
			t('csv.journal'),
			t('csv.numero'),
			t('csv.date_valeur'),
			t('csv.libelle'),
			t('csv.tiers'),
			t('csv.debit'),
			t('csv.credit'),
			t('csv.solde')
		]
	];
	for (const compte of g.comptes) {
		lignes.push([
			compte.compte,
			'',
			'',
			'',
			'',
			t('csv.solde_ouverture'),
			'',
			'',
			'',
			m(compte.solde_ouverture)
		]);
		for (const l of compte.lignes) {
			lignes.push([
				compte.compte,
				l.date,
				l.journal,
				l.numero ?? '',
				l.date_valeur_comptable ?? '',
				l.libelle,
				l.tiers ?? '',
				m(l.debit),
				m(l.credit),
				m(l.solde)
			]);
		}
		lignes.push([
			compte.compte,
			'',
			'',
			'',
			'',
			t('csv.totaux_solde_final'),
			'',
			m(compte.total_debit),
			m(compte.total_credit),
			m(compte.solde_final)
		]);
	}
	return versCsv(lignes, c.options);
}

export function csvJournaux(j: Journaux, c: ContexteCsv): string {
	const lignes: Cellule[][] = [
		...enteteCsv(j, c),
		[
			t('csv.journal'),
			t('csv.numero'),
			t('csv.date'),
			t('csv.date_valeur'),
			t('csv.libelle'),
			t('csv.compte'),
			t('csv.tiers'),
			t('csv.debit'),
			t('csv.credit')
		]
	];
	for (const bloc of j.journaux) {
		for (const e of bloc.ecritures) {
			for (const l of e.lignes) {
				lignes.push([
					bloc.journal,
					e.numero ?? '',
					e.date,
					e.date_valeur_comptable ?? '',
					e.libelle,
					l.compte,
					l.tiers ?? '',
					m(l.debit),
					m(l.credit)
				]);
			}
		}
		lignes.push([
			bloc.journal,
			'',
			'',
			'',
			t('csv.totaux_journal'),
			'',
			'',
			m(bloc.total_debit),
			m(bloc.total_credit)
		]);
	}
	return versCsv(lignes, c.options);
}

function rubrique(nom: string, r: Rubrique, c: ContexteCsv): Cellule[][] {
	const mois = (l: { annee: number; mois: number }) => libelleMois(l, c.regional);
	return [
		...r.par_journal.map((l) => [nom, mois(l), l.journal ?? '', '', m(l.debit), m(l.credit)]),
		...r.par_compte.map((l) => [
			t('csv.par_compte', { rubrique: nom }),
			mois(l),
			l.journal ?? '',
			l.compte ?? '',
			m(l.debit),
			m(l.credit)
		]),
		...r.par_mois.map((l) => [
			t('csv.total_du_mois', { rubrique: nom }),
			mois(l),
			'',
			'',
			m(l.debit),
			m(l.credit)
		]),
		[t('csv.total', { rubrique: nom }), '', '', '', m(r.total_debit), m(r.total_credit)]
	];
}

export function csvCentralisateur(centralisateur: Centralisateur, c: ContexteCsv): string {
	return versCsv(
		[
			...enteteCsv(centralisateur, c),
			[
				t('csv.rubrique'),
				t('csv.mois'),
				t('csv.journal'),
				t('csv.compte'),
				t('csv.debit'),
				t('csv.credit')
			],
			...rubrique(t('csv.mouvements'), centralisateur.mouvements, c),
			...rubrique(t('csv.a_nouveaux'), centralisateur.a_nouveaux, c)
		],
		c.options
	);
}

/** Nom de fichier sûr d'une édition (exercice vérifié : quatre chiffres). */
export function nomFichierEdition(nature: string, exercice: string, mode: string): string {
	const sur = (x: string) => x.replace(/[^A-Za-z0-9-]/g, '');
	return `${sur(nature)}-${sur(exercice)}-${sur(mode)}.csv`;
}
