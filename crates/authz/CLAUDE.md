# crate `authz` — autorisation

**Rôle :** politiques **Cedar** (refus par défaut), schéma Cedar des entités (cabinet, dossier, utilisateur, rôle, ressource), évaluation des requêtes.

**Spec :** §20. **Fondations :** §3.5.

## Règles propres
- Politiques dans des fichiers `.cedar` versionnés ; aucune décision d'autorisation codée en dur ailleurs.
- Chaque politique a des tests : cas autorisés **et** cas refusés (notamment accès inter-cabinets, inter-dossiers, client vers écritures).
- Analyse automatique des politiques (validation contre le schéma) dans la CI.
- Toute modification de politique est revue par deux personnes (CODEOWNERS).
