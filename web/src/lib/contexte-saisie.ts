// Saisie du contexte d'un dossier (ADR 0027) : état du formulaire, valeurs par défaut tirées du
// pack pays de l'API, contrôles de forme et corps des requêtes de création et de modification.
//
// L'interface ne décide de rien : les listes (pays, devises, fuseaux, types d'identifiants) et les
// valeurs par défaut de chaque pays viennent de l'API, les bornes de forme ci-dessous sont celles
// du serveur (`contexte_dossier.rs`), citées pour le confort de saisie ; le serveur contrôle de
// toute façon (422) et son refus est affiché. Module pur, testé sous `node --test`.

import type { CorpsCreationDossier, CorpsOuverture, PackPays, PaysPack } from './api/pays.ts';
import type { ErreurApi } from './api/erreurs.ts';
import type { IdentifiantLegal } from './api/types.ts';
import { estDateIso } from './dates.ts';
import { chercher } from './i18n/index.ts';
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
	/**
	 * Valeur saisie de chaque type d'identifiant, `''` : aucun. Les valeurs des types que le pays
	 * choisi n'admet pas sont **conservées** (masquées, non envoyées) : revenir au pays d'origine les
	 * retrouve (`etatIdentifiants`).
	 */
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
	'nom_absent',
	'nom_trop_long',
	'debut_invalide',
	'annee_invalide',
	'annee_hors_bornes',
	'debut_hors_bornes',
	'prolongation_impossible'
] as const;
export type CleProbleme = (typeof CLES_PROBLEME)[number];

/** Un champ refusé : `champ` est le nom du champ du formulaire (`identifiant:<code du type>` pour un type). */
export interface Probleme {
	champ: string;
	cle: CleProbleme;
	/** Paramètres du message (années admises…), en plus de la borne de `BORNES_PROBLEMES`. */
	params?: Readonly<Record<string, number>>;
}

/** Message d'un problème de saisie, dans la langue courante. */
export function messageProbleme(p: Probleme): string {
	const max = BORNES_PROBLEMES[p.cle];
	return (
		chercher(`contexte.probleme.${p.cle}`, {
			...(max === undefined ? {} : { max }),
			...p.params
		}) ?? ''
	);
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
 * Valeurs d'identifiants après un changement de pays : **toutes** les valeurs déjà saisies sont
 * gardées (un aller-retour entre deux pays ne perd rien), et chaque type admis par le nouveau pays
 * a son champ.
 */
function identifiantsPour(
	pays: PaysPack | undefined,
	anciens: Readonly<Record<string, string>>
): Record<string, string> {
	const nouveaux: Record<string, string> = { ...anciens };
	for (const t of pays?.identifiants ?? []) {
		if (!Object.hasOwn(nouveaux, t.code)) nouveaux[t.code] = '';
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

/** Ce que le formulaire montre et envoie pour les identifiants légaux. */
export interface EtatIdentifiants {
	/** Champs à afficher : types du pays, puis types déjà inscrits que le pays n'a pas changés. */
	affiches: { code: string; libelle: string }[];
	/** Identifiants envoyés : valeurs rognées et non vides des champs affichés. */
	effectifs: IdentifiantLegal[];
	/** Types saisis mais non admis par le pays choisi : masqués, **non envoyés** (à signaler). */
	retires: string[];
}

/**
 * Identifiants du formulaire. Un type admis par le pays choisi est affiché ; un type inscrit au
 * dossier que le pack n'admet pas est conservé tant que le pays n'a pas changé par rapport à
 * `paysInitial` (`null` à la création). Au changement de pays, les types non admis sont retirés de
 * l'envoi (le serveur les refuserait) mais leurs valeurs restent dans le formulaire jusqu'à
 * l'envoi : revenir au pays d'origine les retrouve.
 */
export function etatIdentifiants(
	formulaire: FormulaireContexte,
	pack: PackPays,
	paysInitial: string | null
): EtatIdentifiants {
	const admis = paysDuPack(pack, formulaire.pays)?.identifiants ?? [];
	const codesAdmis = new Set(admis.map((t) => t.code));
	const conserveLesAutres = paysInitial !== null && formulaire.pays === paysInitial;
	const valeurDe = (code: string): string => (formulaire.identifiants[code] ?? '').trim();
	const autres = Object.keys(formulaire.identifiants).filter((c) => !codesAdmis.has(c));
	const conserves = conserveLesAutres ? autres.filter((c) => valeurDe(c) !== '') : [];
	const affiches = [
		...admis.map((t) => ({ code: t.code, libelle: t.libelle })),
		...conserves.map((code) => ({ code, libelle: code }))
	];
	return {
		affiches,
		effectifs: affiches.flatMap(({ code }) => {
			const valeur = valeurDe(code);
			return valeur === '' ? [] : [{ type: code, valeur }];
		}),
		retires: conserveLesAutres ? [] : autres.filter((c) => valeurDe(c) !== '')
	};
}

function problemesTexte(texte: string, champ: string, max: number, trop: CleProbleme): Probleme[] {
	if (CONTROLE.test(texte)) return [{ champ, cle: 'caractere_de_controle' }];
	return longueur(texte) > max ? [{ champ, cle: trop }] : [];
}

/**
 * Contrôles de forme du contexte (confort : le serveur contrôle de toute façon, 422). Avec
 * `initial` (modification), un champ n'est contrôlé que s'il a changé : une valeur déjà inscrite
 * que le pack ne propose plus ne bloque pas la modification d'un autre champ.
 */
export function diagnostiquer(
	formulaire: FormulaireContexte,
	pack: PackPays,
	initial: FormulaireContexte | null = null
): Probleme[] {
	const problemes: Probleme[] = [];
	const change = (champ: 'pays' | 'devise' | 'fuseau' | 'langue') =>
		initial === null || formulaire[champ] !== initial[champ];
	if (formulaire.pays === '') problemes.push({ champ: 'pays', cle: 'pays_absent' });
	else if (change('pays') && paysDuPack(pack, formulaire.pays) === undefined) {
		problemes.push({ champ: 'pays', cle: 'pays_inconnu' });
	}
	if (change('devise') && !pack.devises.some((d) => d.code === formulaire.devise)) {
		problemes.push({ champ: 'devise', cle: 'devise_inconnue' });
	}
	if (change('fuseau') && !pack.fuseaux.some((f) => f.nom === formulaire.fuseau)) {
		problemes.push({ champ: 'fuseau', cle: 'fuseau_inconnu' });
	}
	if (change('langue') && !estLangue(formulaire.langue)) {
		problemes.push({ champ: 'langue', cle: 'langue_invalide' });
	}
	problemes.push(
		...problemesTexte(
			formulaire.formeJuridique.trim(),
			'forme_juridique',
			FORME_JURIDIQUE_MAX,
			'forme_trop_longue'
		)
	);
	const { effectifs } = etatIdentifiants(formulaire, pack, initial?.pays ?? null);
	if (effectifs.length > IDENTIFIANTS_MAX) {
		problemes.push({ champ: 'identifiants', cle: 'trop_d_identifiants' });
	}
	for (const i of effectifs) {
		problemes.push(
			...problemesTexte(
				i.valeur,
				`identifiant:${i.type}`,
				VALEUR_IDENTIFIANT_MAX,
				'valeur_trop_longue'
			)
		);
	}
	return problemes;
}

/** Contrôles de forme du nom du dossier. */
export function diagnostiquerNom(nom: string): Probleme[] {
	const net = nom.trim();
	if (net === '') return [{ champ: 'nom', cle: 'nom_absent' }];
	return problemesTexte(net, 'nom', NOM_DOSSIER_MAX, 'nom_trop_long');
}

/** Année d'une date `AAAA-MM-JJ`. */
const anneeDe = (date: string): number => Number(date.slice(0, 4));

/**
 * Contrôles de l'ouverture du premier exercice. `aujourdhui` est la date du jour **dans le
 * fuseau choisi** (le serveur calcule l'année en cours dans le fuseau du dossier). Les bornes sont
 * celles du serveur (`Instanciation::preparer`, `Exercices::ouvrir_premier`), reprises pour
 * expliquer le refus avant l'envoi : reprise de l'année en cours ou de la précédente ; entité
 * nouvelle débutant de l'année précédente à l'année suivante ; premier exercice finissant l'année
 * suivante seulement si l'activité débute au second semestre.
 */
export function diagnostiquerOuverture(o: FormulaireOuverture, aujourdhui: string): Probleme[] {
	const n = anneeDe(aujourdhui);
	if (o.nature === 'reprise') {
		if (!/^\d{4}$/.test(o.annee)) return [{ champ: 'annee', cle: 'annee_invalide' }];
		const annee = Number(o.annee);
		return annee < n - 1 || annee > n
			? [{ champ: 'annee', cle: 'annee_hors_bornes', params: { min: n - 1, max: n } }]
			: [];
	}
	if (!estDateIso(o.debutActivite)) return [{ champ: 'debut_activite', cle: 'debut_invalide' }];
	const an = anneeDe(o.debutActivite);
	if (an < n - 1 || an > n + 1) {
		return [
			{ champ: 'debut_activite', cle: 'debut_hors_bornes', params: { min: n - 1, max: n + 1 } }
		];
	}
	const mois = Number(o.debutActivite.slice(5, 7));
	return o.finPremier === 'annee_suivante' && mois <= 6
		? [{ champ: 'fin_premier_exercice', cle: 'prolongation_impossible' }]
		: [];
}

/**
 * Ouverture après un changement de fuseau : la date ou l'année **proposées** (celles de la date du
 * jour d'avant) suivent la date du jour du nouveau fuseau ; une valeur saisie à la main reste.
 */
export function ouvertureSuivantFuseau(
	o: FormulaireOuverture,
	avant: string,
	apres: string
): FormulaireOuverture {
	return {
		...o,
		debutActivite: o.debutActivite === avant ? apres : o.debutActivite,
		annee: o.annee === avant.slice(0, 4) ? apres.slice(0, 4) : o.annee
	};
}

/**
 * Vrai si la création a pu aboutir malgré l'erreur : le serveur a répondu 201 mais la réponse n'a
 * pas la forme attendue. Le dossier existe peut-être : ne pas proposer de le recréer.
 */
export function creationPossiblementFaite(erreur: ErreurApi): boolean {
	return erreur.code === 'reponse_invalide' && erreur.statut === 201;
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
	pack: PackPays,
	aujourdhui: string
): CorpsCreationDossier | null {
	if (
		diagnostiquerNom(nom).length > 0 ||
		diagnostiquer(formulaire, pack).length > 0 ||
		diagnostiquerOuverture(ouverture, aujourdhui).length > 0
	) {
		return null;
	}
	const identifiants = etatIdentifiants(formulaire, pack, null).effectifs;
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
