// Lien reçu par courriel (invitation, réinitialisation) : `/lien/#<jeton>`. Le jeton est dans le
// fragment, que le navigateur n'envoie ni au serveur ni dans `Referer` (`crates/api/src/envoi`).

/** Jeton lu dans le fragment (`#…`), ou `null` s'il n'a pas la forme d'un jeton (base64url). */
export function jetonDuFragment(fragment: string): string | null {
	const jeton = fragment.startsWith('#') ? fragment.slice(1) : fragment;
	return /^[A-Za-z0-9_-]{16,256}$/.test(jeton) ? jeton : null;
}
