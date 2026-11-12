# Développement sur GitHub — règles du projet

Ce document fixe **comment** on travaille sur `github.com/Sigma-Books/sigma-books`. Il s'applique à tous les contributeurs, humains ou agents IA (Claude Code).

> **Mode actuel : un seul mainteneur humain + Claude Code.** Les règles sont celles d'une équipe. Les seuls aménagements propres au mode solo sont signalés par 🧍 ; ils sont levés au passage en équipe (§15).

---

## 1. Principes
1. **`main` est toujours livrable** : tout ce qui y entre a passé la CI et une revue humaine.
2. **Petites PR, souvent** : une PR = une intention, idéalement moins de 400 lignes modifiées hors fichiers générés.
3. **Tout passe par une PR**, y compris pour les mainteneurs. Aucune poussée directe sur `main`.
4. **Rien n'est fusionné sans revue humaine.** Un agent IA peut ouvrir une PR, jamais l'approuver ni la fusionner.
5. **La trace écrite fait foi** : décisions dans les ADR, discussions dans les issues et les PR.

## 2. Identités
| Qui | Compte | Rôle |
|---|---|---|
| Mainteneur(s) | Comptes personnels, membres de l'équipe `@Sigma-Books/core` | Revue, approbation, fusion, publication |
| Claude Code | **Compte machine dédié** `fhdiop-ccode` | Branches, commits signés, PR |

Pourquoi un compte dédié pour l'agent : GitHub interdit d'approuver sa propre PR. Si l'agent poussait avec le compte du mainteneur, celui-ci ne pourrait plus approuver les PR. Le compte dédié rend aussi visible ce qui a été produit par l'IA.

Tous les comptes : **2FA obligatoire** (réglage d'organisation), **commits signés** (clé SSH de signature déclarée sur GitHub).

Compte machine de Claude Code :
- rôle **Write** sur le dépôt (jamais Maintain ni Admin) ;
- **jeton fine-grained** limité au seul dépôt `sigma-books` : Contents, Pull requests, Issues en lecture/écriture ; Workflows en écriture (nécessaire pour les fichiers de CI) ; Metadata en lecture ; **expiration 90 jours** ;
- sa propre clé SSH de signature ; `user.name` / `user.email` configurés dans les worktrees de l'agent uniquement ;
- aucun accès aux secrets de déploiement (§12).

🧍 **En solo, tout changement du dépôt passe par le compte machine**, y compris ceux que le mainteneur décide lui-même (mise à jour de la spec après une décision, correction d'une coquille) : il les demande à Claude Code, puis relit et approuve. Il ne pousse jamais lui-même sur une branche. Sinon, la règle « dernier push approuvé par quelqu'un d'autre » (§7) le bloquerait, puisqu'on ne peut pas approuver sa propre PR.

## 3. Modèle de branches : trunk-based
- Une seule branche longue, `main`, protégée par un *ruleset* (§7).
- Branches de travail courtes (quelques jours au plus), créées depuis `main`, supprimées après fusion.
- Nommage : `<type>/<lot>-<description-courte>`, en minuscules, mots séparés par des tirets.
  - Exemples : `feat/l1-ecriture-equilibree`, `fix/l3-lettrage-partiel`, `docs/adr-0010-devises`, `ci/l0-codeql`.
  - Types : `feat`, `fix`, `refactor`, `perf`, `test`, `docs`, `ci`, `build`, `chore`, `security`.
- Plusieurs agents en parallèle : **un worktree Git par branche** (`git worktree add ../sigma-l1-ecritures feat/l1-ecritures`).

## 4. Commits
- **Conventional Commits en français** : `type(portée): description à l'impératif`.
  - `feat(compta-core): ajoute la contre-passation`
  - `fix(formats): refuse les FEC dont l'en-tête est incomplet`
  - Portée = nom du crate ou du dossier (`compta-core`, `api`, `web`, `ci`, `docs`…).
  - Changement incompatible : `!` après la portée et une ligne `BREAKING CHANGE:` dans le corps.
- Commits **signés** (vérifiés par GitHub).
- Pas de secrets, pas de fichiers générés, pas de documents de tiers.
- **Crochets locaux** (`lefthook`, installés par chaque contributeur et par l'agent) : `gitleaks protect --staged`, `cargo fmt --check`, validation du message de commit. Ils évitent des allers-retours ; la CI reste la référence.

## 5. Issues et planification
- **Tout travail part d'une issue** (sauf correction triviale), créée avec un formulaire : `bug`, `fonctionnalité`, `tâche`.
- **Jalons** : un par lot, de `L0 — Fondations` à `L9 — Durcissement`.
- **Étiquettes** (définies dans `.github/labels.yml`) :
  - `type:` bug, fonctionnalité, tâche, sécurité, documentation, dette
  - `zone:` compta-core, syscohada, fiscal-sn, formats, integrite, ia, authz, api, worker, web, site, ci
  - `priorité:` critique, haute, normale, basse
  - `statut:` à-préciser, bloqué, besoin-expert-comptable
  - `ia:généré` pour les PR produites par un agent
  - `good first issue`, `help wanted` pour les contributeurs externes
- **Tableau GitHub Projects** « Sigma Books » : *Backlog → Prêt → En cours → En revue → Terminé* ; champs *Lot*, *Priorité*, *Taille (XS à XL)*.
- Une issue « Prête » a : un objectif clair, des critères d'acceptation, le lot, la zone et la référence à la spec (§).

## 6. Pull requests
- Ouvrir tôt en **brouillon** si le travail est long ; passer en « prête » quand la CI est verte.
- **Titre = message de commit final** (Conventional Commits), car on fusionne en *squash*.
- Corps : modèle `.github/pull_request_template.md` (`Ferme #N`, référence spec/ADR, liste de contrôle sécurité, tests).
- Une PR ne mélange pas refactorisation et changement de comportement.
- Mise à jour par **rebase** sur `main`, pas par fusion de `main` dans la branche.
- **Fusion : squash uniquement**, suppression automatique de la branche. Auto-merge autorisé une fois l'approbation et la CI acquises.

## 7. Protection de `main` (ruleset)
| Règle | Valeur |
|---|---|
| PR obligatoire | oui |
| Approbations requises | **1** tant qu'il n'y a qu'un mainteneur humain ; **2** dès qu'il y en a deux |
| Revue des CODEOWNERS obligatoire | oui |
| Approbations périmées à chaque nouveau commit | oui |
| Approbation du dernier push par quelqu'un d'autre que son auteur | oui |
| Toutes les conversations résolues | oui |
| Checks de CI requis, branche à jour | oui (liste fixée à la fin du lot L0) |
| Historique linéaire | oui |
| Commits signés | oui |
| Force-push et suppression de `main` | interdits |
| Contournement (bypass) | personne, y compris les administrateurs |

Les tags `v*` sont protégés par un second ruleset (création réservée à l'équipe `core`, ni suppression ni modification).

Les rulesets sont versionnés dans `scripts/github/ruleset-main.json` et `ruleset-tags.json`, et appliqués par l'administrateur avec `scripts/github/configurer-depot.sh`.

🧍 **Urgence** (compte machine indisponible, CI cassée par un service externe) : l'administrateur peut désactiver temporairement le ruleset. C'est journalisé par GitHub ; il le réactive immédiatement après et le signale dans la PR concernée. Aucun bypass permanent n'est configuré.

## 8. Revue de code
- **CODEOWNERS** (`.github/CODEOWNERS`) attribue automatiquement les relecteurs par zone.
- Zones sensibles (`compta-core`, `integrite`, `authz`, `api`, `fiscal-sn`, `syscohada`, `.github/`) : revue par `@Sigma-Books/core` obligatoire.
- Le relecteur vérifie : conformité à la spec et aux ADR, règles absolues de `CLAUDE.md`, tests (dont propriétés et cas de référence), sécurité (autorisation, isolation, entrées hostiles, secrets, dépendances), lisibilité.
- **Code produit par une IA** : même exigence, avec une attention particulière aux tests qui « passent pour de mauvaises raisons », aux dépendances ajoutées et aux règles comptables.
- Commentaires préfixés : `bloquant:`, `suggestion:`, `question:`, `détail:`.
- Délai de première revue visé : 2 jours ouvrés (en équipe).
- 🧍 En solo, le mainteneur relit chaque PR de l'agent avec la liste de contrôle du modèle de PR. Pour les zones sensibles, il peut demander à une **seconde session Claude Code, sans droit d'écriture**, une revue critique en commentaire. Cette revue IA complète la revue humaine ; elle ne la remplace pas et n'approuve jamais.

## 9. La CI comme garde-fou
- Tous les workflows sont **requis** sur `main` (voir `docs/lots/L0.md`).
- Actions **épinglées par hash de commit** ; `GITHUB_TOKEN` en lecture seule par défaut, permissions élargies job par job.
- Workflows déclenchés par des PR venant de forks : **approbation manuelle**, aucun secret exposé.
- On ne désactive jamais un contrôle pour faire passer une PR : on corrige ou on ouvre une issue.

## 10. Dépendances
- **Dependabot** (`.github/dependabot.yml`) : mises à jour hebdomadaires groupées pour Cargo, npm (`web/`, `site/`) et GitHub Actions ; mises à jour de sécurité immédiates.
- Chaque PR Dependabot passe la CI complète (dont `cargo deny`) et une revue.

## 11. Versions et publications
- **SemVer** : `v0.x` pendant la construction ; `v1.0.0` à la fin du lot L9.
- CHANGELOG généré depuis les Conventional Commits (*release-please*, à mettre en place avant la première version).
- Tags **signés** ; chaque version embarque binaires, SBOM CycloneDX, signatures Sigstore et attestation de provenance SLSA.

## 12. Sécurité du dépôt et de l'organisation
- Organisation : 2FA obligatoire ; permission de base « lecture » ; droits par équipe ; revue trimestrielle des membres et des accès.
- Dépôt : secret scanning + **push protection**, alertes et mises à jour de sécurité Dependabot, CodeQL, **Private Vulnerability Reporting**.
- Vulnérabilités : jamais en issue publique ; processus dans `SECURITY.md` ; avis publiés via GitHub Security Advisories.
- Applications GitHub installées : le minimum (CLA Assistant, Dependabot), permissions revues avant installation.
- **Secrets** : aucun secret de dépôt tant qu'il n'y a rien à déployer. Ensuite, déploiement par **OIDC** (aucune clé cloud longue durée) depuis un *environment* GitHub protégé (approbation manuelle, branche `main` uniquement). Les branches de travail, donc l'agent, n'y ont jamais accès.
- Jetons et clés : rotation au plus tard tous les 90 jours ; révocation immédiate en cas de doute.

## 13. Contributions externes
- `CONTRIBUTING.md` explique le processus ; **CLA signé** via CLA Assistant avant la première fusion.
- Questions dans **GitHub Discussions** ; bugs et demandes dans les issues.
- Code de conduite : `CODE_OF_CONDUCT.md`.

## 14. Règles propres aux agents IA (Claude Code)
- Travaille **uniquement** sur une branche dédiée, dans un worktree dédié, avec le compte machine.
- Ouvre la PR, applique l'étiquette `ia:généré`, demande la revue ; **ne fusionne jamais**, n'approuve jamais.
- Ne modifie jamais `.github/CODEOWNERS`, `scripts/github/`, les workflows de sécurité, `docs/specification.md` ni `docs/fondations.md` sans instruction explicite du mainteneur dans l'issue.
- Une PR par tâche ; au-delà d'environ 400 lignes, découper.
- En cas de doute (règle comptable, licence, sécurité) : commenter l'issue avec `statut:à-préciser` et s'arrêter.

## 15. Passage en équipe (à l'arrivée d'un deuxième mainteneur)
- [ ] Ajouter la personne à `@Sigma-Books/core` (2FA vérifiée, clé de signature déclarée).
- [ ] Ruleset de `main` : **2 approbations** (`required_approving_review_count` dans `scripts/github/ruleset-main.json`, puis relancer le script).
- [ ] Lever la règle 🧍 « tout passe par le compte machine » : chaque mainteneur peut ouvrir ses propres PR, relues par un autre.
- [ ] Vérifier que CODEOWNERS désigne l'équipe sur toutes les zones sensibles.
- [ ] Revoir les accès : comptes, jetons, applications installées.
- [ ] Activer la *merge queue* si plusieurs PR se croisent régulièrement.
- [ ] Mettre à jour ce document (retirer les mentions 🧍).
