# Reprises depuis Odoo

Odoo (édition communautaire) est publié sous LGPL-3.0 (fichier `LICENSE` à la racine ; `'license': 'LGPL-3'` dans le manifeste du module `account`), donc portable dans Sigma Books (AGPL-3.0-or-later) avec attribution (voir `README.md`).

## delais-de-reglement — types de délai d'un terme de règlement

| Champ | Valeur |
|---|---|
| Projet | Odoo 19.0 (module `account`) |
| Dépôt | https://github.com/odoo/odoo |
| Fichier(s) d'origine | `addons/account/models/account_payment_term.py` (`AccountPaymentTermLine._get_due_date`, champ `delay_type`, contrainte `_check_valid_char_value`) ; `addons/account/tests/test_payment_term.py` (`test_payment_term_days_end_of_month_on_the`, `test_payment_term_days_end_of_month_nb_days_0`, `test_payment_term_days_end_of_month_nb_days_15`, `test_payment_term_days_end_of_month_days_next_month_0`) |
| Commit ou version | `fa175fb2b1dd69883188ddbd07d0d0b7a6cf9118` |
| Licence | LGPL-3.0 (fichier `LICENSE` et manifeste `addons/account/__manifest__.py`, vérifiés ; les fichiers n'ont pas d'en-tête de licence propre) |
| Auteurs / titulaires | © Odoo S.A. |
| Nature de la reprise | Algorithme traduit (quatre types de délai, jour du mois borné, jour 0 = fin du mois) et cas de test repris (dates attendues), réécrits en Rust |
| Destination | `crates/compta-core/src/echeance/delai.rs`, `crates/compta-core/src/echeance/tests.rs` |
| Modifications | jour du mois typé (0 à 31) au lieu d'un texte ; aucune date du jour par défaut (la date de référence est un paramètre) ; tout dépassement du calendrier renvoie une erreur |
| PR | PR 3b du lot L3 (L3-08) |

## syscohada-lettrables — comptes lettrables et préfixes de trésorerie du plan SYSCOHADA

| Champ | Valeur |
|---|---|
| Projet | Odoo 19.0 (module `l10n_syscohada`) |
| Dépôt | https://github.com/odoo/odoo |
| Fichier(s) d'origine | `addons/l10n_syscohada/data/template/account.account-syscohada.csv` (colonnes `code`, `reconcile`) ; `addons/l10n_syscohada/models/template_syscohada.py` (`bank_account_code_prefix`, `cash_account_code_prefix`, `transfer_account_code_prefix`) ; `addons/account/models/company.py:113-115` (compte de virement interne lettrable) |
| Commit ou version | `fa175fb2b1dd69883188ddbd07d0d0b7a6cf9118` |
| Licence | LGPL-3.0 (manifeste `addons/l10n_syscohada/__manifest__.py` : `'license': 'LGPL-3'`, vérifié) |
| Auteurs / titulaires | © Odoo S.A. (manifeste : `'author': 'Odoo S.A.'`) |
| Nature de la reprise | Jeu de données : comptes marqués `reconcile`, l'une des deux sources (avec Tryton, `tryton.md`, `syscohada-lettrables`) de la marque `lettrable` ; préfixes 521 et 571, recoupés avec la spécification §9.1 pour la marque `type_tresorerie`. |
| Destination | `crates/syscohada/data/comptes.toml` (champs `lettrable`, `page_lettrable`, `type_tresorerie`, `page_type_tresorerie`) |
| Modifications | Marque posée sur le compte de la liste officielle le plus court qui couvre les comptes d'Odoo (40, 41, 421, 485, 488, 585), avec la page de la fiche de fonctionnement relue sur le PDF officiel ; comptes d'Odoo hors des collectifs de Sigma Books non repris (4287, 4387, 4441, 4445). Détail dans `crates/syscohada/README.md`. |
| PR | PR 5a du lot L3 (L3-12) |
