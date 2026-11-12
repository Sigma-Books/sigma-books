# crate `api` — serveur applicatif

**Rôle :** serveur HTTP (axum) : authentification (passkeys/WebAuthn, TOTP), sessions, autorisation (via `authz`), accès PostgreSQL (`sqlx`), orchestration des cas d'usage, service des fichiers statiques de l'interface, file de tâches dans PostgreSQL.

**Spec :** §19 à §21. **Fondations :** §3.

## Règles propres
- Chaque requête : authentification → contexte `{cabinet, dossier, utilisateur, rôle}` → décision Cedar → transaction PostgreSQL avec `SET LOCAL` du contexte pour la RLS.
- Requêtes SQL **vérifiées à la compilation** (`sqlx::query!`). Aucune concaténation de SQL.
- Rôles PostgreSQL séparés ; le rôle applicatif n'a **pas** `UPDATE`/`DELETE` sur les écritures validées ni sur le journal d'audit.
- Cookies `HttpOnly`, `Secure`, `SameSite=Strict` ; CSP stricte ; en-têtes de sécurité systématiques.
- Ré-authentification pour : validation par lot, clôture, déverrouillage de période, export complet, changement de droits.
- **Test d'isolation entre tenants** obligatoire pour chaque nouvelle route.
- Toute action métier écrit dans le journal d'audit.
