# crate `api` — base de données et serveur en développement

Socle PostgreSQL de l'ADR 0011 : rôles, migrations, RLS forcée, contexte de tenant. Ce guide sert
à démarrer une base locale, l'approvisionner, la migrer, préparer les métadonnées `sqlx`, lancer
les tests d'isolation et le serveur.

## Rôles

| Rôle | Usage |
|---|---|
| `sigma_migration` | Propriétaire de la base, du schéma et des tables. `sqlx migrate` seulement, jamais à l'exécution. |
| `sigma_application` | Binaire `api`. Non propriétaire, sans `CREATE`, sous RLS. Droits posés table par table par les migrations. |
| `sigma_lecture_audit` | Lecture du journal d'audit (ADR 0014) : `SELECT` seul sur les quatre tables d'audit, pool distinct du binaire. |

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
- `cargo test --all --all-features` : active `tests-postgres` (catalogue, RLS, requête, isolation
  par l'API, journal d'audit). Ils
  exigent `SIGMA_TEST_ADMIN_URL` (super-utilisateur) et `SIGMA_TEST_APPLICATION_URL`, et une base
  migrée. Sans elles, ils **échouent** avec un message qui renvoie ici : ils ne s'ignorent jamais.

Le rôle de lecture d'audit est pris, dans les tests, par des connexions du super-utilisateur qui
font aussitôt `SET ROLE sigma_lecture_audit` (seuls ses droits s'appliquent alors, RLS comprise) ;
de même `SET ROLE sigma_migration` pour les tests où le propriétaire altère une entrée d'audit.

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

## Serveur

```sh
set -a; . ./.env; set +a
cargo run -p api
```

Variables : `SIGMA_ADRESSE_ECOUTE`, `SIGMA_URL_BASE_APPLICATION` (rôle **applicatif**),
`SIGMA_URL_BASE_LECTURE_AUDIT` (rôle de **lecture d'audit**, pool distinct), `SIGMA_ORIGINE`, et facultativement `SIGMA_REPERTOIRE_WEB`, `SIGMA_EN_TETE_ADRESSE_CLIENT`,
`SIGMA_AUTORITES_HORODATAGE`, `SIGMA_CONSERVATION_SESSIONS_JOURS` (`src/serveur/config.rs`). Aucune
connexion par facteur n'existe encore (PR 8b et 8c) : sans session ouverte, toute route de l'API
répond 401.

Pipeline, en-têtes, anti-CSRF, limites : `src/serveur/mod.rs` et `src/serveur/pipeline.rs`.

### Nouvelle route

1. Une entrée dans `ROUTES` (`src/serveur/routes.rs`) : méthode, chemin avec `{cabinet}` (et
   `{dossier}`), action Cedar, portée, cas d'usage. Aucune route hors de cette table. Une route
   hors cabinet (portée `Utilisateur` ou `Session`) ne porte que sur l'utilisateur authentifié.
2. Ses cas dans `attentes`, ou `cas_hors_cabinet` pour une route hors cabinet
   (`tests/isolation.rs`) : sans eux, le test « chaque route » échoue.
   Une route déclarée hors de la table fait échouer `tests/structure.rs`.
3. Une mutation (`POST`) rend ses traces d'audit (`serveur::audit::Trace`) : le pipeline les
   inscrit au journal d'audit chaîné dans la transaction de l'action, et refuse une mutation
   sans trace (ADR 0014).

## Sessions

ADR 0012 ; code : `src/serveur/session.rs` (jeton, cookie, anti-CSRF, authentification),
`src/serveur/moi.rs` (routes), `src/base/session.rs` (table `session`, migration
`0004_sessions.sql`).

- **Jeton** de 256 bits (`getrandom`), en base64url dans le cookie
  `__Host-sigma_session` (`Secure`, `HttpOnly`, `SameSite=Strict`, `Path=/`, sans `Domain`,
  `Max-Age` égal au reste des 12 heures). Seule son empreinte SHA-256 est stockée ; il n'apparaît
  dans aucun journal.
- **Chaque requête relit sa session** dans une courte transaction qui ne pose que
  `sigma.jeton_session` (empreinte) : révocation immédiate. Refus (401) : session révoquée,
  inactive depuis 30 minutes, ouverte depuis plus de 12 heures (15 minutes pour une session
  restreinte ou d'enrôlement), ou de nature autre que `complete` hors des routes de la session
  elle-même. `derniere_activite` n'est réécrite qu'au-delà de 60 secondes. L'horloge est
  injectable (`Application::avec_horloge`, tests).
- **Anti-CSRF lié à la session** : `base64url(SHA-256("sigma-books csrf v1\0" ‖ jeton))`, rendu
  par `GET /api/session` et par la rotation, exigé dans `x-sigma-csrf` sur toute mutation
  authentifiée (403 sinon), comparé en temps constant (`ctutils`), en plus des contrôles
  `Sec-Fetch-Site` et `Origin`.
- **Ouverture** : fonction interne `serveur::session::ouvrir_session`, que les connexions des
  PR 8b et 8c appelleront ; elle purge les sessions closes **du même utilisateur** depuis plus de
  `SIGMA_CONSERVATION_SESSIONS_JOURS` jours (30 par défaut, à confirmer : adresse IP conservée,
  loi 2008-12).

| Route | Portée | Cedar |
|---|---|---|
| `GET /api/session` : état (nature, échéances, fraîcheur) et jeton anti-CSRF | session courante, toute nature | non |
| `POST /api/session/deconnexion` | session courante, toute nature | non |
| `POST /api/session/rotation` : nouveau jeton, même session | utilisateur | non |
| `GET /api/moi/sessions`, `DELETE /api/moi/sessions/{session}` | utilisateur | non |
| `GET /api/moi/cabinets` : ses cabinets, sans cabinet en contexte | utilisateur | `AccederCabinet` par appartenance |

**Rotation et cabinet actif.** L'ADR 0012 demande une rotation au changement de cabinet actif ;
l'ADR 0011 interdit tout cabinet courant mémorisé côté serveur (F1.14). L'interface appelle donc
`POST /api/session/rotation` quand l'utilisateur change de cabinet ; le serveur tourne le jeton
sans rien retenir du cabinet, qui reste lu dans le chemin de chaque requête.

**Hors Cedar.** Une session appartient à un utilisateur, pas à un cabinet : le principal Cedar
(une appartenance) n'a pas de sens pour elle, comme pour la gestion de ses propres facteurs (doc
du crate `authz`). La RLS (`utilisateur_id = sigma.utilisateur`) et le filtre de chaque requête
la bornent à son titulaire ; la session d'un autre est absente (404). La gestion des sessions
d'un membre par l'administrateur (PR 9) sera une action Cedar du cabinet.

**Audit.** Ces actions ne touchent aucun cabinet ; leur traçabilité (événements hors cabinet)
attend une décision du mainteneur. Les points d'insertion sont marqués dans `src/serveur/moi.rs`
et `src/serveur/session.rs`.

## Journal d'audit

Format : `docs/integrite/audit-v1.md` (module pur `integrite::audit`). Tables (migration
`0003_audit.sql`) : entrées `audit_cabinet` et `audit_dossier` en **ajout seul** (`INSERT` seul
pour le rôle applicatif, `SELECT` seul pour le rôle de lecture), têtes `audit_tete_cabinet` et
`audit_tete_dossier` verrouillées `FOR UPDATE`. Routes : consultation paginée (`?apres=&limite=`,
100 au plus), vérification et export (ré-authentifié) de l'audit du cabinet (administrateur) et
d'un dossier (administrateur, valideur, auditeur affectés). La requête revérifie la portée
(administrateur lu en base pour le cabinet, dossier visible pour un dossier) : Cedar n'est pas la
seule barrière. Les lectures d'audit s'exécutent en `REPEATABLE READ READ ONLY`, bornées au rang
de la tête du même instantané : une action concurrente ne produit pas de fausse alerte.

**Limites acceptées en L2** :
- l'export d'audit est construit **en mémoire** (pas de réponse en flux) et **verrouille la tête**
  de la portée pendant toute sa durée : les actions de cette portée attendent la fin de l'export.
  Acceptable pour les volumes de L2 ; un export en flux, ou à partir d'un instantané sans verrou,
  sera à reprendre si les chaînes grossissent ;
- l'alerte à l'administrateur lors d'un export (spec §21, ADR 0014) n'est pas encore émise :
  elle arrive avec la file de tâches et les courriels (PR 9).
