// Libellés et mises en forme des éditions : aucune logique comptable (les totaux, soldes et
// l'équilibre viennent du serveur). Module pur, testé sous `node --test`.

import { formaterMontant } from './montants.ts';
import type {
	AlerteBrouillard,
	AlerteEcheance,
	Balance,
	Centralisateur,
	FormatBalance,
	MoisCentralisateur,
	NatureEdition
} from './api/editions.ts';

export const LIBELLES_NATURE: Readonly<Record<NatureEdition, string>> = {
	balance: 'Balance',
	'grand-livre': 'Grand livre',
	journaux: 'Journaux',
	centralisateur: 'Centralisateur'
};

const LIBELLES_COLONNES: Readonly<Record<string, string>> = {
	ouverture_debit: 'Ouverture débit',
	ouverture_credit: 'Ouverture crédit',
	mouvements_debit: 'Mouvements débit',
	mouvements_credit: 'Mouvements crédit',
	total_debit: 'Total débit',
	total_credit: 'Total crédit',
	solde_debiteur: 'Solde débiteur',
	solde_crediteur: 'Solde créditeur'
};

/** Libellé d'une colonne de balance ; un nom inconnu est rendu tel quel. */
export function libelleColonne(nom: string): string {
	return Object.hasOwn(LIBELLES_COLONNES, nom) ? (LIBELLES_COLONNES[nom] ?? nom) : nom;
}

export const LIBELLES_FORMAT: Readonly<Record<FormatBalance, string>> = {
	2: 'Balance de travail à 2 colonnes (soldes)',
	4: 'Balance de travail à 4 colonnes (totaux et soldes)',
	6: 'Balance générale à 6 colonnes (AUDCIF art. 19)',
	8: 'Balance de travail à 8 colonnes'
};

export function titreBalance(b: Pick<Balance, 'format'>): string {
	return LIBELLES_FORMAT[b.format];
}

const MOIS = [
	'janvier',
	'février',
	'mars',
	'avril',
	'mai',
	'juin',
	'juillet',
	'août',
	'septembre',
	'octobre',
	'novembre',
	'décembre'
];

/** « mars 2026 » ; un mois hors 1 à 12 est rendu en nombre. */
export function libelleMois(m: Pick<MoisCentralisateur, 'annee' | 'mois'>): string {
	const nom = MOIS[m.mois - 1];
	return nom === undefined ? `${m.annee}-${m.mois}` : `${nom} ${m.annee}`;
}

export function messageBrouillonsExclus(exclus: {
	desequilibres: number;
	illisibles: number;
}): string | null {
	const total = exclus.desequilibres + exclus.illisibles;
	if (total === 0) return null;
	const details = [
		exclus.desequilibres > 0 ? `${exclus.desequilibres} déséquilibré(s)` : null,
		exclus.illisibles > 0 ? `${exclus.illisibles} illisible(s)` : null
	]
		.filter((x) => x !== null)
		.join(', ');
	return `Édition provisoire incomplète : ${total} brouillon(s) n’ont pas pu être pris en compte (${details}).`;
}

const STATUTS: Readonly<Record<string, string>> = {
	ouverte: 'ouverte',
	verrouillee: 'verrouillée',
	cloturee: 'clôturée'
};

export function libelleStatutPeriode(statut: string): string {
	return Object.hasOwn(STATUTS, statut) ? (STATUTS[statut] ?? statut) : statut;
}

const pluriel = (n: number, mot: string): string => `${n} ${mot}${Math.abs(n) > 1 ? 's' : ''}`;

export function messageAlerteBrouillard(a: AlerteBrouillard): string {
	return `Journal ${a.journal} : ${pluriel(a.nombre, 'brouillon')} non validé(s) pour la période du ${a.debut_periode} au ${a.fin_periode}, en retard de ${pluriel(a.jours_de_retard, 'jour')} (le plus ancien date du ${a.plus_ancienne}).`;
}

export function messageAlerteEcheance(a: AlerteEcheance): string {
	const periode = `période du ${a.debut} au ${a.fin} (exercice ${a.exercice}, ${libelleStatutPeriode(a.statut)})`;
	if (a.depassee) {
		return `Échéance de clôture dépassée de ${pluriel(-a.jours_restants, 'jour')} pour la ${periode} : échéance le ${a.echeance}.`;
	}
	return `Échéance de clôture le ${a.echeance}, dans ${pluriel(a.jours_restants, 'jour')}, pour la ${periode}.`;
}

/** Montant à afficher ou exporter : « -0 » (ou « -0.00 ») devient « 0 ». Chaîne exacte, sans calcul. */
export function normaliserZero(montant: string): string {
	return /^-0(\.0+)?$/.test(montant) ? '0' : montant;
}

/** Vrai si l'édition reçue ne contient aucune ligne (rien à afficher ni à exporter). */
export function editionVide(
	a:
		| { nature: 'balance'; edition: Pick<Balance, 'lignes'> }
		| { nature: 'grand-livre'; edition: { comptes: unknown[] } }
		| { nature: 'journaux'; edition: { journaux: unknown[] } }
		| { nature: 'centralisateur'; edition: Pick<Centralisateur, 'mouvements' | 'a_nouveaux'> }
): boolean {
	switch (a.nature) {
		case 'balance':
			return a.edition.lignes.length === 0;
		case 'grand-livre':
			return a.edition.comptes.length === 0;
		case 'journaux':
			return a.edition.journaux.length === 0;
		case 'centralisateur': {
			const { mouvements, a_nouveaux: an } = a.edition;
			return [mouvements, an].every(
				(r) => r.par_journal.length === 0 && r.par_compte.length === 0 && r.par_mois.length === 0
			);
		}
	}
}

/** Montant d'une édition pour l'affichage (groupes de milliers, « -0 » ramené à « 0 »). */
export const afficherMontant = (montant: string): string =>
	formaterMontant(normaliserZero(montant));
