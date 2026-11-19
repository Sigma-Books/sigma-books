# 0016 — Consultation de l'audit : la portée cabinet est réservée à l'administrateur

- **Statut :** acceptée (2026-11-19, par le mainteneur)
- **Date :** 2026-11-19

## Contexte
L'ADR 0014 crée deux portées de chaîne d'audit (dossier, cabinet) et ouvre la consultation à « administrateur, valideur, auditeur (spec §20) », sans distinguer les portées.

La portée cabinet inscrit la création de **tous** les dossiers, les rôles et appartenances, les affectations aux dossiers et le droit de valider par dossier (ADR 0014, tableau des portées). Un valideur ou un auditeur affecté à un seul dossier y verrait l'existence des autres dossiers et leurs affectations. Cela contredit :
- le cloisonnement : chaque dossier d'un cabinet est étanche (spec §2 principe 12) ;
- l'affectation explicite des dossiers, l'administrateur seul voyant tout (décision Q5.1 du 2026-11-18).

Relevé par la relecture de sécurité de la PR 5 du lot L2 (politiques Cedar).

## Décision
Décision du mainteneur du 2026-11-19 (option a) :
- **Portée cabinet** : consultation, vérification et export réservés à l'**administrateur du cabinet**. L'export reste soumis à ré-authentification (ADR 0012, ADR 0014).
- **Portée dossier** : inchangée. Administrateur, valideur et auditeur, ces deux derniers pour les seuls dossiers qui leur sont affectés (Q5.1).
- Les politiques Cedar (`crates/authz`) appliquent cette règle ; les tests de la matrice des rôles la vérifient.

Le reste de l'ADR 0014 est inchangé.

## Conséquences
- Un valideur ou un auditeur ne voit rien d'un dossier qui ne lui est pas affecté, ni dans les éditions ni dans l'audit.
- Un auditeur externe qui doit contrôler les droits du cabinet passe par l'administrateur, qui lui remet un export de l'audit du cabinet. Cet export est lui-même inscrit à l'audit.
- La matrice du §20 (« Vérifier l'intégrité, consulter l'audit ») se lit désormais par portée : administrateur, valideur et auditeur pour l'audit d'un dossier ; administrateur seul pour l'audit du cabinet.

## Alternatives écartées
- **Filtrer l'audit du cabinet selon les dossiers affectés** : une chaîne filtrée ne se vérifie plus et ne s'exporte plus en entier, ce qui la prive de son rôle de preuve.
- **Garder la consultation de l'ADR 0014 pour les deux portées** : fuite entre dossiers d'un même cabinet, contraire au principe 12 et à Q5.1.
