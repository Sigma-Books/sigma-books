# Intégrité : écriture canonique v4 et export v5 (devise de tenue)

Référence : spec §5.5 et §6.6, ADR 0007 et **0027**, `crates/integrite`.

Ces formats sont **figés** : toute modification crée une nouvelle version. Le vecteur `crates/integrite/tests/vecteurs/export-v5-reference.txt` les vérifie. Règles communes (§1 de `format-v1.md`) : UTF-8 sans BOM, `\n` partout, une donnée par ligne `cle=valeur`, aucune ligne vide ni commentaire.

## 1. Pourquoi
Jusqu'à l'export v4, le format suppose que **tout montant est en XOF** : `montant=` est « le montant XOF » et `devise=` « un code autre que XOF ». Or un dossier peut être tenu dans une autre devise (XAF, KMF, GNF, CDF, EUR…) et à une autre échelle (le CDF a deux décimales). Sans mention de la devise dans le texte haché :
- `sigma-verifier`, autonome, ne saurait ni accepter `devise=XOF` dans un dossier en XAF, ni arrondir la contre-valeur d'un dossier en CDF au centime ;
- les mêmes écritures d'un dossier XAF et d'un dossier XOF auraient la **même empreinte** : la devise ne serait pas protégée par la chaîne.

La devise de tenue et son échelle entrent donc dans le contenu haché, ce qui impose une nouvelle version (`format-v1.md` §6).

## 2. Écriture canonique v4
Identique à la v3 (`format-v4.md` §2 : date de valeur comptable facultative, étiquettes fiscales facultatives, pièces obligatoires), avec deux différences :
- la première ligne est `ecriture v4` ;
- deux lignes, **obligatoires**, suivent `date-validation=` (et `date-valeur-comptable=` si elle est présente) et précèdent `libelle=` :

| Ligne | Valeur |
|---|---|
| `devise-tenue=` | code ISO 4217 de la devise de tenue du dossier (`CDF`, `XAF`) |
| `decimales-tenue=` | échelle décimale de cette devise : un seul chiffre de `0` à `4` |

Sens des montants : `montant=` d'une ligne est exprimé dans la **devise de tenue** (contre-valeur pour une ligne en devise) ; `devise=` d'une ligne en devise est un code **autre que** la devise de tenue ; `montant-devise=` et `cours=` sont inchangés (la **sémantique de `cours=` change** par rapport aux formats v1 à v3 : « francs CFA pour une unité de devise » devient « unités de la devise de tenue pour une unité de devise »).

Règles propres à la v4, contrôlées à la lecture (donc par `sigma-verifier`, qui ne connaît aucune liste de devises) :
- **Échelle** : un `montant=` n'a pas plus de décimales que `decimales-tenue=`. Les formats v1 à v3 n'imposent aucune échelle (des écritures XOF avec décimales ont été validées, ADR 0020 point 6).
- **Contre-valeur** : pour une ligne en devise, `montant` vaut `montant-devise × cours`, arrondi à `decimales-tenue` décimales, demi éloigné de zéro (spec §6.6). Le vérificateur le recalcule.
- Nombres : forme décimale minimale (`format-v1.md` §2) ; `284288.36`, jamais `284288.360`.

### Choix de la version
La version est choisie par le **contenu** :
- devise de tenue **XOF sans décimale** : l'écriture garde son format v1, v2 ou v3, **octet pour octet** (règles de `format-v3.md` et `format-v4.md`). Les empreintes de tous les dossiers existants ne changent donc pas ;
- toute **autre** devise de tenue, ou le XOF avec une échelle non nulle : v4.

### Lecture stricte
Le texte doit être **exactement** la sérialisation de l'écriture qu'il décrit :
- `ecriture v4` qui déclare `devise-tenue=XOF` avec `decimales-tenue=0` est **refusé** (ce serait une v1, v2 ou v3 : sans ce refus, deux textes décriraient le même contenu et l'injectivité serait perdue) ;
- `devise-tenue=` et `decimales-tenue=` sont refusées dans une écriture v1, v2 ou v3 ;
- ordre des clés fixe, aucun doublon, aucune clé inconnue ; `decimales-tenue=02`, `+2`, `5` et un code en minuscules sont refusés ;
- `sans-piece=` n'existe pas en v4.

**Injectivité** (menace F7.3) : la propriété `serialisation_injective` couvre les quatre versions, pour plusieurs devises de tenue.

## 3. Export v5
Identique à l'export v4 (`format-v4.md` §3), avec deux différences :
- l'en-tête est `sigma-books export v5` ;
- chaque écriture est au format canonique **v1, v2, v3 ou v4**. Un export v4 n'admet que des écritures v1 à v3 (une écriture v4 y est refusée), un export v3 que des v1 et v2.

La chaîne de hachage (`format-v1.md` §3), les jetons (`format-v2.md` §3) et le témoin (`format-v2.md` §4) sont inchangés. L'empreinte de départ dépend toujours du seul identifiant du dossier ; la devise est protégée par chaque écriture.

Sigma Books écrit la version **la plus basse qui admet toutes les écritures** : v3 ou v4 pour un dossier XOF (l'export reste identique à celui d'avant), v5 pour tout autre dossier ayant au moins une écriture. `sigma-verifier` lit les v1 à v5. Une version antérieure du vérificateur refuse un export v5 (« en-tête inconnu ») : un échec explicite, jamais un succès silencieux.

## 4. Vérification
`sigma-verifier` contrôle l'intégrité (forme, chaîne, numérotation, équilibre en devise de tenue, jetons) et, pour la v4, l'échelle et la contre-valeur (§2). Il ne contrôle **pas** que la devise de tenue est celle que le pays du dossier fixe par défaut : il est autonome, et la devise du dossier se fige à sa première écriture validée (ADR 0027).

## 5. Vecteur
`export-v5-reference.txt` : dossier `dossier-cdf`, tenu en CDF (deux décimales), trois écritures v4 : vente taxée à 18 % avec centimes et étiquettes fiscales, achat de 100,55 EUR au cours de 2 827,3333 (contre-valeur 284 288,363315, arrondie à 284 288,36), contre-passation de la vente. Produit **indépendamment** du moteur (Python, `hashlib`) à partir de ce document, sans jeton d'horodatage. Les vecteurs v1 à v4 (dossier XOF) sont inchangés.
