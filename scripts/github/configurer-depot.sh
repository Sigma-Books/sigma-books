#!/usr/bin/env bash
# Applique les réglages GitHub décrits dans docs/collaboration.md.
# Prérequis : gh authentifié avec un compte ADMINISTRATEUR de l'organisation Sigma-Books
#             (gh auth status ; gh auth refresh -s admin:org,repo).
# Usage    : scripts/github/configurer-depot.sh            → affiche ce qui serait fait
#            scripts/github/configurer-depot.sh --appliquer → applique
# Idempotent : peut être relancé sans dommage.
set -euo pipefail

ORG="Sigma-Books"
REPO="sigma-books"
R="$ORG/$REPO"
DIR="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$DIR/../.." && pwd)"
APPLIQUER=0; [[ "${1:-}" == "--appliquer" ]] && APPLIQUER=1

run() { echo "+ $*"; if [[ $APPLIQUER -eq 1 ]]; then "$@"; fi; }

echo "== 1. Réglages du dépôt (fusion squash uniquement, suppression des branches, discussions)"
run gh api -X PATCH "repos/$R" \
  -F allow_squash_merge=true -F allow_merge_commit=false -F allow_rebase_merge=false \
  -F delete_branch_on_merge=true -F allow_auto_merge=true -F allow_update_branch=true \
  -f squash_merge_commit_title=PR_TITLE -f squash_merge_commit_message=PR_BODY \
  -F has_discussions=true -F has_wiki=false -F web_commit_signoff_required=false

echo "== 2. Sécurité du dépôt"
run gh api -X PATCH "repos/$R" --input - <<'JSON'
{ "security_and_analysis": {
    "secret_scanning": { "status": "enabled" },
    "secret_scanning_push_protection": { "status": "enabled" },
    "dependabot_security_updates": { "status": "enabled" } } }
JSON
run gh api -X PUT "repos/$R/vulnerability-alerts"
run gh api -X PUT "repos/$R/private-vulnerability-reporting"

echo "== 3. GitHub Actions : jeton en lecture seule, pas d'approbation de PR par les workflows"
run gh api -X PUT "repos/$R/actions/permissions/workflow" \
  -f default_workflow_permissions=read -F can_approve_pull_request_reviews=false

echo "== 4. Équipe core"
if ! gh api "orgs/$ORG/teams/core" >/dev/null 2>&1; then
  run gh api -X POST "orgs/$ORG/teams" -f name=core -f privacy=closed \
    -f description="Mainteneurs : revue et fusion des zones sensibles"
fi
run gh api -X PUT "orgs/$ORG/teams/core/repos/$R" -f permission=maintain

echo "== 5. Étiquettes"
grep -v '^#' "$ROOT/.github/labels.yml" | while IFS='|' read -r nom couleur desc; do
  [[ -z "$nom" ]] && continue
  run gh label create "$nom" --repo "$R" --color "$couleur" --description "$desc" --force
done

echo "== 6. Jalons (un par lot)"
for j in "L0 — Fondations" "L1 — Noyau comptable" "L2 — Socle applicatif" "L3 — Tiers et trésorerie" \
         "L4 — IA et portail" "L5 — TVA, paie, notes de frais" "L6 — Fin d'exercice et clôture" \
         "L7 — États financiers" "L8 — Interopérabilité" "L9 — Durcissement"; do
  if ! gh api "repos/$R/milestones?state=all&per_page=100" --jq '.[].title' | grep -Fxq "$j"; then
    run gh api -X POST "repos/$R/milestones" -f title="$j"
  fi
done

echo "== 7. Rulesets (main et tags)"
existants="$(gh api "repos/$R/rulesets" --jq '.[].name' 2>/dev/null || true)"
for f in ruleset-main.json ruleset-tags.json; do
  nom="$(python3 -c "import json,sys;print(json.load(open(sys.argv[1]))['name'])" "$DIR/$f")"
  if grep -Fxq "$nom" <<<"$existants"; then
    echo "   ruleset « $nom » déjà présent — à mettre à jour depuis l'interface si besoin"
  else
    run gh api -X POST "repos/$R/rulesets" --input "$DIR/$f"
  fi
done

cat <<'FIN'

== Étapes manuelles (interface GitHub, administrateur de l'organisation)
 a. Organisation > Settings > Authentication security : exiger la 2FA pour tous les membres.
 b. Organisation > Settings > Member privileges : permission de base « Read » ; création de dépôts réservée aux admins.
 c. Organisation > Settings > Actions > General : autoriser uniquement les actions de GitHub et les actions
    épinglées par hash (« Require actions to be pinned to a full-length commit SHA ») ;
    « Require approval for all outside collaborators » pour les PR de forks.
 d. Ajouter le compte machine de Claude Code (fhdiop-ccode) avec le rôle « Write » sur le dépôt
    (pas membre de l'équipe core), clé SSH de signature déclarée.
 e. Installer l'application CLA Assistant sur le dépôt.
 f. Créer le tableau GitHub Projects « Sigma Books » (colonnes et champs : docs/collaboration.md §5).
 g. À la fin du lot L0 : ajouter les checks de CI requis au ruleset « Protection de main »
    (Settings > Rules > Rulesets > Require status checks, option « require branches to be up to date »).
 h. Dès qu'un second mainteneur humain rejoint l'équipe core : passer les approbations requises à 2.
FIN
