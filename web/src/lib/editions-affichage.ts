// Libellés (catalogue `i18n/`) et mises en forme des éditions : aucune logique comptable (les totaux, soldes et
// l'équilibre viennent du serveur). Module pur, testé sous `node --test`.

import { afficherDate } from './dates.ts';
import { chercher, nombreEtUnite, traduire } from './i18n/index.ts';
import { formaterMontant } from './montants.ts';
import { localeDe, type Regional } from './regional.ts';
import type {
	AlerteBrouillard,
	AlerteEcheance,
	Balance,
	Centralisateur,
	FormatBalance,
	MoisCentralisateur,
	NatureEdition
} from './api/editions.ts';

/** Libellé d'une nature d'édition. */
export const libelleEdition = (n: NatureEdition): string => traduire(`edition.nature.${n}`);

/** Libellé d'une colonne de balance ; un nom inconnu est rendu tel quel. */
export function libelleColonne(nom: string): string {
	return chercher(`edition.colonne.${nom}`) ?? nom;
}

export const libelleFormat = (f: FormatBalance): string => traduire(`edition.format.${f}`);

export function titreBalance(b: Pick<Balance, 'format'>): string {
	return libelleFormat(b.format);
}

const MOIS = new Map<string, Intl.DateTimeFormat>();

/**
 * « mars 2026 » : mois et année selon le locale du dossier (`Intl`, calendrier grégorien) ; un mois
 * hors 1 à 12 est rendu en nombre.
 */
export function libelleMois(
	m: Pick<MoisCentralisateur, 'annee' | 'mois'>,
	r: Pick<Regional, 'langue' | 'pays'>
): string {
	if (!Number.isInteger(m.mois) || m.mois < 1 || m.mois > 12 || !Number.isInteger(m.annee)) {
		return `${m.annee}-${m.mois}`;
	}
	const locale = localeDe(r);
	let f = MOIS.get(locale);
	if (f === undefined) {
		f = new Intl.DateTimeFormat(locale, { month: 'long', year: 'numeric', timeZone: 'UTC' });
		MOIS.set(locale, f);
	}
	return f.format(new Date(Date.UTC(m.annee, m.mois - 1, 1)));
}

export function messageBrouillonsExclus(exclus: {
	desequilibres: number;
	illisibles: number;
}): string | null {
	const total = exclus.desequilibres + exclus.illisibles;
	if (total === 0) return null;
	const details = [
		exclus.desequilibres > 0
			? traduire('edition.exclus.desequilibres', { nombre: exclus.desequilibres })
			: null,
		exclus.illisibles > 0
			? traduire('edition.exclus.illisibles', { nombre: exclus.illisibles })
			: null
	]
		.filter((x) => x !== null)
		.join(', ');
	return traduire('edition.exclus', { total, details });
}

export function libelleStatutPeriode(statut: string): string {
	return chercher(`statut.${statut}`) ?? statut;
}

export function messageAlerteBrouillard(a: AlerteBrouillard, r: Regional): string {
	return traduire('alerte.brouillard', {
		journal: a.journal,
		brouillons: nombreEtUnite(a.nombre, 'brouillon'),
		debut: afficherDate(a.debut_periode, r),
		fin: afficherDate(a.fin_periode, r),
		retard: nombreEtUnite(a.jours_de_retard, 'jour'),
		ancienne: afficherDate(a.plus_ancienne, r)
	});
}

export function messageAlerteEcheance(a: AlerteEcheance, r: Regional): string {
	const periode = traduire('alerte.periode', {
		debut: afficherDate(a.debut, r),
		fin: afficherDate(a.fin, r),
		exercice: a.exercice,
		statut: libelleStatutPeriode(a.statut)
	});
	const echeance = afficherDate(a.echeance, r);
	return a.depassee
		? traduire('alerte.echeance_depassee', {
				jours: nombreEtUnite(-a.jours_restants, 'jour'),
				periode,
				echeance
			})
		: traduire('alerte.echeance_proche', {
				jours: nombreEtUnite(a.jours_restants, 'jour'),
				periode,
				echeance
			});
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

/**
 * Montant d'une édition pour l'affichage : devise et locale du dossier (échelle, séparateurs), « -0 »
 * ramené à « 0 ».
 */
export const afficherMontant = (montant: string, r: Regional): string =>
	formaterMontant(normaliserZero(montant), r);
