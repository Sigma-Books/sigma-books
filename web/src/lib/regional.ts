// Contexte régional d'un dossier : pays, devise, échelle décimale, fuseau horaire et langue.
//
// Sigma Books sert toute la zone OHADA (ADR 0027). Ces cinq réglages sont ceux du **dossier** : le
// serveur les renvoie avec lui (`pays`, `devise`, `decimales`, `fuseau`, `langue`) et ils sont
// **obligatoires** (`estDossier`) : l'interface n'en suppose aucun, une réponse qui en manque est
// une erreur de contrat. Aucune logique comptable ici : seulement des gardes de forme et des
// réglages d'affichage. Module pur, testé sous `node --test`.

/** Langues dont l'interface a un catalogue de messages (le français est complet, de référence). */
export const LANGUES = ['fr', 'en', 'pt', 'es'] as const;
export type Langue = (typeof LANGUES)[number];

/** Langue du catalogue de référence (complet) : celui où retombent les autres, avant tout dossier. */
export const LANGUE_DE_REFERENCE: Langue = 'fr';

export interface Regional {
	/** Code pays ISO 3166-1 alpha-2. */
	pays: string;
	/** Code devise de tenue ISO 4217. */
	devise: string;
	/** Échelle décimale de la devise de tenue, copiée du pack pays à la création du dossier. */
	decimales: number;
	/** Fuseau horaire IANA, celui où se calcule la date du jour du dossier. */
	fuseau: string;
	/** Langue du dossier (BCP 47, forme du serveur : `fr`, `pt-GW`). */
	langue: string;
}

/**
 * Réglages suffisants pour afficher une date ou une heure : ceux d'un dossier, ou seulement ceux
 * du navigateur hors dossier (le pays est alors facultatif).
 */
export type Affichage = Pick<Regional, 'langue' | 'fuseau'> & { pays?: string };

/** Échelle décimale maximale d'une devise de tenue (ADR 0027 : 0 à 4). */
export const DECIMALES_TENUE_MAX = 4;

/** Longueur maximale d'une langue BCP 47 (celle du serveur). */
const LANGUE_LONGUEUR_MAX = 35;

export function estPays(valeur: unknown): valeur is string {
	return typeof valeur === 'string' && /^[A-Z]{2}$/.test(valeur);
}

export function estDevise(valeur: unknown): valeur is string {
	return typeof valeur === 'string' && /^[A-Z]{3}$/.test(valeur);
}

export function estDecimales(valeur: unknown): valeur is number {
	return (
		typeof valeur === 'number' &&
		Number.isInteger(valeur) &&
		valeur >= 0 &&
		valeur <= DECIMALES_TENUE_MAX
	);
}

/** Vrai si `Intl` connaît ce nom de fuseau IANA (jamais d'une valeur hostile ou inventée). */
export function estFuseau(valeur: unknown): valeur is string {
	if (typeof valeur !== 'string' || valeur === '' || valeur.length > 64) return false;
	try {
		new Intl.DateTimeFormat('fr', { timeZone: valeur });
		return true;
	} catch {
		return false;
	}
}

/**
 * Forme d'une langue BCP 47 simplifiée, celle du serveur : sous-étiquettes alphanumériques de 1 à
 * 8 caractères séparées par `-`, la première de 2 ou 3 lettres minuscules (`fr`, `fr-SN`, `pt`).
 */
export function estLangue(valeur: unknown): valeur is string {
	if (typeof valeur !== 'string' || valeur.length > LANGUE_LONGUEUR_MAX) return false;
	const [premiere = '', ...reste] = valeur.split('-');
	return /^[a-z]{2,3}$/.test(premiere) && reste.every((p) => /^[A-Za-z0-9]{1,8}$/.test(p));
}

/** Langue dont l'interface a un catalogue (sous-étiquette primaire), ou `null`. */
export function langueSupportee(langue: string): Langue | null {
	const primaire = langue.toLowerCase().split(/[-_]/)[0] ?? '';
	return (LANGUES as readonly string[]).includes(primaire) ? (primaire as Langue) : null;
}

/** Les cinq réglages d'un dossier (la réponse de l'API, déjà vérifiée par `estDossier`). */
export function regionalDuDossier(d: Regional): Regional {
	return {
		pays: d.pays,
		devise: d.devise,
		decimales: d.decimales,
		fuseau: d.fuseau,
		langue: d.langue
	};
}

/**
 * Réglages de l'utilisateur hors dossier (sessions, méthodes de connexion, administration) : langue
 * et fuseau du navigateur. Sans fuseau lisible, UTC (affichage seulement, jamais une date comptable) ;
 * sans langue lisible, la langue du catalogue de référence de l'interface.
 */
export function regionalUtilisateur(): Affichage {
	let fuseau: string | undefined;
	try {
		fuseau = new Intl.DateTimeFormat().resolvedOptions().timeZone;
	} catch {
		fuseau = undefined;
	}
	const navigateur = (globalThis as { navigator?: { language?: string } }).navigator;
	const langue = navigateur?.language;
	return {
		langue: estLangue(langue) ? langue : LANGUE_DE_REFERENCE,
		fuseau: estFuseau(fuseau) ? fuseau : 'UTC'
	};
}

/**
 * Locale BCP 47 d'affichage (`fr-SN`) : la langue du dossier dans les usages de son pays, sauf si
 * elle porte déjà sa région (`pt-GW`) ou si le pays est inconnu (hors dossier).
 */
export function localeDe(r: Pick<Regional, 'langue'> & { pays?: string }): string {
	const [primaire = '', region] = r.langue.split('-');
	const regionPropre = region !== undefined && /^[A-Za-z]{2}$/.test(region);
	const candidat = regionPropre
		? r.langue
		: r.pays === undefined
			? primaire
			: `${primaire}-${r.pays}`;
	try {
		return Intl.getCanonicalLocales(candidat)[0] ?? primaire;
	} catch {
		return primaire;
	}
}

export interface Separateurs {
	decimal: string;
	groupe: string;
}

const CACHE = new Map<string, Separateurs>();

/** Séparateurs décimal et de milliers du locale (`,` et U+202F pour `fr`, `.` et `,` pour `en`). */
export function separateurs(locale: string): Separateurs {
	const connus = CACHE.get(locale);
	if (connus !== undefined) return connus;
	// `always` : certains locales (es) ne groupent pas avant cinq chiffres ; un livre comptable si.
	const parties = new Intl.NumberFormat(locale, { useGrouping: 'always' }).formatToParts(1234567.5);
	const trouve: Separateurs = {
		decimal: parties.find((p) => p.type === 'decimal')?.value ?? ',',
		groupe: parties.find((p) => p.type === 'group')?.value ?? ' '
	};
	CACHE.set(locale, trouve);
	return trouve;
}
