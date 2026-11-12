# crate `compta-core` — moteur comptable

**Rôle :** le cœur du logiciel. Types et règles de la comptabilité en partie double SYSCOHADA : comptes, journaux, exercices, périodes, écritures (brouillard → validation), contre-passation, numérotation, balance, grand livre, lettrage, TVA, amortissements, fin d'exercice, clôture, à-nouveaux, répartition, calcul des états financiers à partir des correspondances fournies par `syscohada`.

**Spec :** `docs/specification.md` §4 à §17. **Fondations :** §2.4, §7.

## Règles propres
- **Pur** : aucune I/O, aucune base, aucun réseau, aucun `SystemTime::now()`, aucun aléa. Tout vient des paramètres.
- **Dépendances minimales** : `rust_decimal`, `thiserror`, éventuellement `serde` (derive) et `time`/`chrono` pour les types de date. Rien d'autre sans ADR.
- **États invalides irreprésentables** : constructeurs privés + fonctions de validation qui renvoient `Result`. Exemples : `NumeroCompte::parse`, `EcritureBrouillon` → `valider_equilibre()` → `EcritureEquilibree`.
- **Aucune modification d'une écriture validée.** Pas de `&mut` exposé sur une écriture validée.
- Montants arrondis selon la devise (XOF : 0 décimale à l'export, calculs internes exacts).
- Les numéros de comptes ne sont pas codés en dur : ils viennent du référentiel passé en paramètre.

## Tests exigés
- Propriétés (`proptest`) : toute suite d'écritures valides donne une balance équilibrée ; contre-passation = annulation exacte ; à-nouveaux = soldes de clôture des classes 1 à 5 ; lettrage total ⇒ solde nul.
- Cas de référence du Guide d'application SYSCOHADA reproduits **exactement** (ex. CAFG = 241 773 904).
- Couverture visée ≥ 90 % des lignes.
