#!/usr/bin/env bash
# Nettoyage sécurisé des artefacts RÉGÉNÉRABLES de Sigma Books. Dry-run par défaut.
# Mode d'emploi et règles : docs/stockage-developpement.md.
#
# Usage :
#   scripts/disque-nettoyer.sh               montre ce qui serait supprimé (rien n'est modifié)
#   scripts/disque-nettoyer.sh --appliquer   supprime
#
# Ce que fait le script (et rien d'autre) :
#   1. artefacts de compilation locaux d'un worktree (`target/`, `web/build`, `web/.svelte-kit`,
#      `crates/integrite/fuzz/target`) ;
#   2. répertoire partagé `.worktrees/.target` : s'il dépasse SIGMA_TARGET_MAX_GO (défaut 12),
#      supprime `debug/incremental`, puis les fichiers de `debug/deps` non modifiés depuis
#      SIGMA_TARGET_JOURS (défaut 14) jours ;
#   3. cache de build Docker au-delà de SIGMA_DOCKER_CACHE_GO (défaut 5) ;
#   4. conteneurs de test ARRÊTÉS dont le nom commence par `sigma-l2-` ou `sigma-test-`.
#
# Ce que le script ne fait JAMAIS :
#   - toucher un worktree qui a un processus actif (répertoire courant dedans) ni, pour le
#     répertoire partagé, quand `cargo` ou `rustc` tourne ;
#   - retirer un worktree, une branche, un volume, une base, une image ou un conteneur en marche
#     (il affiche seulement les commandes `git worktree remove` à examiner) ;
#   - toucher à containerd, au VHDX, à /mnt/c ni /mnt/d.
set -euo pipefail

appliquer=0
case "${1:-}" in
  --appliquer) appliquer=1 ;;
  "") ;;
  -h | --help) sed -n '2,22p' "$0"; exit 0 ;;
  *) echo "option inconnue : $1" >&2; exit 64 ;;
esac

max_go="${SIGMA_TARGET_MAX_GO:-12}"
jours="${SIGMA_TARGET_JOURS:-14}"
cache_go="${SIGMA_DOCKER_CACHE_GO:-5}"

racine="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
commun="$(git -C "$racine" rev-parse --path-format=absolute --git-common-dir)"
principal="$(dirname "$commun")"
partage="$principal/.worktrees/.target"

taille_ko() { du -sk "$1" 2>/dev/null | awk '{print $1}'; }
go() { awk -v k="$1" 'BEGIN{printf "%.1f", k/1048576}'; }
agir() { # agir <description> <ko> <commande...>
  local desc="$1" ko="$2"; shift 2
  if [ "$appliquer" -eq 1 ]; then echo "  supprime : $desc ($(go "$ko") Go)"; "$@"
  else echo "  [dry-run] supprimerait : $desc ($(go "$ko") Go)"; fi
}

# Un processus a-t-il son répertoire courant dans $1 ?
actif() {
  local p
  for p in /proc/[0-9]*; do
    case "$(readlink "$p/cwd" 2>/dev/null || true)" in "$1" | "$1"/*) return 0 ;; esac
  done
  return 1
}
compilation_en_cours() { pgrep -x cargo >/dev/null 2>&1 || pgrep -x rustc >/dev/null 2>&1; }

echo "Mode : $([ "$appliquer" -eq 1 ] && echo APPLIQUER || echo dry-run)"
echo
echo "== 1. Artefacts locaux des worktrees"
git -C "$principal" worktree list --porcelain | awk '/^worktree /{print $2}' | while read -r w; do
  [ "$w" = "$principal" ] && continue
  nom="${w#"$principal"/}"
  if actif "$w"; then echo "  $nom : processus actif, ignoré"; continue; fi
  for a in target web/build web/.svelte-kit crates/integrite/fuzz/target; do
    [ -d "$w/$a" ] || continue
    ko="$(taille_ko "$w/$a")"
    agir "$nom/$a" "$ko" rm -rf -- "${w:?}/$a"
  done
done

echo
echo "== 2. Répertoire de compilation partagé"
if [ -d "$partage" ]; then
  ko="$(taille_ko "$partage")"
  echo "  taille : $(go "$ko") Go (plafond ${max_go} Go)"
  if compilation_en_cours; then echo "  cargo ou rustc tourne : ignoré"
  elif [ "$ko" -gt $((max_go * 1048576)) ]; then
    [ -d "$partage/debug/incremental" ] && agir ".target/debug/incremental" "$(taille_ko "$partage/debug/incremental")" rm -rf -- "$partage/debug/incremental"
    if [ -d "$partage/debug/deps" ]; then
      vieux="$(find "$partage/debug/deps" -type f -mtime +"$jours" -printf '%s\n' 2>/dev/null | awk '{s+=$1} END{print int(s/1024)}')"
      agir ".target/debug/deps : fichiers de plus de ${jours} jours" "${vieux:-0}" \
        find "$partage/debug/deps" -type f -mtime +"$jours" -delete
    fi
  else echo "  sous le plafond : rien à faire"; fi
else echo "  absent"; fi

echo
echo "== 3. Docker"
if command -v docker >/dev/null 2>&1 && docker info >/dev/null 2>&1; then
  if [ "$appliquer" -eq 1 ]; then docker builder prune -f --keep-storage "${cache_go}GB" | tail -1
  else echo "  [dry-run] docker builder prune --keep-storage ${cache_go}GB"; docker system df | sed -n '1p;5p' | sed 's/^/    /'; fi
  docker ps -a --filter status=exited --format '{{.Names}}' | grep -E '^sigma-(l2|test)-' | while read -r c; do
    if [ "$appliquer" -eq 1 ]; then echo "  supprime le conteneur arrêté $c"; docker rm "$c" >/dev/null
    else echo "  [dry-run] supprimerait le conteneur arrêté $c"; fi
  done || true
  anon="$(docker volume ls -qf dangling=true | grep -cE '^[0-9a-f]{64}$' || true)"
  echo "  volumes anonymes sans conteneur : ${anon} → NON supprimés (décision humaine : docker volume ls -f dangling=true)"
else echo "  docker indisponible"; fi

echo
echo "== 4. Worktrees à examiner (jamais retirés par ce script)"
git -C "$principal" worktree list --porcelain | awk '/^worktree /{print $2}' | while read -r w; do
  [ "$w" = "$principal" ] && continue
  [ -z "$(git -C "$w" status --porcelain)" ] || continue
  b="$(git -C "$w" branch --show-current)"
  etat=""
  if command -v gh >/dev/null 2>&1; then
    etat="$(GH_CONFIG_DIR="${GH_CONFIG_DIR:-$HOME/.config/gh-ccode}" gh pr list --state merged --head "$b" --json number -q '.[0].number' 2>/dev/null || true)"
  fi
  [ -n "$etat" ] && echo "  propre, PR #${etat} fusionnée : git worktree remove ${w#"$principal"/}   # la branche $b est conservée"
done
echo
echo "Terminé ($([ "$appliquer" -eq 1 ] && echo appliqué || echo dry-run))."
