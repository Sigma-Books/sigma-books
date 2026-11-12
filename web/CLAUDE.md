# `web/` — interface de l'application

**Rôle :** interface cabinet et portail client (PWA). SvelteKit compilé en **fichiers statiques** (`adapter-static`), servis par le backend Rust. Aucun serveur Node en production.

**Spec :** §7, §11, §19. **Fondations :** §2.5.

## Règles propres
- TypeScript **strict** ; aucune logique métier ni décision d'autorisation dans le navigateur (l'interface masque, le serveur décide).
- Aucune donnée sensible dans `localStorage`/`sessionStorage`.
- Compatible CSP stricte et Trusted Types : pas de `{@html}` sur une donnée non maîtrisée, pas de `eval`, pas de script inline.
- Dépendances minimales ; lockfile versionné ; `pnpm audit` dans la CI.
- Portail client : conçu pour téléphones Android d'entrée de gamme et connexion intermittente.
