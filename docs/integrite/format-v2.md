# Intégrité : export, version 2 (jetons d'horodatage)

Référence : spec §6.4, ADR 0007 et 0010, `crates/integrite`. Menaces couvertes : celles de la v1, plus F2.4 (réécriture complète de l'export).

Ce format est **figé** : toute modification crée une version 3. Le vecteur `crates/integrite/tests/vecteurs/export-v2.txt` le vérifie.

## 1. Ce qui change par rapport à la v1
- L'en-tête devient `sigma-books export v2`.
- Des **jetons d'horodatage** RFC 3161 suivent la tête et le nombre d'écritures.
- **Inchangés** : les règles communes (`format-v1.md` §1), l'écriture canonique (`ecriture v1`, §2) et la chaîne de hachage (§3). Une même chaîne a la même tête en v1 et en v2.
- Sigma Books écrit la v2. `sigma-verifier` lit la v1 et la v2.

## 2. Structure

```
sigma-books export v2
dossier=<identifiant>
<écriture canonique 1>
empreinte=<empreinte_1>
…
<écriture canonique N>
empreinte=<empreinte_N>
tete=<empreinte_N, ou empreinte_0 si N = 0>
nombre=<N>
<jeton 1>
…
<jeton J>
fin-export
```

Les règles de la v1 s'appliquent à la partie commune (`format-v1.md` §4) : rien ne suit `fin-export\n`, lignes de 1 024 octets au plus.

## 3. Jeton

```
jeton
nombre=<n>
tete=<empreinte_n>
der=<64 caractères base64>
…
der=<1 à 64 caractères base64>
fin-jeton
```

| Ligne | Valeur |
|---|---|
| `nombre=` | nombre d'écritures de la chaîne au moment du témoin (0 ≤ n ≤ N), décimal sans zéro de tête |
| `tete=` | empreinte de la chaîne après ces `n` écritures (`empreinte_n`) |
| `der=` | le jeton RFC 3161 (`TimeStampToken`, CMS `SignedData`, encodage DER), en base64 standard (RFC 4648 §4, avec `=`), découpé en lignes de 64 caractères, la dernière de 1 à 64 |

- Le base64 doit être **canonique** : le décoder puis le réencoder redonne exactement le texte lu.
- Un jeton fait au plus 64 Kio (DER). Un export compte au plus 10 000 jetons.
- Les jetons sont dans l'ordre de leur obtention. Plusieurs jetons peuvent porter sur le même état (autorités différentes).

## 4. Témoin (contenu horodaté)

```
sigma-books temoin v1
dossier=<identifiant>
tete=<empreinte_n>
nombre=<n>
```

Chaque ligne se termine par `\n`, la dernière comprise. L'autorité reçoit `SHA-256(témoin)` dans le `messageImprint` de la requête (`TimeStampReq` v1, `certReq` vrai, nonce de 64 bits). Le témoin ne contient aucune donnée comptable.

## 5. Vérification (`sigma-verifier`)

En plus des contrôles de la v1 (`format-v1.md` §5) :

| Contrôle | Anomalie signalée |
|---|---|
| Forme du bloc, base64 canonique, tailles | export illisible, avec la ligne |
| `tete` = chaîne **recalculée** après `nombre` écritures | témoin différent : l'export n'est pas celui qui a été horodaté |
| Jeton : empreinte du témoin, attributs signés, certificat désigné, usage `timeStamping` critique, signature, chaîne jusqu'à une ancre, validité des certificats à la date du jeton | jeton invalide, avec le motif |

Le rapport indique, pour chaque jeton valide : les écritures couvertes (les `n` premières), la date du jeton (UTC), l'autorité et sa racine. Il signale les écritures postérieures au dernier jeton.

Ancres intégrées : DigiCert Trusted Root G4, Sectigo Public Time Stamping Root R46 (`crates/integrite/src/horodatage/ancres/README.md`). Option `--ancre FICHIER` (PEM ou DER) pour en ajouter.

**Limite connue** : la révocation des certificats n'est pas contrôlée (vérification hors ligne).
