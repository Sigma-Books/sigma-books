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
| Modifications | Nombres de jours positifs ou nuls (type `u16`) ; jour du mois typé (0 à 31) au lieu d'un texte ; aucune date du jour par défaut (la date de référence est un paramètre) ; tout dépassement du calendrier renvoie une erreur |
| PR | PR 3b du lot L3 (L3-08) |
