# Intégrité : format canonique et export, version 1

Référence : spec §6.4, ADR 0007, `crates/integrite`. Menaces couvertes : F2.3 (numérotation), F7.1 (export retouché), F7.3 (ambiguïté de sérialisation), F7.5 (export malformé).

Ce format est **figé**. Toute modification, même d'un caractère, crée une version 2 : on ne modifie jamais la version 1. Les vecteurs de `crates/integrite/tests/vecteurs/` le vérifient.

> **Statut** : l'export v1 n'est plus produit ; Sigma Books écrit l'export **v2** (`format-v2.md`), qui ajoute les jetons d'horodatage. `sigma-verifier` lit toujours la v1. L'écriture canonique (§2) et la chaîne (§3) restent celles décrites ici.

## 1. Règles communes
- Texte UTF-8, sans BOM. Chaque ligne se termine par `\n` (jamais `\r\n`), **y compris la dernière**.
- Une donnée par ligne, sous la forme `cle=valeur`. La valeur commence après le premier `=` et va jusqu'à la fin de la ligne ; elle peut contenir `=`.
- Les valeurs ne contiennent jamais de fin de ligne. Les textes saisis (`TexteCourt`) excluent tout caractère de contrôle et sont repris **tels quels**, sans normalisation Unicode ni retrait d'espaces. Les autres valeurs ont un alphabet restreint.
- Aucune ligne vide, aucun commentaire.

## 2. Écriture canonique (contenu haché)

Les clés apparaissent **dans cet ordre**. Les clés facultatives sont omises quand elles sont absentes ; aucune autre ligne n'est admise.

| Ligne | Présence | Valeur |
|---|---|---|
| `ecriture v1` | toujours | marqueur de version : il entre dans le contenu haché |
| `numero=` | toujours | `JOURNAL-EXERCICE-RANG`, rang sur 6 chiffres (`VT-2026-000001`) |
| `date=` | toujours | date comptable `AAAA-MM-JJ` |
| `date-validation=` | toujours | date de validation `AAAA-MM-JJ` |
| `libelle=` | toujours | libellé tel que saisi |
| `origine=` | contre-passation seulement | numéro de l'écriture contre-passée |
| `parametres=` | si renseigné | version des paramètres fiscaux |
| `piece=` | une ou plusieurs (justification par pièces) | empreinte SHA-256 de la pièce, 64 caractères `0-9a-f`, dans l'ordre de rattachement, sans doublon |
| `sans-piece=` | à la place de `piece=` | motif « sans pièce » |
| `ligne` | une par ligne d'écriture, dans l'ordre de saisie | marqueur, suivi des clés de la ligne : |
| `compte=` | toujours | numéro de compte |
| `sens=` | toujours | `D` (débit) ou `C` (crédit) |
| `montant=` | toujours | montant XOF strictement positif (contre-valeur pour une ligne en devise) |
| `devise=` | ligne en devise seulement | code ISO 4217, autre que `XOF` |
| `montant-devise=` | avec `devise=` | montant en devise |
| `cours=` | avec `devise=` | francs CFA pour une unité de devise |
| `tiers=` | si renseigné | identifiant du tiers |
| `fin` | toujours | fin de l'écriture |

**Nombres** : forme décimale minimale, sans signe `+`, sans exposant, sans séparateur de milliers, sans zéro superflu (`150000`, `228.67`, `655.957` ; jamais `150000.00` ni `655.9570`).

**Contre-valeur** : pour une ligne en devise, `montant` vaut `montant-devise × cours`, arrondi au franc (demi éloigné de zéro, spec §6.6). Le vérificateur le recalcule.

**Lecture stricte** : un texte n'est accepté que s'il est **exactement** la sérialisation de l'écriture qu'il décrit. Une variante équivalente (zéro superflu, doublon de pièce…) est refusée. Deux textes différents ne peuvent donc jamais décrire la même écriture (menace F7.3).

## 3. Chaîne de hachage

```
empreinte_0 = SHA-256("sigma-books chaine v1\n" ‖ identifiant du dossier)
empreinte_n = SHA-256(empreinte_{n-1} ‖ écriture canonique n)
```

- `empreinte_{n-1}` entre sous forme de **32 octets bruts** ; l'écriture, sous forme de ses octets UTF-8, `fin\n` compris.
- L'empreinte de départ dépend du dossier : une chaîne ne peut pas être greffée sur celle d'un autre dossier.
- Identifiant du dossier : 1 à 64 caractères parmi `A-Z a-z 0-9 . _ -`.
- Ordre : celui de la validation, tous journaux et exercices confondus (une chaîne par dossier).
- Une empreinte s'écrit en hexadécimal minuscule (64 caractères).

## 4. Export d'intégrité

```
sigma-books export v1
dossier=<identifiant>
<écriture canonique 1>
empreinte=<empreinte_1>
…
<écriture canonique N>
empreinte=<empreinte_N>
tete=<empreinte_N, ou empreinte_0 si N = 0>
nombre=<N>
fin-export
```

`nombre` s'écrit en décimal, sans zéro de tête. Rien ne suit `fin-export\n`.

**Limites de lecture** : une ligne fait au plus 1 024 octets, fin de ligne comprise ; une écriture, au plus 4 Mio. La lecture est ligne à ligne : la mémoire consommée ne dépend pas d'une valeur lue.

## 5. Vérification (`sigma-verifier`)

```
sigma-verifier [--dossier ID] [--tete EMPREINTE] <export | ->
```

| Contrôle | Anomalie signalée |
|---|---|
| Forme (§1, §2, §4) | export illisible, avec la ligne ; la vérification s'arrête là |
| `empreinte_n` = SHA-256(empreinte déclarée précédente ‖ écriture n) | écriture ou empreinte altérée, localisée |
| `tete` = chaîne recalculée depuis `empreinte_0` | tête différente |
| `nombre` = écritures lues | nombre différent |
| Rangs 1, 2, 3… sans trou ni doublon par journal et exercice | numérotation discontinue |
| ∑ débits = ∑ crédits (XOF) | écriture déséquilibrée |
| L'origine d'une contre-passation la précède dans la chaîne | origine inconnue |
| `--dossier` | dossier inattendu |
| `--tete` : chaîne recalculée = empreinte fournie | ancrage différent |

Codes de retour : `0` intègre, `1` anomalie, `2` usage incorrect ou lecture impossible.

**Limite connue** : celui qui maîtrise tout l'export peut le réécrire entièrement et recalculer toute la chaîne. Seule une **référence externe** le détecte : un jeton d'horodatage RFC 3161 (PR suivante du lot L1) ou une empreinte de tête conservée par un témoin (option `--tete`, menace F2.4). Le rapport le rappelle quand aucune référence n'est fournie.

## 6. Évolutions
- Les jetons d'horodatage RFC 3161 et leur vérification arrivent avec la PR « Horodatage » du lot L1 (issue #61). Ils formeront une section ajoutée à l'export, sous une nouvelle version du conteneur. Le format canonique des écritures (§2) et la chaîne (§3) restent inchangés.
- Le contenu canonique ne contient que des données de l'écriture validée. L'ajout d'un champ métier à l'écriture validée impose une version 2 du format canonique.
