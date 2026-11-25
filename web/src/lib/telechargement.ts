// Enregistrement d'un fichier par le navigateur, sans `{@html}` ni script inline : un `Blob`, une
// adresse `blob:` éphémère et un lien `download` (aucune écriture dans le DOM de contenu reçu).

/** Vrai si `nom` est un nom de fichier sûr : caractères simples, pas de chemin, extension connue. */
export function estNomFichierSur(nom: string): boolean {
	return /^[A-Za-z0-9][A-Za-z0-9._-]{0,127}\.(txt|csv)$/.test(nom) && !nom.includes('..');
}

/** Déclenche l'enregistrement. Refuse un nom douteux. À n'appeler que dans le navigateur. */
export function enregistrerFichier(contenu: Blob, nom: string): void {
	if (!estNomFichierSur(nom)) throw new TypeError('nom de fichier non sûr');
	const adresse = URL.createObjectURL(contenu);
	const lien = document.createElement('a');
	lien.href = adresse;
	lien.download = nom;
	lien.rel = 'noopener';
	lien.hidden = true;
	document.body.append(lien);
	lien.click();
	lien.remove();
	// Laisse au navigateur le temps de lire l'adresse avant de la libérer.
	setTimeout(() => URL.revokeObjectURL(adresse), 10_000);
}
