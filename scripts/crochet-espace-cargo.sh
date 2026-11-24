#!/usr/bin/env bash
# Crochet Claude Code (PreToolUse, Bash) : avant une commande `cargo` lourde, vérifie qu'il reste
# assez de place sous le budget WSL (docs/stockage-developpement.md). Bloque (code 2) sinon.
# Lit sur l'entrée standard la description JSON de l'appel ; ne fait rien pour les autres commandes.
set -uo pipefail
entree="$(cat)"
if printf '%s' "$entree" | grep -qE 'cargo[[:space:]]+(\+[a-z0-9.-]+[[:space:]]+)?(build|test|nextest|clippy|check|doc|bench|run)'; then
  racine="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
  if ! sortie="$("$racine/scripts/disque-audit.sh" --alerte --exiger-libre "${SIGMA_LIBRE_MIN_GO:-12}")"; then
    echo "Compilation bloquée : espace insuffisant. $sortie" >&2
    echo "Lancer scripts/disque-nettoyer.sh (dry-run), voir docs/stockage-developpement.md." >&2
    exit 2
  fi
fi
exit 0
