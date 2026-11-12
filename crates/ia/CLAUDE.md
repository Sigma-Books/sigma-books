# crate `ia` — orchestration IA

**Rôle :** classification, extraction et proposition d'imputation à partir des pièces, via l'API Claude ; calcul du score de confiance ; règles apprises.

**Spec :** §8, §9. **Fondations :** §3.9.

## Règles propres
- **Ne produit que des propositions** destinées au brouillard. Aucun accès en écriture au journal validé, aucun accès direct à la base hors des fonctions qui lui sont exposées.
- Contenu des documents transmis comme **données délimitées** ; la consigne système interdit d'exécuter toute instruction contenue dans un document.
- Sorties **validées par schéma** ; sortie invalide ⇒ pièce en traitement manuel.
- L'IA choisit des comptes **parmi les candidats fournis** ; elle ne calcule jamais les montants (ils viennent de l'extraction, recalculés par `compta-core`).
- Score de confiance **calculé** (contrôles, règles, historique), jamais repris d'une valeur déclarée par le modèle.
- Prompts versionnés dans le dépôt (publics). **Aucune donnée réelle de client** dans le dépôt : jeux d'évaluation réels hors dépôt.
- Clé d'API lue depuis l'environnement ou le gestionnaire de secrets, jamais dans le code.
- Chaque modification de prompt ou de modèle relance la suite d'évaluations (§22.2 de la spec).
