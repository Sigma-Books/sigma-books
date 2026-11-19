// Types des réponses de l'API et gardes qui les vérifient à l'exécution.
//
// Le JSON reçu est `unknown` tant qu'une garde ne l'a pas reconnu : une réponse d'une autre forme
// devient une erreur `reponse_invalide`, jamais une donnée affichée à moitié. Les noms des champs
// sont ceux du serveur (`crates/api/src/serveur/routes.rs`, `moi.rs`).

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

/** Un dossier visible (`GET /api/cabinets/{c}/dossiers[/{d}]`). */
export interface Dossier {
	id: Uuid;
	nom: string;
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

export function estDossier(valeur: unknown): valeur is Dossier {
	return estObjet(valeur) && estId(valeur.id) && typeof valeur.nom === 'string';
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
