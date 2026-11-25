// Messages français des codes stables du serveur pour la saisie, le brouillard, la validation et
// la contre-passation (`crates/api/src/serveur/erreur.rs`, `base/brouillard.rs`). Le serveur
// décide ; l'interface ne fait que traduire le code. Aucune donnée de la requête n'est citée.

import { ErreurApi, messageErreur } from './erreurs.ts';

const MESSAGES: Readonly<Record<string, string>> = {
	// Saisie invalide en soi (422).
	journal_inconnu: 'Ce journal n’existe pas dans le dossier.',
	journal_sans_saisie: 'Ce journal n’accepte pas la saisie manuelle.',
	date_hors_exercice: 'Cette date n’appartient à aucun exercice du dossier.',
	date_valeur_non_anterieure: 'La date de valeur doit être antérieure à la date comptable.',
	trop_de_lignes: 'L’écriture comporte trop de lignes.',
	compte_inconnu: 'Ce compte n’existe pas dans le plan du dossier.',
	compte_inactif: 'Ce compte est désactivé.',
	compte_de_regroupement:
		'Ce compte est un compte de regroupement : choisissez un compte de détail.',
	montant_invalide: 'Ce montant est refusé (il doit être strictement positif et dans les bornes).',
	tiers_inconnu: 'Ce tiers n’existe pas dans le dossier.',
	tiers_inactif: 'Ce tiers est désactivé.',
	compte_hors_fiche_du_tiers: 'Ce tiers ne peut pas être porté par ce compte.',
	// Conflits (409) du brouillard.
	brouillon_perime:
		'Ce brouillon a été modifié par quelqu’un d’autre depuis que vous l’avez ouvert. Rechargez-le pour voir sa version actuelle.',
	date_dans_periode_cloturee: 'Cette date tombe dans une période ou un exercice clôturé.',
	journal_non_admis_en_cloture: 'Ce journal n’est pas admis pendant la clôture de l’exercice.',
	aucun_changement: 'Rien n’a changé : il n’y a rien à enregistrer.',
	drapeau_inchange: 'Le drapeau « à revoir » est déjà dans l’état demandé.',
	revue_par_le_poseur: 'Le drapeau doit être levé par une autre personne que celle qui l’a posé.',
	piece_ecartee: 'Cette pièce a été écartée : elle ne peut plus être jointe.',
	piece_deja_liee: 'Cette pièce est déjà jointe à ce brouillon.',
	piece_principale_existante: 'Ce brouillon a déjà une pièce principale.',
	piece_liee: 'Cette pièce est jointe à un brouillon : elle ne peut pas être écartée.',
	trop_de_pieces: 'Trop de pièces sont jointes à ce brouillon.',
	lot_sans_ecriture_a_valider:
		'Aucune écriture du lot ne peut être validée : toutes sont marquées « à revoir ».',
	lot_refuse: 'Le lot est refusé : rien n’a été validé. Corrigez les écritures listées ci-dessous.',
	// Validation (409 : état du dossier ; 422 : écriture invalide).
	periode_verrouillee: 'La période est verrouillée : déverrouillez-la ou changez la date.',
	periode_cloturee: 'La période est clôturée.',
	echeance_de_cloture_depassee: 'L’échéance de clôture de la période est dépassée.',
	exercice_cloture: 'L’exercice est clôturé.',
	exercice_en_cloture: 'L’exercice est en clôture : ce journal n’est pas admis.',
	date_valeur_dans_periode_ouverte: 'La date de valeur tombe dans une période encore ouverte.',
	date_valeur_dans_periode_verrouillee: 'La date de valeur tombe dans une période verrouillée.',
	date_valeur_dans_periode_echue:
		'La date de valeur tombe dans une période dont l’échéance est passée.',
	periode_accueil_verrouillee:
		'La période qui doit accueillir l’opération tardive est verrouillée.',
	operation_tardive_mal_datee: 'L’opération tardive est mal datée.',
	sequence_epuisee: 'La numérotation du journal est épuisée.',
	ecriture_desequilibree:
		'L’écriture n’est pas équilibrée : total des débits différent du total des crédits.',
	ecriture_une_seule_ligne: 'Une écriture doit comporter au moins deux lignes.',
	ecriture_vide: 'L’écriture ne comporte aucune ligne.',
	validation_compte_inconnu: 'Un compte de l’écriture n’existe pas dans le plan.',
	validation_compte_inactif: 'Un compte de l’écriture est désactivé.',
	validation_compte_de_regroupement: 'Un compte de l’écriture est un compte de regroupement.',
	tiers_manquant: 'Une ligne sur un compte collectif n’a pas de tiers.',
	justification_manquante:
		'Aucune pièce justificative n’est jointe : joignez-en une avant de valider.',
	compte_tresorerie_absent:
		'Ce journal de trésorerie exige un mouvement sur son compte de trésorerie.',
	validation_journal_inconnu: 'Le journal de l’écriture n’existe pas.',
	validation_journal_sans_saisie: 'Le journal de l’écriture n’accepte pas la saisie.',
	validation_date_hors_exercice: 'La date de l’écriture n’appartient à aucun exercice.',
	validation_date_valeur_non_anterieure:
		'La date de valeur doit être antérieure à la date comptable.',
	montant_echelle_excessive: 'Un montant a trop de décimales.',
	ecriture_invalide: 'L’écriture est invalide (montant ou texte refusé).',
	ecriture_non_representable:
		'L’écriture ne peut pas être enregistrée sous la forme figée requise.',
	date_anterieure_a_l_origine:
		'La contre-passation ne peut pas être datée avant l’écriture d’origine.',
	// Pièces.
	type_non_admis: 'Ce type de fichier n’est pas admis (PDF, JPEG ou PNG seulement).',
	// Ré-authentification (422).
	facteurs_refuses: 'Mot de passe ou code refusé.',
	code_invalide: 'Code refusé.'
};

/** Message d'un motif stable, ou `null` s'il est inconnu de l'interface. */
export function messageMotif(motif: string): string | null {
	return Object.hasOwn(MESSAGES, motif) ? (MESSAGES[motif] ?? null) : null;
}

/** Message d'une erreur de l'API : le motif du serveur d'abord, sinon le message général. */
export function messageMetier(erreur: ErreurApi): string {
	const motif = erreur.detail?.motif;
	const base = motif === null || motif === undefined ? null : messageMotif(motif);
	if (base === null) return messageErreur(erreur);
	return erreur.detail?.ligne == null ? base : `Ligne ${erreur.detail.ligne} : ${base}`;
}

/** Vrai si l'erreur est le conflit de version du brouillon (409 `brouillon_perime`). */
export function estConflitDeVersion(erreur: ErreurApi): boolean {
	return erreur.statut === 409 && erreur.detail?.motif === 'brouillon_perime';
}

/** Vrai si le brouillon cité n'est plus modifiable pour une raison de conflit (409). */
export function estConflit(erreur: ErreurApi): boolean {
	return erreur.statut === 409;
}
