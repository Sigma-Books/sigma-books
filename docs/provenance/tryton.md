# Provenance — Tryton

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
