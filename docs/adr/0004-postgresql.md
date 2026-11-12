# 0004 — PostgreSQL : RLS, rôles séparés, file de tâches intégrée

- **Statut :** acceptée — **Date :** 2026-11-12

## Décision
- **PostgreSQL** comme unique base.
- **Row Level Security** sur toutes les tables multi-tenant ; contexte positionné par `SET LOCAL` dans chaque transaction.
- Rôles séparés (migration, application, lecture d'audit). Le rôle applicatif n'a pas `UPDATE`/`DELETE` sur les écritures validées ni sur le journal d'audit.
- File de tâches dans PostgreSQL (`FOR UPDATE SKIP LOCKED`) plutôt qu'un service supplémentaire.
- Accès via `sqlx` (requêtes vérifiées à la compilation).
