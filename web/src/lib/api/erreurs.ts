// Erreurs de l'API traduites en types (crates/api/src/serveur/erreur.rs).
//
// Le serveur répond toujours `{"erreur":"<code>"}`, code pris dans une liste fermée. Le client
// n'en garde que ce code et le statut : jamais le corps brut, jamais l'URL ni un identifiant.
// Une ressource d'autrui et une ressource absente donnent le même 404 (ADR 0011) : l'interface
// les présente de la même façon (« introuvable »), sans rien supposer de plus.

/** Codes émis par le serveur (`ErreurApi::code`). */
export const CODES_SERVEUR = [
	'requete_invalide',
	'non_authentifie',
	'interdit',
	'introuvable',
	'methode_non_admise',
	'corps_trop_grand',
	'trop_de_requetes',
	'erreur_interne',
	'delai_depasse',
	'conflit',
	'refus'
] as const;

export type CodeServeur = (typeof CODES_SERVEUR)[number];

/**
 * Motifs d'un refus métier (409 et 422), pris dans une liste fermée : seuls ceux que l'interface
 * de connexion, d'enrôlement et d'administration sait expliquer. Un autre motif est ignoré (le
 * message générique du statut s'affiche).
 */
export const MOTIFS_SERVEUR = [
	// Facteurs et passkeys (422)
	'mot_de_passe_trop_court',
	'mot_de_passe_trop_long',
	'mot_de_passe_compromis',
	'code_invalide',
	'jeton_enrolement_invalide',
	'facteurs_refuses',
	'lien_invalide',
	'aucune_passkey',
	'passkey_refusee',
	'passkey_deja_enregistree',
	'passkeys_max',
	'dernier_facteur',
	// Administration (409)
	'deja_membre',
	'invitation_en_cours',
	'invitation_non_en_attente',
	'action_sur_soi_meme',
	'dernier_administrateur',
	'reinitialisation_en_cours',
	'reinitialisation_non_en_attente',
	'compte_existant',
	'membre_d_un_autre_cabinet',
	'aucun_changement'
] as const;

export type MotifServeur = (typeof MOTIFS_SERVEUR)[number];

/** Codes propres au client : serveur injoignable, réponse d'une forme inattendue. */
export type CodeClient = 'reseau' | 'reponse_invalide';

export type CodeErreur = CodeServeur | CodeClient;

/**
 * Précisions d'un refus métier (409, 422) : motif stable du serveur (liste fermée côté serveur,
 * forme `[a-z0-9_]{1,64}` vérifiée ici), nombre et ligne cités, écritures refusées d'un lot.
 */
export interface DetailRefus {
	readonly motif: string | null;
	readonly nombre: number | null;
	readonly ligne: number | null;
	readonly refus: readonly { ecriture: string; code: string }[];
}

/** Erreur d'un appel à l'API. Son message est le code seul : aucune donnée n'y figure. */
export class ErreurApi extends Error {
	readonly code: CodeErreur;
	/** Statut HTTP, ou `null` si aucune réponse n'a été reçue. */
	readonly statut: number | null;
	/** Délai avant de réessayer, en secondes (`Retry-After` d'un 429). */
	readonly reessayerDans: number | null;
	/** Motif d'un refus 409 ou 422, s'il appartient à la liste fermée. */
	readonly motif: MotifServeur | null;
	/** Précisions d'un refus métier (motif stable, ligne, nombre, refus d'un lot). */
	readonly detail: DetailRefus | null;

	constructor(
		code: CodeErreur,
		statut: number | null,
		reessayerDans: number | null = null,
		motif: MotifServeur | null = null,
		detail: DetailRefus | null = null
	) {
		super(code);
		this.detail = detail;
		this.name = 'ErreurApi';
		this.code = code;
		this.statut = statut;
		this.reessayerDans = reessayerDans;
		this.motif = motif;
	}
}

export function estMotifServeur(valeur: unknown): valeur is MotifServeur {
	return typeof valeur === 'string' && (MOTIFS_SERVEUR as readonly string[]).includes(valeur);
}

export function estCodeServeur(valeur: unknown): valeur is CodeServeur {
	return typeof valeur === 'string' && (CODES_SERVEUR as readonly string[]).includes(valeur);
}

/** Code déduit du seul statut, comme le fait le serveur (`erreur::pour_statut`). */
export function codeDepuisStatut(statut: number): CodeServeur {
	switch (statut) {
		case 400:
			return 'requete_invalide';
		case 401:
			return 'non_authentifie';
		case 403:
			return 'interdit';
		case 404:
			return 'introuvable';
		case 405:
			return 'methode_non_admise';
		case 409:
			return 'conflit';
		case 413:
			return 'corps_trop_grand';
		case 422:
			return 'refus';
		case 429:
			return 'trop_de_requetes';
		case 503:
			return 'delai_depasse';
		default:
			return statut >= 500 ? 'erreur_interne' : 'requete_invalide';
	}
}

/** `Retry-After` en secondes entières ; toute autre forme est ignorée. */
function lireReessayer(valeur: string | null): number | null {
	if (valeur === null || !/^\d{1,6}$/.test(valeur.trim())) return null;
	return Number.parseInt(valeur.trim(), 10);
}

/** Statuts dont l'interface tire une conduite (connexion, refus, absence) : le statut décide. */
const STATUTS_DECISIFS = new Set([401, 403, 404, 429]);

/**
 * Traduit une réponse d'erreur en {@link ErreurApi}. Pour 401, 403, 404 et 429, le statut seul
 * fixe le code : un corps inattendu ne change jamais la conduite de l'interface. Pour les autres
 * statuts, le code du corps est retenu s'il appartient à la liste fermée, sinon le statut décide.
 */
export async function traduireReponse(reponse: Response): Promise<ErreurApi> {
	let code: CodeServeur = codeDepuisStatut(reponse.status);
	let motif: MotifServeur | null = null;
	let detail: DetailRefus | null = null;
	if (!STATUTS_DECISIFS.has(reponse.status)) {
		try {
			const corps: unknown = await reponse.json();
			if (typeof corps === 'object' && corps !== null && 'erreur' in corps) {
				if (reponse.status === 409 || reponse.status === 422) {
					if (estMotifServeur(corps.erreur)) motif = corps.erreur;
				} else if (estCodeServeur(corps.erreur)) {
					code = corps.erreur;
				}
			}
			if ([409, 415, 422].includes(reponse.status)) detail = lireDetailRefus(corps);
		} catch {
			// Corps absent ou non JSON : le statut suffit.
		}
	}
	const reessayer =
		code === 'trop_de_requetes' ? lireReessayer(reponse.headers.get('retry-after')) : null;
	return new ErreurApi(code, reponse.status, reessayer, motif, detail);
}

const MOTIF_CODE = /^[a-z0-9_]{1,64}$/;

function entierOuNul(valeur: unknown): number | null {
	return typeof valeur === 'number' && Number.isSafeInteger(valeur) && valeur >= 0 ? valeur : null;
}

/** Précisions du corps d'un 409 ou 422 : tout ce qui n'a pas la forme attendue est ignoré. */
export function lireDetailRefus(corps: unknown): DetailRefus | null {
	if (typeof corps !== 'object' || corps === null) return null;
	const o = corps as Record<string, unknown>;
	const motif = typeof o.erreur === 'string' && MOTIF_CODE.test(o.erreur) ? o.erreur : null;
	if (motif === null) return null;
	const refus: { ecriture: string; code: string }[] = [];
	if (Array.isArray(o.refus)) {
		for (const r of o.refus.slice(0, 1000)) {
			if (typeof r !== 'object' || r === null) continue;
			const { ecriture, code } = r as Record<string, unknown>;
			if (typeof ecriture === 'string' && typeof code === 'string' && MOTIF_CODE.test(code)) {
				refus.push({ ecriture, code });
			}
		}
	}
	return { motif, nombre: entierOuNul(o.nombre), ligne: entierOuNul(o.ligne), refus };
}

const MESSAGES_MOTIFS: Record<MotifServeur, string> = {
	mot_de_passe_trop_court: 'Le mot de passe est trop court (12 caractères au moins).',
	mot_de_passe_trop_long: 'Le mot de passe est trop long (128 caractères au plus).',
	mot_de_passe_compromis: 'Ce mot de passe figure dans une liste de mots de passe compromis.',
	code_invalide: 'Le code est invalide ou a déjà été utilisé.',
	jeton_enrolement_invalide: 'La préparation a expiré. Recommencez l’enrôlement.',
	facteurs_refuses: 'La vérification a échoué.',
	lien_invalide: 'Ce lien est invalide, expiré ou déjà utilisé.',
	aucune_passkey: 'Aucune passkey n’est enregistrée sur ce compte.',
	passkey_refusee: 'La passkey n’a pas été acceptée. Recommencez.',
	passkey_deja_enregistree: 'Cet appareil ou cette clé est déjà enregistré.',
	passkeys_max: 'Le nombre maximal de passkeys est atteint.',
	dernier_facteur: 'Impossible : ce serait votre dernier moyen de connexion.',
	deja_membre: 'Cette personne est déjà membre du cabinet.',
	invitation_en_cours: 'Une invitation est déjà en attente pour cette adresse.',
	invitation_non_en_attente: 'Cette invitation n’est plus en attente.',
	action_sur_soi_meme: 'Cette action ne peut pas porter sur votre propre compte.',
	dernier_administrateur: 'Impossible : le cabinet doit garder un administrateur actif.',
	reinitialisation_en_cours: 'Une réinitialisation est déjà en attente pour ce membre.',
	reinitialisation_non_en_attente: 'Cette réinitialisation n’est plus en attente.',
	compte_existant:
		'Un compte existe déjà pour cette adresse. Connectez-vous, puis acceptez l’invitation.',
	membre_d_un_autre_cabinet:
		'Ce membre appartient aussi à un autre cabinet : sa réinitialisation est impossible ici.',
	aucun_changement: 'Rien à changer : l’état demandé est déjà celui du serveur.'
};

/** Message en français, sans aucune donnée de la requête ni de la réponse. */
export function messageErreur(erreur: ErreurApi): string {
	if (erreur.motif !== null) return MESSAGES_MOTIFS[erreur.motif];
	switch (erreur.code) {
		case 'requete_invalide':
			return 'La demande est invalide.';
		case 'non_authentifie':
			return 'Votre session est fermée ou a expiré. Reconnectez-vous.';
		case 'interdit':
			return 'Action refusée par le serveur.';
		case 'introuvable':
			return 'Élément introuvable.';
		case 'methode_non_admise':
			return 'Opération non prise en charge par le serveur.';
		case 'corps_trop_grand':
			return 'La demande est trop volumineuse.';
		case 'trop_de_requetes':
			return erreur.reessayerDans === null
				? 'Trop de demandes. Réessayez dans un instant.'
				: `Trop de demandes. Réessayez dans ${erreur.reessayerDans} s.`;
		case 'erreur_interne':
			return 'Erreur du serveur. Réessayez plus tard.';
		case 'delai_depasse':
			return 'Le serveur n’a pas répondu à temps. Réessayez.';
		case 'reseau':
			return 'Serveur injoignable. Vérifiez votre connexion.';
		case 'reponse_invalide':
			return 'Réponse inattendue du serveur.';
		case 'conflit':
			return 'Action impossible dans l’état actuel.';
		case 'refus':
			return 'La saisie a été refusée.';
	}
}
