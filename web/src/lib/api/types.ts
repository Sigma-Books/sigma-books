// Types des réponses de l'API et gardes qui les vérifient à l'exécution.
//
// Le JSON reçu est `unknown` tant qu'une garde ne l'a pas reconnu : une réponse d'une autre forme
// devient une erreur `reponse_invalide`, jamais une donnée affichée à moitié. Les noms des champs
// sont ceux du serveur (`crates/api/src/serveur/routes.rs`, `moi.rs`).

import { estDecimales, estDevise, estFuseau, estLangue, estPays } from '../regional.ts';
/** Identifiant exposé par l'API (UUID, ADR 0011), vérifié par `lireUuid`. */
export type Uuid = string & { readonly __marque: 'Uuid' };

const MOTIF_UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;

/**
 * Identifiant reconnu, en minuscules, ou `null`. Toute valeur lue dans l'URL de l'interface passe
 * par ici avant de former un chemin de l'API : aucun `../`, `?` ou `/` ne peut s'y glisser.
 */
export function lireUuid(valeur: string | null | undefined): Uuid | null {
	return typeof valeur === 'string' && MOTIF_UUID.test(valeur)
		? (valeur.toLowerCase() as Uuid)
		: null;
}

/** État de la session courante (`GET /api/session`), sans le jeton anti-CSRF. */
export interface EtatSession {
	id: string;
	/** `complete`, `restreinte` ou `enrolement`. */
	nature: string;
	methode: string;
	cree_le: string;
	fin_absolue: string;
	fin_inactivite: string;
	derniere_verification: string;
	fraiche: boolean;
	fraiche_jusqu_a: string;
}

/** Un cabinet dont l'utilisateur est membre (`GET /api/moi/cabinets`). */
export interface CabinetMembre {
	id: Uuid;
	nom: string;
	role: string;
}

/**
 * Longueur maximale d'un code de journal : `CODE_MAX` de `crates/compta-core/src/journal.rs`
 * (`CodeJournal::parse`, 1 à 8 majuscules ou chiffres). Le brouillard et les éditions la partagent.
 */
export const CODE_JOURNAL_MAX = 8;

const MOTIF_CODE_JOURNAL = new RegExp(`^[A-Z0-9]{1,${CODE_JOURNAL_MAX}}$`);

/** Vrai si le texte a la forme d'un code de journal du serveur. */
export function estCodeJournal(texte: string): boolean {
	return MOTIF_CODE_JOURNAL.test(texte);
}

/** Un identifiant légal d'un dossier : un type du pack du pays (code défini par le pack) et sa valeur. */
export interface IdentifiantLegal {
	type: string;
	valeur: string;
}

/**
 * Un dossier visible (`GET /api/cabinets/{c}/dossiers[/{d}]`, ADR 0027). Le contexte du dossier
 * (`pays`, `devise`, `decimales`, `fuseau`, `langue`) est **obligatoire** : une réponse qui en
 * manque est une erreur de contrat (`reponse_invalide`), jamais complétée par une valeur
 * supposée. Les identifiants légaux ne sont rendus que par la lecture d'un dossier, la création
 * et la modification du contexte, pas par la liste.
 */
export interface Dossier {
	id: Uuid;
	nom: string;
	pays: string;
	/** Devise de tenue (ISO 4217). */
	devise: string;
	/** Échelle décimale de la devise de tenue. */
	decimales: number;
	fuseau: string;
	langue: string;
	forme_juridique: string | null;
	identifiants_legaux?: IdentifiantLegal[];
}

/** Une session de l'utilisateur (`GET /api/moi/sessions`). */
export interface SessionListee {
	id: Uuid;
	nature: string;
	methode: string;
	cree_le: string;
	derniere_activite: string;
	echeance: string;
	appareil: string | null;
	adresse: string | null;
	courante: boolean;
}

export type Garde<T> = (valeur: unknown) => valeur is T;

type Objet = Record<string, unknown>;

function estObjet(valeur: unknown): valeur is Objet {
	return typeof valeur === 'object' && valeur !== null && !Array.isArray(valeur);
}

function chaines(o: Objet, champs: readonly string[]): boolean {
	return champs.every((champ) => typeof o[champ] === 'string');
}

function chaineOuNul(valeur: unknown): boolean {
	return valeur === null || typeof valeur === 'string';
}

function estId(valeur: unknown): boolean {
	return typeof valeur === 'string' && lireUuid(valeur) === valeur;
}

/** Réponse de `GET /api/session` : l'état, plus le jeton anti-CSRF que seul le client garde. */
export function estReponseSession(valeur: unknown): valeur is EtatSession & { csrf: string } {
	return (
		estObjet(valeur) &&
		chaines(valeur, [
			'id',
			'nature',
			'methode',
			'cree_le',
			'fin_absolue',
			'fin_inactivite',
			'derniere_verification',
			'fraiche_jusqu_a',
			'csrf'
		]) &&
		typeof valeur.fraiche === 'boolean' &&
		valeur.csrf !== ''
	);
}

/** Réponse de `POST /api/session/rotation`. */
export function estReponseRotation(valeur: unknown): valeur is { csrf: string } {
	return estObjet(valeur) && typeof valeur.csrf === 'string' && valeur.csrf !== '';
}

export function estCabinetMembre(valeur: unknown): valeur is CabinetMembre {
	return estObjet(valeur) && estId(valeur.id) && chaines(valeur, ['nom', 'role']);
}

export function estIdentifiantLegal(valeur: unknown): valeur is IdentifiantLegal {
	return estObjet(valeur) && chaines(valeur, ['type', 'valeur']);
}

export function estDossier(valeur: unknown): valeur is Dossier {
	return (
		estObjet(valeur) &&
		estId(valeur.id) &&
		typeof valeur.nom === 'string' &&
		estPays(valeur.pays) &&
		estDevise(valeur.devise) &&
		estDecimales(valeur.decimales) &&
		estFuseau(valeur.fuseau) &&
		estLangue(valeur.langue) &&
		chaineOuNul(valeur.forme_juridique) &&
		(valeur.identifiants_legaux === undefined ||
			(Array.isArray(valeur.identifiants_legaux) &&
				valeur.identifiants_legaux.every(estIdentifiantLegal)))
	);
}

export function estSessionListee(valeur: unknown): valeur is SessionListee {
	return (
		estObjet(valeur) &&
		estId(valeur.id) &&
		chaines(valeur, ['nature', 'methode', 'cree_le', 'derniere_activite', 'echeance']) &&
		chaineOuNul(valeur.appareil) &&
		chaineOuNul(valeur.adresse) &&
		typeof valeur.courante === 'boolean'
	);
}

/** Garde d'une liste dont chaque élément passe la garde donnée. */
export function listeDe<T>(garde: Garde<T>): Garde<T[]> {
	return (valeur: unknown): valeur is T[] => Array.isArray(valeur) && valeur.every(garde);
}

/** Une passkey de l'utilisateur (`GET /api/moi/facteurs` : ni clé ni compteur). */
export interface PasskeyListee {
	id: Uuid;
	nom: string;
	cree_le: string;
	derniere_utilisation: string | null;
}

/** Les méthodes de connexion de l'utilisateur (`GET /api/moi/facteurs`). */
export interface FacteursMoi {
	/** Méthode de secours ; `utilisable` faux : à réenrôler (clé de l'instance changée). */
	secours: { modifie_le: string; utilisable: boolean } | null;
	codes_restants: number;
	passkeys: PasskeyListee[];
}

/** Préparation de l'enrôlement du secours (`POST /api/moi/facteurs/secours/preparation`). */
export interface PreparationSecours {
	secret: string;
	uri: string;
	jeton_enrolement: string;
	expire_le: string;
}

/** Série de codes de récupération, rendue une seule fois. */
export interface CodesRecuperation {
	codes_recuperation: string[];
}

/** Défi WebAuthn (`{"defi": …}`) : la forme des options est vérifiée par `lib/webauthn.ts`. */
export interface Defi {
	defi: unknown;
}

/** Session ouverte par une connexion ou l'usage d'un lien (le jeton anti-CSRF est relu ensuite). */
export interface Connexion {
	nature: string;
}

/** Passkey enrôlée. */
export interface PasskeyEnrolee {
	id: Uuid;
	nom: string;
}

export function estPasskeyListee(valeur: unknown): valeur is PasskeyListee {
	return (
		estObjet(valeur) &&
		estId(valeur.id) &&
		chaines(valeur, ['nom', 'cree_le']) &&
		chaineOuNul(valeur.derniere_utilisation)
	);
}

export function estFacteursMoi(valeur: unknown): valeur is FacteursMoi {
	if (!estObjet(valeur)) return false;
	const { secours } = valeur;
	const secoursValide =
		secours === null ||
		(estObjet(secours) &&
			typeof secours.modifie_le === 'string' &&
			typeof secours.utilisable === 'boolean');
	return (
		secoursValide &&
		typeof valeur.codes_restants === 'number' &&
		Number.isInteger(valeur.codes_restants) &&
		Array.isArray(valeur.passkeys) &&
		valeur.passkeys.every(estPasskeyListee)
	);
}

export function estPreparationSecours(valeur: unknown): valeur is PreparationSecours {
	return estObjet(valeur) && chaines(valeur, ['secret', 'uri', 'jeton_enrolement', 'expire_le']);
}

export function estCodesRecuperation(valeur: unknown): valeur is CodesRecuperation {
	return (
		estObjet(valeur) &&
		Array.isArray(valeur.codes_recuperation) &&
		valeur.codes_recuperation.every((c) => typeof c === 'string')
	);
}

export function estDefi(valeur: unknown): valeur is Defi {
	return estObjet(valeur) && 'defi' in valeur;
}

export function estConnexion(valeur: unknown): valeur is Connexion {
	return estObjet(valeur) && typeof valeur.nature === 'string';
}

export function estPasskeyEnrolee(valeur: unknown): valeur is PasskeyEnrolee {
	return estObjet(valeur) && estId(valeur.id) && typeof valeur.nom === 'string';
}

/** Réponse sans contenu utile (le corps n'est pas lu plus loin) : un objet JSON quelconque. */
export function estObjetJson(valeur: unknown): valeur is Record<string, unknown> {
	return estObjet(valeur);
}

// --- Administration du cabinet (PR 9a) ---------------------------------------------------------

/** Un membre du cabinet (`GET /api/cabinets/{c}/membres`). */
export interface Membre {
	id: Uuid;
	nom: string;
	courriel: string;
	role: string;
	actif: boolean;
}

/** Une invitation en attente (`GET /api/cabinets/{c}/invitations`). */
export interface Invitation {
	id: Uuid;
	courriel: string;
	nom: string;
	role: string;
	cree_le: string;
}

/** Une réinitialisation d'accès en attente (`GET /api/cabinets/{c}/reinitialisations`). */
export interface Reinitialisation {
	id: Uuid;
	membre: Uuid;
	demandee_par: Uuid | null;
	motif: string;
	cree_le: string;
	echeance: string;
}

export function estMembre(valeur: unknown): valeur is Membre {
	return (
		estObjet(valeur) &&
		estId(valeur.id) &&
		chaines(valeur, ['nom', 'courriel', 'role']) &&
		typeof valeur.actif === 'boolean'
	);
}

export function estInvitation(valeur: unknown): valeur is Invitation {
	return (
		estObjet(valeur) && estId(valeur.id) && chaines(valeur, ['courriel', 'nom', 'role', 'cree_le'])
	);
}

export function estReinitialisation(valeur: unknown): valeur is Reinitialisation {
	return (
		estObjet(valeur) &&
		estId(valeur.id) &&
		estId(valeur.membre) &&
		(valeur.demandee_par === null || estId(valeur.demandee_par)) &&
		chaines(valeur, ['motif', 'cree_le', 'echeance'])
	);
}
