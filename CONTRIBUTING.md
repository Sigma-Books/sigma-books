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
6. Un mainteneur relit ; la PR est fusionnée en *squash*. Le **titre de la PR** devient le message du commit : il doit respecter Conventional Commits (vérifié par la CI).

## Crochets Git locaux (lefthook)
Les crochets de `lefthook.yml` bloquent avant le commit : les secrets (`gitleaks`), le code Rust mal formaté (`cargo fmt --check`) et les messages hors Conventional Commits (`scripts/verifier-message-commit.sh`). La CI refait les mêmes contrôles ; les crochets évitent seulement des allers-retours.

1. Installez les outils (binaires officiels, vérifiez leur somme SHA-256 publiée) :
   - [lefthook](https://github.com/evilmartians/lefthook/releases) ;
   - [gitleaks](https://github.com/gitleaks/gitleaks/releases) (8.19 ou plus récent) ;
   - la toolchain Rust via [rustup](https://rustup.rs) (la version est fixée par `rust-toolchain.toml`).
2. Dans votre copie du dépôt :
   ```
   lefthook install
   ```
3. Vérifiez : `lefthook run pre-commit`.

Ne contournez pas les crochets (`--no-verify`) : si l'un d'eux se trompe, ouvrez une issue.

## Ce qui sera refusé
- Code avec `unsafe`, montants en flottants, secrets, données réelles de clients, documents de tiers.
- Nouvelle dépendance non justifiée ou sous licence incompatible avec l'AGPL-3.0.
- Code copié d'un autre projet sans licence compatible ni attribution.
- Règles comptables ou fiscales sans référence (SYSCOHADA, AUDCIF, CGI : article, page).

## Code de conduite
Ce projet applique le code de conduite décrit dans `CODE_OF_CONDUCT.md`.
