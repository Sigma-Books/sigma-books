# 0003 — Monolithe modulaire et workers isolés

- **Statut :** acceptée — **Date :** 2026-11-12

## Décision
Un **monolithe modulaire** (workspace Cargo : `compta-core`, `syscohada`, `fiscal-sn`, `formats`, `integrite`, `ia`, `authz`, `api`) et des **workers documents isolés** en bac à sable (aucun réseau, aucun accès base) pour les fichiers non fiables.

## Conséquences
Moins de composants, donc moins de surface d'attaque et d'exploitation ; isolation forte là où le risque est réel (analyse de PDF et d'images). Le découpage en crates garde la possibilité d'extraire un service plus tard.

## Alternatives écartées
Microservices : complexité et surface d'attaque disproportionnées à ce stade.
