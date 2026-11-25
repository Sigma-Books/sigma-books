// Pièces justificatives : dépôt d'un fichier, pièce interne, liste (`routes/pieces.rs`).
//
// Le fichier part brut (Blob) avec son nom et son empreinte annoncés en paramètres ; le serveur
// reconnaît le type par signature (PDF, JPEG, PNG) et recalcule l'empreinte. Un doublon (même
// empreinte dans le dossier) n'est pas stocké : le serveur le signale et l'utilisateur choisit.

import type { ClientApi } from './client.ts';
import { chaines, curseurOuNul, entier, estId, estObjet, tableauDe } from './gardes.ts';
import type { Uuid } from './types.ts';

export interface Piece {
	id: Uuid;
	nature: string;
	type: string;
	taille: number;
	empreinte: string;
	nom: string | null;
	deposee_le: string;
	ecartee: boolean;
}

export type ResultatDepot = { doublon: false; piece: Piece } | { doublon: true; pieces: Piece[] };

/** Taille maximale d'un fichier déposé (le serveur refuse au-delà : 413). */
export const TAILLE_MAX_PIECE = 20 * 1024 * 1024;

export const estPiece = (v: unknown): v is Piece =>
	estObjet(v) &&
	estId(v.id) &&
	chaines(v, ['nature', 'type', 'empreinte', 'deposee_le']) &&
	(v.nom === null || typeof v.nom === 'string') &&
	entier(v.taille) &&
	typeof v.ecartee === 'boolean';

const estDoublon = (v: unknown): v is { doublon: true; pieces: Piece[] } =>
	estObjet(v) && v.doublon === true && tableauDe(v.pieces, estPiece);

const estDepot = (v: unknown): v is Piece | { doublon: true; pieces: Piece[] } =>
	estPiece(v) || estDoublon(v);

const estPagePieces = (v: unknown): v is { pieces: Piece[]; suivant?: string | null } =>
	estObjet(v) && tableauDe(v.pieces, estPiece) && curseurOuNul(v.suivant);

const base = (cabinet: Uuid, dossier: Uuid) =>
	`/api/cabinets/${cabinet}/dossiers/${dossier}/pieces`;

/** Nom de fichier sûr pour le paramètre `nom` : sans chemin ni caractère de contrôle, borné. */
export function nomPieceSur(nom: string): string {
	const net = [...nom]
		.filter((c) => {
			const code = c.codePointAt(0) ?? 0;
			return code >= 0x20 && code !== 0x7f;
		})
		.join('')
		.replace(/[\\/]/g, '_')
		.trim();
	let borne = net;
	// Le serveur limite le nom à 255 octets UTF-8.
	while (new TextEncoder().encode(borne).length > 255) borne = borne.slice(0, -1);
	return borne;
}

/** Empreinte SHA-256 en hexadécimal minuscule (calculée par le navigateur, recontrôlée par le serveur). */
export async function empreinteSha256(octets: ArrayBuffer): Promise<string> {
	const condensat = await globalThis.crypto.subtle.digest('SHA-256', octets);
	return [...new Uint8Array(condensat)].map((o) => o.toString(16).padStart(2, '0')).join('');
}

export async function deposerPiece(
	api: ClientApi,
	cabinet: Uuid,
	dossier: Uuid,
	fichier: File,
	confirmerDoublon = false
): Promise<ResultatDepot> {
	const empreinte = await empreinteSha256(await fichier.arrayBuffer());
	const p = new URLSearchParams({ empreinte });
	const nom = nomPieceSur(fichier.name);
	if (nom !== '') p.set('nom', nom);
	if (confirmerDoublon) p.set('doublon', 'confirme');
	const reponse = await api.muter(
		'POST',
		`${base(cabinet, dossier)}?${p.toString()}`,
		estDepot,
		fichier
	);
	return 'doublon' in reponse
		? { doublon: true, pieces: reponse.pieces }
		: { doublon: false, piece: reponse };
}

/** Note d'opérations diverses (pièce interne) : date de l'opération, motif, lignes d'explication. */
export function deposerPieceInterne(
	api: ClientApi,
	cabinet: Uuid,
	dossier: Uuid,
	corps: { date: string; motif: string; lignes: string[] }
): Promise<Piece> {
	return api.muter('POST', `${base(cabinet, dossier)}/internes`, estPiece, corps);
}

export async function listerPieces(api: ClientApi, cabinet: Uuid, dossier: Uuid): Promise<Piece[]> {
	return (await api.lire(`${base(cabinet, dossier)}?limite=100`, estPagePieces)).pieces;
}
