# Pièce interne : note d'opérations diverses, forme canonique, version 1

Référence : spec §2 principe 2, §6.5 ; ADR 0015 ; `crates/api/src/pieces/interne.rs`. Menaces couvertes : F3.1 (remplacement d'une pièce), F7.3 (ambiguïté de sérialisation).

Une **pièce interne** est saisie dans Sigma Books : en L2, une note d'opérations diverses datée et motivée. Le serveur la sérialise sous la forme ci-dessous, en calcule la SHA-256, puis la stocke **telle quelle** comme une pièce externe (stockage objet WORM, ADR 0015). Le texte stocké est la pièce : son empreinte est celle qu'une écriture référence. Une correction crée une nouvelle pièce.

Ce format est **figé**. Toute modification, même d'un caractère, crée une version 2 : on ne modifie jamais la version 1. Les vecteurs de `crates/api/tests/vecteurs/piece-interne-v1/`, produits indépendamment du code (Python, `hashlib`), le vérifient (`crates/api/tests/piece_interne.rs`).

## 1. Règles communes

Celles de `audit-v1.md` §1 :
- texte UTF-8 sans BOM ; chaque ligne se termine par `\n`, **y compris la dernière** ; aucun `\r` ;
- une donnée par ligne, `cle=valeur` ; la valeur va du premier `=` à la fin de la ligne et peut contenir `=` ;
- aucune ligne vide, aucun commentaire ; les clés apparaissent **dans l'ordre du §2** ;
- lecture **stricte** : un texte n'est accepté que s'il est exactement la sérialisation de la note qu'il décrit. Deux textes différents ne décrivent jamais la même note.

| Nom | Forme |
|---|---|
| UUID | 36 caractères `8-4-4-4-12`, hexadécimal **minuscule** |
| texte | 0 à 1 000 octets UTF-8, sans caractère de contrôle (`Cc`, tabulation comprise), repris tel quel : ni normalisation Unicode, ni retrait d'espaces |
| instant | `AAAA-MM-JJTHH:MM:SS.ffffffZ` : UTC, toujours six décimales, années 0000 à 9999 |
| date | `AAAA-MM-JJ`, date du calendrier grégorien, années 0000 à 9999 |

## 2. Note canonique

| Ligne | Présence | Valeur |
|---|---|---|
| `sigma-books piece-interne v1` | toujours | marqueur de version : il entre dans le contenu haché |
| `nature=` | toujours | `note-od` |
| `cabinet=` | toujours | UUID du cabinet |
| `dossier=` | toujours | UUID du dossier : la note n'est valable que dans ce dossier |
| `auteur=` | toujours | UUID de l'utilisateur qui l'a rédigée |
| `instant=` | toujours | instant de rédaction, horloge de `api` |
| `date=` | toujours | date de l'opération justifiée |
| `motif=` | toujours | texte **non blanc** : la justification de l'opération |
| `ligne=` | zéro à 200 fois | texte : l'explication détaillée, une ligne par ligne (une ligne vide s'écrit `ligne=`) |
| `fin` | toujours | fin de la note |

Exemple (vecteur `note-od.txt`) :

```
sigma-books piece-interne v1
nature=note-od
cabinet=0193e0c2-7d1a-4c4e-9f00-1a2b3c4d5e6f
dossier=0193e0c3-1111-4a2b-8c3d-000000000001
auteur=0193e0c4-aaaa-4bbb-8ccc-000000000001
instant=2026-11-19T08:30:00.000001Z
date=2026-11-30
motif=Dotation aux amortissements de novembre = 1/12 de l'annuité
ligne=Matériel de bureau, linéaire sur 5 ans
ligne=
ligne=Tableau d'amortissement : voir le registre des immobilisations
fin
```

SHA-256 : `980f78942ec243e9f5b9f0fcc12c15730c771efcfb982ce0c58e2c73878b04b2` (`note-od.sha256`).

**Minimisation** : aucun montant ni numéro de compte n'est exigé par le format ; ils figurent dans l'écriture, pas dans la note. Le texte libre peut en citer.

## 3. Stockage et téléchargement

- Type stocké et servi : `text/plain; charset=utf-8`, toujours en pièce jointe (ADR 0015), nom proposé `note-od-<date>.txt`.
- L'empreinte est recalculée à chaque lecture, comme pour une pièce externe.

## 4. Vecteurs

| Fichier | Attendu |
|---|---|
| `note-od.txt`, `note-od-minimale.txt` | admis ; resérialisés à l'identique ; SHA-256 du fichier `.sha256` voisin |
| `rejet-*.txt` | refusés : fin de ligne finale absente, UUID en majuscules, instant sans microsecondes, motif blanc, clés dans le désordre, clé inconnue, date invalide (29 février 2027), tabulation |
| (construit par le test) | refusé : fins de ligne CRLF (le dépôt normalise les fins de ligne de ses fichiers texte) |
