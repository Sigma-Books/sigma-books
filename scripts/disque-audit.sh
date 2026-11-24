#!/usr/bin/env bash
# Audit de l'espace disque du développement de Sigma Books (lecture seule).
# Mode d'emploi et budgets : docs/stockage-developpement.md.
#
# Usage :
#   scripts/disque-audit.sh                  rapport complet (mesure les répertoires : quelques secondes)
#   scripts/disque-audit.sh --alerte         une ligne, rapide (df seul) : pour les crochets et les agents
#   scripts/disque-audit.sh --exiger-libre N échoue (code 2) s'il reste moins de N Go sous le budget
#
# Code de sortie : 0 normal, 1 seuil d'alerte dépassé, 2 seuil critique ou espace insuffisant.
# Variables : SIGMA_BUDGET_GO (défaut 100, plafond visé pour WSL), SIGMA_ALERTE_PCT (70),
# SIGMA_CRITIQUE_PCT (85).
#
# Ce script ne lit que le disque de WSL (`/`) et le dépôt. Il ne parcourt jamais /mnt/c ni /mnt/d.
set -euo pipefail

budget="${SIGMA_BUDGET_GO:-100}"
alerte_pct="${SIGMA_ALERTE_PCT:-70}"
critique_pct="${SIGMA_CRITIQUE_PCT:-85}"
mode="complet"
exiger=0
while [ $# -gt 0 ]; do
  case "$1" in
    --alerte) mode="alerte" ;;
    --exiger-libre) exiger="${2:?--exiger-libre attend un nombre de Go}"; shift ;;
    -h | --help) sed -n '2,13p' "$0"; exit 0 ;;
    *) echo "option inconnue : $1" >&2; exit 64 ;;
  esac
  shift
done

utilise=$(df --output=used -BG / | tail -1 | tr -dc '0-9')
pct=$((utilise * 100 / budget))
libre=$((budget - utilise))
code=0
etat="OK"
if [ "$pct" -ge "$critique_pct" ]; then etat="CRITIQUE"; code=2
elif [ "$pct" -ge "$alerte_pct" ]; then etat="ALERTE"; code=1; fi
if [ "$exiger" -gt 0 ] && [ "$libre" -lt "$exiger" ]; then etat="INSUFFISANT"; code=2; fi

echo "disque WSL : ${utilise} Go utilisés sur un budget de ${budget} Go (${pct} %), ${libre} Go libres : ${etat}"
if [ "$mode" = "alerte" ]; then
  [ "$code" -ne 0 ] && echo "→ lancer scripts/disque-nettoyer.sh (dry-run), puis --appliquer ; voir docs/stockage-developpement.md" >&2
  exit "$code"
fi

racine="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# Racine du clone principal : depuis un worktree, `git rev-parse --git-common-dir` la désigne.
commun="$(git -C "$racine" rev-parse --path-format=absolute --git-common-dir)"
principal="$(dirname "$commun")"
taille() { du -sk "$1" 2>/dev/null | awk '{printf "%.1f", $1/1048576}'; }

echo
echo "== Compilation partagée"
partage="$principal/.worktrees/.target"
[ -d "$partage" ] && echo "  .worktrees/.target : $(taille "$partage") Go" || echo "  (absent : voir .worktrees/.cargo/config.toml)"

echo
echo "== Worktrees (git worktree list)"
git -C "$principal" worktree list --porcelain | awk '/^worktree /{print $2}' | while read -r w; do
  nom="${w#"$principal"/}"
  [ "$w" = "$principal" ] && nom="(clone principal)"
  branche="$(git -C "$w" branch --show-current 2>/dev/null || echo '?')"
  modifs="$(git -C "$w" status --porcelain 2>/dev/null | wc -l)"
  propre="propre"; [ "$modifs" -gt 0 ] && propre="${modifs} fichiers modifiés ou non suivis"
  t="-"; [ -d "$w/target" ] && t="$(taille "$w/target") Go (target local)"
  printf '  %-28s %-45s %s ; %s\n' "$nom" "$branche" "$propre" "$t"
done

echo
echo "== Docker"
if command -v docker >/dev/null 2>&1; then
  docker system df 2>/dev/null | sed 's/^/  /' || echo "  (docker inaccessible)"
  anon="$(docker volume ls -qf dangling=true 2>/dev/null | grep -cE '^[0-9a-f]{64}$' || true)"
  echo "  volumes anonymes sans conteneur : ${anon} (jamais supprimés automatiquement)"
fi
exit "$code"
