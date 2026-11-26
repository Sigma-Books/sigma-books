// Contexte régional d'un dossier : pays, devise, échelle décimale, fuseau horaire et langue.
//
// Sigma Books sert toute la zone OHADA (XOF, XAF, KMF, GNF, CDF ; français, anglais, portugais,
// espagnol). Ces cinq réglages sont ceux du **dossier** : le serveur les renvoie avec le dossier
// (`pays`, `devise`, `decimales`, `fuseau`, `langue`). Tant qu'un champ est absent (ancienne
// version de l'API), la valeur de {@link REGIONAL_DEFAUT} le remplace, champ par champ : ce
// module est le seul endroit qui connaît ces valeurs par défaut. Aucune logique comptable ici :
// seulement des réglages d'affichage et de lecture. Module pur, testé sous `node --test`.

/** Langues de l'interface. Le catalogue français est complet ; les autres retombent sur lui. */
export const LANGUES = ['fr', 'en', 'pt', 'es'] as const;
export type Langue = (typeof LANGUES)[number];

export interface Regional {
	/** Code pays ISO 3166-1 alpha-2 (`SN`). */
	pays: string;
	/** Code devise ISO 4217 (`XOF`). */
	devise: string;
	/** Échelle décimale de la devise (0 pour XOF, XAF, KMF, GNF ; 2 pour CDF). */
	decimales: number;
	/** Fuseau horaire IANA (`Africa/Dakar`). */
	fuseau: string;
	langue: Langue;
}

/** Valeurs d'un dossier dont l'API ne renvoie pas (encore) le contexte régional. */
export const REGIONAL_DEFAUT: Readonly<Regional> = {
	pays: 'SN',
	devise: 'XOF',
	decimales: 0,
	fuseau: 'Africa/Dakar',
	langue: 'fr'
};

/**
 * Échelle décimale usuelle d'une devise (ISO 4217), pour le dossier dont l'API ne renvoie pas
 * `decimales` : les francs de la zone (XOF, XAF, KMF, GNF) n'ont pas de décimales, le franc
 * congolais (CDF) en a deux ; toute autre devise : deux.
 */
const ECHELLES_DEVISE: Readonly<Record<string, number>> = {
	XOF: 0,
	XAF: 0,
	KMF: 0,
	GNF: 0,
	CDF: 2
};
const ECHELLE_AUTRES_DEVISES = 2;

/** Échelle décimale usuelle de la devise (voir {@link ECHELLES_DEVISE}). */
export function echelleDeLaDevise(devise: string): number {
	return Object.hasOwn(ECHELLES_DEVISE, devise)
		? (ECHELLES_DEVISE[devise] ?? ECHELLE_AUTRES_DEVISES)
		: ECHELLE_AUTRES_DEVISES;
}

/** Échelle décimale maximale (celle de la saisie : voir `ECHELLE` de `montants.ts`). */
const DECIMALES_MAX = 6;

function fuseauValide(fuseau: string): boolean {
	try {
		new Intl.DateTimeFormat('fr', { timeZone: fuseau });
		return true;
	} catch {
		return false;
	}
}

function langueDe(valeur: unknown): Langue | null {
	if (typeof valeur !== 'string') return null;
	const primaire = valeur.toLowerCase().split(/[-_]/)[0] ?? '';
	return (LANGUES as readonly string[]).includes(primaire) ? (primaire as Langue) : null;
}

/**
 * Contexte régional lu dans la réponse d'un dossier (ou dans tout objet). Chaque champ absent ou
 * de forme invalide prend la valeur par défaut ; rien n'est jamais refusé.
 */
export function lireRegional(brut: unknown): Regional {
	const o = typeof brut === 'object' && brut !== null ? (brut as Record<string, unknown>) : {};
	const { pays, devise, decimales, fuseau } = o;
	const codeDevise =
		typeof devise === 'string' && /^[A-Z]{3}$/.test(devise) ? devise : REGIONAL_DEFAUT.devise;
	return {
		pays: typeof pays === 'string' && /^[A-Z]{2}$/.test(pays) ? pays : REGIONAL_DEFAUT.pays,
		devise: codeDevise,
		// Sans `decimales` valide, l'échelle est celle de la devise : zéro pour un dossier XOF, deux
		// pour un dossier CDF (jamais zéro « par défaut » pour une devise qui a des décimales).
		decimales:
			typeof decimales === 'number' &&
			Number.isInteger(decimales) &&
			decimales >= 0 &&
			decimales <= DECIMALES_MAX
				? decimales
				: echelleDeLaDevise(codeDevise),
		fuseau: typeof fuseau === 'string' && fuseauValide(fuseau) ? fuseau : REGIONAL_DEFAUT.fuseau,
		langue: langueDe(o.langue) ?? REGIONAL_DEFAUT.langue
	};
}

/**
 * Contexte de l'utilisateur hors dossier (sessions, méthodes de connexion) : langue et fuseau du
 * navigateur quand ils sont connus, sinon les valeurs par défaut.
 */
export function regionalUtilisateur(): Regional {
	let fuseau: string | undefined;
	try {
		fuseau = new Intl.DateTimeFormat().resolvedOptions().timeZone;
	} catch {
		fuseau = undefined;
	}
	const navigateur = (globalThis as { navigator?: { language?: string } }).navigator;
	return lireRegional({ fuseau, langue: navigateur?.language });
}

/** Locale BCP 47 d'affichage (`fr-SN`) : la langue du dossier dans les usages de son pays. */
export function localeDe(r: Pick<Regional, 'langue' | 'pays'>): string {
	try {
		return Intl.getCanonicalLocales(`${r.langue}-${r.pays}`)[0] ?? r.langue;
	} catch {
		return r.langue;
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
