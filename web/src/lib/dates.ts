// Dates de l'interface : `AAAA-MM-JJ`, la forme de l'API. Aucun calcul comptable ici.

const FORME = /^(\d{4})-(\d{2})-(\d{2})$/;

/** Date locale d'un `Date` JavaScript au format `AAAA-MM-JJ` (pour proposer « aujourd'hui »). */
export function dateIso(d: Date): string {
	const deux = (n: number) => String(n).padStart(2, '0');
	return `${String(d.getFullYear()).padStart(4, '0')}-${deux(d.getMonth() + 1)}-${deux(d.getDate())}`;
}

/** Vrai si le texte est une date `AAAA-MM-JJ` du calendrier (jour existant). */
export function estDateIso(texte: string): boolean {
	const m = FORME.exec(texte);
	if (m === null) return false;
	const [annee, mois, jour] = [Number(m[1]), Number(m[2]), Number(m[3])];
	const d = new Date(Date.UTC(annee, mois - 1, jour));
	return d.getUTCFullYear() === annee && d.getUTCMonth() === mois - 1 && d.getUTCDate() === jour;
}

/** `AAAA-MM-JJ` en `JJ/MM/AAAA` pour l'affichage ; toute autre forme est rendue telle quelle. */
export function afficherDate(texte: string): string {
	const m = FORME.exec(texte);
	return m === null ? texte : `${m[3]}/${m[2]}/${m[1]}`;
}
