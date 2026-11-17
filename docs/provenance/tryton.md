# Reprises depuis Tryton

Tryton est publié sous GPL-3.0-or-later (fichiers `LICENSE` et `COPYRIGHT` à la racine du dépôt et de chaque module), donc portable dans Sigma Books (AGPL-3.0-or-later) avec attribution (voir `README.md`).

## move-cancel — scénario de contre-passation (« Cancel reversal Move »)

| Champ | Valeur |
|---|---|
| Projet | Tryton (module `account`) |
| Dépôt | https://github.com/tryton/tryton (miroir du dépôt officiel https://foss.heptapod.net/tryton/tryton) |
| Fichier(s) d'origine | `modules/account/tests/scenario_move_cancel.rst` (sections « Create Move to cancel » et « Cancel reversal Move ») |
| Commit ou version | `f5354ccd9231703470611d7ebf1c9cdc65e952bc` |
| Licence | GPL-3.0-or-later (fichier `modules/account/COPYRIGHT` vérifié ; le scénario n'a pas d'en-tête propre) |
| Auteurs / titulaires | © 2008-2026 Cédric Krier, © 2008-2013 Bertrand Chenal, © 2008-2026 B2CK SRL, © 2004-2008 Tiny SPRL |
| Nature de la reprise | cas de test repris (données et résultats attendus), réécrit en Rust |
| Destination | `crates/compta-core/tests/reference_tryton.rs` |
| Modifications | comptes du dossier de test au lieu du plan Tryton ; ligne en seconde devise (30 EUR pour 32) passée en XOF, car Sigma Books exige l'équilibre par devise (spec §6.2, inv. 1) ; variante « Cancel Move » par montants négatifs non reprise (spec §6.2, inv. 2) ; lettrage automatique de l'origine et du miroir non repris (lettrage : lot L3) |
| PR | PR 4 du lot L1 (#64, #59, #63) |
