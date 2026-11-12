# 0002 — Interface SvelteKit statique, site public Astro

- **Statut :** acceptée — **Date :** 2026-11-12

## Décision
- Application : **SvelteKit + TypeScript strict**, compilée en fichiers statiques et servie par le backend Rust. Aucun serveur Node en production.
- Site public et documentation : **Astro** (statique, zéro JS par défaut).

## Conséquences
Surface d'attaque réduite (pas de runtime Node exposé), CSP stricte possible, peu de dépendances. Toute décision d'autorisation reste côté serveur.

## Alternatives écartées
Rust → WebAssembly (Leptos, Dioxus) : écosystème encore jeune pour des écrans métier denses.
