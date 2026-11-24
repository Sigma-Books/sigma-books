# Stockage du développement (WSL2)

Le développement de Sigma Books a saturé le disque de la machine : 288 Go sous WSL, dont 217 Go dans
`.worktrees/` (14 worktrees de 1 à 28 Go). Cette page fixe les causes, les budgets et les règles qui
empêchent le retour du problème. Les règles courtes sont dans `CLAUDE.md` (« Espace disque »).

## Causes constatées (novembre 2026)

- **Un `target/` Rust complet par worktree** (16 à 28 Go chacun) : aucun répertoire de compilation
  partagé, `CARGO_TARGET_DIR` non défini.
- **Symboles de débogage complets** : chaque binaire de test d'intégration de `crates/api` (19
  fichiers dans `tests/`) pesait environ 250 Mo, et `target/debug/build/api` en gardait 187 versions
  successives (17,7 Go dans un seul worktree).
- **Compilation incrémentale** : environ 4,6 Go par worktree.
- **Worktrees jamais retirés** après la fusion de leur PR.
- **Docker** : cache de build de 31 Go (aucun actif), 61 volumes anonymes de bases `postgres` de test
  jamais supprimés.

## Budgets (WSL limité à 100 Go à terme)

| Ressource | Budget |
|---|---:|
| Sigma Books, code et dépendances | 10 Go |
| Worktrees et artefacts de compilation | 15 Go |
| Docker et containerd | 20 Go |
| Autres composants WSL | 20 Go |
| Marge | 35 Go |

Les budgets 1 et 2 tiennent avec les mesures ci-dessous (répertoire partagé d'environ 10 Go,
worktrees de 0,1 Go chacun hors compilation). Le budget Docker tient si le cache BuildKit reste
sous 5 Go.

## Mesures en place

1. **Un répertoire de compilation partagé** : `.worktrees/.cargo/config.toml` (local, non versionné,
   lu par Cargo depuis tout worktree) fixe `target-dir = ".worktrees/.target"` et
   `incremental = false`. Les dépendances (cedar, sqlx, rustls…) ne sont compilées qu'une fois ;
   Cargo verrouille le répertoire, deux compilations simultanées s'attendent. Les crates du
   dépôt sont recompilées par worktree (chemins différents) mais coexistent sans se gêner.
   Le clone principal garde son propre `target/` (0,5 Go).
   **Limite constatée** : ces binaires du dépôt (surtout les tests d'intégration de `api`) sont
   recréés pour chaque worktree et chaque jeu d'options (`--all-features`, `clippy`…) et
   s'accumulent : 35 Go en une journée de trois agents avant que le profil allégé soit fusionné
   (leurs branches avaient encore le débogage complet, 325 Mo par binaire). `scripts/disque-nettoyer.sh`
   les supprime au-delà du plafond de 12 Go ; les dépendances externes sont conservées.
2. **Débogage allégé** (`Cargo.toml`, versionné) : `debug = "line-tables-only"` pour le dépôt,
   `debug = false` pour les dépendances. Les traces de panique gardent les numéros de ligne.
3. **Tests d'intégration non regroupés.** Réunir les 19 fichiers de `crates/api/tests/` en un seul
   binaire ferait gagner de la place, mais ils partagent des objets PostgreSQL à nom fixe (rôle
   `sigma_test_contournement`, base `sigma_test_catalogue_jetable`) et s'exécuteraient dans le même
   processus : risque de courses. La fiabilité des tests prime ; le gain vient de (1) et (2).
4. **Scripts** : `scripts/disque-audit.sh` (lecture seule, seuils d'alerte, `--exiger-libre`) et
   `scripts/disque-nettoyer.sh` (dry-run par défaut, `--appliquer`).
5. **Crochets Claude Code** (`.claude/settings.json`) : ligne d'état de l'espace au démarrage de la
   session ; blocage d'une commande `cargo` lourde s'il reste moins de 12 Go sous le budget
   (`SIGMA_LIBRE_MIN_GO`).

Seuils (`SIGMA_BUDGET_GO`, `SIGMA_ALERTE_PCT`, `SIGMA_CRITIQUE_PCT`) : alerte à 70 %, critique à
85 % du budget.

## Cycle de vie d'un worktree

1. **Créer** seulement pour une PR : `git worktree add -b <branche> .worktrees/<nom> origin/main`.
   Vérifier `scripts/disque-audit.sh --alerte`. Réutiliser un worktree existant et propre plutôt
   qu'en créer un (`git switch -c`).
2. **Travailler** : commits fréquents ; un seul agent compile à la fois.
3. **Valider** : push, PR ouverte.
4. **Nettoyer dès l'ouverture de la PR si plus aucun travail n'est en cours** (sinon après la
   fusion) : `git worktree remove .worktrees/<nom>` (sans `--force` : Git refuse s'il reste du
   travail non commité). La branche et les commits restent.
5. **Après la fusion** : supprimer la branche seulement sur autorisation du mainteneur.
6. En fin de session : `scripts/disque-nettoyer.sh` (dry-run), vérifier, puis `--appliquer`.

Un agent est responsable des ressources qu'il crée : son worktree, ses conteneurs (`sigma-l2-*`,
supprimés en fin de tâche), ses fichiers temporaires.

## Docker

- Cache BuildKit : `scripts/disque-nettoyer.sh --appliquer` exécute
  `docker builder prune --keep-storage 5GB`.
- Conteneurs de test : nom préfixé `sigma-l2-` ou `sigma-test-`, **sans volume anonyme**
  (`docker run --rm`, ou `-v` nommé supprimé avec le conteneur). Un `postgres` lancé sans `--rm` ni
  volume nommé laisse un volume anonyme.
- Politique permanente du ramasse-miettes BuildKit : elle se règle dans `/etc/docker/daemon.json`
  (configuration du démon, hors du dépôt, redémarrage de Docker requis). À appliquer par le
  mainteneur s'il le souhaite :

```json
{ "builder": { "gc": { "enabled": true, "defaultKeepStorage": "5GB" } } }
```

## Ce qui n'est jamais fait

Supprimer une branche, un volume nommé ou une base sans autorisation ; toucher à containerd ou au
VHDX ; parcourir `/mnt/c` ou `/mnt/d` ; déplacer le dépôt vers un autre disque. Le disque Windows ne
récupère l'espace libéré sous WSL qu'après compactage du disque virtuel, opération du mainteneur
hors du dépôt.
