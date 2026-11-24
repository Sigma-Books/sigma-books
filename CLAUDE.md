# Sigma Books — instructions pour Claude Code

Sigma Books est un logiciel comptable **SYSCOHADA révisé** (zone OHADA, Sénégal en premier), multi-dossiers, piloté par l'IA, publié en open source sous **AGPL-3.0-or-later**. Il est conçu « security by design ».

## Sources de vérité (à lire avant toute tâche)
| Document | Rôle |
|---|---|
| `docs/specification.md` | **Ce que** le produit fait (fonctionnel, règles comptables, lots L0 à L9) |
| `docs/fondations.md` | **Comment** il est construit et sécurisé (langage, architecture, sécurité, licence) |
| `docs/adr/` | Décisions d'architecture. Toute nouvelle décision structurante = un nouvel ADR |
| `docs/lots/Lx.md` | Le lot en cours : périmètre, tâches, critères de fin |
| `CLAUDE.md` de chaque crate | Règles propres au crate |

Ne modifie **jamais** `docs/specification.md` ni `docs/fondations.md` sans instruction explicite. Si une tâche contredit ces documents, arrête-toi et signale la contradiction.

## Langue et conventions
- Documentation, commentaires, messages de commit, issues et PR : **français**.
- Identifiants du domaine comptable en français, sans accents (`Ecriture`, `LigneEcriture`, `Compte`, `Exercice`, `Periode`, `Tiers`). Identifiants techniques génériques en anglais acceptés (`Config`, `Error`).
- Commits : Conventional Commits en français (`feat(compta-core): ajoute la contre-passation`), **signés**.

## Flux de travail Git et GitHub
Les règles complètes sont dans **`docs/collaboration.md`**. L'essentiel :
- Tu travailles avec le **compte machine** dédié, jamais avec le compte du mainteneur.
- Une PR = une intention = une branche `<type>/<lot>-<description>`, dans un **worktree dédié**. Une PR peut fermer plusieurs issues d'un même lot.
- Aucune poussée directe sur `main`. PR < 400 lignes si possible ; au-delà, guide la relecture dans la description.
- Rythme (`docs/collaboration.md` §6) : seul le plan du lot attend une validation ; ensuite, enchaîne les PR. Tranche toi-même les choix techniques courants (note-les dans la PR) ; regroupe les vraies questions (§ « Quand s'arrêter ») en un seul message.
- Titre de PR au format Conventional Commits (fusion en squash). Remplis le modèle de PR, applique l'étiquette `ia:généré`, lie l'issue (`Closes #N` : GitHub ne reconnaît que les mots-clés anglais).
- Tu **n'approuves jamais** et **ne fusionnes jamais** une PR. Un humain le fait.
- La fusion passe par la **file de fusion** ; ne rebase sur `main` qu'en cas de conflit.
- Ne modifie jamais `.github/CODEOWNERS`, les workflows de sécurité, les rulesets ni les scripts de `scripts/github/` sans instruction explicite dans l'issue.

## Règles absolues (une PR qui en enfreint une est refusée)
1. **Sécurité mémoire** : `#![forbid(unsafe_code)]` dans toutes les crates. Aucune exception sans ADR.
2. **Montants** : jamais de `f32`/`f64`. Décimal exact (`rust_decimal`) dans le moteur, `NUMERIC` en base.
3. **`compta-core` est pur** : aucune I/O, aucune base de données, aucun réseau, aucune lecture d'horloge système, aucun aléa. Les dates et identifiants sont passés en paramètre.
4. **Invariants comptables dans les types** : une écriture ne peut être enregistrée que si elle est de type `EcritureEquilibree` (cf. `docs/fondations.md` §2.4). Pas de contournement par `pub` sur les champs.
5. **Écriture validée = définitive** : aucune fonction ne modifie ni ne supprime une écriture validée. Correction = contre-passation.
6. **L'IA propose, l'humain valide** : le code IA n'a aucun accès en écriture au journal validé ; il écrit uniquement dans le brouillard.
7. **Refus par défaut** : tout accès passe par une politique Cedar explicite. L'isolation multi-tenant est assurée à trois niveaux (Cedar, requête, RLS PostgreSQL).
8. **Aucun secret dans le dépôt**, même factice réaliste. Valeurs d'exemple uniquement dans `*.example`.
9. **Numéros de comptes et paramètres fiscaux** : uniquement depuis les fichiers de données de `crates/syscohada` et `crates/fiscal-sn`, chaque entrée citant sa source (document, page). Aucun numéro de compte codé en dur ailleurs.
10. **Pas de copie de textes de tiers** : les documents de référence (PDF OHADA, CGI…) ne sont **jamais** ajoutés au dépôt. Ils sont consultables localement (voir ci-dessous). On n'en reprend que des faits (numéros, intitulés, taux) avec citation.
11. **Code porté d'un autre projet** (Tryton, Odoo, OCA, ERPNext…) : uniquement sous licence compatible AGPL-3.0, avec attribution dans `docs/provenance/`. En cas de doute sur la licence : ne pas porter, signaler.
12. **Ne jamais désactiver, contourner ou affaiblir un contrôle de la CI** (lint, test, audit, licence, secrets) pour faire passer une PR.

## Références locales (jamais dans le dépôt)
Textes officiels, documents de travail et moteurs comptables de référence (`references/moteurs_compta/`, Fondations §6.2), en lecture seule, dans le dossier `references/` **à la racine du clone principal**, jamais versionné. Depuis un worktree de `.worktrees/`, il se trouve deux niveaux au-dessus (`../../references/`). S'il est introuvable, demande-le au mainteneur.
**Tout le travail reste dans le dossier du dépôt** : worktrees sous `.worktrees/`, références sous `references/`, rien ailleurs sur le disque.
Le nom `references/` est **ignoré par Git** (`.gitignore`) : ne jamais le forcer (`git add -f`), ne jamais en copier un fichier dans le dépôt.
Lire d'abord son `README.md` (ordre d'autorité, erreurs connues). La liste officielle des comptes est extraite dans `references/extraits/`.

## Espace disque (WSL limité à 100 Go ; détail dans `docs/stockage-developpement.md`)
- **Avant** toute compilation ou création de worktree : `scripts/disque-audit.sh --alerte`. Au-delà de 70 % du budget, nettoyer d'abord (`scripts/disque-nettoyer.sh`, dry-run puis `--appliquer`). Un crochet bloque `cargo` s'il reste moins de 12 Go.
- Les compilations vont dans le répertoire partagé `.worktrees/.target` (configuré par `.worktrees/.cargo/config.toml`). Ne définis pas de `CARGO_TARGET_DIR` ni de `target/` local par worktree ; un seul agent compile à la fois.
- Un worktree par tâche, créé seulement si nécessaire (réutilise un worktree propre). **Retire-le dès la PR ouverte** si le travail est terminé : `git worktree remove` sans `--force`. Jamais de suppression de branche sans autorisation.
- Tu es responsable de ce que tu crées : worktree, conteneurs (préfixe `sigma-l2-` ou `sigma-test-`, lancés avec `--rm` ou un volume nommé, supprimés en fin de tâche), fichiers temporaires. Pas de mot de passe ni de fichier hors du dépôt.
- Jamais de `du` ou de parcours sur tout `.worktrees/`, ni sur `/mnt/c` et `/mnt/d` (interdits). Ne touche ni à containerd ni au VHDX, aucun volume nommé ni base sans autorisation.

## Dépendances
- Toute nouvelle dépendance est justifiée dans la PR (besoin, alternatives, licence, maintenance).
- Préférer la bibliothèque standard et les crates listées dans `docs/fondations.md` §6.3.
- `cargo deny check` doit passer (licences autorisées, avis de sécurité, sources).

## Qualité
- Chaque fonctionnalité métier arrive avec ses tests : unitaires, propriétés (`proptest`) pour les invariants, et cas de référence quand ils existent (exemples chiffrés du Guide d'application SYSCOHADA).
- Les analyseurs de formats externes (CSV, FEC, relevés) ont une cible de fuzzing.
- Avant de proposer une PR : `cargo fmt --all`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo test --all`, `cargo deny check`.

## Quand s'arrêter et demander
- Une règle comptable ou fiscale n'est pas tranchée par la spec (voir `docs/specification.md` §26).
- Une tâche impose d'enfreindre une règle absolue.
- Une licence est incertaine.
- Une décision d'architecture n'est couverte par aucun ADR.
