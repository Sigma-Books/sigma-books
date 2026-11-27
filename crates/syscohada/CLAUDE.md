# crate `syscohada` — référentiel normatif

**Rôle :** données normatives du SYSCOHADA révisé : liste des comptes (2 à 4 chiffres, intitulés, classe, nature, sens), troncs, correspondances postes ↔ comptes du Bilan, du Compte de résultat et du TFT, liste des notes annexes.

**Spec :** §5.1, §17.3. **Fondations :** §6, §7.

## Règles propres
- Les données vivent dans des **fichiers de données versionnés** (`data/*.toml` ou `*.json`), chargés et validés au démarrage ou à la compilation (`include_str!` + test de validation).
- **Chaque entrée cite sa source** : document, page, version. Exemple : `source = "AUDCIF+SYSCOHADA, JO OHADA 15/02/2017, Titre VII §3, p. PDF 249"`.
- Source qui fait foi : la **liste officielle des comptes** de l'AUDCIF (références locales : `references/officiel/AUDCIF_…pdf`, extrait `references/extraits/SYSCOHADA_liste_des_comptes_OCR.txt`). L'extrait est issu d'une OCR : en cas de doute, vérifier sur le PDF.
- Les tableaux de `references/travail/` ne sont **pas** officiels : ne pas les reprendre sans contrôle contre les modèles officiels de l'AUDCIF (erreur connue : poste BB `21` au lieu de `31`).
- Aucun texte de tiers recopié au-delà des numéros et intitulés de comptes et des libellés de postes.

## Tests exigés
- Unicité des numéros ; chaque compte a un parent cohérent ; chaque tronc référencé par une correspondance existe.
- Chaque poste du Bilan, du CdR et du TFT a une correspondance ; aucun compte de bilan n'est orphelin.
- **Pack pays** (`data/pays.toml`, ADR 0027) : valeurs par défaut d'un dossier par pays de l'OHADA (devise de tenue et échelle, fuseau à décalage constant, langue, types d'identifiants légaux). Aucun paramètre fiscal (ils relèvent de `fiscal-*`). Une devise, un fuseau ou un type d'identifiant s'ajoute par une ligne de données **avec sa source** ; un pays à heure d'été exige un nouvel ADR.
- **Tiers et trésorerie** (L3-12, ADR 0032 et 0034) : marques `lettrable` (héritée, jamais sur un compte admis en journal de trésorerie, jamais redondante) et `type_tresorerie` (un compte par type, admis en journal de trésorerie) dans `comptes.toml`, chacune avec la page de sa fiche ; rôles de comptes désignés par le dossier dans `roles-comptes.toml` (codes fermés, suggestions sourcées « à confirmer », aucune suggestion sans source). Un compte non sourcé n'est pas marqué.
