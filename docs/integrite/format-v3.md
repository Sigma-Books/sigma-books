# Intégrité : écriture canonique v2 et export v3 (date de valeur comptable)

Référence : spec §6.2 (invariant 5) et §6.4, AUDCIF art. 22 4° (p. PDF 24), ADR 0007 et 0010, `crates/integrite`.

Ces formats sont **figés** : toute modification crée une nouvelle version. Le vecteur `crates/integrite/tests/vecteurs/export-v3-reference.txt` les vérifie.

## 1. Pourquoi
Une opération dont la **date de valeur comptable** tombe dans une période déjà fermée est enregistrée au premier jour de la plus ancienne période ouverte, et sa date de valeur comptable est « mentionnée distinctement » (AUDCIF art. 22 4°). Cette date est une donnée de l'écriture validée : elle entre dans le contenu haché, ce qui impose une nouvelle version du format canonique (`format-v1.md` §6).

## 2. Écriture canonique v2
Identique à la v1 (`format-v1.md` §2), avec deux différences :
- la première ligne est `ecriture v2` ;
- une ligne `date-valeur-comptable=AAAA-MM-JJ`, **obligatoire**, suit `date-validation=`.

```
ecriture v2
numero=OD-2026-000002
date=2026-02-01
date-validation=2026-02-01
date-valeur-comptable=2026-01-28
libelle=…
piece=…
ligne
…
fin
```

Règles :
- Une écriture **sans** date de valeur comptable reste au format **v1**, octet pour octet. Les empreintes des écritures existantes ne changent donc pas.
- Une écriture **avec** date de valeur comptable est toujours au format v2 : `ecriture v1` suivi de `date-valeur-comptable=` est refusé, tout comme `ecriture v2` sans cette ligne.
- La lecture reste stricte : le texte doit être exactement la sérialisation de l'écriture qu'il décrit.
- `sans-piece=` n'existe pas en v2 : le moteur exige une pièce pour toute écriture (AUDCIF art. 17 et 22).

Ne pas confondre avec la « date de valeur » bancaire (SYSCOHADA, Titre VI) : c'est pourquoi la clé s'appelle `date-valeur-comptable`.

## 3. Export v3
Identique à l'export v2 (`format-v2.md`), avec deux différences :
- l'en-tête est `sigma-books export v3` ;
- chaque écriture est au format canonique **v1 ou v2**. Un export v1 ou v2 n'admet que des écritures v1.

La chaîne de hachage (`format-v1.md` §3), les jetons (`format-v2.md` §3) et le témoin (`format-v2.md` §4) sont inchangés.

Sigma Books écrit la v3. `sigma-verifier` lit les v1, v2 et v3.
