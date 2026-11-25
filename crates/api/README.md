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
| `0100` à `0199` | Authentification (PR 8b `0100`, 8c `0101`, 9) |
| `0200` à `0299` | Comptabilité (PR 11, 13, 14) |
| `0300` à `0399` | Pièces (PR 12) |
| `0400` à `0499` | Intégrité et horodatage (PR 15) |

Exception, pour la dépendance entre voies décrite ci-dessous : `0350_brouillard_pieces.sql` (voie
comptabilité, PR 13a) lie le brouillard aux pièces (`0300_pieces.sql`) ; elle porte un numéro
supérieur à celui de la voie dont elle dépend, hors de sa plage. Même cas pour
`0351_ecriture_validee_pieces.sql` (PR 13b), qui lie les écritures validées (`0203_validation.sql`,
dans la plage) aux pièces ; `0352_piece_liee_non_ecartee.sql` (déclencheur sur `piece`) en dépend aussi.
`0204_contre_passation.sql` (PR 13c) ne dépend que de `0203` : contraintes d'une contre-passation (et, à l'origine, un index unique sur l'origine). `0205_contre_passations_multiples.sql` lève cette unicité (ADR 0021 point 4, révisé le 2026-11-25) : une écriture validée peut être contre-passée plusieurs fois ; l'index sur l'origine devient non unique.

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
(« Méthode de secours » ci-dessous) ; les passkeys (PR 8c) : « Passkeys, WebAuthn » ci-dessous. Sans session ouverte,
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
| `SIGMA_PIECES_S3_POINT_ACCES`, `SIGMA_PIECES_S3_COMPARTIMENT`, `SIGMA_PIECES_S3_CLE`, `SIGMA_PIECES_S3_SECRET` | oui | pièces (`src/pieces/config.rs`) |
| `SIGMA_PIECES_S3_HTTP_ASSUME` | oui (point d'accès `http://`) | pièces (`src/pieces/config.rs`) |
| `SIGMA_PIECES_S3_REGION`, `SIGMA_PIECES_S3_STYLE`, `SIGMA_PIECES_TAILLE_MAX`, `SIGMA_PIECES_RETENTION_ANS`, `SIGMA_PIECES_DELAI_DEPOT` | non | pièces (`src/pieces/config.rs`) |

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

- **Mot de passe** : 12 caractères au moins (blancs de tête et de fin retirés, suites internes
  réduites, comme pour la liste), 128 au plus, aucune règle de composition ; refusé
  s'il figure dans la liste embarquée des mots de passe compromis (`donnees/`, tirée de SecLists
  par `scripts/generer-mots-de-passe-compromis.py`, attribution `docs/provenance/seclists.md`).
  Argon2id, 19 Mio, 2 passes ; re-haché à la connexion si les paramètres changent. Le hachage
  s'exécute hors de la boucle asynchrone, au plus 4 à la fois, et hors de toute transaction.
- **TOTP** : RFC 6238, 6 chiffres, pas de 30 s, tolérance d'un pas. **Anti-rejeu** : le dernier
  pas accepté est en base ; un pas n'est admis que par une mise à jour conditionnelle
  (`totp_dernier_pas < $pas`, mot de passe inchangé) : de deux connexions concurrentes avec le
  même code, une seule passe. Un réenrôlement ne fait jamais reculer ce pas (`GREATEST`).
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
  session et l'enrôlement du secours, rien d'autre (F1.12).
- **ADR 0012** : la session restreinte sert à enrôler un facteur. Elle enrôle une **passkey**
  (PR 8c, `EnrolerPasskey`) ou remplace le secours (`EnrolerSecours`, écart assumé de la PR 8b,
  conservé : le secours reste le repli de l'ADR).
- **Connexion sans contexte** : la transaction de connexion ne pose **jamais**
  `sigma.utilisateur`, mais `sigma.courriel_connexion` (politiques `*_connexion` de la migration
  `0100`) : elle ne voit que la ligne `utilisateur` du courriel saisi, son secours et ses codes, et
  ne peut qu'y consommer un pas ou un code et inscrire un échec de connexion. Les politiques ne
  comparant pas l'avant et l'après d'une ligne, deux déclencheurs `BEFORE UPDATE`
  (`facteur_secours_garde_connexion`, `code_recuperation_garde_connexion`, sans
  `SECURITY DEFINER`, `EXECUTE` retiré à PUBLIC) refusent, sans utilisateur authentifié, toute
  mise à jour autre que `totp_dernier_pas` en hausse stricte et `utilise_le` de NULL à une
  date ; l'insertion d'un événement y est bornée à un échec de connexion sans session. Type distinct
  (`TransactionConnexion`) ; elle ne devient la transaction de l'utilisateur que sur
  présentation d'un `FacteurVerifie`, fabriqué par la seule consommation d'un facteur. La
  consommation, la session, l'événement et l'audit sont validés ensemble. Aucune fonction
  `SECURITY DEFINER`.
- **Anti-énumération** : courriel inconnu, compte sans facteur, facteur faux ou rejoué, compte
  plafonné : même réponse (401) et même durée. Toute réponse d'échec d'une connexion, sur les
  deux routes, part au plus tôt après une **durée plancher** constante (500 ms, à garder
  au-dessus du pire cas mesuré) ; Argon2id s'exécute aussi dans tous les cas (chaîne factice
  sans secours, et pour la connexion par un code). Rien n'est inscrit pour un courriel inconnu.
- **Débit** : par adresse (20 tentatives de connexion par quart d'heure), par courriel saisi
  (10, que le compte existe ou non ; clé HMAC, le courriel n'est pas gardé en mémoire), par
  utilisateur pour la ré-authentification et l'enrôlement (10). Ces limites en mémoire se
  contournent par éviction (100 000 clés) ; s'y ajoute un **plafond persistant par compte** :
  10 échecs inscrits aux événements sur 15 minutes, au-delà desquels le facteur n'est plus
  vérifié, avec la même réponse 401 et la même durée (jamais un 429, qui révélerait le
  compte). Les tentatives restent inscrites : un tiers qui connaît le courriel peut retarder
  la connexion par secours du titulaire tant qu'il insiste.
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

**Hors de cette PR** : réinitialisation par un administrateur et alertes par courriel (PR 9),
purge des événements (durée à fixer, ADR 0017). Les passkeys (PR 8c) sont décrites ci-dessous.

### Passkeys, WebAuthn (PR 8c ; ADR 0006, ADR 0012, ADR 0018)

Les passkeys sont la méthode principale ; le secours ci-dessus est le repli. La vérification est
celle de `webauthn-rs` 0.6.2 (MPL-2.0, audité par SUSE), sur une copie corrigée de `crypto-glue`
(`vendor/crypto-glue/`, ADR 0018, `docs/provenance/crypto-glue.md`) ; rien de cryptographique
n'est écrit dans le dépôt. Code : `src/facteurs/passkey.rs` (protocole), `src/base/passkeys.rs`
(base), `src/serveur/passkeys.rs` (cas d'usage), migration `0101_passkeys.sql`.

- **Partie de confiance** : l'hôte de `SIGMA_ORIGINE`, seule origine admise. Une origine sans hôte
  nommé (adresse IP) interdit toute passkey : le binaire refuse de démarrer.
- **Vérification de l'utilisateur exigée** (code, biométrie), à l'enrôlement comme à chaque
  authentification, et recontrôlée sur le résultat : une passkey est un facteur fort à elle seule.
  Pas d'attestation, pas de passkey découvrable : on part du courriel, puis `allowCredentials`.
- **Une cérémonie, deux appels** : un « défi » (options WebAuthn rendues, état gardé en base, jeton
  dans le cookie `__Host-sigma_ceremonie`, `HttpOnly`, `Secure`, `SameSite=Strict`, 5 minutes), puis
  la réponse de l'authentificateur. L'état est **à usage unique** (`DELETE … RETURNING`) : une
  réponse rejouée, un défi échu, d'une autre session ou d'un autre compte ne passent pas. Le
  jeton n'est stocké que sous forme d'empreinte SHA-256.
- **Rien de secret en base** : `facteur_passkey.cle` est la `Passkey` sérialisée (clé publique,
  identifiant, compteur) ; la clé privée reste dans l'authentificateur. Le compteur est remplacé
  par une mise à jour conditionnelle (`WHERE cle = <état lu>`) : deux connexions concurrentes ne se
  superposent pas. Un identifiant de credential est unique dans toute l'instance.
- **RLS de la connexion** : comme le secours, sans utilisateur ; elle ne voit que les passkeys et
  les défis de **connexion** du compte du courriel saisi, ne peut changer d'une passkey que son
  état et sa dernière utilisation (déclencheur), et ne devient une transaction d'utilisateur que
  sur présentation du facteur vérifié.
- **Anti-énumération** : le défi d'une connexion rend la même forme (`{"defi": …}`, cookie compris),
  la même durée plancher (en succès comme en échec) pour un courriel inconnu, un compte sans
  passkey et un compte plafonné : faux identifiants dérivés du courriel par la clé de l'instance
  (stables ; 1 à 10, de 16 à 128 octets, tirés comme de vrais), rien d'écrit ; un compte plafonné
  reçoit ses vraies options, sans état gardé. Un échec rend toujours 401 `non_authentifie`.
- **Bornes** : 10 passkeys par utilisateur ; 4 défis de connexion en cours par compte ; nom de
  1 à 100 caractères sans caractère de contrôle ; mêmes débits que le secours.
- **Suppression** : session complète fraîche ; le compte garde au moins un facteur fort (une autre
  passkey ou un secours utilisable), sinon 422 `dernier_facteur` (F1.1). Un enrôlement ou une
  suppression révoque les autres sessions (ADR 0012).
- **Événements et audit** : `connexion` (méthode `passkey`), `enrolement_passkey`,
  `suppression_passkey`, `reauthentification`, et, dans l'audit de chaque cabinet actif,
  `OuvrirSession`, `EnrolerPasskey`, `SupprimerPasskey`. Aucun secret ni credential.

| Route | Portée | Cedar |
|---|---|---|
| `POST /api/connexion/passkey/defi` : courriel ; options d'authentification (réelles ou factices) | sans session | `SeConnecter` (Anonyme) |
| `POST /api/connexion/passkey` : courriel, réponse ; session complète | sans session | `SeConnecter`, puis `OuvrirSession` (Utilisateur) |
| `POST /api/session/reauthentification/passkey/defi`, `…/passkey` : réponse ; session fraîche, jeton tourné | session complète | `SeReauthentifier` |
| `POST /api/moi/facteurs/passkeys/defi`, `POST /api/moi/facteurs/passkeys` : nom, réponse | session complète fraîche, ou restreinte | `EnrolerPasskey` |
| `DELETE /api/moi/facteurs/passkeys/{passkey}` | session complète fraîche | `SupprimerPasskey` |

`GET /api/moi/facteurs` rend aussi ses passkeys (identifiant, nom, dates ; ni clé ni compteur).

**Tests** : `tests/passkeys.rs` (parcours de bout en bout avec un authentificateur logiciel,
`webauthn-authenticator-rs` en dépendance de développement : enrôlement, connexion,
ré-authentification, suppression, rejeu, défi échu, cérémonie d'un autre, concurrence, anti-
énumération, session restreinte, RLS) et `tests/isolation/passkeys.rs` (un cas par route, et
« Cedar se trompe »).

**Surveillance** (ADR 0018) : `cargo deny` ne vérifie pas les avis de la copie locale ; toute PR qui
touche à l'authentification vérifie les avis publiés pour `crypto-glue` et `webauthn-rs`.

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

## Pièces justificatives

ADR 0015 ; code : `src/pieces/` (signature, stockage, client S3, forme canonique de la pièce
interne, accès à la table), `src/serveur/routes/pieces.rs` (routes), migration `0300_pieces.sql`,
format `docs/integrite/piece-interne-v1.md`.

| Route | Cedar | Contenu |
|---|---|---|
| `POST …/dossiers/{dossier}/pieces?nom=&empreinte=&doublon=confirme` | `DeposerPiece` | Corps : le fichier, brut. Type par signature (PDF, JPEG, PNG ; sinon 415), 20 Mio au plus pendant la réception (413), empreinte annoncée différente : 400. Doublon dans le dossier : 200 `{"doublon":true,"pieces":[…]}`, rien de stocké ; `doublon=confirme` l'ajoute (201, journalisé) |
| `POST …/pieces/internes` | `DeposerPiece` | `{"date","motif","lignes"}` : note d'OD canonique, stockée comme un fichier |
| `GET …/pieces?apres=&limite=` | `ConsulterEditions` | Pièces non écartées, plus récentes d'abord, 100 au plus |
| `GET …/pieces/{piece}/contenu` | `ConsulterEditions` | Pièce jointe (RFC 6266), `nosniff`, CSP `sandbox; default-src 'none'` ; empreinte recalculée, écart : 500, alerte `alerte=piece_alteree|piece_absente` et entrée d'audit |
| `POST …/pieces/{piece}/ecart` | `DeposerPiece` | `{"motif"}` : hors des listes, définitif, journalisé |

Consultation : action Cedar `ConsulterEditions` (les quatre rôles du cabinet, §20), retenue
faute d'une ligne « consulter les pièces » au §20 ; aucune action dédiée en L2.

Doublons concurrents : le dépôt verrouille la ligne de son dossier (`FOR NO KEY UPDATE`) avant
de chercher l'empreinte, donc avant d'écrire l'objet. De deux dépôts simultanés du même fichier,
un seul passe sans avertissement, et aucun objet WORM n'est écrit pour l'autre. Un index unique
partiel ne détecterait le doublon qu'à l'insertion de la ligne, après l'écriture de l'objet,
gardé dix ans ; un verrou consultatif partagerait un espace de clés entre tenants (F3.5).

Mutations : rôle relu en base (administrateur, valideur, collaborateur), dossier visible relu
(affectation) ; Cedar n'est pas la seule barrière. La limite de 64 Kio des autres routes reste ;
le dépôt a la sienne (`SIGMA_PIECES_TAILLE_MAX`) et son délai (`SIGMA_PIECES_DELAI_DEPOT`),
appliqués par les intergiciels d'après la route reconnue (`Application::limites`).

**Stockage** : compatible S3, **obligatoire** : sans ses quatre variables, ou sans versionnement
et verrouillage d'objet du compartiment, le serveur refuse de démarrer. Le binaire ne connaît que
ce stockage ; celui en mémoire n'existe que dans `tests/` (test de structure). HTTP seul en L2 :
le TLS vers le stockage est reporté à L9 (relais TLS de l'hébergement ou client TLS) ; le
serveur refuse un point d'accès `http://` sans `SIGMA_PIECES_S3_HTTP_ASSUME=1`, qui assume
explicitement le transport en clair (« L9 : TLS ou relais local »). Chaque dépôt relit la
version écrite (`HEAD`) : sans mode `GOVERNANCE` ni fin de rétention au moins égale à celle
demandée, erreur, et rien n'est inscrit en base.

**Identifiants du stockage** : le rôle applicatif ne doit **jamais** avoir
`s3:BypassGovernanceRetention`, `s3:DeleteObjectVersion`, ni être l'identité racine du service.
Le démarrage le vérifie : il dépose une sentinelle (`sentinelle/<UUID>`, rétention d'un jour) et
tente de la supprimer en contournant le mode gouvernance ; si le service accepte, le serveur
refuse de démarrer. Chaque démarrage laisse donc une sentinelle, effaçable après un jour.

**Limites acceptées en L2** :
- le corps d'un dépôt (20 Mio au plus) est lu **avant** la décision Cedar, après
  l'authentification : le pipeline le lit avant d'ouvrir la transaction, pour qu'un client lent
  ne retienne pas de connexion à la base ; le lire après la décision imposerait de garder la
  transaction ouverte pendant la réception ;
- l'écriture de l'objet (`PUT`, puis `HEAD`) se fait dans la transaction de la requête, verrou
  du dossier tenu : une connexion à la base et le verrou restent pris pendant l'échange avec le
  stockage. Les relâcher imposerait de scinder la transaction du pipeline (audit compris),
  partagée par toutes les voies ; à reprendre si les dépôts d'un dossier se gênent ;
- **objets orphelins** : un objet écrit dont la ligne n'est pas inscrite (échec de la base, de
  l'audit ou du contrôle de rétention après le `PUT`) reste stocké sous verrou jusqu'au terme de
  sa rétention et n'est jamais servi. Aucune tâche ne les signale encore (file de tâches, issue
  à venir). Chaque objet : clé
`<cabinet>/<dossier>/<UUID v4>`, rétention de gouvernance d'au moins dix ans,
`If-None-Match: *`, `x-amz-checksum-sha256` ; lecture de la version écrite (`versionId`). Le rôle
applicatif du stockage n'a ni `s3:DeleteObjectVersion` ni `s3:BypassGovernanceRetention`.

**Service de développement et de test** (Silo, `pgsty/silo` : fork de MinIO, AGPL-3.0, même API ; MinIO
est archivé depuis avril 2026 et n'a plus d'image publiée. Conteneur à soi, port local seulement ;
identifiants tirés au hasard, jamais versionnés). Les images épinglées par empreinte sont dans
`.github/workflows/rust.yml` (`SIGMA_S3_IMAGE`, `SIGMA_S3_CLIENT_IMAGE`) :

```sh
docker run -d --name sigma-pieces-s3 -p 127.0.0.1:9100:9000 \
  -e MINIO_ROOT_USER -e MINIO_ROOT_PASSWORD "$SIGMA_S3_IMAGE" server /data
# Rôle applicatif : lecture et écriture des objets, rétention, configuration du compartiment.
cat > politique.json <<'FIN'
{"Version":"2012-10-17","Statement":[
 {"Effect":"Allow","Action":["s3:GetBucketVersioning","s3:GetBucketObjectLockConfiguration"],
  "Resource":["arn:aws:s3:::sigma-*"]},
 {"Effect":"Allow","Action":["s3:PutObject","s3:PutObjectRetention","s3:GetObject",
  "s3:GetObjectVersion","s3:GetObjectRetention"],"Resource":["arn:aws:s3:::sigma-*/*"]}]}
FIN
# Client `mc` : image `pgsty/mc` (l'image du serveur n'en contient pas).
mc() { docker run --rm --network host -e MC_HOST_local -v "$PWD/politique.json:/politique.json:ro" \
  "$SIGMA_S3_CLIENT_IMAGE" "$@"; }
export MC_HOST_local="http://$MINIO_ROOT_USER:$MINIO_ROOT_PASSWORD@127.0.0.1:9100"
mc admin user add local "$SIGMA_TEST_S3_CLE" "$SIGMA_TEST_S3_SECRET"
mc admin policy create local sigma-pieces /politique.json
mc admin policy attach local sigma-pieces --user "$SIGMA_TEST_S3_CLE"
mc mb --with-lock local/sigma-pieces
```

**Tests** :
- sans service S3 : `tests/pieces.rs` (`tests-postgres`, stockage en mémoire de même contrat) et
  `tests/piece_interne.rs` (vecteurs) ;
- contre le service : `tests/pieces_s3.rs`, fonctionnalité `tests-s3`. Ses tests sont marqués
  `#[ignore]` : `--all-features` les compile et les compte « ignorés » dans l'étape principale
  de la CI. Les lancer :
  `cargo test -p api --features tests-s3 --test pieces_s3 -- --include-ignored`. Sans service ni
  variables (`SIGMA_TEST_S3_*`, `.env.example`), ils **échouent**. Ils créent leurs propres
  compartiments `sigma-test-…` avec l'identité d'administrateur.
- **En CI** : le job `rust-test` lance Silo (`docker run`, identifiants tirés au hasard et masqués),
  prépare le rôle et le compartiment, puis exécute ces tests dans une étape dédiée
  (`-- --include-ignored`). Aucun nouveau check requis.

## File de tâches et horodatage (PR 15a ; ADR 0010, 0022)

- Table d'instance `tache` (migration `0400`) : tâches `horodatage` (quotidien, verrouillage, clôture) et `notification` (exports d'audit, retard, témoin ; courriel : PR 9). Accès : `base::taches` (tenant : `planifier_horodatage`, `notifier` ; travailleur : `TransactionTravailleur`).
- Jetons : `jeton_horodatage` (ajout seul), `base::jetons`.
- Travailleur : `horodatage::tache::Travailleur`, lancé par le binaire `api` (un passage par minute ; `Horodateur` = le client des autorités, remplaçable en test). Pas de variable d'environnement de plus : la liste des autorités est `SIGMA_AUTORITES_HORODATAGE`.
- Tests : `tests/taches.rs` (autorité simulée, jeton figé de L1).

## Intégrité d'un dossier (PR 15b ; ADR 0007, 0010 ; spec §6.4, §21)

Routes `/api/cabinets/{c}/dossiers/{d}/integrite…` (administrateur, valideur, auditeur affectés) :

| Route | Rôle |
|---|---|
| `GET /integrite` | État : nombre d'écritures, tête, jetons, écritures sans jeton, attente et retard d'horodatage (> 24 h), témoin courant |
| `GET /integrite/jetons` | Jetons stockés (autorité, dates, nombre d'écritures couvert) |
| `GET /integrite/verification` | **Recalcul de la chaîne depuis la base** (#188) : rang, empreinte précédente, empreinte recalculée du texte stocké, colonnes (numéro, dates, libellé, format, lignes) rapprochées du texte, tête stockée, témoin et signature de chaque jeton. Rapport « intègre » ou anomalies localisées par rang et numéro (codes stables, jamais un libellé ni un montant) |
| `GET /integrite/temoin` | Témoin externe courant en pièce jointe, à conserver hors de l'instance (F2.4) |
| `POST /integrite/export` | Export d'intégrité au format en vigueur (v3, v4 si une écriture porte une étiquette fiscale), jetons compris ; **session fraîche**, audit (tête, nombre, jetons), alerte à l'administrateur dans la même transaction |

Choix : l'export recopie le texte et les empreintes **stockés** sans rien recalculer (une base altérée se voit à `sigma-verifier`) ; il est construit en mémoire, dans une transaction à instantané unique (limite acceptée en L2, comme l'export d'audit). La vérification reprend le calcul sur l'empreinte stockée après une anomalie : seule l'écriture altérée est signalée. Les signatures des jetons se vérifient hors du fil des requêtes (`spawn_blocking`). Tests : `tests/integrite.rs` (altérations faites par le rôle propriétaire), `api::verification` (propriété sur chaque octet).

## Brouillard

Brouillard persistant et journalisé (PR 13a ; `src/base/brouillard.rs`, `src/serveur/routes/brouillard.rs`).
Un seul état « brouillon », un drapeau facultatif « à revoir », un verrou optimiste (`version`).

- **Drapeau « à revoir »** : on enregistre qui l'a posé (`a_revoir_par`) ; **celui qui l'a posé ne
  le lève pas** (409 `revue_par_le_poseur`). Un drapeau posé par l'IA (lot ultérieur) aura un
  acteur distinct de l'humain, qui le lèvera. Il n'empêche jamais la validation unitaire (13b) ;
  **13c exclura de la validation par lot les écritures marquées** « à revoir ».
- **Période verrouillée** : la date d'un brouillon y est acceptée (seule une période ou un
  exercice clôturé refuse). La validation (13b) la refusera ; la clôture reste bloquée tant que le
  brouillon existe.
- **Montant XOF avec décimales** : accepté en saisie (jusqu'à 20 chiffres entiers, au-delà 422).
  La validation (13b) tranchera l'échelle d'après la devise de tenue : question ouverte.
- **Référentiel chargé sans verrou** : la saisie relit plan, journaux, tiers et exercices sans
  les verrouiller (limite assumée). Un compte ou un tiers peut être désactivé entre la saisie et
  la validation : la validation (13b) relit et recontrôle tout.
- **Lecture** : `GET /brouillard/{id}` s'exécute dans une transaction `REPEATABLE READ` en lecture
  seule (instantané unique). La liste est paginée par le curseur `apres=AAAA-MM-JJ_<uuid>` (clé
  de tri), `suivant` n'est émis que s'il reste une ligne.
- **Tables dépendantes** (`brouillon_ligne`, `_contributeur`, `_piece`) : exemptées du fil-piège
  sous condition (aucune colonne temporelle, clé étrangère vers `brouillon`), voir le catalogue.

## Éditions et alertes

Lectures seules (PR 14), ouvertes aux quatre rôles du cabinet sur les dossiers qu'ils voient
(§20) : `GET …/dossiers/{dossier}/editions/{balance,grand-livre,journaux,centralisateur}` et
`GET …/dossiers/{dossier}/alertes`. Le calcul est celui de `compta_core::editions` ; l'API relit
les écritures validées de l'exercice jusqu'à la fin de la période (et, en mode `provisoire`, le
brouillard équilibré) dans **un seul instantané** (`REPEATABLE READ`, lecture seule), en deux
requêtes à plat par nature d'écriture (`src/base/editions.rs`), et ne calcule rien elle-même.

- Paramètres : `exercice`, `du`, `au`, `mode` (`definitif` par défaut, `provisoire`), `comptes`,
  `journaux` (listes de 50 au plus) ; balance seulement : `format` (2, 4, 6 par défaut, 8) et
  `regroupement` (`compte`, `tronc`). Paramètre inconnu, répété ou invalide : 400.
- Borne : au plus 20 000 écritures par lecture (réglage `editions_ecritures_max`) ; au-delà, 409
  `edition_trop_volumineuse` (réduire la période). Pas de pagination : une édition est un état
  entier. Seules les écritures **de l'exercice** comptent.
- Lecture sur un **pool distinct** (`Requete::pool`, 4 connexions) : la transaction du pipeline
  tient déjà une connexion du pool applicatif ; en prendre une seconde dans le même pool
  interbloquerait l'API sous charge. Test : 16 éditions simultanées pour un pool de 4.
- Une écriture validée relue sans justification (aucune pièce liée) fait échouer l'édition (500) :
  `compta-core` refuse de la reconstituer, et c'est une alerte d'intégrité, non un cas à masquer.
  Seule la validation crée une écriture validée (et exige une justification) ; la reprise de
  données (§9) devra exiger une pièce.
- Mode provisoire : un brouillon déséquilibré ou illisible (valeur stockée que le référentiel
  actuel ne relit plus) est exclu et **signalé** (`brouillons_exclus`), il ne fait jamais
  échouer l'édition.
- Alertes : brouillard non validé par (période, journal) dès le lendemain de la fin de la période
  (AUDCIF art. 22 2°), et échéance de clôture des périodes terminées non clôturées, à 30 jours ou
  dépassée (point 15). La date du jour est celle de l'horloge de la requête.

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
