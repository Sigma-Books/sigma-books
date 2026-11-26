// Messages des codes stables du serveur pour la saisie, le brouillard, la validation et
// la contre-passation (catalogue `i18n/`, clés `saisie.motif.*` ; `crates/api/src/serveur/erreur.rs`,
// `base/brouillard.rs`). Le serveur
// décide ; l'interface ne fait que traduire le code. Aucune donnée de la requête n'est citée.

import { chercher, traduire } from '../i18n/index.ts';
import { ErreurApi, messageErreur } from './erreurs.ts';

/** Message d'un motif stable, ou `null` s'il est inconnu de l'interface. */
export function messageMotif(motif: string): string | null {
	return chercher(`saisie.motif.${motif}`);
}

/** Message d'une erreur de l'API : le motif du serveur d'abord, sinon le message général. */
export function messageMetier(erreur: ErreurApi): string {
	const motif = erreur.detail?.motif;
	const base = motif === null || motif === undefined ? null : messageMotif(motif);
	if (base === null) return messageErreur(erreur);
	return erreur.detail?.ligne == null
		? base
		: traduire('saisie.ligne', { ligne: erreur.detail.ligne, message: base });
}

/** Vrai si l'erreur est le conflit de version du brouillon (409 `brouillon_perime`). */
export function estConflitDeVersion(erreur: ErreurApi): boolean {
	return erreur.statut === 409 && erreur.detail?.motif === 'brouillon_perime';
}

/** Vrai si le brouillon cité n'est plus modifiable pour une raison de conflit (409). */
export function estConflit(erreur: ErreurApi): boolean {
	return erreur.statut === 409;
}
