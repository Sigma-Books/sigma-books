# 0014 — Journal d'audit chaîné : deux portées, format canonique, même transaction

- **Statut :** proposée
- **Date :** 2026-11-18

## Contexte
La spec exige un journal d'audit « consultable (qui, quoi, quand, avant/après), exportable, chaîné » (§21), la journalisation de chaque modification du brouillard avant et après (§6.1) et du déverrouillage motivé d'une période (§4.3). Les Fondations le placent en ajout seul, lu par un rôle séparé (§3.11 ; ADR 0004). Menaces : F1.6 (répudiation), F1.15 (administrateur malveillant), F1.16 (collusion), F2.6 (altération de l'audit).

Décisions du mainteneur (2026-11-18) : droit de valider par dossier, séparation des tâches activée par défaut, désactivable par l'administrateur avec ré-authentification, audit et alerte (Q5) ; risques résiduels acceptés avec audit chaîné **incluant les changements de droits** et notification aux autres administrateurs (Q10a) ; témoin envoyé au cabinet à chaque verrouillage et clôture (Q9b) ; aucun chiffrement applicatif par défaut.

Le cloisonnement (spec §2 principe 12) interdit qu'un export d'audit remis pour un dossier révèle quoi que ce soit d'un autre dossier.

Constats des moteurs de référence (`references/moteurs_compta/`) :
- **Dolibarr** `Logevents` trace connexions, utilisateurs et groupes, mais chaque événement est **désactivé par défaut** et l'entrée **n'est pas chaînée** (`htdocs/core/triggers/interface_20_all_Logevents.class.php:37-49, 99-102`). Son journal inaltérable `blockedlog` est chaîné et **inscrit chaque export** (`BLOCKEDLOG_EXPORT`, `htdocs/blockedlog/admin/blockedlog_archives.php:708`) : nous reprenons cette idée.
- **Odoo** : suivi par champ (`tracking=True`, `mail_tracking_value`), sans chaînage, modifiable par un administrateur.
- **Bigcapital** (`modules/AuditLogs`) : 55 événements métier, aucun sur les rôles ni les utilisateurs.

Aucun ne trace de façon chaînée et obligatoire les changements de droits.

## Décision

### Deux portées de chaîne
| Portée | Ce qui y est inscrit |
|---|---|
| **Dossier** (une chaîne par dossier) | Brouillard : création, modification (avant/après), suppression. Validation, contre-passation. Pièces. Référentiel du dossier (comptes, journaux, tiers, exercices). Périodes : verrouillage, déverrouillage (motif), clôture informatique. Horodatage obtenu ou en échec. Envoi du témoin. Exports (intégrité, audit, éditions complètes) |
| **Cabinet** (une chaîne par cabinet) | Création du cabinet (première entrée, par la commande d'administration, Q4a). Utilisateurs : invitation, activation, suspension. Rôles et appartenances. Affectation aux dossiers, droit de valider par dossier. Séparation des tâches (activation, désactivation, motif). Création et archivage d'un dossier. Révocation de sessions par un administrateur. Export de l'audit du cabinet |

- Une action ne s'inscrit que dans une portée, sauf la création d'un dossier : entrée au cabinet, puis première entrée de la chaîne du dossier.
- Les **changements de droits et d'administration sont toujours inscrits**, sans réglage possible.
- Les événements d'authentification antérieurs au choix du cabinet (connexion, enrôlement, récupération) relèvent de l'ADR 0012.

### Contenu d'une entrée
| Donnée | Clé | Remarque |
|---|---|---|
| Qui | `acteur=`, `role=`, `session=` | UUID de l'utilisateur ou `systeme` (tâche de la file) ; rôle dans le cabinet ; identifiant public de la session, **jamais son jeton** |
| Quoi | `action=`, `ressource=` | Identifiant d'action Cedar (`ValiderEcriture`…) ; `type:uuid` de la ressource |
| Quand | `instant=` | UTC à la microseconde, horloge de `api` |
| Contexte | `cabinet=`, `dossier=` | `dossier=` en portée dossier seulement |
| Décision Cedar | `politique=` (une ou plusieurs), `politiques=` | Politiques déterminantes ; version du jeu de politiques |
| Ré-authentification | `reauthentification=` | `oui` ou `non` |
| Motif | `motif=` | Si l'action l'exige (déverrouillage, désactivation de la séparation des tâches) |
| Avant/après | `champ=`, `avant=`, `apres=` | Champs modifiés seulement |

**Minimisation.** Seuls les champs modifiés figurent dans `avant`/`apres`. Une validation ne recopie pas l'écriture : elle cite son identifiant, son numéro et son empreinte, déjà chaînés au grand livre (ADR 0007). L'acteur est désigné par son UUID ; son nom est résolu à l'affichage. Aucune adresse IP (donnée personnelle conservée dix ans dans un journal immuable, loi n° 2008-12) : elle reste dans la session (ADR 0012). Jamais de jeton, de secret ni de clé.

Les refus de Cedar ne changent rien : ils vont aux journaux techniques (identifiants seuls, F2.11) et alimentent les alertes de sécurité, pas la chaîne.

### Format canonique
Documenté dans `docs/integrite/audit-v1.md` (PR 7), figé, vérifié par des vecteurs. Mêmes règles communes et limites que `format-v1.md` §1 et §4 : UTF-8, `cle=valeur`, une donnée par ligne, ordre des clés fixe, `\n` final, textes saisis sans caractère de contrôle repris tels quels, lecture stricte (F7.3).

```
audit v1
portee=dossier
cabinet=<uuid>
dossier=<uuid>
rang=<n>
instant=2026-11-18T09:14:03.120000Z
acteur=<uuid>
…
fin
```

### Chaînage
- Comme l'ADR 0007 : `empreinte_0 = SHA-256("sigma-books audit v1\n" ‖ portée ‖ "=" ‖ identifiant)`, puis `empreinte_n = SHA-256(empreinte_{n-1} ‖ entrée canonique n)`. Une chaîne ne peut être greffée ni sur une autre portée ni sur un autre tenant.
- Sérialisation, chaînage et vérification dans `integrite` (pur, RustCrypto) ; `sigma-verifier` vérifie aussi un export d'audit.
- Unicité (portée, identifiant, rang) ; la tête est un état dérivé verrouillé `FOR UPDATE` (ADR 0011).

### Même transaction
- L'entrée est insérée **dans la transaction de l'action** (`crates/api/CLAUDE.md`). Si l'audit échoue, l'action est annulée. Aucune action sans trace, aucune trace sans action.
- Ordre des verrous : ceux de l'action (ADR 0011), puis tête d'audit du cabinet, puis tête d'audit du dossier.
- Les notifications aux autres administrateurs (Q10a) et l'envoi du témoin (Q9b) sont des tâches de la file insérées dans la même transaction : elles partent si et seulement si l'action est validée.

### Rôles et lecture
- **Rôle applicatif** : `INSERT` seul sur les entrées, ni `SELECT`, ni `UPDATE`, ni `DELETE` (F2.6). Il ne relit jamais l'audit.
- **Rôle de lecture d'audit** : `SELECT` seul, sous RLS forcée et contexte du tenant (ADR 0011). Pool de connexions distinct dans `api`.
- Consultation : administrateur, valideur, auditeur (spec §20), après décision Cedar ; pagination imposée (F2.9).
- **Vérification** depuis l'interface : recalcul de la chaîne d'une portée, résultat affiché.
- **Export** : par portée, ré-authentification, alerte à l'administrateur (spec §21) ; l'export lui-même est inscrit à l'audit, avec tête et nombre exportés. Une annexe non chaînée associe chaque UUID d'acteur à son nom au moment de l'export.
- **Chaque export d'intégrité** est inscrit à la chaîne du dossier, avec le témoin exporté (`format-v2.md` §4).

## Conséquences
- Un export d'audit d'un dossier se vérifie seul et ne révèle rien d'un autre dossier ni de l'administration du cabinet.
- Toute modification de droits laisse une trace chaînée qu'un administrateur ne peut ni effacer ni désactiver (F1.15).
- Les actions d'un dossier sont sérialisées par la tête d'audit, comme par la tête de chaîne (ADR 0011) ; les actions du cabinet, peu nombreuses, par la tête du cabinet.
- Tests (PR 7) : chaque action crée une entrée chaînée ; une entrée modifiée, supprimée ou réordonnée est détectée ; le rôle applicatif n'a ni `SELECT`, ni `UPDATE`, ni `DELETE` sur l'audit ; aucun export d'un dossier ne contient d'identifiant d'un autre dossier.
- **Limite connue** : comme pour le grand livre, qui contrôle toute la base peut réécrire une chaîne d'audit entière. Sans ancrage externe de la tête d'audit, ce n'est pas détectable. Un témoin d'audit, ou l'ajout de sa tête au témoin horodaté (nouvelle version de `format-v2.md` §4), est à étudier après L2.
- Si un chiffrement applicatif par cabinet est ajouté plus tard, l'empreinte reste calculée sur le texte canonique en clair : la chaîne n'en dépend pas.

## Alternatives écartées
- **Une chaîne unique par cabinet** : l'export d'audit d'un dossier révélerait les autres, ou serait invérifiable sans eux (principe 12).
- **Une chaîne par dossier seulement** : les droits et l'administration n'ont pas de dossier ; les recopier dans chaque dossier révélerait l'organisation du cabinet.
- **Audit asynchrone** (après validation de la transaction) : une action peut être validée sans sa trace (F1.6, F2.6).
- **Audit désactivable ou facultatif** (Dolibarr `Logevents`) : un administrateur malveillant le couperait d'abord (F1.15).
- **Suivi par champ sans chaînage** (Odoo) : altération indétectable.
- **Journaux techniques comme audit** : non chaînés, conservation courte, liste blanche sans avant/après (F2.11).
- **Déclencheurs PostgreSQL ou `pgaudit`** : voient des requêtes, pas l'acteur, l'intention ni la décision Cedar ; `pgaudit` écrit dans les journaux du serveur, non chaînés.
- **JSON comme format canonique** : ordre des clés, forme des nombres et échappements ambigus (F7.3) ; le format `cle=valeur` du grand livre a déjà son analyseur strict.
- **Copie complète de l'écriture validée dans l'audit** : doublon de données comptables ; la citer par son empreinte suffit.
