// Longueurs de texte comme le serveur les compte.

/** Longueur en octets UTF-8 : le serveur borne le motif d'une contre-passation en octets. */
export function longueurOctets(texte: string): number {
	return new TextEncoder().encode(texte).length;
}
