# 0017 — Événements d'authentification hors cabinet et conservation des sessions

- **Statut :** acceptée (2026-11-19, par le mainteneur)
- **Date :** 2026-11-19

## Contexte
L'ADR 0012 demande que chaque étape de la récupération de compte soit « inscrite au journal d'audit (ADR 0014) et alertée ». Or l'ADR 0014 ne connaît que deux portées, le dossier et le cabinet. Plusieurs événements d'authentification n'en relèvent pas, car ils précèdent le choix d'un cabinet ou concernent un utilisateur membre de plusieurs cabinets :
- connexion réussie ou refusée ;
- connexion par la méthode de secours ;
- enrôlement ou retrait d'un facteur ;
- génération ou usage d'un code de récupération ;
- révocation de sessions par l'utilisateur.

La table `session` (PR 8a, issue #159) garde aussi, pour chaque session, l'appareil et l'adresse IP. L'adresse IP est une donnée personnelle (loi n° 2008-12 ; modèle de menaces, VP9 et Q15).

Décisions du mainteneur du 2026-11-19 (questions 3 et 4 de l'étude préalable de la PR 8).

## Décision
### 1. Journal d'instance des événements d'authentification
- Une table d'instance `evenement_authentification`, sans `cabinet_id`, inscrite dans la liste des tables d'instance de `crates/api/catalogue.toml`.
- **RLS activée et forcée.** Un utilisateur authentifié ne lit que ses propres événements (`utilisateur_id = sigma.utilisateur`, réglage réservé à un utilisateur authentifié, migration `0004`).
- **Ajout seul** : le rôle applicatif n'y a qu'`INSERT`, sans `UPDATE` ni `DELETE`. La table entre dans la liste « ajout seul » du catalogue.
- **Contenu** :
  - nature de l'événement, dans une liste fermée ;
  - utilisateur, quand il est connu : rien n'est enregistré pour un courriel inconnu, afin de ne pas constituer une liste de courriels essayés ;
  - session, s'il y en a une ;
  - méthode ;
  - instant ;
  - appareil (200 caractères au plus) et adresse IP ;
  - issue.
  
  Jamais de mot de passe, de code, de jeton ni d'empreinte.
- **Non chaîné.** Ce journal sert à la surveillance et à la consultation par l'utilisateur, pas à prouver l'intégrité de la comptabilité. Les actions qui touchent un cabinet restent dans l'audit chaîné de l'ADR 0014.

### 2. Inscription dans l'audit chaîné de chaque cabinet
Les événements qui affectent la sécurité de l'accès aux cabinets sont **aussi** inscrits dans la chaîne d'audit de **chaque cabinet** où l'utilisateur a une appartenance active, selon l'ADR 0014, dans la même transaction :
- usage d'un code de récupération ;
- réinitialisation par un administrateur, à chacune de ses étapes (ADR 0012) ;
- enrôlement ou retrait d'un facteur ;
- connexion par la méthode de secours.

Les administrateurs de ces cabinets voient ainsi, dans leur propre audit, ce qui pourrait signaler une prise de compte (F1.12, F1.15). Les alertes sont envoyées par la file de tâches (PR 9).

### 3. Conservation
- **Sessions expirées ou révoquées : 30 jours**, durée réglable (`SIGMA_CONSERVATION_SESSIONS_JOURS`), puis suppression. La purge ne porte que sur l'utilisateur dont une session s'ouvre ; aucune opération ne traverse plusieurs utilisateurs.
- **Événements d'authentification** : durée à fixer avec les autres durées de conservation (modèle de menaces, Q15). Aucune purge en L2.

## Conséquences
- F1.6 (répudiation), F1.12 (récupération) et F1.15 (administrateur malveillant) : les événements sensibles d'un utilisateur sont visibles par lui et par les administrateurs de ses cabinets.
- La PR 8b crée la table et inscrit les événements de connexion et de secours. Les étapes de la réinitialisation par un administrateur arrivent avec la PR 9.
- Le registre des traitements (Fondations §3.13) mentionne ce journal, avec la conservation de 30 jours des sessions.

## Alternatives écartées
- **Événements dans l'audit d'un seul cabinet** : un utilisateur membre de plusieurs cabinets, ou qui n'a pas encore choisi de cabinet, n'a pas de cabinet unique.
- **Troisième portée chaînée, « utilisateur »** : une chaîne par utilisateur, à exporter et vérifier, sans gain de preuve pour la comptabilité. La surveillance n'exige pas de chaîne, et les événements qui touchent un cabinet sont déjà chaînés dans son audit.
- **Enregistrer les tentatives pour un courriel inconnu** : cela constituerait une liste de courriels essayés, donnée personnelle sans utilité de sécurité suffisante. La limitation de débit (PR 6) couvre l'abus.
- **Garder les sessions expirées sans limite** : conservation d'adresses IP sans finalité, contraire à la loi n° 2008-12.
