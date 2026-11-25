# 0022 — File de tâches de l'instance : table d'instance, travailleur sans cabinet

- **Statut :** acceptée
- **Date :** 2026-11-24

## Contexte
L'ADR 0010 confie l'horodatage à « une tâche de la file PostgreSQL exécutée par le binaire `api` » ; l'ADR 0014 et la spec §21 exigent une alerte à l'administrateur à chaque export. La PR 15a (L2-30, #155, #157) crée la file. Un travailleur doit parcourir les tâches de **tous** les cabinets sans connaître un cabinet à l'avance, alors que toutes les tables à cabinet ou dossier sont sous RLS forcée, de forme exacte vérifiée par le test de catalogue (ADR 0011). Tryton (`ir/queue_.py`) tient une file unique d'instance ; `ir/cron.py` interroge à période fixe.

## Décision
1. **Une table d'instance `tache`** (migration `0400`), sans `cabinet_id` ni `dossier_id` : ses cibles (`cabinet_cible`, `dossier_cible`) sont de simples identifiants sans clé étrangère. Aucune donnée comptable : identifiants, instants, codes stables.
2. **RLS en deux temps.** Un tenant lit et crée les tâches de sa **cible exacte** (cabinet du contexte et son dossier, ou aucun dossier), jamais de mise à jour ni de suppression. Le travailleur (`TransactionTravailleur`) n'a pas de cabinet en contexte et pose `sigma.travail = 'file'` ; la politique correspondante n'ouvre que cette table. Comme `sigma.courriel_connexion` (ADR 0012), c'est un réglage de l'application : il borne ce que la RLS ouvre à ce code, il ne remplace pas le contrôle d'accès. Le **travail** lui-même (lecture de la chaîne, jeton, audit) s'exécute dans une transaction de tenant ordinaire, au contexte tiré de la tâche.
3. **Planification par les événements, sans parcours des dossiers.** La validation d'une écriture (`mettre_a_jour_tete`, donc unitaire, par lot et contre-passation) crée, dans sa transaction, une tâche `quotidien` due au début du jour suivant (UTC), une seule **en attente** par dossier (une tâche en cours n'empêche pas d'en planifier une autre : une validation survenue pendant l'appel à l'autorité garde sa tâche). Le verrouillage et la clôture d'une période créent une tâche due tout de suite. La règle de décision reste celle d'`integrite::horodatage_requis` (au plus un jeton par jour ; une tâche non couverte qui tombe un jour où un jeton existe déjà est reportée au lendemain).
4. **Reprise.** Le travailleur prend **une tâche à la fois** : elle passe `en_cours` avec un bail (5 min, supérieur au pire échange) et un jeton de prise ; seule la prise courante peut la terminer ou la reporter (un travailleur au bail expiré ne clôt pas la prise du suivant) ; un report qui rencontrerait une tâche en attente du même sujet les fusionne (la plus ancienne date de création est gardée, pour le retard). La boucle est supervisée : une panique est journalisée et la boucle relancée. Passé le bail, passé le bail, un autre passage la reprend (`FOR UPDATE SKIP LOCKED`). L'appel à l'autorité se fait hors de toute transaction. Un échec rejoue avec un délai doublé (1 min à 1 h). L'enregistrement du jeton et la fin de la tâche sont deux transactions ; le cas intermédiaire donne un jeton valide et une tâche rejouée « sans objet ».
5. **Alerte de retard (F6.2).** Toute écriture non couverte a une tâche non terminée, créée à la première validation qui a suivi le dernier jeton : une tâche de plus de 24 h crée une notification `retard_horodatage` (une seule non traitée par dossier) et un avertissement technique.
6. **Notifications.** Les alertes d'export (audit du cabinet, d'un dossier ; #157), le témoin à remettre au cabinet (question 9b) et le retard sont des tâches `notification`, créées dans la transaction de l'action : elles n'existent que si l'action est validée. Elles attendent le courriel de la PR 9 (`en_attente`) ; aucune donnée du dossier n'y figure (F12.2).
7. **Jetons** : table `jeton_horodatage`, **ajout seul** (F2.5), un jeton vérifié par `integrite` à la réception.
8. **Audit** : l'obtention d'un jeton est inscrite à l'audit du dossier, acteur « système » (`Acteur::Systeme`, ADR 0014), sans décision de Cedar.

## Conséquences
- Un dossier sans validation ne coûte rien : aucune tâche. Aucun parcours de tous les dossiers.
- La validation prend un `INSERT … ON CONFLICT DO NOTHING` de plus, sous le verrou de la tête de chaîne déjà tenu.
- Plusieurs processus `api` peuvent tourner : `SKIP LOCKED` et le bail évitent le double traitement.
- La purge ne supprime que les tâches d'horodatage terminées depuis plus de 30 jours ; celle des notifications relève de la PR 9.

## Alternatives écartées
- **Parcourir `chaine_tete` de tous les dossiers chaque jour** : exige une lecture inter-tenants d'une table de dossier, donc une exception à la forme exacte des politiques.
- **Rôle PostgreSQL de travailleur avec `BYPASSRLS`** : contraire à l'ADR 0011 (aucun rôle ne contourne la RLS).
- **Fonction `SECURITY DEFINER`** : la RLS forcée s'applique aussi au propriétaire.
- **File externe** (Redis, broker) : une dépendance et un état de plus, sans atomicité avec la validation.
