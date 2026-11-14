# Réglages GitHub — état et étapes restantes

Ce document suit l'application des réglages décrits dans `docs/collaboration.md` (§5, §7, §12) au dépôt `Sigma-Books/sigma-books` et à l'organisation `Sigma-Books`. Les réglages automatisables sont dans `scripts/github/configurer-depot.sh` ; les autres sont manuels et réservés à un administrateur.

- **Dernier relevé :** 2026-11-14, par lecture de l'API GitHub avec le compte machine (droits `Write`). Les réglages de l'organisation et la politique Actions ne lui sont pas lisibles : ils sont marqués « déclaré » quand le mainteneur a confirmé les avoir appliqués.
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
| Ruleset « Protection de main » | actif | actif : `deletion`, `non_fast_forward`, `required_linear_history`, `required_signatures`, `pull_request` | ✅ |
| Ruleset « Protection des tags de version » | actif | actif | ✅ |
| **Checks requis** dans « Protection de main » | liste du §3 | les 13 checks, sources fixées, branche à jour exigée | ✅ |
| Actions autorisées | GitHub + actions épinglées par hash | politique de l'organisation (voir ci-dessous) | ✅ déclaré |
| Environnement `publication` | approbation requise, tags `v*` | absent | ❌ étape i |
| Catégories Discussions | §4 | catégories par défaut de GitHub | ❌ étape j |
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
| Tableau Projects « Sigma Books » | présent | non lisible (le jeton n'a pas `read:project`) | à vérifier, étape f |

## 2. Simulation du script

`scripts/github/configurer-depot.sh` sans argument (simulation, exécutée le 2026-11-12) propose :
1. les réglages du dépôt (fusion, discussions) ;
2. la sécurité (secret scanning, push protection, Dependabot, PVR) ;
3. la permission en lecture seule du jeton Actions ;
4. le droit `maintain` de `core` ;
5. les 28 étiquettes (`--force`, idempotent) ;
6. rien pour les jalons, qui existent déjà ;
7. rien pour les rulesets, qui existent déjà.

Tous ces réglages sont déjà constatés en place (§1) : relancer `--appliquer` est sans effet nuisible. **Le script ne met pas à jour un ruleset existant** : l'ajout des checks requis se fait par l'interface ou par l'API (§3).

## 3. Checks requis (étape g)

Noms exacts, relevés sur les PR du lot L0 (pile #40 → #50). Tous sont produits à chaque PR, sans filtre `paths`, pour qu'aucun ne reste « en attente ».

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
| `CodeQL` | résultat de l'analyse (échoue sur une nouvelle alerte) | GitHub Advanced Security |
| `sbom` | `sbom.yml` | GitHub Actions |
| `pr-titre` | `pr-titre.yml` | GitHub Actions |

Non requis : `scorecard` (hebdomadaire et sur `main`, pas sur les PR) et `publication.yml` (tags `v*`).

Option « Require branches to be up to date before merging » : **activée**.

Pour l'interface : Settings → Rules → Rulesets → « Protection de main » → *Require status checks to pass* → ajouter les 13 noms ci-dessus.

Équivalent à ajouter dans `scripts/github/ruleset-main.json` (`rules`), si vous préférez versionner la règle. Le fichier relève de `scripts/github/` : il ne sera modifié que sur instruction explicite.

```json
{
  "type": "required_status_checks",
  "parameters": {
    "strict_required_status_checks_policy": true,
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
      { "context": "CodeQL", "integration_id": 57789 },
      { "context": "sbom", "integration_id": 15368 },
      { "context": "pr-titre", "integration_id": 15368 }
    ]
  }
}
```

`integration_id` 15368 = GitHub Actions ; 57789 = GitHub Advanced Security (relevés via l'API des *check runs*). Les fixer empêche un autre outil de publier un statut du même nom.

## 4. Catégories GitHub Discussions (étape j)

| Catégorie | Format | Qui peut ouvrir | À partir de |
|---|---|---|---|
| **Questions** | Questions/réponses | tous | renommer « Q&A » |
| **Idées** | Discussion ouverte | tous | renommer « Ideas » |
| **Annonces** | Annonce | mainteneurs | renommer « Announcements » |
| **Marque et licence** | Questions/réponses | tous | à créer (usage du nom « Sigma Books », AGPL, licence commerciale, CLA) |

Supprimer « General », « Polls » et « Show and tell ». Rappel dans la description de chaque catégorie : **aucune vulnérabilité** dans les discussions (voir `SECURITY.md`).

## 5. Étapes manuelles restantes (administrateur)

Les lettres a à h reprennent celles du script ; i à k sont ajoutées par le lot L0.

- [x] **a.** Organisation → Settings → Authentication security : exiger la 2FA.
- [x] **b.** Organisation → Settings → Member privileges : création de dépôts réservée aux admins.
- [x] **c.** Organisation (ou dépôt) → Settings → Actions → General : autoriser les actions de GitHub et une liste explicite d'actions tierces, et cocher « Require actions to be pinned to a full-length commit SHA ». Actions tierces utilisées : `Swatinem/rust-cache`, `pnpm/action-setup`, `ossf/scorecard-action`, `sigstore/cosign-installer`. Garder « Require approval for all outside collaborators » pour les PR de forks.
- [x] **d.** Compte machine `fhdiop-ccode` en `Write`, hors `core`.
- [ ] **e.** Installer CLA Assistant sur le dépôt (à vérifier).
- [ ] **f.** Tableau Projects « Sigma Books » (colonnes et champs : `docs/collaboration.md` §5), à vérifier.
- [x] **g.** Checks requis : §3, une fois les PR du lot L0 fusionnées.
- [ ] **h.** Second mainteneur : approbations requises à 2 (`docs/collaboration.md` §15).
- [ ] **i.** Settings → Environments → créer **`publication`** : *Required reviewers* = `@Sigma-Books/core`, *Prevent self-review*, *Deployment branches and tags* = tags `v*` uniquement. Utilisé par `publication.yml`.
- [ ] **j.** Catégories Discussions : §4.
- [ ] **k.** (Facultatif) Settings → Code security : activer les motifs génériques du secret scanning (*non-provider patterns*).

## 6. Rappels périodiques

| Quoi | Quand |
|---|---|
| Rotation du jeton fine-grained du compte machine | ≤ 90 jours (`docs/collaboration.md` §2) |
| Champ `Expires` de `site/public/.well-known/security.txt` | avant le 2027-11-12, puis chaque année |
| Revue des membres, équipes, jetons et applications installées | chaque trimestre (`docs/collaboration.md` §12) |
| Mise à jour des versions épinglées des outils de CI (cargo-deny, gitleaks, cargo-cyclonedx) | avec les PR Dependabot ; ces binaires sont épinglés par version et SHA-256 dans les workflows, Dependabot ne les voit pas |
