// Modèle d'une écriture en cours de saisie et corps de la requête de l'API.
//
// Aucune règle comptable ici : le module met en forme ce que l'utilisateur a tapé (montants en
// chaînes canoniques, lignes vides ignorées) et indique, à titre indicatif, l'équilibre. Le
// serveur décide de tout (comptes admis, équilibre, numéros) et renvoie des codes stables.

import { ecart, lireMontant, type Ecart } from '../montants.ts';

export interface LigneSaisie {
	compte: string;
	/** Identifiant du tiers choisi, ou chaîne vide. */
	tiers: string;
	debit: string;
	credit: string;
}

export interface EnteteSaisie {
	journal: string;
	/** `AAAA-MM-JJ`. */
	date: string;
	libelle: string;
	/** `AAAA-MM-JJ` ou chaîne vide. */
	dateValeur: string;
}

export const LIGNES_MAX = 500;

export function ligneVide(): LigneSaisie {
	return { compte: '', tiers: '', debit: '', credit: '' };
}

/** Vrai si rien n'a été saisi sur la ligne. */
export function estLigneVide(l: LigneSaisie): boolean {
	return l.compte.trim() === '' && l.tiers === '' && l.debit.trim() === '' && l.credit.trim() === '';
}

export interface CorpsLigne {
	compte: string;
	sens: 'debit' | 'credit';
	montant: string;
	tiers?: string;
}

export interface CorpsEcriture {
	version?: number;
	journal: string;
	date: string;
	libelle: string;
	date_valeur_comptable?: string;
	lignes: CorpsLigne[];
}

export type ProblemeLocal = 'montant_illisible' | 'debit_et_credit' | 'montant_absent' | 'compte_absent';

export type Construction =
	| {
			ok: true;
			corps: CorpsEcriture;
			/** `lignes[i]` du corps vient de la ligne `positions[i]` (à partir de 0) de la grille. */
			positions: number[];
	  }
	| { ok: false; probleme: ProblemeLocal; ligne: number };

/**
 * Corps de la requête : lignes vides ignorées, montants en chaînes canoniques. Ne refuse que ce
 * que le serveur ne pourrait pas lire (montant illisible, débit et crédit ensemble) ; tout le
 * reste (compte, équilibre, justification) est contrôlé par le serveur.
 */
export function corpsEcriture(
	entete: EnteteSaisie,
	lignes: readonly LigneSaisie[],
	version?: number
): Construction {
	const corpsLignes: CorpsLigne[] = [];
	const positions: number[] = [];
	for (const [i, l] of lignes.entries()) {
		if (estLigneVide(l)) continue;
		const d = l.debit.trim();
		const c = l.credit.trim();
		if (d !== '' && c !== '') return { ok: false, probleme: 'debit_et_credit', ligne: i };
		if (d === '' && c === '') return { ok: false, probleme: 'montant_absent', ligne: i };
		const brut = d === '' ? c : d;
		const montant = lireMontant(brut);
		if (montant === null) return { ok: false, probleme: 'montant_illisible', ligne: i };
		if (l.compte.trim() === '') return { ok: false, probleme: 'compte_absent', ligne: i };
		const ligne: CorpsLigne = {
			compte: l.compte.trim(),
			sens: d === '' ? 'credit' : 'debit',
			montant
		};
		if (l.tiers !== '') ligne.tiers = l.tiers;
		corpsLignes.push(ligne);
		positions.push(i);
	}
	const corps: CorpsEcriture = {
		journal: entete.journal,
		date: entete.date,
		libelle: entete.libelle.trim(),
		lignes: corpsLignes
	};
	if (entete.dateValeur !== '') corps.date_valeur_comptable = entete.dateValeur;
	if (version !== undefined) corps.version = version;
	return { ok: true, corps, positions };
}

/** Équilibre indicatif de la grille (les montants illisibles comptent pour zéro). */
export function equilibreIndicatif(lignes: readonly LigneSaisie[]): Ecart {
	return ecart(
		lignes.map((l) => l.debit),
		lignes.map((l) => l.credit)
	);
}

export interface Solde {
	champ: 'debit' | 'credit';
	montant: string;
}

/**
 * Solde proposé par `=` pour la ligne `index` : le montant qui équilibre les autres lignes, porté
 * sur le côté demandé par la colonne où l'utilisateur se trouve. `null` si rien à équilibrer.
 */
export function soldePropose(lignes: readonly LigneSaisie[], index: number): Solde | null {
	const autres = lignes.filter((_, i) => i !== index);
	const e = equilibreIndicatif(autres);
	if (e.sens === 'equilibre') return null;
	return { champ: e.sens, montant: e.difference };
}

/** Une ligne de la grille à partir de la position (à partir de 1) citée par le serveur. */
export function indexDepuisPosition(positions: readonly number[], positionServeur: number): number | null {
	return positions[positionServeur - 1] ?? null;
}
