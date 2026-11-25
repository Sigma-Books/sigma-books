// Raccourcis clavier de la saisie (spec §7.1) et recherche de compte. Fonctions pures : le
// composant lit l'événement, ce module décide de l'action ; testé par `node --test`.

export type ActionClavier =
	/** Entrée : ligne suivante (une ligne est ajoutée après la dernière). */
	| 'ligne-suivante'
	/** `=` dans un montant vide : propose le solde qui équilibre la saisie. */
	| 'solde'
	/** F2 : recherche de compte par numéro ou libellé. */
	| 'recherche-compte'
	/** Ctrl+Entrée : enregistrer au brouillard (état « prête » de la spec : le brouillard n'en a qu'un). */
	| 'enregistrer';

export interface EvenementClavier {
	key: string;
	ctrlKey: boolean;
	metaKey: boolean;
	altKey: boolean;
	shiftKey: boolean;
}

export type ChampSaisie = 'entete' | 'compte' | 'tiers' | 'libelle' | 'debit' | 'credit';

export interface ContexteClavier {
	champ: ChampSaisie;
	/** Le champ est vide (le `=` n'est un raccourci que dans un montant vide). */
	vide: boolean;
	/** Une liste de choix est ouverte : Entrée la valide, ce n'est pas un raccourci. */
	listeOuverte?: boolean;
}

/** Action du clavier, ou `null` : la touche garde alors son effet normal. */
export function actionClavier(e: EvenementClavier, c: ContexteClavier): ActionClavier | null {
	const modifie = e.altKey || e.metaKey;
	if (modifie) return null;
	if (e.key === 'Enter') {
		if (e.ctrlKey) return e.shiftKey ? null : 'enregistrer';
		if (e.shiftKey || c.listeOuverte === true) return null;
		return c.champ === 'entete' ? null : 'ligne-suivante';
	}
	if (e.ctrlKey || e.shiftKey) return null;
	if (e.key === 'F2' && c.champ === 'compte') return 'recherche-compte';
	if (e.key === '=' && (c.champ === 'debit' || c.champ === 'credit') && c.vide) return 'solde';
	return null;
}

export interface CompteRecherchable {
	numero: string;
	intitule: string;
}

/** Minuscules sans accents ni espaces superflus, pour comparer des libellés. */
export function normaliser(texte: string): string {
	return texte
		.normalize('NFD')
		.replace(/\p{M}/gu, '')
		.toLowerCase()
		.replace(/\s+/g, ' ')
		.trim();
}

/**
 * Comptes dont le numéro commence par la requête, puis ceux dont l'intitulé contient tous ses
 * mots. Commodité de saisie : le serveur contrôle seul les comptes admis.
 */
export function rechercherComptes<T extends CompteRecherchable>(
	comptes: readonly T[],
	requete: string,
	max = 8
): T[] {
	const q = normaliser(requete);
	if (q === '') return [];
	const mots = q.split(' ');
	const parNumero: T[] = [];
	const parIntitule: T[] = [];
	for (const c of comptes) {
		if (c.numero.startsWith(q)) parNumero.push(c);
		else {
			const intitule = normaliser(c.intitule);
			if (mots.every((m) => intitule.includes(m))) parIntitule.push(c);
		}
	}
	return [...parNumero, ...parIntitule].slice(0, max);
}
