# `web/` — interface de l'application

**Rôle :** interface cabinet et portail client (PWA). SvelteKit compilé en **fichiers statiques** (`adapter-static`), servis par le backend Rust. Aucun serveur Node en production.

**Spec :** §7, §11, §19. **Fondations :** §2.5.

## Règles propres
- TypeScript **strict** ; aucune logique métier ni décision d'autorisation dans le navigateur (l'interface masque, le serveur décide).
- Aucune donnée sensible dans `localStorage`/`sessionStorage`.
- Compatible CSP stricte et Trusted Types : pas de `{@html}` sur une donnée non maîtrisée, pas de `eval`, pas de script inline.
- Dépendances minimales ; lockfile versionné ; `pnpm audit` dans la CI.
- Imports internes par `#lib/…` **avec leur extension** (`#lib/api/client.ts`) : les modules purs de `src/lib/api/` et `src/lib/instants.ts` sont testés par `node --test` (`pnpm test`, types retirés par Node), sans SvelteKit. Ils n'importent donc rien de `$app/*`.
- `pnpm build && pnpm test` contrôle aussi la CSP des pages compilées (`src/csp.test.ts`) ; sans `build/`, ce contrôle est sauté.
- Le cabinet et le dossier sont dans l'URL de l'interface (`?cabinet=…&dossier=…`), validés par `lireUuid` avant de former un chemin de l'API ; aucun cabinet courant caché.
- Portail client : conçu pour téléphones Android d'entrée de gamme et connexion intermittente.
