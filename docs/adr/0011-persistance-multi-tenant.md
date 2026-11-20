# 0011 — Persistance et multi-tenant : rôles, tables, RLS forcée, contexte

- **Statut :** acceptée (2026-11-18, par le mainteneur)
- **Date :** 2026-11-18

## Contexte
L'ADR 0004 retient PostgreSQL, la RLS et trois rôles séparés, sans en fixer la mise en œuvre. Le lot L2 crée le premier schéma (`docs/lots/L2.md`, PR 4, 6, 13). Exigences :
- triple isolation entre tenants : Cedar, requête, RLS (spec §2 principe 11 ; Fondations §3.2) ;
- cloisonnement : chaque cabinet, et dans un cabinet chaque dossier, est étanche (spec §2 principe 12, §4.1) ;
- écriture validée définitive, numérotation sans trou, chaîne par dossier (spec §6.1 à §6.4 ; ADR 0007 ; règle absolue 5) ;
- menaces F1.7, F1.8, F1.14, F1.15, F2.1, F2.2, F2.3, F2.6, F2.7, F2.10.

Décisions du mainteneur (2026-11-18) : création des cabinets par commande d'administration (Q4a) ; affectation explicite des dossiers, l'administrateur voit tout (Q5.1) ; code neutre sur l'hébergement (Q12c) ; disques des serveurs chiffrés par l'infrastructure, **aucun chiffrement applicatif par défaut** : le schéma de L2 n'a pas de colonne chiffrée.

Constats des moteurs de référence (`references/moteurs_compta/`) :
- **Bigcapital** : une base par tenant, choisie par son nom (`bigcapital_tenant_<id>`), avec les **mêmes identifiants de connexion** pour toutes (`modules/Tenancy/TenancyDB/TenancyDB.module.ts:18-33`). L'isolation tient au nommage, pas aux droits.
- **Tryton** (`ir/rule.py`) et **Odoo** (`ir.rule`, `account/security/account_security.xml:131-143`, `company_id in company_ids`) : règles applicatives seules. Une requête SQL directe ou un contournement (`sudo`) les ignore. C'est un niveau sur trois.

## Décision

### Rôles PostgreSQL
| Rôle | Droits | Usage |
|---|---|---|
| Migration | Propriétaire du schéma et des tables ; ni `SUPERUSER` ni `BYPASSRLS` | `sqlx migrate` seulement, jamais à l'exécution (F2.10) |
| Application | `NOBYPASSRLS`, non propriétaire, aucun droit sur le schéma. `SELECT`/`INSERT` sur les tables en ajout seul ; ni `UPDATE` ni `DELETE` sur les écritures validées, leurs lignes, leurs liens aux pièces, ni sur l'audit (`INSERT` seul, ADR 0014) | Binaire `api`, requêtes et file de tâches |
| Lecture d'audit | `SELECT` sur les tables d'audit seulement, `NOBYPASSRLS` | Consultation et export de l'audit (ADR 0014) |

Les trois rôles ont des identifiants distincts, fournis à l'exécution (aucun dans le dépôt). Les noms exacts sont fixés par la PR 4.

### Tables
- **Brouillard et écritures validées sont des tables distinctes**, à l'image des types `EcritureBrouillon` et `EcritureValidee`. L'interdiction d'`UPDATE`/`DELETE` se pose par `GRANT` sur la table entière, sans déclencheur.
- Une écriture validée ne porte **aucun état modifiable**. « Contre-passée » se déduit de la contre-passation qui la cite (champ `origine`).
- **La chaîne est portée par les écritures validées**, en ajout seul : rang dans le dossier, empreinte, empreinte précédente. Contraintes d'unicité : (dossier, rang de chaîne) et (dossier, journal, exercice, numéro). Deux validations concurrentes ne peuvent pas occuper la même place.
- **Tête de chaîne et compteurs** (dernier rang, dernière empreinte ; dernier numéro par journal et exercice) sont un état dérivé, recalculable depuis la chaîne. Le rôle applicatif n'y a `UPDATE` que sur ces colonnes : PostgreSQL l'exige pour `SELECT … FOR UPDATE`. Une tête falsifiée ne modifie aucune écriture validée ; elle produit une chaîne rompue, détectée par la vérification.
- Chaque écriture validée stocke la **date de valeur comptable** (facultative) et la **version du format canonique** utilisée à sa validation (`ecriture v1` ou `v2`, `docs/integrite/format-v3.md` §2). La sérialisation d'une écriture ancienne ne change jamais avec le format en vigueur.
- Montants en `NUMERIC`, dates comptables en `date`, instants en `timestamptz` (règle absolue 2).

### RLS forcée
- Toute table multi-tenant porte `cabinet_id` ; toute table d'un dossier porte aussi `dossier_id`. Elle a `ENABLE` et `FORCE ROW LEVEL SECURITY` : la RLS s'applique aussi au propriétaire.
- Politiques `USING` **et** `WITH CHECK` : la ligne doit appartenir au cabinet du contexte et, pour une table de dossier, au dossier du contexte.
- **Clés étrangères composites** (`cabinet_id`, `dossier_id`, `id`) : une ligne ne peut pas citer une écriture, un compte ou une pièce d'un autre dossier, même par erreur de code.
- **Tables d'instance** sans `cabinet_id` (identité, facteurs d'authentification, sessions : ADR 0012 ; file de tâches ; table de `sqlx`) : liste d'exceptions **versionnée et justifiée**, sans donnée comptable. Une tâche de la file ne contient que des identifiants ; elle s'exécute dans une transaction au contexte de son dossier, avec l'acteur `systeme`.
- L'affectation d'un utilisateur à un dossier (`AccesDossier`) est vérifiée par Cedar et par la requête, pas par la RLS : la RLS garantit l'étanchéité entre cabinets et entre dossiers.

### Contexte de tenant
- Chaque transaction pose cabinet, dossier (absent pour une action du cabinet) et utilisateur par `set_config('sigma.…', $1, true)`, l'équivalent paramétré de `SET LOCAL` (aucune concaténation, F2.1). Jamais de réglage de session : le contexte meurt avec la transaction et ne fuit pas d'une connexion du pool à la suivante.
- **Contexte absent ⇒ aucune ligne** : les politiques lisent `NULLIF(current_setting('sigma.cabinet', true), '')::uuid`. Une comparaison à `NULL` n'est jamais vraie.
- Cabinet et dossier sont **explicites dans chaque requête HTTP** (chemin), revérifiés par Cedar puis par la RLS. Aucun « cabinet courant » en session (F1.14).

### Identifiants
- Identifiants exposés : **UUID v4**, générés dans `api` (l'aléa reste hors de `compta-core`). Jamais d'identifiant séquentiel exposé (F1.8).
- L'identifiant du dossier dans la chaîne (`format-v1.md` §3) est son UUID en minuscules.
- **Ressource d'autrui = ressource absente** : autre cabinet ou dossier non affecté ⇒ même réponse 404, même corps, que pour un identifiant inexistant. Une action refusée sur une ressource visible donne 403.

### Validation
Une validation (ou un lot borné, borne fixée par la PR 13) = **une transaction**, niveau `READ COMMITTED`, verrous explicites dans un **ordre fixe** :
1. période (`FOR SHARE`) : un verrouillage concurrent attend ;
2. tête de chaîne du dossier (`FOR UPDATE`) ;
3. compteur du journal et de l'exercice (`FOR UPDATE`) ;
4. tête d'audit du dossier (ADR 0014).

*Complété par l'ADR 0020 (PR 13b), sans changer l'ordre relatif ci-dessus : exercices avec la période, puis le brouillon, les comptes et tiers cités et les pièces liées s'intercalent avant la tête de chaîne.*

Puis : contrôle par `compta-core` à partir des faits lus, numéro, contenu canonique, empreinte, insertion de l'écriture validée, suppression du brouillon, mise à jour de la tête et du compteur, entrée d'audit. Tout échec annule tout : aucun numéro consommé, aucun trou (F2.3).

### Tests (exigés en CI à chaque build)
| Niveau | Ce qui est prouvé |
|---|---|
| Cedar | Refus inter-cabinets, inter-dossiers, dossier non affecté, utilisateur membre de deux cabinets hors du contexte demandé |
| Requête | Les fonctions d'accès, exécutées par un **rôle de test `BYPASSRLS`** créé par la seule base de test (jamais par une migration), ne renvoient rien d'un autre tenant : le filtre de la requête suffit seul |
| RLS | Rôle applicatif, contexte du cabinet A : `SELECT`, `INSERT`, `UPDATE` bruts sur les lignes de B n'affectent rien ; sans contexte, aucune ligne |
| Catalogue | Toute table hors liste d'exceptions a la RLS activée et forcée, des politiques pour toutes les commandes ; le rôle applicatif n'est propriétaire de rien, n'a aucun droit sur le schéma, ni `UPDATE`/`DELETE` sur les tables en ajout seul ; aucun rôle des migrations n'a `BYPASSRLS` ; aucune colonne `real` ni `double precision` |
| Chaque route | Le test énumère les routes du routeur et échoue si l'une n'a pas son cas d'isolation (ressource d'autrui = ressource absente) |

## Conséquences
- Toute nouvelle table passe le test de catalogue : une table oubliée fait échouer la CI (F2.7).
- Une erreur dans une écriture validée ne se corrige que par contre-passation, quel que soit le rôle (F1.15). Une intervention par le rôle de migration est un incident, hors de l'application.
- Une migration de données qui parcourt plusieurs tenants le fait explicitement, tenant par tenant, visible en revue.
- Toutes les validations d'un dossier sont sérialisées par la tête de chaîne : c'est inhérent à une chaîne par dossier. Les dossiers restent indépendants entre eux.
- Index en tête sur (`cabinet_id`, `dossier_id`) : la RLS ajoute ces conditions à chaque requête.
- **Limite** : la RLS protège contre l'oubli de filtre, pas contre un processus `api` compromis, qui peut poser n'importe quel contexte. Ce risque est borné par les droits en ajout seul, la chaîne, l'horodatage (ADR 0010) et l'audit (ADR 0014).
- PostgreSQL standard, sans extension propre à un hébergeur (Q12c).

## Alternatives écartées
- **Une base par cabinet** (Bigcapital) : migrations et pools multipliés ; un utilisateur de plusieurs cabinets (spec §4.1) exige une identité hors des bases ; aucun cloisonnement entre dossiers d'un même cabinet. Avec des identifiants communs, un `api` compromis atteint toutes les bases : l'isolation n'est pas plus forte.
- **Un schéma par cabinet** : le `search_path` devient le seul sélecteur de tenant, point de défaillance unique ; catalogue démultiplié ; pas de cloisonnement entre dossiers.
- **Règles applicatives seules** (Tryton, Odoo) : un seul niveau ; une requête oubliée ou directe fuit (F2.7).
- **RLS seule** : une politique erronée ou un contexte faux fuit sans bruit. Les trois niveaux se testent séparément.
- **Table unique avec statut** pour brouillard et validées : le refus d'`UPDATE` exigerait un déclencheur ou une politique conditionnelle, plus fragiles qu'un `GRANT`.
- **Niveau `SERIALIZABLE`** : échecs de sérialisation à rejouer ; les verrous explicites rendent l'ordre lisible et testable.
- **Verrous consultatifs** (`pg_advisory_xact_lock`) pour la tête : clé calculée hors des données, collision possible ; le verrou de ligne porte sur l'état qu'il protège.
- **Contrôle de l'affectation dans la RLS** : sous-requête à chaque ligne et politiques croisées entre tables RLS. Cedar et la requête le couvrent ; à rouvrir si un test d'isolation le justifie.
- **Identifiants séquentiels exposés** : énumération (F1.8).
