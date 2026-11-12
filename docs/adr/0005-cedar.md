# 0005 — Autorisation par politiques Cedar

- **Statut :** acceptée — **Date :** 2026-11-12

## Décision
Toutes les décisions d'autorisation passent par **Cedar** (`cedar-policy`, Apache-2.0), avec **refus par défaut**. Politiques versionnées, testées (cas autorisés et refusés) et validées contre un schéma dans la CI.

## Conséquences
Politiques lisibles et auditables par un tiers ; séparation nette entre code et droits. L'autorisation est le premier des trois niveaux d'isolation (Cedar, requête, RLS).
