#!/usr/bin/env bash
# Prépare une base PostgreSQL de Sigma Books : approvisionnement (rôles, base, droits),
# puis migrations sous le rôle de migration (ADR 0011). Utilisé en développement et en CI
# (.github/workflows/rust.yml). Idempotent. Mode d'emploi : crates/api/README.md.
#
# Aucun mot de passe en argument de commande (visible par `ps`) : tous passent par
# l'environnement, lisible du seul propriétaire du processus.
#
# Variables :
#   PGHOST, PGPORT, PGUSER   super-utilisateur (variables libpq) ; PGHOST est un nom d'hôte ou
#                            une adresse, repris dans l'URL du rôle de migration
#   PGPASSWORD               son mot de passe (ou un fichier ~/.pgpass ; absent en CI : trust)
#   SIGMA_PG_BASE            nom de la base (défaut : sigma_books)
#   SIGMA_MDP_MIGRATION, SIGMA_MDP_APPLICATION, SIGMA_MDP_LECTURE_AUDIT
#                            mots de passe des trois rôles (caractères sûrs dans une URL),
#                            lus par psql (\getenv) et par sqlx-cli (DATABASE_URL)
# Outils : psql 15 ou plus, sqlx (sqlx-cli, version épinglée dans rust.yml).
set -euo pipefail

: "${PGHOST:?PGHOST absente (hôte du super-utilisateur)}"
: "${PGPORT:?PGPORT absente}"
: "${PGUSER:?PGUSER absente (super-utilisateur)}"
: "${SIGMA_MDP_MIGRATION:?mot de passe du rôle sigma_migration absent}"
: "${SIGMA_MDP_APPLICATION:?mot de passe du rôle sigma_application absent}"
: "${SIGMA_MDP_LECTURE_AUDIT:?mot de passe du rôle sigma_lecture_audit absent}"
export SIGMA_MDP_MIGRATION SIGMA_MDP_APPLICATION SIGMA_MDP_LECTURE_AUDIT
base="${SIGMA_PG_BASE:-sigma_books}"
racine="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

psql -X -q -d postgres -v ON_ERROR_STOP=1 -v base="$base" \
  -f "$racine/crates/api/sql/approvisionnement.sql"

# Migrations sous le rôle de migration ; son URL (avec mot de passe) passe par l'environnement.
DATABASE_URL="postgres://sigma_migration:${SIGMA_MDP_MIGRATION}@${PGHOST}:${PGPORT}/${base}" \
  sqlx migrate run --source "$racine/crates/api/migrations"
