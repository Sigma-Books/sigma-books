# 0025 — Alertes par courriel aux administrateurs du cabinet

- **Statut :** proposée
- **Date :** 2026-11-25

## Contexte
Plusieurs décisions attendaient le courriel de la PR 9 : l'alerte d'export d'audit (#157, ADR 0014 et 0016), l'alerte de retard d'horodatage de plus de 24 heures (F6.2, ADR 0010 et 0022), le témoin remis au cabinet à chaque verrouillage et clôture (question 9b, menaces Q8, F2.4), les alertes de sécurité de l'ADR 0012 (réinitialisation demandée, changement de droits, connexion par le secours, code de récupération utilisé) et la notification des actions d'administration aux autres administrateurs (question 10a, F1.15). L'ADR 0022 crée déjà les tâches `notification` « en attente » ; l'ADR 0024 fournit le transport.

## Décision
1. **Une alerte est une tâche `notification`**, créée **dans la transaction de l'action** qui la motive (elle n'existe que si l'action est validée), sans dossier sauf pour les alertes de dossier. Elle ne porte que des identifiants : cabinet, dossier éventuel, auteur (`acteur_cible`) et utilisateur concerné (`objet`). Sept sujets s'ajoutent à ceux de l'ADR 0022 (migration `0451`) : `reinitialisation_demandee`, `reinitialisation_annulee`, `reinitialisation_aboutie`, `droits_modifies`, `separation_desactivee`, `connexion_secours`, `code_recuperation_utilise`.
2. **Destinataires établis à l'envoi**, en base, sous le contexte du cabinet : tous les administrateurs **actifs** du cabinet de la tâche, jamais d'un autre. Y compris l'auteur : si son compte est détourné, l'alerte part vers sa boîte, qui est celle du titulaire. Pour la demande et l'annulation d'une réinitialisation, le **membre concerné** reçoit aussi un texte qui lui est propre. Une alerte sans destinataire est « sans objet ».
3. **Texte constant par sujet** (`envoi/modele.rs`) : ni nom, ni montant, ni donnée de tiers ; un identifiant de dossier (UUID) pour les alertes de dossier. **Témoin** (question 9b) : nombre d'écritures chaînées, empreinte de tête et, s'il existe, nombre d'écritures couvert par le dernier jeton et son instant d'obtention ; rien de comptable (`format-v2.md` §4) ; le jeton lui-même s'obtient par l'export d'intégrité.
4. **Contestation par le membre concerné** (ADR 0012 : « l'utilisateur et les autres administrateurs peuvent annuler ») : `POST …/reinitialisations/{id}/contestation`, action Cedar `AnnulerSaReinitialisation` (tout rôle du cabinet) ; la requête vérifie en base que la réinitialisation est la sienne, sinon 404 (même réponse qu'une réinitialisation absente). Elle ferme le lien et alerte.
5. **Connexion par le secours ou par un code de récupération** : l'alerte est créée par la fonction qui inscrit l'événement à l'audit de chaque cabinet actif de l'utilisateur (ADR 0017 §2), donc pour chacun de ses cabinets, dans la même transaction.
6. **Remise « au moins une fois »** : un échec provisoire rejoue l'envoi à **tous** les destinataires (délai doublé, huit tentatives) ; un destinataire déjà servi peut recevoir un doublon. Choisi pour ne pas multiplier les tâches par destinataire.
7. **Purge** : les notifications terminées depuis plus de 30 jours sont supprimées avec les autres tâches terminées (ADR 0022).

## Conséquences
- Un export, un changement de droits ou une réinitialisation ne bloquent jamais sur le courriel : l'alerte part ensuite, par la file ; sans relais SMTP, elles attendent (ADR 0024).
- Les alertes sont des informations de sécurité, non une preuve : la preuve est l'audit chaîné (ADR 0014).
- Aucune préférence de notification en L2 : tous les administrateurs actifs reçoivent tout.

## Alternatives écartées
- **Alerter seulement les « autres » administrateurs** : un administrateur seul, ou dont le compte est pris, ne serait jamais prévenu.
- **Stocker les destinataires dans la tâche** : adresses personnelles dans une table d'instance, et liste périmée si les membres changent avant l'envoi.
- **Une tâche par destinataire** : plus de lignes pour un bénéfice limité à l'exactement-une-fois.
- **Joindre le jeton RFC 3161 au courriel du témoin** : le message resterait un texte brut ; l'export d'intégrité le remet déjà.
