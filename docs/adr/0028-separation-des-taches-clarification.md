# 0028 — Séparation des tâches : activée par défaut, appliquée tant que le paramètre du cabinet est actif (clarifie les ADR 0014 et 0020)

- **Statut :** proposée
- **Date :** 2026-11-26

## Contexte
Deux textes semblaient se contredire :
- l'ADR 0014 (décisions du mainteneur du 2026-11-18, Q5.3 de `docs/lots/L2.md`) dit la séparation des tâches **activée par défaut**, désactivable par l'administrateur avec ré-authentification, audit et alerte ;
- l'ADR 0020, point 5, dit que le créateur et les correcteurs ne valident pas « **quand le cabinet l'a activée** », ce qui se lit comme un réglage éteint tant que personne ne l'allume.

L'audit des règles bloquantes (principe du mainteneur : faire comme Odoo et Tryton, qui n'ont **aucune** séparation des tâches, `docs/lots/L2.md`, Q5, *Sources*) demandait de dire laquelle des deux formulations est vraie. Cet ADR constate le comportement **réel** du code ; il ne le change pas.

## Décision (constat)
**L'ADR 0014 est vraie : la séparation des tâches est activée par défaut. L'ADR 0020 point 5 est vraie aussi, mais sa formulation est trompeuse : « quand le cabinet l'a activée » signifie « tant que le paramètre du cabinet est actif », et il l'est dès la création.**

Le code le montre :
1. **Valeur par défaut** : `cabinet.separation_taches boolean NOT NULL DEFAULT true` (`crates/api/migrations/0001_organisation.sql`, l. 26). La commande d'administration crée le cabinet sans la renseigner (`crates/api/src/instance.rs`, `INSERT INTO cabinet (id, nom)`) : tout cabinet naît avec la séparation active.
2. **Application** : Cedar (`crates/authz/politiques/invariants.cedar`, `invariant-separation-des-taches`) refuse `ValiderEcriture` quand `principal.cabinet.separation_des_taches` est vrai et que le principal est parmi les `contributeurs` du brouillon (créateur et correcteurs, administrateur compris). L'API relit le paramètre et les contributeurs **en base** après le verrou du brouillon et refuse de même, même si Cedar se trompait (`routes/brouillard/validation.rs`, `if cabinet.separation_taches && b.contributeurs.contains(&utilisateur)` ; `routes/brouillard/lot.rs`, même contrôle par écriture du lot).
3. **Désactivation** : `DesactiverSeparationDesTaches` exige une session fraîche (`invariant-reauthentification`), est tracée dans l'audit du cabinet (champ `separation_taches`, avant et après) et alerte les administrateurs (`SujetNotification::SeparationDesactivee`, test `desactiver_la_separation_des_taches_alerte`). `ActiverSeparationDesTaches` la rétablit. Désactivée, le créateur valide (`sans_separation_le_createur_valide`).
4. **Périmètre** : la séparation vise la **validation** d'un brouillon, unitaire ou par lot. La **contre-passation** n'y est pas soumise (ADR 0021, point 4 : administrateur et valideur, sans séparation des tâches ; spec §6.1, F1.16 accepté).

Conséquence pratique, déjà vraie : un cabinet d'une seule personne ne peut valider ses propres saisies qu'après avoir désactivé la séparation (ré-authentification, audit, alerte).

## Conséquences
- Aucun changement de code ni de comportement.
- À la relecture de l'ADR 0020, lire « quand le cabinet l'a activée » comme « tant que `separation_taches` est vrai (état par défaut) ».
- Écart avec le principe d'alignement sur Odoo et Tryton : aucun des deux n'a de séparation des tâches ; Sigma l'a ajoutée par décision du mainteneur (Q5.3), **conservée** : c'est un réglage du cabinet, désactivable, et non une règle comptable bloquante.

## Alternatives écartées
- Corriger l'ADR 0020 : une décision se complète par un nouvel ADR, elle ne se réécrit pas.
- Désactiver la séparation par défaut : contredirait la décision Q5.3 du mainteneur, que le code applique.
