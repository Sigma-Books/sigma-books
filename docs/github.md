# Réglages GitHub — état et étapes restantes

Ce document suit l'application des réglages décrits dans `docs/collaboration.md` (§5, §7, §12) au dépôt `Sigma-Books/sigma-books` et à l'organisation `Sigma-Books`. Les réglages automatisables sont dans `scripts/github/configurer-depot.sh` ; les autres sont manuels et réservés à un administrateur.

- **Dernier relevé :** 2026-11-17 (fin du lot L0 ; file de fusion préparée par #88), par lecture de l'API GitHub avec le compte machine (droits `Write`). Les réglages de l'organisation et la politique Actions ne lui sont pas lisibles : ils sont marqués « déclaré » quand le mainteneur a confirmé les avoir appliqués.
- **Mise à jour :** à chaque exécution du script ou changement manuel, mettre à jour le tableau du §1 (et la date).

## 1. État des réglages

### Dépôt
| Réglage | Attendu | Constaté | État |
|---|---|---|---|
| Fusion | squash uniquement, titre = titre de PR, message = corps de PR | idem | ✅ |
| Suppression des branches après fusion ; mise à jour de branche ; auto-merge | activés | activés | ✅ |
| Discussions activées, wiki désactivé | oui | oui | ✅ |
| Secret scanning + **push protection** | activés | activés | ✅ |
| Alertes et mises à jour de sécurité Dependabot | activées | activées | ✅ |
| Private Vulnerability Reporting | activé | activé | ✅ |
| `GITHUB_TOKEN` en lecture seule ; workflows sans droit d'approbation | oui | oui | ✅ |
| Équipe `core` | droit `maintain` | `maintain` | ✅ |
| Compte machine `fhdiop-ccode` | rôle `Write`, hors `core` | `write` | ✅ |
| Étiquettes (`.github/labels.yml`) | présentes | présentes | ✅ |
| Jalons L0 à L9 | présents | présents | ✅ |
| Ruleset « Protection de main » | actif, conforme à `ruleset-main.json` (avec `merge_queue` et `code_scanning`, #88) | actif depuis le 2026-11-17 : `deletion`, `non_fast_forward`, `required_linear_history`, `required_signatures`, `pull_request`, `required_status_checks` (12 checks), `merge_queue`, `code_scanning` | ✅ |
| Ruleset « Protection des tags de version » | actif, conforme à `ruleset-tags.json` | actif : `deletion`, `non_fast_forward`, `update`, `required_signatures` ; **ne restreint pas la création** | ✅ |
| Ruleset « Création des tags de version » | actif : `creation` sur `refs/tags/v*`, contournement `core` (`ruleset-tags-creation.json`) | actif depuis le 2026-11-14 (id 27275387) : `creation`, contournement équipe `core` (id 20664322) | ✅ |
| **Checks requis** dans « Protection de main » | liste du §3 (12 checks), file de fusion à la place de « branche à jour » | les 12 checks, sources fixées, « branche à jour » désactivée | ✅ |
| Actions autorisées | GitHub + actions épinglées par hash | politique de l'organisation (voir ci-dessous) | ✅ déclaré |
| Environnement `publication` | approbation requise, tags `v*` | relecteurs `core`, auto-approbation interdite, contournement par les admins interdit, tags `v*` seulement | ✅ |
| Catégories Discussions | §4 | catégories par défaut de GitHub (décision : les garder) | ✅ |
| Code scanning « configuration par défaut » | **désactivée** (analyse avancée via `codeql.yml`) | `not-configured` | ✅ (à laisser ainsi) |
| Secret scanning, motifs génériques (`non_provider_patterns`) | recommandé | désactivé | ⚠️ étape k (facultative) |

### Organisation
| Réglage | Attendu | Constaté | État |
|---|---|---|---|
| 2FA obligatoire | oui | appliqué le 2026-11-14 | ✅ déclaré |
| Permission de base | `read` | `read` | ✅ |
| Création de dépôts | réservée aux admins | appliqué le 2026-11-14 | ✅ déclaré |
| Politique Actions de l'organisation | GitHub + 4 actions tierces listées + épinglage par hash | appliqué le 2026-11-14 | ✅ déclaré |
| CLA Assistant installé | oui | non lisible | à vérifier, étape e |
| Tableau Projects « Sigma Books » | présent | [projet n° 2](https://github.com/orgs/Sigma-Books/projects/2) : colonnes et champs Lot, Priorité, Taille ; issues L0 et L1 rangées | ✅ (automatisations : étape f) |

## 2. Simulation du script

`scripts/github/configurer-depot.sh` sans argument (simulation, exécutée le 2026-11-12) propose :
1. les réglages du dépôt (fusion, discussions) ;
2. la sécurité (secret scanning, push protection, Dependabot, PVR) ;
3. la permission en lecture seule du jeton Actions ;
4. le droit `maintain` de `core` ;
5. les 28 étiquettes (`--force`, idempotent) ;
6. rien pour les jalons, qui existent déjà ;
7. pour les rulesets : vérifie que « Protection de main » et « Protection des tags de version » sont conformes à leurs fichiers, et crée « Création des tags de version », absent (simulation du 2026-11-14).

Relancer `--appliquer` est sans effet nuisible sur ce qui existe. **Le script ne modifie jamais un ruleset existant** : il compare ses règles au fichier versionné et signale les écarts. Il crée seulement les rulesets absents, dont « Création des tags de version » (étape l).

## 3. Checks requis (étape g)

Noms exacts, relevés sur les PR du lot L0 (pile #40 → #50). Tous sont produits à chaque PR **et à chaque groupe de la file de fusion** (déclencheur `merge_group`), sans filtre `paths`, pour qu'aucun ne reste « en attente ».

| Check | Workflow | Source |
|---|---|---|
| `rust-fmt` | `rust.yml` | GitHub Actions |
| `rust-clippy` | `rust.yml` | GitHub Actions |
| `rust-test` | `rust.yml` | GitHub Actions |
| `rust-deny` | `rust.yml` | GitHub Actions |
| `web` | `web.yml` | GitHub Actions |
| `site` | `web.yml` | GitHub Actions |
| `gitleaks` | `secrets.yml` | GitHub Actions |
| `codeql (rust)` | `codeql.yml` | GitHub Actions |
| `codeql (javascript-typescript)` | `codeql.yml` | GitHub Actions |
| `codeql (actions)` | `codeql.yml` | GitHub Actions |
| `sbom` | `sbom.yml` | GitHub Actions |
| `pr-titre` | `pr-titre.yml` | GitHub Actions |

Non requis : `scorecard` (hebdomadaire et sur `main`, pas sur les PR) et `publication.yml` (tags `v*`).

Le résultat de l'analyse CodeQL (check `CodeQL`, publié par GitHub Advanced Security) n'est plus un check requis : GitHub ne le publie pas pour les groupes de la file de fusion, qui resteraient bloqués. Il est remplacé par la règle **`code_scanning`** du ruleset, aux mêmes seuils que le check par défaut : la PR est bloquée par toute nouvelle alerte de sécurité élevée ou critique, ou toute erreur. Les trois analyses `codeql (…)` restent requises et tournent aussi dans la file.

Option « Require branches to be up to date before merging » : **désactivée** depuis #88. La **file de fusion** offre la même garantie sans rebase ni réapprobation : chaque groupe repasse les checks requis sur la PR combinée au dernier `main` avant la fusion.

Cette règle est versionnée dans `scripts/github/ruleset-main.json` (`rules`) :

```json
{
  "type": "required_status_checks",
  "parameters": {
    "strict_required_status_checks_policy": false,
    "do_not_enforce_on_create": false,
    "required_status_checks": [
      { "context": "rust-fmt", "integration_id": 15368 },
      { "context": "rust-clippy", "integration_id": 15368 },
      { "context": "rust-test", "integration_id": 15368 },
      { "context": "rust-deny", "integration_id": 15368 },
      { "context": "web", "integration_id": 15368 },
      { "context": "site", "integration_id": 15368 },
      { "context": "gitleaks", "integration_id": 15368 },
      { "context": "codeql (rust)", "integration_id": 15368 },
      { "context": "codeql (javascript-typescript)", "integration_id": 15368 },
      { "context": "codeql (actions)", "integration_id": 15368 },
      { "context": "sbom", "integration_id": 15368 },
      { "context": "pr-titre", "integration_id": 15368 }
    ]
  }
}
```

`integration_id` 15368 = GitHub Actions (relevé via l'API des *check runs*). Le fixer empêche un autre outil de publier un statut du même nom.

File de fusion et analyse de code, dans le même fichier :

```json
{ "type": "merge_queue", "parameters": { "merge_method": "SQUASH", "grouping_strategy": "ALLGREEN", "max_entries_to_build": 5, "min_entries_to_merge": 1, "max_entries_to_merge": 5, "min_entries_to_merge_wait_minutes": 1, "check_response_timeout_minutes": 60 } },
{ "type": "code_scanning", "parameters": { "code_scanning_tools": [ { "tool": "CodeQL", "security_alerts_threshold": "high_or_higher", "alerts_threshold": "errors" } ] } }
```

`ALLGREEN` : un groupe n'est fusionné que si tous ses checks sont verts ; en cas d'échec, la PR fautive est retirée de la file et les autres sont retestées sans elle.

## 4. Catégories GitHub Discussions (étape j)

Décision du mainteneur (2026-11-14) : **conserver les catégories par défaut de GitHub, en anglais** (Announcements, General, Ideas, Polls, Q&A, Show and tell). Les catégories francisées proposées initialement (Questions, Idées, Annonces, Marque et licence) ne sont pas retenues.

Rappel utile dans la description des catégories : **aucune vulnérabilité** dans les discussions (voir `SECURITY.md`).

## 5. Étapes manuelles restantes (administrateur)

Les lettres a à h reprennent celles du script ; i à l sont ajoutées par le lot L0, m par #88.

- [x] **a.** Organisation → Settings → Authentication security : exiger la 2FA.
- [x] **b.** Organisation → Settings → Member privileges : création de dépôts réservée aux admins.
- [x] **c.** Organisation (ou dépôt) → Settings → Actions → General : autoriser les actions de GitHub et une liste explicite d'actions tierces, et cocher « Require actions to be pinned to a full-length commit SHA ». Actions tierces utilisées : `Swatinem/rust-cache`, `pnpm/action-setup`, `ossf/scorecard-action`, `sigstore/cosign-installer`. Garder « Require approval for all outside collaborators » pour les PR de forks.
- [x] **d.** Compte machine `fhdiop-ccode` en `Write`, hors `core`.
- [ ] **e.** Installer CLA Assistant sur le dépôt (à vérifier).
- [x] **f.** Tableau Projects « Sigma Books » créé par le compte machine ([n° 2](https://github.com/orgs/Sigma-Books/projects/2)). Reste à régler dans l'interface (⋯ → *Workflows*, non disponible par l'API) : *Item added to project* → `Backlog` ; *Item closed* et *Pull request merged* → `Terminé` ; *Auto-add to project* depuis `Sigma-Books/sigma-books`, filtre `is:issue,pr is:open`.
- [x] **g.** Checks requis : §3, une fois les PR du lot L0 fusionnées.
- [ ] **h.** Second mainteneur : approbations requises à 2 (`docs/collaboration.md` §15).
- [x] **i.** Settings → Environments → créer **`publication`** : *Required reviewers* = `@Sigma-Books/core`, *Prevent self-review*, *Deployment branches and tags* = tags `v*` uniquement. Utilisé par `publication.yml`.
- [x] **j.** Catégories Discussions : catégories par défaut conservées (§4).
- [ ] **k.** (Facultatif) Settings → Code security : activer les motifs génériques du secret scanning (*non-provider patterns*).
- [x] **l.** Créer le ruleset « Création des tags de version » : `scripts/github/configurer-depot.sh --appliquer` avec le compte administrateur (le script résout l'identifiant de l'équipe `core`). Le compte machine vérifie ensuite par l'API.
- [x] **m.** (#88) **Après la fusion de la PR de #88**, sinon cette PR resterait bloquée dans la file, appliquer `ruleset-main.json` au ruleset existant « Protection de main » (id 27145400). Le script ne modifie pas un ruleset existant ; depuis un clone à jour de `main`, avec le compte administrateur :
  ```
  gh api -X PUT repos/Sigma-Books/sigma-books/rulesets/27145400 --input scripts/github/ruleset-main.json
  ```
  Ou dans l'interface (Settings → Rules → Rulesets → « Protection de main ») : cocher *Require merge queue* (squash, *Only merge non-failing pull requests*, timeout 60 min) ; décocher *Require branches to be up to date* ; retirer le check `CodeQL` ; cocher *Require code scanning results* (CodeQL, alertes de sécurité *High or higher*, alertes *Errors*). Le compte machine vérifie ensuite par l'API.

## 6. Rappels périodiques

| Quoi | Quand |
|---|---|
| Rotation du jeton fine-grained du compte machine | ≤ 90 jours (`docs/collaboration.md` §2) |
| Champ `Expires` de `site/public/.well-known/security.txt` | avant le 2027-11-12, puis chaque année |
| Revue des membres, équipes, jetons et applications installées | chaque trimestre (`docs/collaboration.md` §12) |
| Exception d'audit `GHSA-fc5q-39xc-vjpf` (picomatch, `web/` et `site/`, `auditConfig.ignoreGhsas`) | dès qu'une version corrigée existe : mettre à jour et retirer l'exception ; sinon revoir avant le 2026-12-17 |
| Mise à jour des versions épinglées des outils de CI (cargo-deny, gitleaks, cargo-cyclonedx) | avec les PR Dependabot ; ces binaires sont épinglés par version et SHA-256 dans les workflows, Dependabot ne les voit pas |
