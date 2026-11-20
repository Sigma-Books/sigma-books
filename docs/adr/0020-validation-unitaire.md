# 0020 — Validation unitaire d'une écriture : verrous, séparation des tâches, échelle des montants

- **Statut :** proposée
- **Date :** 2026-11-20

## Contexte
La PR 13b (issue #184, L2-26) persiste la validation : numéro sans trou, chaîne de hachage, audit, écriture définitive. L'ADR 0007 fixe la chaîne, l'ADR 0011 l'ordre des verrous et les droits (ajout seul), l'ADR 0014 l'audit, l'ADR 0015 le lien des pièces. Cet ADR en précise l'application et acte la décision sur les décimales du XOF (commentaire du 2026-11-20 de l'issue #184).

Ce que font les moteurs de référence : Tryton (`account.move.post`) numérote à la validation par une séquence incrémentée sous verrou ; Odoo (`sequence_mixin`) déduit le numéro du dernier enregistré, avec des trous possibles ; Dolibarr (`blockedlog`) chaîne après coup et numérote par `MAX + 1` sans verrou. Aucun ne prend un verrou de chaîne unique par dossier ni ne refuse la validation dans une période clôturée par une règle pure testée contre un livre de référence.

## Décision
1. **Ordre unique des verrous**, pris après la décision Cedar, dans la transaction du pipeline : exercices puis périodes du dossier (`FOR SHARE`, ordre chronologique) ; brouillon (`FOR UPDATE`) ; **comptes et tiers cités par les lignes** (`FOR SHARE`, par ordre croissant : un compte désactivé concurremment attend la fin de la validation) ; pièces liées (`FOR SHARE`, par identifiant) ; tête de chaîne (`FOR UPDATE`) ; compteur du journal et de l'exercice (`FOR UPDATE`) ; tête d'audit du dossier (prise à l'inscription des traces). Le référentiel, le brouillon complet, les contributeurs et les statuts sont relus **après** les verrous. L'**instant de validation** (`valide_le`, date de validation) est relu sur l'horloge de l'application juste après la tête de chaîne : la validation a pu attendre, ou franchir minuit. Une période clôturée reste visible sous `FOR SHARE` : c'est `compta_core::valider` qui la refuse. Les maillons « comptes et tiers » s'intercalent dans l'ordre de l'ADR 0011 sans le contredire.
2. **La tête de chaîne sérialise** les validations d'un dossier : aucun verrou de ligne n'est possible sur une écriture validée (ajout seul). La tête et le compteur sont des états dérivés, créés à la demande (`INSERT … ON CONFLICT DO NOTHING`, puis `SELECT … FOR UPDATE`), avec `UPDATE` sur leurs seules colonnes. L'échec de toute étape, audit compris, annule la transaction : aucun numéro n'est consommé.
3. **Aucune règle de validation dans `api`** : équilibre, période, exercice, échéance, opération tardive, comptes, tiers, justification, journal de trésorerie et numéro viennent de `compta-core`. `api` traduit chaque motif en code stable (409 pour l'état du dossier, 422 pour l'écriture). La fonction d'application pure rend les mêmes numéros, contenus canoniques et refus que `Livre` (propriété `proptest`).
4. **Le contenu canonique est stocké** avec sa version de format (1, 2 ou 3, déduite du contenu par `integrite`) et l'empreinte précédente : la chaîne se recalcule depuis la base, et se compare à un export, sans réimplémenter le format.
5. **Séparation des tâches et rôle relus en base** après le verrou du brouillon (une délégation retirée pendant l'attente est vue), même si Cedar décide : le créateur et les correcteurs ne valident pas (administrateur compris) quand le cabinet l'a activée ; un collaborateur ne valide qu'avec la délégation du dossier.
6. **Décimales du XOF** : acceptées à la saisie, exactes en base (aucun texte n'impose ni n'interdit une précision décimale). `compta-core` n'impose aucune échelle à une ligne en XOF ; seule la contre-valeur d'une ligne en devise est arrondie au franc (choix du produit, spec §6.6). Le format canonique ne relit pas plus de 28 chiffres : la validation refuse (422 `montant_echelle_excessive`) tout montant XOF, tout montant en devise et tout cours qui les dépasserait, puis vérifie que le contenu haché se relit à l'identique. **Un écart d'arrondi ou de change est équilibré par une ligne explicite saisie par l'utilisateur, jamais ajoutée par le serveur.**
7. **Cohérence étiquettes ↔ comptes de TVA** (`controler_taxes_et_comptes`) : non appliquée à la validation, faute d'une donnée de `syscohada` qui désigne les comptes de TVA collectée et déductible (règle absolue 9). À brancher quand cette donnée existera (L5).

7 bis. **Seconde barrière** : un déclencheur `BEFORE UPDATE` sur `piece` (`0352`) refuse d'écarter une pièce liée à un brouillon ou à une écriture validée, même par SQL direct.

## Limites connues (à traiter plus tard)
- La **vérification de la chaîne depuis la base** n'existe qu'en test (recalcul des empreintes depuis le texte stocké, export et `sigma-verifier`) : elle est à prévoir dans une PR ultérieure.
- Aucune contrainte en base ne lie `numero` au journal et à l'exercice de la ligne ; le filet est l'unicité (dossier, journal, exercice, numéro) et celle de la séquence.
- `version_parametres` est stockée mais **jamais renseignée** (le brouillard ne la porte pas) : à rattacher à L5.
- L'**ordre des pièces** de la justification suit l'identifiant de la pièce, non l'ordre de rattachement (le brouillon ne conserve pas cet ordre) ; il est figé dans le contenu haché à la validation.
- Les états en **francs entiers** arrondissent des décimales conservées exactes en base : ils peuvent ne pas s'équilibrer. À traiter dans le lot des états.

## Conséquences
- Une validation prend la tête de chaîne du dossier pendant sa durée (de l'ordre de la milliseconde) : les validations d'un même dossier sont sérialisées, celles de dossiers distincts ne se gênent pas.
- La contre-passation et la validation par lot (PR 13c) réutilisent la tête, le compteur et la colonne `origine_id`.
