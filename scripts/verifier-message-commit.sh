#!/usr/bin/env bash
# Vérifie qu'une première ligne de message suit Conventional Commits
# (docs/collaboration.md §3, §4). Utilisé par le crochet commit-msg (lefthook)
# et par le workflow pr-titre.yml (titre de PR = message du commit squashé).
# Usage : verifier-message-commit.sh --fichier <fichier>   (crochet commit-msg)
#         verifier-message-commit.sh --texte "<ligne>"       (CI)
set -euo pipefail

local_=0
case "${1:-}" in
  --fichier) ligne="$(head -n 1 "$2")"; local_=1 ;;
  --texte) ligne="$2" ;;
  *) echo "usage : $0 --fichier <fichier> | --texte <ligne>" >&2; exit 2 ;;
esac

# Commits intermédiaires d'un rebase (--autosquash) : acceptés en local
# uniquement, jamais comme titre de PR.
if (( local_ )) && [[ "$ligne" =~ ^(fixup|squash|amend)!\  ]]; then exit 0; fi

types='feat|fix|refactor|perf|test|docs|ci|build|chore|security|revert'
motif="^(${types})(\([a-z0-9][a-z0-9._/-]*\))?!?: [^ ].*[^ ]$"

if [[ ! "$ligne" =~ $motif ]]; then
  cat >&2 <<MSG
✖ « $ligne »
  ne respecte pas Conventional Commits : type(portée): description
  types admis : ${types//|/, }
  exemple : feat(compta-core): ajoute la contre-passation
MSG
  exit 1
fi
if (( ${#ligne} > 100 )); then
  echo "✖ première ligne trop longue (${#ligne} > 100 caractères)" >&2
  exit 1
fi
