// Saisie d'une version de paramètres fiscaux propre à un dossier (ADR 0030) : contrôles de forme
// de l'API repris pour le confort de saisie, et corps de la requête. Module pur, testé sous
// `node --test`.
//
// L'interface n'invente aucune règle fiscale : les rôles de taux, les régimes retirables et leurs
// libellés viennent de la version applicable renvoyée par l'API ; les contrôles ci-dessous ne sont
// que ceux de la forme et de l'état que l'API contrôle elle-même (le serveur décide, 422 et 409).

import type {
	Applicable,
	CorpsVersion,
	RegimeFiscal,
	TauxFiscal,
	VersionFiscale
} from './api/parametres-fiscaux.ts';
import { longueurOctets } from './api/texte.ts';
import { estDateIso } from './dates.ts';
import { chercher } from './i18n/index.ts';

/** Bornes de l'API (`parametres_fiscaux.rs`, `fiscal::motif_sur`, `VERSIONS_MAX`). */
export const MOTIF_MAX_OCTETS = 400;
export const VERSIONS_PROPRES_MAX = 50;
/** `date_effet` au plus 366 jours après le jour du dossier. */
export const DATE_EFFET_MAX_JOURS = 366;

/**
 * Vrai si le motif est un texte sûr, comme l'API (`fiscal::motif_sur`) : non blanc, au plus 400
 * octets UTF-8, sans caractère de contrôle (C0, C1, U+007F) ni caractère de contrôle
 * bidirectionnel (U+200E, U+200F, U+202A à U+202E, U+2066 à U+2069).
 */
export function motifSur(motif: string): boolean {
	return (
		motif.trim() !== '' &&
		longueurOctets(motif) <= MOTIF_MAX_OCTETS &&
		// eslint-disable-next-line no-control-regex
		!/[\u0000-\u001f\u007f-\u009f‎‏‪-‮⁦-⁩]/.test(motif)
	);
}

/**
 * Taux canonique de l'API (`TauxTva::parse`) : décimal sans signe ni exposant, au plus trois
 * chiffres entiers sans zéro de tête, au plus six décimales sans zéro final, dans ]0 ; 100].
 */
export function tauxCanonique(texte: string): boolean {
	const m = /^(\d{1,3})(?:\.(\d{1,6}))?$/.exec(texte);
	if (m === null) return false;
	const [, entier = '', fraction] = m;
	if (entier.length > 1 && entier.startsWith('0')) return false;
	if (fraction !== undefined && fraction.endsWith('0')) return false;
	const nul = /^0+$/.test(entier) && (fraction === undefined || /^0+$/.test(fraction));
	if (nul) return false;
	return Number(entier) < 100 || (entier === '100' && fraction === undefined);
}

/** Date `AAAA-MM-JJ` décalée de `jours` jours (calcul de calendrier, en UTC). */
export function ajouterJours(date: string, jours: number): string {
	const [a = 0, m = 1, j = 1] = date.split('-').map(Number);
	return new Date(Date.UTC(a, m - 1, j + jours)).toISOString().slice(0, 10);
}

/** Dernière date limite de `date_effet` pour un jour du dossier donné. */
export function dateEffetLimite(aujourdhui: string): string {
	return ajouterJours(aujourdhui, DATE_EFFET_MAX_JOURS);
}

/** Date d'effet de la dernière version propre du dossier, ou `null` s'il n'y en a pas. */
export function derniereDateEffetPropre(versions: readonly VersionFiscale[]): string | null {
	const propres = versions.filter((v) => v.portee === 'dossier').map((v) => v.date_effet);
	return propres.length === 0 ? null : propres.reduce((a, b) => (a > b ? a : b));
}

/** Nombre de versions propres du dossier. */
export function nombreVersionsPropres(versions: readonly VersionFiscale[]): number {
	return versions.filter((v) => v.portee === 'dossier').length;
}

/**
 * Régimes que l'interface propose de retirer : ceux de la version applicable qui ne sont pas
 * exonérés et n'ont aucun motif (l'API refuse les autres : 422 `parametres_regime_non_retirable`).
 */
export function regimesRetirables(a: Applicable): RegimeFiscal[] {
	const avecMotifs = new Set(a.motifs.map((m) => m.regime));
	return a.regimes.filter((r) => !r.exonere && !avecMotifs.has(r.code));
}

/** Ce que l'utilisateur saisit pour une version propre. */
export interface FormulaireVersion {
	dateEffet: string;
	/** `''` : sans date de fin. */
	dateFin: string;
	motif: string;
	/** Nouvelle valeur par rôle de taux (`''` : inchangé). */
	taux: Record<string, string>;
	/** Codes des régimes retirés. */
	retires: string[];
}

export function formulaireVersionVide(): FormulaireVersion {
	return { dateEffet: '', dateFin: '', motif: '', taux: {}, retires: [] };
}

export const CLES_PROBLEME_VERSION = [
	'date_effet_invalide',
	'date_effet_lointaine',
	'date_effet_anterieure',
	'date_fin_invalide',
	'date_fin_anterieure',
	'motif_invalide',
	'taux_invalide',
	'aucun_changement',
	'trop_de_versions',
	'aucune_version_applicable'
] as const;
export type CleProblemeVersion = (typeof CLES_PROBLEME_VERSION)[number];

export interface ProblemeVersion {
	champ: string;
	cle: CleProblemeVersion;
	/** Paramètres du message (date limite, nombre maximal…). */
	params?: Readonly<Record<string, string | number>>;
}

/** Ce que le contrôle de forme doit savoir de l'état du dossier. */
export interface EtatVersions {
	/** Jour du dossier, dans son fuseau (`AAAA-MM-JJ`). */
	aujourdhui: string;
	versions: readonly VersionFiscale[];
	/** Version applicable à la date d'effet (donne les rôles de taux et les régimes). */
	base: Applicable | null;
}

/** Taux qui diffèrent de la valeur actuelle de leur rôle (les champs vides sont ignorés). */
export function tauxChanges(
	f: FormulaireVersion,
	actuels: readonly TauxFiscal[]
): [string, string][] {
	return actuels.flatMap((t): [string, string][] => {
		const saisi = (f.taux[t.role] ?? '').trim();
		return saisi === '' || saisi === t.valeur ? [] : [[t.role, saisi]];
	});
}

/**
 * Contrôles de forme et d'état de la création, ceux que l'API refuse ensuite (confort : le
 * serveur décide). `[]` : la requête peut partir.
 */
export function diagnostiquerVersion(f: FormulaireVersion, etat: EtatVersions): ProblemeVersion[] {
	const problemes: ProblemeVersion[] = [];
	if (nombreVersionsPropres(etat.versions) >= VERSIONS_PROPRES_MAX) {
		problemes.push({
			champ: 'versions',
			cle: 'trop_de_versions',
			params: { max: VERSIONS_PROPRES_MAX }
		});
	}
	if (!estDateIso(f.dateEffet)) {
		problemes.push({ champ: 'date_effet', cle: 'date_effet_invalide' });
	} else {
		const limite = dateEffetLimite(etat.aujourdhui);
		const derniere = derniereDateEffetPropre(etat.versions);
		if (f.dateEffet > limite) {
			problemes.push({ champ: 'date_effet', cle: 'date_effet_lointaine', params: { limite } });
		} else if (derniere !== null && f.dateEffet <= derniere) {
			problemes.push({
				champ: 'date_effet',
				cle: 'date_effet_anterieure',
				params: { derniere }
			});
		}
	}
	if (f.dateFin !== '') {
		if (!estDateIso(f.dateFin)) problemes.push({ champ: 'date_fin', cle: 'date_fin_invalide' });
		else if (estDateIso(f.dateEffet) && f.dateFin < f.dateEffet) {
			problemes.push({ champ: 'date_fin', cle: 'date_fin_anterieure' });
		}
	}
	if (!motifSur(f.motif)) {
		problemes.push({ champ: 'motif', cle: 'motif_invalide', params: { max: MOTIF_MAX_OCTETS } });
	}
	if (etat.base === null || etat.base.version === null) {
		if (estDateIso(f.dateEffet)) {
			problemes.push({ champ: 'date_effet', cle: 'aucune_version_applicable' });
		}
	} else {
		for (const t of etat.base.taux) {
			const saisi = (f.taux[t.role] ?? '').trim();
			if (saisi !== '' && !tauxCanonique(saisi)) {
				problemes.push({ champ: `taux:${t.role}`, cle: 'taux_invalide' });
			}
		}
		if (tauxChanges(f, etat.base.taux).length === 0 && f.retires.length === 0) {
			problemes.push({ champ: 'changement', cle: 'aucun_changement' });
		}
	}
	return problemes;
}

/**
 * Corps de la requête, ou `null` si un contrôle échoue. Seuls les taux changés et les régimes
 * retirés partent ; les régimes retirés sont ceux que la version applicable propose.
 */
export function corpsVersion(f: FormulaireVersion, etat: EtatVersions): CorpsVersion | null {
	if (diagnostiquerVersion(f, etat).length > 0 || etat.base === null) return null;
	const taux = Object.fromEntries(tauxChanges(f, etat.base.taux));
	const offerts = new Set(regimesRetirables(etat.base).map((r) => r.code));
	const retires = f.retires.filter((c) => offerts.has(c));
	return {
		date_effet: f.dateEffet,
		...(f.dateFin === '' ? {} : { date_fin: f.dateFin }),
		motif: f.motif.trim(),
		...(Object.keys(taux).length === 0 ? {} : { taux }),
		...(retires.length === 0 ? {} : { regimes_retires: retires })
	};
}

/**
 * Vrai si la version prendrait effet à une date déjà atteinte : des écritures validées peuvent
 * alors y être datées, elles garderaient leur version enregistrée (deux taux dans une même
 * période). Le nombre exact n'est connu qu'après la création (avertissement de l'API).
 */
export function pourraitEtreRetroactive(dateEffet: string, aujourdhui: string): boolean {
	return estDateIso(dateEffet) && dateEffet <= aujourdhui;
}

/** Message d'un contrôle de forme, dans la langue courante. */
export function messageProblemeVersion(p: ProblemeVersion): string {
	return chercher(`parametres.probleme.${p.cle}`, p.params) ?? '';
}
