# Lot L2 — Bilan des tests d'isolation entre tenants

> Établi le 2026-11-25 sur `main` au commit `0b50707` (PR 18 fusionnée), pour le critère de fin du lot : « les tests d'isolation des trois niveaux, le test de catalogue et le test « chaque route » passent en CI à chaque build » (`docs/lots/L2.md`, « Critères de fin »). Cadre : ADR 0011, F1.7, F1.8, F1.14, F2.7.
>
> **Les chiffres sont relevés dans le code et dans la base migrée, pas écrits à la main** ; la section « Régénérer » donne les commandes. Ils changent quand une route ou une table s'ajoute : les tests ci-dessous échouent alors tant que le cas ou la déclaration manque, et ce document se met à jour avec le lot suivant.

## 1. Les trois niveaux et ce qui les éprouve

Une requête franchit trois barrières indépendantes (`crates/api/CLAUDE.md`) : la décision **Cedar**, le **filtre de la requête** SQL, la **RLS** PostgreSQL. Chacune est éprouvée **seule**, les deux autres étant neutralisées.

| Niveau | Fichier de test (`crates/api/tests/`) | Tests | Ce qu'il prouve |
|---|---|---|---|
| Cedar | `crates/authz/` (tests des politiques) et chaque `tests/<voie>.rs` (refus par rôle, séparation des tâches, ré-authentification) | voir ci-dessous | Un utilisateur du cabinet A est refusé sur une ressource du cabinet B ; un collaborateur l'est sur un dossier non affecté ; un membre de deux cabinets n'agit que dans le contexte demandé ; le rôle client n'écrit rien. |
| Requête | `requete.rs` | 3 | Les fonctions d'accès, exécutées par un rôle **qui contourne la RLS** (`sigma_test_contournement`, créé par la seule base de test), ne renvoient rien d'autrui : le filtre de la requête suffit seul. |
| RLS | `rls.rs` | 10 | Avec le rôle applicatif : `SELECT`, `INSERT`, `UPDATE`, `DELETE` bruts sur un autre cabinet n'ont aucun effet ; sans contexte, aucune ligne ; le contexte ne fuit pas d'une transaction à la suivante ; une clé composite ne peut citer un dossier d'un autre cabinet ; le pool refuse un rôle qui contourne la RLS. |
| Cedar défaillant (les trois niveaux ensemble) | `isolation/main.rs`, `isolation/facteurs.rs`, `isolation/passkeys.rs` | 3 | Avec une décision **forcée à « autorisé »** (`ToujoursAutorise`), le filtre de la requête et la RLS ne livrent toujours rien d'autrui ni d'un dossier non affecté. |

Le niveau « Cedar » n'a pas de fichier unique : la décision s'éprouve dans chaque test de voie (un refus par rôle et par action) et dans `isolation/` (appartenance suspendue = non-membre sur chaque route : `une_appartenance_suspendue_vaut_non_membre_sur_chaque_route`).

## 2. Test de catalogue (`catalogue.rs`, 2 tests)

`le_catalogue_respecte_les_regles_d_isolation` lit les catalogues de PostgreSQL de la base migrée ; `une_table_oubliee_fait_echouer_le_controle` introduit dans une base jetable chaque écart possible et vérifie que chacun est détecté. Règles (détail : en-tête du fichier) : RLS **activée et forcée** et politique de forme exacte sur toute table ; le rôle applicatif ne possède rien ; ni `UPDATE` ni `DELETE` sur une table en ajout seul ; aucune fonction `SECURITY DEFINER` ; aucune colonne `real`, `double precision`, `money` (règle absolue 2) ; aucune politique restrictive ; irréversibilité des exercices et périodes clos ; fil-piège des tables du brouillard lues à la clôture d'une période.

Relevé sur la base migrée (migrations `0001` à `0451`) :

| Mesure | Valeur |
|---|---|
| Tables du schéma `public` (hors `_sqlx_migrations`) | 36 |
| dont RLS activée **et** forcée | 36 (toutes) |
| Politiques de RLS | 58 |
| Tables portant `cabinet_id` | 26 |
| Tables portant `dossier_id` | 20 |
| Tables sans RLS | 1 : `_sqlx_migrations` (propriété du rôle de migration, aucun droit pour le rôle applicatif) |
| Fonctions `SECURITY DEFINER` | 0 (liste vide : `catalogue/socle.toml`) |

### Exceptions justifiées (`crates/api/catalogue/*.toml`, chaque entrée porte sa justification)

Les 10 tables sans `cabinet_id` (table `cabinet` comprise) ne suivent pas la règle commune « cabinet ET dossier » ; chacune a sa RLS forcée et des politiques qui lisent le contexte.

| Catégorie | Tables | Pourquoi |
|---|---|---|
| Instance (hors de tout cabinet) | `utilisateur`, `session`, `facteur_secours`, `code_recuperation`, `facteur_passkey`, `defi_webauthn`, `evenement_authentification` | Un compte peut appartenir à plusieurs cabinets (spec §4.1) ; données d'identité et d'authentification, aucune donnée comptable. Visibles de leur seul titulaire (`sigma.utilisateur`) ou, à la connexion, du compte du courriel saisi (`sigma.courriel_connexion`) ou de l'empreinte présentée (`sigma.jeton_session`, `sigma.jeton_defi`). |
| Instance, cibles sans clé étrangère | `lien_acces`, `tache` | Le porteur d'un lien n'a ni session ni cabinet ; le travailleur parcourt la file sans connaître de cabinet. Un tenant ne lit et n'écrit que les lignes de sa cible exacte ; le travailleur est reconnu par un réglage que pose seule `base::taches::TransactionTravailleur`. |
| Dossier facultatif (tout le cabinet quand aucun dossier n'est choisi) | `dossier`, `acces_dossier` | Liste et création des dossiers, administration des affectations : actions du cabinet sans dossier choisi ; Cedar et la requête vérifient l'affectation. |
| Lecture de ses propres cabinets | `appartenance`, `cabinet` | Choisir le cabinet actif : politique `FOR SELECT` seulement, forme exacte « aucun cabinet en contexte ET appartenance de l'utilisateur ». |
| Ajout seul (`UPDATE`/`DELETE` refusés au rôle applicatif) | `ecriture_validee`, `ecriture_validee_ligne`, `ecriture_validee_piece`, `audit_cabinet`, `audit_dossier`, `jeton_horodatage`, `evenement_authentification` | Écriture validée définitive, audit chaîné, jeton d'horodatage : preuves (F2.2, F2.5, F2.6). |
| Hors RLS | `_sqlx_migrations` | Voir ci-dessus. |

## 3. Test « chaque route » (`isolation/main.rs`)

`chaque_route_a_son_cas_d_isolation_et_le_respecte` parcourt `api::serveur::routes::ROUTES` (le routeur n'est construit qu'à partir de cette table : une route absente n'existe pas) et **échoue si une route n'a pas ses attentes** dans le fichier de cas de son module (`isolation/<module>.rs`) ; `chaque_module_a_son_fichier_de_cas` échoue si un module de routes n'a pas son fichier.

Monde : deux cabinets A et B, deux dossiers chacun ; dans chacun un administrateur et un collaborateur affecté au seul premier dossier ; un utilisateur membre des deux (administrateur de A, collaborateur de B affecté au premier dossier de B).

Relevé de `ROUTES` :

| Mesure | Valeur |
|---|---|
| Routes au total | **95** (35 `GET`, 52 `POST`, 8 `DELETE`) |
| Routes d'un cabinet (`{cabinet}` dans le chemin) | 72 |
| Routes hors cabinet | 23 |

| Module | Routes | Module | Routes |
|---|---|---|---|
| `administration` | 20 | `editions` | 5 |
| `referentiel` | 15 | `pieces` | 5 |
| `brouillard` | 14 | `integrite` | 5 |
| `facteurs` | 8 | `organisation` | 4 |
| `passkeys` | 7 | `session` | 3 |
| `audit` | 6 | `moi` | 3 |

- **72 routes d'un cabinet** : pour chacune, de façon générique, une ressource de l'autre cabinet donne **404 avec le même corps** qu'un identifiant inexistant (F1.8), un dossier non affecté donne 404 au collaborateur, l'absence d'authentification donne 401 ; puis les attentes propres au module (qui voit quoi, mutations qui créent réellement la ressource et l'inscrivent à l'audit).
- **23 routes hors cabinet** (exception justifiée : pas de `{cabinet}` à substituer) : session courante, ses sessions, ses cabinets, facteurs de secours, passkeys, usage d'un lien et acceptation d'invitation (ADR 0012). Leurs scénarios s'exécutent avec des **sessions réelles** (cookie, jeton anti-CSRF lié) de deux utilisateurs de cabinets différents et du membre des deux : aucun ne voit ni n'agit sur ce qui est à un autre.
- `les_journaux_d_un_scenario_complet_restent_en_liste_blanche` : les journaux techniques du scénario ne contiennent ni nom, ni montant, ni identifiant (F2.11).

## 4. Démonstration de bout en bout (`demonstration.rs`)

Le test du critère de fin du lot (sur sa **propre base**, créée, migrée puis supprimée par le test) rejoue l'isolation à **chacune de ses dix étapes** : l'administrateur du cabinet B tente 25 lectures et 6 mutations sur les ressources du cabinet A, puis, pour chaque famille de ressources connue à l'étape (brouillon, écriture validée, pièce, période, compte, membre, invitation ; 19 requêtes en tout), présente les **vrais identifiants de A**, chez A et dans son propre cabinet. Le refus (404 ou 403) doit être **indiscernable** de celui d'un identifiant inexistant (même statut, même corps) et ne révéler aucun identifiant ni nom de A ; l'état de A (écritures, brouillons, périodes, chaîne, audit, droits) est relu avant et après et doit être inchangé. Les listes de B (cabinets, dossiers) ne contiennent rien de A.

## 5. Ce que ces tests ne couvrent pas (résidus, voir le modèle de menaces v0.4)

- La RLS est éprouvée avec le rôle applicatif et, pour le niveau « requête », avec un rôle de test qui la contourne ; aucun test n'exerce un rôle d'exploitation (accès nominatifs et journalisés : L9).
- Un **administrateur d'instance** (accès à plusieurs cabinets, réinitialisation d'un membre de plusieurs cabinets) n'existe pas : prévu hors L2.
- Les tests de catalogue lisent le schéma de la base **migrée en CI** ; la conformité de la base de production relève du déploiement (L9).
- Aucun test de charge : l'isolation sous requêtes concurrentes est éprouvée par les tests de concurrence de chaque voie (rangs d'audit continus, numérotation sans trou).

## Régénérer les chiffres

```sh
# Routes : `ROUTES.len()` et par module (test temporaire, non versionné, à ajouter sous crates/api/tests/)
#   use api::serveur::routes::{MODULES, ROUTES};   println!("{}", ROUTES.len());
#   for m in MODULES { println!("{} {}", m.nom, m.routes.len()); }
# Tables et politiques, sur la base migrée (scripts/preparer-base.sh) :
psql -At -c "select count(*) from pg_class c join pg_namespace n on n.oid = c.relnamespace
             where n.nspname = 'public' and c.relkind in ('r','p') and c.relname <> '_sqlx_migrations'"
psql -At -c "select count(*) from pg_class c join pg_namespace n on n.oid = c.relnamespace
             where n.nspname = 'public' and c.relkind = 'r' and c.relrowsecurity
               and c.relforcerowsecurity and c.relname <> '_sqlx_migrations'"
psql -At -c "select count(*) from pg_policies where schemaname = 'public'"
psql -At -c "select count(distinct table_name) from information_schema.columns
             where table_schema = 'public' and column_name = 'cabinet_id'"   # idem 'dossier_id'
# Liste des tests de chaque niveau :
cargo test -p api --all-features --test isolation --test catalogue --test rls --test requete -- --list
```
