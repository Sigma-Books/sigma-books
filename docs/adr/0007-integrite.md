# 0007 — Écriture validée définitive et chaîne de hachage

- **Statut :** acceptée — **Date :** 2026-11-12

## Contexte
L'AUDCIF exige le caractère définitif des enregistrements comptables.

## Décision
- Deux états : **brouillard** (modifiable, journalisé) et **validée** (définitive). Correction d'une écriture validée = contre-passation.
- Numérotation continue sans trou par journal et par exercice, attribuée à la validation.
- **Chaîne de hachage SHA-256** par dossier sur une sérialisation canonique ; hash de tête horodaté (RFC 3161) chaque jour et à la clôture.
- Outil de vérification open source pour les auditeurs.

## Conséquences
Inaltérabilité démontrable. Point à confirmer par un expert-comptable : moment du caractère définitif (spec §26, point 1) ; la chaîne reste compatible si le verrou devait intervenir plus tard.
