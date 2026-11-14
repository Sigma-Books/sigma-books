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

echo "== 7. Rulesets (main, tags : modification/suppression, tags : création)"
# Les acteurs de contournement de type « Team » sont écrits avec actor_id 0 dans les
# fichiers JSON : l'identifiant réel de l'équipe core est résolu ici, à l'exécution.
core_id="$(gh api "orgs/$ORG/teams/core" --jq '.id' 2>/dev/null || true)"
if [[ ! "$core_id" =~ ^[0-9]+$ ]]; then
  [[ $APPLIQUER -eq 1 ]] && { echo "✖ identifiant de l'équipe core illisible (droits administrateur requis)" >&2; exit 1; }
  core_id="<id-equipe-core>"
fi
existants="$(gh api "repos/$R/rulesets" --jq '.[] | "\(.id)\t\(.name)"' 2>/dev/null || true)"
for f in ruleset-main.json ruleset-tags.json ruleset-tags-creation.json; do
  nom="$(python3 -c "import json,sys;print(json.load(open(sys.argv[1]))['name'])" "$DIR/$f")"
  id="$(awk -F'\t' -v n="$nom" '$2==n{print $1}' <<<"$existants")"
  if [[ -n "$id" ]]; then
    # Le script ne modifie jamais un ruleset existant : il signale seulement les écarts de règles.
    attendu="$(python3 -c "import json,sys;print(' '.join(sorted(r['type'] for r in json.load(open(sys.argv[1]))['rules'])))" "$DIR/$f")"
    reel="$(gh api "repos/$R/rulesets/$id" --jq '[.rules[].type] | sort | join(" ")' 2>/dev/null || echo "?")"
    if [[ "$attendu" == "$reel" ]]; then
      echo "   ruleset « $nom » présent, règles conformes au fichier ($attendu)"
    else
      echo "   ⚠ ruleset « $nom » présent mais différent du fichier : attendu [$attendu], réel [$reel] — à corriger dans l'interface"
    fi
  else
    entree="$(mktemp)"
    python3 -c "
import json, sys
d = json.load(open(sys.argv[1]))
for a in d.get('bypass_actors', []):
    if a['actor_type'] == 'Team' and a['actor_id'] == 0:
        a['actor_id'] = int(sys.argv[2]) if sys.argv[2].isdigit() else sys.argv[2]
json.dump(d, open(sys.argv[3], 'w'), ensure_ascii=False)
" "$DIR/$f" "$core_id" "$entree"
    echo "   création du ruleset « $nom » (équipe core en contournement : $core_id)"
    [[ $APPLIQUER -eq 0 ]] && python3 -m json.tool "$entree" | sed 's/^/     /'
    run gh api -X POST "repos/$R/rulesets" --input "$entree"
    rm -f "$entree"
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
 g. Checks de CI requis : versionnés dans ruleset-main.json, appliqués à la création du ruleset.
    Sur un ruleset existant, l'étape 7 signale tout écart ; corriger alors dans Settings > Rules > Rulesets.
 h. Dès qu'un second mainteneur humain rejoint l'équipe core : passer les approbations requises à 2.
FIN
