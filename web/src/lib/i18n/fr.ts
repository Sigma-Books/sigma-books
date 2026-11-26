// Catalogue de messages en français : la langue de référence, complète. Les autres catalogues
// (`en.ts`, `pt.ts`, `es.ts`) ne contiennent que ce qui est traduit ; le reste retombe sur celui-ci.
// Un message est un modèle où `{nom}` est remplacé par un paramètre. Aucune donnée du serveur
// ni de la requête ne figure dans un message : seulement des codes stables traduits.

export const fr = {
	// Erreurs générales de l'API (`api/erreurs.ts`).
	'motif.mot_de_passe_trop_court': 'Le mot de passe est trop court (12 caractères au moins).',
	'motif.mot_de_passe_trop_long': 'Le mot de passe est trop long (128 caractères au plus).',
	'motif.mot_de_passe_compromis':
		'Ce mot de passe figure dans une liste de mots de passe compromis.',
	'motif.code_invalide': 'Le code est invalide ou a déjà été utilisé.',
	'motif.jeton_enrolement_invalide': 'La préparation a expiré. Recommencez l’enrôlement.',
	'motif.facteurs_refuses': 'La vérification a échoué.',
	'motif.lien_invalide': 'Ce lien est invalide, expiré ou déjà utilisé.',
	'motif.aucune_passkey': 'Aucune passkey n’est enregistrée sur ce compte.',
	'motif.passkey_refusee': 'La passkey n’a pas été acceptée. Recommencez.',
	'motif.passkey_deja_enregistree': 'Cet appareil ou cette clé est déjà enregistré.',
	'motif.passkeys_max': 'Le nombre maximal de passkeys est atteint.',
	'motif.dernier_facteur': 'Impossible : ce serait votre dernier moyen de connexion.',
	'motif.deja_membre': 'Cette personne est déjà membre du cabinet.',
	'motif.invitation_en_cours': 'Une invitation est déjà en attente pour cette adresse.',
	'motif.invitation_non_en_attente': 'Cette invitation n’est plus en attente.',
	'motif.action_sur_soi_meme': 'Cette action ne peut pas porter sur votre propre compte.',
	'motif.dernier_administrateur': 'Impossible : le cabinet doit garder un administrateur actif.',
	'motif.reinitialisation_en_cours': 'Une réinitialisation est déjà en attente pour ce membre.',
	'motif.reinitialisation_non_en_attente': 'Cette réinitialisation n’est plus en attente.',
	'motif.compte_existant':
		'Un compte existe déjà pour cette adresse. Connectez-vous, puis acceptez l’invitation.',
	'motif.membre_d_un_autre_cabinet':
		'Ce membre appartient aussi à un autre cabinet : sa réinitialisation est impossible ici.',
	'motif.aucun_changement': 'Rien à changer : l’état demandé est déjà celui du serveur.',

	// Codes de l'API sans motif précis.
	'erreur.requete_invalide': 'La demande est invalide.',
	'erreur.non_authentifie': 'Votre session est fermée ou a expiré. Reconnectez-vous.',
	'erreur.interdit': 'Action refusée par le serveur.',
	'erreur.introuvable': 'Élément introuvable.',
	'erreur.methode_non_admise': 'Opération non prise en charge par le serveur.',
	'erreur.corps_trop_grand': 'La demande est trop volumineuse.',
	'erreur.trop_de_requetes': 'Trop de demandes. Réessayez dans un instant.',
	'erreur.trop_de_requetes_delai': 'Trop de demandes. Réessayez dans {secondes} s.',
	'erreur.erreur_interne': 'Erreur du serveur. Réessayez plus tard.',
	'erreur.delai_depasse': 'Le serveur n’a pas répondu à temps. Réessayez.',
	'erreur.reseau': 'Serveur injoignable. Vérifiez votre connexion.',
	'erreur.reponse_invalide': 'Réponse inattendue du serveur.',
	'erreur.conflit': 'Action impossible dans l’état actuel.',
	'erreur.refus': 'La saisie a été refusée.',

	// Motifs de la saisie, du brouillard, de la validation et de la contre-passation.
	'saisie.motif.journal_inconnu': 'Ce journal n’existe pas dans le dossier.',
	'saisie.motif.journal_sans_saisie': 'Ce journal n’accepte pas la saisie manuelle.',
	'saisie.motif.date_hors_exercice': 'Cette date n’appartient à aucun exercice du dossier.',
	'saisie.motif.date_valeur_non_anterieure':
		'La date de valeur doit être antérieure à la date comptable.',
	'saisie.motif.trop_de_lignes': 'L’écriture comporte trop de lignes.',
	'saisie.motif.compte_inconnu': 'Ce compte n’existe pas dans le plan du dossier.',
	'saisie.motif.compte_inactif': 'Ce compte est désactivé.',
	'saisie.motif.compte_de_regroupement':
		'Ce compte est un compte de regroupement : choisissez un compte de détail.',
	'saisie.motif.montant_invalide':
		'Ce montant est refusé (il doit être strictement positif et dans les bornes).',
	'saisie.motif.tiers_inconnu': 'Ce tiers n’existe pas dans le dossier.',
	'saisie.motif.tiers_inactif': 'Ce tiers est désactivé.',
	'saisie.motif.compte_hors_fiche_du_tiers': 'Ce tiers ne peut pas être porté par ce compte.',
	'saisie.motif.brouillon_perime':
		'Ce brouillon a été modifié par quelqu’un d’autre depuis que vous l’avez ouvert. Rechargez-le pour voir sa version actuelle.',
	'saisie.motif.date_dans_periode_cloturee':
		'Cette date tombe dans une période ou un exercice clôturé.',
	'saisie.motif.journal_non_admis_en_cloture':
		'Ce journal n’est pas admis pendant la clôture de l’exercice.',
	'saisie.motif.aucun_changement': 'Rien n’a changé : il n’y a rien à enregistrer.',
	'saisie.motif.drapeau_inchange': 'Le drapeau « à revoir » est déjà dans l’état demandé.',
	'saisie.motif.revue_par_le_poseur':
		'Le drapeau doit être levé par une autre personne que celle qui l’a posé.',
	'saisie.motif.piece_ecartee': 'Cette pièce a été écartée : elle ne peut plus être jointe.',
	'saisie.motif.piece_deja_liee': 'Cette pièce est déjà jointe à ce brouillon.',
	'saisie.motif.piece_principale_existante': 'Ce brouillon a déjà une pièce principale.',
	'saisie.motif.piece_liee':
		'Cette pièce est jointe à un brouillon : elle ne peut pas être écartée.',
	'saisie.motif.trop_de_pieces': 'Trop de pièces sont jointes à ce brouillon.',
	'saisie.motif.lot_sans_ecriture_a_valider':
		'Aucune écriture du lot ne peut être validée : toutes sont marquées « à revoir ».',
	'saisie.motif.lot_refuse':
		'Le lot est refusé : rien n’a été validé. Corrigez les écritures listées ci-dessous.',
	'saisie.motif.periode_verrouillee':
		'La période est verrouillée : déverrouillez-la ou changez la date.',
	'saisie.motif.periode_cloturee': 'La période est clôturée.',
	'saisie.motif.exercice_cloture': 'L’exercice est clôturé.',
	'saisie.motif.exercice_en_cloture': 'L’exercice est en clôture : ce journal n’est pas admis.',
	'saisie.motif.date_valeur_dans_periode_ouverte':
		'La date de valeur tombe dans une période encore ouverte.',
	'saisie.motif.date_valeur_dans_periode_verrouillee':
		'La date de valeur tombe dans une période verrouillée.',
	'saisie.motif.periode_accueil_verrouillee':
		'La période qui doit accueillir l’opération tardive est verrouillée.',
	'saisie.motif.operation_tardive_mal_datee': 'L’opération tardive est mal datée.',
	'saisie.motif.sequence_epuisee': 'La numérotation du journal est épuisée.',
	'saisie.motif.ecriture_desequilibree':
		'L’écriture n’est pas équilibrée : total des débits différent du total des crédits.',
	'saisie.motif.ecriture_une_seule_ligne': 'Une écriture doit comporter au moins deux lignes.',
	'saisie.motif.ecriture_vide': 'L’écriture ne comporte aucune ligne.',
	'saisie.motif.validation_compte_inconnu': 'Un compte de l’écriture n’existe pas dans le plan.',
	'saisie.motif.validation_compte_inactif': 'Un compte de l’écriture est désactivé.',
	'saisie.motif.validation_compte_de_regroupement':
		'Un compte de l’écriture est un compte de regroupement.',
	'saisie.motif.tiers_manquant': 'Une ligne sur un compte collectif n’a pas de tiers.',
	'saisie.motif.justification_manquante':
		'Aucune pièce justificative n’est jointe : joignez-en une avant de valider.',
	'saisie.motif.validation_journal_inconnu': 'Le journal de l’écriture n’existe pas.',
	'saisie.motif.validation_journal_sans_saisie':
		'Le journal de l’écriture n’accepte pas la saisie.',
	'saisie.motif.validation_date_hors_exercice':
		'La date de l’écriture n’appartient à aucun exercice.',
	'saisie.motif.validation_date_valeur_non_anterieure':
		'La date de valeur doit être antérieure à la date comptable.',
	'saisie.motif.montant_echelle_excessive': 'Un montant a trop de décimales.',
	'saisie.motif.ecriture_invalide': 'L’écriture est invalide (montant ou texte refusé).',
	'saisie.motif.ecriture_non_representable':
		'L’écriture ne peut pas être enregistrée sous la forme figée requise.',
	'saisie.motif.type_non_admis': 'Ce type de fichier n’est pas admis (PDF, JPEG ou PNG seulement).',
	'saisie.motif.facteurs_refuses': 'Mot de passe ou code refusé.',
	'saisie.motif.code_invalide': 'Code refusé.',
	'saisie.ligne': 'Ligne {ligne} : {message}',

	// Avertissements d'une validation réussie (l'écriture est validée malgré tout).
	'avertissement.echeance_de_cloture_depassee':
		'L’échéance de clôture de la période commençant le {periode} était le {echeance} : pensez à clôturer cette période.',
	'avertissement.compte_tresorerie_absent':
		'L’écriture du journal {journal} ne mouvemente pas son compte de trésorerie {compte}.',
	'avertissement.inconnu':
		'Le serveur signale un avertissement que cette version de l’interface ne sait pas expliquer.',
	'avertissement.titre.une': 'Écriture validée, avec avertissement :',
	'avertissement.titre.lot': 'Écritures validées, avec avertissements :',
	'avertissement.ecriture': '{numero} : {message}',
	'avertissement.suite': '… et {nombre} autre(s) avertissement(s).',

	// Éditions, intégrité, alertes.
	'anomalie.code.rang_discontinu':
		'Rang discontinu : une écriture manque ou a été insérée dans la chaîne.',
	'anomalie.code.precedente_incorrecte':
		'L’empreinte précédente de l’écriture ne correspond pas à la précédente de la chaîne.',
	'anomalie.code.empreinte_alteree':
		'L’empreinte stockée ne correspond pas au texte stocké de l’écriture.',
	'anomalie.code.contenu_illisible': 'Le texte stocké de l’écriture est illisible.',
	'anomalie.code.colonne_differente': 'Une colonne de l’écriture diffère de son texte stocké.',
	'anomalie.code.tete_absente': 'La tête de chaîne n’est pas enregistrée.',
	'anomalie.code.tete_differente': 'La tête enregistrée diffère de la tête recalculée.',
	'anomalie.code.tete_rang_different':
		'Le rang de la tête enregistrée diffère du nombre d’écritures.',
	'anomalie.code.jeton_hors_chaine':
		'Un jeton d’horodatage atteste une chaîne que la base ne contient pas.',
	'anomalie.code.jeton_invalide': 'La signature d’un jeton d’horodatage est invalide.',
	'anomalie.code.jeton_refuse': 'Un jeton d’horodatage n’est pas accepté (témoin ou autorité).',
	'anomalie.avec_code': 'Anomalie « {code} ».',
	'anomalie.inconnue': 'Anomalie inconnue.',
	'edition.trop_volumineuse':
		'L’édition dépasse {limite} écritures : restreignez la période (dates du et au) ou filtrez par journal ou par compte, puis réessayez.',
	'edition.trop_volumineuse_sans_limite':
		'L’édition dépasse la limite du serveur : restreignez la période (dates du et au) ou filtrez par journal ou par compte, puis réessayez.',
	'edition.filtre_refuse':
		'Le serveur a refusé le filtre : vérifiez l’exercice, les dates (dans l’exercice), les comptes et les journaux.',
	'edition.interdit':
		'Votre rôle ne permet pas de consulter cette édition, ou ce dossier ne vous est pas affecté.',
	'integrite.interdit':
		'Accès réservé aux administrateurs, valideurs et auditeurs affectés au dossier.',
	'integrite.delai':
		'La vérification n’a pas abouti à temps : le serveur est très sollicité. Réessayez dans un instant.',
	'edition.export_annule': 'Export annulé : la ré-authentification n’a pas eu lieu.',
	'edition.export_impossible': 'Le fichier n’a pas pu être préparé.',

	// Libellés des éditions.
	'edition.nature.balance': 'Balance',
	'edition.nature.grand-livre': 'Grand livre',
	'edition.nature.journaux': 'Journaux',
	'edition.nature.centralisateur': 'Centralisateur',
	'edition.colonne.ouverture_debit': 'Ouverture débit',
	'edition.colonne.ouverture_credit': 'Ouverture crédit',
	'edition.colonne.mouvements_debit': 'Mouvements débit',
	'edition.colonne.mouvements_credit': 'Mouvements crédit',
	'edition.colonne.total_debit': 'Total débit',
	'edition.colonne.total_credit': 'Total crédit',
	'edition.colonne.solde_debiteur': 'Solde débiteur',
	'edition.colonne.solde_crediteur': 'Solde créditeur',
	'edition.format.2': 'Balance de travail à 2 colonnes (soldes)',
	'edition.format.4': 'Balance de travail à 4 colonnes (totaux et soldes)',
	'edition.format.6': 'Balance générale à 6 colonnes (AUDCIF art. 19)',
	'edition.format.8': 'Balance de travail à 8 colonnes',
	'edition.exclus.desequilibres': '{nombre} déséquilibré(s)',
	'edition.exclus.illisibles': '{nombre} illisible(s)',
	'edition.exclus':
		'Édition provisoire incomplète : {total} brouillon(s) n’ont pas pu être pris en compte ({details}).',
	'statut.ouverte': 'ouverte',
	'statut.verrouillee': 'verrouillée',
	'statut.cloturee': 'clôturée',
	'unite.brouillon.one': 'brouillon',
	'unite.brouillon.other': 'brouillons',
	'unite.jour.one': 'jour',
	'unite.jour.other': 'jours',
	'alerte.brouillard':
		'Journal {journal} : {brouillons} non validé(s) pour la période du {debut} au {fin}, en retard de {retard} (le plus ancien date du {ancienne}).',
	'alerte.periode': 'période du {debut} au {fin} (exercice {exercice}, {statut})',
	'alerte.echeance_depassee':
		'Échéance de clôture dépassée de {jours} pour la {periode} : échéance le {echeance}.',
	'alerte.echeance_proche': 'Échéance de clôture le {echeance}, dans {jours}, pour la {periode}.',
	'filtre.exercice': 'Le code de l’exercice comporte quatre chiffres (par exemple 2026).',
	'filtre.du': 'La date de début n’est pas une date valide.',
	'filtre.au': 'La date de fin n’est pas une date valide.',
	'filtre.bornes_inversees': 'La date de début est postérieure à la date de fin.',
	'filtre.comptes': 'Un numéro de compte du filtre est illisible.',
	'filtre.journaux':
		'Un code de journal du filtre est illisible (1 à {max} majuscules ou chiffres).',
	'filtre.trop_de_comptes': 'Au plus {max} comptes dans le filtre.',
	'filtre.trop_de_journaux': 'Au plus {max} journaux dans le filtre.',
	'filtre.comptes_exemple': 'Numéros de compte, séparés par des virgules',
	'edition.exercice_dates': 'exercice {code} (du {debut} au {fin})',
	'edition.exercice_code': 'exercice {code}',

	// Saisie.
	'saisie.probleme.montant_illisible':
		'Montant illisible : saisissez un nombre positif, séparateur décimal « {decimal} », milliers par groupes de trois chiffres (par exemple {exemple}).',
	'saisie.probleme.montant_decimales':
		'Trop de décimales : la devise {devise} en admet {decimales} au plus (par exemple {exemple}). Vérifiez le séparateur décimal « {decimal} ».',
	'saisie.probleme.debit_et_credit':
		'Une ligne ne porte qu’un montant : débit ou crédit, pas les deux.',
	'saisie.probleme.montant_absent': 'Cette ligne n’a ni débit ni crédit.',
	'saisie.probleme.compte_absent': 'Cette ligne n’a pas de compte.',
	'saisie.aide_piece':
		'Saisie par pièce : une écriture à la fois, avec ses pièces justificatives. Le montant en devise et l’étiquette fiscale ({taxe}) ne sont pas saisissables ici.',
	'taxe.sigle': 'TVA',

	// Contre-passation.
	'contre_passation.libelle_defaut': 'Contre-passation de {numero}',
	'contre_passation.motif_defaut': 'Contre-passation de {numero}',

	// Export CSV des éditions.
	'csv.dossier': 'Dossier',
	'csv.devise': 'Devise',
	'csv.mode': 'Mode',
	'csv.mode.provisoire': 'provisoire',
	'csv.mode.definitif': 'définitif',
	'csv.exercice': 'Exercice',
	'csv.periode': 'Période',
	'csv.brouillons_exclus': 'Brouillons exclus',
	'csv.compte': 'Compte',
	'csv.tronc': 'Tronc',
	'csv.totaux': 'Totaux',
	'csv.date': 'Date',
	'csv.journal': 'Journal',
	'csv.numero': 'Numéro',
	'csv.date_valeur': 'Date de valeur',
	'csv.libelle': 'Libellé',
	'csv.tiers': 'Tiers',
	'csv.debit': 'Débit',
	'csv.credit': 'Crédit',
	'csv.solde': 'Solde',
	'csv.solde_ouverture': 'Solde d’ouverture',
	'csv.totaux_solde_final': 'Totaux et solde final',
	'csv.totaux_journal': 'Totaux du journal',
	'csv.rubrique': 'Rubrique',
	'csv.mois': 'Mois',
	'csv.mouvements': 'Mouvements',
	'csv.a_nouveaux': 'À-nouveaux',
	'csv.par_compte': '{rubrique} (par compte)',
	'csv.total_du_mois': '{rubrique} (total du mois)',
	'csv.total': '{rubrique} (total)',
	'csv.options': 'Options du fichier CSV',
	'csv.option_separateur': 'Séparateur de colonnes',
	'csv.option_decimal': 'Séparateur décimal',
	'csv.option_bom': 'Marque d’ordre des octets UTF-8 (lecture directe par Excel)',
	'csv.sep.pointvirgule': 'point-virgule ( ; )',
	'csv.sep.virgule': 'virgule ( , )',
	'csv.sep.tabulation': 'tabulation',
	'csv.dec.virgule': 'virgule ( , )',
	'csv.dec.point': 'point ( . )',

	// Libellés des valeurs codées (`libelles.ts`).
	'role.administrateur': 'Administrateur',
	'role.valideur': 'Valideur',
	'role.collaborateur': 'Collaborateur',
	'role.auditeur': 'Auditeur',
	'methode.passkey': 'Passkey',
	'methode.secours': 'Mot de passe et TOTP',
	'methode.code_recuperation': 'Code de récupération',
	'methode.lien_enrolement': 'Lien d’enrôlement',
	'nature_session.complete': 'Complète',
	'nature_session.restreinte': 'Restreinte (enrôlement d’une passkey)',
	'nature_session.enrolement': 'Enrôlement',
	'reinitialisation.appareil_perdu': 'Appareil perdu',
	'reinitialisation.facteurs_perdus': 'Moyens de connexion perdus',
	'reinitialisation.compte_compromis': 'Compte compromis',
	'reinitialisation.depart_reprise': 'Départ ou reprise du poste',
	'reinitialisation.autre': 'Autre motif'
} as const;

/** Clé d'un message du catalogue. */
export type Cle = keyof typeof fr;
