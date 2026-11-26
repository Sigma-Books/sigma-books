// Saisie du contexte d'un dossier (ADR 0027) : état du formulaire, valeurs par défaut tirées du
// pack pays de l'API, contrôles de forme et corps des requêtes de création et de modification.
//
// L'interface ne décide de rien : les listes (pays, devises, fuseaux, types d'identifiants) et les
// valeurs par défaut de chaque pays viennent de l'API, les bornes de forme ci-dessous sont celles
// du serveur (`contexte_dossier.rs`), citées pour le confort de saisie ; le serveur contrôle de
// toute façon (422) et son refus est affiché. Module pur, testé sous `node --test`.

import type { CorpsCreationDossier, CorpsOuverture, PackPays, PaysPack } from './api/pays.ts';
import type { IdentifiantLegal } from './api/types.ts';
import { estDateIso } from './dates.ts';
import { estLangue } from './regional.ts';

/** Bornes du serveur (`crates/api/src/contexte_dossier.rs`, `organisation.rs`). */
export const IDENTIFIANTS_MAX = 10;
export const VALEUR_IDENTIFIANT_MAX = 64;
export const FORME_JURIDIQUE_MAX = 100;
export const NOM_DOSSIER_MAX = 200;

/** Contexte d'un dossier tel qu'on le saisit : tout est du texte, `''` pour « non renseigné ». */
export interface FormulaireContexte {
	pays: string;
	devise: string;
	fuseau: string;
	langue: string;
	formeJuridique: string;
	/** Valeur saisie de chaque type d'identifiant admis par le pays (`''` : aucun). */
	identifiants: Record<string, string>;
}

/** Ouverture du premier exercice, telle qu'on la saisit (création d'un dossier). */
export interface FormulaireOuverture {
	nature: 'creation' | 'reprise';
	/** `AAAA-MM-JJ`. */
	debutActivite: string;
	finPremier: 'annee_de_debut' | 'annee_suivante';
	/** Année de l'exercice repris (quatre chiffres). */
	annee: string;
}

export const CLES_PROBLEME = [
	'pays_absent',
	'pays_inconnu',
	'devise_inconnue',
	'fuseau_inconnu',
	'langue_invalide',
	'caractere_de_controle',
	'forme_trop_longue',
	'valeur_trop_longue',
	'trop_d_identifiants',
	'type_non_admis',
	'nom_absent',
	'nom_trop_long',
	'debut_invalide',
	'annee_invalide'
] as const;
export type CleProbleme = (typeof CLES_PROBLEME)[number];

/** Un champ refusé : `champ` est le nom du champ du formulaire (`identifiant:<code du type>` pour un type). */
export interface Probleme {
	champ: string;
	cle: CleProbleme;
}

/** Bornes citées dans les messages (`{max}`). */
export const BORNES_PROBLEMES: Readonly<Partial<Record<CleProbleme, number>>> = {
	forme_trop_longue: FORME_JURIDIQUE_MAX,
	valeur_trop_longue: VALEUR_IDENTIFIANT_MAX,
	trop_d_identifiants: IDENTIFIANTS_MAX,
	nom_trop_long: NOM_DOSSIER_MAX
};

// eslint-disable-next-line no-control-regex
const CONTROLE = /[\u0000-\u001f\u007f-\u009f]/;

const longueur = (texte: string): number => [...texte].length;

/** Le pays du pack, ou `undefined`. */
export function paysDuPack(pack: PackPays, code: string): PaysPack | undefined {
	return pack.pays.find((p) => p.code === code);
}

/** Formulaire sans aucun choix : le pays doit être choisi (il détermine les valeurs par défaut). */
export function formulaireVide(): FormulaireContexte {
	return { pays: '', devise: '', fuseau: '', langue: '', formeJuridique: '', identifiants: {} };
}

/**
 * Identifiants du formulaire pour les types admis par le pays : la valeur déjà saisie d'un type
 * encore admis est conservée, celle d'un type qui ne l'est plus disparaît.
 */
function identifiantsPour(
	pays: PaysPack | undefined,
	anciens: Readonly<Record<string, string>>
): Record<string, string> {
	const nouveaux: Record<string, string> = {};
	for (const t of pays?.identifiants ?? []) {
		nouveaux[t.code] = Object.hasOwn(anciens, t.code) ? (anciens[t.code] ?? '') : '';
	}
	return nouveaux;
}

/**
 * Choix d'un pays : devise de tenue, fuseau et langue prennent les valeurs par défaut **du pack**
 * pour ce pays, sauf celles que l'utilisateur a déjà changées de la valeur par défaut de
 * l'ancien pays (son choix explicite est conservé). Un code absent du pack vide les réglages.
 */
export function choisirPays(
	formulaire: FormulaireContexte,
	pack: PackPays,
	code: string
): FormulaireContexte {
	const ancien = paysDuPack(pack, formulaire.pays);
	const nouveau = paysDuPack(pack, code);
	const suit = (actuel: string, avant: string | undefined, apres: string | undefined): string => {
		if (apres === undefined) return '';
		return actuel === '' || actuel === avant ? apres : actuel;
	};
	return {
		...formulaire,
		pays: nouveau === undefined ? code : nouveau.code,
		devise: suit(formulaire.devise, ancien?.devise, nouveau?.devise),
		fuseau: suit(formulaire.fuseau, ancien?.fuseau, nouveau?.fuseau),
		langue: suit(formulaire.langue, ancien?.langue, nouveau?.langue),
		identifiants: identifiantsPour(nouveau, formulaire.identifiants)
	};
}

/** Ouverture par défaut : reprise de l'exercice de l'année de la date du jour (`AAAA-MM-JJ`) donnée. */
export function ouvertureParDefaut(aujourdhui: string): FormulaireOuverture {
	return {
		nature: 'reprise',
		debutActivite: aujourdhui,
		finPremier: 'annee_de_debut',
		annee: aujourdhui.slice(0, 4)
	};
}

/** Identifiants saisis, rognés, dans l'ordre des types du pack (les valeurs vides sont ignorées). */
export function identifiantsSaisis(
	formulaire: FormulaireContexte,
	pack: PackPays
): IdentifiantLegal[] {
	const ordre = (paysDuPack(pack, formulaire.pays)?.identifiants ?? []).map((t) => t.code);
	const types = [
		...ordre,
		...Object.keys(formulaire.identifiants).filter((t) => !ordre.includes(t))
	];
	return types.flatMap((type) => {
		const valeur = (formulaire.identifiants[type] ?? '').trim();
		return valeur === '' ? [] : [{ type, valeur }];
	});
}

function problemesTexte(texte: string, champ: string, max: number, trop: CleProbleme): Probleme[] {
	if (CONTROLE.test(texte)) return [{ champ, cle: 'caractere_de_controle' }];
	return longueur(texte) > max ? [{ champ, cle: trop }] : [];
}

/** Contrôles de forme du contexte (confort : le serveur contrôle de toute façon, 422). */
export function diagnostiquer(formulaire: FormulaireContexte, pack: PackPays): Probleme[] {
	const problemes: Probleme[] = [];
	const pays = paysDuPack(pack, formulaire.pays);
	if (formulaire.pays === '') problemes.push({ champ: 'pays', cle: 'pays_absent' });
	else if (pays === undefined) problemes.push({ champ: 'pays', cle: 'pays_inconnu' });
	if (!pack.devises.some((d) => d.code === formulaire.devise)) {
		problemes.push({ champ: 'devise', cle: 'devise_inconnue' });
	}
	if (!pack.fuseaux.some((f) => f.nom === formulaire.fuseau)) {
		problemes.push({ champ: 'fuseau', cle: 'fuseau_inconnu' });
	}
	if (!estLangue(formulaire.langue)) problemes.push({ champ: 'langue', cle: 'langue_invalide' });
	problemes.push(
		...problemesTexte(
			formulaire.formeJuridique.trim(),
			'forme_juridique',
			FORME_JURIDIQUE_MAX,
			'forme_trop_longue'
		)
	);
	const admis = new Set((pays?.identifiants ?? []).map((t) => t.code));
	const saisis = identifiantsSaisis(formulaire, pack);
	if (saisis.length > IDENTIFIANTS_MAX) {
		problemes.push({ champ: 'identifiants', cle: 'trop_d_identifiants' });
	}
	for (const i of saisis) {
		const champ = `identifiant:${i.type}`;
		if (!admis.has(i.type)) problemes.push({ champ, cle: 'type_non_admis' });
		else {
			problemes.push(
				...problemesTexte(i.valeur, champ, VALEUR_IDENTIFIANT_MAX, 'valeur_trop_longue')
			);
		}
	}
	return problemes;
}

/** Contrôles de forme du nom du dossier. */
export function diagnostiquerNom(nom: string): Probleme[] {
	const net = nom.trim();
	if (net === '') return [{ champ: 'nom', cle: 'nom_absent' }];
	return problemesTexte(net, 'nom', NOM_DOSSIER_MAX, 'nom_trop_long');
}

/** Contrôles de forme de l'ouverture du premier exercice (les bornes de dates sont celles du serveur). */
export function diagnostiquerOuverture(o: FormulaireOuverture): Probleme[] {
	if (o.nature === 'creation') {
		return estDateIso(o.debutActivite) ? [] : [{ champ: 'debut_activite', cle: 'debut_invalide' }];
	}
	return /^\d{4}$/.test(o.annee) ? [] : [{ champ: 'annee', cle: 'annee_invalide' }];
}

function corpsOuverture(o: FormulaireOuverture): CorpsOuverture {
	return o.nature === 'creation'
		? {
				nature: 'creation',
				debut_activite: o.debutActivite,
				fin_premier_exercice: o.finPremier
			}
		: { nature: 'reprise', annee: Number(o.annee) };
}

/**
 * Corps de la création d'un dossier, ou `null` si un contrôle de forme échoue. Le pays est
 * toujours envoyé ; devise, fuseau et langue sont envoyés tels que l'utilisateur les voit.
 */
export function corpsCreation(
	nom: string,
	formulaire: FormulaireContexte,
	ouverture: FormulaireOuverture,
	pack: PackPays
): CorpsCreationDossier | null {
	if (
		diagnostiquerNom(nom).length > 0 ||
		diagnostiquer(formulaire, pack).length > 0 ||
		diagnostiquerOuverture(ouverture).length > 0
	) {
		return null;
	}
	const identifiants = identifiantsSaisis(formulaire, pack);
	const forme = formulaire.formeJuridique.trim();
	return {
		nom: nom.trim(),
		pays: formulaire.pays,
		devise: formulaire.devise,
		fuseau: formulaire.fuseau,
		langue: formulaire.langue,
		...(forme === '' ? {} : { forme_juridique: forme }),
		...(identifiants.length === 0 ? {} : { identifiants_legaux: identifiants }),
		...corpsOuverture(ouverture)
	};
}

/** Libellé d'une langue dans la langue demandée (`Français`), ou le code si `Intl` ne la connaît pas. */
export function libelleLangue(code: string, dans: string): string {
	try {
		const nom = new Intl.DisplayNames([dans], { type: 'language' }).of(code);
		return nom === undefined || nom === code
			? code
			: nom.charAt(0).toLocaleUpperCase(dans) + nom.slice(1);
	} catch {
		return code;
	}
}

/** Décalage d'un fuseau par rapport à UTC, en minutes, écrit `UTC+01:00` (`UTC` pour zéro). */
export function libelleDecalage(minutes: number): string {
	if (minutes === 0) return 'UTC';
	const signe = minutes < 0 ? '-' : '+';
	const absolu = Math.abs(minutes);
	const deux = (n: number) => String(n).padStart(2, '0');
	return `UTC${signe}${deux(Math.floor(absolu / 60))}:${deux(absolu % 60)}`;
}
