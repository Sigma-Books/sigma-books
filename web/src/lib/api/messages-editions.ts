// Messages français des codes stables de l'API pour les éditions, l'intégrité et les alertes
// (`crates/api/src/serveur/erreur.rs`, `verification.rs`). Le serveur décide ; l'interface ne fait
// que traduire. Aucune donnée de la requête n'est citée.

import { ErreurApi, messageErreur } from './erreurs.ts';

/** Vrai si l'erreur est le 409 `edition_trop_volumineuse` (plage de plus de N écritures). */
export function estEditionTropVolumineuse(erreur: ErreurApi): boolean {
	return erreur.statut === 409 && erreur.detail?.motif === 'edition_trop_volumineuse';
}

/** Message d'une erreur de lecture d'une édition ou d'une alerte. */
export function messageEdition(erreur: ErreurApi): string {
	if (estEditionTropVolumineuse(erreur)) {
		const limite = erreur.detail?.nombre;
		return `L’édition dépasse ${limite == null ? 'la limite du serveur' : `${limite} écritures`} : restreignez la période (dates du et au) ou filtrez par journal ou par compte, puis réessayez.`;
	}
	if (erreur.code === 'requete_invalide') {
		return 'Le serveur a refusé le filtre : vérifiez l’exercice, les dates (dans l’exercice), les comptes et les journaux.';
	}
	if (erreur.code === 'interdit') {
		return 'Votre rôle ne permet pas de consulter cette édition, ou ce dossier ne vous est pas affecté.';
	}
	return messageErreur(erreur);
}

/** Message d'une erreur de l'écran Intégrité (consultation, vérification, export). */
export function messageIntegrite(erreur: ErreurApi): string {
	if (erreur.code === 'interdit') {
		return 'Action refusée : l’intégrité est réservée à l’administrateur, au valideur et à l’auditeur affectés au dossier (l’export exige en plus une session fraîche).';
	}
	if (erreur.code === 'delai_depasse') {
		return 'La vérification n’a pas abouti à temps : le serveur est très sollicité. Réessayez dans un instant.';
	}
	return messageErreur(erreur);
}

const ANOMALIES: Readonly<Record<string, string>> = {
	rang_discontinu: 'Rang discontinu : une écriture manque ou a été insérée dans la chaîne.',
	precedente_incorrecte:
		'L’empreinte précédente de l’écriture ne correspond pas à la précédente de la chaîne.',
	empreinte_alteree: 'L’empreinte stockée ne correspond pas au texte stocké de l’écriture.',
	contenu_illisible: 'Le texte stocké de l’écriture est illisible.',
	colonne_differente: 'Une colonne de l’écriture diffère de son texte stocké.',
	tete_absente: 'La tête de chaîne n’est pas enregistrée.',
	tete_differente: 'La tête enregistrée diffère de la tête recalculée.',
	tete_rang_different: 'Le rang de la tête enregistrée diffère du nombre d’écritures.',
	jeton_hors_chaine: 'Un jeton d’horodatage atteste une chaîne que la base ne contient pas.',
	jeton_invalide: 'La signature d’un jeton d’horodatage est invalide.',
	jeton_refuse: 'Un jeton d’horodatage n’est pas accepté (témoin ou autorité).'
};

const MOTIF_CODE = /^[a-z0-9_]{1,64}$/;

/** Message d'une anomalie ; un code inconnu est rendu en toutes lettres s'il a la forme attendue. */
export function messageAnomalie(code: string): string {
	if (Object.hasOwn(ANOMALIES, code)) return ANOMALIES[code] ?? code;
	return MOTIF_CODE.test(code) ? `Anomalie « ${code} ».` : 'Anomalie inconnue.';
}

/** Un export ne se télécharge qu'après ré-authentification : message du refus de l'annulation. */
export const MESSAGE_EXPORT_ANNULE = 'Export annulé : la ré-authentification n’a pas eu lieu.';

/** Précision d'une anomalie (colonne ou motif) : seulement si elle a la forme d'un code stable. */
export function detailAnomalie(detail: string | null): string | null {
	return detail !== null && MOTIF_CODE.test(detail) ? detail : null;
}
