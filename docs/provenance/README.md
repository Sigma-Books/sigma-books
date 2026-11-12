# Registre de provenance du code porté

Sigma Books (AGPL-3.0-or-later) peut reprendre ou porter du code d'autres projets **uniquement** si leur licence est compatible (`docs/fondations.md` §4.1 et §6, ADR 0009, règle absolue 11 de `CLAUDE.md`). Ce dossier en garde la trace : pour chaque reprise, on doit pouvoir retrouver le fichier d'origine, sa licence et ses auteurs.

## Quand une entrée est obligatoire
- Code **copié** ou **traduit** (porté de Python, PHP, Java… vers Rust), même partiellement.
- **Suite de tests** ou jeu de données de test reprise d'un autre projet.
- Algorithme suivi de près au point que la structure du code d'origine est reconnaissable.

Pas d'entrée pour : l'usage d'une dépendance déclarée dans `Cargo.toml` ou `package.json` (couverte par `cargo deny` et le SBOM), ni pour une idée générale sans reprise de code.

## Licences
| Licence d'origine | Statut |
|---|---|
| AGPL-3.0, GPL-3.0(-or-later), LGPL-3.0 | ✅ portable, avec attribution |
| MIT, Apache-2.0, BSD-2/3-Clause, ISC, Zlib, MPL-2.0 | ✅ portable, avec attribution et avis de licence conservés |
| GPL-2.0-or-later | ✅ portable (sous GPL-3.0+), à vérifier fichier par fichier |
| **GPL-2.0-only**, BSL, licences « non commerciales », absence de licence | ❌ **interdit** : étude seulement, aucune reprise |
| Doute | ❌ ne pas porter, ouvrir une issue avec `statut:à-préciser` |

La licence se vérifie **au niveau du fichier** (en-tête), pas seulement à la racine du projet d'origine.

## Format d'une entrée
Une entrée par reprise, dans un fichier `docs/provenance/<projet>.md` (exemple : `tryton.md`), sous la forme suivante :

```markdown
## <identifiant court> — <ce qui est repris>

| Champ | Valeur |
|---|---|
| Projet | Tryton (module `account`) |
| Dépôt | https://… |
| Fichier(s) d'origine | `modules/account/move.py` (fonctions `…`) |
| Commit ou version | `<hash complet>` ou tag |
| Licence | GPL-3.0-or-later (en-tête du fichier vérifié) |
| Auteurs / titulaires | tels qu'indiqués dans l'en-tête ou le fichier COPYRIGHT |
| Nature de la reprise | traduction en Rust / copie / tests repris / adaptation |
| Destination | `crates/compta-core/src/…` |
| Modifications | résumé des écarts par rapport à l'original |
| PR | #N |
```

## Dans le code
Chaque fichier qui contient du code porté porte un commentaire en tête :

```rust
// Portions portées de Tryton (modules/account/move.py, commit <hash>),
// GPL-3.0-or-later, © <titulaires>. Voir docs/provenance/tryton.md#<identifiant>.
```

Les avis de licence et de copyright d'origine exigés par la licence (notamment Apache-2.0 `NOTICE`, BSD, MIT) sont conservés.

## Revue
Une PR qui porte du code coche « code porté » dans sa description, lie l'entrée du registre et passe la revue CODEOWNERS. Le relecteur vérifie la licence **dans le fichier d'origine**, au commit cité.

## Registre
Aucune reprise à ce jour.
