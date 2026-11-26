#!/usr/bin/env bash
# Purge, dans le répertoire de compilation PARTAGÉ, des artefacts des crates du dépôt. Dry-run par défaut.
# Mode d'emploi et règles : docs/stockage-developpement.md (section « Limite constatée »).
#
# Pourquoi : `.worktrees/.target` est commun à tous les worktrees. Un agent peut s'y voir servir les
# crates du dépôt compilées par un AUTRE worktree : ses `cargo test` et `cargo clippy` portent alors,
# en partie, sur le code d'un autre. La CI compile à neuf et reste le filet ; cette purge rend les
# vérifications locales fidèles avant de rendre la main.
#
# Usage :
#   scripts/cargo-propre.sh                       montre ce qui serait nettoyé (rien n'est modifié)
#   scripts/cargo-propre.sh --appliquer           exécute `cargo clean -p <crate>` pour chaque crate du dépôt
#   scripts/cargo-propre.sh --tout                dry-run, en comptant aussi `debug/incremental`
#   scripts/cargo-propre.sh --tout --appliquer    supprime aussi `debug/incremental`
#
# Ce que fait le script (et rien d'autre) :
#   1. lit `members` dans le `Cargo.toml` du workspace (aucun nom de crate codé en dur) et relève le
#      nom du paquet de chaque membre ;
#   2. `cargo clean -p <crate>` pour chacun (profils dev et release) : les artefacts des crates du
#      dépôt disparaissent, les dépendances externes (l'essentiel du temps de compilation) sont gardées ;
#   3. avec --tout : supprime en plus `debug/incremental` du répertoire de compilation.
#
# Ce que le script ne fait JAMAIS :
#   - tourner en mode --appliquer quand `cargo` ou `rustc` est actif (autre agent en train de compiler) ;
#     en dry-run il le signale seulement ;
#   - toucher aux dépendances externes, à un `target/` local, à /mnt/c ni /mnt/d ;
#   - compiler quoi que ce soit.
set -euo pipefail

appliquer=0
tout=0
for a in "$@"; do
  case "$a" in
    --appliquer) appliquer=1 ;;
    --tout) tout=1 ;;
    -h | --help) sed -n '2,26p' "$0"; exit 0 ;;
    *) echo "option inconnue : $a (usage : $0 [--tout] [--appliquer])" >&2; exit 64 ;;
  esac
done

racine="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
commun="$(git -C "$racine" rev-parse --path-format=absolute --git-common-dir)"
principal="$(dirname "$commun")"
cible="${CARGO_TARGET_DIR:-$principal/.worktrees/.target}"

compilation_en_cours() { pgrep -x cargo >/dev/null 2>&1 || pgrep -x rustc >/dev/null 2>&1; }

echo "Mode : $([ "$appliquer" -eq 1 ] && echo APPLIQUER || echo dry-run)$([ "$tout" -eq 1 ] && echo ' (+ debug/incremental)')"
echo "Répertoire de compilation : $cible"

if compilation_en_cours; then
  if [ "$appliquer" -eq 1 ]; then
    echo "cargo ou rustc tourne : refus (purger sous un autre agent qui compile corromprait sa compilation)." >&2
    exit 75
  fi
  echo "ATTENTION : cargo ou rustc tourne ; --appliquer serait refusé."
fi
command -v cargo >/dev/null 2>&1 || { echo "cargo introuvable (PATH : ~/.cargo/bin)" >&2; exit 69; }

# Membres du workspace : motifs de `members = [...]` du Cargo.toml racine, développés en dossiers.
motifs="$(awk '
  /^members[[:space:]]*=/ { dedans = 1 }
  dedans { print }
  dedans && /\]/ { exit }' "$racine/Cargo.toml" | grep -o '"[^"]*"' | tr -d '"' || true)"
[ -n "$motifs" ] || { echo "aucun membre lu dans $racine/Cargo.toml" >&2; exit 65; }

crates=()
while IFS= read -r motif; do
  # Le motif n'est jamais interprété par un autre outil que le globbing de bash, sans ligne de commande.
  for d in "$racine"/$motif; do
    [ -f "$d/Cargo.toml" ] || continue
    nom="$(awk '/^\[package\]/{p=1; next} /^\[/{p=0} p && /^name[[:space:]]*=/ {gsub(/^[^"]*"|".*$/, ""); print; exit}' "$d/Cargo.toml")"
    [ -n "$nom" ] || { echo "  $d : nom de paquet introuvable, ignoré" >&2; continue; }
    crates+=("$nom")
  done
done <<< "$motifs"
[ "${#crates[@]}" -gt 0 ] || { echo "aucune crate trouvée" >&2; exit 65; }

echo "Crates du dépôt (${#crates[@]}) : ${crates[*]}"
echo

args=()
for c in "${crates[@]}"; do args+=(-p "$c"); done

cd "$racine"
echo "== cargo clean (profil dev)"
if [ "$appliquer" -eq 1 ]; then cargo clean "${args[@]}"; else cargo clean --dry-run "${args[@]}" 2>&1 | tail -3; fi
if [ -d "$cible/release" ]; then
  echo "== cargo clean (profil release)"
  if [ "$appliquer" -eq 1 ]; then cargo clean --release "${args[@]}"; else cargo clean --release --dry-run "${args[@]}" 2>&1 | tail -3; fi
fi

if [ "$tout" -eq 1 ]; then
  echo
  echo "== debug/incremental"
  for p in "$cible/debug/incremental" "$cible/release/incremental"; do
    [ -d "$p" ] || continue
    if [ "$appliquer" -eq 1 ]; then echo "  supprime : ${p#"$cible"/}"; rm -rf -- "${p:?}"
    else echo "  [dry-run] supprimerait : ${p#"$cible"/}"; fi
  done
fi

echo
echo "Terminé ($([ "$appliquer" -eq 1 ] && echo appliqué || echo dry-run))."
[ "$appliquer" -eq 1 ] || echo "Rien n'a été modifié ; relancer avec --appliquer pour purger."
