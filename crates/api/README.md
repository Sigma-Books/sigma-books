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
| `migrations/` | Migrations `sqlx` : tables, RLS, politiques, `GRANT`. Numéros par plage de voie (« Migrations » ci-dessous). |
| `catalogue/<module>.toml` | Listes justifiées lues par le test de catalogue, un fichier par module (`socle`, `organisation`, `audit`, `sessions`, `facteurs`) : tables d'instance, tables de dossier sans dossier en contexte, lecture de ses cabinets, tables en ajout seul, fonctions `SECURITY DEFINER`. Une table n'est déclarée qu'une fois, tous fichiers confondus ; seule la liste `ajout_seul` se combine avec une autre (journal d'instance en ajout seul, ADR 0017). |
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

## Migrations

Chaque voie de travail numérote ses migrations dans sa **plage réservée** : deux voies ne
prennent jamais le même numéro.

| Plage | Voie |
|---|---|
| `0001` à `0099` | Socle déjà fusionné (organisation, identifiants, audit, sessions) |
| `0100` à `0199` | Authentification (PR 8b, 8c, 9) |
| `0200` à `0299` | Comptabilité (PR 11, 13, 14) |
| `0300` à `0399` | Pièces (PR 12) |
| `0400` à `0499` | Intégrité et horodatage (PR 15) |

`sqlx migrate` accepte des versions non contiguës et les applique dans l'ordre croissant sur une
base neuve (vérifié avec `sqlx-cli` 0.9.1). Limites :
- sur une base **existante**, une migration de numéro inférieur à la plus haute déjà appliquée
  (par exemple `0105` arrivée après `0200`) est appliquée **après**, sans erreur : l'ordre réel
  diffère alors de celui d'une base neuve. Les migrations de deux plages doivent donc être
  **indépendantes** (aucune ne dépend d'une table ou d'une colonne de l'autre plage) ; une
  dépendance entre voies passe par la fusion de la voie dont on dépend, puis une migration de
  numéro supérieur ;
- une migration appliquée puis retirée des sources (branche abandonnée sur une base de
  développement) fait échouer `sqlx migrate run` (« previously applied but is missing ») :
  recréer la base de développement ;
- une migration fusionnée ne se modifie jamais (somme de contrôle vérifiée par `sqlx`).

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
2. Exceptions, chacune justifiée dans le fichier du module, `catalogue/<module>.toml` (créé au
   besoin) : table d'instance (sans cabinet, avec ou
   sans RLS), table de dossier visible sans dossier en contexte, table en ajout seul (ni `UPDATE`
   ni `DELETE`), fonction `SECURITY DEFINER`. Toute fonction retire `EXECUTE` à `PUBLIC` ; aucun
   `ALTER DEFAULT PRIVILEGES`.
3. `cargo test --all --all-features` : le test de catalogue refuse toute table oubliée.

## Serveur

```sh
set -a; . ./.env; set +a
cargo run -p api
```

Connexion par la méthode de secours (mot de passe et TOTP) ou par un code de récupération
(« Méthode de secours » ci-dessous) ; les passkeys arrivent avec la PR 8c. Sans session ouverte,
toute autre route de l'API répond 401.

### Variables d'environnement

Chaque module lit ses variables et les documente dans une table de sa documentation ;
`serveur::config::Configuration` les compose (`src/serveur/config.rs`). Index :

| Variable | Obligatoire | Module (documentation) |
|---|---|---|
| `SIGMA_ADRESSE_ECOUTE` | oui | serveur (`src/serveur/config.rs`) |
| `SIGMA_ORIGINE` | oui | serveur (`src/serveur/config.rs`) |
| `SIGMA_REPERTOIRE_WEB` | non | serveur (`src/serveur/config.rs`) |
| `SIGMA_EN_TETE_ADRESSE_CLIENT` | non | serveur (`src/serveur/config.rs`) |
| `SIGMA_URL_BASE_APPLICATION` | oui | base, rôle **applicatif** (`src/base/config.rs`) |
| `SIGMA_URL_BASE_LECTURE_AUDIT` | oui | base, rôle de **lecture d'audit**, pool distinct (`src/base/config.rs`) |
| `SIGMA_AUTORITES_HORODATAGE` | non | horodatage (`src/horodatage/config.rs`) |
| `SIGMA_CONSERVATION_SESSIONS_JOURS` | non | sessions (`src/serveur/session.rs`) |
| `SIGMA_FICHIER_CLE_INSTANCE` | oui | facteurs : fichier de la clé de l'instance (`src/serveur/facteurs.rs`) |

Un module qui ajoute des variables crée sa structure et sa lecture, documente leur table dans
son module, ajoute une ligne à cet index et branche sa lecture dans `Configuration`, à
l'emplacement réservé à sa voie.

Pipeline, en-têtes, anti-CSRF, limites : `src/serveur/mod.rs` et `src/serveur/pipeline.rs`.

### Nouvelle route

1. Une entrée dans la tranche de son module (`src/serveur/routes/<module>.rs`, constante
   `ROUTES`) : méthode, chemin avec `{cabinet}` (et `{dossier}`), action Cedar, portée, cas
   d'usage du module (variante de son énumération `Cas`). La table `routes::ROUTES` est
   l'assemblage des tranches de `routes::MODULES` : aucune route hors d'une tranche. Une route
   hors cabinet (portée `Utilisateur` ou `Session`) ne porte que sur l'utilisateur authentifié.
   Le module exécute ses cas d'usage ; son bras de `routes::executer` (cabinet) ou de
   `serveur::moi::executer` (hors cabinet) l'appelle.
2. Ses cas dans le fichier de cas de son module (`tests/isolation/<module>.rs`) : attentes pour
   une route d'un cabinet, scénario pour une route hors cabinet ; sans eux, le test « chaque
   route » échoue. Une route déclarée hors de la table fait échouer `tests/structure.rs`.
3. Une mutation (`POST`) rend ses traces d'audit (`serveur::audit::Trace`) : le pipeline les
   inscrit au journal d'audit chaîné dans la transaction de l'action, et refuse une mutation
   sans trace (ADR 0014).

## Sessions

ADR 0012 ; code : `src/serveur/session.rs` (jeton, cookie, anti-CSRF, authentification),
`src/serveur/routes/session.rs` et `src/serveur/routes/moi.rs` (routes), `src/serveur/moi.rs`
(cas d'usage hors cabinet), `src/base/session.rs` (table `session`, migration
`0004_sessions.sql`).

- **Jeton** de 256 bits (`getrandom`), en base64url dans le cookie
  `__Host-sigma_session` (`Secure`, `HttpOnly`, `SameSite=Strict`, `Path=/`, sans `Domain`,
  `Max-Age` égal au reste des 12 heures). Seule son empreinte SHA-256 est stockée ; il n'apparaît
  dans aucun journal.
- **Chaque requête relit sa session** dans une courte transaction qui ne pose que
  `sigma.jeton_session` (empreinte) : révocation immédiate. Refus (401) : session révoquée,
  inactive depuis 30 minutes, ouverte depuis plus de 12 heures (15 minutes pour une session
  restreinte ou d'enrôlement), ou de nature autre que `complete` hors des routes de la session
  elle-même. `derniere_activite` n'est réécrite qu'après une requête admise (nature, anti-CSRF,
  décision), au plus une fois par minute ; la rotation exige que la session porte encore
  l'empreinte du jeton présenté. L'horloge est
  injectable (`Application::avec_horloge`, tests).
- **Anti-CSRF lié à la session** : `base64url(SHA-256("sigma-books csrf v1\0" ‖ jeton))`, rendu
  par `GET /api/session` et par la rotation, exigé dans `x-sigma-csrf` sur toute mutation
  authentifiée (403 sinon), comparé en temps constant (`ctutils`), en plus des contrôles
  `Sec-Fetch-Site` et `Origin`.
- **Ouverture** : par une connexion (PR 8b : `serveur::session::ouvrir_session_dans`, dans la
  transaction de la connexion ; PR 8c : passkeys) ; elle purge les sessions closes **du même
  utilisateur** depuis plus de
  `SIGMA_CONSERVATION_SESSIONS_JOURS` jours (30 par défaut, à confirmer : adresse IP conservée,
  loi 2008-12).

| Route | Portée | Cedar |
|---|---|---|
| `GET /api/session` : état (nature, échéances, fraîcheur) et jeton anti-CSRF | session courante, toute nature | `ConsulterSaSession` (principal Utilisateur) |
| `POST /api/session/deconnexion` | session courante, toute nature | `RevoquerSaSession` (Utilisateur) |
| `POST /api/session/rotation` : nouveau jeton, même session | utilisateur | `TournerSaSession` (Utilisateur) |
| `GET /api/moi/sessions` | utilisateur | `ListerSesSessions` sur soi, puis `ConsulterSaSession` par session (Utilisateur) |
| `DELETE /api/moi/sessions/{session}` | utilisateur | `RevoquerSaSession` (Utilisateur) |
| `GET /api/moi/cabinets` : ses cabinets, sans cabinet en contexte | utilisateur | `AccederCabinet` par appartenance |

**Rotation et cabinet actif.** L'ADR 0012 demande une rotation au changement de cabinet actif ;
l'ADR 0011 interdit tout cabinet courant mémorisé côté serveur (F1.14). L'interface appelle donc
`POST /api/session/rotation` quand l'utilisateur change de cabinet ; le serveur tourne le jeton
sans rien retenir du cabinet, qui reste lu dans le chemin de chaque requête.

**Cedar, principal Utilisateur.** Toute route passe par Cedar (règle absolue 7). Une session
appartient à un utilisateur, pas à un cabinet : ses actions ont pour principal l'**utilisateur**
de la session authentifiée et pour ressource la session lue en base sous RLS, avec son titulaire
(`authz::Autorisation::decider_utilisateur`). Seul le titulaire est autorisé
(`crates/authz/politiques/utilisateur.cedar`, invariants `invariant-titulaire-session` et
`invariant-soi-meme`). La RLS (`utilisateur_id = sigma.utilisateur`) et le filtre de chaque
requête la bornent aussi : la session d'un autre est absente (404), même si Cedar se trompait.
La gestion des sessions d'un membre par l'administrateur (PR 9) sera une action de cabinet.

**Audit.** Ces actions ne touchent aucun cabinet. La déconnexion et la révocation d'une session
sont inscrites au journal d'instance des événements d'authentification (ADR 0017, « Méthode de
secours » ci-dessous) ; la rotation, sans effet sur l'accès, ne l'est pas.

## Méthode de secours

ADR 0012 et 0017 ; PR 8b (issue #170). Code : `src/facteurs/` (cryptographie, sans base ni
HTTP), `src/base/facteurs.rs` et `src/base/evenement.rs` (tables), `src/serveur/facteurs.rs`
(cas d'usage), `src/serveur/routes/facteurs.rs` (routes) ; migration `0100_secours.sql`,
catalogue `catalogue/facteurs.toml`.

- **Mot de passe** : 12 caractères au moins, 128 au plus, aucune règle de composition ; refusé
  s'il figure dans la liste embarquée des mots de passe compromis (`donnees/`, tirée de SecLists
  par `scripts/generer-mots-de-passe-compromis.py`, attribution `docs/provenance/seclists.md`).
  Argon2id, 19 Mio, 2 passes ; re-haché à la connexion si les paramètres changent. Le hachage
  s'exécute hors de la boucle asynchrone, au plus 4 à la fois, et hors de toute transaction.
- **TOTP** : RFC 6238, 6 chiffres, pas de 30 s, tolérance d'un pas. **Anti-rejeu** : le dernier
  pas accepté est en base ; un pas n'est admis que par une mise à jour conditionnelle
  (`totp_dernier_pas < $pas`, mot de passe inchangé) : de deux connexions concurrentes avec le
  même code, une seule passe.
- **Clé de l'instance** (`SIGMA_FICHIER_CLE_INSTANCE`) : fichier de 32 octets en base64, droits
  0600 (ou 0400), refusé sinon ; sans clé, le serveur refuse de démarrer. Sous-clés HKDF-SHA256
  (TOTP, jeton d'enrôlement, débit par courriel). Secret TOTP chiffré en AES-256-GCM, utilisateur
  et identifiant de clé en données associées. Après un changement de clé, le TOTP est à
  réenrôler (`GET /api/moi/facteurs` : `utilisable` à faux). Créer une clé :
  `umask 077 && head -c 32 /dev/urandom | base64 > <fichier>`.
- **Enrôlement** en deux temps, sans état entre eux : la préparation rend le secret (et l'URI
  `otpauth`) et un jeton d'enrôlement qui le scelle, lié à l'utilisateur, à la session et à une
  échéance de 10 minutes ; la confirmation exige ce jeton, le mot de passe et un code. Elle
  remplace le secours, tire dix nouveaux codes de récupération (l'ancienne série est annulée) et
  révoque les autres sessions de l'utilisateur (changement de facteur, ADR 0012).
- **Codes de récupération** : 120 bits, base32 de Crockford, hachés (SHA-256), à usage unique
  (`utilise_le IS NULL`, atomique). Un code n'ouvre qu'une **session restreinte** : sa propre
  session et l'enrôlement du secours, rien d'autre (F1.12 ; passkeys en PR 8c).
- **Connexion sans contexte** : la transaction de connexion ne pose **jamais**
  `sigma.utilisateur`, mais `sigma.courriel_connexion` (politiques `*_connexion` de la migration
  `0100`) : elle ne voit que la ligne `utilisateur` du courriel saisi, son secours et ses codes, et
  ne peut qu'y consommer un pas ou un code et inscrire un événement. Type distinct
  (`TransactionConnexion`) ; elle ne devient la transaction de l'utilisateur que sur
  présentation d'un `FacteurVerifie`, fabriqué par la seule consommation d'un facteur. La
  consommation, la session, l'événement et l'audit sont validés ensemble. Aucune fonction
  `SECURITY DEFINER`.
- **Anti-énumération** : courriel inconnu, compte sans secours, facteur faux ou rejoué : même
  réponse (401) et durée comparable (Argon2id exécuté sur une chaîne factice). Rien n'est
  inscrit pour un courriel inconnu.
- **Débit** : par adresse (20 tentatives de connexion par quart d'heure), par courriel saisi
  (10, que le compte existe ou non ; clé HMAC, le courriel n'est pas gardé en mémoire), par
  utilisateur pour la ré-authentification et l'enrôlement (10). Pas de verrouillage du compte :
  un tiers ne peut pas bloquer un utilisateur.
- **Événements d'authentification** (ADR 0017) : table d'instance `evenement_authentification`,
  en ajout seul, non chaînée, sous RLS par utilisateur ; connexion (réussie ou refusée),
  ré-authentification, enrôlement, nouvelle série de codes, déconnexion, révocation. Ni mot de
  passe, ni code, ni jeton, ni empreinte. La connexion par le secours ou par un code,
  l'enrôlement et une nouvelle série de codes sont aussi inscrits dans l'audit chaîné de
  **chaque cabinet** où l'appartenance est active (acteur : l'utilisateur, avec son rôle dans ce
  cabinet).

| Route | Portée | Cedar |
|---|---|---|
| `POST /api/connexion/secours` : courriel, mot de passe, code TOTP ; session complète | sans session | `SeConnecter` (principal Anonyme), puis `OuvrirSession` (Utilisateur) |
| `POST /api/connexion/code-recuperation` : courriel, code ; session restreinte | sans session | `SeConnecter` (Anonyme), puis `OuvrirSession` (Utilisateur) |
| `POST /api/session/reauthentification` : mot de passe, code ; session fraîche, jeton tourné | session complète | `SeReauthentifier` |
| `GET /api/moi/facteurs` : secours (utilisable ou non), codes restants | session complète | `ConsulterSesFacteurs` |
| `GET /api/moi/evenements` : ses 100 derniers événements | session complète | `ConsulterSesEvenements` |
| `POST /api/moi/facteurs/secours/preparation` | session complète fraîche, ou restreinte | `EnrolerSecours` |
| `POST /api/moi/facteurs/secours` : jeton, mot de passe, code ; dix codes rendus | session complète fraîche, ou restreinte | `EnrolerSecours` |
| `POST /api/moi/facteurs/codes-recuperation` : nouvelle série | session complète fraîche | `RegenererCodesRecuperation` |

**Cedar et la connexion.** Toute route passe par Cedar (règle absolue 7). La connexion n'a pas
encore d'utilisateur authentifié : elle est décidée en deux temps (`crates/authz/src/facteurs.rs`,
`politiques/facteurs.cedar`). D'abord `SeConnecter`, de principal `Anonyme` et de ressource
`Instance` : action **publique et bornée**, qui ne rend rien par elle-même, dont la limitation de
débit borne les essais, et qu'aucun autre type n'admet (contrôlé au chargement des politiques).
Puis, le facteur vérifié et consommé, `OuvrirSession` avec le principal `Utilisateur` et, en
contexte, la méthode vérifiée et la nature de la session : le secours n'ouvre qu'une session
complète, le code de récupération qu'une session restreinte (invariant
`invariant-recuperation-restreinte`). La nature est en outre fixée par le facteur dans le code :
même une décision forcée à « autorisé » n'ouvre qu'une session restreinte avec un code (test
`isolation/facteurs.rs`). Les autres routes ont pour principal l'utilisateur de la session,
pour ressource lui-même, et pour contexte la session lue en base (nature, méthode, fraîcheur) ;
la RLS et le filtre des requêtes bornent tout à cet utilisateur.

**Hors de cette PR** : passkeys (PR 8c), réinitialisation par un administrateur et alertes par
courriel (PR 9), purge des événements (durée à fixer, ADR 0017).

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

## Travailler en voies parallèles

Trois voies avancent en parallèle sur le lot L2 : **authentification** (PR 8b, 8c, 9),
**comptabilité** (PR 11 à 15 : référentiel, pièces, brouillard, éditions, intégrité) et
**interface** (PR 16 à 18, `web/`). Règle : **une voie n'édite que ses modules**. Les registres
communs (liste des modules de routes, cas d'usage, fichiers de cas d'isolation) sont déjà
remplis pour tous les modules prévus ; les points d'insertion communs qui restent ont un
emplacement par voie, séparé des autres par un commentaire.

| Voie | Fichiers qui lui appartiennent |
|---|---|
| Authentification | `src/serveur/routes/{facteurs,passkeys,administration}.rs`, `tests/isolation/{facteurs,passkeys,administration}.rs`, `catalogue/<module>.toml` de ses tables, migrations `0100` à `0199`, `crates/authz/politiques/schema/utilisateur.cedarschema` et `utilisateur.cedar`, section « utilisateur » de `crates/authz/src/action.rs` ; méthode de secours (PR 8b) : `src/facteurs/`, `src/serveur/facteurs.rs`, `src/base/{facteurs,evenement}.rs`, `catalogue/facteurs.toml`, `crates/authz/politiques/schema/facteurs.cedarschema`, `crates/authz/politiques/facteurs.cedar`, `crates/authz/src/facteurs.rs` |
| Comptabilité | `src/serveur/routes/{referentiel,brouillard,editions}.rs` et leurs fichiers de cas, migrations `0200` à `0299` ; pièces (`pieces`) : `0300` à `0399` ; intégrité (`integrite`) : `0400` à `0499` ; `crates/authz/politiques/schema/{dossier,ecritures}.cedarschema`, `roles/{dossier,ecritures}.cedar`, sections correspondantes de `action.rs` |
| Interface | `web/` |

Points d'insertion communs, une ligne ou un bloc par voie : bras de `routes::executer` et de
`serveur::moi::executer` (remplacer `match cas {}` par l'appel au module), emplacement de la voie
dans `serveur::config::Configuration`, index des variables ci-dessus, `corps_de` de
`tests/commun/serveur.rs` (corps d'exemple d'une mutation). Les champs de journal d'un module se
déclarent dans sa tranche (`Module::champs_journal`), ceux d'un module hors routes à côté de
`horodatage::CHAMPS_JOURNAL`. `crates/authz/politiques/invariants.cedar` reste d'un seul tenant :
une modification y est revue par toutes les voies.

`Cargo.lock` se **régénère**, il ne se fusionne pas à la main : en cas de conflit, reprendre la
version de `main` puis relancer `cargo build` (ou `cargo update -p <crate>` pour la seule
dépendance ajoutée). De même, `.sqlx/` se régénère par `cargo sqlx prepare --workspace`.
