# `syscohada` — liste officielle des comptes

`data/comptes.toml` contient les **1 401 comptes** du SYSCOHADA révisé :
- 85 à deux chiffres, 430 à trois chiffres, 886 à quatre chiffres ;
- les 9 classes.

Le crate les charge par `include_str!`, puis les valide à chaque chargement : `Referentiel::officiel()`.

## Source et méthode
- **Source qui fait foi** : AUDCIF et Système comptable OHADA, JO OHADA n° spécial du 15/02/2017, Titre VII, section 3, « Liste des comptes ». Ce sont les pages PDF 219 à 272 ; la page imprimée vaut la page PDF moins 3.
  - Le champ `page` de chaque compte renvoie à la page PDF.
  - Seuls les numéros et intitulés sont repris. Les notes de bas de page ne le sont pas.
- **Extraction** :
  - Point de départ : l'extrait OCR des références locales.
  - Accents restitués lorsque l'intitulé coïncidait exactement, à la casse et aux accents près, avec celui du module Tryton `account_syscohada`.
  - Chacune des 54 pages a ensuite été **relue sur l'image du PDF** et corrigée ligne par ligne : erreurs d'OCR, comptes manquants ou fusionnés, numéros mal lus. Par exemple, l'OCR lisait « 6383 Mécénat » au lieu de 6583, et « 15 Autres produits » au lieu de 75.
- **Casse** : celle du texte officiel. Les comptes à deux et trois chiffres sont en majuscules. Certains intitulés de sous-comptes sont elliptiques, tels qu'imprimés : « 1661 sur emprunts obligataires », sous « 166 INTÉRÊTS COURUS ».

## Écarts volontaires avec le texte imprimé
**Coquilles manifestes corrigées :**
- 130 « INSANCE » devient « INSTANCE » ;
- 27 « IMMOBLISATIONS » devient « IMMOBILISATIONS » ;
- 393 « APPOVISIONNEMENTS » devient « APPROVISIONNEMENTS » ;
- 656 « COMMERCIALE » devient « COMMERCIALES » ;
- accents manquants restitués sur des intitulés en majuscules.

**Numéros ou intitulés restitués**, signalés par le champ `note` (3 comptes) :

| Compte | Texte imprimé | Retenu | Raison |
|---|---|---|---|
| 4125 | « 412 Organismes Internationaux, Effets à recevoir » | 4125 | Numéro tronqué. Analogie avec 4115 ; Tryton retient aussi 4125. |
| 4794 | « Diminution des dettes d'exploitation » | « Diminution des dettes financières » | Doublon de 4793. Symétrie avec 4784. |
| 908 | « 907 AUTRES ENGAGEMENTS ACCORDÉS » | 908 | Doublon de 907. Sous-comptes 9081 à 9088 ; « Engagements accordés 905 à 908 ». |

**Non repris :**
- les plages « 911 à 914 » et « 915 à 918 » sous le compte 91 : le texte n'en donne pas les comptes ;
- les notes de bas de page.

## Champs qui ne viennent pas de la liste officielle
- **`nature`** : déduite de l'intitulé officiel de la classe.

  | Comptes | Nature |
  |---|---|
  | Classes 1 à 5 | bilan |
  | Classe 6 ; comptes 81, 83, 85, 87 et 89 | charge |
  | Classe 7 ; comptes 82, 84, 86 et 88 | produit |
  | Comptes 90 et 91 | hors bilan |
  | Comptes 92 à 99 | comptabilité analytique de gestion |

- **`sens`**, le sens normal du solde : la liste officielle ne le donne pas. C'est une **convention de Sigma Books, à faire valider par un expert-comptable**. Elle sert à l'affichage et aux contrôles de vraisemblance, jamais au calcul. Règles, du préfixe le plus long au plus court :
  - **Par défaut, selon la classe** : 1 et 7 créditeurs ; 2, 3, 5 et 6 débiteurs ; 4 et 9 variables.
  - **Classe 8** : comptes de charges débiteurs, comptes de produits créditeurs.
  - **Créditeurs** : 28, 29, 39 et 49 ; 59 ; 56 ; 40, sauf 409 ; 419 ; 42, sauf 421 ; 43 ; 441, 442, 443, 4441, 446 et 447 ; 462 et 465 ; 477 et 479 ; 481, 482 et 484.
  - **Débiteurs** : 109, 129 et 139 ; 409 ; 41, sauf 419 ; 421 ; 445 et 4449 ; 467 ; 476 et 478 ; 485 et 488.
  - **Rabais, remises et ristournes** : les comptes en `60x9` sont créditeurs, les comptes en `70x9` débiteurs.
  - **Comptes de variation** (603, 73) et comptes de liaison (18, 58) : variables.
  - **Charges à payer et produits à recevoir** (4286/4287, 4386/4387, 4486/4487, 5261/5267) : respectivement créditeurs et débiteurs.
  - **Comptes de regroupement** : un compte dont les sous-comptes ont des sens différents est variable.
- **`collectif`** : compte collectif de tiers (spec §5.1, invariant 4 du §6.2) ; les sous-comptes en héritent dans le plan du dossier.
  - **Repris des comptes `party_required` du module Tryton `account_syscohada`** : 103, 109, 181 à 185, 401, 402, 408, 409, 411, 412, 414 à 416, 418, 419, 421, 451, 452, 461 à 463, 465, 466, 481, 482, 485.
  - **Ajouts de Sigma Books**, comptes de tiers de la liste officielle absents de Tryton : 404, 413, 467.
  - Voir `docs/provenance/tryton.md`.

## Comparaison avec Tryton et Odoo
| | Tryton `account_syscohada` (2016, `f5354cc`) | Odoo 19.0 `l10n_syscohada` (`fa175fb`) |
|---|---|---|
| Comptes hors liste officielle | 21 : 1666, 1676, 2715, 2766, 2821, 2825, 4126, 4442, 4714, 4727, 4728, 4981 à 4983, 5361, 5367, 611, 6235, 6721, 687, 872 | 39 : 2715, 2766, 4113, 4445, 469, 4856, 6388, et des sous-comptes inventés sous 91 (9111 à 9188) |
| Comptes officiels absents | 149, dont toute la classe 9 et des comptes ajoutés en 2017 (2184, 4016, 404x, 413x, 551 à 558…) | 9 comptes à quatre chiffres (5181 à 5187, 5211, 5215, 5261, 5267) |
| Intitulés | Souvent paraphrasés ; décalages sous 168 (1684 à 1686) et 571 à 573 | Paraphrasés (ex. 1011 « Capital engagé non appelé ») |

Aucune des deux références n'est fiable seule : **le PDF officiel a tranché chaque écart**.
