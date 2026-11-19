# crate `api` — base de données en développement

Socle PostgreSQL de l'ADR 0011 : rôles, migrations, RLS forcée, contexte de tenant. Ce guide sert
à démarrer une base locale, l'approvisionner, la migrer, préparer les métadonnées `sqlx` et lancer
les tests d'isolation.

## Rôles

| Rôle | Usage |
|---|---|
| `sigma_migration` | Propriétaire de la base, du schéma et des tables. `sqlx migrate` seulement, jamais à l'exécution. |
| `sigma_application` | Binaire `api`. Non propriétaire, sans `CREATE`, sous RLS. Droits posés table par table par les migrations. |
| `sigma_lecture_audit` | Lecture du journal d'audit (ADR 0014, PR 7). Aucun droit de table pour l'instant. |

Aucun n'a `SUPERUSER`, `BYPASSRLS`, `CREATEROLE` ni `CREATEDB`. Ils sont créés par
l'**approvisionnement** (`sql/approvisionnement.sql`, exécuté par un super-utilisateur), pas par
une migration : un rôle se crée au niveau du cluster et le rôle de migration n'a pas `CREATEROLE`.

## Fichiers

| Fichier | Contenu |
|---|---|
| `sql/approvisionnement.sql` | Rôles, base, droits de connexion. Mots de passe lus dans l'environnement (`\getenv`), aucun dans le dépôt. |
| `migrations/` | Migrations `sqlx` : tables, RLS, politiques, `GRANT`. |
| `catalogue.toml` | Listes justifiées lues par le test de catalogue : tables d'instance, tables de dossier sans dossier en contexte, tables en ajout seul, fonctions `SECURITY DEFINER`. |
| `../../scripts/preparer-base.sh` | Approvisionnement puis migrations (développement et CI). |
| `../../.sqlx/` | Métadonnées des requêtes `query!`, versionnées : la compilation se passe de base. |

## Démarrer

Prérequis : Docker, `psql` 15 ou plus (`\getenv`), et `sqlx-cli` à la version de la CI :

```sh
cargo install sqlx-cli --locked --version 0.9.1 --no-default-features --features postgres,rustls
```

1. Copier `.env.example` (racine du dépôt) en `.env`, ignoré par Git, et remplacer chaque
   `changez-moi` par une valeur locale. Charger le fichier dans le shell :
   ```sh
   set -a; . ./.env; set +a
   ```
2. Démarrer PostgreSQL 17 (même version majeure que la CI), joignable de la seule machine :
   ```sh
   docker run -d --name sigma-books-pg -p 127.0.0.1:5432:5432 \
     -e POSTGRES_PASSWORD="$PGPASSWORD" postgres:17
   ```
3. Approvisionner et migrer (idempotent). Aucun mot de passe ne passe en argument de commande :
   psql lit ceux des rôles par `\getenv`, le super-utilisateur s'authentifie par `PGPASSWORD`
   (ou `~/.pgpass`), sqlx-cli reçoit `DATABASE_URL` par l'environnement.
   ```sh
   scripts/preparer-base.sh
   ```
4. Après toute modification d'une requête `query!` ou d'une migration, régénérer et versionner
   `.sqlx/` :
   ```sh
   cargo sqlx prepare --workspace
   ```

Sans base, la compilation lit `.sqlx/` (`SQLX_OFFLINE=true` force ce mode quand `DATABASE_URL`
est définie).

## Tests

- `cargo test --all` : sans base, les tests de PostgreSQL ne sont pas compilés.
- `cargo test --all --all-features` : active `tests-postgres` (catalogue, RLS, requête). Ils
  exigent `SIGMA_TEST_ADMIN_URL` (super-utilisateur) et `SIGMA_TEST_APPLICATION_URL`, et une base
  migrée. Sans elles, ils **échouent** avec un message qui renvoie ici : ils ne s'ignorent jamais.

Les tests écrivent des cabinets aux identifiants aléatoires, créent le rôle de test
`sigma_test_contournement` (`BYPASSRLS`, `NOLOGIN`) et la base jetable
`sigma_test_catalogue_jetable`. Pour garder la base de développement propre, préparer une base de
test à part (`SIGMA_PG_BASE=sigma_books_test scripts/preparer-base.sh`) et y pointer les deux URL
de test.

## Nouvelle table

1. Migration : `cabinet_id` (et `dossier_id` pour une table de dossier), clés étrangères
   composites, index en tête sur `cabinet_id`, `ENABLE` et `FORCE ROW LEVEL SECURITY`, `GRANT`
   minimal au rôle applicatif, et des politiques `USING` et `WITH CHECK` de la forme exacte :
   `cabinet_id = NULLIF(current_setting('sigma.cabinet', true), '')::uuid`, plus, pour une table
   de dossier, `AND dossier_id = NULLIF(current_setting('sigma.dossier', true), '')::uuid`.
2. Exceptions, chacune justifiée dans `catalogue.toml` : table d'instance (sans cabinet, avec ou
   sans RLS), table de dossier visible sans dossier en contexte, table en ajout seul (ni `UPDATE`
   ni `DELETE`), fonction `SECURITY DEFINER`. Toute fonction retire `EXECUTE` à `PUBLIC` ; aucun
   `ALTER DEFAULT PRIVILEGES`.
3. `cargo test --all --all-features` : le test de catalogue refuse toute table oubliée.
