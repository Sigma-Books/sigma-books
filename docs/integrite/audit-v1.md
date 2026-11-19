# Journal d'audit : entrée canonique, chaîne et export, version 1

Référence : spec §21, ADR 0014 et 0016, `crates/integrite/src/audit/`. Menaces couvertes : F1.6 (répudiation), F1.15 (administrateur malveillant), F2.6 (altération de l'audit), F7.3 (ambiguïté de sérialisation), F7.5 (export malformé).

Ce format est **figé**. Toute modification, même d'un caractère, crée une version 2 : on ne modifie jamais la version 1. Les vecteurs de `crates/integrite/tests/vecteurs/audit-v1/`, produits indépendamment du code (Python, `hashlib`), le vérifient.

## 1. Règles communes
Celles de `format-v1.md` §1 et §4 :
- texte UTF-8 sans BOM ; chaque ligne se termine par `\n`, **y compris la dernière** ;
- une donnée par ligne, `cle=valeur` ; la valeur va du premier `=` à la fin de la ligne et peut contenir `=` ;
- aucune ligne vide, aucun commentaire ; les clés apparaissent **dans l'ordre du §2** ; une clé facultative absente est omise ;
- lecture **stricte** : un texte n'est accepté que s'il est exactement la sérialisation de l'entrée qu'il décrit. Deux textes différents ne décrivent jamais la même entrée (F7.3) ;
- une ligne fait au plus 1 024 octets, fin de ligne comprise ; une entrée, au plus 4 Mio.

Alphabets des valeurs :

| Nom | Forme |
|---|---|
| UUID | 36 caractères `8-4-4-4-12`, hexadécimal **minuscule** |
| symbole | 1 à 64 caractères parmi `A-Z a-z 0-9 . _ -` |
| texte | 0 à 1 000 octets UTF-8, sans caractère de contrôle (`Cc`), repris tel quel : ni normalisation Unicode, ni retrait d'espaces |
| empreinte | SHA-256, 64 caractères `0-9a-f` |
| instant | `AAAA-MM-JJTHH:MM:SS.ffffffZ` : UTC, toujours six décimales, années 0000 à 9999 |
| rang | décimal, strictement positif, sans signe ni zéro de tête |

## 2. Entrée canonique (contenu haché)

| Ligne | Présence | Valeur |
|---|---|---|
| `audit v1` | toujours | marqueur de version : il entre dans le contenu haché |
| `portee=` | toujours | `cabinet` ou `dossier` |
| `cabinet=` | toujours | UUID du cabinet |
| `dossier=` | portée `dossier` seulement | UUID du dossier |
| `rang=` | toujours | rang dans la chaîne de la portée, 1 pour la première entrée |
| `instant=` | toujours | instant de l'action, horloge de `api` |
| `acteur=` | toujours | UUID de l'utilisateur, ou `systeme` pour une tâche de la file |
| `role=` | acteur utilisateur seulement | symbole : son rôle dans le cabinet |
| `session=` | acteur utilisateur seulement | UUID public de la session, **jamais son jeton** |
| `action=` | toujours | symbole : identifiant d'action Cedar (`CreerDossier`…) |
| `ressource=` | toujours | `genre:uuid`, le genre étant un symbole (`dossier:…`) |
| `politique=` | acteur utilisateur : une ou plusieurs | symbole : `@id` d'une politique déterminante, **dans l'ordre croissant des octets, sans doublon** |
| `politiques=` | acteur utilisateur seulement | empreinte : version du jeu de politiques (§6) |
| `reauthentification=` | acteur utilisateur seulement | `oui` si la session était « fraîche » (facteur vérifié depuis moins de 5 minutes, ADR 0012), sinon `non` |
| `motif=` | si l'action l'exige | texte non blanc |
| `donnee=` puis `valeur=` | zéro ou plusieurs paires | symbole, puis texte : faits cités par l'action (tête et nombre exportés, numéro et empreinte d'une écriture validée…). Noms sans doublon, dans l'ordre de l'action |
| `champ=` puis `avant=`, `apres=` | zéro ou plusieurs groupes | symbole, puis textes : champ modifié. `avant=` absent pour une création, `apres=` absent pour une suppression, au moins l'un des deux. Noms sans doublon, dans l'ordre de l'action |
| `fin` | toujours | fin de l'entrée |

Un acteur `systeme` n'a ni `role=`, ni `session=`, ni décision (`politique=`, `politiques=`, `reauthentification=`).

Exemple (vecteur `entree-cabinet.txt`) :

```
audit v1
portee=cabinet
cabinet=0193e0c2-7d1a-4c4e-9f00-1a2b3c4d5e6f
rang=1
instant=2026-11-19T08:30:00.000001Z
acteur=0193e0c4-aaaa-4bbb-8ccc-000000000001
role=administrateur
session=0193e0c5-0000-4000-8000-0000000000a1
action=CreerDossier
ressource=dossier:0193e0c3-1111-4a2b-8c3d-000000000001
politique=s20-creer-dossier
politiques=526f827b79753f86d781ffa8dffaf6c9dd5f9c3374f69aa2115ca16c20885e90
reauthentification=non
champ=nom
apres=SARL Téranga = Dakar
champ=affectation
apres=0193e0c4-aaaa-4bbb-8ccc-000000000001
fin
```

**Minimisation** (ADR 0014) : aucune adresse IP, aucun jeton, secret ni clé ; l'acteur est désigné par son UUID ; seuls les champs modifiés figurent dans `champ=`.

## 3. Chaîne de hachage

```
empreinte_0 = SHA-256("sigma-books audit v1\n" ‖ portée ‖ "=" ‖ identifiant)
empreinte_n = SHA-256(empreinte_{n-1} ‖ entrée canonique n)
```

- `portée` vaut `cabinet` ou `dossier` ; `identifiant` est l'UUID du cabinet ou du dossier, sous sa forme canonique. Exemple : `sigma-books audit v1\ndossier=0193e0c3-1111-4a2b-8c3d-000000000001`.
- `empreinte_{n-1}` entre sous forme de **32 octets bruts** ; l'entrée, sous forme de ses octets UTF-8, `fin\n` compris.
- Une chaîne par cabinet et une par dossier. Le départ dépend de la portée et de l'identifiant, et chaque entrée répète sa portée, son cabinet, son dossier et son rang : une chaîne ne peut être greffée ni sur une autre portée ni sur un autre cabinet ou dossier.
- La création d'un dossier s'inscrit au cabinet, puis en première entrée de la chaîne du dossier (ADR 0014).

## 4. Vérification
Recalcul de la chaîne d'une portée depuis `empreinte_0`. Anomalies :

| Contrôle | Anomalie (code de l'API) |
|---|---|
| L'entrée est une entrée canonique (§1, §2) | `entree_illisible` |
| Portée, cabinet et dossier de l'entrée = ceux de la chaîne | `portee_differente` |
| Rang de l'entrée (et rang enregistré en base) = sa position | `rang_different` : entrée supprimée, insérée ou déplacée |
| `empreinte_n` = SHA-256(empreinte déclarée précédente ‖ entrée n) | `empreinte_alteree`, localisée |
| Tête enregistrée (ou `tete=` de l'export) = chaîne recalculée | `tete_differente` |
| Nombre enregistré (rang de la tête, ou `nombre=`) = entrées lues | `nombre_different` |

**Limite connue** (ADR 0014) : qui contrôle toute la base, ou tout l'export, peut réécrire une chaîne entière. Sans ancrage externe de la tête (option `--tete`), ce n'est pas détectable.

## 5. Export d'audit

```
sigma-books export-audit v1
portee=<cabinet | dossier>
cabinet=<uuid>
dossier=<uuid>                (portée dossier seulement)
<entrée canonique 1>
empreinte=<empreinte enregistrée de l'entrée 1>
…
<entrée canonique N>
empreinte=<empreinte enregistrée de l'entrée N>
tete=<tête enregistrée>
nombre=<N>
annexe-acteurs
acteur=<uuid>
nom=<texte>                   (facultatif)
…
fin-annexe
fin-export
```

- Les entrées et empreintes sont celles **enregistrées**, sans recalcul : la vérification de l'export dit si elles sont intègres.
- `nombre` s'écrit en décimal, sans zéro de tête. Rien ne suit `fin-export\n`.
- **Annexe non chaînée** : chaque UUID d'acteur utilisateur des entrées, une fois, dans l'ordre croissant, suivi de son nom au moment de l'export (texte non vide) s'il est connu. Elle n'entre dans aucune empreinte : la modifier ne change pas la chaîne. La vérification contrôle seulement qu'elle cite exactement les acteurs des entrées.
- L'export est lui-même inscrit à l'audit de la portée (ADR 0014) : action `ExporterAuditCabinet` ou `ExporterComplet`, avec les données `tete` et `nombre` exportées (et `export=audit` pour un dossier).

`sigma-verifier` reconnaît l'export d'audit à sa première ligne et le vérifie (§4), avec l'option `--tete` pour un ancrage externe. Codes de retour : `0` intègre, `1` anomalie ou export illisible, `2` usage incorrect ou lecture impossible.

## 6. Version du jeu de politiques
`politiques=` est la SHA-256 de la suite, pour le schéma puis chaque fichier de politiques embarqué dans l'ordre de `authz::POLITIQUES`, de : nom du fichier, `\n`, longueur du contenu en octets (décimal), `\n`, contenu. Elle change dès qu'un caractère du schéma ou d'une politique change (`crates/api/src/serveur/audit.rs`).
