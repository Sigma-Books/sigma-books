// Pack pays et corps des requêtes du contexte d'un dossier (ADR 0027,
// `crates/api/src/serveur/routes/organisation.rs`, `referentiel.rs`).
//
// L'API fournit les listes (pays, devises avec leur échelle, fuseaux, types d'identifiants légaux
// du pays) et les valeurs par défaut de chaque pays : l'interface n'en connaît aucune. Les gardes
// vérifient la forme de la réponse ; le serveur décide de tout le reste.

import { estDecimales, estDevise, estFuseau, estLangue, estPays } from '../regional.ts';
import type { IdentifiantLegal } from './types.ts';

/** Un type d'identifiant légal admis pour un pays (code défini par le pack), avec son libellé. */
export interface TypeIdentifiant {
	code: string;
	libelle: string;
}

/** Un État du pack avec ses valeurs par défaut. */
export interface PaysPack {
	code: string;
	nom: string;
	/** Devise de tenue par défaut du pays. */
	devise: string;
	/** Fuseau par défaut du pays. */
	fuseau: string;
	/** Langue par défaut du pays. */
	langue: string;
	identifiants: TypeIdentifiant[];
}

export interface DevisePack {
	code: string;
	intitule: string;
	decimales: number;
}

export interface FuseauPack {
	nom: string;
	decalage_minutes: number;
}

/** Réponse de `GET /api/cabinets/{cabinet}/pays`. */
export interface PackPays {
	pays: PaysPack[];
	devises: DevisePack[];
	fuseaux: FuseauPack[];
}

type Objet = Record<string, unknown>;

const estObjet = (v: unknown): v is Objet =>
	typeof v === 'object' && v !== null && !Array.isArray(v);
const liste = <T>(v: unknown, garde: (x: unknown) => x is T): v is T[] =>
	Array.isArray(v) && v.every(garde);
/** Texte d'affichage fourni par le serveur : non vide, borné. */
const texte = (v: unknown): v is string => typeof v === 'string' && v !== '' && v.length <= 300;
/** Code d'un type d'identifiant : majuscules, chiffres et tiret bas. */
const codeType = (v: unknown): v is string => typeof v === 'string' && /^[A-Z0-9_]{1,32}$/.test(v);

function estTypeIdentifiant(v: unknown): v is TypeIdentifiant {
	return estObjet(v) && codeType(v.code) && texte(v.libelle);
}

function estPaysPack(v: unknown): v is PaysPack {
	return (
		estObjet(v) &&
		estPays(v.code) &&
		texte(v.nom) &&
		estDevise(v.devise) &&
		estFuseau(v.fuseau) &&
		estLangue(v.langue) &&
		liste(v.identifiants, estTypeIdentifiant)
	);
}

function estDevisePack(v: unknown): v is DevisePack {
	return estObjet(v) && estDevise(v.code) && texte(v.intitule) && estDecimales(v.decimales);
}

function estFuseauPack(v: unknown): v is FuseauPack {
	return (
		estObjet(v) &&
		estFuseau(v.nom) &&
		typeof v.decalage_minutes === 'number' &&
		Number.isInteger(v.decalage_minutes)
	);
}

/** Garde du pack : un pays dont la devise ou le fuseau manque aux tables est refusé (contrat). */
export function estPackPays(v: unknown): v is PackPays {
	if (
		!estObjet(v) ||
		!liste(v.pays, estPaysPack) ||
		!liste(v.devises, estDevisePack) ||
		!liste(v.fuseaux, estFuseauPack)
	) {
		return false;
	}
	const devises = new Set(v.devises.map((d) => d.code));
	const fuseaux = new Set(v.fuseaux.map((f) => f.nom));
	return v.pays.every((p) => devises.has(p.devise) && fuseaux.has(p.fuseau));
}

/** Début de l'activité ou année de reprise : l'ouverture du premier exercice (création). */
export type CorpsOuverture =
	| {
			nature: 'creation';
			/** `AAAA-MM-JJ`. */
			debut_activite: string;
			fin_premier_exercice: 'annee_de_debut' | 'annee_suivante';
	  }
	| { nature: 'reprise'; annee: number };

/** Corps de `POST /api/cabinets/{cabinet}/dossiers` : le pays est obligatoire, le reste facultatif. */
export type CorpsCreationDossier = {
	nom: string;
	pays: string;
	devise?: string;
	fuseau?: string;
	langue?: string;
	forme_juridique?: string;
	identifiants_legaux?: IdentifiantLegal[];
} & CorpsOuverture;

/**
 * Motifs de refus propres au contexte du dossier et à l'échelle de la devise de tenue, avec le
 * statut que le serveur leur donne (422 : refus de forme ; 409 : conflit d'état). Chacun a son
 * message dans le catalogue (`saisie.motif.<motif>` : `messageMetier` les traduit) ; les tests
 * le vérifient en passant par `traduireReponse`.
 */
export const MOTIFS_CONTEXTE = [
	['pays_inconnu', 422],
	['devise_tenue_inconnue', 422],
	['fuseau_inconnu', 422],
	['contexte_dossier_invalide', 422],
	['montant_echelle_devise_tenue', 422],
	['validation_montant_echelle_devise_tenue', 422],
	['dossier_deja_ecritures', 409],
	['dossier_brouillard_non_vide', 409],
	['contexte_dossier_modifie', 409],
	['brouillon_devise_tenue_perimee', 409],
	['aucun_changement', 409]
] as const;
