# Contribuer à Sigma Books

Merci de votre intérêt. Sigma Books est un logiciel comptable : la rigueur et la sécurité passent avant la vitesse.

## Avant de commencer
- Lisez `README.md`, `docs/collaboration.md` (règles de travail) et `CLAUDE.md` (règles techniques absolues).
- Questions : **GitHub Discussions**. Bugs et propositions : **issues** (formulaires fournis).
- **Vulnérabilité** : jamais en issue publique, voir `SECURITY.md`.

## Accord de licence de contribution (CLA)
Avant la fusion de votre première PR, le bot CLA Assistant vous demandera de signer l'accord de contribution. Il permet au projet de rester sous AGPL-3.0 tout en proposant une licence commerciale aux organisations qui en ont besoin.

## Processus
1. Choisissez ou ouvrez une issue ; attendez qu'elle soit « Prête » pour une fonctionnalité.
2. Forkez, créez une branche `<type>/<lot>-<description>` depuis `main`.
3. Commits **signés**, au format Conventional Commits en français (`feat(formats): …`).
4. Avant d'ouvrir la PR :
   ```
   cargo fmt --all
   cargo clippy --all-targets --all-features -- -D warnings
   cargo test --all
   cargo deny check
   ```
5. Ouvrez la PR en remplissant le modèle ; une PR = une intention, idéalement < 400 lignes.
6. Un mainteneur relit ; la PR est fusionnée en *squash*.

## Ce qui sera refusé
- Code avec `unsafe`, montants en flottants, secrets, données réelles de clients, documents de tiers.
- Nouvelle dépendance non justifiée ou sous licence incompatible avec l'AGPL-3.0.
- Code copié d'un autre projet sans licence compatible ni attribution.
- Règles comptables ou fiscales sans référence (SYSCOHADA, AUDCIF, CGI : article, page).

## Code de conduite
Ce projet applique le code de conduite décrit dans `CODE_OF_CONDUCT.md`.
