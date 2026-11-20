# 0021 — Validation par lot et contre-passation : tout ou rien, une contre-passation par origine

- **Statut :** proposée
- **Date :** 2026-11-20

## Contexte
La PR 13c (issue #185, L2-27) ajoute la validation par lot et la contre-passation à la validation unitaire de l'ADR 0020. L'ADR 0011 fixe le lot « tout ou rien » et son autorisation (`decider_lot`), l'ADR 0007 la chaîne, l'ADR 0020 l'ordre des verrous. `compta-core` fournit `contre_passer` à partir de faits (origine relue, état de sa contre-passation, compteur). Tryton (`account.move`, `cancel`) n'empêche pas d'annuler deux fois la même pièce ; Bigcapital verrouille puis vérifie.

## Décision
1. **Lot** : de 1 à 100 écritures, sans doublon, en **une transaction** et dans l'ordre de verrous de l'ADR 0020 (calendrier, brouillons par identifiant, comptes et tiers, pièces, tête, compteurs par journal et exercice, audit). Trois temps : verrous et faits ; validation pure de **toutes** les écritures en mémoire (compteurs et tête avancent) ; insertions. Un seul refus : rien n'est écrit, réponse 409 `lot_refuse` avec la liste `(ecriture, code)` de **tous** les refus. Ordre d'attribution : date comptable, instant de création, identifiant. Les écritures « à revoir » sont **exclues** et comptées (`exclues_a_revoir`) ; sans aucune écriture à valider : 409. Ré-authentification : invariant Cedar, recontrôlé par le cas d'usage. Une trace d'audit par écriture (action `ValiderParLot`).
2. **Autorisation du lot** : `Decideur::decider_lot` et `Garde::decider_lot` (pipeline) enveloppent `Autorisation::decider_lot` : `ValiderParLot` sur le dossier **et** `ValiderEcriture` sur chaque brouillon (séparation des tâches). Le pipeline gagne la portée `Portee::EcritureValidee` (décision sur l'écriture validée lue en base sous RLS ; absente : 404).
3. **Réponse d'erreur d'un cas d'usage** : le pipeline annule la transaction de toute réponse 4xx rendue en `Ok` par un cas d'usage (sans trace exigée ni inscrite). C'est ce qui permet le corps `lot_refuse` sans rendre `ErreurApi` non copiable.
4. **Contre-passation** : faits lus **après** le verrou de la tête de chaîne (origine reconstituée et **vérifiée** : contenu canonique recalculé et empreinte chaînée ; sinon 500), état = numéro de l'écriture qui la cite. **Une origine n'est contre-passée qu'une fois** (choix de conception) : refus 409 `deja_contre_passee`, filet par l'index unique partiel `ecriture_validee_origine_unique` (`0204`). Contre-passer une contre-passation reste admis. Justification, liens de pièces et étiquettes fiscales (contre-passées avec la ligne) sont repris de l'origine ; le compteur est celui du journal pour l'exercice de la **nouvelle** date ; pas de date de valeur. Validée aussitôt, entre dans la chaîne et l'audit comme une validation (motif obligatoire, 400 octets au plus, en audit). Administrateur et valideur, sans ré-authentification ni séparation des tâches (spec §6.1, F1.16 accepté).
5. **Aucune règle comptable dans `api`** : la fonction pure `decider_contre_passation` rend les mêmes résultats que `Livre::contre_passer` (propriété `proptest`).

## Conséquences
- Un lot tient la tête de chaîne du dossier pendant sa durée (borne 100 : de l'ordre de la seconde).
- Les refus d'un lot ne divulguent que des identifiants cités par la requête et des codes stables.
