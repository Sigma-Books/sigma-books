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
| Modifications | comptes du dossier de test au lieu du plan Tryton ; ligne en seconde devise (30 EUR pour 32) exprimée par un cours (1.0667), dont Sigma Books calcule la contre-valeur ; variante « Cancel Move » par montants négatifs non reprise (spec §6.2, inv. 2) ; lettrage automatique de l'origine et du miroir non repris (lettrage : lot L3) |
| PR | PR 4 du lot L1 (#64, #59, #63) |

## syscohada-collectifs — comptes collectifs et transcription des intitulés du plan SYSCOHADA

| Champ | Valeur |
|---|---|
| Projet | Tryton (module `account_syscohada`) |
| Dépôt | https://github.com/tryton/tryton |
| Fichier(s) d'origine | `modules/account_syscohada/account_syscohada_2016_fr.xml` (enregistrements `account.account.template` : champs `code`, `name`, `party_required`) |
| Commit ou version | `f5354ccd9231703470611d7ebf1c9cdc65e952bc` |
| Licence | GPL-3.0-or-later (en-tête du fichier : « This file is part of Tryton », renvoi au fichier COPYRIGHT du module, vérifié) |
| Auteurs / titulaires | © 2022-2026 B2CK SRL ; © 2022-2026 Cédric Krier (fichier `modules/account_syscohada/COPYRIGHT`) |
| Nature de la reprise | Jeu de données. (1) Liste des comptes marqués `party_required`, reprise comme comptes collectifs de tiers. (2) Accents et graphie des intitulés, repris seulement lorsque le texte coïncidait exactement, à la casse et aux accents près, avec l'OCR de la liste officielle. Chaque intitulé a ensuite été relu sur le PDF officiel, qui fait foi. |
| Destination | `crates/syscohada/data/comptes.toml` (champs `collectif` et `intitule`) |
| Modifications | Comptes absents de la liste officielle écartés (4126…). Ajout de 404, 413 et 467 comme collectifs. Intitulés alignés sur le texte officiel du JO OHADA du 15/02/2017. Détail dans `crates/syscohada/README.md`. |
| PR | voir la PR de l'issue #53 |

## payment-term — répartition d'un total selon une condition de règlement

| Champ | Valeur |
|---|---|
| Projet | Tryton (module `account_invoice`) |
| Dépôt | https://github.com/tryton/tryton |
| Fichier(s) d'origine | `modules/account_invoice/payment_term.py` (`PaymentTerm.compute`, `check_remainder`) ; `modules/account_invoice/tests/test_module.py` (`test_payment_term`, `test_negative_amount`) |
| Commit ou version | `f5354ccd9231703470611d7ebf1c9cdc65e952bc` |
| Licence | GPL-3.0-or-later (en-tête des fichiers : « This file is part of Tryton », renvoi au fichier COPYRIGHT, vérifié) |
| Auteurs / titulaires | © 2008-2026 Cédric Krier, © 2008-2013 Bertrand Chenal, © 2008-2026 B2CK SRL, © 2004-2008 Tiny SPRL (fichier `modules/account_invoice/COPYRIGHT`) |
| Nature de la reprise | Algorithme adapté (reste décroissant, terme plafonné au reste, terme nul omis, total nul donnant un terme nul, signe reporté) et cas de test repris (montants et dates attendus), réécrits en Rust |
| Destination | `crates/compta-core/src/echeance/condition.rs`, `crates/compta-core/src/echeance/tests.rs` |
| Modifications | Pourcentage du total seulement (pas de « pourcentage du reste » : ADR 0033) ; premier terme du test exprimé en pourcentage du total (même montant) et délais `relativedelta` exprimés en jours (mêmes dates) ; arrondi demi éloigné de zéro (Tryton : au pair) ; résultat trié par date ; total plus précis que la devise refusé au lieu de l'erreur « reste non nul » |
| PR | PR 3b du lot L3 (L3-08) |
