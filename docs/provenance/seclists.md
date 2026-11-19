# Reprises depuis SecLists

SecLists est publié sous licence MIT (fichier `LICENSE` à la racine du dépôt), compatible avec l'AGPL-3.0-or-later de Sigma Books avec attribution et avis de licence conservé (voir `README.md`).

## mots-de-passe-compromis — liste des mots de passe compromis embarquée

| Champ | Valeur |
|---|---|
| Projet | SecLists |
| Dépôt | https://github.com/danielmiessler/SecLists |
| Fichier(s) d'origine | `Passwords/Common-Credentials/Pwdb_top-1000000.txt` (SHA-256 `e9a88f67aafe65496682dc374559ee714e978bee50314767494c3e37a18c9fc8`) et `Passwords/Common-Credentials/xato-net-10-million-passwords-1000000.txt` (SHA-256 `424a3e03a17df0a2bc2b3ca749d81b04e79d59cb7aeec8876a5a3f308d0caf51`) |
| Commit ou version | `f90961c9c5be42d80e2f1b6e27b968530918862d` |
| Licence | MIT (fichier `LICENSE` du dépôt vérifié, SHA-256 `3dbdc93d5f8829de0941744841730a09c106d0732e5ae0e98ca1d77be7ded66c`) |
| Auteurs / titulaires | Copyright (c) 2018 Daniel Miessler |
| Nature de la reprise | Jeu de données transformé : seuls les mots de passe d'au moins 12 caractères sont gardés, normalisés (lettres ASCII en minuscules), puis remplacés par les 8 premiers octets de leur SHA-256 préfixé, triés et dédoublonnés. Aucun mot de passe n'est stocké en clair. |
| Destination | `crates/api/donnees/mots-de-passe-compromis.bin` (72 540 empreintes, 580 320 octets), lu par `crates/api/src/facteurs/compromis.rs` |
| Génération | `scripts/generer-mots-de-passe-compromis.py` : télécharge les deux fichiers au commit épinglé, vérifie leur SHA-256, produit le fichier octet pour octet (reproductible). SHA-256 du fichier produit : `62e9efcc7dfcfd528becc15dc9a17b21b0ed959738ca30696c7b05bdc999ef62` |
| Modifications | filtrage par longueur, normalisation, empreintes tronquées (voir ci-dessus) |
| PR | PR 8b du lot L2 (#170) |

Avis de licence de SecLists, conservé comme l'exige la licence MIT :

```text
MIT License

Copyright (c) 2018 Daniel Miessler

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```
