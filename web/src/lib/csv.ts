// Export CSV de l'édition affichée, côté navigateur : mise en forme des colonnes **reçues**, sans
// aucun calcul (le serveur a calculé les totaux et les soldes). Séparateur `;` et BOM UTF-8 (lecture
// directe par Excel en français). Les montants restent exacts : chaîne décimale reçue, virgule
// décimale, aucun `number`. Toute cellule de texte est neutralisée contre l'injection de formules
// (OWASP « CSV injection ») : un texte qui commence par `=`, `+`, `-`, `@`, tabulation ou retour
// chariot est précédé d'une apostrophe. Module pur, testé sous `node --test`.

import type { Balance, Centralisateur, GrandLivre, Journaux, Rubrique } from './api/editions.ts';
import { estMontantChaine } from './api/editions.ts';
import { libelleColonne, libelleMois } from './editions-affichage.ts';

/** Cellule : texte libre (neutralisé) ou montant décimal exact (reçu du serveur). */
export type Cellule = string | { montant: string };

const SEPARATEUR = ';';
const FIN_DE_LIGNE = '\r\n';
const BOM = '﻿';
const DECLENCHEURS_FORMULE = /^[\s]*[=+\-@\t\r]/;

/** Texte neutralisé : apostrophe devant tout début de formule possible. */
export function neutraliser(texte: string): string {
	return DECLENCHEURS_FORMULE.test(texte) ? `'${texte}` : texte;
}

function proteger(texte: string): string {
	return /[;"\r\n]/.test(texte) ? `"${texte.replaceAll('"', '""')}"` : texte;
}

/** Une cellule en CSV. Un « montant » illisible est traité comme du texte (donc neutralisé). */
export function formaterCellule(cellule: Cellule): string {
	if (typeof cellule === 'string') return proteger(neutraliser(cellule));
	if (!estMontantChaine(cellule.montant)) return proteger(neutraliser(cellule.montant));
	return cellule.montant.replace('.', ',');
}

/** Document CSV : lignes de cellules, fins de ligne `\r\n`, BOM en tête. */
export function versCsv(lignes: readonly (readonly Cellule[])[]): string {
	return (
		BOM +
		lignes.map((l) => l.map(formaterCellule).join(SEPARATEUR)).join(FIN_DE_LIGNE) +
		FIN_DE_LIGNE
	);
}

const m = (montant: string): Cellule => ({ montant });

export function csvBalance(b: Balance): string {
	const cle = b.regroupement === 'tronc' ? 'Tronc' : 'Compte';
	return versCsv([
		[cle, ...b.colonnes.map(libelleColonne)],
		...b.lignes.map((l) => [l.cle, ...l.valeurs.map(m)]),
		['Totaux', ...b.totaux.map(m)]
	]);
}

export function csvGrandLivre(g: GrandLivre): string {
	const lignes: Cellule[][] = [
		[
			'Compte',
			'Date',
			'Journal',
			'Numéro',
			'Date de valeur',
			'Libellé',
			'Tiers',
			'Débit',
			'Crédit',
			'Solde'
		]
	];
	for (const c of g.comptes) {
		lignes.push([c.compte, '', '', '', '', 'Solde d’ouverture', '', '', '', m(c.solde_ouverture)]);
		for (const l of c.lignes) {
			lignes.push([
				c.compte,
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
			c.compte,
			'',
			'',
			'',
			'',
			'Totaux et solde final',
			'',
			m(c.total_debit),
			m(c.total_credit),
			m(c.solde_final)
		]);
	}
	return versCsv(lignes);
}

export function csvJournaux(j: Journaux): string {
	const lignes: Cellule[][] = [
		['Journal', 'Numéro', 'Date', 'Date de valeur', 'Libellé', 'Compte', 'Tiers', 'Débit', 'Crédit']
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
			'Totaux du journal',
			'',
			'',
			m(bloc.total_debit),
			m(bloc.total_credit)
		]);
	}
	return versCsv(lignes);
}

function rubrique(nom: string, r: Rubrique): Cellule[][] {
	return [
		...r.par_journal.map((l) => [
			nom,
			libelleMois(l),
			l.journal ?? '',
			'',
			m(l.debit),
			m(l.credit)
		]),
		...r.par_compte.map((l) => [
			`${nom} (par compte)`,
			libelleMois(l),
			l.journal ?? '',
			l.compte ?? '',
			m(l.debit),
			m(l.credit)
		]),
		...r.par_mois.map((l) => [
			`${nom} (total du mois)`,
			libelleMois(l),
			'',
			'',
			m(l.debit),
			m(l.credit)
		]),
		[`${nom} (total)`, '', '', '', m(r.total_debit), m(r.total_credit)]
	];
}

export function csvCentralisateur(c: Centralisateur): string {
	return versCsv([
		['Rubrique', 'Mois', 'Journal', 'Compte', 'Débit', 'Crédit'],
		...rubrique('Mouvements', c.mouvements),
		...rubrique('À-nouveaux', c.a_nouveaux)
	]);
}

/** Nom de fichier sûr d'une édition (exercice vérifié : quatre chiffres). */
export function nomFichierEdition(nature: string, exercice: string, mode: string): string {
	const sur = (x: string) => x.replace(/[^A-Za-z0-9-]/g, '');
	return `${sur(nature)}-${sur(exercice)}-${sur(mode)}.csv`;
}
