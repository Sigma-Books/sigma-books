# Intégrité : écriture canonique v3 et export v4 (étiquette fiscale)

Référence : spec §6.2 (invariants 4 et 5) et §6.4, L2 question 7.2, ADR 0007 et **0019**, `crates/integrite`.

Ces formats sont **figés** : toute modification crée une nouvelle version. Les vecteurs `crates/integrite/tests/vecteurs/export-v4-reference.txt` et `v4-rejets/` les vérifient. Règles communes (§1 de `format-v1.md`) : UTF-8 sans BOM, `\n` partout, une donnée par ligne `cle=valeur`, aucune ligne vide ni commentaire.

## 1. Pourquoi
La ligne d'écriture porte une **étiquette fiscale** facultative : ce qu'elle représente pour la TVA (base ou taxe, collectée ou déductible, régime, taux, exigibilité, motif d'exonération). L'information doit être figée avec l'écriture : elle entre dans le contenu haché, ce qui impose une nouvelle version du format canonique (`format-v1.md` §6). La correspondance avec les cases de la déclaration reste **hors** de la ligne (donnée versionnée, L5).

## 2. Écriture canonique v3
Identique à la v1 (`format-v1.md` §2) et à la v2 (`format-v3.md` §2), avec trois différences :
- la première ligne est `ecriture v3` ;
- `date-valeur-comptable=` est **facultative** (présente si l'écriture en porte une, à sa place habituelle, après `date-validation=`) ;
- dans le bloc `ligne`, **après `tiers=`**, un groupe de clés d'étiquette, dans cet ordre :

| Ligne | Présence | Valeur |
|---|---|---|
| `fiscal-nature=` | si la ligne est étiquetée | `base` ou `taxe` |
| `fiscal-sens=` | avec `fiscal-nature=` | `collectee` ou `deductible` |
| `fiscal-regime=` | avec `fiscal-nature=` | code du régime : 1 à 32 caractères `A-Z a-z 0-9 _ -`, le premier alphanumérique (`IMP18`, `EXO361` dans les vecteurs publiés ; `TAXABLE_NORMAL`, `EXO_SANS_DEDUCTION` depuis l'ADR 0030) |
| `fiscal-taux=` | si le régime comporte un taux | taux en pourcent, décimal exact : `18`, `10`, `5.5` |
| `fiscal-exigibilite=` | si le régime en porte une | `debit` ou `encaissement` |
| `fiscal-motif=` | ligne de **base** d'un régime exonéré | code du motif, même alphabet ; préfixé du code du régime et de `-` (`EXO361-4` pour `EXO361`, `EXO_SANS_DEDUCTION-4` pour `EXO_SANS_DEDUCTION`) |

Les trois premières clés vont **ensemble** (tout ou rien) ; les trois suivantes sont facultatives. Une ligne sans étiquette n'a aucune de ces clés.

**Taux** : chiffres seulement, au plus 3 chiffres avant le point et 6 après ; ni signe, ni exposant, ni zéro de tête, ni zéro final superflu (`18`, jamais `18.0` ni `018`) ; strictement positif et au plus 100.

**Cohérence interne**, contrôlée par la lecture (et donc par `sigma-verifier`, qui ne connaît aucune liste de régimes) : un motif n'accompagne qu'une ligne de base (une opération exonérée n'a pas de ligne de taxe) et son code est préfixé de celui du régime. L'appartenance du régime et du motif aux listes fermées de `crates/fiscal` (socle et paquet du pays, version de paramètres du contenu haché `parametres=`, ADR 0030), et les règles du régime (motif obligatoire pour un régime exonéré, taux admis, exigibilité), sont appliquées par le moteur à la saisie et à la validation ; l'écriture est ensuite figée par la chaîne.

### Choix de la version
La version est choisie par le **contenu** :
- aucune ligne étiquetée, pas de date de valeur comptable : **v1**, octet pour octet ;
- aucune ligne étiquetée, une date de valeur comptable : **v2**, octet pour octet ;
- **au moins une ligne étiquetée** : v3.

Les empreintes des écritures sans étiquette ne changent donc pas.

### Lecture stricte
Le texte doit être **exactement** la sérialisation de l'écriture qu'il décrit :
- `ecriture v3` sans aucune ligne étiquetée est refusé (ce serait une v1 ou une v2) ;
- les clés `fiscal-*` sont refusées dans une écriture v1 ou v2 ;
- ordre des clés fixe, aucun doublon, aucune clé inconnue ;
- `sans-piece=` n'existe pas en v3 (comme en v2) ;
- taux non canonique, code mal formé, valeur d'énumération inconnue : refusés.

**Injectivité** (menace F7.3) : la propriété `serialisation_injective` couvre les trois versions : toute écriture se relit exactement depuis sa forme canonique, deux écritures distinctes ont deux formes distinctes.

### Contre-passation
La ligne miroir d'une contre-passation reprend l'étiquette **telle quelle** : seul le sens `D`/`C` s'inverse. La déclaration, qui somme débit − crédit par étiquette, s'annule d'elle-même.

## 3. Export v4
Identique à l'export v3 (`format-v3.md` §3), avec deux différences :
- l'en-tête est `sigma-books export v4` ;
- chaque écriture est au format canonique **v1, v2 ou v3**. Un export v3 n'admet que des écritures v1 et v2 (une écriture v3 y est refusée), un export v1 ou v2 que des écritures v1.

La chaîne de hachage (`format-v1.md` §3), les jetons (`format-v2.md` §3) et le témoin (`format-v2.md` §4) sont inchangés.

Sigma Books écrit la version **la plus basse qui admet toutes les écritures** : v3 si aucune écriture n'est étiquetée (l'export reste identique à celui d'avant), v4 sinon. `sigma-verifier` lit les v1 à v4.

## 4. Vérification
`sigma-verifier` contrôle l'intégrité (forme, chaîne, numérotation, équilibre, jetons) comme pour les versions précédentes, plus la forme et la cohérence interne des étiquettes (§2). Il ne contrôle **pas** les listes de `fiscal` : il est autonome. Une étiquette retouchée change l'empreinte de l'écriture : l'altération est signalée.

## 5. Vecteurs
- `export-v4-reference.txt` : les 6 écritures du vecteur v3 (mêmes empreintes), puis quatre écritures v3 (vente taxée à 18 %, vente exonérée art. 361-4, contre-passation de la vente avec les mêmes étiquettes, achat à l'encaissement avec date de valeur comptable). Produit **indépendamment** du moteur (Python, `hashlib`) à partir de ce document, sans jeton d'horodatage (le conteneur est celui de la v2).
- `v4-rejets/` : douze exports re-chaînés, dont la 7e écriture est défectueuse (v3 sans étiquette, taxe avec motif, taux non canonique ou nul, clés en désordre, nature ou exigibilité inconnue, étiquette incomplète, motif hors régime, code de régime mal formé, écriture v3 dans un export v3, clés fiscales dans une écriture v1), et un export à étiquette retouchée dont l'empreinte n'a pas été recalculée.
