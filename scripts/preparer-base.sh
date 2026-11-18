#!/usr/bin/env bash
# Prépare une base PostgreSQL de Sigma Books : approvisionnement (rôles, base, droits),
# puis migrations sous le rôle de migration (ADR 0011). Utilisé en développement et en CI
# (.github/workflows/rust.yml). Idempotent. Mode d'emploi : crates/api/README.md.
#
# Variables (aucune valeur par défaut pour les mots de passe, jamais de secret dans le dépôt) :
#   SIGMA_PG_SUPER_URL       URL d'un super-utilisateur, sans nom de base
#                            (ex. postgres://postgres:<mot de passe>@127.0.0.1:5432)
#   SIGMA_PG_BASE            nom de la base (défaut : sigma_books)
#   SIGMA_PG_HOTE_PORT       hôte:port pour l'URL du rôle de migration (défaut : 127.0.0.1:5432)
#   SIGMA_MDP_MIGRATION, SIGMA_MDP_APPLICATION, SIGMA_MDP_LECTURE_AUDIT
#                            mots de passe des trois rôles (caractères sûrs dans une URL)
# Outils : psql, sqlx (sqlx-cli, version épinglée dans rust.yml).
set -euo pipefail

: "${SIGMA_PG_SUPER_URL:?URL du super-utilisateur absente}"
: "${SIGMA_MDP_MIGRATION:?mot de passe du rôle sigma_migration absent}"
: "${SIGMA_MDP_APPLICATION:?mot de passe du rôle sigma_application absent}"
: "${SIGMA_MDP_LECTURE_AUDIT:?mot de passe du rôle sigma_lecture_audit absent}"
base="${SIGMA_PG_BASE:-sigma_books}"
hote_port="${SIGMA_PG_HOTE_PORT:-127.0.0.1:5432}"
racine="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

psql "${SIGMA_PG_SUPER_URL%/}/postgres" -X -q -v ON_ERROR_STOP=1 \
  -v base="$base" \
  -v mdp_migration="$SIGMA_MDP_MIGRATION" \
  -v mdp_application="$SIGMA_MDP_APPLICATION" \
  -v mdp_lecture_audit="$SIGMA_MDP_LECTURE_AUDIT" \
  -f "$racine/crates/api/sql/approvisionnement.sql"

# L'URL est passée par l'environnement, pas en argument : elle n'apparaît pas dans `ps`.
DATABASE_URL="postgres://sigma_migration:${SIGMA_MDP_MIGRATION}@${hote_port}/${base}" \
  sqlx migrate run --source "$racine/crates/api/migrations"
