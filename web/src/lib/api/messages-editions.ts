// Messages des codes stables de l'API pour les éditions, l'intégrité et les alertes
// (`crates/api/src/serveur/erreur.rs`, `verification.rs`). Le serveur décide ; l'interface ne fait
// que traduire. Aucune donnée de la requête n'est citée.

import { chercher, traduire } from '../i18n/index.ts';
import { ErreurApi, messageErreur } from './erreurs.ts';

/** Vrai si l'erreur est le 409 `edition_trop_volumineuse` (plage de plus de N écritures). */
export function estEditionTropVolumineuse(erreur: ErreurApi): boolean {
	return erreur.statut === 409 && erreur.detail?.motif === 'edition_trop_volumineuse';
}

/** Message d'une erreur de lecture d'une édition ou d'une alerte. */
export function messageEdition(erreur: ErreurApi): string {
	if (estEditionTropVolumineuse(erreur)) {
		const limite = erreur.detail?.nombre;
		return limite == null
			? traduire('edition.trop_volumineuse_sans_limite')
			: traduire('edition.trop_volumineuse', { limite });
	}
	if (erreur.code === 'requete_invalide') {
		return traduire('edition.filtre_refuse');
	}
	if (erreur.code === 'interdit') {
		return traduire('edition.interdit');
	}
	return messageErreur(erreur);
}

/** Message d'une erreur de l'écran Intégrité (consultation, vérification, export). */
export function messageIntegrite(erreur: ErreurApi): string {
	if (erreur.code === 'interdit') {
		return traduire('integrite.interdit');
	}
	if (erreur.code === 'delai_depasse') {
		return traduire('integrite.delai');
	}
	return messageErreur(erreur);
}

const MOTIF_CODE = /^[a-z0-9_]{1,64}$/;

/** Message d'une anomalie ; un code inconnu est rendu en toutes lettres s'il a la forme attendue. */
export function messageAnomalie(code: string): string {
	const connue = chercher(`anomalie.code.${code}`);
	if (connue !== null) return connue;
	return MOTIF_CODE.test(code)
		? traduire('anomalie.avec_code', { code })
		: traduire('anomalie.inconnue');
}

/** Un export ne se télécharge qu'après ré-authentification : message du refus de l'annulation. */
export const messageExportAnnule = (): string => traduire('edition.export_annule');

/** Précision d'une anomalie (colonne ou motif) : seulement si elle a la forme d'un code stable. */
export function detailAnomalie(detail: string | null): string | null {
	return detail !== null && MOTIF_CODE.test(detail) ? detail : null;
}
